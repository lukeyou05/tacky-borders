// TODO Add result handling. There's so many let _ =
use crate::colors::*;
use crate::utils::*;
use std::ptr;
use std::sync::LazyLock;
use std::sync::OnceLock;
use std::thread;
use std::time;
use windows::{
    core::*, Foundation::Numerics::*, Win32::Foundation::*, Win32::Graphics::Direct2D::Common::*,
    Win32::Graphics::Direct2D::*, Win32::Graphics::Dwm::*, Win32::Graphics::Dxgi::Common::*,
    Win32::Graphics::Gdi::*, Win32::UI::WindowsAndMessaging::*,
};

pub static RENDER_FACTORY: LazyLock<ID2D1Factory> = unsafe {
    LazyLock::new(|| {
        D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_MULTI_THREADED, None)
            .expect("creating RENDER_FACTORY failed")
    })
};

#[derive(Debug, Default)]
pub struct WindowBorder {
    pub border_window: HWND,
    pub tracking_window: HWND,
    pub window_rect: RECT,
    pub border_width: i32,
    pub border_offset: i32,
    pub border_radius: f32,
    pub brush_properties: D2D1_BRUSH_PROPERTIES,
    pub render_target: OnceLock<ID2D1HwndRenderTarget>,
    pub rounded_rect: D2D1_ROUNDED_RECT,
    pub active_color: Color,
    pub inactive_color: Color,
    pub current_color: Color,
    pub animations: Vec<AnimationType>,
    pub animation_speed: f32,
    pub animation_fps: i32,
    pub last_render_time: Option<time::Instant>,
    pub last_anim_time: Option<time::Instant>,
    pub in_event_anim: i32,
    // Delay border visbility when tracking window is in unminimize animation
    pub unminimize_delay: u64,
    // This is to pause the border from doing anything when it doesn't need to
    pub pause: bool,
}

impl WindowBorder {
    pub fn create_border_window(&mut self, hinstance: HINSTANCE) -> Result<()> {
        unsafe {
            self.border_window = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                w!("tacky-border"),
                w!("tacky-border"),
                WS_POPUP | WS_DISABLED,
                0,
                0,
                0,
                0,
                None,
                None,
                hinstance,
                Some(ptr::addr_of!(*self) as _),
            )?;
        }

        Ok(())
    }

    pub fn init(&mut self, init_delay: u64) -> Result<()> {
        // Delay the border while the tracking window is in its creation animation
        thread::sleep(time::Duration::from_millis(init_delay));

        if !self.animations.is_empty() {
            let timer_duration = (1000 / self.animation_fps) as u32;
            unsafe {
                SetTimer(self.border_window, 1, timer_duration, None);
            }
        }

        unsafe {
            // Make the window border transparent. Idk how this works. I took it from PowerToys.
            let pos: i32 = -GetSystemMetrics(SM_CXVIRTUALSCREEN) - 8;
            let hrgn = CreateRectRgn(pos, 0, pos + 1, 1);
            let mut bh: DWM_BLURBEHIND = Default::default();
            if !hrgn.is_invalid() {
                bh = DWM_BLURBEHIND {
                    dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
                    fEnable: TRUE,
                    hRgnBlur: hrgn,
                    fTransitionOnMaximized: FALSE,
                };
            }
            let _ = DwmEnableBlurBehindWindow(self.border_window, &bh);

            if SetLayeredWindowAttributes(self.border_window, COLORREF(0x00000000), 0, LWA_COLORKEY)
                .is_err()
            {
                println!("Error Setting Layered Window Attributes!");
            }
            if SetLayeredWindowAttributes(self.border_window, COLORREF(0x00000000), 255, LWA_ALPHA)
                .is_err()
            {
                println!("Error Setting Layered Window Attributes!");
            }

            let _ = self.create_render_targets();
            if has_native_border(self.tracking_window) {
                let _ = self.update_position(Some(SWP_SHOWWINDOW));
                let _ = self.render();

                // Sometimes, it doesn't show the window at first, so we wait 5ms and update it.
                // This is very hacky and needs to be looked into. It may be related to the issue
                // detailed in the wnd_proc. TODO
                thread::sleep(time::Duration::from_millis(5));
                let _ = self.update_position(Some(SWP_SHOWWINDOW));
                let _ = self.render();
            }

            let mut message = MSG::default();
            while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            println!("exiting border thread for {:?}!", self.tracking_window);
        }

        Ok(())
    }

    pub fn create_render_targets(&mut self) -> Result<()> {
        let render_target_properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_UNKNOWN,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        };
        let hwnd_render_target_properties = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd: self.border_window,
            pixelSize: Default::default(),
            presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
        };
        self.brush_properties = D2D1_BRUSH_PROPERTIES {
            opacity: 1.0,
            transform: Matrix3x2::identity(),
        };

        self.rounded_rect = D2D1_ROUNDED_RECT {
            rect: Default::default(),
            radiusX: self.border_radius,
            radiusY: self.border_radius,
        };

        if is_active_window(self.tracking_window) {
            self.current_color = self.active_color.clone();
        } else {
            self.current_color = self.inactive_color.clone();
        }

        unsafe {
            let factory = &*RENDER_FACTORY;
            let _ = self.render_target.set(
                factory
                    .CreateHwndRenderTarget(
                        &render_target_properties,
                        &hwnd_render_target_properties,
                    )
                    .expect("creating self.render_target failed"),
            );
            let render_target = self.render_target.get().unwrap();
            render_target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }

        let _ = self.update_color();
        let _ = self.update_window_rect();
        let _ = self.update_position(None);

        Ok(())
    }

    pub fn update_window_rect(&mut self) -> Result<()> {
        let result = unsafe {
            DwmGetWindowAttribute(
                self.tracking_window,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                ptr::addr_of_mut!(self.window_rect) as _,
                size_of::<RECT>() as u32,
            )
        };
        if result.is_err() {
            println!("Error getting frame rect! This is normal for apps running with elevated privileges");
            unsafe {
                let _ = ShowWindow(self.border_window, SW_HIDE);
                self.pause = true;
            }
        }

        self.window_rect.top -= self.border_width;
        self.window_rect.left -= self.border_width;
        self.window_rect.right += self.border_width;
        self.window_rect.bottom += self.border_width;

        Ok(())
    }

    pub fn update_position(&mut self, c_flags: Option<SET_WINDOW_POS_FLAGS>) -> Result<()> {
        unsafe {
            // Place the window border above the tracking window
            let hwnd_above_tracking = GetWindow(self.tracking_window, GW_HWNDPREV);
            let mut u_flags =
                SWP_NOSENDCHANGING | SWP_NOACTIVATE | SWP_NOREDRAW | c_flags.unwrap_or_default();

            // If hwnd_above_tracking is the window border itself, we have what we want and there's
            //  no need to change the z-order (plus it results in an error if we try it).
            // If hwnd_above_tracking returns an error, it's likely that tracking_window is already
            //  the highest in z-order, so we use HWND_TOP to place the window border above.
            if hwnd_above_tracking == Ok(self.border_window) {
                u_flags |= SWP_NOZORDER;
            }

            let result = SetWindowPos(
                self.border_window,
                hwnd_above_tracking.unwrap_or(HWND_TOP),
                self.window_rect.left,
                self.window_rect.top,
                self.window_rect.right - self.window_rect.left,
                self.window_rect.bottom - self.window_rect.top,
                u_flags,
            );
            if result.is_err() {
                println!("Error setting window position! This is normal for apps running with elevated privileges");
                let _ = ShowWindow(self.border_window, SW_HIDE);
                self.pause = true;
            }
        }
        Ok(())
    }

    pub fn update_color(&mut self) -> Result<()> {
        // TODO maybe I can move the fade anim code from WM_TIMER into this function instead
        if self.animations.contains(&AnimationType::Fade) {
            return Ok(());
        }

        self.current_color = if is_active_window(self.tracking_window) {
            self.active_color.clone()
        } else {
            self.inactive_color.clone()
        };

        Ok(())
    }

    pub fn render(&mut self) -> Result<()> {
        self.last_render_time = Some(time::Instant::now());

        // Get the render target
        let Some(render_target) = self.render_target.get() else {
            return Ok(());
        };

        let pixel_size = D2D_SIZE_U {
            width: (self.window_rect.right - self.window_rect.left) as u32,
            height: (self.window_rect.bottom - self.window_rect.top) as u32,
        };

        self.rounded_rect.rect = D2D_RECT_F {
            left: (self.border_width / 2 - self.border_offset) as f32,
            top: (self.border_width / 2 - self.border_offset) as f32,
            right: (self.window_rect.right - self.window_rect.left - self.border_width / 2
                + self.border_offset) as f32,
            bottom: (self.window_rect.bottom - self.window_rect.top - self.border_width / 2
                + self.border_offset) as f32,
        };

        unsafe {
            let _ = render_target.Resize(&pixel_size);

            //let before = std::time::Instant::now();

            let Some(brush) = self.current_color.create_brush(
                render_target,
                &self.window_rect,
                &self.brush_properties,
            ) else {
                return Ok(());
            };

            //println!("time to create brush: {:?}", before.elapsed());

            render_target.BeginDraw();
            render_target.Clear(None);
            render_target.DrawRoundedRectangle(
                &self.rounded_rect,
                &brush,
                self.border_width as f32,
                None,
            );
            let _ = render_target.EndDraw(None, None);

            // TODO figure out the other TODO in the WM_PAINT message
            //let _ = InvalidateRect(self.border_window, None, false);
        }

        Ok(())
    }

    pub fn animate_fade(&mut self, anim_elapsed: &time::Duration) {
        match self.active_color.clone() {
            Color::Solid(active_solid) => match self.inactive_color.clone() {
                Color::Solid(inactive_solid) => {
                    self.interpolate_solids(&active_solid, &inactive_solid, anim_elapsed);
                }
                Color::Gradient(inactive_gradient) => {
                    // If self.current_color is of type solid, we must convert it to a
                    // gradient here.
                    if let Color::Solid(current_solid) = self.current_color.clone() {
                        let mut solid_as_gradient = inactive_gradient.clone();
                        for i in 0..solid_as_gradient.gradient_stops.len() {
                            solid_as_gradient.gradient_stops[i].color = current_solid.color;
                        }
                        self.current_color = Color::Gradient(solid_as_gradient);
                    }

                    self.interpolate_gradients(
                        None,
                        Some(&active_solid),
                        Some(&inactive_gradient),
                        None,
                        anim_elapsed,
                    )
                }
            },
            Color::Gradient(active_gradient) => match self.inactive_color.clone() {
                Color::Solid(inactive_solid) => {
                    //let before = std::time::Instant::now();
                    // If self.current_color is of type solid, we must convert it to a
                    // gradient here.
                    if let Color::Solid(current_solid) = self.current_color.clone() {
                        let mut solid_as_gradient = active_gradient.clone();
                        for i in 0..solid_as_gradient.gradient_stops.len() {
                            solid_as_gradient.gradient_stops[i].color = current_solid.color;
                        }
                        self.current_color = Color::Gradient(solid_as_gradient);
                    }

                    self.interpolate_gradients(
                        Some(&active_gradient),
                        None,
                        None,
                        Some(&inactive_solid),
                        anim_elapsed,
                    )
                }
                Color::Gradient(inactive_gradient) => self.interpolate_gradients(
                    Some(&active_gradient),
                    None,
                    Some(&inactive_gradient),
                    None,
                    anim_elapsed,
                ),
            },
        }
    }

    pub fn interpolate_solids(
        &mut self,
        active_solid: &Solid,
        inactive_solid: &Solid,
        anim_elapsed: &time::Duration,
    ) {
        //let before = std::time::Instant::now();
        let Color::Solid(current_solid) = self.current_color.clone() else {
            // TODO do something else for the else statement like return
            // self.inactive_color.clone() or smth idk
            return;
        };

        let mut finished = false;
        self.current_color = Color::Solid(Solid {
            color: interpolate_d2d1_colors(
                &current_solid.color,
                &active_solid.color,
                &inactive_solid.color,
                anim_elapsed.as_secs_f32(),
                self.animation_speed,
                self.in_event_anim,
                &mut finished,
            ),
        });

        if finished {
            self.in_event_anim = ANIM_NONE;
        }
        //println!("time elapsed: {:?}", before.elapsed());
    }

    pub fn interpolate_gradients(
        &mut self,
        active_gradient: Option<&Gradient>,
        active_solid: Option<&Solid>,
        inactive_gradient: Option<&Gradient>,
        inactive_solid: Option<&Solid>,
        anim_elapsed: &time::Duration,
    ) {
        let Color::Gradient(current_gradient) = self.current_color.clone() else {
            // TODO i should return something else instead of returning
            return;
        };

        let mut all_finished = true;
        let mut gradient_stops: Vec<D2D1_GRADIENT_STOP> = Vec::new();
        for i in 0..current_gradient.gradient_stops.len() {
            let mut current_finished = false;

            // TODO we are making assumptions that the programmer (me) always passes one of the two
            // types of colors (either gradient or solid)
            let active_color = if let Some(gradient) = active_gradient {
                gradient.gradient_stops[i].color
            } else {
                active_solid.unwrap().color
            };
            let inactive_color = if let Some(gradient) = inactive_gradient {
                gradient.gradient_stops[i].color
            } else {
                inactive_solid.unwrap().color
            };

            let color = interpolate_d2d1_colors(
                &current_gradient.gradient_stops[i].color,
                &active_color,
                &inactive_color,
                anim_elapsed.as_secs_f32(),
                self.animation_speed,
                self.in_event_anim,
                &mut current_finished,
            );

            if !current_finished {
                all_finished = false;
            }

            // TODO here we are also making assumptions that I am a good programmer and will always
            // pass one of these two gradients
            let position = if let Some(active) = active_gradient {
                active.gradient_stops[i].position
            } else {
                inactive_gradient.unwrap().gradient_stops[i].position
            };

            let stop = D2D1_GRADIENT_STOP { color, position };
            gradient_stops.push(stop);
        }

        // TODO again we are assuming I am not an idiot
        let direction = if let Some(active) = active_gradient {
            active.direction.clone()
        } else {
            inactive_gradient.unwrap().direction.clone()
        };

        // TODO, move this down below if all_finished and add self.current_color =
        // self.active_color.clone to the match statement. That way, we can avoid
        // setting current_color twice when we are going from active to inactive. But
        // for now, I have it like this to test whether I am getting the proper colors
        // from interpolate_d2d1_colors.
        self.current_color = Color::Gradient(Gradient {
            gradient_stops,
            direction,
        });

        if all_finished {
            match self.in_event_anim {
                ANIM_FADE_TO_ACTIVE => {
                    // TODO uncomment these after you have finished testing whether
                    // interpolate_d2d1_colors works correctly
                    //self.current_color = self.active_color.clone()
                }
                ANIM_FADE_TO_INACTIVE => {
                    //self.current_color = self.inactive_color.clone()
                }
                _ => {}
            }
            self.in_event_anim = ANIM_NONE;
        }
    }

    // When CreateWindowExW is called, we can optionally pass a value to its LPARAM field which will
    // get sent to the window process on creation. In our code, we've passed a pointer to the
    // WindowBorder structure during the window creation process, and here we are getting that pointer
    // and attaching it to the window using SetWindowLongPtrW.
    pub unsafe extern "system" fn s_wnd_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let mut border_pointer: *mut WindowBorder = GetWindowLongPtrW(window, GWLP_USERDATA) as _;

        if border_pointer.is_null() && message == WM_CREATE {
            let create_struct: *mut CREATESTRUCTW = lparam.0 as *mut _;
            border_pointer = (*create_struct).lpCreateParams as *mut _;
            SetWindowLongPtrW(window, GWLP_USERDATA, border_pointer as _);
        }
        match !border_pointer.is_null() {
            true => Self::wnd_proc(&mut *border_pointer, window, message, wparam, lparam),
            false => DefWindowProcW(window, message, wparam, lparam),
        }
    }

    pub unsafe fn wnd_proc(
        &mut self,
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            // EVENT_OBJECT_LOCATIONCHANGE
            WM_APP_LOCATIONCHANGE => {
                if self.pause {
                    return LRESULT(0);
                }

                // TODO I could probably move some of this message's code into a new message for
                // EVENT_SYSTEM_MOVESIZESTART and MOVESIZEEND but the relevant code doesn't seem to
                // eat up much CPU anyways
                if !has_native_border(self.tracking_window) {
                    let _ = self.update_position(Some(SWP_HIDEWINDOW));
                    return LRESULT(0);
                }

                let flags = if !is_window_visible(self.border_window) {
                    Some(SWP_SHOWWINDOW)
                } else {
                    None
                };

                let old_rect = self.window_rect;
                let _ = self.update_window_rect();
                let _ = self.update_position(flags);

                // TODO When a window is minimized, all four points of the rect go way below 0. For
                // some reason, after unminimizing/restoring, render() will sometimes render at
                // this minimized size. For now, I just do self.window_rect = old_rect to fix that.
                if !is_rect_visible(&self.window_rect) {
                    self.window_rect = old_rect;
                } else if !are_rects_same_size(&self.window_rect, &old_rect) {
                    // Only re-render the border when its size changes
                    let _ = self.render();
                }
            }
            // EVENT_OBJECT_REORDER
            WM_APP_REORDER => {
                if self.pause {
                    return LRESULT(0);
                }

                let _ = self.update_color();
                let _ = self.update_position(None);
                let _ = self.render();
            }
            // EVENT_OBJECT_SHOW / EVENT_OBJECT_UNCLOAKED
            WM_APP_SHOWUNCLOAKED => {
                // With GlazeWM, if I switch to another workspace while a window is minimized and
                // switch back, then we will receive this message even though the window is not yet
                // visible. And, the window rect will be all weird. So, we apply the following fix.
                let old_rect = self.window_rect;
                let _ = self.update_window_rect();
                if !is_rect_visible(&self.window_rect) {
                    self.window_rect = old_rect;
                    return LRESULT(0);
                }

                if has_native_border(self.tracking_window) {
                    let _ = self.update_color();
                    let _ = self.update_position(Some(SWP_SHOWWINDOW));
                    let _ = self.render();
                }
                self.pause = false;
            }
            // EVENT_OBJECT_HIDE / EVENT_OBJECT_CLOAKED / EVENT_OBJECT_MINIMIZESTART
            WM_APP_HIDECLOAKED | WM_APP_MINIMIZESTART => {
                let _ = self.update_position(Some(SWP_HIDEWINDOW));
                self.pause = true;
            }
            // EVENT_SYSTEM_MINIMIZEEND
            // When a window is about to be unminimized, hide the border and let the thread sleep
            // to wait for the window animation to finish, then show the border.
            WM_APP_MINIMIZEEND => {
                thread::sleep(time::Duration::from_millis(self.unminimize_delay));

                if has_native_border(self.tracking_window) {
                    let _ = self.update_color();
                    let _ = self.update_window_rect();
                    let _ = self.update_position(Some(SWP_SHOWWINDOW));
                    let _ = self.render();
                }
                self.pause = false;
            }
            WM_APP_EVENTANIM => {
                match wparam.0 as i32 {
                    ANIM_FADE_TO_ACTIVE | ANIM_FADE_TO_INACTIVE => {
                        if self.animations.contains(&AnimationType::Fade) {
                            // TODO idk if converting usize to i32 is a good idea but we don't use
                            // any large integers so I think it's fine.
                            self.in_event_anim = wparam.0 as i32;
                        }
                    }
                    _ => {}
                }
            }
            WM_TIMER => {
                if self.pause {
                    return LRESULT(0);
                }

                let anim_elapsed = self
                    .last_anim_time
                    .unwrap_or(time::Instant::now())
                    .elapsed();
                let render_elapsed = self
                    .last_render_time
                    .unwrap_or(time::Instant::now())
                    .elapsed();

                self.last_anim_time = Some(time::Instant::now());

                for anim_type in self.animations.iter() {
                    match anim_type {
                        AnimationType::Spiral => {
                            let center_x = (self.window_rect.right - self.window_rect.left) / 2;
                            let center_y = (self.window_rect.bottom - self.window_rect.top) / 2;
                            self.brush_properties.transform = self.brush_properties.transform
                                * Matrix3x2::rotation(
                                    self.animation_speed * anim_elapsed.as_secs_f32(),
                                    center_x as f32,
                                    center_y as f32,
                                );
                        }
                        AnimationType::Fade => {}
                    }
                }

                //let before = std::time::Instant::now();
                match self.in_event_anim {
                    ANIM_FADE_TO_ACTIVE | ANIM_FADE_TO_INACTIVE => self.animate_fade(&anim_elapsed),
                    _ => {}
                }
                //println!("time elapsed: {:?}", before.elapsed());

                if render_elapsed >= time::Duration::from_millis((1000 / self.animation_fps) as u64)
                {
                    let _ = self.render();
                }
            }
            // TODO if we call InvalidateRect within the render() function, then we get brought to
            // this WM_PAINT message. And if we call self.render() again within this message, it
            // fixes an issue with task manager reporting high GPU usage on my pc but makes it
            // worse my laptop. Figure out why. Hopefully it's just a bug with task manager because
            // logically, this fix should not work.
            WM_PAINT => {
                //println!("window: {:?}", window);

                //let _ = self.render();
                let _ = ValidateRect(window, None);
            }
            WM_DESTROY => {
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                PostQuitMessage(0);
            }
            // Ignore these window position messages
            WM_WINDOWPOSCHANGING | WM_WINDOWPOSCHANGED => {}
            _ => {
                return DefWindowProcW(window, message, wparam, lparam);
            }
        }
        LRESULT(0)
    }
}

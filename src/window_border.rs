// TODO Add result handling. There's so many let _ =
use crate::anim_timer::AnimationTimer;
use crate::colors::*;
use crate::utils::*;
use std::collections::HashMap;
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
    pub animations: HashMap<AnimationType, f32>,
    pub in_event_anim: i32,
    pub animation_fps: i32,
    pub last_render_time: Option<time::Instant>,
    pub last_anim_time: Option<time::Instant>,
    pub anim_timer: Option<AnimationTimer>,
    pub spiral_anim_angle: f32,
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
                error!("Could not set layered window attributes!");
            }
            if SetLayeredWindowAttributes(self.border_window, COLORREF(0x00000000), 255, LWA_ALPHA)
                .is_err()
            {
                error!("Could not set layered window attributes!");
            }

            let _ = self.create_render_targets();

            match self.animations.contains_key(&AnimationType::Fade) && init_delay != 0 {
                true => {
                    // Reset last_anim_time here to make interpolate_d2d1_alphas work correctly
                    self.last_anim_time = Some(time::Instant::now());
                    self.prepare_fade_to_visible();
                }
                false => {
                    let _ = self.update_color();
                }
            }

            let _ = self.update_window_rect();

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

            self.set_anim_timer();

            let mut message = MSG::default();
            while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            debug!("Exiting border thread for {:?}!", self.tracking_window);
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
            warn!("Could not get window rect. This is normal for elevated/admin windows.");
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
                warn!("Could not set window position! This is normal for elevated/admin windows.");
                let _ = ShowWindow(self.border_window, SW_HIDE);
                self.pause = true;
            }
        }
        Ok(())
    }

    // TODO this is kinda scuffed to work with fade animations
    pub fn update_color(&mut self) -> Result<()> {
        if self.animations.contains_key(&AnimationType::Fade) {
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

        let width = self.border_width as f32;
        let offset = self.border_offset as f32;
        self.rounded_rect.rect = D2D_RECT_F {
            left: width / 2.0 - offset,
            top: width / 2.0 - offset,
            right: (self.window_rect.right - self.window_rect.left) as f32 - width / 2.0 + offset,
            bottom: (self.window_rect.bottom - self.window_rect.top) as f32 - width / 2.0 + offset,
        };

        unsafe {
            let _ = render_target.Resize(&pixel_size);

            let Some(brush) = self.current_color.create_brush(
                render_target,
                &self.window_rect,
                &self.brush_properties,
            ) else {
                return Ok(());
            };

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

    pub fn animate_fade(&mut self, anim_elapsed: &time::Duration, anim_speed: f32) {
        if let Color::Solid(_) = self.active_color {
            if let Color::Solid(_) = self.inactive_color {
                // If both active and inactive color are solids, use interpolate_solids
                self.interpolate_solids(anim_elapsed, anim_speed);
            }
        } else {
            self.interpolate_gradients(anim_elapsed, anim_speed);
        }
    }

    pub fn interpolate_solids(&mut self, anim_elapsed: &time::Duration, anim_speed: f32) {
        let Color::Solid(current_solid) = self.current_color.clone() else {
            error!("Could not convert current_color for interpolation");
            return;
        };
        let Color::Solid(active_solid) = self.active_color.clone() else {
            error!("Could not convert active_color for interpolation");
            return;
        };
        let Color::Solid(inactive_solid) = self.inactive_color.clone() else {
            error!("Could not convert inactive_color for interpolation");
            return;
        };

        let mut finished = false;
        let color = match self.in_event_anim {
            ANIM_FADE_TO_VISIBLE => {
                let end_color = match is_window_visible(self.tracking_window) {
                    true => &active_solid.color,
                    false => &inactive_solid.color,
                };

                interpolate_d2d1_alphas(
                    &current_solid.color,
                    end_color,
                    anim_elapsed.as_secs_f32(),
                    anim_speed,
                    &mut finished,
                )
            }
            ANIM_FADE_TO_ACTIVE | ANIM_FADE_TO_INACTIVE => {
                let (start_color, end_color) = match self.in_event_anim {
                    ANIM_FADE_TO_ACTIVE => (&inactive_solid.color, &active_solid.color),
                    ANIM_FADE_TO_INACTIVE => (&active_solid.color, &inactive_solid.color),
                    _ => return,
                };

                interpolate_d2d1_colors(
                    &current_solid.color,
                    start_color,
                    end_color,
                    anim_elapsed.as_secs_f32(),
                    anim_speed,
                    &mut finished,
                )
            }
            _ => return,
        };

        if finished {
            self.in_event_anim = ANIM_NONE;
        } else {
            self.current_color = Color::Solid(Solid { color });
        }
    }

    pub fn interpolate_gradients(&mut self, anim_elapsed: &time::Duration, anim_speed: f32) {
        let current_gradient = match self.current_color.clone() {
            Color::Gradient(gradient) => gradient,
            Color::Solid(solid) => {
                // If current_color is not a gradient, that means at least one of active or inactive
                // color must be solid, so only one of these if let statements should evaluate true
                let gradient = if let Color::Gradient(active_gradient) = self.active_color.clone() {
                    active_gradient
                } else if let Color::Gradient(inactive_gradient) = self.inactive_color.clone() {
                    inactive_gradient
                } else {
                    error!("Could not convert active_color or inactive_color for interpolation");
                    return;
                };

                // Convert current_color to a gradient
                let mut solid_as_gradient = gradient.clone();
                for i in 0..solid_as_gradient.gradient_stops.len() {
                    solid_as_gradient.gradient_stops[i].color = solid.color;
                }
                solid_as_gradient
            }
        };

        let mut all_finished = true;
        let mut gradient_stops: Vec<D2D1_GRADIENT_STOP> = Vec::new();
        for i in 0..current_gradient.gradient_stops.len() {
            let mut current_finished = false;

            let active_color = match self.active_color.clone() {
                Color::Gradient(gradient) => gradient.gradient_stops[i].color,
                Color::Solid(solid) => solid.color,
            };

            let inactive_color = match self.inactive_color.clone() {
                Color::Gradient(gradient) => gradient.gradient_stops[i].color,
                Color::Solid(solid) => solid.color,
            };

            let color = match self.in_event_anim {
                ANIM_FADE_TO_VISIBLE => {
                    let end_color = match is_window_visible(self.tracking_window) {
                        true => &active_color,
                        false => &inactive_color,
                    };

                    interpolate_d2d1_alphas(
                        &current_gradient.gradient_stops[i].color,
                        end_color,
                        anim_elapsed.as_secs_f32(),
                        anim_speed,
                        &mut current_finished,
                    )
                }
                ANIM_FADE_TO_ACTIVE | ANIM_FADE_TO_INACTIVE => {
                    let (start_color, end_color) = match self.in_event_anim {
                        ANIM_FADE_TO_ACTIVE => (&inactive_color, &active_color),
                        ANIM_FADE_TO_INACTIVE => (&active_color, &inactive_color),
                        _ => return,
                    };

                    interpolate_d2d1_colors(
                        &current_gradient.gradient_stops[i].color,
                        start_color,
                        end_color,
                        anim_elapsed.as_secs_f32(),
                        anim_speed,
                        &mut current_finished,
                    )
                }
                _ => return,
            };

            if !current_finished {
                all_finished = false;
            }

            // TODO currently this works well because users cannot adjust the positions of the
            // gradient stops, so both inactive and active gradients will have the same positions,
            // but this might need to be interpolated if we add position configuration.
            let position = current_gradient.gradient_stops[i].position;

            let stop = D2D1_GRADIENT_STOP { color, position };
            gradient_stops.push(stop);
        }

        let mut direction = current_gradient.direction;

        // Interpolate direction if both active and inactive are gradients
        // TODO maybe find a better way to handle ANIM_FADE_TO_VISIBLE here
        if self.in_event_anim != ANIM_FADE_TO_VISIBLE {
            if let Color::Gradient(inactive_gradient) = self.inactive_color.clone() {
                if let Color::Gradient(active_gradient) = self.active_color.clone() {
                    let (start_direction, end_direction) = match self.in_event_anim {
                        ANIM_FADE_TO_ACTIVE => {
                            (&inactive_gradient.direction, &active_gradient.direction)
                        }
                        ANIM_FADE_TO_INACTIVE => {
                            (&active_gradient.direction, &inactive_gradient.direction)
                        }
                        _ => return,
                    };

                    direction = interpolate_direction(
                        &direction,
                        start_direction,
                        end_direction,
                        anim_elapsed.as_secs_f32(),
                        anim_speed,
                    );
                }
            }
        }

        if all_finished {
            match self.in_event_anim {
                ANIM_FADE_TO_ACTIVE => self.current_color = self.active_color.clone(),
                ANIM_FADE_TO_INACTIVE => self.current_color = self.inactive_color.clone(),
                ANIM_FADE_TO_VISIBLE => {
                    self.current_color = match is_active_window(self.tracking_window) {
                        true => self.active_color.clone(),
                        false => self.inactive_color.clone(),
                    }
                }
                _ => {}
            }
            self.in_event_anim = ANIM_NONE;
        } else {
            self.current_color = Color::Gradient(Gradient {
                gradient_stops,
                direction,
            });
        }
    }

    pub fn prepare_fade_to_visible(&mut self) {
        self.current_color = if is_active_window(self.tracking_window) {
            self.active_color.clone()
        } else {
            self.inactive_color.clone()
        };

        if let Color::Gradient(mut current_gradient) = self.current_color.clone() {
            let mut gradient_stops: Vec<D2D1_GRADIENT_STOP> = Vec::new();
            for i in 0..current_gradient.gradient_stops.len() {
                current_gradient.gradient_stops[i].color.a = 0.0;
                let color = current_gradient.gradient_stops[i].color;
                let position = current_gradient.gradient_stops[i].position;
                gradient_stops.push(D2D1_GRADIENT_STOP { color, position });
            }

            let direction = current_gradient.direction;

            self.current_color = Color::Gradient(Gradient {
                gradient_stops,
                direction,
            })
        } else if let Color::Solid(mut current_solid) = self.current_color.clone() {
            current_solid.color.a = 0.0;
            let color = current_solid.color;

            self.current_color = Color::Solid(Solid { color });
        }

        self.in_event_anim = ANIM_FADE_TO_VISIBLE;
    }

    pub fn set_anim_timer(&mut self) {
        if !self.animations.is_empty() && self.anim_timer.is_none() {
            let timer_duration = (1000.0 / self.animation_fps as f32) as u64;
            self.anim_timer = Some(AnimationTimer::start(self.border_window, timer_duration));
        }
    }

    pub fn destroy_anim_timer(&mut self) {
        if let Some(anim_timer) = self.anim_timer.as_mut() {
            anim_timer.stop();
            self.anim_timer = None;
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

                if !is_window_visible(self.border_window) {
                    let _ = ShowWindow(self.border_window, SW_SHOWNA);
                }

                let old_rect = self.window_rect;
                let _ = self.update_window_rect();
                let _ = self.update_position(None);

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

                self.set_anim_timer();

                self.pause = false;
            }
            // EVENT_OBJECT_HIDE / EVENT_OBJECT_CLOAKED / EVENT_OBJECT_MINIMIZESTART
            WM_APP_HIDECLOAKED | WM_APP_MINIMIZESTART => {
                let _ = self.update_position(Some(SWP_HIDEWINDOW));

                self.destroy_anim_timer();

                self.pause = true;
            }
            // EVENT_SYSTEM_MINIMIZEEND
            // When a window is about to be unminimized, hide the border and let the thread sleep
            // to wait for the window animation to finish, then show the border.
            WM_APP_MINIMIZEEND => {
                thread::sleep(time::Duration::from_millis(self.unminimize_delay));

                if has_native_border(self.tracking_window) {
                    match self.animations.contains_key(&AnimationType::Fade)
                        && self.unminimize_delay != 0
                    {
                        true => {
                            // Reset last_anim_time here because otherwise, anim_elapsed will be
                            // too large due to being paused and interpolation won't work correctly
                            self.last_anim_time = Some(time::Instant::now());
                            self.prepare_fade_to_visible();
                        }
                        false => {
                            let _ = self.update_color();
                        }
                    }
                    let _ = self.update_window_rect();
                    let _ = self.update_position(Some(SWP_SHOWWINDOW));
                    let _ = self.render();
                }

                self.set_anim_timer();

                self.pause = false;
            }
            WM_APP_EVENTANIM => {
                match wparam.0 as i32 {
                    ANIM_FADE_TO_ACTIVE | ANIM_FADE_TO_INACTIVE => {
                        if self.animations.contains_key(&AnimationType::Fade) {
                            // TODO idk if converting usize to i32 is a good idea but we don't use
                            // any large integers so I think it's fine.
                            self.in_event_anim = wparam.0 as i32;
                        }
                    }
                    _ => {}
                }
            }
            WM_APP_ANIMATE => {
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

                let mut update = false;

                for (anim_type, anim_speed) in self.animations.iter() {
                    match anim_type {
                        AnimationType::Spiral => {
                            if self.spiral_anim_angle >= 360.0 {
                                self.spiral_anim_angle -= 360.0;
                            }
                            self.spiral_anim_angle +=
                                (anim_elapsed.as_secs_f32() * anim_speed * 2.0).min(359.0);

                            let center_x = (self.window_rect.right - self.window_rect.left) / 2;
                            let center_y = (self.window_rect.bottom - self.window_rect.top) / 2;

                            self.brush_properties.transform = Matrix3x2::rotation(
                                self.spiral_anim_angle,
                                center_x as f32,
                                center_y as f32,
                            );

                            update = true;
                        }
                        AnimationType::Fade => {}
                    }
                }

                match self.in_event_anim {
                    ANIM_FADE_TO_ACTIVE | ANIM_FADE_TO_INACTIVE | ANIM_FADE_TO_VISIBLE => {
                        let anim_speed =
                            self.animations.get(&AnimationType::Fade).unwrap_or(&200.0);
                        // divide anim_speed by 15 just cuz otherwise it's too fast lol
                        self.animate_fade(&anim_elapsed, *anim_speed / 15.0);

                        update = true;
                    }
                    _ => {}
                }

                //println!("time since last render: {:?}", render_elapsed.as_secs_f32());

                let interval = 1.0 / self.animation_fps as f32;
                let diff = render_elapsed.as_secs_f32() - interval;
                if update && (diff.abs() <= 0.001 || diff >= 0.0) {
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
                self.destroy_anim_timer();

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

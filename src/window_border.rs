// TODO Add result handling. There's so many let _ =
use crate::anim_timer::AnimationTimer;
use crate::animations;
use crate::animations::*;
use crate::colors::*;
use crate::utils::*;
use std::ptr;
use std::sync::{LazyLock, OnceLock};
use std::thread;
use std::time;
use windows::core::{w, PCWSTR};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{
    COLORREF, FALSE, HINSTANCE, HWND, LPARAM, LRESULT, RECT, TRUE, WPARAM,
};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Brush, ID2D1Factory, ID2D1HwndRenderTarget,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BRUSH_PROPERTIES, D2D1_FACTORY_TYPE_MULTI_THREADED,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_IMMEDIATELY,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::Dwm::{
    DwmEnableBlurBehindWindow, DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS,
    DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows::Win32::Graphics::Gdi::{CreateRectRgn, ValidateRect};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetSystemMetrics, GetWindow,
    GetWindowLongPtrW, PostQuitMessage, SetLayeredWindowAttributes, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, TranslateMessage, CREATESTRUCTW, GWLP_USERDATA, GW_HWNDPREV,
    HWND_TOP, LWA_ALPHA, LWA_COLORKEY, MSG, SET_WINDOW_POS_FLAGS, SM_CXVIRTUALSCREEN,
    SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOREDRAW, SWP_NOSENDCHANGING, SWP_NOZORDER, SWP_SHOWWINDOW,
    SW_SHOWNA, WM_CREATE, WM_NCDESTROY, WM_PAINT, WM_WINDOWPOSCHANGED, WM_WINDOWPOSCHANGING,
    WS_DISABLED, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
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
    pub animations: Animations,
    pub event_anim: i32,
    pub last_render_time: Option<time::Instant>,
    pub last_anim_time: Option<time::Instant>,
    pub anim_timer: Option<AnimationTimer>,
    // Delay border visbility when tracking window is in unminimize animation
    pub unminimize_delay: u64,
    // This is to pause the border from doing anything when it doesn't need to
    pub pause: bool,
    pub is_active_window: bool,
}

impl WindowBorder {
    pub fn create_border_window(&mut self, hinstance: HINSTANCE) -> windows::core::Result<()> {
        unsafe {
            let self_title = format!("{}{}", "tacky-", get_window_title(self.tracking_window));
            let mut string: Vec<u16> = self_title.encode_utf16().collect();
            string.push(0);

            self.border_window = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                w!("tacky-border"),
                PCWSTR::from_raw(string.as_ptr()),
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

    pub fn init(&mut self, initialize_delay: u64) -> Result<(), ()> {
        // Delay the border while the tracking window is in its creation animation
        thread::sleep(time::Duration::from_millis(initialize_delay));

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

            self.is_active_window = is_active_window(self.tracking_window);

            self.animations.current = match self.is_active_window {
                true => self.animations.active.clone(),
                false => self.animations.inactive.clone(),
            };

            let _ = self.update_color(Some(initialize_delay));

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

    pub fn create_render_targets(&mut self) -> Result<(), ()> {
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

    pub fn update_window_rect(&mut self) -> Result<(), ()> {
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
                self.destroy_anim_timer();
                PostQuitMessage(0);
            }
        }

        self.window_rect.top -= self.border_width;
        self.window_rect.left -= self.border_width;
        self.window_rect.right += self.border_width;
        self.window_rect.bottom += self.border_width;

        Ok(())
    }

    pub fn update_position(&mut self, c_flags: Option<SET_WINDOW_POS_FLAGS>) -> Result<(), ()> {
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
                self.destroy_anim_timer();
                PostQuitMessage(0);
            }
        }
        Ok(())
    }

    // TODO this is kinda scuffed to work with fade animations
    pub fn update_color(&mut self, check_delay: Option<u64>) -> Result<(), ()> {
        match self.animations.current.contains_key(&AnimationType::Fade) && check_delay != Some(0) {
            true => {
                self.event_anim = ANIM_FADE;
            }
            false => {
                // TODO needing this here is kinda jank but it works for now
                self.animations.fade_progress = match self.is_active_window {
                    true => 1.0,
                    false => 0.0,
                };
                let (top_color, bottom_color) = match self.is_active_window {
                    true => (&mut self.active_color, &mut self.inactive_color),
                    false => (&mut self.inactive_color, &mut self.active_color),
                };
                top_color.set_opacity(1.0);
                bottom_color.set_opacity(0.0);
            }
        }

        Ok(())
    }

    pub fn render(&mut self) -> Result<(), ()> {
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

            // TODO wtf is this mess..
            let active_opacity = self.active_color.get_opacity();
            let inactive_opacity = self.inactive_color.get_opacity();

            let (bottom_opacity, top_opacity) = match self.is_active_window {
                true => (inactive_opacity, active_opacity),
                false => (active_opacity, inactive_opacity),
            };

            let (bottom_color, top_color) = match self.is_active_window {
                true => (&self.inactive_color, &self.active_color),
                false => (&self.active_color, &self.inactive_color),
            };

            render_target.BeginDraw();
            render_target.Clear(None);

            if bottom_opacity > 0.0 {
                let Some(brush) = bottom_color.create_brush(
                    render_target,
                    &self.window_rect,
                    &self.brush_properties,
                ) else {
                    return Ok(());
                };
                self.draw_rectangle(render_target, &brush);
            }
            if top_opacity > 0.0 {
                let Some(brush) = top_color.create_brush(
                    render_target,
                    &self.window_rect,
                    &self.brush_properties,
                ) else {
                    return Ok(());
                };
                self.draw_rectangle(render_target, &brush);
            }

            let _ = render_target.EndDraw(None, None);
        }

        Ok(())
    }

    pub fn draw_rectangle(&self, render_target: &ID2D1HwndRenderTarget, brush: &ID2D1Brush) {
        unsafe {
            match self.border_radius {
                0.0 => render_target.DrawRectangle(
                    &self.rounded_rect.rect,
                    brush,
                    self.border_width as f32,
                    None,
                ),
                _ => render_target.DrawRoundedRectangle(
                    &self.rounded_rect,
                    brush,
                    self.border_width as f32,
                    None,
                ),
            }
        }
    }

    pub fn set_anim_timer(&mut self) {
        if (!self.animations.active.is_empty() || !self.animations.inactive.is_empty())
            && self.anim_timer.is_none()
        {
            let timer_duration = (1000.0 / self.animations.fps as f32) as u64;
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
            true => (*border_pointer).wnd_proc(window, message, wparam, lparam),
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

                // For apps like firefox, when you hover over a tab, a popup window spawns that
                // changes the z-order and causes the border to sit under the tracking window. To
                // remedy that, we just re-update the position/z-order when windows are reordered.
                let _ = self.update_position(None);
            }
            // EVENT_OBJECT_FOCUS
            WM_APP_FOCUS => {
                self.is_active_window = is_active_window(self.tracking_window);

                // Update the current animations list
                self.animations.current = match self.is_active_window {
                    true => self.animations.active.clone(),
                    false => self.animations.inactive.clone(),
                };

                let _ = self.update_color(None);
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
                    //let _ = self.update_color(Some(0));
                    let _ = self.update_position(Some(SWP_SHOWWINDOW));
                    let _ = self.render();
                }

                self.set_anim_timer();

                self.pause = false;
            }
            // EVENT_OBJECT_HIDE / EVENT_OBJECT_CLOAKED
            WM_APP_HIDECLOAKED => {
                let _ = self.update_position(Some(SWP_HIDEWINDOW));

                self.destroy_anim_timer();

                self.pause = true;
            }
            // EVENT_OBJECT_MINIMIZESTART
            WM_APP_MINIMIZESTART => {
                let _ = self.update_position(Some(SWP_HIDEWINDOW));

                // TODO this is scuffed to work with fade animations
                self.active_color.set_opacity(0.0);
                self.inactive_color.set_opacity(0.0);

                self.destroy_anim_timer();

                self.pause = true;
            }
            // EVENT_SYSTEM_MINIMIZEEND
            // When a window is about to be unminimized, hide the border and let the thread sleep
            // to wait for the window animation to finish, then show the border.
            WM_APP_MINIMIZEEND => {
                thread::sleep(time::Duration::from_millis(self.unminimize_delay));

                // TODO scuffed to work with fade animations. When the window is minimized,
                // last_anim_time stops updating so when we go back to unminimize it,
                // last_anim_time.elapsed() will be large. So, we have to reset it here.
                self.last_anim_time = Some(time::Instant::now());

                if has_native_border(self.tracking_window) {
                    let _ = self.update_color(Some(self.unminimize_delay));
                    let _ = self.update_window_rect();
                    let _ = self.update_position(Some(SWP_SHOWWINDOW));
                    let _ = self.render();
                }

                self.set_anim_timer();

                self.pause = false;
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

                for (anim_type, anim_speed) in self.animations.current.clone().iter() {
                    match anim_type {
                        AnimationType::Spiral => {
                            // multiply anim_speed by 2.0 otherwise it's too slow lol
                            animations::animate_spiral(self, &anim_elapsed, *anim_speed * 2.0);
                            update = true;
                        }
                        AnimationType::ReverseSpiral => {
                            // multiply anim_speed by 2.0 otherwise it's too slow lol
                            animations::animate_reverse_spiral(
                                self,
                                &anim_elapsed,
                                *anim_speed * 2.0,
                            );
                            update = true;
                        }
                        AnimationType::Fade => {}
                    }
                }

                if self.event_anim == ANIM_FADE {
                    let anim_speed = self
                        .animations
                        .current
                        .get(&AnimationType::Fade)
                        .unwrap_or(&200.0);

                    // divide anim_speed by 20 just cuz otherwise it's too fast lol
                    animations::animate_fade(self, &anim_elapsed, *anim_speed / 20.0);
                    update = true;
                }

                let interval = 1.0 / self.animations.fps as f32;
                let diff = render_elapsed.as_secs_f32() - interval;
                if update && (diff.abs() <= 0.001 || diff >= 0.0) {
                    let _ = self.render();
                }
            }
            WM_PAINT => {
                let _ = ValidateRect(window, None);
            }
            WM_NCDESTROY => {
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

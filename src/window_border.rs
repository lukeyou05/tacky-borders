use crate::animations::{self, *};
use crate::colors::*;
use crate::utils::*;
use crate::BORDERS;
use anyhow::{anyhow, Context};
use std::ptr;
use std::sync::LazyLock;
use std::thread;
use std::time;
use windows::core::{w, PCWSTR};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{
    COLORREF, D2DERR_RECREATE_TARGET, FALSE, HINSTANCE, HWND, LPARAM, LRESULT, RECT, TRUE, WPARAM,
};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Brush, ID2D1Factory, ID2D1HwndRenderTarget,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BRUSH_PROPERTIES, D2D1_FACTORY_TYPE_MULTI_THREADED,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_IMMEDIATELY,
    D2D1_PRESENT_OPTIONS_RETAIN_CONTENTS, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_ROUNDED_RECT,
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
    SetWindowPos, TranslateMessage, CREATESTRUCTW, CW_USEDEFAULT, GWLP_USERDATA, GW_HWNDPREV,
    HWND_TOP, LWA_ALPHA, MSG, SET_WINDOW_POS_FLAGS, SM_CXVIRTUALSCREEN, SWP_HIDEWINDOW,
    SWP_NOACTIVATE, SWP_NOREDRAW, SWP_NOSENDCHANGING, SWP_NOZORDER, SWP_SHOWWINDOW, WM_CREATE,
    WM_NCDESTROY, WM_PAINT, WM_WINDOWPOSCHANGED, WM_WINDOWPOSCHANGING, WS_DISABLED, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

static RENDER_FACTORY: LazyLock<ID2D1Factory> = unsafe {
    LazyLock::new(|| {
        match D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_MULTI_THREADED, None) {
            Ok(factory) => factory,
            Err(e) => {
                // Not sure how I can recover from this error so I'm just going to panic
                error!("could not create ID2D1Factory: {e}");
                panic!("could not create ID2D1Factory: {e}");
            }
        }
    })
};

#[derive(Debug, Default)]
pub struct WindowBorder {
    pub border_window: HWND,
    pub tracking_window: HWND,
    pub is_active_window: bool,
    pub window_rect: RECT,
    pub border_width: i32,
    pub border_offset: i32,
    pub border_radius: f32,
    pub render_target: Option<ID2D1HwndRenderTarget>,
    pub rounded_rect: D2D1_ROUNDED_RECT,
    pub active_color: Color,
    pub inactive_color: Color,
    pub animations: Animations,
    pub last_render_time: Option<time::Instant>,
    pub last_anim_time: Option<time::Instant>,
    pub initialize_delay: u64,
    pub unminimize_delay: u64,
    pub pause: bool,
}

impl WindowBorder {
    pub fn create_border_window(&mut self, hinstance: HINSTANCE) -> windows::core::Result<()> {
        let title: Vec<u16> = format!(
            "tacky-border | {} | {:?}\0",
            get_window_title(self.tracking_window),
            self.tracking_window
        )
        .encode_utf16()
        .collect();

        unsafe {
            self.border_window = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                w!("border"),
                PCWSTR(title.as_ptr()),
                WS_POPUP | WS_DISABLED,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None,
                hinstance,
                Some(ptr::addr_of!(*self) as _),
            )?;
        }

        Ok(())
    }

    pub fn init(&mut self) -> anyhow::Result<()> {
        // Delay the border while the tracking window is in its creation animation
        thread::sleep(time::Duration::from_millis(self.initialize_delay));

        unsafe {
            // Make the window transparent (stole the code from PowerToys; dunno how it works).
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
            // These functions below are pretty important, so if they fail, just return an Error
            DwmEnableBlurBehindWindow(self.border_window, &bh)
                .context("could not make window transparent")?;

            SetLayeredWindowAttributes(self.border_window, COLORREF(0x00000000), 255, LWA_ALPHA)
                .context("could not set LWA_ALPHA")?;

            self.create_render_resources()
                .context("could not create render resources in init()")?;

            // This is used a lot, so I think it's better to store it into its own variable instead
            // of constantly making the same Windows API calls
            self.is_active_window = is_active_window(self.tracking_window);

            self.update_color(Some(self.initialize_delay)).log_if_err();
            self.update_window_rect().log_if_err();

            if has_native_border(self.tracking_window) {
                self.update_position(Some(SWP_SHOWWINDOW)).log_if_err();
                self.render().log_if_err();

                // TODO sometimes, the border doesn't show up on the first try. So, we just wait
                // 5ms and call render() again. This seems to be an issue with the visibility of
                // the window itself.
                thread::sleep(time::Duration::from_millis(5));
                self.update_position(Some(SWP_SHOWWINDOW)).log_if_err();
                self.render().log_if_err();
            }

            animations::set_timer_if_anims_enabled(self);

            let mut message = MSG::default();
            while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            debug!("exiting border thread for {:?}!", self.tracking_window);
        }

        Ok(())
    }

    fn create_render_resources(&mut self) -> anyhow::Result<()> {
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
            presentOptions: D2D1_PRESENT_OPTIONS_RETAIN_CONTENTS | D2D1_PRESENT_OPTIONS_IMMEDIATELY,
        };
        let brush_properties = D2D1_BRUSH_PROPERTIES {
            opacity: 1.0,
            transform: Matrix3x2::identity(),
        };

        self.rounded_rect = D2D1_ROUNDED_RECT {
            rect: Default::default(),
            radiusX: self.border_radius,
            radiusY: self.border_radius,
        };

        unsafe {
            let render_target = RENDER_FACTORY.CreateHwndRenderTarget(
                &render_target_properties,
                &hwnd_render_target_properties,
            )?;

            render_target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

            self.active_color
                .create_brush(&render_target, &self.window_rect, &brush_properties)
                .log_if_err();
            self.inactive_color
                .create_brush(&render_target, &self.window_rect, &brush_properties)
                .log_if_err();

            self.render_target = Some(render_target);
        }

        Ok(())
    }

    fn update_window_rect(&mut self) -> anyhow::Result<()> {
        if let Err(e) = unsafe {
            DwmGetWindowAttribute(
                self.tracking_window,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                ptr::addr_of_mut!(self.window_rect) as _,
                size_of::<RECT>() as u32,
            )
            .context(format!(
                "could not get window rect for {:?}",
                self.tracking_window
            ))
        } {
            self.exit_border_thread();
            return Err(e);
        }

        // Increase the size of the window rect to make space for the border
        self.window_rect.top -= self.border_width;
        self.window_rect.left -= self.border_width;
        self.window_rect.right += self.border_width;
        self.window_rect.bottom += self.border_width;

        Ok(())
    }

    fn update_position(&mut self, other_flags: Option<SET_WINDOW_POS_FLAGS>) -> anyhow::Result<()> {
        unsafe {
            // Get the hwnd above the tracking hwnd so we can place the border window in between
            let hwnd_above_tracking = GetWindow(self.tracking_window, GW_HWNDPREV);

            let mut swp_flags = SWP_NOSENDCHANGING
                | SWP_NOACTIVATE
                | SWP_NOREDRAW
                | other_flags.unwrap_or_default();

            // If hwnd_above_tracking is the window border itself, we have what we want and there's
            // no need to change the z-order (plus it results in an error if we try it).
            if hwnd_above_tracking == Ok(self.border_window) {
                swp_flags |= SWP_NOZORDER;
            }

            if let Err(e) = SetWindowPos(
                self.border_window,
                hwnd_above_tracking.unwrap_or(HWND_TOP),
                self.window_rect.left,
                self.window_rect.top,
                self.window_rect.right - self.window_rect.left,
                self.window_rect.bottom - self.window_rect.top,
                swp_flags,
            )
            .context(format!(
                "could not set window position for {:?}",
                self.tracking_window
            )) {
                self.exit_border_thread();
                return Err(e);
            }
        }
        Ok(())
    }

    fn update_color(&mut self, check_delay: Option<u64>) -> anyhow::Result<()> {
        match animations::get_current_anims(self).contains_key(&AnimType::Fade) {
            false => self.update_brush_opacities(),
            true if check_delay == Some(0) => {
                self.update_brush_opacities();
                animations::update_fade_progress(self)
            }
            true => self.animations.event = ANIM_FADE,
        }

        Ok(())
    }

    fn update_brush_opacities(&mut self) {
        let (top_color, bottom_color) = match self.is_active_window {
            true => (&mut self.active_color, &mut self.inactive_color),
            false => (&mut self.inactive_color, &mut self.active_color),
        };
        top_color.set_opacity(1.0);
        bottom_color.set_opacity(0.0);
    }

    fn render(&mut self) -> anyhow::Result<()> {
        self.last_render_time = Some(time::Instant::now());

        let Some(ref render_target) = self.render_target else {
            return Err(anyhow!("render_target has not been set yet"));
        };

        let pixel_size = D2D_SIZE_U {
            width: (self.window_rect.right - self.window_rect.left) as u32,
            height: (self.window_rect.bottom - self.window_rect.top) as u32,
        };

        let border_width = self.border_width as f32;
        let border_offset = self.border_offset as f32;

        self.rounded_rect.rect = D2D_RECT_F {
            left: border_width / 2.0 - border_offset,
            top: border_width / 2.0 - border_offset,
            right: (self.window_rect.right - self.window_rect.left) as f32 - border_width / 2.0
                + border_offset,
            bottom: (self.window_rect.bottom - self.window_rect.top) as f32 - border_width / 2.0
                + border_offset,
        };

        unsafe {
            render_target
                .Resize(&pixel_size)
                .context("could not resize render_target")?;

            // Determine which color/rectangle should be drawn on top
            let (bottom_color, top_color) = match self.is_active_window {
                true => (&self.inactive_color, &self.active_color),
                false => (&self.active_color, &self.inactive_color),
            };

            render_target.BeginDraw();
            render_target.Clear(None);

            if bottom_color.get_opacity() > Some(0.0) {
                if let Color::Gradient(gradient) = bottom_color {
                    gradient.update_start_end_points(&self.window_rect);
                }

                match bottom_color.get_brush() {
                    Some(id2d1_brush) => self.draw_rectangle(render_target, id2d1_brush),
                    None => debug!("ID2D1Brush for bottom_color has not been created yet"),
                }
            }
            if top_color.get_opacity() > Some(0.0) {
                if let Color::Gradient(gradient) = top_color {
                    gradient.update_start_end_points(&self.window_rect);
                }

                match top_color.get_brush() {
                    Some(id2d1_brush) => self.draw_rectangle(render_target, id2d1_brush),
                    None => debug!("ID2D1Brush for top_color has not been created yet"),
                }
            }

            match render_target.EndDraw(None, None) {
                Ok(_) => {}
                Err(e) if e.code() == D2DERR_RECREATE_TARGET => {
                    // D2DERR_RECREATE_TARGET is recoverable if we just recreate the render target.
                    // This error can be caused by things like waking up from sleep, updating GPU
                    // drivers, changing screen resolution, etc.
                    warn!("render_target has been lost; attempting to recreate");

                    match self.create_render_resources() {
                        Ok(_) => info!("successfully recreated render_target; resuming thread"),
                        Err(e_2) => {
                            error!("could not recreate render_target; exiting thread: {e_2}");
                            self.exit_border_thread();
                        }
                    }
                }
                Err(other) => {
                    error!("render_target.EndDraw() failed; exiting thread: {other}");
                    self.exit_border_thread();
                }
            }
        }

        Ok(())
    }

    fn draw_rectangle(&self, render_target: &ID2D1HwndRenderTarget, brush: &ID2D1Brush) {
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

    fn exit_border_thread(&mut self) {
        self.pause = true;
        animations::destroy_timer(self);
        BORDERS
            .lock()
            .unwrap()
            .remove(&(self.tracking_window.0 as isize));
        unsafe { PostQuitMessage(0) };
    }

    pub unsafe extern "system" fn s_wnd_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // Retrieve the pointer to this WindowBorder struct using GWLP_USERDATA
        let mut border_pointer: *mut WindowBorder = GetWindowLongPtrW(window, GWLP_USERDATA) as _;

        // If a pointer has not yet been assigned to GWLP_USERDATA, assign it here using the LPARAM
        // from CreateWindowExW
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

    unsafe fn wnd_proc(
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

                // If the tracking window does not have a native border visible, hide this border
                // window, and return early to avoid unnecessary processing below
                if !has_native_border(self.tracking_window) {
                    self.update_position(Some(SWP_HIDEWINDOW)).log_if_err();
                    return LRESULT(0);
                }

                let old_rect = self.window_rect;
                self.update_window_rect().log_if_err();

                let update_pos_flags = match is_window_visible(self.border_window) {
                    true => None,
                    false => Some(SWP_SHOWWINDOW),
                };
                self.update_position(update_pos_flags).log_if_err();

                // TODO When a window is minimized, all four points of the rect go way below 0. For
                // some reason, after unminimizing/restoring, render() will sometimes render at
                // this minimized size. For now, I just do self.window_rect = old_rect to fix that.
                match is_rect_visible(&self.window_rect) {
                    true => {
                        if !are_rects_same_size(&self.window_rect, &old_rect) {
                            // Only re-render the border when its size changes
                            self.render().log_if_err();
                        }
                    }
                    false => self.window_rect = old_rect,
                }
            }
            // EVENT_OBJECT_REORDER
            WM_APP_REORDER => {
                // If something changes the z-order of windows, it may put the border window behind
                // the tracking window, so we update the border's position here when that happens
                self.update_position(None).log_if_err();
            }
            // EVENT_SYSTEM_FOREGROUND
            WM_APP_FOREGROUND => {
                self.is_active_window = is_active_window(self.tracking_window);

                self.update_color(None).log_if_err();
                self.update_position(None).log_if_err();
                self.render().log_if_err();
            }
            // EVENT_OBJECT_SHOW / EVENT_OBJECT_UNCLOAKED
            WM_APP_SHOWUNCLOAKED => {
                // With GlazeWM, if I switch to another workspace while a window is minimized and
                // switch back, then we will receive this message even though the window is not yet
                // visible. And, the window rect will be all weird. So, we apply the following fix.
                let old_rect = self.window_rect;
                self.update_window_rect().log_if_err();

                if !is_rect_visible(&self.window_rect) {
                    self.window_rect = old_rect;
                    return LRESULT(0);
                }

                if has_native_border(self.tracking_window) {
                    self.update_position(Some(SWP_SHOWWINDOW)).log_if_err();
                    self.render().log_if_err();
                }

                animations::set_timer_if_anims_enabled(self);
                self.pause = false;
            }
            // EVENT_OBJECT_HIDE / EVENT_OBJECT_CLOAKED
            WM_APP_HIDECLOAKED => {
                self.update_position(Some(SWP_HIDEWINDOW)).log_if_err();
                animations::destroy_timer(self);
                self.pause = true;
            }
            // EVENT_OBJECT_MINIMIZESTART
            WM_APP_MINIMIZESTART => {
                self.update_position(Some(SWP_HIDEWINDOW)).log_if_err();

                self.active_color.set_opacity(0.0);
                self.inactive_color.set_opacity(0.0);

                animations::destroy_timer(self);
                self.pause = true;
            }
            // EVENT_SYSTEM_MINIMIZEEND
            WM_APP_MINIMIZEEND => {
                // Keep the border hidden while the tracking window is in its unminimize animation
                thread::sleep(time::Duration::from_millis(self.unminimize_delay));

                if has_native_border(self.tracking_window) {
                    self.update_color(Some(self.unminimize_delay)).log_if_err();
                    self.update_window_rect().log_if_err();
                    self.update_position(Some(SWP_SHOWWINDOW)).log_if_err();
                    self.render().log_if_err();
                }

                animations::set_timer_if_anims_enabled(self);
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

                for (anim_type, anim_params) in animations::get_current_anims(self).clone().iter() {
                    match anim_type {
                        AnimType::Spiral => {
                            animations::animate_spiral(self, &anim_elapsed, anim_params, false);
                            update = true;
                        }
                        AnimType::ReverseSpiral => {
                            animations::animate_spiral(self, &anim_elapsed, anim_params, true);
                            update = true;
                        }
                        AnimType::Fade => {
                            if self.animations.event == ANIM_FADE {
                                animations::animate_fade(self, &anim_elapsed, anim_params);
                                update = true;
                            }
                        }
                    }
                }

                let render_interval = 1.0 / self.animations.fps as f32;
                let time_diff = render_elapsed.as_secs_f32() - render_interval;
                if update && (time_diff.abs() <= 0.001 || time_diff >= 0.0) {
                    self.render().log_if_err();
                }
            }
            WM_PAINT => {
                let _ = ValidateRect(window, None);
            }
            WM_NCDESTROY => {
                // TODO not actually sure if we need to set GWLP_USERDATA to 0 here
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                self.exit_border_thread();
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

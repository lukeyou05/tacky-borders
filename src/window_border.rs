use anyhow::{anyhow, Context};
use std::ptr;
use std::thread;
use std::time;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Foundation::{COLORREF, FALSE, HWND, LPARAM, LRESULT, RECT, TRUE, WPARAM};
use windows::Win32::Graphics::Direct2D::ID2D1RenderTarget;
use windows::Win32::Graphics::Direct2D::D2D1_BRUSH_PROPERTIES;
use windows::Win32::Graphics::Dwm::{
    DwmEnableBlurBehindWindow, DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS,
    DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND,
};
use windows::Win32::Graphics::Gdi::{CreateRectRgn, ValidateRect, HMONITOR};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetSystemMetrics, GetWindow,
    GetWindowLongPtrW, PostQuitMessage, SetLayeredWindowAttributes, SetWindowLongPtrW,
    SetWindowPos, TranslateMessage, CREATESTRUCTW, CW_USEDEFAULT, GWLP_USERDATA, GW_HWNDPREV,
    HWND_TOP, LWA_ALPHA, MSG, SET_WINDOW_POS_FLAGS, SM_CXVIRTUALSCREEN, SWP_HIDEWINDOW,
    SWP_NOACTIVATE, SWP_NOREDRAW, SWP_NOSENDCHANGING, SWP_NOZORDER, SWP_SHOWWINDOW, WM_CREATE,
    WM_NCDESTROY, WM_PAINT, WM_WINDOWPOSCHANGED, WM_WINDOWPOSCHANGING, WS_DISABLED, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::animations::{AnimType, AnimVec};
use crate::border_drawer::BorderDrawer;
use crate::config::WindowRule;
use crate::komorebi::WindowKind;
use crate::render_backend::{RenderBackend, RenderBackendConfig};
use crate::utils::{
    are_rects_same_size, get_dpi_for_window, get_monitor_info, get_window_rule, get_window_title,
    has_native_border, is_rect_visible, is_window_minimized, is_window_visible,
    monitor_from_window, post_message_w, LogIfErr, WM_APP_ANIMATE, WM_APP_FOREGROUND,
    WM_APP_HIDECLOAKED, WM_APP_KOMOREBI, WM_APP_LOCATIONCHANGE, WM_APP_MINIMIZEEND,
    WM_APP_MINIMIZESTART, WM_APP_REORDER, WM_APP_SHOWUNCLOAKED,
};
use crate::APP_STATE;

#[derive(Debug, Default)]
pub struct WindowBorder {
    border_window: HWND,
    tracking_window: HWND,
    window_state: WindowState,
    window_rect: RECT,
    window_padding: i32,
    current_monitor: HMONITOR,
    current_dpi: f32,
    border_drawer: BorderDrawer,
    initialize_delay: u64,
    unminimize_delay: u64,
    is_paused: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum WindowState {
    #[default]
    Active,
    Inactive,
}

impl WindowState {
    pub fn update(&mut self, self_hwnd: isize, active_hwnd: isize) {
        if self_hwnd == active_hwnd {
            *self = WindowState::Active;
        } else {
            *self = WindowState::Inactive;
        }
    }
}

impl WindowBorder {
    pub fn new(tracking_window: HWND) -> Self {
        Self {
            tracking_window,
            ..Default::default()
        }
    }

    pub fn create_window(&mut self) -> windows::core::Result<HWND> {
        let title: Vec<u16> = format!(
            "tacky-border | {} | {:?}\0",
            get_window_title(self.tracking_window).unwrap_or_default(),
            self.tracking_window
        )
        .encode_utf16()
        .collect();

        unsafe {
            self.border_window = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                w!("border"),
                PCWSTR(title.as_ptr()),
                WS_POPUP | WS_DISABLED,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None,
                None,
                Some(ptr::addr_of!(*self) as _),
            )?;
        }

        Ok(self.border_window)
    }

    pub fn init(&mut self, window_rule: WindowRule) -> anyhow::Result<()> {
        self.load_from_config(window_rule)?;

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

            let m_info = get_monitor_info(self.current_monitor).context("mi")?;
            let screen_width = (m_info.rcMonitor.right - m_info.rcMonitor.left) as u32;
            let screen_height = (m_info.rcMonitor.bottom - m_info.rcMonitor.top) as u32;

            self.border_drawer
                .init_renderer(
                    screen_width
                        + ((self.border_drawer.border_width + self.window_padding) * 2) as u32,
                    screen_height
                        + ((self.border_drawer.border_width + self.window_padding) * 2) as u32,
                    self.border_window,
                    &self.window_rect,
                    APP_STATE.config.read().unwrap().render_backend,
                )
                .context("could not initialize border drawer in init()")?;

            // Update the border's color
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

            self.border_drawer
                .animations
                .set_timer_if_enabled(self.border_window, &mut self.border_drawer.last_anim_time);

            // Handle the case where the tracking window is already minimized
            // TODO: maybe put this in a better spot but idk where
            if is_window_minimized(self.tracking_window) {
                post_message_w(
                    Some(self.border_window),
                    WM_APP_MINIMIZESTART,
                    WPARAM(0),
                    LPARAM(0),
                )
                .context("could not post WM_APP_MINIMIZESTART message in init()")
                .log_if_err();
            }

            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).into() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            debug!("exiting border thread for {:?}!", self.tracking_window);
        }

        Ok(())
    }

    pub fn load_from_config(&mut self, window_rule: WindowRule) -> anyhow::Result<()> {
        let config = APP_STATE.config.read().unwrap();
        let global = &config.global;

        let width_config = window_rule.border_width.unwrap_or(global.border_width);
        let offset_config = window_rule.border_offset.unwrap_or(global.border_offset);
        let radius_config = window_rule
            .border_radius
            .as_ref()
            .unwrap_or(&global.border_radius);
        let active_color_config = window_rule
            .active_color
            .as_ref()
            .unwrap_or(&global.active_color);
        let inactive_color_config = window_rule
            .inactive_color
            .as_ref()
            .unwrap_or(&global.inactive_color);
        let animations_config = window_rule
            .animations
            .as_ref()
            .unwrap_or(&global.animations);
        let effects_config = window_rule.effects.as_ref().unwrap_or(&global.effects);

        self.current_monitor = monitor_from_window(self.tracking_window);
        self.current_dpi = match get_dpi_for_window(self.tracking_window) {
            Ok(dpi) => dpi as f32,
            Err(err) => {
                self.cleanup_and_queue_exit();
                return Err(anyhow!("could not get dpi for window: {err}"));
            }
        };

        // Adjust the border width and radius based on the window/monitor dpi
        let border_width = (width_config * self.current_dpi / 96.0).round() as i32;
        let border_offset = offset_config;
        let border_radius =
            radius_config.to_radius(border_width, self.current_dpi, self.tracking_window);
        let active_color = active_color_config.to_color(true);
        let inactive_color = inactive_color_config.to_color(false);

        let animations = animations_config.to_animations();
        let effects = effects_config.to_effects();

        // Configure the border's appearance using above variables
        self.border_drawer.configure_border(
            border_width,
            border_offset,
            border_radius,
            active_color,
            inactive_color,
            animations,
            effects,
        );

        // This padding is used to increase the size of the border window such that effects don't
        // get clipped. However, effects are not supported by the Legacy rendering backend, so
        // we'll set the padding to 0 if that's what's being used.
        self.window_padding = match config.render_backend {
            RenderBackendConfig::V2 => {
                let max_active_padding = self
                    .border_drawer
                    .effects
                    .active
                    .iter()
                    .max_by_key(|params| {
                        // Try to find the effect params with the largest required padding
                        let max_std_dev = params.std_dev;
                        let max_translation = (params.translation.x).max(params.translation.y);

                        ((max_std_dev * 3.0).ceil() + max_translation.ceil()) as i32
                    })
                    .map(|params| {
                        // Now that we found it, go ahead and calculate it as an f32
                        let max_std_dev = params.std_dev;
                        let max_translation = (params.translation.x).max(params.translation.y);

                        (max_std_dev * 3.0).ceil() + max_translation.ceil()
                    })
                    .unwrap_or(0.0);
                let max_inactive_padding = self
                    .border_drawer
                    .effects
                    .inactive
                    .iter()
                    .max_by_key(|params| {
                        // Try to find the effect params with the largest required padding
                        let max_std_dev = params.std_dev;
                        let max_translation = (params.translation.x).max(params.translation.y);

                        // 3 standard deviations gets us 99.7% coverage, which should be good enough
                        ((max_std_dev * 3.0).ceil() + max_translation.ceil()) as i32
                    })
                    .map(|params| {
                        // Now that we found it, go ahead and calculate it as an f32
                        let max_std_dev = params.std_dev;
                        let max_translation = (params.translation.x).max(params.translation.y);

                        // 3 standard deviations gets us 99.7% coverage, which should be good enough
                        (max_std_dev * 3.0).ceil() + max_translation.ceil()
                    })
                    .unwrap_or(0.0);

                max_active_padding.max(max_inactive_padding).ceil() as i32
            }
            RenderBackendConfig::Legacy => 0,
        };

        // If the tracking window is part of the initial windows list (meaning it was already open when
        // tacky-borders was launched), then there should be no initialize delay.
        self.initialize_delay = match APP_STATE
            .initial_windows
            .lock()
            .unwrap()
            .contains(&(self.tracking_window.0 as isize))
        {
            true => 0,
            false => window_rule
                .initialize_delay
                .unwrap_or(global.initialize_delay),
        };
        self.unminimize_delay = window_rule
            .unminimize_delay
            .unwrap_or(global.unminimize_delay);

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
            self.cleanup_and_queue_exit();
            return Err(e);
        }

        let adjustment = self.border_drawer.border_width + self.window_padding;
        // Make space for the border + padding
        self.window_rect.top -= adjustment;
        self.window_rect.left -= adjustment;
        self.window_rect.right += adjustment;
        self.window_rect.bottom += adjustment;

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
                Some(hwnd_above_tracking.unwrap_or(HWND_TOP)),
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
                self.cleanup_and_queue_exit();
                return Err(e);
            }
        }
        Ok(())
    }

    fn update_color(&mut self, check_delay: Option<u64>) -> anyhow::Result<()> {
        self.window_state.update(
            self.tracking_window.0 as isize,
            *APP_STATE.active_window.lock().unwrap(),
        );

        match self
            .border_drawer
            .animations
            .get_current(self.window_state)
            .contains_type(AnimType::Fade)
        {
            false => self.update_brush_opacities(),
            true if check_delay == Some(0) => {
                self.update_brush_opacities();
                self.border_drawer
                    .animations
                    .update_fade_progress(self.window_state)
            }
            true => self.border_drawer.animations.should_fade = true,
        }

        Ok(())
    }

    fn update_brush_opacities(&mut self) {
        let (top_color, bottom_color) = match self.window_state {
            WindowState::Active => (
                &mut self.border_drawer.active_color,
                &mut self.border_drawer.inactive_color,
            ),
            WindowState::Inactive => (
                &mut self.border_drawer.inactive_color,
                &mut self.border_drawer.active_color,
            ),
        };
        top_color.set_opacity(1.0);
        bottom_color.set_opacity(0.0);
    }

    fn update_width_radius(&mut self) {
        let window_rule = get_window_rule(self.tracking_window);
        let config = APP_STATE.config.read().unwrap();
        let global = &config.global;

        let width_config = window_rule.border_width.unwrap_or(global.border_width);
        let radius_config = window_rule
            .border_radius
            .as_ref()
            .unwrap_or(&global.border_radius);

        self.border_drawer.border_width = (width_config * self.current_dpi / 96.0).round() as i32;
        self.border_drawer.border_radius = radius_config.to_radius(
            self.border_drawer.border_width,
            self.current_dpi,
            self.tracking_window,
        );
    }

    fn render(&mut self) -> anyhow::Result<()> {
        if let Err(err) =
            self.border_drawer
                .render(&self.window_rect, self.window_padding, self.window_state)
        {
            self.handle_render_error(err)?
        };

        Ok(())
    }

    fn handle_render_error(&mut self, err: windows::core::Error) -> anyhow::Result<()> {
        if err.code() == D2DERR_RECREATE_TARGET {
            // D2DERR_RECREATE_TARGET is recoverable if we just recreate the render target.
            // This error can be caused by things like waking up from sleep, updating GPU
            // drivers, changing screen resolution, etc.
            warn!("render target has been lost; attempting to recreate");

            let pixel_size = self.border_drawer.render_backend.get_pixel_size()?;
            let render_backend_config = match self.border_drawer.render_backend {
                RenderBackend::V2(_) => RenderBackendConfig::V2,
                RenderBackend::Legacy(_) => RenderBackendConfig::Legacy,
                RenderBackend::None => {
                    // This branch should be unreachable (theoretically)
                    self.cleanup_and_queue_exit();
                    return Err(anyhow!("render_backend is None"));
                }
            };

            if let Err(err2) = self.border_drawer.init_renderer(
                pixel_size.width,
                pixel_size.height,
                self.border_window,
                &self.window_rect,
                render_backend_config,
            ) {
                self.cleanup_and_queue_exit();
                return Err(anyhow!(
                    "could not recreate render target; exiting thread: {err2}"
                ));
            };

            info!("successfully recreated render target; resuming thread");
        } else {
            self.cleanup_and_queue_exit();
            return Err(anyhow!("self.render() failed; exiting thread: {err}"));
        }

        Ok(())
    }

    fn cleanup_and_queue_exit(&mut self) {
        self.is_paused = true;
        self.border_drawer.animations.destroy_timer();
        APP_STATE
            .borders
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
                if self.is_paused {
                    return LRESULT(0);
                }

                let mut should_render = false;

                // Hide tacky-borders' custom border if no native border is present
                if !has_native_border(self.tracking_window) {
                    self.update_position(Some(SWP_HIDEWINDOW)).log_if_err();
                    return LRESULT(0);
                }

                let old_rect = self.window_rect;
                self.update_window_rect().log_if_err();

                // TODO: After restoring a minimized window, render() may use the minimized
                // (invisible) rect instead of the updated one. This is a temporary "fix".
                if !is_rect_visible(&self.window_rect) {
                    self.window_rect = old_rect;
                    return LRESULT(0);
                }

                // If the window rect changes size, we need to re-render the border
                if !are_rects_same_size(&self.window_rect, &old_rect) {
                    should_render |= true;
                }

                let update_pos_flags =
                    (!is_window_visible(self.border_window)).then_some(SWP_SHOWWINDOW);
                self.update_position(update_pos_flags).log_if_err();

                let new_monitor = monitor_from_window(self.tracking_window);
                if new_monitor != self.current_monitor {
                    self.current_monitor = new_monitor;

                    // TODO: maybe dont lock this behind the new_monitor != current_monitor
                    // condition because it's possible to have dpi change but monitor not change if
                    // someone changes the display scaling
                    let new_dpi = match get_dpi_for_window(self.tracking_window) {
                        Ok(dpi) => dpi as f32,
                        Err(err) => {
                            error!("could not get dpi for window: {err}");
                            self.cleanup_and_queue_exit();
                            return LRESULT(0);
                        }
                    };
                    if new_dpi != self.current_dpi {
                        self.current_dpi = new_dpi;
                        self.update_width_radius();
                    }

                    let m_info = get_monitor_info(self.current_monitor)
                        .context("mi")
                        .unwrap();
                    let screen_width = (m_info.rcMonitor.right - m_info.rcMonitor.left) as u32;
                    let screen_height = (m_info.rcMonitor.bottom - m_info.rcMonitor.top) as u32;

                    self.border_drawer
                        .update_renderer(
                            screen_width
                                + ((self.border_drawer.border_width + self.window_padding) * 2)
                                    as u32,
                            screen_height
                                + ((self.border_drawer.border_width + self.window_padding) * 2)
                                    as u32,
                        )
                        .context("could not update border drawer")
                        .log_if_err();

                    should_render |= true;
                }

                if should_render {
                    self.render().log_if_err();
                }
            }
            // EVENT_OBJECT_REORDER
            WM_APP_REORDER => {
                // When the tracking window reorders its contents, it may change the z-order. So,
                // we first check whether the border is still above the tracking window, and if
                // not, we must update its position and place it back on top
                if GetWindow(self.tracking_window, GW_HWNDPREV) != Ok(self.border_window) {
                    self.update_position(None).log_if_err();
                }
            }
            // EVENT_SYSTEM_FOREGROUND
            WM_APP_FOREGROUND => {
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

                self.update_color(None).log_if_err();

                if has_native_border(self.tracking_window) {
                    self.update_position(Some(SWP_SHOWWINDOW)).log_if_err();
                    self.render().log_if_err();
                }

                self.border_drawer.animations.set_timer_if_enabled(
                    self.border_window,
                    &mut self.border_drawer.last_anim_time,
                );
                self.is_paused = false;
            }
            // EVENT_OBJECT_HIDE / EVENT_OBJECT_CLOAKED
            WM_APP_HIDECLOAKED => {
                self.update_position(Some(SWP_HIDEWINDOW)).log_if_err();
                self.border_drawer.animations.destroy_timer();
                self.is_paused = true;
            }
            // EVENT_OBJECT_MINIMIZESTART
            WM_APP_MINIMIZESTART => {
                self.update_position(Some(SWP_HIDEWINDOW)).log_if_err();

                self.border_drawer.active_color.set_opacity(0.0);
                self.border_drawer.inactive_color.set_opacity(0.0);

                self.border_drawer.animations.destroy_timer();
                self.is_paused = true;
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

                self.border_drawer.animations.set_timer_if_enabled(
                    self.border_window,
                    &mut self.border_drawer.last_anim_time,
                );
                self.is_paused = false;
            }
            WM_APP_ANIMATE => {
                if self.is_paused {
                    return LRESULT(0);
                }

                self.border_drawer.animate(
                    &self.window_rect,
                    self.window_padding,
                    self.window_state,
                );
            }
            WM_APP_KOMOREBI => {
                let window_rule = get_window_rule(self.tracking_window);
                let global = &APP_STATE.config.read().unwrap().global;

                // Exit if komorebi colors are disabled for this tracking window
                // TODO: it might be better to store komorebi_colors in this WindowBorder struct
                if !window_rule
                    .komorebi_colors
                    .as_ref()
                    .map(|komocolors| komocolors.enabled)
                    .unwrap_or(global.komorebi_colors.enabled)
                {
                    return LRESULT(0);
                }

                let komorebi_integration = APP_STATE.komorebi_integration.lock().unwrap();
                let focus_state = komorebi_integration.focus_state.lock().unwrap();

                // TODO: idk what to do with None so i just do unwrap_or() rn
                let window_kind = *focus_state
                    .get(&(self.tracking_window.0 as isize))
                    .unwrap_or(&WindowKind::Single);

                drop(focus_state);
                drop(komorebi_integration);

                // Ignore Unfocused window kind
                if window_kind == WindowKind::Unfocused {
                    return LRESULT(0);
                }

                let active_color_config = window_rule
                    .active_color
                    .as_ref()
                    .unwrap_or(&global.active_color);
                let komorebi_colors_config = window_rule
                    .komorebi_colors
                    .as_ref()
                    .unwrap_or(&global.komorebi_colors);

                let old_opacity = self
                    .border_drawer
                    .active_color
                    .get_opacity()
                    .unwrap_or_default();
                let old_transform = self
                    .border_drawer
                    .active_color
                    .get_transform()
                    .unwrap_or_default();

                self.border_drawer.active_color = match window_kind {
                    WindowKind::Single => active_color_config.to_color(true),
                    WindowKind::Stack => komorebi_colors_config
                        .stack_color
                        .as_ref()
                        .unwrap_or(active_color_config)
                        .to_color(true),
                    WindowKind::Monocle => komorebi_colors_config
                        .monocle_color
                        .as_ref()
                        .unwrap_or(active_color_config)
                        .to_color(true),
                    WindowKind::Floating => komorebi_colors_config
                        .floating_color
                        .as_ref()
                        .unwrap_or(active_color_config)
                        .to_color(true),
                    WindowKind::Unfocused => {
                        debug!("what."); // It shouldn't be possible to reach this match branch
                        return LRESULT(0);
                    }
                };

                let renderer: &ID2D1RenderTarget = match self.border_drawer.render_backend {
                    RenderBackend::V2(ref backend) => &backend.d2d_context,
                    RenderBackend::Legacy(ref backend) => &backend.render_target,
                    RenderBackend::None => {
                        error!("render backend is None");
                        return LRESULT(0);
                    }
                };

                let brush_properties = D2D1_BRUSH_PROPERTIES {
                    opacity: old_opacity,
                    transform: old_transform,
                };

                self.border_drawer
                    .active_color
                    .init_brush(renderer, &self.window_rect, &brush_properties)
                    .log_if_err();
            }
            WM_PAINT => {
                let _ = ValidateRect(Some(window), None);
            }
            WM_NCDESTROY => {
                // TODO not actually sure if we need to set GWLP_USERDATA to 0 here
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                self.cleanup_and_queue_exit();
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

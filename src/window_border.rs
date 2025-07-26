use anyhow::{Context, anyhow};
use std::ptr;
use std::thread;
use std::time;
use windows::Win32::Foundation::{
    COLORREF, D2DERR_RECREATE_TARGET, FALSE, HWND, LPARAM, LRESULT, RECT, TRUE, WPARAM,
};
use windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U;
use windows::Win32::Graphics::Direct2D::{D2D1_BRUSH_PROPERTIES, ID2D1RenderTarget};
use windows::Win32::Graphics::Dwm::{
    DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND, DWMWA_EXTENDED_FRAME_BOUNDS,
    DwmEnableBlurBehindWindow, DwmGetWindowAttribute,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_ERROR_DEVICE_REMOVED,
    DXGI_GPU_PREFERENCE_UNSPECIFIED, IDXGIAdapter, IDXGIFactory6,
};
use windows::Win32::Graphics::Gdi::{CreateRectRgn, HMONITOR, ValidateRect};
use windows::Win32::UI::HiDpi::MDT_DEFAULT;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CW_USEDEFAULT, CreateWindowExW, DBT_DEVNODES_CHANGED, DefWindowProcW,
    DispatchMessageW, GW_HWNDPREV, GWLP_USERDATA, GetMessageW, GetSystemMetrics, GetWindow,
    GetWindowLongPtrW, HWND_TOP, LWA_ALPHA, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND,
    PBT_APMSUSPEND, PostQuitMessage, SET_WINDOW_POS_FLAGS, SM_CXVIRTUALSCREEN, SWP_HIDEWINDOW,
    SWP_NOACTIVATE, SWP_NOREDRAW, SWP_NOSENDCHANGING, SWP_NOZORDER, SWP_SHOWWINDOW,
    SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, TranslateMessage, WM_CREATE,
    WM_DEVICECHANGE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_NCDESTROY, WM_PAINT, WM_POWERBROADCAST,
    WM_WINDOWPOSCHANGED, WM_WINDOWPOSCHANGING, WS_DISABLED, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::APP_STATE;
use crate::animations::{AnimType, AnimVec};
use crate::border_drawer::BorderDrawer;
use crate::config::{RadiusConfig, WindowRule, ZOrderMode};
use crate::komorebi::WindowKind;
use crate::render_backend::{RenderBackend, RenderBackendConfig};
use crate::utils::{
    LogIfErr, ReentrancyBlocker, ReentrancyBlockerExt, StandaloneWindowsError, T_E_ERROR,
    T_E_REENTRANCY, T_E_UNINIT, ToWindowsResult, WM_APP_ANIMATE, WM_APP_FOREGROUND,
    WM_APP_HIDECLOAKED, WM_APP_KOMOREBI, WM_APP_LOCATIONCHANGE, WM_APP_MINIMIZEEND,
    WM_APP_MINIMIZESTART, WM_APP_RECREATE_DRAWER, WM_APP_REORDER, WM_APP_SHOWUNCLOAKED,
    WindowsCompatibleError, WindowsCompatibleResult, WindowsContext, are_rects_same_size,
    get_dpi_for_monitor, get_monitor_resolution, get_window_rule, get_window_title,
    has_native_border, is_window_arranged, is_window_cloaked, is_window_minimized,
    is_window_visible, loword, monitor_from_window, post_message_w,
};

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

#[derive(Debug, Default)]
pub struct WindowBorder {
    border_window: HWND,
    tracking_window: HWND,
    window_state: WindowState,
    window_rect: RECT,
    window_padding: i32,
    current_monitor: HMONITOR,
    current_dpi: u32,
    border_drawer: BorderDrawer,
    // ---- cached config values ----
    radius_config: RadiusConfig,
    border_z_order: ZOrderMode,
    follow_native_border: bool,
    initialize_delay: u64,
    unminimize_delay: u64,
    // ------------------------------
    is_paused: bool,
}

impl WindowBorder {
    pub fn new(tracking_window: HWND) -> Self {
        Self {
            tracking_window,
            ..Default::default()
        }
    }

    pub fn create_window(&mut self) -> WindowsCompatibleResult<HWND> {
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
        self.current_monitor = monitor_from_window(self.tracking_window);
        self.current_dpi =
            get_dpi_for_monitor(self.current_monitor, MDT_DEFAULT).map_err(|err| {
                self.cleanup_and_queue_exit();
                anyhow!("could not get dpi for {:?}: {}", self.current_monitor, err)
            })?;
        self.load_from_config(window_rule, self.current_dpi)?;

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
            DwmEnableBlurBehindWindow(self.border_window, &bh)
                .context("could not make window transparent")?;

            SetLayeredWindowAttributes(self.border_window, COLORREF(0x00000000), 255, LWA_ALPHA)
                .context("could not set LWA_ALPHA")?;

            let (screen_width, screen_height) = get_monitor_resolution(self.current_monitor)
                .windows_context("could not get monitor resolution")?;
            self.init_drawer(screen_width, screen_height)
                .context("could not initialize border drawer in init()")?;

            self.update_color(Some(self.initialize_delay));
            self.update_window_rect().log_if_err();

            if self.should_show_border() {
                self.update_position(Some(SWP_SHOWWINDOW)).log_if_err();
                self.render().log_if_err();

                // TODO: sometimes, the border doesn't show up on the first try. So, we just wait
                // 5ms and call render() again. This seems to be an issue with the visibility of
                // the window itself.
                thread::sleep(time::Duration::from_millis(5));
                self.update_position(Some(SWP_SHOWWINDOW)).log_if_err();
                self.render().log_if_err();

                self.border_drawer
                    .set_anims_timer_if_needed(self.border_window);
            }

            // Handle the edge case where the tracking window is already minimized
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

            {
                let komorebi_integration = APP_STATE.komorebi_integration.lock().unwrap();
                if komorebi_integration.is_running() {
                    let self_focus_state = komorebi_integration
                        .focus_state
                        .lock()
                        .unwrap()
                        .get(&(self.tracking_window.0 as isize))
                        .copied();

                    // Handle the edge case where the focus state is already komorebi-specific upon border creation
                    if !matches!(
                        self_focus_state,
                        None | Some(WindowKind::Single) | Some(WindowKind::Unfocused)
                    ) {
                        post_message_w(
                            Some(self.border_window),
                            WM_APP_KOMOREBI,
                            WPARAM(0),
                            LPARAM(0),
                        )
                        .context("could not post WM_APP_KOMOREBI message in init()")
                        .log_if_err();
                    }
                }
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

    pub fn load_from_config(&mut self, window_rule: WindowRule, dpi: u32) -> anyhow::Result<()> {
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

        self.radius_config = *radius_config;

        // Adjust the border parameters based on the window/monitor dpi
        let border_width = (width_config * dpi as f32 / 96.0).round() as i32;
        let border_offset = (offset_config as f32 * dpi as f32 / 96.0).round() as i32;
        let border_radius = radius_config.to_radius(border_width, dpi, self.tracking_window);
        let active_color = active_color_config.to_color_brush(true);
        let inactive_color = inactive_color_config.to_color_brush(false);

        let animations = animations_config.to_animations();
        let effects = effects_config.to_effects();

        self.border_drawer.configure_appearance(
            border_width,
            border_offset,
            border_radius,
            active_color,
            inactive_color,
            animations,
            effects,
        );

        // This padding is used to adjust the border window such that the border and its effects
        // don't get clipped. However, effects are not supported by the Legacy render backend, so
        // we'll just set the padding to border_offset if that's what's being used.
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
                        let max_translation =
                            f32::max(params.translation.x.abs(), params.translation.y.abs());

                        // 3 standard deviations gets us 99.7% coverage, which should be good enough
                        ((max_std_dev * 3.0).ceil() + max_translation.ceil()) as i32
                    })
                    .map(|params| {
                        // Now that we found it, go ahead and calculate it as an f32
                        let max_std_dev = params.std_dev;
                        let max_translation =
                            f32::max(params.translation.x.abs(), params.translation.y.abs());

                        // 3 standard deviations gets us 99.7% coverage, which should be good enough
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
                        let max_translation =
                            f32::max(params.translation.x.abs(), params.translation.y.abs());

                        ((max_std_dev * 3.0).ceil() + max_translation.ceil()) as i32
                    })
                    .map(|params| {
                        // Now that we found it, go ahead and calculate it as an f32
                        let max_std_dev = params.std_dev;
                        let max_translation =
                            f32::max(params.translation.x.abs(), params.translation.y.abs());

                        (max_std_dev * 3.0).ceil() + max_translation.ceil()
                    })
                    .unwrap_or(0.0);

                f32::max(max_active_padding, max_inactive_padding).ceil() as i32 + border_offset
            }
            RenderBackendConfig::Legacy => border_offset,
        };

        self.border_z_order = window_rule.border_z_order.unwrap_or(global.border_z_order);
        self.follow_native_border = window_rule
            .follow_native_border
            .unwrap_or(global.follow_native_border);

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

    fn compute_proper_renderer_size(
        screen_width: u32,
        screen_height: u32,
        border_width: i32,
        window_padding: i32,
    ) -> D2D_SIZE_U {
        D2D_SIZE_U {
            width: (screen_width as i32 + ((border_width + window_padding) * 2)) as u32,
            height: (screen_height as i32 + ((border_width + window_padding) * 2)) as u32,
        }
    }

    fn raw_init_drawer(
        &mut self,
        screen_width: u32,
        screen_height: u32,
    ) -> WindowsCompatibleResult<()> {
        let renderer_size = Self::compute_proper_renderer_size(
            screen_width,
            screen_height,
            self.border_drawer.border_width,
            self.window_padding,
        );
        self.border_drawer
            .init(
                renderer_size.width,
                renderer_size.height,
                self.border_window,
                &self.window_rect,
                APP_STATE.config.read().unwrap().render_backend,
            )
            .windows_context("could not initialize border drawer")?;

        Ok(())
    }

    fn init_drawer(
        &mut self,
        screen_width: u32,
        screen_height: u32,
    ) -> WindowsCompatibleResult<()> {
        self.raw_init_drawer(screen_width, screen_height)
            .or_else(|err| {
                self.handle_directx_errors(err)?;
                self.raw_init_drawer(screen_width, screen_height)
            })
            .inspect_err(|err| {
                if err.code() != T_E_REENTRANCY {
                    self.cleanup_and_queue_exit();
                }
            })
    }

    fn needs_drawer_recreation(&self) -> WindowsCompatibleResult<bool> {
        match self.border_drawer.render_backend {
            // With the V2 backend, we use the stored adapter LUID to check whether our backend is
            // still using the primary display adapter.
            RenderBackend::V2(ref backend) => {
                let dxgi_factory: IDXGIFactory6 =
                    unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS::default()) }
                        .windows_context(
                            "could not create dxgi_factory to check for GPU adapter changes",
                        )?;

                let new_dxgi_adapter: IDXGIAdapter = unsafe {
                    dxgi_factory.EnumAdapterByGpuPreference(0, DXGI_GPU_PREFERENCE_UNSPECIFIED)?
                };
                let new_adapter_desc = unsafe { new_dxgi_adapter.GetDesc() }
                    .windows_context("could not get new_adapter_desc")?;

                Ok(backend.adapter_luid != new_adapter_desc.AdapterLuid)
            }
            // With the Legacy backend, we check whether the underlying display adapter is still
            // valid. This does not guarantee that it is the primary adapter.
            RenderBackend::Legacy(ref backend) => unsafe {
                backend.render_target.BeginDraw();
                Ok(backend.render_target.EndDraw(None, None).is_err())
            },
            RenderBackend::None => Err(WindowsCompatibleError::Standalone(
                StandaloneWindowsError::new(T_E_UNINIT, "render_backend is None"),
            )),
        }
    }

    fn recreate_drawer_if_needed(&mut self) -> WindowsCompatibleResult<()> {
        if self
            .needs_drawer_recreation()
            .windows_context("could not check if border drawer needs to be recreated")?
        {
            let (screen_width, screen_height) = get_monitor_resolution(self.current_monitor)
                .windows_context("could not get monitor resolution")?;
            self.init_drawer(screen_width, screen_height)
                .windows_context("could not recreate border drawer")?;
            self.update_color(None);
            self.render().windows_context("could not render")?;
        }

        Ok(())
    }

    fn should_show_border(&self) -> bool {
        (!self.follow_native_border || has_native_border(self.tracking_window))
            && is_window_visible(self.tracking_window)
            && !is_window_cloaked(self.tracking_window)
            && !is_window_minimized(self.tracking_window)
    }

    // NOTE: Avoid calling this function + self.render() while the tracking window is minimized
    // as its coordinates will be meaningless and will lead to an oddly sized border.
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

    // NOTE: SetWindowPos is asynchronous, meaning it sends a request to the DWM but doesn't wait
    // for it to complete. Keep this in mind when ordering function calls elsewhere in the code.
    fn update_position(&mut self, other_flags: Option<SET_WINDOW_POS_FLAGS>) -> anyhow::Result<()> {
        unsafe {
            let mut swp_flags = SWP_NOSENDCHANGING
                | SWP_NOACTIVATE
                | SWP_NOREDRAW
                | other_flags.unwrap_or_default();

            let hwndinsertafter = match self.border_z_order {
                ZOrderMode::AboveWindow => {
                    // Get the hwnd above the tracking hwnd so we can place the border window in between
                    let hwnd_above_tracking = GetWindow(self.tracking_window, GW_HWNDPREV);

                    // If hwnd_above_tracking is the window border itself, we have what we want and there's
                    // no need to change the z-order (plus it results in an error if we try it).
                    if hwnd_above_tracking == Ok(self.border_window) {
                        swp_flags |= SWP_NOZORDER;
                    }

                    hwnd_above_tracking.unwrap_or(HWND_TOP)
                }
                ZOrderMode::BelowWindow => self.tracking_window,
            };

            if let Err(e) = SetWindowPos(
                self.border_window,
                Some(hwndinsertafter),
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

    fn update_color(&mut self, check_delay: Option<u64>) {
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
            true => {} // We will rely on the animations callback to update color
        }
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
        top_color.set_opacity(1.0).log_if_err();
        bottom_color.set_opacity(0.0).log_if_err();
    }

    fn handle_directx_errors(
        &mut self,
        err: WindowsCompatibleError,
    ) -> WindowsCompatibleResult<()> {
        thread_local! {
            static REENTRANCY_BLOCKER: ReentrancyBlocker = ReentrancyBlocker::new();
        }

        if err.code() == D2DERR_RECREATE_TARGET || err.code() == DXGI_ERROR_DEVICE_REMOVED {
            let _guard = REENTRANCY_BLOCKER
                .enter()
                .context("handle_directx_errors")
                .to_windows_result(T_E_REENTRANCY)?;

            if let Some(directx_devices) = APP_STATE.directx_devices.write().unwrap().as_mut() {
                directx_devices
                    .recreate_if_needed()
                    .windows_context("could not recreate directx devices if needed")?;
            }
            self.recreate_drawer_if_needed()
                .windows_context("could not recreate border drawer if needed")?;
        } else if err.code() == T_E_UNINIT {
            // Functions like render() may be called via callback functions before init()
            // completes, leading to errors due to uninitialized objects. This is likely only
            // temporary, so I'll just use debug! instead of logging it as a full error.
            debug!("an object is currently unitialized: {err}");
        } else {
            return Err(WindowsCompatibleError::Standalone(
                StandaloneWindowsError::new(
                    T_E_ERROR,
                    format!("self.render() failed; exiting thread: {err}"),
                ),
            ));
        }

        Ok(())
    }

    fn raw_render(&mut self) -> WindowsCompatibleResult<()> {
        self.border_drawer
            .render(&self.window_rect, self.window_padding, self.window_state)
    }

    pub fn render(&mut self) -> WindowsCompatibleResult<()> {
        self.raw_render()
            .or_else(|err| {
                self.handle_directx_errors(err)?;
                self.raw_render()
            })
            .inspect_err(|err| {
                if err.code() != T_E_REENTRANCY {
                    self.cleanup_and_queue_exit();
                }
            })
    }

    fn rescale_border(&mut self, new_dpi: u32) {
        let window_rule = get_window_rule(self.tracking_window);
        let config = APP_STATE.config.read().unwrap();
        let global = &config.global;

        let width_config = window_rule.border_width.unwrap_or(global.border_width);
        let offset_config = window_rule.border_offset.unwrap_or(global.border_offset);
        let radius_config = window_rule
            .border_radius
            .as_ref()
            .unwrap_or(&global.border_radius);

        self.border_drawer.border_width = (width_config * new_dpi as f32 / 96.0).round() as i32;
        self.border_drawer.border_offset =
            (offset_config as f32 * new_dpi as f32 / 96.0).round() as i32;
        let new_radius = radius_config.to_radius(
            self.border_drawer.border_width,
            new_dpi,
            self.tracking_window,
        );
        self.border_drawer
            .border_radius
            .set(new_radius)
            .unwrap_or_else(|err| debug!("border_radius: {err}")); // non-critical, so debug
    }

    fn needs_renderer_resize(&self, screen_width: u32, screen_height: u32) -> anyhow::Result<bool> {
        let correct_renderer_size = Self::compute_proper_renderer_size(
            screen_width,
            screen_height,
            self.border_drawer.border_width,
            self.window_padding,
        );
        let actual_renderer_size = self
            .border_drawer
            .render_backend
            .get_pixel_size()
            .context("could not get actual renderer size")?;

        Ok(correct_renderer_size != actual_renderer_size)
    }

    fn raw_resize_renderer(
        &mut self,
        screen_width: u32,
        screen_height: u32,
    ) -> WindowsCompatibleResult<()> {
        let renderer_size = Self::compute_proper_renderer_size(
            screen_width,
            screen_height,
            self.border_drawer.border_width,
            self.window_padding,
        );
        self.border_drawer
            .resize_renderer(renderer_size.width, renderer_size.height)
            .windows_context("could not update renderer")
    }

    fn resize_renderer(
        &mut self,
        screen_width: u32,
        screen_height: u32,
    ) -> WindowsCompatibleResult<()> {
        self.raw_resize_renderer(screen_width, screen_height)
            .or_else(|err| {
                self.handle_directx_errors(err)?;
                self.raw_resize_renderer(screen_width, screen_height)
            })
            .inspect_err(|err| {
                if err.code() != T_E_REENTRANCY {
                    self.cleanup_and_queue_exit();
                }
            })
    }

    fn rescale_border_and_resize_renderer_if_needed(
        &mut self,
        new_monitor: HMONITOR,
    ) -> anyhow::Result<bool> {
        let mut is_updated = false;

        let new_dpi =
            get_dpi_for_monitor(new_monitor, MDT_DEFAULT).context("could not get new_dpi")?;
        if new_dpi != self.current_dpi {
            self.current_dpi = new_dpi;
            debug!("dpi has changed! new dpi: {new_dpi}");
            is_updated = true;

            self.rescale_border(new_dpi);
        }

        let (screen_width, screen_height) =
            get_monitor_resolution(new_monitor).context("could not get monitor resolution")?;

        if self
            .needs_renderer_resize(screen_width, screen_height)
            .context("could not check if renderer needs resizing")?
        {
            self.resize_renderer(screen_width, screen_height)?;
            is_updated = true;
        }

        Ok(is_updated)
    }

    // Overrides the border radius to be square when the tracking window is arranged (snapped)
    fn sync_radius_for_snapped_state(&mut self) -> bool {
        let mut is_updated = false;

        // RadiusConfig::Custom(-1.0) is also checked for legacy reasons
        let is_radius_config_auto = matches!(
            self.radius_config,
            RadiusConfig::Auto | RadiusConfig::Custom(-1.0)
        );
        let is_tracking_window_arranged = is_window_arranged(self.tracking_window);
        let is_radius_locked = self.border_drawer.border_radius.is_locked();

        if is_radius_config_auto && is_tracking_window_arranged && !is_radius_locked {
            self.border_drawer
                .border_radius
                .set(0.0)
                .unwrap_or_else(|err| debug!("border_radius: {err}")); // non-critical, so debug
            self.border_drawer.border_radius.lock_writes();

            is_updated = true;
        } else if !is_tracking_window_arranged && is_radius_locked {
            self.border_drawer.border_radius.unlock_writes();
            let radius = self.radius_config.to_radius(
                self.border_drawer.border_width,
                self.current_dpi,
                self.tracking_window,
            );
            self.border_drawer
                .border_radius
                .set(radius)
                .unwrap_or_else(|err| debug!("border_radius: {err}")); // non-critical, so debug

            is_updated = true;
        }

        is_updated
    }

    fn cleanup_and_queue_exit(&mut self) {
        self.is_paused = true;
        self.border_drawer.destroy_anims_timer();
        unsafe { PostQuitMessage(0) };
    }

    /// # Safety
    ///
    /// This is only here because clippy is throwing warnings at me lol. It's just a window
    /// procedure; don't use it for other things.
    pub unsafe extern "system" fn s_wnd_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // Retrieve the pointer to this WindowBorder struct using GWLP_USERDATA
        let mut border_pointer: *mut WindowBorder =
            unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as _;

        // If a pointer has not yet been assigned to GWLP_USERDATA, assign it here using the LPARAM
        // from CreateWindowExW
        if border_pointer.is_null() && message == WM_CREATE {
            let create_struct: *mut CREATESTRUCTW = lparam.0 as *mut _;
            border_pointer = unsafe { (*create_struct).lpCreateParams } as *mut _;
            unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, border_pointer as _) };
        }

        match !border_pointer.is_null() {
            true => unsafe { (*border_pointer).wnd_proc(window, message, wparam, lparam) },
            false => unsafe { DefWindowProcW(window, message, wparam, lparam) },
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
                // This is here to prevent LOCATIONCHANGE events from being handled before
                // MINIMIZEEND or SHOW/UNCLOAKED events.
                if self.is_paused {
                    return LRESULT(0);
                }

                // This is mainly here to handle cases where a window is made borderless fullscreen
                // (if 'follow_native_border' is set to true in the config), which doesn't have any
                // dedicated events like MINIMIZEEND. Note that we don't set 'is_paused' to true as
                // doing so would prevent us from handling the transition back to a regular window.
                if !self.should_show_border() {
                    self.update_position(Some(SWP_HIDEWINDOW)).log_if_err();
                    self.border_drawer.destroy_anims_timer();

                    return LRESULT(0);
                }

                let prev_rect = self.window_rect;
                self.update_window_rect().log_if_err();

                let update_pos_flags =
                    (!is_window_visible(self.border_window)).then_some(SWP_SHOWWINDOW);
                self.update_position(update_pos_flags).log_if_err();

                let mut needs_render = !are_rects_same_size(&self.window_rect, &prev_rect);

                let new_monitor = monitor_from_window(self.tracking_window);
                if new_monitor != self.current_monitor {
                    self.current_monitor = new_monitor;
                    debug!("monitor has changed! new monitor: {new_monitor:?}");

                    needs_render |=
                        match self.rescale_border_and_resize_renderer_if_needed(new_monitor) {
                            Ok(is_updated) => is_updated,
                            Err(err) => {
                                error!("could not update appearance and renderer: {err}");
                                return LRESULT(0);
                            }
                        };
                }

                needs_render |= self.sync_radius_for_snapped_state();

                if needs_render {
                    self.render().log_if_err();
                }

                self.border_drawer
                    .set_anims_timer_if_needed(self.border_window);
            }
            // EVENT_OBJECT_REORDER
            WM_APP_REORDER => match self.border_z_order {
                ZOrderMode::AboveWindow => {
                    // When the tracking window reorders its contents, it may change the z-order. So,
                    // we first check whether the border is still above the tracking window, and if
                    // not, we must update its position and place it back on top
                    if unsafe { GetWindow(self.tracking_window, GW_HWNDPREV) }
                        != Ok(self.border_window)
                    {
                        self.update_position(None).log_if_err();
                    }
                }
                ZOrderMode::BelowWindow => {} // do nothing
            },
            // EVENT_SYSTEM_FOREGROUND
            WM_APP_FOREGROUND => {
                self.update_color(None);
                self.update_position(None).log_if_err();
                self.render().log_if_err();
            }
            // EVENT_OBJECT_SHOW / EVENT_OBJECT_UNCLOAKED
            // NOTE: This message can still be sent while the window is minimized.
            WM_APP_SHOWUNCLOAKED => {
                if !is_window_visible(self.tracking_window)
                    || is_window_cloaked(self.tracking_window)
                    || is_window_minimized(self.tracking_window)
                {
                    return LRESULT(0);
                }

                if self.should_show_border() {
                    self.update_color(None);
                    self.update_window_rect().log_if_err();
                    self.update_position(Some(SWP_SHOWWINDOW)).log_if_err();
                    self.render().log_if_err();

                    self.border_drawer
                        .set_anims_timer_if_needed(self.border_window);
                }

                self.is_paused = false;
            }
            // EVENT_OBJECT_HIDE / EVENT_OBJECT_CLOAKED
            WM_APP_HIDECLOAKED => {
                self.update_position(Some(SWP_HIDEWINDOW)).log_if_err();
                self.border_drawer.destroy_anims_timer();
                self.is_paused = true;
            }
            // EVENT_OBJECT_MINIMIZESTART
            WM_APP_MINIMIZESTART => {
                self.update_position(Some(SWP_HIDEWINDOW)).log_if_err();

                // Needed for the fade animation to work correctly when window is unminimized
                self.border_drawer
                    .active_color
                    .set_opacity(0.0)
                    .log_if_err();
                self.border_drawer
                    .inactive_color
                    .set_opacity(0.0)
                    .log_if_err();

                self.border_drawer.destroy_anims_timer();
                self.is_paused = true;
            }
            // EVENT_SYSTEM_MINIMIZEEND
            WM_APP_MINIMIZEEND => {
                if !is_window_visible(self.tracking_window)
                    || is_window_cloaked(self.tracking_window)
                    || is_window_minimized(self.tracking_window)
                {
                    return LRESULT(0);
                }

                // Keep the border hidden while the tracking window is in its unminimize animation
                thread::sleep(time::Duration::from_millis(self.unminimize_delay));

                if self.should_show_border() {
                    self.update_color(Some(self.unminimize_delay));
                    self.update_window_rect().log_if_err();
                    self.update_position(Some(SWP_SHOWWINDOW)).log_if_err();
                    self.render().log_if_err();

                    self.border_drawer
                        .set_anims_timer_if_needed(self.border_window);
                }

                self.is_paused = false;
            }
            WM_APP_ANIMATE => {
                if self.is_paused {
                    return LRESULT(0);
                }

                self.border_drawer
                    .animate(&self.window_rect, self.window_padding, self.window_state)
                    .log_if_err();
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

                let window_kind = {
                    let komorebi_integration = APP_STATE.komorebi_integration.lock().unwrap();
                    let focus_state = komorebi_integration.focus_state.lock().unwrap();

                    *focus_state
                        .get(&(self.tracking_window.0 as isize))
                        .unwrap_or_else(|| {
                            error!("could not get window_kind for komorebi integration");
                            &WindowKind::Single
                        })
                };

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
                    WindowKind::Single => active_color_config.to_color_brush(true),
                    WindowKind::Stack => komorebi_colors_config
                        .stack_color
                        .as_ref()
                        .unwrap_or(active_color_config)
                        .to_color_brush(true),
                    WindowKind::Monocle => komorebi_colors_config
                        .monocle_color
                        .as_ref()
                        .unwrap_or(active_color_config)
                        .to_color_brush(true),
                    WindowKind::Floating => komorebi_colors_config
                        .floating_color
                        .as_ref()
                        .unwrap_or(active_color_config)
                        .to_color_brush(true),
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
                self.render().log_if_err();
            }
            WM_PAINT => {
                let _ = unsafe { ValidateRect(Some(window), None) };
            }
            WM_NCDESTROY => {
                // We'll set GWLP_USERDATA to 0 so that the window procedure can't find the
                // border's pointer anymore, making it stop processing our custom messages.
                unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, 0) };
                self.cleanup_and_queue_exit();
            }
            // This message is sent when a display setting has changed (e.g. resolution change). It
            // is not sent when the window moves to a different monitor.
            WM_DISPLAYCHANGE => {
                // The LPARAM supposedly will contain the new? resolution of the primary display,
                // but it may not be relevant to our border window in a multi-monitor setup, so
                // we'll run our own tests to determine whether we actually need to update anything.
                let needs_render =
                    match self.rescale_border_and_resize_renderer_if_needed(self.current_monitor) {
                        Ok(is_updated) => is_updated,
                        Err(err) => {
                            error!("could not update appearance and renderer: {err}");
                            return LRESULT(0);
                        }
                    };

                if needs_render && is_window_visible(self.border_window) {
                    self.render().log_if_err();
                }
            }
            // Although we already check for DPI changes when the window moves between monitors,
            // it's possible for the DPI to change without moving to a different monitor, or
            // without even moving at all. That's why we still handle this message.
            WM_DPICHANGED => {
                // According to MSDN, the X-axis and Y-axis values for the new dpi should be
                // identical for Windows apps, so we'll just grab the X-axis value here
                let new_dpi = loword(wparam.0) as u32;
                if new_dpi != self.current_dpi {
                    self.current_dpi = new_dpi;
                    debug!("dpi has changed! new dpi: {new_dpi}");

                    self.rescale_border(new_dpi);

                    let (screen_width, screen_height) =
                        match get_monitor_resolution(self.current_monitor) {
                            Ok(resolution) => resolution,
                            Err(err) => {
                                error!("could not get monitor resolution: {err}");
                                return LRESULT(0);
                            }
                        };

                    self.resize_renderer(screen_width, screen_height)
                        .log_if_err();
                    self.render().log_if_err();
                }
            }
            // This message is sent when a device is added or removed to the system. AFAIK, it
            // doesn't directly have anything to do with GPU adapters, but we can still use it to
            // help detect adapter changes in specific scenarios (e.g. when a monitor is
            // connected/disconnected on an NVIDIA Optimus-supported laptop).
            WM_DEVICECHANGE if wparam.0 as u32 == DBT_DEVNODES_CHANGED => {
                if let Some(directx_devices) = APP_STATE.directx_devices.write().unwrap().as_mut()
                    && let Err(err) = directx_devices.recreate_if_needed()
                {
                    error!("could not recreate directx devices if needed: {err}");
                    self.cleanup_and_queue_exit();
                    return LRESULT(0);
                }
                if let Err(err) = self.recreate_drawer_if_needed() {
                    error!("could not recreate border drawer if needed: {err}");
                    self.cleanup_and_queue_exit();
                    return LRESULT(0);
                }
            }
            // This message is sent by the DisplayAdaptersWatcher
            WM_APP_RECREATE_DRAWER => {
                if let Err(err) = self.recreate_drawer_if_needed() {
                    error!("could not recreate border drawer if needed: {err}");
                    self.cleanup_and_queue_exit();
                }
            }
            // This message should let us know when the system enters/leaves sleep/hibernation
            WM_POWERBROADCAST => match wparam.0 as u32 {
                PBT_APMSUSPEND => {
                    debug!("system is suspending; uninitializing border drawer");
                    self.border_drawer.destroy_anims_timer();
                    self.border_drawer.uninit();
                }
                PBT_APMRESUMESUSPEND | PBT_APMRESUMEAUTOMATIC
                    if matches!(self.border_drawer.render_backend, RenderBackend::None) =>
                {
                    debug!("system is resuming; reinitializing border drawer");
                    let (screen_width, screen_height) =
                        match get_monitor_resolution(self.current_monitor) {
                            Ok(resolution) => resolution,
                            Err(err) => {
                                error!(
                                    "could not get monitor resolution in WM_POWERBROADCAST: {err}"
                                );
                                self.cleanup_and_queue_exit();
                                return LRESULT(0);
                            }
                        };
                    if let Err(err) = self.init_drawer(screen_width, screen_height) {
                        error!("could not initialize border drawer in WM_POWERBROADCAST: {err}");
                        self.cleanup_and_queue_exit();
                        return LRESULT(0);
                    };

                    if self.should_show_border() {
                        self.border_drawer
                            .set_anims_timer_if_needed(self.border_window);
                    }
                }
                _ => {}
            },
            // Ignore these window position messages
            WM_WINDOWPOSCHANGING | WM_WINDOWPOSCHANGED => {}
            _ => {
                return unsafe { DefWindowProcW(window, message, wparam, lparam) };
            }
        }
        LRESULT(0)
    }
}

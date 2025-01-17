use anyhow::{anyhow, Context};
use std::mem::ManuallyDrop;
use std::ptr;
use std::thread;
use std::time;
use windows::core::{w, PCWSTR};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{
    COLORREF, D2DERR_RECREATE_TARGET, FALSE, HWND, LPARAM, LRESULT, RECT, S_OK, TRUE, WPARAM,
};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_COMPOSITE_MODE_SOURCE_OVER,
    D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1AlphaMask, CLSID_D2D1Composite, CLSID_D2D1GaussianBlur, ID2D1Bitmap1, ID2D1Brush,
    ID2D1CommandList, ID2D1DeviceContext7, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_BRUSH_PROPERTIES, D2D1_COMBINE_MODE_XOR, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
    D2D1_DIRECTIONALBLUR_OPTIMIZATION_SPEED, D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION,
    D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, D2D1_INTERPOLATION_MODE_LINEAR,
    D2D1_PROPERTY_TYPE_ENUM, D2D1_PROPERTY_TYPE_FLOAT, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dwm::{
    DwmEnableBlurBehindWindow, DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS,
    DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIFactory7, IDXGISurface, IDXGISwapChain1, DXGI_PRESENT, DXGI_SCALING_STRETCH,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::{CreateRectRgn, ValidateRect, HMONITOR};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetSystemMetrics, GetWindow,
    GetWindowLongPtrW, PostQuitMessage, SetLayeredWindowAttributes, SetWindowLongPtrW,
    SetWindowPos, TranslateMessage, CREATESTRUCTW, CW_USEDEFAULT, GWLP_USERDATA, GW_HWNDPREV,
    HWND_TOP, LWA_ALPHA, MSG, SET_WINDOW_POS_FLAGS, SM_CXVIRTUALSCREEN, SWP_HIDEWINDOW,
    SWP_NOACTIVATE, SWP_NOREDRAW, SWP_NOSENDCHANGING, SWP_NOZORDER, SWP_SHOWWINDOW, WM_CREATE,
    WM_NCDESTROY, WM_PAINT, WM_WINDOWPOSCHANGED, WM_WINDOWPOSCHANGING, WS_DISABLED, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::animations::{self, AnimType, AnimVec, Animations};
use crate::border_config::WindowRule;
use crate::colors::Color;
use crate::komorebi::WindowKind;
use crate::utils::{
    are_rects_same_size, get_dpi_for_window, get_monitor_info, get_window_rule, get_window_title,
    has_native_border, is_rect_visible, is_window_minimized, is_window_visible,
    monitor_from_window, post_message_w, LogIfErr, WM_APP_ANIMATE, WM_APP_FOREGROUND,
    WM_APP_HIDECLOAKED, WM_APP_KOMOREBI, WM_APP_LOCATIONCHANGE, WM_APP_MINIMIZEEND,
    WM_APP_MINIMIZESTART, WM_APP_REORDER, WM_APP_SHOWUNCLOAKED,
};
use crate::APP_STATE;

const BLUR_EFFECT_STANDARD_DEVIATION: f32 = 8.0;

#[derive(Debug, Default)]
pub struct WindowBorder {
    pub border_window: HWND,
    pub tracking_window: HWND,
    pub is_active_window: bool,
    pub window_rect: RECT,
    pub window_padding: i32,
    pub render_rect: D2D1_ROUNDED_RECT,
    pub border_width: i32,
    pub border_offset: i32,
    pub border_radius: f32,
    pub current_monitor: HMONITOR,
    pub current_dpi: f32,
    pub d2d_context: Option<ID2D1DeviceContext7>,
    pub swap_chain: Option<IDXGISwapChain1>,
    pub target_bitmap: Option<ID2D1Bitmap1>,
    pub border_bitmap: Option<ID2D1Bitmap1>,
    pub mask_bitmap: Option<ID2D1Bitmap1>,
    pub command_list: Option<ID2D1CommandList>,
    pub d_comp_device: Option<IDCompositionDevice>,
    pub d_comp_target: Option<IDCompositionTarget>,
    pub d_comp_visual: Option<IDCompositionVisual>,
    pub active_color: Color,
    pub inactive_color: Color,
    pub animations: Animations,
    pub last_render_time: Option<time::Instant>,
    pub last_anim_time: Option<time::Instant>,
    pub initialize_delay: u64,
    pub unminimize_delay: u64,
    pub is_paused: bool,
}

impl WindowBorder {
    pub fn new(tracking_window: HWND) -> Self {
        Self {
            tracking_window,
            ..Default::default()
        }
    }

    pub fn create_window(&mut self) -> windows::core::Result<()> {
        let title: Vec<u16> = format!(
            "tacky-border | {} | {:?}\0",
            get_window_title(self.tracking_window).unwrap_or_default(),
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
                None,
                Some(ptr::addr_of!(*self) as _),
            )?;
        }

        Ok(())
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

            self.create_render_resources()
                .context("could not create render resources in init()")?;

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

            // TODO: testing; remove when done
            self.update_render_resources().log_if_err();

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

        // TODO: change this to smth else
        // 3 standard deviations gets us 99.7% coverage, which should be good enough
        self.window_padding = (BLUR_EFFECT_STANDARD_DEVIATION * 3.0) as i32;

        self.active_color = active_color_config.to_color(true);
        self.inactive_color = inactive_color_config.to_color(false);

        self.current_monitor = monitor_from_window(self.tracking_window);
        self.current_dpi = match get_dpi_for_window(self.tracking_window) {
            Ok(dpi) => dpi as f32,
            Err(err) => {
                self.cleanup_and_queue_exit();
                return Err(anyhow!("could not get dpi for window: {err}"));
            }
        };

        // Adjust the border width and radius based on the window/monitor dpi
        self.border_width = (width_config * self.current_dpi / 96.0).round() as i32;
        self.border_offset = offset_config;
        self.border_radius =
            radius_config.to_radius(self.border_width, self.current_dpi, self.tracking_window);

        self.animations = animations_config.to_animations();

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

    fn create_render_resources(&mut self) -> anyhow::Result<()> {
        let d2d_context = unsafe {
            APP_STATE
                .d2d_device
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
        }
        .context("d2d_context")?;

        unsafe { d2d_context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };

        let m_info = get_monitor_info(self.current_monitor).context("mi")?;
        let screen_width = (m_info.rcMonitor.right - m_info.rcMonitor.left) as u32;
        let screen_height = (m_info.rcMonitor.bottom - m_info.rcMonitor.top) as u32;

        let swap_chain_desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: screen_width + ((self.border_width + self.window_padding) * 2) as u32,
            Height: screen_height + ((self.border_width + self.window_padding) * 2) as u32,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: FALSE,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };

        unsafe {
            let dxgi_adapter = APP_STATE.dxgi_device.GetAdapter().context("dxgi_adapter")?;
            let dxgi_factory: IDXGIFactory7 = dxgi_adapter.GetParent().context("dxgi_factory")?;

            let swap_chain = dxgi_factory
                .CreateSwapChainForComposition(&APP_STATE.device, &swap_chain_desc, None)
                .context("swap_chain")?;

            let d_comp_device: IDCompositionDevice =
                DCompositionCreateDevice(&APP_STATE.dxgi_device)?;
            let d_comp_target = d_comp_device
                .CreateTargetForHwnd(self.border_window, true)
                .context("d_comp_target")?;
            let d_comp_visual = d_comp_device.CreateVisual().context("visual")?;

            d_comp_visual
                .SetContent(&swap_chain)
                .context("d_comp_visual.SetContent()")?;
            d_comp_target
                .SetRoot(&d_comp_visual)
                .context("d_comp_target.SetRoot()")?;
            d_comp_device.Commit().context("d_comp_device.Commit()")?;

            // We move these vars into self here even though create_bitmaps() needs some of them
            // because Rust borrow checker be mad elsewhere in the code :P
            self.d2d_context = Some(d2d_context);
            self.swap_chain = Some(swap_chain);
            self.d_comp_device = Some(d_comp_device);
            self.d_comp_target = Some(d_comp_target);
            self.d_comp_visual = Some(d_comp_visual);
        }

        // So instead of passing ^ args, we retrieve the above vars from self within below function
        self.create_bitmaps(screen_width, screen_height)
            .context("could not create bitmaps and effects")?;

        self.create_command_list()
            .context("could not create command list")?;

        // We will adjust opacity later. For now, we set it to 0.
        let brush_properties = D2D1_BRUSH_PROPERTIES {
            opacity: 0.0,
            transform: Matrix3x2::identity(),
        };

        self.render_rect = D2D1_ROUNDED_RECT {
            rect: Default::default(),
            radiusX: self.border_radius,
            radiusY: self.border_radius,
        };

        // Get d2d_context again because Rust borrow checker earlier
        let d2d_context = self
            .d2d_context
            .as_ref()
            .context("could not get d2d_context")?;

        self.active_color
            .init_brush(d2d_context, &self.window_rect, &brush_properties)
            .log_if_err();
        self.inactive_color
            .init_brush(d2d_context, &self.window_rect, &brush_properties)
            .log_if_err();

        Ok(())
    }

    fn create_bitmaps(&mut self, screen_width: u32, screen_height: u32) -> anyhow::Result<()> {
        let d2d_context = self
            .d2d_context
            .as_ref()
            .context("could not get d2d_context")?;
        let swap_chain = self
            .swap_chain
            .as_ref()
            .context("could not get swap_chain")?;

        let bitmap_properties = D2D1_BITMAP_PROPERTIES1 {
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            colorContext: ManuallyDrop::new(None),
        };

        let dxgi_back_buffer: IDXGISurface =
            unsafe { swap_chain.GetBuffer(0) }.context("dxgi_back_buffer")?;

        let target_bitmap = unsafe {
            d2d_context.CreateBitmapFromDxgiSurface(&dxgi_back_buffer, Some(&bitmap_properties))
        }
        .context("d2d_target_bitmap")?;

        unsafe { d2d_context.SetTarget(&target_bitmap) };

        // We create two bitmaps because the first (target_bitmap) cannot be used for effects
        let bitmap_properties = D2D1_BITMAP_PROPERTIES1 {
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            colorContext: ManuallyDrop::new(None),
        };
        let border_bitmap = unsafe {
            d2d_context.CreateBitmap(
                D2D_SIZE_U {
                    width: screen_width + ((self.border_width + self.window_padding) * 2) as u32,
                    height: screen_height + ((self.border_width + self.window_padding) * 2) as u32,
                },
                None,
                0,
                &bitmap_properties,
            )
        }
        .context("border_bitmap")?;

        // Aaaand yet another for the mask
        let mask_bitmap = unsafe {
            d2d_context.CreateBitmap(
                D2D_SIZE_U {
                    width: screen_width + ((self.border_width + self.window_padding) * 2) as u32,
                    height: screen_height + ((self.border_width + self.window_padding) * 2) as u32,
                },
                None,
                0,
                &bitmap_properties,
            )
        }
        .context("mask_bitmap")?;

        self.target_bitmap = Some(target_bitmap);
        self.border_bitmap = Some(border_bitmap);
        self.mask_bitmap = Some(mask_bitmap);

        Ok(())
    }

    fn create_command_list(&mut self) -> anyhow::Result<()> {
        let d2d_context = self
            .d2d_context
            .as_ref()
            .context("could not get d2d_context")?;
        let border_bitmap = self
            .border_bitmap
            .as_ref()
            .context("could not get border_bitmap")?;
        let mask_bitmap = self
            .mask_bitmap
            .as_ref()
            .context("could not get mask_bitmap")?;

        unsafe {
            // Open a command list to record draw operations
            let command_list = d2d_context
                .CreateCommandList()
                .context("d2d_context.CreateCommandList()")?;

            // Set the command list as the target so we can begin recording
            d2d_context.SetTarget(&command_list);

            // Create the blur effect and link it to the border_bitmap
            let blur_effect = d2d_context
                .CreateEffect(&CLSID_D2D1GaussianBlur)
                .context("blur_effect")?;
            blur_effect.SetInput(0, border_bitmap, false);
            blur_effect
                .SetValue(
                    D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
                    D2D1_PROPERTY_TYPE_FLOAT,
                    &BLUR_EFFECT_STANDARD_DEVIATION.to_le_bytes(),
                )
                .context("blur_effect.SetValue() std deviation")?;
            blur_effect
                .SetValue(
                    D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION.0 as u32,
                    D2D1_PROPERTY_TYPE_ENUM,
                    &D2D1_DIRECTIONALBLUR_OPTIMIZATION_SPEED.0.to_le_bytes(),
                )
                .context("blur_effect.SetValue() optimization")?;

            // Create an alpha mask effect to mask out the inner glow
            let mask_effect = d2d_context
                .CreateEffect(&CLSID_D2D1AlphaMask)
                .context("mask_effect")?;
            mask_effect.SetInput(
                0,
                &blur_effect
                    .GetOutput()
                    .context("could not get blur output")?,
                false,
            );
            mask_effect.SetInput(1, mask_bitmap, false);

            // Create a composite effect and link it to the effect(s) and border_bitmap
            let composite_effect = d2d_context
                .CreateEffect(&CLSID_D2D1Composite)
                .context("composite_effect")?;
            composite_effect.SetInput(
                0,
                &mask_effect
                    .GetOutput()
                    .context("could not get mask output")?,
                false,
            );
            composite_effect.SetInput(1, border_bitmap, false);

            d2d_context.BeginDraw();
            d2d_context.Clear(None);

            // Draw the composite effect (recorded by the command list)
            d2d_context.DrawImage(
                &composite_effect
                    .GetOutput()
                    .context("could not get composite output")?,
                None,
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_OVER,
            );

            d2d_context
                .EndDraw(None, None)
                .unwrap_or_else(|err| self.handle_end_draw_error(err));

            // Close the command list to tell it we are done recording
            command_list.Close().context("command_list.Close()")?;

            self.command_list = Some(command_list);
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
            self.cleanup_and_queue_exit();
            return Err(e);
        }

        let adjustment = self.border_width + self.window_padding;
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
        self.is_active_window =
            self.tracking_window.0 as isize == *APP_STATE.active_window.lock().unwrap();

        match animations::get_current_anims(self).contains_type(AnimType::Fade) {
            false => self.update_brush_opacities(),
            true if check_delay == Some(0) => {
                self.update_brush_opacities();
                animations::update_fade_progress(self)
            }
            true => self.animations.should_fade = true,
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

    fn update_width_radius(&mut self) {
        let window_rule = get_window_rule(self.tracking_window);
        let config = APP_STATE.config.read().unwrap();
        let global = &config.global;

        let width_config = window_rule.border_width.unwrap_or(global.border_width);
        let radius_config = window_rule
            .border_radius
            .as_ref()
            .unwrap_or(&global.border_radius);

        self.border_width = (width_config * self.current_dpi / 96.0).round() as i32;
        self.border_radius =
            radius_config.to_radius(self.border_width, self.current_dpi, self.tracking_window);
    }

    fn update_render_resources(&mut self) -> anyhow::Result<()> {
        let d2d_context = self
            .d2d_context
            .as_ref()
            .context("could not get d2d_context")?;
        let swap_chain = self
            .swap_chain
            .as_ref()
            .context("could not get swap_chain")?;

        // Release buffer references
        unsafe { d2d_context.SetTarget(None) };
        self.target_bitmap = None;

        let m_info = get_monitor_info(self.current_monitor).context("mi")?;
        let screen_width = (m_info.rcMonitor.right - m_info.rcMonitor.left) as u32;
        let screen_height = (m_info.rcMonitor.bottom - m_info.rcMonitor.top) as u32;

        unsafe {
            swap_chain.ResizeBuffers(
                2,
                screen_width + ((self.border_width + self.window_padding) * 2) as u32,
                screen_height + ((self.border_width + self.window_padding) * 2) as u32,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_SWAP_CHAIN_FLAG::default(),
            )
        }
        .context("swap_chain.ResizeBuffers()")?;

        self.create_bitmaps(screen_width, screen_height)
            .context("could not create bitmaps and effects")?;

        self.create_command_list()
            .context("could not create command list")?;

        Ok(())
    }

    fn render(&mut self) -> anyhow::Result<()> {
        self.last_render_time = Some(time::Instant::now());

        let d2d_context = self
            .d2d_context
            .as_ref()
            .context("could not get d2d_context")?;

        let border_width = self.border_width as f32;
        let border_offset = self.border_offset as f32;
        let window_padding = self.window_padding as f32;

        self.render_rect.rect = D2D_RECT_F {
            left: border_width / 2.0 + window_padding - border_offset,
            top: border_width / 2.0 + window_padding - border_offset,
            right: (self.window_rect.right - self.window_rect.left) as f32
                - border_width / 2.0
                - window_padding
                + border_offset,
            bottom: (self.window_rect.bottom - self.window_rect.top) as f32
                - border_width / 2.0
                - window_padding
                + border_offset,
        };

        unsafe {
            // Determine which color should be drawn on top (for color fade animation)
            let (bottom_color, top_color) = match self.is_active_window {
                true => (&self.inactive_color, &self.active_color),
                false => (&self.active_color, &self.inactive_color),
            };

            // Set the d2d_context target to the border_bitmap
            let border_bitmap = self
                .border_bitmap
                .as_ref()
                .context("could not get border_bitmap")?;
            d2d_context.SetTarget(border_bitmap);

            // Draw to the border_bitmap
            d2d_context.BeginDraw();
            d2d_context.Clear(None);

            if bottom_color.get_opacity() > Some(0.0) {
                if let Color::Gradient(gradient) = bottom_color {
                    gradient.update_start_end_points(&self.window_rect);
                }

                match bottom_color.get_brush() {
                    Some(id2d1_brush) => self.draw_rectangle(d2d_context, id2d1_brush),
                    None => debug!("ID2D1Brush for bottom_color has not been created yet"),
                }
            }
            if top_color.get_opacity() > Some(0.0) {
                if let Color::Gradient(gradient) = top_color {
                    gradient.update_start_end_points(&self.window_rect);
                }

                match top_color.get_brush() {
                    Some(id2d1_brush) => self.draw_rectangle(d2d_context, id2d1_brush),
                    None => debug!("ID2D1Brush for top_color has not been created yet"),
                }
            }

            d2d_context
                .EndDraw(None, None)
                .unwrap_or_else(|err| self.handle_end_draw_error(err));

            // Get d2d_context again to satisfy Rust's borrow checker
            let d2d_context = self
                .d2d_context
                .as_ref()
                .context("could not get d2d_context")?;

            // Set the d2d_context target to the mask_bitmap so we can create an alpha mask
            let mask_bitmap = self
                .mask_bitmap
                .as_ref()
                .context("could not get target_bitmap")?;
            d2d_context.SetTarget(mask_bitmap);

            // Create our mask geometry (masks out inner glow/blur)
            let render_rect_geometry = APP_STATE
                .factory
                .CreateRoundedRectangleGeometry(&self.render_rect)
                .context("render_rect_geometry")?;
            let window_rect_geometry = APP_STATE
                .factory
                .CreateRectangleGeometry(&D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: (self.window_rect.right - self.window_rect.left) as f32,
                    bottom: (self.window_rect.bottom - self.window_rect.top) as f32,
                })
                .context("window_rect_geometry")?;

            // Combine the two geometries
            let path_geometry = APP_STATE
                .factory
                .CreatePathGeometry()
                .context("path_geometry")?;
            let geometry_sink = path_geometry.Open().context("geometry_sink")?;
            render_rect_geometry
                .CombineWithGeometry(
                    &window_rect_geometry,
                    D2D1_COMBINE_MODE_XOR,
                    None,
                    0.5,
                    &geometry_sink,
                )
                .context("render_rect_geometry.CombineWithGeometry()")?;
            geometry_sink.Close().context("geometry_sink.Close()")?;

            // Create a 100% opaque brush because our active/inactive colors' brushes might not be
            let opaque_brush = d2d_context
                .CreateSolidColorBrush(
                    &D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                    None,
                )
                .context("opaque_brush")?;

            // Draw to the mask_bitmap
            d2d_context.BeginDraw();
            d2d_context.Clear(None);

            d2d_context.FillGeometry(&path_geometry, &opaque_brush, None);

            d2d_context
                .EndDraw(None, None)
                .unwrap_or_else(|err| self.handle_end_draw_error(err));

            // Get d2d_context again to satisfy Rust's borrow checker
            let d2d_context = self
                .d2d_context
                .as_ref()
                .context("could not get d2d_context")?;

            // Set d2d_context's target back to the target_bitmap so we can draw to the display
            let target_bitmap = self
                .target_bitmap
                .as_ref()
                .context("could not get target_bitmap")?;
            d2d_context.SetTarget(target_bitmap);

            // Retrieve our command list (includes border_bitmap, mask_bitmap, and effects)
            let command_list = self
                .command_list
                .as_ref()
                .context("could not get command_list")?;

            // Draw to the target_bitmap
            d2d_context.BeginDraw();
            d2d_context.Clear(None);

            // Draw using the command list
            d2d_context.DrawImage(
                command_list,
                None,
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_OVER,
            );

            d2d_context
                .EndDraw(None, None)
                .unwrap_or_else(|err| self.handle_end_draw_error(err));

            // Present the swap chain buffer
            let hresult = self
                .swap_chain
                .as_ref()
                .context("could not get swap_chain")?
                .Present(1, DXGI_PRESENT::default());
            if hresult != S_OK {
                return Err(anyhow!("could not present swap_chain: {hresult}"));
            }
        }

        Ok(())
    }

    fn draw_rectangle(&self, d2d_context: &ID2D1DeviceContext7, brush: &ID2D1Brush) {
        unsafe {
            match self.border_radius {
                0.0 => d2d_context.DrawRectangle(
                    &self.render_rect.rect,
                    brush,
                    self.border_width as f32,
                    None,
                ),
                _ => d2d_context.DrawRoundedRectangle(
                    &self.render_rect,
                    brush,
                    self.border_width as f32,
                    None,
                ),
            }
        }
    }

    fn handle_end_draw_error(&mut self, err: windows::core::Error) {
        if err.code() == D2DERR_RECREATE_TARGET {
            // D2DERR_RECREATE_TARGET is recoverable if we just recreate the render target.
            // This error can be caused by things like waking up from sleep, updating GPU
            // drivers, changing screen resolution, etc.
            warn!("render target has been lost; attempting to recreate");

            match self.create_render_resources() {
                Ok(_) => info!("successfully recreated render target; resuming thread"),
                Err(err_2) => {
                    error!("could not recreate render target; exiting thread: {err_2}");
                    self.cleanup_and_queue_exit();
                }
            }
        } else {
            error!("d2d_context.EndDraw() failed; exiting thread: {err}");
            self.cleanup_and_queue_exit();
        }
    }

    fn cleanup_and_queue_exit(&mut self) {
        self.is_paused = true;
        animations::destroy_timer(self);
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
                    self.update_render_resources()
                        .context("could not update render resources")
                        .log_if_err();

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

                    should_render |= true;
                }

                if should_render {
                    self.render().log_if_err();
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

                animations::set_timer_if_anims_enabled(self);
                self.is_paused = false;
            }
            // EVENT_OBJECT_HIDE / EVENT_OBJECT_CLOAKED
            WM_APP_HIDECLOAKED => {
                self.update_position(Some(SWP_HIDEWINDOW)).log_if_err();
                animations::destroy_timer(self);
                self.is_paused = true;
            }
            // EVENT_OBJECT_MINIMIZESTART
            WM_APP_MINIMIZESTART => {
                self.update_position(Some(SWP_HIDEWINDOW)).log_if_err();

                self.active_color.set_opacity(0.0);
                self.inactive_color.set_opacity(0.0);

                animations::destroy_timer(self);
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

                animations::set_timer_if_anims_enabled(self);
                self.is_paused = false;
            }
            WM_APP_ANIMATE => {
                if self.is_paused {
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

                for anim_params in animations::get_current_anims(self).clone().iter() {
                    match anim_params.anim_type {
                        AnimType::Spiral => {
                            animations::animate_spiral(self, &anim_elapsed, anim_params, false);
                            update = true;
                        }
                        AnimType::ReverseSpiral => {
                            animations::animate_spiral(self, &anim_elapsed, anim_params, true);
                            update = true;
                        }
                        AnimType::Fade => {
                            if self.animations.should_fade {
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

                let old_opacity = self.active_color.get_opacity().unwrap_or_default();
                let old_transform = self.active_color.get_transform().unwrap_or_default();

                self.active_color = match window_kind {
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

                let Some(ref d2d_context) = self.d2d_context else {
                    error!("render target has not been set yet");
                    return LRESULT(0);
                };

                let brush_properties = D2D1_BRUSH_PROPERTIES {
                    opacity: old_opacity,
                    transform: old_transform,
                };

                self.active_color
                    .init_brush(d2d_context, &self.window_rect, &brush_properties)
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

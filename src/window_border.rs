// TODO Add result handling. There's so many let _ = lmao it's so bad.
use std::sync::LazyLock;
use std::sync::OnceLock;
use windows::{
    core::*,
    Foundation::Numerics::*,
    Win32::Foundation::*,
    Win32::Graphics::Gdi::*,
    Win32::Graphics::Dwm::*,
    Win32::Graphics::Direct2D::*,
    Win32::Graphics::Direct2D::Common::*,
    Win32::Graphics::Dxgi::Common::*,
    Win32::UI::WindowsAndMessaging::*,
};
use crate::utils::*;

pub static RENDER_FACTORY: LazyLock<ID2D1Factory> = unsafe { LazyLock::new(|| 
    D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_MULTI_THREADED, None).expect("creating RENDER_FACTORY failed")) 
};

#[derive(Debug, Default)]
pub struct WindowBorder {
    pub border_window: HWND,
    pub tracking_window: HWND,
    pub window_rect: RECT,
    pub border_size: i32,
    pub border_offset: i32,
    pub force_border_radius: f32,
    pub dpi: f32,
    pub render_target_properties: D2D1_RENDER_TARGET_PROPERTIES,
    pub hwnd_render_target_properties: D2D1_HWND_RENDER_TARGET_PROPERTIES,
    pub render_target: OnceLock<ID2D1HwndRenderTarget>,
    pub border_brush: D2D1_BRUSH_PROPERTIES,
    pub rounded_rect: D2D1_ROUNDED_RECT,
    pub active_color: D2D1_COLOR_F,
    pub inactive_color: D2D1_COLOR_F,
    pub current_color: D2D1_COLOR_F,
    pub tracking_is_minimized: bool,
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
                Some(std::ptr::addr_of!(*self) as *const _)
            )?;

            let dpi_aware = SetProcessDPIAware();
            if !dpi_aware.as_bool() {
                println!("Failed to make process DPI aware");
            }
        }

        Ok(())
    }

    pub fn init(&mut self) -> Result<()> {
        unsafe {
            // Make the window border transparent 
            let pos: i32 = -GetSystemMetrics(SM_CXVIRTUALSCREEN) - 8;
            let hrgn = CreateRectRgn(pos, 0, pos + 1, 1);
            let mut bh: DWM_BLURBEHIND = Default::default();
            if !hrgn.is_invalid() {
                bh = DWM_BLURBEHIND {
                    dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
                    fEnable: TRUE,
                    hRgnBlur: hrgn,
                    fTransitionOnMaximized: FALSE
                };
            }

            let _ = DwmEnableBlurBehindWindow(self.border_window, &bh);
            if SetLayeredWindowAttributes(self.border_window, COLORREF(0x00000000), 0, LWA_COLORKEY).is_err() {
                println!("Error Setting Layered Window Attributes!");
            }
            if SetLayeredWindowAttributes(self.border_window, COLORREF(0x00000000), 255, LWA_ALPHA).is_err() {
                println!("Error Setting Layered Window Attributes!");
            }
            
            let _ = self.create_render_targets();
            if has_native_border(self.tracking_window) {
                let _ = self.update_position(Some(SWP_SHOWWINDOW));
                let _ = self.render();
            }

            let mut message = MSG::default();
            while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }

        return Ok(());
    }

    pub fn create_render_targets(&mut self) -> Result<()> {
        self.dpi = 96.0;
        self.render_target_properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT { 
                format: DXGI_FORMAT_UNKNOWN, 
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED 
            },
            dpiX: self.dpi,
            dpiY: self.dpi,
            ..Default::default()
        };
        self.hwnd_render_target_properties = D2D1_HWND_RENDER_TARGET_PROPERTIES { 
            hwnd: self.border_window, 
            pixelSize: Default::default(), 
            presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY 
        };
        self.border_brush = D2D1_BRUSH_PROPERTIES { 
            opacity: 1.0 as f32, 
            transform: Matrix3x2::identity() 
        };

        // Create a rounded_rect with radius depending on the force_border_radius variable 
        let mut border_radius = 0.0;
        let mut corner_preference = DWM_WINDOW_CORNER_PREFERENCE::default();
        if self.force_border_radius == -1.0 {
            let result = unsafe { DwmGetWindowAttribute(
                self.tracking_window,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                std::ptr::addr_of_mut!(corner_preference) as *mut _,
                size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32
            ) }; 
            if result.is_err() {
                println!("Error getting window corner preference!");
            }
            match corner_preference.0 {
                0 => border_radius = 6.0 + ((self.border_size/2) as f32),
                1 => border_radius = 0.0,
                2 => border_radius = 6.0 + ((self.border_size/2) as f32),
                3 => border_radius = 3.0 + ((self.border_size/2) as f32),
                _ => {}
            }
        } else {
            border_radius = self.force_border_radius;
        }
        
        self.rounded_rect = D2D1_ROUNDED_RECT { 
            rect: Default::default(), 
            radiusX: border_radius, 
            radiusY: border_radius 
        };

        // Initialize the actual border color assuming it is in focus
        self.current_color = self.active_color;

        unsafe {
            let factory = &*RENDER_FACTORY;
            let _ = self.render_target.set(
                factory.CreateHwndRenderTarget(&self.render_target_properties, &self.hwnd_render_target_properties).expect("creating self.render_target failed")
            );
            let render_target = self.render_target.get().unwrap();
            render_target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }

        let _ = self.update_color();
        let _ = self.update_window_rect();
        let _ = self.update_position(None);

        return Ok(());
    }

    pub fn update_window_rect(&mut self) -> Result<()> {
        // TODO fix render issue (read further below)
        let old_rect = self.window_rect.clone();

        let result = unsafe { DwmGetWindowAttribute(
            self.tracking_window, 
            DWMWA_EXTENDED_FRAME_BOUNDS,
            std::ptr::addr_of_mut!(self.window_rect) as *mut _,
            size_of::<RECT>() as u32
        ) }; 
        if result.is_err() {
            println!("Error getting frame rect!");
            unsafe { let _ = ShowWindow(self.border_window, SW_HIDE); }
        }

        // TODO When a window is minimized, all four of these points go far below 0, and for some
        // reason, render() will sometimes render at this minimized size, even when the
        // render_target size and rounded_rect.rect are changed correctly after an
        // update_window_rect call. So, this is a temporary solution but it should absolutely be
        // looked further into.
        if self.window_rect.top <= 0
        && self.window_rect.left <= 0
        && self.window_rect.right <= 0
        && self.window_rect.bottom <= 0 {
            self.window_rect = old_rect;
            return Ok(());
        }

        self.window_rect.top -= self.border_size;
        self.window_rect.left -= self.border_size;
        self.window_rect.right += self.border_size;
        self.window_rect.bottom += self.border_size;

        return Ok(());
    }

    pub fn update_position(&mut self, c_flags: Option<SET_WINDOW_POS_FLAGS>) -> Result<()> {
        unsafe {
            // Place the window border above the tracking window
            let mut hwnd_above_tracking = GetWindow(self.tracking_window, GW_HWNDPREV);
            let custom_flags = match c_flags {
                Some(flags) => flags,
                None => SET_WINDOW_POS_FLAGS::default(),
            };
            let mut u_flags = SWP_NOSENDCHANGING | SWP_NOACTIVATE | SWP_NOREDRAW | custom_flags;

            // If hwnd_above_tracking is the window border itself, we have what we want and there's
            //  no need to change the z-order (plus it results in an error if we try it). 
            // If hwnd_above_tracking returns an error, it's likely that tracking_window is already
            //  the highest in z-order, so we use HWND_TOP to place the window border above.
            if hwnd_above_tracking == Ok(self.border_window) {
                u_flags = u_flags | SWP_NOZORDER;
            } else if hwnd_above_tracking.is_err() {
                hwnd_above_tracking = Ok(HWND_TOP);
            }

            let result = SetWindowPos(self.border_window,
                hwnd_above_tracking.unwrap(),
                self.window_rect.left,
                self.window_rect.top,
                self.window_rect.right - self.window_rect.left,
                self.window_rect.bottom - self.window_rect.top,
                u_flags 
            );
            if result.is_err() {
                println!("Error setting window pos!");
                let _ = ShowWindow(self.border_window, SW_HIDE);
            }
        }
        return Ok(());
    }

    pub fn update_color(&mut self) -> Result<()> {
        if is_active_window(self.tracking_window) {
            self.current_color = self.active_color;
        } else {
            self.current_color = self.inactive_color; 
        }
        return Ok(());
    }

    pub fn render(&mut self) -> Result<()> {
        // Get the render target
        let render_target_option = self.render_target.get();
        if render_target_option.is_none() {
            return Ok(()); 
        }
        let render_target = render_target_option.unwrap();

        self.hwnd_render_target_properties.pixelSize = D2D_SIZE_U { 
            width: (self.window_rect.right - self.window_rect.left) as u32,
            height: (self.window_rect.bottom - self.window_rect.top) as u32
        };

        self.rounded_rect.rect = D2D_RECT_F { 
            left: (self.border_size/2 - self.border_offset) as f32, 
            top: (self.border_size/2 - self.border_offset) as f32, 
            right: (self.window_rect.right - self.window_rect.left - self.border_size/2 + self.border_offset) as f32, 
            bottom: (self.window_rect.bottom - self.window_rect.top - self.border_size/2 + self.border_offset) as f32
        };

        unsafe {
            let _ = render_target.Resize(&self.hwnd_render_target_properties.pixelSize as *const _);

            let brush = render_target.CreateSolidColorBrush(&self.current_color, Some(&self.border_brush))?;

            render_target.BeginDraw();
            render_target.Clear(None);
            render_target.DrawRoundedRectangle(
                &self.rounded_rect,
                &brush,
                self.border_size as f32,
                None
            );
            let _ = render_target.EndDraw(None, None);
        }
        return Ok(());
    }

    // When CreateWindowExW is called, we can optionally pass a value to its LPARAM field which will
    // get sent to the window process on creation. In our code, we've passed a pointer to the
    // WindowBorder structure during the window creation process, and here we are getting that pointer 
    // and attaching it to the window using SetWindowLongPtrW.
    pub unsafe extern "system" fn s_wnd_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let mut border_pointer: *mut WindowBorder = GetWindowLongPtrW(window, GWLP_USERDATA) as _;
        
        if border_pointer == std::ptr::null_mut() && message == WM_CREATE {
            let create_struct: *mut CREATESTRUCTW = lparam.0 as *mut _;
            border_pointer = (*create_struct).lpCreateParams as *mut _;
            SetWindowLongPtrW(window, GWLP_USERDATA, border_pointer as _);
        }
        match border_pointer != std::ptr::null_mut() {
            true => return Self::wnd_proc(&mut *border_pointer, window, message, wparam, lparam),
            false => return DefWindowProcW(window, message, wparam, lparam),
        }                                          
    }

    pub unsafe fn wnd_proc(&mut self, window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match message {
            // EVENT_OBJECT_LOCATIONCHANGE
            5000 => {
                if self.tracking_is_minimized || is_cloaked(self.tracking_window) || !is_window_visible(self.tracking_window) {
                    return LRESULT(0);
                }

                if !has_native_border(self.tracking_window) {
                    let _ = self.update_position(Some(SWP_HIDEWINDOW));
                    return LRESULT(0);
                } else if !is_window_visible(self.border_window) {
                    let _ = self.update_position(Some(SWP_SHOWWINDOW));
                }

                let old_rect = self.window_rect.clone();
                let _ = self.update_window_rect();
                let _ = self.update_position(None);
              
                // Only re-render the border when its size changes
                if get_rect_width(self.window_rect) != get_rect_width(old_rect)
                || get_rect_height(self.window_rect) != get_rect_height(old_rect) {
                    let _ = self.render();
                }
            },
            // EVENT_OBJECT_REORDER
            5001 => {
                if self.tracking_is_minimized || is_cloaked(self.tracking_window) || !is_window_visible(self.tracking_window) {
                    return LRESULT(0);
                }

                let _ = self.update_color();
                let _ = self.update_position(None);
                let _ = self.render();
            },
            // EVENT_OBJECT_SHOW / EVENT_OBJECT_UNCLOAKED
            5002 => {
                if self.tracking_is_minimized {
                    return LRESULT(0);
                }

                if has_native_border(self.tracking_window) {
                    let _ = self.update_window_rect();
                    let _ = self.update_position(Some(SWP_SHOWWINDOW));
                    let _ = self.render();
                }
            },
            // EVENT_OBJECT_HIDE / EVENT_OBJECT_CLOAKED
            5003 => {
                let _ = self.update_position(Some(SWP_HIDEWINDOW));
            }
            // EVENT_OBJECT_MINIMIZESTART
            5004 => {
                let _ = self.update_position(Some(SWP_HIDEWINDOW));
                self.tracking_is_minimized = true;
            },
            // EVENT_SYSTEM_MINIMIZEEND
            // When a window is about to be unminimized, hide the border and let the thread sleep
            // for 200ms to wait for the window animation to finish, then show the border.
            5005 => {
                std::thread::sleep(std::time::Duration::from_millis(200));

                if has_native_border(self.tracking_window) {
                    let _ = self.update_window_rect();
                    let _ = self.update_position(Some(SWP_SHOWWINDOW));
                    let _ = self.render();
                }
                self.tracking_is_minimized = false;
            },
            WM_DESTROY => {
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                PostQuitMessage(0);
            },
            // Ignore these window position messages
            WM_WINDOWPOSCHANGING => {},
            WM_WINDOWPOSCHANGED => {},
            _ => {
                return DefWindowProcW(window, message, wparam, lparam);
            }
        }
        LRESULT(0)
    }
}



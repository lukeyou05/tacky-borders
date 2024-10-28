use std::ffi::c_ulong;
use std::sync::LazyLock;
use std::sync::OnceLock;
use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::Graphics::Gdi::*,
    Win32::Graphics::Dwm::*,
    Win32::Graphics::Direct2D::*,
    Win32::Graphics::Direct2D::Common::*,
    Win32::Graphics::Dxgi::Common::*,
    Win32::UI::WindowsAndMessaging::*,
};

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

    pub fn init(&mut self, hinstance: HINSTANCE) -> Result<()> {
        unsafe {
            // Make the window border transparent 
            let pos: i32 = -GetSystemMetrics(SM_CXVIRTUALSCREEN) - 8;
            let hrgn = CreateRectRgn(pos, 0, (pos + 1), 1);
            let mut bh: DWM_BLURBEHIND = Default::default();
            if !hrgn.is_invalid() {
                bh = DWM_BLURBEHIND {
                    dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
                    fEnable: TRUE,
                    hRgnBlur: hrgn,
                    fTransitionOnMaximized: FALSE
                };
            }

            DwmEnableBlurBehindWindow(self.border_window, &bh);
            if SetLayeredWindowAttributes(self.border_window, COLORREF(0x00000000), 0, LWA_COLORKEY).is_err() {
                println!("Error Setting Layered Window Attributes!");
            }
            if SetLayeredWindowAttributes(self.border_window, COLORREF(0x00000000), 255, LWA_ALPHA).is_err() {
                println!("Error Setting Layered Window Attributes!");
            }
            
            self.create_render_targets();
            self.render();
            
            // TODO Here im running all the render commands again because it sometimes doesn't
            // render properly at first and I'm too lazy to figure out why. Definitely should be
            // looked into in the future.
            std::thread::sleep(std::time::Duration::from_millis(5));
            self.update_color();
            self.update_window_rect();
            self.update_position();
            self.render();

            let mut message = MSG::default();
            while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
                //println!("message received in window border thread");
                TranslateMessage(&message);
                DispatchMessageW(&message);
                std::thread::sleep(std::time::Duration::from_millis(10));
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
            ..Default::default() };
        self.hwnd_render_target_properties = D2D1_HWND_RENDER_TARGET_PROPERTIES { 
            hwnd: self.border_window, 
            pixelSize: Default::default(), 
            presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY 
        };
        self.border_brush = D2D1_BRUSH_PROPERTIES { 
            opacity: 1.0 as f32, 
            transform: Default::default() 
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
            self.render_target.set(
                factory.CreateHwndRenderTarget(&self.render_target_properties, &self.hwnd_render_target_properties).expect("creating self.render_target failed")
            );
        }

        self.update_color();
        self.update_window_rect();
        self.update_position();

        return Ok(());
    }

    pub fn update_window_rect(&mut self) -> Result<()> {
        let result = unsafe { DwmGetWindowAttribute(
            self.tracking_window, 
            DWMWA_EXTENDED_FRAME_BOUNDS,
            std::ptr::addr_of_mut!(self.window_rect) as *mut _,
            size_of::<RECT>() as u32
        ) }; 
        if result.is_err() {
            println!("Error getting frame rect!");
            // I have not tested if this actually works yet
            unsafe { SendMessageW(self.border_window, WM_DESTROY, WPARAM(0), LPARAM(0)) };
        }

        self.window_rect.top -= self.border_size;
        self.window_rect.left -= self.border_size;
        self.window_rect.right += self.border_size;
        self.window_rect.bottom += self.border_size;

        return Ok(());
    }

    pub fn update_position(&mut self) -> Result<()> {
        unsafe {
            // Place the window border above the tracking window so that it looks nice with window
            // drop shadows enabled.
            let mut hwnd_above_tracking = GetWindow(self.tracking_window, GW_HWNDPREV);
            let mut u_flags = SWP_NOSENDCHANGING | SWP_NOACTIVATE | SWP_NOREDRAW;

            // If the tracking window does not have a window edge, don't show the window border.
            // The reason I'm not just destroying the window border is because going into
            // fullscreen in browsers also gets rid of the WINDOWEDGE style, but I want to keep the
            // window border for when they exit fullscreen.
            let ex_style = GetWindowLongW(self.tracking_window, GWL_EXSTYLE) as u32;
            if ex_style & WS_EX_WINDOWEDGE.0 == 0 {
                u_flags = u_flags | SWP_HIDEWINDOW;
            } else {
                u_flags = u_flags | SWP_SHOWWINDOW;
            }

            // If hwnd_above_tracking is the window border itself, we have what we want and there's
            // no need to change the z-order. If hwnd_above_tracking returns an error, it's likely
            // that tracking window is already the highest in z-order, so we use HWND_TOP to place
            // the window border above.
            if hwnd_above_tracking == Ok(self.border_window) {
                u_flags = u_flags | SWP_NOZORDER;
            } else if hwnd_above_tracking.is_err() {
                hwnd_above_tracking = Ok(HWND_TOP);
            }

            SetWindowPos(self.border_window,
                hwnd_above_tracking.unwrap(),
                self.window_rect.left,
                self.window_rect.top,
                self.window_rect.right - self.window_rect.left,
                self.window_rect.bottom - self.window_rect.top,
                u_flags 
            );
        }
        return Ok(());
    }

    pub fn update_color(&mut self) {
        if unsafe { GetForegroundWindow() } == self.tracking_window {
            self.current_color = self.active_color;
        } else {
            self.current_color = self.inactive_color; 
        }
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

        unsafe {
            render_target.Resize(&self.hwnd_render_target_properties.pixelSize as *const _);
            render_target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

            let brush = render_target.CreateSolidColorBrush(&self.current_color, Some(&self.border_brush))?;
            
            self.rounded_rect.rect = D2D_RECT_F { 
                left: (self.border_size/2 - self.border_offset) as f32, 
                top: (self.border_size/2 - self.border_offset) as f32, 
                right: (self.window_rect.right - self.window_rect.left - self.border_size/2 + self.border_offset) as f32, 
                bottom: (self.window_rect.bottom - self.window_rect.top - self.border_size/2 + self.border_offset) as f32
            };


            render_target.BeginDraw();
            render_target.Clear(None);
            render_target.DrawRoundedRectangle(
                &self.rounded_rect,
                &brush,
                self.border_size as f32,
                None
            );
            render_target.EndDraw(None, None);
        }

        Ok(())
    }

    // When CreateWindowExW is called, we can optionally pass a value to its LPARAM field which will
    // get sent to the window process on creation. In our code, we've passed a pointer to the
    // WindowBorder structure during the window creation process, and here we are getting that pointer 
    // and attaching it to the window using SetWindowLongPtrW.
    pub unsafe extern "system" fn s_wnd_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let mut border_pointer: *mut WindowBorder = GetWindowLongPtrW(window, GWLP_USERDATA) as _;
        
        if border_pointer == std::ptr::null_mut() && message == WM_CREATE {
            //println!("ref is null, assigning new ref");
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
            WM_SHOWWINDOW => {
                SetWindowPos(self.border_window,
                    self.tracking_window,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOSENDCHANGING | SWP_NOACTIVATE | SWP_NOREDRAW | SWP_NOZORDER | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW 
                );
            }
            WM_CLOSE => {
                SetWindowPos(self.border_window,
                    self.tracking_window,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOSENDCHANGING | SWP_NOACTIVATE | SWP_NOREDRAW | SWP_NOZORDER | SWP_NOMOVE | SWP_NOSIZE | SWP_HIDEWINDOW 
                );
            }
            WM_MOVE => {
                // TODO WM_MOVE and WM_SETFOCUS may be called after WM_CLOSE, causing the window to
                // be visible again which is not what we want. That's why I check here to make sure
                // whether the window is cloaked/visible or not. It doesn't take up much processing
                // time so it's totally fine to leave as is, but it might still be worth trying to
                // make better.
                let mut is_cloaked = FALSE;
                let result = unsafe { DwmGetWindowAttribute(
                    self.tracking_window, 
                    DWMWA_CLOAKED,
                    std::ptr::addr_of_mut!(is_cloaked) as *mut _,
                    size_of::<BOOL>() as u32
                ) };
                if result.is_err() || is_cloaked.as_bool() || !IsWindowVisible(self.tracking_window).as_bool() {
                    return LRESULT(0);
                }

                self.update_window_rect();
                self.update_position();
                self.render();
            },
            WM_SETFOCUS => {
                let mut is_cloaked = FALSE;
                let result = unsafe { DwmGetWindowAttribute(
                    self.tracking_window, 
                    DWMWA_CLOAKED,
                    std::ptr::addr_of_mut!(is_cloaked) as *mut _,
                    size_of::<BOOL>() as u32
                ) };
                if result.is_err() || is_cloaked.as_bool() || !IsWindowVisible(self.tracking_window).as_bool() {
                    return LRESULT(0);
                }

                self.update_color();
                if self.tracking_window == GetForegroundWindow() {
                    self.update_position();
                }
                self.render();
            },
            WM_DESTROY => {
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                PostQuitMessage(0);
            },
            _ => { /*println!("message: {:?}", message)*/ }
        }
        LRESULT(0)
    }
}



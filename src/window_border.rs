use std::ffi::c_ulong;
use std::sync::LazyLock;
use std::sync::OnceLock;
use core::ffi::c_void;
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
    pub dpi: f32,
    pub render_target_properties: D2D1_RENDER_TARGET_PROPERTIES,
    pub hwnd_render_target_properties: D2D1_HWND_RENDER_TARGET_PROPERTIES,
    pub render_target: OnceLock<ID2D1HwndRenderTarget>,
    pub border_brush: D2D1_BRUSH_PROPERTIES,
    pub rounded_rect: D2D1_ROUNDED_RECT,
    pub color: D2D1_COLOR_F,
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

            self.update_window_rect()?;

            let dpi_aware = SetProcessDPIAware();
            if !dpi_aware.as_bool() {
                println!("Failed to make process DPI aware");
            }
        }

        Ok(())
    }

    pub fn init(&mut self, hinstance: HINSTANCE) -> Result<()> {
        unsafe {
            //println!("render target: {:?}", RENDER_TARGET.get());
            // make window transparent
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

            /*let val: BOOL = TRUE;

            let result = DwmSetWindowAttribute(self.border_window, DWMWA_EXCLUDED_FROM_PEEK, std::ptr::addr_of!(val) as *const c_void, size_of::<BOOL>() as u32);
            if result.is_err() {
                println!("could not exclude border from peek");
            }*/

            // Make the native windows border transparent... for some reason it makes the borders
            // uneven sizes... so thats why u might notice that transparent is not actually set to
            // transparent for the time being...
            let transparent = COLORREF(0xFFFFFFFF);
            let result = DwmSetWindowAttribute(self.tracking_window, DWMWA_BORDER_COLOR, std::ptr::addr_of!(transparent) as *const c_void, size_of::<c_ulong>() as u32);
            if result.is_err() {
                println!("could not set native border color");
            }
            
            if IsWindowVisible(self.tracking_window).as_bool() {
                ShowWindow(self.border_window, SW_SHOWNA);
            }

            self.create_render_targets();
            self.render();

            let mut message = MSG::default();
            while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
                //let before = std::time::Instant::now();
                //println!("message received");
                TranslateMessage(&message);
                DispatchMessageW(&message);
                //std::thread::sleep(std::time::Duration::from_millis(10));
                //println!("Elapsed time (message loop): {:.2?}", before.elapsed());
            }
        }

        return Ok(());
    }

    pub fn create_render_targets(&mut self) {
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

        self.rounded_rect = D2D1_ROUNDED_RECT { 
            rect: Default::default(), 
            radiusX: 6.0 + ((self.border_size/2) as f32), 
            radiusY: 6.0 + ((self.border_size/2) as f32)
        };

        self.color = D2D1_COLOR_F { 
            r: 0.0, 
            g: 0.0, 
            b: 0.0, 
            a: 1.0 
        };

        unsafe {
            let factory = &*RENDER_FACTORY;
            self.render_target.set(
                factory.CreateHwndRenderTarget(&self.render_target_properties, &self.hwnd_render_target_properties).expect("creating self.render_target failed")
            );
        }

        self.update_color();
        self.update_border_location();
    }

    pub fn render(&mut self) -> Result<()> {
        //let before = std::time::Instant::now();
        let render_target_option = self.render_target.get();
        if render_target_option.is_none() {
            return Ok(()); 
        }
        let render_target = render_target_option.unwrap();
        //println!("Elapsed time to get render_target: {:?}", before.elapsed());

        self.hwnd_render_target_properties.pixelSize = D2D_SIZE_U { 
            width: (self.window_rect.right - self.window_rect.left) as u32,
            height: (self.window_rect.bottom - self.window_rect.top) as u32
        };

        unsafe {
            //let before = std::time::Instant::now();
            render_target.Resize(&self.hwnd_render_target_properties.pixelSize as *const _);
            // I'm not even sure what SetAntiAliasMode does because without the line, the corners are still anti-aliased.
            // Maybe there's an ever so slight bit more anti-aliasing with it but I could just be crazy. 
            //render_target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

            let brush = render_target.CreateSolidColorBrush(&self.color, Some(&self.border_brush))?;
            //println!("Time it takes to create render target and brush: {:.2?}", before.elapsed());
            
            // Goofy size calculations
            self.rounded_rect.rect = D2D_RECT_F { 
                left: (self.border_size/2 + self.border_offset) as f32, 
                top: (self.border_size/2 + self.border_offset) as f32, 
                right: (self.window_rect.right - self.window_rect.left - self.border_size/2 - self.border_offset) as f32, 
                bottom: (self.window_rect.bottom - self.window_rect.top - self.border_size/2 - self.border_offset) as f32
            };


            //let before = std::time::Instant::now();
            render_target.BeginDraw();
            render_target.Clear(None);
            render_target.DrawRoundedRectangle(
                &self.rounded_rect,
                &brush,
                self.border_size as f32,
                None
            );
            render_target.EndDraw(None, None);
            //println!("Time it takes to render: {:.2?}", before.elapsed());
        }

        Ok(())
    }

    pub fn update_border_location(&mut self) {
        //let before = std::time::Instant::now();
        self.update_window_rect();
        self.update_pos();
        self.render();
        //println!("Elapsed time (update): {:.2?}", before.elapsed());
    }

    pub fn update_window_rect(&mut self) -> Result<()> {
        if unsafe { DwmGetWindowAttribute(self.tracking_window, DWMWA_EXTENDED_FRAME_BOUNDS, &mut self.window_rect as *mut _ as *mut c_void, size_of::<RECT>() as u32).is_err() } {
            println!("Error getting frame rect!");
            //unsafe { ExitThread(0) };
        }

        self.window_rect.top -= self.border_size;
        self.window_rect.left -= self.border_size;
        self.window_rect.right += self.border_size;
        self.window_rect.bottom += self.border_size;

        return Ok(());
    }

    pub fn update_pos(&mut self) {
        unsafe {
            SetWindowPos(self.border_window,
                self.tracking_window,
                self.window_rect.left,
                self.window_rect.top,
                self.window_rect.right - self.window_rect.left,
                self.window_rect.bottom - self.window_rect.top,
            SWP_NOREDRAW | SWP_NOACTIVATE
            );
        }
    }

    pub fn update_color(&mut self) {
        let mut pcr_colorization: u32 = 0;
        let mut pf_opaqueblend: BOOL = BOOL(0);
        //TODO should check whether DwmGetColorzationColor was successful or not. 
        unsafe { DwmGetColorizationColor(&mut pcr_colorization, &mut pf_opaqueblend) };

        let r = ((pcr_colorization & 0x00FF0000) >> 16) as f32;
        let g = ((pcr_colorization & 0x0000FF00) >> 8) as f32;
        let b = ((pcr_colorization & 0x000000FF) >> 0) as f32;

        if unsafe { GetForegroundWindow() } == self.tracking_window {
            self.color.r = r/255.0;
            self.color.g = g/255.0;
            self.color.b = b/255.0;
        } else {
            self.color.r = r/255.0/1.5;
            self.color.g = g/255.0/1.5;
            self.color.b = b/255.0/1.5;
        }
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

    // TODO event_hook will send more messages than necessary if I do an action for long enough. I
    // should find a way to fix that.
    pub unsafe fn wnd_proc(&mut self, window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match message {
            // TODO maybe switch out WM_MOVE with WM_WINDOWPOSCHANGING because that seems like the
            // more correct way to do it. But if I do it that way, I have to pass a WINDOWPOS
            // structure which I'm too lazy to deal with right now.
            WM_MOVE => {
                //let before = std::time::Instant::now();
                self.update_border_location();
                //std::thread::sleep(std::time::Duration::from_millis(7));
                //println!("time elapsed: {:.2?}", before.elapsed());
            },
            WM_SETFOCUS => {
                //println!("Focus set: {:?}", self.tracking_window);
                self.update_color();
                self.render();
            },
            WM_KILLFOCUS => {
                //println!("Focus killed: {:?}", self.tracking_window);
                self.update_color();
                self.render();
            },
            WM_DESTROY => {
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                PostQuitMessage(0);
            },
            _ => {}
        }
        LRESULT(0)
    }
}



use std::ffi::c_ulong;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::prelude::OsStringExt;
use core::ffi::c_void;
use core::ffi::c_int;
use windows::{
    core::*,
    Win32::Foundation::*,
    Foundation::Numerics::*,
    Win32::Graphics::Gdi::*,
    Win32::Graphics::Dwm::*,
    Win32::Graphics::Direct2D::*,
    Win32::Graphics::Direct2D::Common::*,
    Win32::Graphics::Dxgi::Common::*,
    Win32::System::LibraryLoader::GetModuleHandleA,
    Win32::System::SystemServices::IMAGE_DOS_HEADER,
    Win32::UI::WindowsAndMessaging::*,
    Win32::UI::Accessibility::*,
};
use std::cell::Cell;
use std::sync::LazyLock;

// Can I use mod drawer here somehow?
/*use crate::drawer::*;*/
use crate::event_hook;

pub static FACTORY: LazyLock<ID2D1Factory> = unsafe { LazyLock::new(|| D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_MULTI_THREADED, None).expect("REASON")) };
//pub static m_render_target = FACTORY.CreateHwndRenderTarget(Default::default, Default::default).expect("REASON");

#[derive(Debug, Default, Copy, Clone)]
pub struct WindowBorder {
    pub m_window: HWND,
    pub m_tracking_window: HWND,
    pub window_rect: RECT,
    pub border_size: i32,
    pub border_offset: i32,
    pub win_event_hook: HWINEVENTHOOK,
    pub dpi: f32,
    pub render_target_properties: D2D1_RENDER_TARGET_PROPERTIES,
    pub hwnd_render_target_properties: D2D1_HWND_RENDER_TARGET_PROPERTIES,
    pub m_border_brush: D2D1_BRUSH_PROPERTIES,
    pub rounded_rect: D2D1_ROUNDED_RECT,
    pub color: D2D1_COLOR_F,
    //pub hwnd_render_target: ID2D1HwndRenderTarget,
    //pub factory: &'static ID2D1Factory,
}

impl WindowBorder {
    pub fn create(window: HWND) -> WindowBorder {
        let mut border = WindowBorder { 
            m_window: HWND::default(), 
            m_tracking_window: window, 
            window_rect: RECT::default(), 
            border_size: 4, 
            border_offset: 1,
            ..Default::default()
        };

        //TODO maybe check if dpi_aware is true or not
        let dpi_aware = unsafe { SetProcessDPIAware() };

        return border;
    }

    pub fn init(&mut self, hinstance: HINSTANCE) -> Result<()> {
        if self.m_tracking_window.is_invalid() {
            println!("Error at m_tracking_window!");
        }

        self.get_frame_rect()?;

        unsafe {
            self.m_window = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                w!("tacky-border"),
                w!("tacky-border"),
                WS_POPUP | WS_DISABLED,
                self.window_rect.left,
                self.window_rect.top,
                self.window_rect.right - self.window_rect.left,
                self.window_rect.bottom - self.window_rect.top,
                None,
                None,
                hinstance,
                Some(std::ptr::addr_of!(*self) as *const _)
            )?;

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

            DwmEnableBlurBehindWindow(self.m_window, &bh);

            if SetLayeredWindowAttributes(self.m_window, COLORREF(0x00000000), 0, LWA_COLORKEY).is_err() {
                println!("Error Setting Layered Window Attributes!");
            }
            if SetLayeredWindowAttributes(self.m_window, COLORREF(0x00000000), 255, LWA_ALPHA).is_err() {
                println!("Error Setting Layered Window Attributes!");
            }

            /*// set position of the border-window behind the tracking window
            // helps to prevent border overlapping (happens after turning borders off and on)
            let set_pos = SetWindowPos(self.m_tracking_window,
                self.m_window,
                self.window_rect.left,
                self.window_rect.top,
                self.window_rect.right - self.window_rect.left,
                self.window_rect.bottom - self.window_rect.top,
                SWP_NOMOVE | SWP_NOSIZE);

            if set_pos.is_err() {
                println!("Error with SetWindowPos!");
            }*/
            
            /*let val: BOOL = TRUE;

            let result = DwmSetWindowAttribute(self.m_window, DWMWA_EXCLUDED_FROM_PEEK, std::ptr::addr_of!(val) as *const c_void, size_of::<BOOL>() as u32);
            if result.is_err() {
                println!("could not exclude border from peek");
            }*/

            // Make the native windows border transparent... for some reason it makes the borders
            // uneven sizes...
            let transparent = COLORREF(0xFFFFFFFF);
            let result = DwmSetWindowAttribute(self.m_tracking_window, DWMWA_BORDER_COLOR, std::ptr::addr_of!(transparent) as *const c_void, size_of::<c_ulong>() as u32);
            if result.is_err() {
                println!("could not set native border color");
            }
            
            if IsWindowVisible(self.m_tracking_window).as_bool() {
                ShowWindow(self.m_window, SW_SHOWNA);
            }
            UpdateWindow(self.m_window);

            self.create_render_targets();
            self.render();

            //TODO idk how to feel about sleeping the thread bc if the event hook sends too many
            //messages then this will continue processing all of them even after the event is over. 
            let mut message = MSG::default();

            //let mut before = std::time::Instant::now();
            while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
                //let before = std::time::Instant::now();
                if message.hwnd != self.m_window {
                    //println!("Dispatching message!");
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
                //println!("Elapsed time (message loop): {:.2?}", before.elapsed());
                //before = std::time::Instant::now();
            }
        }

        return Ok(());
    }

    pub fn get_frame_rect(&mut self) -> Result<()> {
        //unsafe { println!("m_tracking_window: {:?}", self.m_tracking_window) };
        if unsafe { DwmGetWindowAttribute(self.m_tracking_window, DWMWA_EXTENDED_FRAME_BOUNDS, &mut self.window_rect as *mut _ as *mut c_void, size_of::<RECT>() as u32).is_err() } {
            println!("Error getting frame rect!");
        }

        /*if unsafe { GetWindowRect(self.m_tracking_window, &mut self.window_rect).is_err() } {
            println!("Error getting frame rect!");
        }*/

        self.window_rect.top -= self.border_size;
        self.window_rect.left -= self.border_size;
        self.window_rect.right += self.border_size;
        self.window_rect.bottom += self.border_size;

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
            hwnd: self.m_window, 
            pixelSize: Default::default(), 
            presentOptions: D2D1_PRESENT_OPTIONS_NONE 
        };

        self.m_border_brush = D2D1_BRUSH_PROPERTIES { 
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
        self.update_color();
    }

    pub fn render(&mut self) -> Result<()> {
        let factory: &ID2D1Factory = &*FACTORY;

        self.hwnd_render_target_properties.pixelSize = D2D_SIZE_U { 
            width: (self.window_rect.right - self.window_rect.left) as u32,
            height: (self.window_rect.bottom - self.window_rect.top) as u32
        };

        unsafe {
            //let before = std::time::Instant::now();
            let m_render_target = factory.CreateHwndRenderTarget(&self.render_target_properties, &self.hwnd_render_target_properties)?;
            // I'm not even sure what SetAntiAliasMode does because without the line, the corners are still anti-aliased.
            // Maybe there's an ever so slight bit more anti-aliasing with it but I could just be crazy. 
            //m_render_target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

            let m_brush = m_render_target.CreateSolidColorBrush(&self.color, Some(&self.m_border_brush))?;
            //println!("Time it takes to create render target and brush: {:.2?}", before.elapsed());
            
            // Goofy size calculations
            self.rounded_rect.rect = D2D_RECT_F { 
                left: (self.border_size/2 + self.border_offset) as f32, 
                top: (self.border_size/2 + self.border_offset) as f32, 
                right: (self.window_rect.right - self.window_rect.left - self.border_size/2 - self.border_offset) as f32, 
                bottom: (self.window_rect.bottom - self.window_rect.top - self.border_size/2 - self.border_offset) as f32
            };


            //let before = std::time::Instant::now();
            m_render_target.BeginDraw();
            m_render_target.Clear(None);
            m_render_target.DrawRoundedRectangle(
                &self.rounded_rect,
                &m_brush,
                self.border_size as f32,
                None
            );
            m_render_target.EndDraw(None, None);
            //println!("Time it takes to render: {:.2?}", before.elapsed());
        }

        Ok(())
    }

    pub fn update(&mut self) {
        //let before = std::time::Instant::now();
        let old_rect = self.window_rect.clone();
        self.get_frame_rect();
        unsafe {
            self.update_pos();
            self.render();
        }
        //println!("Elapsed time (update): {:.2?}", before.elapsed());
    }

    pub fn update_pos(&mut self) {
        unsafe {
            SetWindowPos(self.m_window,
                self.m_tracking_window,
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

        if unsafe { GetForegroundWindow() } == self.m_tracking_window {
            self.color.r = r/255.0;
            self.color.g = g/255.0;
            self.color.b = b/255.0;
        } else {
            self.color.r = r/255.0/1.5;
            self.color.g = g/255.0/1.5;
            self.color.b = b/255.0/1.5;
        }

        self.update();
    }

    // When CreateWindowExW is called, we can optinally pass a value to its last field which will
    // get sent to the window process on creation. In our code, we've passed a pointer to the border 
    // structure, and here we are getting that pointer and assigning it to the window using SetWindowLongPtrW.
    pub unsafe extern "system" fn s_wnd_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let mut this_ref: *mut WindowBorder = GetWindowLongPtrW(window, GWLP_USERDATA) as _;
        
        if this_ref == std::ptr::null_mut() && message == WM_CREATE {
            let create_struct: *mut CREATESTRUCTW = lparam.0 as *mut _;
            this_ref = (*create_struct).lpCreateParams as *mut _;
            SetWindowLongPtrW(window, GWLP_USERDATA, this_ref as _);
        }
        match this_ref != std::ptr::null_mut() {
            true => return Self::wnd_proc(&mut *this_ref, window, message, wparam, lparam),
            false => return DefWindowProcW(window, message, wparam, lparam),
        }                                          
    }

    // TODO event_hook will send more messages than necessary if I do an action for long enough. I
    // should find a way to fix that.
    pub unsafe fn wnd_proc(&mut self, window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match message {
            WM_MOVE => {
                //let before = std::time::Instant::now();

                // Attempt to jump into another message loop with no sleep so as to maximize draw fps (doesn't work though).
                /*let mut message = MSG::default();
                while GetMessageW(&mut message, HWND::default(), 0, 0).into() && message.message == WM_MOVE {
                    println!("Moving");
                    self.update();
                    //GetMessageW(&mut message, HWND::default(), 0, 0);
                }*/
                self.update();
                //println!("time elapsed: {:.2?}", before.elapsed());
            },
            //TODO maybe switch out WM_MOVE with WM_WINDOWPOSCHANGING because that seems like the
            //more correct way to do it. But if I do it that way, I have to pass a WINDOWPOS
            //structure which I'm too lazy to deal with right now.
            //WM_WINDOWPOSCHANGING => { self.update() },
            //WM_WINDOWPOSCHANGED => { self.update() },
            WM_SETFOCUS => {
                //println!("Focus set: {:?}", self.m_tracking_window);
                self.update_pos();
                self.update_color();
            },
            WM_KILLFOCUS => {
                //println!("Focus killed: {:?}", self.m_tracking_window);
                self.update_pos();
                self.update_color();
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



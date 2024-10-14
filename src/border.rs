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

// Can I use mod drawer here somehow?
/*use crate::drawer::*;*/

const SW_SHOWNA: i32 = 8;


#[derive(Debug)]
#[derive(Default)]
pub struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[derive(Debug)]
pub struct WindowBorder {
    pub m_window: HWND,
    pub m_tracking_window: HWND,
    window_rect: RECT,
    border_size: i32,
    border_offset: i32
}

impl WindowBorder {
    pub fn create(window: HWND, hinstance: HINSTANCE) -> WindowBorder {
        // let mut border: Box<WindowBorder> = Box::new(WindowBorder { m_window: HWND::default(), m_tracking_window: window } );
        let mut border = WindowBorder { 
            m_window: HWND::default(), 
            m_tracking_window: window, 
            window_rect: RECT::default(), 
            border_size: 4, 
            border_offset: 1
        };
        //println!("hinstance: {:?}", hinstance);
        //println!("border.m_window: {:?}", border.m_window);
        //println!("border.m_tracking_window: {:?}", border.m_tracking_window);

        // The lines below are currently useless because if a WindowBorder is successfully
        // initialized, it will be in a message loop and will never reach this part of the code.
        match WindowBorder::init(&mut border, hinstance) {
            Ok(val) => return border,
            Err(err) => println!("Error! {}", err),
        }

        return border;
    }

    pub fn init(&mut self, hinstance: HINSTANCE) -> Result<()> {
        /*let window_rect_opt: Option<RECT> = match self.m_tracking_window {
            Some(x) => get_frame_rect(x),
            None => return false,
        };*/

        if self.m_tracking_window.is_invalid() {
            /*return Err();*/
            println!("Error at m_tracking_window!");
        }

        self.get_frame_rect()?;

        /*let window_rect: RECT;
        match window_rect_opt {
            Some(val) => window_rect = val,
            /*None => return Err(),*/
            None => return Ok(()),
        };*/

        // println!("window_rect: {:?}", window_rect);

        unsafe {
            self.m_window = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                /*WS_EX_TOPMOST | WS_EX_TOOLWINDOW,*/
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
                Some(std::mem::transmute(&mut *self))
            )?;

            // println!("self: {:?}", self);

            // make window transparent
            let pos: i32 = -GetSystemMetrics(SM_CXVIRTUALSCREEN) - 8;
            //println!("pos: {:?}", pos);
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

            // set position of the border-window behind the tracking window
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
            }
            
            let val: BOOL = TRUE;

            // I doubt the code below is functioning properly (the std::mem::transmute(&val))
            DwmSetWindowAttribute(self.m_window, DWMWA_EXCLUDED_FROM_PEEK, std::mem::transmute(&val), size_of::<BOOL>() as u32);
            //println!("pointer to BOOL: {:?} {:?}", &val, std::mem::transmute::<&BOOL, isize>(&val));

            ShowWindow(self.m_window, SHOW_WINDOW_CMD(SW_SHOWNA));

            UpdateWindow(self.m_window);

            // println!("self.m_window (from init): {:?}", self.m_window);

            self.render();
            loop {
                self.get_frame_rect();
                self.render();
                SetWindowPos(self.m_window,
                    self.m_tracking_window,
                    self.window_rect.left,
                    self.window_rect.top,
                    self.window_rect.right - self.window_rect.left,
                    self.window_rect.bottom - self.window_rect.top,
                SWP_NOREDRAW | SWP_NOACTIVATE
                );
                std::thread::sleep(std::time::Duration::from_millis(16))
            }

            //Self::set_windows_hook(&self);

            let mut message = MSG::default();
            while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
                TranslateMessage(&message);
                DispatchMessageW(&message);
                std::thread::sleep(std::time::Duration::from_millis(1000))
            }
            println!("Potential error with message loop, exiting!");
        }

        return Ok(());
    }

    pub fn get_frame_rect(&mut self) -> Result<()> {
        if unsafe { DwmGetWindowAttribute(self.m_tracking_window, DWMWA_EXTENDED_FRAME_BOUNDS, &mut self.window_rect as *mut _ as *mut c_void, size_of::<RECT>() as u32).is_err() } {
            println!("Error getting frame rect!");
        }

        self.window_rect.top -= self.border_size;
        self.window_rect.left -= self.border_size;
        self.window_rect.right += self.border_size;
        self.window_rect.bottom += self.border_size;

        return Ok(());
    }

    pub fn render(&self) -> Result<()> {
        let hr: HRESULT;

        //println!("self.m_window (from render): {:?}", self.m_window);

        let dpi: f32 = 96.0;
        let render_target_properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT { 
                format: DXGI_FORMAT_UNKNOWN, 
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED 
            },
            dpiX: dpi,
            dpiY: dpi,
            ..Default::default() };

        /*let render_target_size = D2D_SIZE_U { width: (client_rect.right - client_rect.left) as u32, height: (client_rect.bottom - client_rect.top) as u32 };*/
        let render_target_size = D2D_SIZE_U { 
            width: (self.window_rect.right - self.window_rect.left) as u32,
            height: (self.window_rect.bottom - self.window_rect.top) as u32
        };
        println!("render_target_size: {:?}", render_target_size);

        let hwnd_render_target_properties = D2D1_HWND_RENDER_TARGET_PROPERTIES { 
            hwnd: self.m_window, 
            pixelSize: render_target_size, 
            presentOptions: D2D1_PRESENT_OPTIONS_NONE 
        };
        //println!("hwnd_render_target_properties: {:?}", hwnd_render_target_properties);

        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&D2D1_FACTORY_OPTIONS::default()))?;
            let m_render_target = factory.CreateHwndRenderTarget(&render_target_properties, &hwnd_render_target_properties)?;

            m_render_target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            let color = D2D1_COLOR_F { 
                r: 107.0/255.0, 
                g: 145.0/255.0, 
                b: 241.0/255.0, 
                a: 1.0 
            };

            let m_border_brush = D2D1_BRUSH_PROPERTIES { 
                opacity: 1.0 as f32, 
                transform: std::mem::zeroed() 
            };
            let m_brush = m_render_target.CreateSolidColorBrush(&color, Some(&m_border_brush))?;
            //println!("m_brush: {:?}", color);

            // Yes, the size calculations below are confusing, but they work, and that's all that
            // really matters.
            let rect = D2D_RECT_F { 
                left: (self.border_size/2 + self.border_offset) as f32, 
                top: (self.border_size/2 + self.border_offset) as f32, 
                right: (self.window_rect.right - self.window_rect.left - self.border_size/2 - self.border_offset) as f32, 
                bottom: (self.window_rect.bottom - self.window_rect.top - self.border_size/2 - self.border_offset) as f32
            };
            let rounded_rect = D2D1_ROUNDED_RECT { 
                rect: rect, 
                radiusX: 6.0 + ((self.border_size/2) as f32), 
                radiusY: 6.0 + ((self.border_size/2) as f32)
            };

            //println!("m_render_target: {:?}", m_render_target);

            m_render_target.BeginDraw();
            m_render_target.DrawRoundedRectangle(
                &rounded_rect,
                &m_brush,
                self.border_size as f32,
                None
            );
            m_render_target.EndDraw(None, None);
        }

        Ok(())
    }

    pub fn set_windows_hook(&self) {
        unsafe {
            let thread = GetWindowThreadProcessId(self.m_tracking_window, None);
            let error = GetLastError();
            println!("Is there an error from getting window thread? Last Error: {:?} Thread: {:?} (note that the error may be from elsewhere in the program)", error, thread);
            SetWindowsHookExW(
                WH_CALLWNDPROC,
                Some(Self::win_hook),
                HINSTANCE::default(),
                thread,
            );
        }
    }

    pub unsafe extern "system" fn win_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
    }

    // When CreateWindowExW is called, we can optinally pass a value to its last field which will
    // get sent to the window process on creation. In our code, we've passed a pointer to the border 
    // structure, and here we are getting that pointer and assigning it to the window using SetWindowLongPtrW.
    pub unsafe extern "system" fn s_wnd_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        println!("Window Message: {:?}", message);
        let mut this_ref: *mut WindowBorder = GetWindowLongPtrW(window, GWLP_USERDATA) as _;
        
        if this_ref == std::ptr::null_mut() && message == WM_CREATE {
            let create_struct: *mut CREATESTRUCTW = lparam.0 as *mut _;
            this_ref = (*create_struct).lpCreateParams as *mut _;
            // println!("this_ref: {:?}", *this_ref);
            SetWindowLongPtrW(window, GWLP_USERDATA, this_ref as _);
        }
        match this_ref != std::ptr::null_mut() {
            true => return Self::wnd_proc(&mut *this_ref, window, message, wparam, lparam),
            false => return DefWindowProcW(window, message, wparam, lparam),
        }                                          
    }

    // TODO these messages are the border window itself, which do nothing. I need to somehow get
    // messages from the tracking window. Alternatively I can just put everything on a timer lol.
    pub unsafe fn wnd_proc(&mut self, window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match message {
            WM_MOVING => {
                // println!("Re-rendering!"); 
                Self::render(self);
            },
            WM_WINDOWPOSCHANGED => {
                // println!("Re-ordering window z order!");
                SetWindowPos(self.m_tracking_window,
                    self.m_window,
                    self.window_rect.left,
                    self.window_rect.top,
                    self.window_rect.right - self.window_rect.left,
                    self.window_rect.bottom - self.window_rect.top,
                    SWP_NOMOVE | SWP_NOSIZE
                );
            },
            WM_DESTROY => {
                let ptr = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut i32;
                // Converting to a box like below means it will automatically clean up when it goes
                // out of scope (I think).
                Box::from_raw(ptr);
                println!("Cleaned up the box.");
                PostQuitMessage(0);
            },
            _ => {}
        }
        LRESULT(0)
    }
}



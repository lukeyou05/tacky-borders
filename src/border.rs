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

pub struct WindowBorder {
    m_window: HWND,
    m_tracking_window: HWND,
    window_rect: RECT,
    border_size: i32,
}

impl WindowBorder {
    pub fn create(window: HWND, hinstance: HINSTANCE) -> WindowBorder {
        // let mut border: Box<WindowBorder> = Box::new(WindowBorder { m_window: HWND::default(), m_tracking_window: window } );
        let mut border = WindowBorder { m_window: HWND::default(), m_tracking_window: window, window_rect: RECT::default(), border_size: 4 };
        println!("hinstance: {:?}", hinstance);
        println!("border.m_window: {:?}", border.m_window);
        println!("border.m_tracking_window: {:?}", border.m_tracking_window);

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

        self.get_frame_rect(self.m_tracking_window)?;

        /*let window_rect: RECT;
        match window_rect_opt {
            Some(val) => window_rect = val,
            /*None => return Err(),*/
            None => return Ok(()),
        };*/

        // println!("window_rect: {:?}", window_rect);

        unsafe {
            // I need to change this line below so that it changes self's variables. I looked up
            // online and couldn't find much on how to do that. I may have to use somthing other
            // than Box.
            let open_window = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
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
                None
            )?;

            self.m_window = open_window;
          
            // make window transparent
            let pos: i32 = -GetSystemMetrics(SM_CXVIRTUALSCREEN) - 8;
            println!("pos: {:?}", pos);
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

            DwmEnableBlurBehindWindow(open_window, &bh);

            if SetLayeredWindowAttributes(open_window, COLORREF(0x00000000), 0, LWA_COLORKEY).is_err() {
                println!("Error Setting Layered Window Attributes!");
            }
            if SetLayeredWindowAttributes(open_window, COLORREF(0x00000000), 255, LWA_ALPHA).is_err() {
                println!("Error Setting Layered Window Attributes!");
            }

            // set position of the border-window behind the tracking window
            // helps to prevent border overlapping (happens after turning borders off and on)
            SetWindowPos(self.m_tracking_window,
                open_window,
                self.window_rect.left,
                self.window_rect.top,
                self.window_rect.right - self.window_rect.left,
                self.window_rect.bottom - self.window_rect.top,
                SWP_NOMOVE | SWP_NOSIZE);
            
            let val: BOOL = TRUE;

            // I doubt the code below is functioning properly (the std::mem::transmute(&val))
            DwmSetWindowAttribute(open_window, DWMWA_EXCLUDED_FROM_PEEK, std::mem::transmute(&val), size_of::<BOOL>() as u32);
            println!("pointer to BOOL: {:?} {:?}", &val, std::mem::transmute::<&BOOL, isize>(&val));

            ShowWindow(open_window, SHOW_WINDOW_CMD(SW_SHOWNA));

            UpdateWindow(open_window);

            println!("open_window (from init): {:?}", open_window);

            WindowBorder::render(&self, open_window);

            let mut message = MSG::default();
            while GetMessageW(&mut message, HWND(std::ptr::null_mut()), 0, 0).into() {
                DispatchMessageA(&message);
            }
            
            if self.m_window.is_invalid() {
                println!("m_window is invalid!");
            }

        }

        return Ok(());
    }

    pub fn get_frame_rect(&mut self, window: HWND) -> Result<()> {
        if unsafe { DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS, &mut self.window_rect as *mut _ as *mut c_void, size_of::<RECT>() as u32).is_err() } {
            println!("Error getting frame rect!");
        }

        self.window_rect.top -= self.border_size;
        self.window_rect.left -= self.border_size;
        self.window_rect.right += self.border_size;
        self.window_rect.bottom += self.border_size;

        return Ok(());
    }

    pub fn render(&self, open_window: HWND) -> Result<()> {
        let hr: HRESULT;

        println!("open_window (from render): {:?}", open_window);

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
            width: self.window_rect.right as u32 - self.window_rect.left as u32,
            height: self.window_rect.bottom as u32 - self.window_rect.top as u32
        };
        println!("render_target_size: {:?}", render_target_size);

        let hwnd_render_target_properties = D2D1_HWND_RENDER_TARGET_PROPERTIES { 
            hwnd: open_window, 
            pixelSize: render_target_size, 
            presentOptions: D2D1_PRESENT_OPTIONS_NONE 
        };
        println!("hwnd_render_target_properties: {:?}", hwnd_render_target_properties);

        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&D2D1_FACTORY_OPTIONS::default()))?;
            let m_render_target = factory.CreateHwndRenderTarget(&render_target_properties, &hwnd_render_target_properties)?;

            m_render_target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            let color = D2D1_COLOR_F { 
                r: 90.0/255.0, 
                g: 194.0/255.0, 
                b: 247.0/255.0, 
                a: 1.0 
            };

            let m_border_brush = D2D1_BRUSH_PROPERTIES { 
                opacity: 1.0 as f32, 
                transform: std::mem::zeroed() 
            };
            let m_brush = m_render_target.CreateSolidColorBrush(&color, Some(&m_border_brush))?;
            println!("m_brush: {:?}", color);

            let rect = D2D_RECT_F { 
                left: 3.0, 
                top: 3.0, 
                right: self.window_rect.right as f32 - self.window_rect.left as f32 - self.border_size as f32, 
                bottom: self.window_rect.bottom as f32 - self.window_rect.top as f32 - self.border_size as f32
            };
            let rounded_rect = D2D1_ROUNDED_RECT { 
                rect: rect, 
                radiusX: 8.0, 
                radiusY: 8.0 
            };

            println!("m_render_target: {:?}", m_render_target);

            m_render_target.BeginDraw();
            m_render_target.DrawRoundedRectangle(
                &rounded_rect,
                &m_brush,
                4.0,
                None
            );
            m_render_target.EndDraw(None, None);
            

            // testrect below is only used to get the coordinates of the border. can delete later
            // when finished with the program.
            let mut testrect: RECT = RECT { top: 0, left: 0, right: 0, bottom: 0 };
            DwmGetWindowAttribute(open_window, DWMWA_EXTENDED_FRAME_BOUNDS, &mut testrect as *mut _ as *mut c_void, size_of::<RECT>() as u32);
            println!("{:?}", testrect);
        }

        Ok(())
    }
}


    unsafe extern "system" fn s_wnd_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let mut this_ref: *mut WindowBorder = std::mem::transmute(GetWindowLongPtrW(window, GWLP_USERDATA));
        println!("is this a magic cookie or not?: {:?}", this_ref);
        
        if this_ref == std::ptr::null_mut() && message == WM_CREATE {
            let create_struct: *mut CREATESTRUCTW = std::mem::transmute(lparam.0);
            println!("create_struct: {:?}", create_struct);
            this_ref = std::mem::transmute((*create_struct).lpCreateParams);
            SetWindowLongPtrW(window, GWLP_USERDATA, std::mem::transmute(this_ref));
        }
        match this_ref != std::ptr::null_mut() {
            true => return wnd_proc(message, wparam, lparam),
            false => return DefWindowProcW(window, message, wparam, lparam),
        }                                          
    }

    pub fn wnd_proc(message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        /*match message {
            WM_TIMER => {
                match wparam {
                    REFRESH_BORDER_TIMER_ID => {
                    },
                }
            },
        }*/
        return LRESULT(10);
    }

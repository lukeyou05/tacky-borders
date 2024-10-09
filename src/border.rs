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

#[derive(Debug)]
pub struct RECT {
    top: i32,
    left: i32,
    right: i32,
    bottom: i32,
}

pub fn get_frame_rect(window: HWND) -> Option<RECT> {
    let mut rect: RECT = RECT { top: 0, left: 0, right: 0, bottom: 0 };

    if unsafe { DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS, &mut rect as *mut _ as *mut c_void, size_of::<RECT>() as u32).is_err() } {
        return None;
    }

    let border: i32 = 4;
    rect.top -= border;
    rect.left -= border;
    rect.right += border;
    rect.bottom += border;
    
    return Some(rect);
}

pub struct WindowBorder {
    m_window: HWND,
    m_tracking_window: HWND,
}

impl WindowBorder {
    pub fn create(window: HWND, hinstance: HINSTANCE) -> Box<WindowBorder> {
        let mut border: Box<WindowBorder> = Box::new(WindowBorder { m_window: HWND(std::ptr::null_mut()), m_tracking_window: window } );
        println!("hinstance: {:?}", hinstance);
        println!("border.m_window: {:?}", border.m_window);
        println!("border.m_tracking_window: {:?}", border.m_tracking_window);

        match WindowBorder::init(&border, hinstance) {
            Ok(val) => return border,
            Err(err) => println!("Error! {}", err),
        }

        return border;
    }

    pub fn init(&self, hinstance: HINSTANCE) -> Result<()> {
        /*let window_rect_opt: Option<RECT> = match self.m_tracking_window {
            Some(x) => get_frame_rect(x),
            None => return false,
        };*/

        if self.m_tracking_window.is_invalid() {
            /*return Err();*/
            println!("Error at m_tracking_window!");
        }

        let mut window_rect_opt: Option<RECT> = get_frame_rect(self.m_tracking_window);

        let window_rect: RECT;
        match window_rect_opt {
            Some(val) => window_rect = val,
            /*None => return Err(),*/
            None => return Ok(()),
        };

        println!("window_rect: {:?}", window_rect);

        let window_class = w!("tacky-border");
        println!("creating window_class");
        unsafe {
            let mut wcex = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(DefWindowProcW),
                hInstance: hinstance,
                lpszClassName: window_class,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            let atom = RegisterClassExW(&wcex);
            println!("wcex.hCursor: {:?}", wcex.hCursor);
            
            if atom == 0 {
                let last_error = GetLastError();
                println!("ERROR: RegisterClassExW(&wcex): {:?}", last_error);
            }

            // I need to change this line below so that it changes self's variables. I looked up
            // online and couldn't find much on how to do that. I may have to use somthing other
            // than Box.
            /*let m_window = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                window_class,
                w!("tacky-border"),
                WS_POPUP | WS_DISABLED,
                window_rect.left,
                window_rect.top,
                window_rect.right - window_rect.left,
                window_rect.bottom - window_rect.top,
                None,
                None,
                hinstance,
                None);*/

            let open_window = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                window_class,
                w!("This is a sample window"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                800,
                600,
                None,
                None,
                hinstance,
                None,
            )?;

            println!("open_window (from init): {:?}", open_window);

            let mut message = MSG::default();

            WindowBorder::render(&self, window_rect, open_window);

            while GetMessageW(&mut message, HWND(std::ptr::null_mut()), 0, 0).into() {
                DispatchMessageA(&message);
            }
            
            if self.m_window.is_invalid() {
                println!("m_window is invalid!");
            }

            let pos: i32 = -GetSystemMetrics(SM_CXVIRTUALSCREEN) - 8;
        }

        return Ok(());
    }

    pub fn render(&self, client_rect: RECT, open_window: HWND) -> Result<()> {
        let hr: HRESULT;

        println!("open_window (from render): {:?}", open_window);

        let dpi: f32 = 96.0;
        let render_target_properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_UNKNOWN, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: dpi,
            dpiY: dpi,
            ..Default::default() };

        /*let render_target_size = D2D_SIZE_U { width: (client_rect.right - client_rect.left) as u32, height: (client_rect.bottom - client_rect.top) as u32 };*/
        let render_target_size = D2D_SIZE_U { width: 100, height: 100};
        println!("render_target_size: {:?}", render_target_size);
        let hwnd_render_target_properties = D2D1_HWND_RENDER_TARGET_PROPERTIES { hwnd: open_window, pixelSize: render_target_size, presentOptions: D2D1_PRESENT_OPTIONS_NONE };
        println!("hwnd_render_target_properties: {:?}", hwnd_render_target_properties);
        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&D2D1_FACTORY_OPTIONS::default()))?;
            let m_render_target = factory.CreateHwndRenderTarget(&render_target_properties, &hwnd_render_target_properties)?;

            m_render_target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            let color = D2D1_COLOR_F { r: 1.0, b: 0.0, g: 0.0, a: 1.0 };
            let m_border_brush = D2D1_BRUSH_PROPERTIES { opacity: 1.0 as f32, transform: std::mem::zeroed() };
            let m_brush = m_render_target.CreateSolidColorBrush(&color, Some(&m_border_brush))?;
            println!("m_brush: {:?}", color);
            let rect = D2D_RECT_F { left: 0.0, top: 0.0, right: 100.0, bottom: 100.0 };
            let rounded_rect = D2D1_ROUNDED_RECT { rect: rect, radiusX: 8.0, radiusY: 8.0 };

            println!("m_render_target: {:?}", m_render_target);

            m_render_target.BeginDraw();
            m_render_target.DrawRoundedRectangle(
                &rounded_rect,
                &m_brush,
                2.0,
                None
            );
            m_render_target.EndDraw(None, None);
        }

        Ok(())
    }
}

    #[link(name = "User32")]
    extern "system" {
        /// [`DefWindowProcW`](https://docs.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-defwindowprocw)
        pub fn DefWindowProcW(
            hWnd: HWND, Msg: u32, wParam: WPARAM, lParam: LPARAM,
        ) -> LRESULT;
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

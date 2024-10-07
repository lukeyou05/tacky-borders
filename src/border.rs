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
    Win32::Graphics::Gdi::*,
    Win32::Graphics::Dwm::*,
    Win32::System::LibraryLoader::GetModuleHandleA,
    Win32::System::SystemServices::IMAGE_DOS_HEADER,
    Win32::UI::WindowsAndMessaging::*,
};

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
        let mut border: Box<WindowBorder> = Box::new(WindowBorder {m_window: HWND(std::ptr::null_mut()), m_tracking_window: window});
        println!("hinstance: {:?}", hinstance);
        println!("border.m_window: {:?}", border.m_window);

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

        let window_class = w!("tacky-border");
        println!("creating window_class");
        unsafe {
            let mut wcex = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(WindowBorder::s_wnd_proc),
                hInstance: hinstance,
                lpszClassName: window_class,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            RegisterClassExW(&wcex);
            println!("wcex.hCursor: {:?}", wcex.hCursor);


            let m_window = CreateWindowExW(
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
                None);

            /*CreateWindowExW(
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
            );

            let mut message = MSG::default();


            while GetMessageW(&mut message, HWND(std::ptr::null_mut()), 0, 0).into() {
                DispatchMessageA(&message);
            }*/
            
            if m_window.is_err() {
                println!("m_window is error!");
            }
            
            let pos: i32 = -GetSystemMetrics(SM_CXVIRTUALSCREEN) - 8;
        }

        return Ok(());
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
            true => return WindowBorder::wnd_proc(message, wparam, lparam),
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
}



use std::ffi::c_ulong;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::prelude::OsStringExt;
use core::ffi::c_void;
use core::ffi::c_int;
/*use winapi::ctypes::c_int;
use winapi::ctypes::c_void;
use winapi::shared::minwindef::{BOOL, LPARAM};
use winapi::shared::windef::HWND;
use winapi::um::shellapi::ShellExecuteExW;
use winapi::um::shellapi::SEE_MASK_NOASYNC;
use winapi::um::shellapi::SEE_MASK_NOCLOSEPROCESS;
use winapi::um::shellapi::SHELLEXECUTEINFOW;*/

/*use winapi::shared::winerror::SUCCEEDED;
use winapi::um::winnt::*;
*use winapi::um::winuser::*;
use winapi::um::dwmapi::*;*/

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
            None => println!("Error at window_rect_opt!"),
        };

        let window_class = w!("wide_border");
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
        }

        

        /*let m_window = CreateWindowExW(WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            "Border",
            "",
            WS_POPUP | WS_DISABLED,
            window_rect.left,
            window_rect.top,
            window_rect.right - window_rect.left,
            window_rect.bottom - window_rect.top,
            std::ptr::null,
            std::ptr::null,
            hinstance,
            self);*/

        return Ok(());
    }

    unsafe extern "system" fn s_wnd_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let this_ref = std::mem::transmute(GetWindowLongPtrW(window, GWLP_USERDATA));
        /*println!("is this a magic cookie or not?: {:?}", GetLongWindowPtrW(window, GWLP_USERDATA));*/
        return this_ref;
    }

    /*pub fn WndProc(message: UINT, wparam: WPARAM, lparam: LPARAM) -> LRESULT {

    }*/
}



/*#![windows_subsystem = "windows"]*/
#![allow(unused)]

use std::ffi::c_ulong;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::prelude::OsStringExt;
use core::ffi::c_void;
use core::ffi::c_int;

mod border;
mod event_hook;

const DWMWA_COLOR_DEFAULT: u32 = 0xFFFFFFFF;
const DWMWA_COLOR_NONE: u32 = 0xFFFFFFFE;
const COLOR_INVALID: u32 = 0x000000FF;

use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::Graphics::Gdi::*,
    Win32::Graphics::Dwm::*,
    Win32::System::LibraryLoader::GetModuleHandleA,
    Win32::System::SystemServices::IMAGE_DOS_HEADER,
    Win32::UI::WindowsAndMessaging::*,
    Win32::UI::Accessibility::*,
};

extern "C" {
    static __ImageBase: IMAGE_DOS_HEADER;
}

// The code below allows me to send a HWND across threads. This can be VERY UNSAFE, and I should
// probably search for whether or not it's okay for a HWND, but it works for now.
pub struct SendHWND(HWND);

unsafe impl Send for SendHWND {}
unsafe impl Sync for SendHWND {}

fn main() {
    /*std::thread::spawn(|| loop {
        println!("Entering thread!");
        apply_colors(false);
        std::thread::sleep(std::time::Duration::from_millis(100));
    });*/
    print!("applying colors\n");
    let m_tracking_window: Option<HWND> = None; 
    print!("finished applying\n");
    register_window_class();

    let mut visible_borders = apply_colors();
    /*loop {
        std::thread::sleep(std::time::Duration::from_millis(1000));
        println!("Destroying borders!");
        if destroy_borders(visible_borders).is_err() {
            println!("Error destroying borders!");
        }
        let new_visible_borders = apply_colors();
        visible_borders = new_visible_borders;
    }*/
    unsafe {
        let mut message = MSG::default();
        while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
            TranslateMessage(&message);
            DispatchMessageW(&message);
            std::thread::sleep(std::time::Duration::from_millis(100))
        }
        println!("Potential error with message loop, exiting!");
    }
}

fn apply_colors() -> Vec<HWND> {
    let mut visible_windows: Vec<HWND> = Vec::new();
    let mut visible_borders: Vec<HWND> = Vec::new();
    unsafe {
        EnumWindows(
            Some(enum_windows_callback),
            LPARAM(&mut visible_windows as *mut _ as isize),
        );
    }

    for hwnd in visible_windows {
        unsafe {
            println!("creating hwnd: {:?}", hwnd);
            /*let active = GetForegroundWindow();
            let string = "#FF0000";
            let rgb_red = hex_to_colorref(&string);
            let rgb_green = 65280 as u32;

            if active == hwnd {
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_BORDER_COLOR,
                    &rgb_red as *const _ as *const c_void, 
                    std::mem::size_of::<c_ulong>() as u32,
                );

                println!("{:X}\n", rgb_red);
            }*/
            
            if IsWindowVisible(hwnd).as_bool() {
                let send = SendHWND(hwnd);
                let thread = std::thread::spawn(move || {
                    // println!("Spawning thread! {:?}", send.0);
                    let border = assign_border(send).unwrap();
                    println!("Exiting thread! Possibly panicked?");
                    std::thread::sleep(std::time::Duration::from_millis(100))
                });
            }
        }
    }
    /*let send = SendHWND(*test);
    let thread = std::thread::spawn(move || {
        println!("Spawning thread! {:?}", send.0);
        assign_border(send);
        println!("Exiting thread!");
        std::thread::sleep(std::time::Duration::from_millis(100))
    });

    let res = thread.join().expect("The thread has panicked");*/
    return visible_borders;
}

pub fn destroy_borders(mut visible_windows: Vec<HWND>) -> Result<()> {
    for hwnd in visible_windows {
        println!("destrying hwnd: {:?}", hwnd);
        unsafe { 
            let result = DestroyWindow(hwnd);
            return result;
        }
    }
    return Ok(());
}

unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
  if IsWindowVisible(hwnd).as_bool() {
    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    // println!("Style: {:x}", style);
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;

    // Exclude certain window styles like WS_EX_TOOLWINDOW
    if ex_style & WS_EX_TOOLWINDOW.0 == 0 && style & WS_POPUP.0 == 0 {
      let visible_windows: &mut Vec<HWND> = std::mem::transmute(lparam);
      visible_windows.push(hwnd);
    }
  }

  BOOL(1)
}

pub fn hex_to_colorref(hex: &str) -> u32 {
  let r = u8::from_str_radix(&hex[1..3], 16);
  let g = u8::from_str_radix(&hex[3..5], 16);
  let b = u8::from_str_radix(&hex[5..7], 16);

  match (r, g, b) {
    (Ok(r), Ok(g), Ok(b)) => (b as u32) << 16 | (g as u32) << 8 | r as u32,
    _ => {
      COLOR_INVALID
    }
  }
}

pub fn register_window_class() -> Result<()> {
    unsafe {
        let window_class = w!("tacky-border");
        // println!("creating window_class");
        let hinstance: HINSTANCE = std::mem::transmute(&__ImageBase);

        let mut wcex = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(border::WindowBorder::s_wnd_proc),
            hInstance: hinstance,
            lpszClassName: window_class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        let atom = RegisterClassExW(&wcex);
        // println!("wcex.hCursor: {:?}", wcex.hCursor);
            
        if atom == 0 {
            let last_error = GetLastError();
            println!("ERROR: RegisterClassExW(&wcex): {:?}", last_error);
        }
    }

    return Ok(());
}

pub fn assign_border(window: SendHWND) -> Option<border::WindowBorder> {
    unsafe {
        /*if window.0 == GetForegroundWindow() {
            let m_hinstance: HINSTANCE = std::mem::transmute(&__ImageBase);
            let border = border::WindowBorder::create(window.0, m_hinstance);
        }*/
        let m_hinstance: HINSTANCE = std::mem::transmute(&__ImageBase);
        let border = border::WindowBorder::create(window.0, m_hinstance);
        return Some(border);
    }
    return None;
}

/*pub fn window_process(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    border::WindowBorder::window_process(hwnd, msg, wparam, lparam);
    LRESULT(0)
}*/

    #[link(name = "User32")]
    extern "system" {
        /// [`DefWindowProcW`](https://docs.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-defwindowprocw)
        pub fn DefWindowProcW(hWnd: HWND, Msg: u32, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
        pub fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> BOOL;
    }

pub fn set_windows_event_hook() {
    unsafe {
        SetWinEventHook(
            EVENT_MIN,
            EVENT_MAX,
            None,
            Some(win_event_hook),
            0,
            0,
            0
        );
    }
}

pub extern "system" fn win_event_hook(
    _h_win_event_hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _id_event_thread: u32,
    _dwms_event_time: u32,
) {

}



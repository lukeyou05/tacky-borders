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

use std::sync::{Arc, Mutex, LazyLock};
use std::cell::Cell;
use std::collections::HashMap;

extern "C" {
    pub static __ImageBase: IMAGE_DOS_HEADER;
}

pub static mut BORDERS: LazyLock<Mutex<HashMap<isize, isize>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
//pub static FACTORY: LazyLock<ID2D1Factory> = unsafe { LazyLock::new(|| D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_MULTI_THREADED, None).expect("REASON")) };

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
    println!("registering window class");
    register_window_class();
    println!("window class is registered!");
    //println!("{:?}", BORDERS.get());

    let mut borders = enum_windows();
    //enum_borders();
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
        set_event_hook();
        //TODO should check whether dpi_aware is true or not
        let dpi_aware = SetProcessDPIAware();
        println!("Entering message loop!");
        let mut message = MSG::default();
        while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
            TranslateMessage(&message);
            DispatchMessageW(&message);
            std::thread::sleep(std::time::Duration::from_millis(10))
        }
        println!("Potential error with message loop, exiting!");
    }
}

pub unsafe fn set_event_hook() {
    SetWinEventHook(
        EVENT_MIN,
        EVENT_MAX,
        None,
        Some(event_hook::handle_win_event_main),
        0,
        0,
        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
    );
}

pub fn enum_windows(){
    println!("In apply_colors!");
    let mut windows: Vec<HWND> = Vec::new();
    //let mut borders = Arc::new(Mutex::new(Vec::new()));
    unsafe {
        EnumWindows(
            Some(enum_windows_callback),
            LPARAM(&mut windows as *mut _ as isize),
        );
    }
    println!("Windows have been enumerated!");
    println!("Windows: {:?}", windows);

    for hwnd in windows {
        println!("Iterating over windows");
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
                let window = SendHWND(hwnd);
                let borders = unsafe{ &*BORDERS };

                let thread = std::thread::spawn(move || {
                    // println!("Spawning thread! {:?}", send.0);
                    let mut borders_sent = borders.lock().unwrap();
                    let mut window_sent = window;

                    let mut border = border::WindowBorder::create(window_sent.0);

                    let window_isize = window_sent.0.0 as isize; 
                    let border_isize = std::ptr::addr_of!(border) as isize;
                    borders_sent.entry(window_isize).or_insert(border_isize);
                    drop(borders_sent);

                    let m_hinstance: HINSTANCE = std::mem::transmute(&__ImageBase);
                    border.init(m_hinstance);

                    //assign_border(send);
                    println!("Exiting thread! Possibly panicked?");
                    //std::thread::sleep(std::time::Duration::from_millis(100))
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
}

pub fn enum_borders() -> Vec<HWND> {
    let mut borders: Vec<HWND> = Vec::new();
    unsafe {
        EnumWindows(
            Some(enum_borders_callback),
            LPARAM(&mut borders as *mut _ as isize),
        );
    }
    return borders;
}


/*pub fn assign_border(window: SendHWND) -> Option<border::WindowBorder> {
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
}*/

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
        //println!("In enum_windows_callback and window is visible!");
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;

        // Exclude certain window styles like WS_EX_TOOLWINDOW
        if ex_style & WS_EX_TOOLWINDOW.0 == 0 && style & WS_POPUP.0 == 0 {
            let visible_windows: &mut Vec<HWND> = std::mem::transmute(lparam.0);
            //println!("lparam: {:?}", lparam.0);
            println!("visible_windows: {:?}", visible_windows);
            visible_windows.push(hwnd);
        }
    }

  BOOL(1)
}

unsafe extern "system" fn enum_borders_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if IsWindowVisible(hwnd).as_bool() {
        let mut class_name = vec![0u16; (MAX_PATH + 1).try_into().unwrap()];
        println!("enum_borders_callback hwnd: {:?}", hwnd);
        GetClassNameW(hwnd, &mut class_name);
        let class = OsString::from_wide(&class_name).to_string_lossy().into_owned();
        println!("enum_borders_callback class_name: {:?}", class);
        let border_class = w!("tacky-border");
        println!("enum_borders_callback border_class: {:?}", border_class);

            let borders: &mut Vec<HWND> = std::mem::transmute(lparam.0);
            borders.push(hwnd);

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



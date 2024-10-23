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
use std::collections::HashMap;

extern "C" {
    pub static __ImageBase: IMAGE_DOS_HEADER;
}

pub static mut BORDERS: LazyLock<Mutex<HashMap<isize, isize>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

// This shit supposedly unsafe af but it works so idgaf. 
pub struct SendHWND(HWND);

unsafe impl Send for SendHWND {}
unsafe impl Sync for SendHWND {}

fn main() {
    println!("registering window class");
    register_window_class();
    println!("window class is registered!");

    let mut borders = enum_windows();

    unsafe {
        set_event_hook();
        //TODO should check whether dpi_aware is true or not
        let dpi_aware = SetProcessDPIAware();

        println!("Entering message loop!");
        let mut message = MSG::default();
        while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
            TranslateMessage(&message);
            DispatchMessageW(&message);
            std::thread::sleep(std::time::Duration::from_millis(100))
        }
        println!("MESSSAGE LOOP IN MAIN.RS EXITED. THIS SHOULD NOT HAPPEN");
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
    let mut windows: Vec<HWND> = Vec::new();
    unsafe {
        EnumWindows(
            Some(enum_windows_callback),
            LPARAM(&mut windows as *mut _ as isize),
        );
    }
    println!("Windows have been enumerated!");
    println!("Windows: {:?}", windows);

    for hwnd in windows {
        unsafe {
            println!("creating border for hwnd: {:?}", hwnd);
            let window = SendHWND(hwnd);
            let borders = unsafe{ &*BORDERS };

            let thread = std::thread::spawn(move || {
                let mut borders_sent = borders.lock().unwrap();
                let mut window_sent = window;

                let mut border = border::WindowBorder::create(window_sent.0);

                let window_isize = window_sent.0.0 as isize; 
                let border_isize = std::ptr::addr_of!(border) as isize;
                borders_sent.entry(window_isize).or_insert(border_isize);
                drop(borders_sent);

                let m_hinstance: HINSTANCE = std::mem::transmute(&__ImageBase);
                border.init(m_hinstance);

                //println!("Exiting thread! Perhaps window closed?");
            });
        }
    }
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

unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if IsWindowVisible(hwnd).as_bool() {
        //println!("In enum_windows_callback and window is visible!");
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;

        // Exclude certain window styles
        // TODO for some reason there are a few non-visible windows that aren't tool windows or
        // child windows. They are however, popup windows, but I don't want to exclude ALL popup
        // windows during the initial window creation process if possible.
        if ex_style & WS_EX_TOOLWINDOW.0 == 0 && style & WS_POPUP.0 == 0 && style & WS_CHILD.0 == 0 {
            let visible_windows: &mut Vec<HWND> = std::mem::transmute(lparam.0);
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

pub fn register_window_class() -> Result<()> {
    unsafe {
        let window_class = w!("tacky-border");
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
            
        if atom == 0 {
            let last_error = GetLastError();
            println!("ERROR: RegisterClassExW(&wcex): {:?}", last_error);
        }
    }

    return Ok(());
}

#[link(name = "User32")]
extern "system" {
    /// [`DefWindowProcW`](https://docs.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-defwindowprocw)
    pub fn DefWindowProcW(hWnd: HWND, Msg: u32, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    pub fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> BOOL;
}

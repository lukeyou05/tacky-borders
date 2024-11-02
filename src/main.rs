//#![allow(unused)]
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::sync::{Mutex, LazyLock};
use std::collections::HashMap;
use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::System::SystemServices::IMAGE_DOS_HEADER,
    Win32::System::Threading::*,
    Win32::UI::WindowsAndMessaging::*,
    Win32::UI::Accessibility::*,
    Win32::UI::HiDpi::*,
};

extern "C" {
    pub static __ImageBase: IMAGE_DOS_HEADER;
}

mod window_border;
mod event_hook;
mod sys_tray_icon;
mod border_config;
mod utils;

use crate::utils::*;

pub static BORDERS: LazyLock<Mutex<HashMap<isize, isize>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

// This is supposedly unsafe af but it works soo 
pub struct SendHWND(HWND);
unsafe impl Send for SendHWND {}
unsafe impl Sync for SendHWND {}

fn main() {
    let dpi_aware = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if dpi_aware.is_err() {
        println!("Failed to make process DPI aware");
    }

    let _ = register_window_class();
    println!("window class is registered!");
    let _ = enum_windows();

    let main_thread = unsafe { GetCurrentThreadId() };
    let tray_icon_option = sys_tray_icon::create_tray_icon(main_thread);
    if tray_icon_option.is_err() {
        println!("Error creating tray icon!");
    }

    let win_event_hook = set_event_hook();
    unsafe {
        println!("Entering message loop!");
        let mut message = MSG::default();
        while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
            if message.message == WM_CLOSE {
                let result = UnhookWinEvent(win_event_hook);
                if result.as_bool() {
                    ExitProcess(0);
                } else {
                    println!("Error. Could not unhook win event hook");
                }
            }

            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
            std::thread::sleep(std::time::Duration::from_millis(16))
        }
        println!("MESSSAGE LOOP IN MAIN.RS EXITED. THIS SHOULD NOT HAPPEN");
    }
}

pub fn register_window_class() -> Result<()> {
    unsafe {
        let window_class = w!("tacky-border");
        let hinstance: HINSTANCE = std::mem::transmute(&__ImageBase);

        let wcex = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_border::WindowBorder::s_wnd_proc),
            hInstance: hinstance,
            lpszClassName: window_class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        let result = RegisterClassExW(&wcex);
            
        if result == 0 {
            let last_error = GetLastError();
            println!("ERROR: RegisterClassExW(&wcex): {:?}", last_error);
        }
    }

    return Ok(());
}

pub fn set_event_hook() -> HWINEVENTHOOK {
    unsafe {
        return SetWinEventHook(
            EVENT_MIN,
            EVENT_MAX,
            None,
            Some(event_hook::handle_win_event_main),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
    }
}

pub fn enum_windows() -> Result<()> {
    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_callback),
            LPARAM::default(),
        );
    }
    println!("Windows have been enumerated!");
    return Ok(());
}

pub fn restart_borders() {
    let mutex = &*BORDERS;
    let mut borders = mutex.lock().unwrap();
    for value in borders.values() {
        let border_window = HWND(*value as *mut _);
        unsafe { SendMessageW(border_window, WM_DESTROY, WPARAM(0), LPARAM(0)) };
    }
    let _ = borders.drain();
    drop(borders);
    let _ = enum_windows();
}

unsafe extern "system" fn enum_windows_callback(_hwnd: HWND, _lparam: LPARAM) -> BOOL {
    // Returning FALSE will exit the EnumWindows loop so we must return TRUE here
    if !is_window_visible(_hwnd) || is_cloaked(_hwnd) || has_filtered_style(_hwnd) {
        return TRUE;
    }

    let _ = create_border_for_window(_hwnd, 0);
    return TRUE; 
}

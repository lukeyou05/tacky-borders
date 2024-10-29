// TODO remove allow unused and fix all the warnings generated
#![allow(unused)]
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::sync::{Arc, Mutex, LazyLock};
use std::collections::HashMap;
use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::System::SystemServices::IMAGE_DOS_HEADER,
    Win32::System::Threading::*,
    Win32::UI::WindowsAndMessaging::*,
    Win32::UI::Accessibility::*,
    Win32::Graphics::Dwm::*,
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

pub static mut BORDERS: LazyLock<Mutex<HashMap<isize, isize>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

// This shit supposedly unsafe af but it works so idgaf. 
pub struct SendHWND(HWND);
unsafe impl Send for SendHWND {}
unsafe impl Sync for SendHWND {}

fn main() {
    register_window_class();
    println!("window class is registered!");
    enum_windows();

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

            TranslateMessage(&message);
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

        let mut wcex = WNDCLASSEXW {
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

pub fn enum_windows() {
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
        create_border_for_window(hwnd, 0);
    }
}

pub fn restart_borders() {
    let mutex = unsafe { &*BORDERS };
    let mut borders = mutex.lock().unwrap();
    for value in borders.values() {
        let border_window = HWND(*value as *mut _);
        unsafe { SendMessageW(border_window, WM_DESTROY, WPARAM(0), LPARAM(0)) };
        // TODO figure out why DestroyWindow doesn't work
        //unsafe { DestroyWindow(border_window) };
    }
    let _ = borders.drain();
    drop(borders);
    enum_windows();
}

unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if IsWindowVisible(hwnd).as_bool() {
        if has_filtered_style(hwnd) || is_cloaked(hwnd) {
            return TRUE;
        }

        let visible_windows: &mut Vec<HWND> = std::mem::transmute(lparam.0);
        visible_windows.push(hwnd);
    }
    TRUE 
}

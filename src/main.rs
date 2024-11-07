//#![allow(unused)]
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use windows::{
    core::*, Win32::Foundation::*, Win32::System::SystemServices::IMAGE_DOS_HEADER,
    Win32::UI::Accessibility::*, Win32::UI::HiDpi::*, Win32::UI::WindowsAndMessaging::*,
};

mod border_config;
mod event_hook;
mod sys_tray_icon;
mod utils;
mod window_border;

use crate::utils::*;

extern "C" {
    pub static __ImageBase: IMAGE_DOS_HEADER;
}

// TODO get rid of the Cell if I never replace it more than once
thread_local! {
    pub static EVENT_HOOK: Cell<HWINEVENTHOOK> = Cell::new(HWINEVENTHOOK::default());
}

pub static BORDERS: LazyLock<Mutex<HashMap<isize, isize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// This is supposedly unsafe af but it works soo + I never dereference anything
pub struct SendHWND(HWND);
unsafe impl Send for SendHWND {}
unsafe impl Sync for SendHWND {}

fn main() {
    let dpi_aware =
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if dpi_aware.is_err() {
        println!("Failed to make process DPI aware");
    }

    let _ = register_window_class();
    println!("window class is registered!");
    let _ = enum_windows();

    let tray_icon_option = sys_tray_icon::create_tray_icon();
    if tray_icon_option.is_err() {
        println!("Error creating tray icon!");
    }

    EVENT_HOOK.replace(set_event_hook());
    unsafe {
        println!("Entering message loop!");
        let mut message = MSG::default();
        while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
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

    Ok(())
}

pub fn set_event_hook() -> HWINEVENTHOOK {
    unsafe {
        SetWinEventHook(
            EVENT_MIN,
            EVENT_MAX,
            None,
            Some(event_hook::handle_win_event_main),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    }
}

pub fn enum_windows() -> Result<()> {
    unsafe {
        let _ = EnumWindows(Some(enum_windows_callback), LPARAM::default());
    }
    println!("Windows have been enumerated!");
    Ok(())
}

pub fn reload_borders() {
    //let event_hook = EVENT_HOOK.get();
    //let result = unsafe { UnhookWinEvent(event_hook) };
    //println!("result of unhooking win event: {:?}", result);

    let mutex = &*BORDERS;
    let mut borders = mutex.lock().unwrap();
    for value in borders.values() {
        let border_window = HWND(*value as _);
        unsafe {
            // DefWindowProcW for WM_CLOSE will call DestroyWindow which will call WM_NCDESTROY and
            // WM_DESTROY
            let _ = PostMessageW(border_window, WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
    borders.clear();
    drop(borders);
    let _ = enum_windows();

    //EVENT_HOOK.replace(set_event_hook());
}

unsafe extern "system" fn enum_windows_callback(_hwnd: HWND, _lparam: LPARAM) -> BOOL {
    // Returning FALSE will exit the EnumWindows loop so we must return TRUE here
    if !is_window_visible(_hwnd) || is_cloaked(_hwnd) || has_filtered_style(_hwnd) {
        return TRUE;
    }

    let _ = create_border_for_window(_hwnd, None);
    TRUE
}

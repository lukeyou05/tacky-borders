#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

#[macro_use]
extern crate log;
extern crate simplelog;

use simplelog::*;
use std::cell::Cell;
use std::collections::HashMap;
use std::fs::File;
use std::sync::{LazyLock, Mutex};
use windows::{
    core::*, Win32::Foundation::*, Win32::System::SystemServices::IMAGE_DOS_HEADER,
    Win32::UI::Accessibility::*, Win32::UI::HiDpi::*, Win32::UI::Input::Ime::*,
    Win32::UI::WindowsAndMessaging::*,
};

mod anim_timer;
mod animations;
mod border_config;
mod colors;
mod event_hook;
mod sys_tray_icon;
mod utils;
mod window_border;

use crate::utils::*;

extern "C" {
    pub static __ImageBase: IMAGE_DOS_HEADER;
}

thread_local! {
    pub static EVENT_HOOK: Cell<HWINEVENTHOOK> = Cell::new(HWINEVENTHOOK::default());
}

pub static BORDERS: LazyLock<Mutex<HashMap<isize, isize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub static INITIAL_WINDOWS: LazyLock<Mutex<Vec<isize>>> = LazyLock::new(|| Mutex::new(Vec::new()));

// This is supposedly unsafe af but it works soo + I never dereference anything
pub struct SendHWND(HWND);
unsafe impl Send for SendHWND {}
unsafe impl Sync for SendHWND {}

fn main() {
    // Note: this Config struct is different from the ones used for the Logger below
    let log_path = border_config::Config::get_config_location().join("tacky-borders.log");

    CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Warn,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        TermLogger::new(
            LevelFilter::Debug,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Info,
            Config::default(),
            // TODO move the log somewhere else like to .config/tacky-borders
            File::create(log_path).unwrap(),
        ),
    ])
    .unwrap();

    // Idk what exactly IME windows do... smth text input language smth... but I don't think we
    // need them. Also, -1 is 0xFFFFFFFF, which we can use to disable IME windows for all threads
    // in the current process.
    if unsafe { !ImmDisableIME(std::mem::transmute::<i32, u32>(-1)).as_bool() } {
        error!("Could not disable IME!");
    }

    if unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_err() }
    {
        error!("Failed to make process DPI aware");
    }

    let tray_icon_result = sys_tray_icon::create_tray_icon();
    if tray_icon_result.is_err() {
        error!("Error creating tray icon!");
    }

    EVENT_HOOK.replace(set_event_hook());
    let _ = register_window_class();
    let _ = enum_windows();

    unsafe {
        debug!("Entering message loop!");
        let mut message = MSG::default();
        while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        error!("MESSSAGE LOOP IN MAIN.RS EXITED. THIS SHOULD NOT HAPPEN");
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
            error!("ERROR: RegisterClassExW(&wcex): {:?}", last_error);
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
            Some(event_hook::handle_win_event),
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
    debug!("Windows have been enumerated!");
    Ok(())
}

pub fn reload_borders() {
    let mut borders = BORDERS.lock().unwrap();
    for value in borders.values() {
        let border_window = HWND(*value as _);
        unsafe {
            // DefWindowProcW for WM_CLOSE will call DestroyWindow which will send WM_NCDESTROY and
            // WM_DESTROY messages
            let _ = PostMessageW(border_window, WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
    borders.clear();
    drop(borders);

    INITIAL_WINDOWS.lock().unwrap().clear();

    let _ = enum_windows();
}

unsafe extern "system" fn enum_windows_callback(_hwnd: HWND, _lparam: LPARAM) -> BOOL {
    if !has_filtered_style(_hwnd) {
        if is_window_visible(_hwnd) && !is_cloaked(_hwnd) {
            let _ = create_border_for_window(_hwnd);
        }

        INITIAL_WINDOWS.lock().unwrap().push(_hwnd.0 as isize);
    }
    TRUE
}

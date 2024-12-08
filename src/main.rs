#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

#[macro_use]
extern crate log;
extern crate simplelog;

use anyhow::Context;
use simplelog::*;
use std::cell::Cell;
use std::collections::HashMap;
use std::fs::File;
use std::sync::{LazyLock, Mutex};
use windows::core::w;
use windows::Win32::Foundation::{GetLastError, BOOL, HINSTANCE, HWND, LPARAM, TRUE, WPARAM};
use windows::Win32::System::SystemServices::IMAGE_DOS_HEADER;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EnumWindows, GetMessageW, LoadCursorW, RegisterClassExW, TranslateMessage,
    EVENT_MAX, EVENT_MIN, IDC_ARROW, MSG, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    WM_NCDESTROY, WNDCLASSEXW,
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
    static __ImageBase: IMAGE_DOS_HEADER;
}

thread_local! {
    static EVENT_HOOK: Cell<HWINEVENTHOOK> = Cell::new(HWINEVENTHOOK::default());
}

static BORDERS: LazyLock<Mutex<HashMap<isize, isize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static INITIAL_WINDOWS: LazyLock<Mutex<Vec<isize>>> = LazyLock::new(|| Mutex::new(Vec::new()));

// This is used to send HWNDs across threads even though HWND doesn't implement Send and Sync.
struct SendHWND(HWND);
unsafe impl Send for SendHWND {}
unsafe impl Sync for SendHWND {}

fn main() {
    if let Err(e) = create_logger() {
        println!("[ERROR] {}", e);
    };

    // xFFFFFFFF can be used to disable IME windows for all threads in the current process.
    if !imm_disable_ime(0xFFFFFFFF).as_bool() {
        error!("could not disable ime!");
    }

    if let Err(e) = set_process_dpi_awareness_context(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) {
        error!("could not make process dpi aware: {e}");
    }

    // This is responsible for the actual tray icon window, so it must be kept in scope
    let tray_icon_result = sys_tray_icon::create_tray_icon();
    if let Err(e) = tray_icon_result {
        // TODO for some reason if I use {:#} or {:?}, it repeatedly prints the error. Could be
        // something to do with how it implements .source()?
        error!("could not create tray icon: {e:#?}");
    }

    EVENT_HOOK.replace(set_event_hook());
    log_if_err!(register_window_class());
    log_if_err!(enum_windows());

    unsafe {
        debug!("entering message loop!");
        let mut message = MSG::default();
        while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        error!("exited messsage loop in main.rs; this should not happen");
    }
}

fn create_logger() -> anyhow::Result<()> {
    let log_dir = border_config::Config::get_config_dir()?;
    let log_path = log_dir.join("tacky-borders.log");

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
            File::create(log_path).unwrap(),
        ),
    ])?;

    Ok(())
}

fn register_window_class() -> windows::core::Result<()> {
    unsafe {
        let hinstance: HINSTANCE = std::mem::transmute(&__ImageBase);

        let window_class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_border::WindowBorder::s_wnd_proc),
            hInstance: hinstance,
            lpszClassName: w!("border"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };

        let result = RegisterClassExW(&window_class);
        if result == 0 {
            let last_error = GetLastError();
            error!("could not register window class: {last_error:?}");
        }
    }

    Ok(())
}

fn set_event_hook() -> HWINEVENTHOOK {
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

fn enum_windows() -> windows::core::Result<()> {
    unsafe {
        EnumWindows(Some(enum_windows_callback), LPARAM::default())?;
    }
    debug!("windows have been enumerated!");
    Ok(())
}

fn reload_borders() {
    let mut borders = BORDERS.lock().unwrap();

    // Send destroy messages to all the border windows
    for value in borders.values() {
        let border_window = HWND(*value as _);
        log_if_err!(
            post_message_w(border_window, WM_NCDESTROY, WPARAM(0), LPARAM(0))
                .context("reload_borders")
        );
    }

    // Clear the borders hashmap
    borders.clear();
    drop(borders);

    // Clear the initial windows list
    INITIAL_WINDOWS.lock().unwrap().clear();

    log_if_err!(enum_windows());
}

unsafe extern "system" fn enum_windows_callback(_hwnd: HWND, _lparam: LPARAM) -> BOOL {
    if !has_filtered_style(_hwnd) {
        if is_window_visible(_hwnd) && !is_cloaked(_hwnd) {
            create_border_for_window(_hwnd);
        }

        // Add currently open windows to the intial windows list so we can keep track of them
        INITIAL_WINDOWS.lock().unwrap().push(_hwnd.0 as isize);
    }

    TRUE
}

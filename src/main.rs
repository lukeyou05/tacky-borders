#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

#[macro_use]
extern crate log;
extern crate sp_log;

use anyhow::{anyhow, Context};
use sp_log::{
    ColorChoice, CombinedLogger, Config, FileLogger, LevelFilter, TermLogger, TerminalMode,
};
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use windows::core::w;
use windows::Win32::Foundation::{GetLastError, BOOL, HWND, LPARAM, TRUE, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
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

use crate::border_config::EnableMode;
use crate::utils::{
    create_border_for_window, get_window_rule, has_filtered_style, imm_disable_ime,
    is_window_cloaked, is_window_top_level, is_window_visible, post_message_w,
    set_process_dpi_awareness_context, LogIfErr,
};

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

    info!("starting tacky-borders");

    // xFFFFFFFF can be used to disable IME windows for all threads in the current process.
    if !imm_disable_ime(0xFFFFFFFF).as_bool() {
        error!("could not disable ime!");
    }

    set_process_dpi_awareness_context(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
        .context("could not make process dpi aware")
        .log_if_err();

    // This is responsible for the actual tray icon window, so it must be kept in scope
    let tray_icon_res = sys_tray_icon::create_tray_icon();
    if let Err(e) = tray_icon_res {
        // TODO for some reason if I use {:#} or {:?}, it repeatedly prints the error. Could be
        // something to do with how it implements .source()?
        error!("could not create tray icon: {e:#?}");
    }

    let hwineventhook = set_event_hook();
    EVENT_HOOK.replace(hwineventhook);

    register_window_class().log_if_err();
    enum_windows().log_if_err();

    unsafe {
        let mut message = MSG::default();
        while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    info!("exiting tacky-borders");
}

fn create_logger() -> anyhow::Result<()> {
    let log_path = border_config::Config::get_config_dir()?.join("tacky-borders.log");
    let Some(path_str) = log_path.to_str() else {
        return Err(anyhow!("could not convert log_path to str"));
    };

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
        FileLogger::new(
            LevelFilter::Info,
            Config::default(),
            path_str,
            // 1 MB
            Some(1024 * 1024),
        ),
    ])?;

    Ok(())
}

fn register_window_class() -> windows::core::Result<()> {
    unsafe {
        let window_class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_border::WindowBorder::s_wnd_proc),
            hInstance: GetModuleHandleW(None)?.into(),
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
            Some(event_hook::process_win_event),
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
        post_message_w(border_window, WM_NCDESTROY, WPARAM(0), LPARAM(0))
            .context("reload_borders")
            .log_if_err();
    }

    // Clear the borders hashmap
    borders.clear();
    drop(borders);

    // Clear the initial windows list
    INITIAL_WINDOWS.lock().unwrap().clear();

    enum_windows().log_if_err();
}

unsafe extern "system" fn enum_windows_callback(_hwnd: HWND, _lparam: LPARAM) -> BOOL {
    if is_window_top_level(_hwnd) {
        // Only create borders for visible windows
        if is_window_visible(_hwnd) && !is_window_cloaked(_hwnd) {
            let window_rule = get_window_rule(_hwnd);

            if window_rule.enabled == Some(EnableMode::Bool(false)) {
                info!("border is disabled for {_hwnd:?}");
            } else if window_rule.enabled == Some(EnableMode::Bool(true))
                || !has_filtered_style(_hwnd)
            {
                create_border_for_window(_hwnd, window_rule);
            }
        }

        // Add currently open windows to the intial windows list so we can keep track of them
        INITIAL_WINDOWS.lock().unwrap().push(_hwnd.0 as isize);
    }

    TRUE
}

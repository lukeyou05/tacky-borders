#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

#[macro_use]
extern crate log;
extern crate sp_log;

use anyhow::{anyhow, Context};
use sp_log::{ColorChoice, CombinedLogger, FileLogger, LevelFilter, TermLogger, TerminalMode};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, RwLock};
use utils::get_foreground_window;
use windows::core::w;
use windows::Win32::Foundation::{GetLastError, BOOL, HWND, LPARAM, TRUE, WPARAM};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Factory, D2D1_FACTORY_TYPE_MULTI_THREADED,
};
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

use crate::border_config::{Config, ConfigWatcher, EnableMode};
use crate::utils::{
    create_border_for_window, get_window_rule, has_filtered_style, imm_disable_ime,
    is_window_cloaked, is_window_top_level, is_window_visible, post_message_w,
    set_process_dpi_awareness_context, LogIfErr,
};

// TODO: dunno if I should pass an Arc ptr of this to other functions/structs
static APP_STATE: LazyLock<AppState> = LazyLock::new(AppState::new);

struct AppState {
    borders: Mutex<HashMap<isize, isize>>,
    initial_windows: Mutex<Vec<isize>>,
    active_window: Mutex<isize>,
    is_polling_active_window: AtomicBool,
    config: RwLock<Config>,
    config_watcher: Mutex<ConfigWatcher>,
    render_factory: ID2D1Factory,
}

impl AppState {
    fn new() -> Self {
        let active_window = get_foreground_window().0 as isize;

        // TODO: right now we use unwrap_or_default(), but I should probably handle the Err
        let mut config_watcher = ConfigWatcher::new(
            Config::get_dir().unwrap_or_default().join("config.yaml"),
            500,
            Config::config_watcher_callback,
        );

        let config = match Config::create() {
            Ok(config) => {
                if config.watch_config_changes {
                    config_watcher.start().log_if_err();
                }
                config
            }
            Err(err) => {
                error!("could not read config.yaml: {err:#}");
                Config::default()
            }
        };

        let render_factory = unsafe {
            D2D1CreateFactory(D2D1_FACTORY_TYPE_MULTI_THREADED, None).unwrap_or_else(|err| {
                error!("could not create ID2D1Factory: {err}");
                panic!()
            })
        };

        AppState {
            borders: Mutex::new(HashMap::new()),
            initial_windows: Mutex::new(Vec::new()),
            active_window: Mutex::new(active_window),
            is_polling_active_window: AtomicBool::new(false),
            config: RwLock::new(config),
            config_watcher: Mutex::new(config_watcher),
            render_factory,
        }
    }

    fn is_polling_active_window(&self) -> bool {
        self.is_polling_active_window.load(Ordering::SeqCst)
    }

    fn set_polling_active_window(&self, val: bool) {
        self.is_polling_active_window.store(val, Ordering::SeqCst);
    }
}

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

    let hwineventhook = set_event_hook();

    // This is responsible for the actual tray icon window, so it must be kept in scope
    let tray_icon_res = sys_tray_icon::create_tray_icon(hwineventhook);
    if let Err(e) = tray_icon_res {
        // TODO for some reason if I use {:#} or {:?}, it repeatedly prints the error. Could be
        // something to do with how it implements .source()?
        error!("could not create tray icon: {e:#?}");
    }

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
    // NOTE: there are two Config structs in this function: tacky-borders' and sp_log's
    let log_path = Config::get_dir()?.join("tacky-borders.log");
    let Some(path_str) = log_path.to_str() else {
        return Err(anyhow!("could not convert log_path to str"));
    };

    CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Warn,
            sp_log::Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        TermLogger::new(
            LevelFilter::Debug,
            sp_log::Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        FileLogger::new(
            LevelFilter::Info,
            sp_log::Config::default(),
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
    let mut borders = APP_STATE.borders.lock().unwrap();

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
    APP_STATE.initial_windows.lock().unwrap().clear();

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
        APP_STATE
            .initial_windows
            .lock()
            .unwrap()
            .push(_hwnd.0 as isize);
    }

    TRUE
}

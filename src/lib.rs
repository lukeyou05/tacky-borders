#[macro_use]
extern crate log;
extern crate sp_log;

use anyhow::{Context, anyhow};
use config::{Config, ConfigWatcher, EnableMode, config_watcher_callback};
use core::time;
use komorebi::KomorebiIntegration;
use render_backend::RenderBackendConfig;
use sp_log::{ColorChoice, CombinedLogger, FileLogger, LevelFilter, TermLogger, TerminalMode};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, RwLock};
use std::thread;
use utils::{
    LogIfErr, create_border_for_window, get_foreground_window, get_last_error, get_window_rule,
    has_filtered_style, is_window_cloaked, is_window_top_level, is_window_visible, post_message_w,
    send_message_w,
};
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Foundation::{ERROR_CLASS_ALREADY_EXISTS, HMODULE, HWND, LPARAM, TRUE};
use windows::Win32::Graphics::Direct2D::{
    D2D1_FACTORY_TYPE_MULTI_THREADED, D2D1CreateFactory, ID2D1Device4, ID2D1Factory5,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_9_1, D3D_FEATURE_LEVEL_9_2,
    D3D_FEATURE_LEVEL_9_3, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0,
    D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_MAX, EVENT_MIN, EnumWindows, IDC_ARROW, LoadCursorW, MB_ICONERROR, MB_OK,
    MB_SETFOREGROUND, MB_TOPMOST, MessageBoxW, RegisterClassExW, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS, WM_NCDESTROY, WNDCLASSEXW,
};
use windows::core::{BOOL, Interface, PCWSTR, w};

pub mod anim_timer;
pub mod animations;
pub mod border_drawer;
pub mod colors;
pub mod config;
pub mod effects;
pub mod event_hook;
pub mod iocp;
pub mod komorebi;
pub mod render_backend;
pub mod sys_tray_icon;
pub mod utils;
pub mod window_border;

static IS_WINDOWS_11: LazyLock<bool> = LazyLock::new(|| {
    let mut version_info = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    unsafe { RtlGetVersion(&mut version_info) }
        .ok()
        .log_if_err();

    debug!(
        "windows version: {}.{}.{}",
        version_info.dwMajorVersion, version_info.dwMinorVersion, version_info.dwBuildNumber
    );

    version_info.dwBuildNumber >= 22000
});
static APP_STATE: LazyLock<AppState> = LazyLock::new(AppState::new);

struct AppState {
    borders: Mutex<HashMap<isize, isize>>,
    initial_windows: Mutex<Vec<isize>>,
    active_window: Mutex<isize>,
    is_polling_active_window: AtomicBool,
    config: RwLock<Config>,
    config_watcher: Mutex<ConfigWatcher>,
    render_factory: ID2D1Factory5,
    directx_devices: RwLock<Option<DirectXDevices>>,
    komorebi_integration: Mutex<KomorebiIntegration>,
}

unsafe impl Send for AppState {}
unsafe impl Sync for AppState {}

impl AppState {
    fn new() -> Self {
        let active_window = get_foreground_window().0 as isize;

        let mut config_watcher = ConfigWatcher::new(
            Config::get_dir()
                .map(|dir| dir.join("config.yaml"))
                .unwrap_or_else(|err| {
                    error!("could not get path for config watcher: {err}");
                    PathBuf::default()
                }),
            500,
            config_watcher_callback,
        );

        let mut komorebi_integration = KomorebiIntegration::new();

        let config = match Config::create() {
            Ok(config) => {
                if config_watcher.is_enabled(&config) {
                    config_watcher.start().log_if_err();
                }

                if komorebi_integration.is_enabled(&config) {
                    komorebi_integration.start().log_if_err();
                }

                config
            }
            Err(err) => {
                error!("could not read config: {err:#}");
                display_error_box(format!("could not read config: {err:#}"));

                Config::default()
            }
        };

        let render_factory: ID2D1Factory5 = unsafe {
            D2D1CreateFactory(D2D1_FACTORY_TYPE_MULTI_THREADED, None).unwrap_or_else(|err| {
                error!("could not create ID2D1Factory: {err}");
                panic!()
            })
        };

        let directx_devices_opt = match config.render_backend {
            RenderBackendConfig::V2 => {
                // I think I have to just panic if .unwrap() fails tbh; don't know what else I could do.
                let directx_devices = DirectXDevices::new(&render_factory).unwrap_or_else(|err| {
                    error!("could not create directx devices: {err}");
                    panic!("could not create directx devices: {err}");
                });

                Some(directx_devices)
            }
            RenderBackendConfig::Legacy => None,
        };

        AppState {
            borders: Mutex::new(HashMap::new()),
            initial_windows: Mutex::new(Vec::new()),
            active_window: Mutex::new(active_window),
            is_polling_active_window: AtomicBool::new(false),
            config: RwLock::new(config),
            config_watcher: Mutex::new(config_watcher),
            render_factory,
            directx_devices: RwLock::new(directx_devices_opt),
            komorebi_integration: Mutex::new(komorebi_integration),
        }
    }

    fn is_polling_active_window(&self) -> bool {
        self.is_polling_active_window.load(Ordering::SeqCst)
    }

    fn set_polling_active_window(&self, val: bool) {
        self.is_polling_active_window.store(val, Ordering::SeqCst);
    }
}

struct DirectXDevices {
    d3d11_device: ID3D11Device,
    dxgi_device: IDXGIDevice,
    d2d_device: ID2D1Device4,
}

impl DirectXDevices {
    fn new(factory: &ID2D1Factory5) -> anyhow::Result<Self> {
        let creation_flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;

        let feature_levels = [
            D3D_FEATURE_LEVEL_11_1,
            D3D_FEATURE_LEVEL_11_0,
            D3D_FEATURE_LEVEL_10_1,
            D3D_FEATURE_LEVEL_10_0,
            D3D_FEATURE_LEVEL_9_3,
            D3D_FEATURE_LEVEL_9_2,
            D3D_FEATURE_LEVEL_9_1,
        ];

        let mut device_opt: Option<ID3D11Device> = None;
        let mut feature_level: D3D_FEATURE_LEVEL = D3D_FEATURE_LEVEL::default();

        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                creation_flags,
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut device_opt),
                Some(&mut feature_level),
                None,
            )
        }?;

        debug!("directx feature_level: {feature_level:X?}");

        let d3d11_device = device_opt.context("could not get d3d11 device")?;
        let dxgi_device: IDXGIDevice = d3d11_device.cast().context("id3d11device cast")?;
        let d2d_device = unsafe { factory.CreateDevice(&dxgi_device) }.context("d2d_device")?;

        Ok(Self {
            d3d11_device,
            dxgi_device,
            d2d_device,
        })
    }
}

pub fn create_logger() -> anyhow::Result<()> {
    // NOTE: there are two Config structs in this function: tacky-borders' and sp_log's
    let log_path = crate::Config::get_dir()?.join("tacky-borders.log");
    let Some(path_str) = log_path.to_str() else {
        return Err(anyhow!("could not convert log_path to str"));
    };

    CombinedLogger::init(vec![
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

pub fn register_border_window_class() -> anyhow::Result<()> {
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
            let last_error = get_last_error();
            if last_error != ERROR_CLASS_ALREADY_EXISTS {
                return Err(anyhow!("could not register window class: {last_error:?}"));
            }
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
            Some(event_hook::process_win_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    }
}

pub fn create_borders_for_existing_windows() -> windows::core::Result<()> {
    unsafe { EnumWindows(Some(create_borders_callback), LPARAM::default()) }?;
    debug!("windows have been enumerated!");

    Ok(())
}

pub fn destroy_borders() {
    const MAX_ATTEMPTS: u32 = 3;

    for i in 0..MAX_ATTEMPTS {
        // Copy the hashmap's values to prevent mutex deadlocks
        let border_hwnds: Vec<HWND> = APP_STATE
            .borders
            .lock()
            .unwrap()
            .values()
            .map(|hwnd_isize| HWND(*hwnd_isize as _))
            .collect();

        for hwnd in border_hwnds {
            let _ = send_message_w(hwnd, WM_NCDESTROY, None, None);
        }

        // SendMessageW ensures that the border windows have processed their messages, but it
        // does not guarantee that the thread has exited, so we still must wait a few ms
        thread::sleep(time::Duration::from_millis(5));

        let remaining_borders = APP_STATE.borders.lock().unwrap();
        if remaining_borders.is_empty() {
            break;
        } else if i == MAX_ATTEMPTS - 1 {
            error!(
                "could not successfully destroy all borders (still remaining: {:?})",
                *remaining_borders
            );
        }
    }

    // NOTE: we will rely on each border thread to remove themselves from the hashmap, so we won't
    // do any manual cleanup here
}

pub fn reload_borders() {
    destroy_borders();
    APP_STATE.initial_windows.lock().unwrap().clear();
    create_borders_for_existing_windows().log_if_err();
}

pub fn display_error_box<T: std::fmt::Display>(err: T) {
    let error_vec: Vec<u16> = err
        .to_string()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let _ = thread::spawn(move || {
        let _ = unsafe {
            MessageBoxW(
                None,
                PCWSTR(error_vec.as_ptr()),
                w!("Error!"),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST,
            )
        };
    });
}

unsafe extern "system" fn create_borders_callback(_hwnd: HWND, _lparam: LPARAM) -> BOOL {
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

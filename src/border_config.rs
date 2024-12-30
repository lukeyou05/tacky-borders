use crate::animations::AnimationsConfig;
use crate::colors::ColorConfig;
use crate::reload_borders;
use crate::utils::{get_adjusted_radius, get_window_corner_preference, LogIfErr};
use anyhow::{anyhow, Context};
use dirs::home_dir;
use serde::{Deserialize, Serialize};
use std::fs::{self, DirBuilder};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, RwLock};
use std::{iter, ptr, slice, thread, time};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, FALSE, HANDLE, HWND};
use windows::Win32::Graphics::Dwm::{
    DWMWCP_DEFAULT, DWMWCP_DONOTROUND, DWMWCP_ROUND, DWMWCP_ROUNDSMALL,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::CancelIoEx;

pub static CONFIG: LazyLock<RwLock<Config>> = LazyLock::new(|| {
    RwLock::new(match Config::create_config() {
        Ok(config) => config,
        Err(e) => {
            error!("could not read config.yaml: {e:#}");
            Config::default()
        }
    })
});

pub static CONFIG_WATCHER: LazyLock<Mutex<ConfigWatcher>> = LazyLock::new(|| {
    // TODO: right now we use unwrap_or_default() to prevent panic, but we should probably handle it
    Mutex::new(ConfigWatcher::new(
        Config::get_config_dir()
            .unwrap_or_default()
            .join("config.yaml"),
        500,
        Config::config_watcher_callback,
    ))
});

const DEFAULT_CONFIG: &str = include_str!("resources/config.yaml");

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub watch_config_changes: bool,
    #[serde(default = "serde_default_global")]
    pub global: Global,
    #[serde(default)]
    pub window_rules: Vec<WindowRule>,
}

// Show borders even if the config.yaml is completely empty
// NOTE: this is just for serde and is intentionally kept separate from the Default trait
// because I still want the width and offset zeroed out when I call Config::default()
fn serde_default_global() -> Global {
    Global {
        border_width: serde_default_f32::<4>(),
        border_offset: serde_default_i32::<-1>(),
        ..Default::default()
    }
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Global {
    #[serde(default = "serde_default_f32::<4>")]
    pub border_width: f32,
    #[serde(default = "serde_default_i32::<-1>")]
    pub border_offset: i32,
    #[serde(default)]
    pub border_radius: RadiusConfig,
    #[serde(default)]
    pub active_color: ColorConfig,
    #[serde(default)]
    pub inactive_color: ColorConfig,
    #[serde(default)]
    pub animations: AnimationsConfig,
    #[serde(alias = "init_delay")]
    #[serde(default = "serde_default_u64::<250>")]
    pub initialize_delay: u64, // Adjust delay when creating new windows/borders
    #[serde(alias = "restore_delay")]
    #[serde(default = "serde_default_u64::<200>")]
    pub unminimize_delay: u64, // Adjust delay when restoring minimized windows
}

pub fn serde_default_u64<const V: u64>() -> u64 {
    V
}

pub fn serde_default_i32<const V: i32>() -> i32 {
    V
}

// f32 cannot be a const, so we have to do the following instead
pub fn serde_default_f32<const V: i32>() -> f32 {
    V as f32
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WindowRule {
    #[serde(rename = "match")]
    pub kind: Option<MatchKind>,
    pub name: Option<String>,
    pub strategy: Option<MatchStrategy>,
    pub border_width: Option<f32>,
    pub border_offset: Option<i32>,
    pub border_radius: Option<RadiusConfig>,
    pub active_color: Option<ColorConfig>,
    pub inactive_color: Option<ColorConfig>,
    pub enabled: Option<EnableMode>,
    pub animations: Option<AnimationsConfig>,
    #[serde(alias = "init_delay")]
    pub initialize_delay: Option<u64>,
    #[serde(alias = "restore_delay")]
    pub unminimize_delay: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum MatchKind {
    Title,
    Class,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum MatchStrategy {
    Equals,
    Contains,
    Regex,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub enum RadiusConfig {
    #[default]
    Auto,
    Square,
    Round,
    RoundSmall,
    #[serde(untagged)]
    Custom(f32),
}

impl RadiusConfig {
    pub fn to_radius(&self, border_width: i32, dpi: f32, tracking_window: HWND) -> f32 {
        match self {
            // We also check Custom(-1.0) for legacy reasons (don't wanna break anyone's old config)
            RadiusConfig::Auto | RadiusConfig::Custom(-1.0) => {
                match get_window_corner_preference(tracking_window) {
                    // TODO check if the user is running Windows 11 or 10
                    DWMWCP_DEFAULT => get_adjusted_radius(8.0, dpi, border_width),
                    DWMWCP_DONOTROUND => 0.0,
                    DWMWCP_ROUND => get_adjusted_radius(8.0, dpi, border_width),
                    DWMWCP_ROUNDSMALL => get_adjusted_radius(4.0, dpi, border_width),
                    _ => 0.0,
                }
            }
            RadiusConfig::Square => 0.0,
            RadiusConfig::Round => get_adjusted_radius(8.0, dpi, border_width),
            RadiusConfig::RoundSmall => get_adjusted_radius(4.0, dpi, border_width),
            RadiusConfig::Custom(radius) => radius * dpi / 96.0,
        }
    }
}
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub enum EnableMode {
    #[default]
    Auto,
    #[serde(untagged)]
    Bool(bool),
}

impl Config {
    pub fn create_config() -> anyhow::Result<Self> {
        let config_dir = Self::get_config_dir()?;
        let config_path = config_dir.join("config.yaml");

        // If the config.yaml does not exist, try to create it
        if !fs::exists(&config_path).context("could not check if config path exists")? {
            let default_contents = DEFAULT_CONFIG.as_bytes();
            fs::write(&config_path, default_contents)
                .context("could not create default config.yaml")?;

            info!("generating default config in {}", config_dir.display());
        }

        let contents = fs::read_to_string(&config_path).context("could not read config.yaml")?;
        let config: Config = serde_yml::from_str(&contents)?;

        if config.watch_config_changes && !CONFIG_WATCHER.lock().unwrap().is_running() {
            CONFIG_WATCHER.lock().unwrap().start()?;
        } else if !config.watch_config_changes && CONFIG_WATCHER.lock().unwrap().is_running() {
            CONFIG_WATCHER.lock().unwrap().stop()?;
        }

        Ok(config)
    }

    pub fn get_config_dir() -> anyhow::Result<PathBuf> {
        let Some(home_dir) = home_dir() else {
            return Err(anyhow!("could not find home directory!"));
        };

        let config_dir = home_dir.join(".config").join("tacky-borders");

        // If the config directory doesn't exist, try to create it
        if !config_dir.exists() {
            DirBuilder::new()
                .recursive(true)
                .create(&config_dir)
                .context("could not create config directory")?;
        };

        Ok(config_dir)
    }

    pub fn reload_config() {
        let new_config = match Self::create_config() {
            Ok(config) => config,
            Err(e) => {
                error!("could not reload config: {e:#}");
                Config::default()
            }
        };
        *CONFIG.write().unwrap() = new_config;
    }

    fn config_watcher_callback() {
        let old_config = (*CONFIG.read().unwrap()).clone();
        Self::reload_config();
        let new_config = CONFIG.read().unwrap();

        if old_config != *new_config {
            info!("config.yaml has changed; reloading borders");
            reload_borders();
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigWatcher {
    config_path: PathBuf,
    debounce_time: time::Duration,
    callback_fn: fn(),
    config_dir_handle: Option<isize>,
}

impl ConfigWatcher {
    pub fn new(config_path: PathBuf, debounce_time: u64, callback_fn: fn()) -> Self {
        Self {
            config_path,
            debounce_time: time::Duration::from_millis(debounce_time),
            callback_fn,
            config_dir_handle: None,
        }
    }

    pub fn start(&mut self) -> anyhow::Result<()> {
        debug!("starting config watcher");

        if self.is_running() {
            return Err(anyhow!("config watcher is already running"));
        }

        // NOTE: apparently you can use context() on an Option lol
        let config_dir = self
            .config_path
            .parent()
            .context("could not get parent dir for config watcher")?;
        let config_dir_vec: Vec<u16> = config_dir
            .as_os_str()
            .encode_wide()
            .chain(iter::once(0))
            .collect();

        let dir_handle = unsafe {
            CreateFileW(
                PCWSTR(config_dir_vec.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                HANDLE::default(),
            )
            .context("could not create dir handle for config watcher")?
        };

        // Convert HANDLE to isize so we can move it into the new thread
        let dir_handle_isize = dir_handle.0 as isize;
        self.config_dir_handle = Some(dir_handle_isize);

        // Also initialize these variables so we move them into the new thread
        let config_name = self
            .config_path
            .file_name()
            .context("could not get config name for config watcher")?
            .to_owned()
            .into_string()
            .map_err(|_| anyhow!("could not convert config name for config watcher"))?;
        let debounce_time = self.debounce_time;
        let callback_fn = self.callback_fn;

        let _ = thread::spawn(move || unsafe {
            // Reconvert isize back to HANDLE
            let dir_handle = HANDLE(dir_handle_isize as _);

            let mut buffer = [0u8; 1024];
            let mut bytes_returned = 0u32;

            let mut now = time::Instant::now();

            loop {
                if let Err(e) = ReadDirectoryChangesW(
                    dir_handle,
                    buffer.as_mut_ptr() as _,
                    buffer.len() as u32,
                    FALSE,
                    FILE_NOTIFY_CHANGE_LAST_WRITE,
                    Some(ptr::addr_of_mut!(bytes_returned)),
                    None,
                    None,
                ) {
                    error!("could not check for changes in config dir: {e}");
                    break;
                }

                // Prevent too many directory checks in quick succession
                if now.elapsed() < debounce_time {
                    thread::sleep(debounce_time - now.elapsed());
                }

                Self::process_dir_change_notifs(&buffer, bytes_returned, &config_name, callback_fn);
                now = time::Instant::now();
            }

            debug!("exiting config watcher thread");
        });

        Ok(())
    }

    pub fn process_dir_change_notifs(
        buffer: &[u8; 1024],
        bytes_returned: u32,
        config_name: &str,
        callback_fn: fn(),
    ) {
        let mut offset = 0usize;

        while offset < bytes_returned as usize {
            let info = unsafe { &*(buffer.as_ptr().add(offset) as *const FILE_NOTIFY_INFORMATION) };

            // We divide FileNameLength by 2 because it's in bytes (u8), but FileName is in u16
            let name_slice = unsafe {
                slice::from_raw_parts(info.FileName.as_ptr(), info.FileNameLength as usize / 2)
            };
            let file_name = String::from_utf16_lossy(name_slice);
            debug!("file changed: {}", file_name);

            if file_name == *config_name {
                callback_fn();
                break; // Prevent multiple callbacks from the same notification
            }

            // If NextEntryOffset = 0, then we have reached the end of the notification
            if info.NextEntryOffset == 0 {
                break;
            } else {
                offset += info.NextEntryOffset as usize
            }
        }
    }

    pub fn stop(&mut self) -> anyhow::Result<()> {
        debug!("stopping config watcher");

        if let Some(dir_handle_isize) = self.config_dir_handle {
            let dir_handle = HANDLE(dir_handle_isize as _);

            // Cancel all pending I/O operations on the handle
            unsafe { CancelIoEx(dir_handle, None) }.log_if_err();

            // Close the handle for cleanup. This should automatically exit the watcher thread.
            let res = unsafe { CloseHandle(dir_handle) }.map_err(anyhow::Error::new);

            // Reset the config dir handle if we successfully closed it
            if res.is_ok() {
                self.config_dir_handle = None;
            }

            res
        } else {
            Err(anyhow!("config watcher is not running"))
        }
    }

    pub fn is_running(&self) -> bool {
        self.config_dir_handle.is_some()
    }
}

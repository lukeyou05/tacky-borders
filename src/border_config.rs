use crate::animations::Animations;
use crate::colors::ColorConfig;
use crate::reload_borders;
use crate::utils::{get_adjusted_radius, get_window_corner_preference, LogIfErr};
use anyhow::{anyhow, Context};
use dirs::home_dir;
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::fs::{self, DirBuilder};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};
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

thread_local! {
    static CONFIG_DIR_HANDLE: Cell<Option<isize>> = Cell::new(None);
}

pub static CONFIG: LazyLock<RwLock<Config>> = LazyLock::new(|| {
    RwLock::new(match Config::create_config() {
        Ok(config) => config,
        Err(e) => {
            error!("could not read config.yaml: {e:#}");
            Config::default()
        }
    })
});

const DEFAULT_CONFIG: &str = include_str!("resources/config.yaml");

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub global: Global,
    pub window_rules: Vec<WindowRule>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Global {
    pub border_width: f32,
    pub border_offset: i32,
    pub border_radius: RadiusConfig,
    pub active_color: ColorConfig,
    pub inactive_color: ColorConfig,
    pub animations: Option<Animations>,
    #[serde(alias = "init_delay")]
    pub initialize_delay: Option<u64>, // Adjust delay when creating new windows/borders
    #[serde(alias = "restore_delay")]
    pub unminimize_delay: Option<u64>, // Adjust delay when restoring minimized windows
}

#[derive(Clone, Debug, Default, Deserialize)]
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

#[derive(Clone, Debug, Default, Deserialize)]
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
    pub enabled: Option<bool>,
    pub animations: Option<Animations>,
    #[serde(alias = "init_delay")]
    pub initialize_delay: Option<u64>,
    #[serde(alias = "restore_delay")]
    pub unminimize_delay: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MatchKind {
    Title,
    Class,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MatchStrategy {
    Equals,
    Contains,
    Regex,
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

        let config = serde_yaml::from_str(&contents)?;
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

    pub fn spawn_config_listener() -> anyhow::Result<()> {
        let config_dir: Vec<u16> = Self::get_config_dir()?
            .into_os_string()
            .encode_wide()
            .chain(iter::once(0))
            .collect();

        let dir_handle = unsafe {
            CreateFileW(
                PCWSTR(config_dir.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                HANDLE::default(),
            )
            .context("could not create dir handle for config monitoring")?
        };

        // Convert HANDLE to isize so we can pass it into the thread
        let dir_handle_isize = dir_handle.0 as isize;
        CONFIG_DIR_HANDLE.replace(Some(dir_handle_isize));

        let _ = thread::spawn(move || unsafe {
            // Reconvert isize back to HANDLE
            let dir_handle = HANDLE(dir_handle_isize as _);

            let mut buffer = [0u8; 256];
            let mut bytes_returned = 0u32;

            let mut now = time::Instant::now();
            let delay = time::Duration::from_secs(1);

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
                    return;
                }

                // Prevent too many directory checks in quick succession
                if now.elapsed() < delay {
                    thread::sleep(delay - now.elapsed());
                }

                Self::process_dir_change_notifs(&buffer, bytes_returned);
                now = time::Instant::now();
            }
        });

        Ok(())
    }

    fn process_dir_change_notifs(buffer: &[u8; 256], bytes_returned: u32) {
        let mut offset = 0usize;

        while offset < bytes_returned as usize {
            let info = unsafe { &*(buffer.as_ptr().add(offset) as *const FILE_NOTIFY_INFORMATION) };

            // We divide FileNameLength by 2 because it's in bytes (u8), but FileName is the start of a u16 slice
            let name_slice = unsafe {
                slice::from_raw_parts(info.FileName.as_ptr(), info.FileNameLength as usize / 2)
            };
            let file_name = String::from_utf16_lossy(name_slice);
            debug!("file changed: {}", file_name);

            if file_name == "config.yaml" {
                Config::reload_config();
                reload_borders();

                // Break to prevent multiple reloads from the same notification
                break;
            }

            // If NextEntryOffset = 0, then we have reached the end of the notification
            if info.NextEntryOffset == 0 {
                break;
            } else {
                offset += info.NextEntryOffset as usize
            }
        }
    }

    pub fn destroy_config_listener() -> anyhow::Result<()> {
        if let Some(dir_handle_isize) = CONFIG_DIR_HANDLE.get() {
            let dir_handle = HANDLE(dir_handle_isize as _);

            // Cancel all pending I/O operations on the handle
            unsafe { CancelIoEx(dir_handle, None) }.log_if_err();

            // Close the handle for cleanup
            return unsafe { CloseHandle(dir_handle) }.map_err(anyhow::Error::new);
        }

        info!("config_dir_handle not found; skipping cleanup");
        Ok(())
    }
}

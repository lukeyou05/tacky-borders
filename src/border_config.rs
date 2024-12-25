use crate::animations::Animations;
use crate::colors::ColorConfig;
use crate::utils::{get_adjusted_radius, get_window_corner_preference};
use anyhow::{anyhow, Context};
use dirs::home_dir;
use serde::{Deserialize, Serialize};
use std::fs::{self, DirBuilder};
use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWMWCP_DEFAULT, DWMWCP_DONOTROUND, DWMWCP_ROUND, DWMWCP_ROUNDSMALL,
};

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
}

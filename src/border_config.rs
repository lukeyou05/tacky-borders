use crate::animations::Animations;
use crate::colors::ColorConfig;
use anyhow::{anyhow, Context};
use dirs::home_dir;
use serde::{Deserialize, Serialize};
use std::fs::{self, DirBuilder};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

pub static CONFIG: LazyLock<Mutex<Config>> = LazyLock::new(|| {
    Mutex::new(match Config::create_config() {
        Ok(config) => config,
        Err(e) => {
            error!("could not read config.yaml: {e:#}");
            Config::default()
        }
    })
});
const DEFAULT_CONFIG: &str = include_str!("resources/config.yaml");

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    pub global: Global,
    pub window_rules: Vec<WindowRule>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Global {
    pub border_width: f32,
    pub border_offset: i32,
    pub border_radius: f32,
    pub active_color: ColorConfig,
    pub inactive_color: ColorConfig,
    pub animations: Option<Animations>,
    #[serde(alias = "init_delay")]
    pub initialize_delay: Option<u64>, // Adjust delay when creating new windows/borders
    #[serde(alias = "restore_delay")]
    pub unminimize_delay: Option<u64>, // Adjust delay when restoring minimized windows
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WindowRule {
    #[serde(rename = "match")]
    pub kind: Option<MatchKind>,
    pub name: Option<String>,
    pub strategy: Option<MatchStrategy>,
    pub border_width: Option<f32>,
    pub border_offset: Option<i32>,
    pub border_radius: Option<f32>,
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
        *CONFIG.lock().unwrap() = new_config;
    }
}

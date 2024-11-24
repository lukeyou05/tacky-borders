use crate::animations::Animations;
use crate::colors::ColorConfig;
use dirs::home_dir;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::fs::DirBuilder;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

pub static CONFIG: LazyLock<Mutex<Config>> = LazyLock::new(|| Mutex::new(Config::create_config()));
pub const DEFAULT_CONFIG: &str = include_str!("resources/config.yaml");

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub global: Global,
    pub window_rules: Vec<WindowRule>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Global {
    pub border_width: f32,
    pub border_offset: i32,
    pub border_radius: f32,
    pub active_color: ColorConfig,
    pub inactive_color: ColorConfig,
    pub animations: Option<Animations>,
    // TODO maybe need better names for these two below
    pub initialize_delay: Option<u64>, // Adjust delay when creating new windows/borders
    pub unminimize_delay: Option<u64>, // Adjust delay when restoring minimized windows
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WindowRule {
    #[serde(rename = "match")]
    pub kind: Option<MatchKind>,
    #[serde(rename = "name")]
    pub pattern: Option<String>,
    pub strategy: Option<MatchStrategy>,
    pub border_width: Option<f32>,
    pub border_offset: Option<i32>,
    pub border_radius: Option<f32>,
    pub active_color: Option<ColorConfig>,
    pub inactive_color: Option<ColorConfig>,
    pub enabled: Option<bool>,
    pub animations: Option<Animations>,
    pub initialize_delay: Option<u64>,
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
    pub fn create_config() -> Self {
        let config_dir = Self::get_config_location();
        let config_path = config_dir.join("config.yaml");

        // If .config/tacky-borders/config.yaml does not exist, create it
        if !fs::exists(&config_path).expect("couldn't check if config path exists") {
            let default_contents = DEFAULT_CONFIG.as_bytes();
            std::fs::write(&config_path, default_contents)
                .expect("could not generate default config.yaml");

            info!(r"generating default config in {}", config_dir.display());
        }

        let contents = match fs::read_to_string(&config_path) {
            Ok(contents) => contents,
            Err(_) => panic!("could not read config.yaml in: {}", config_path.display()),
        };

        let config: Config = serde_yaml::from_str(&contents).expect("error reading config.yaml");
        config
    }

    pub fn get_config_location() -> PathBuf {
        let home_dir = home_dir().expect("can't find home path");
        let config_dir = home_dir.join(".config").join("tacky-borders");
        if !config_dir.exists() {
            DirBuilder::new()
                .recursive(true)
                .create(&config_dir)
                .expect("could not create config directory!");
        }
        config_dir
    }

    pub fn reload_config() {
        let mut config = CONFIG.lock().unwrap();
        *config = Self::create_config();
        drop(config);
    }
}

use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::sync::{LazyLock, Mutex};

pub static CONFIG: LazyLock<Mutex<Config>> = LazyLock::new(|| Mutex::new(Config::create_config()));

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub border_size: i32,
    pub border_offset: i32,
    pub border_radius: f32,
    pub active_color: String,
    pub inactive_color: String
}

impl Config {
    pub fn create_config() -> Self {
        let contents = match fs::read_to_string("src/resources/config.yaml") {
            Ok(contents) => contents,
            _ => panic!("could not read config.yaml!"),
        }; 
        let config: Config = serde_yaml::from_str(&contents).unwrap();
        return config;
    }

    pub fn reload_config() {
        let mut config = CONFIG.lock().unwrap();
        *config = Self::create_config();
        drop(config);
    }
}

use serde::Deserialize;
use serde::Serialize;
use dirs::home_dir;
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
        let home_dir = home_dir().expect("can't find home path");
        let config_path = home_dir.join(".config").join("tacky-borders").join("config.yaml");

        if !fs::exists(&config_path).expect("couldn't check if config path exists") {
            // TODO automatically generate a config file
            println!("Using default config. To adjust options, create a config.yaml file in C:/Users/<username>/.config/tacky-borders");
            return Config::default();
        }

        let contents = match fs::read_to_string(&config_path) {
            Ok(contents) => contents,
            Err(err) => panic!("could not read config.yaml in: {}", config_path.display()),
        }; 

        let config: Config = serde_yaml::from_str(&contents).expect("error reading config.yaml");
        return config;
    }

    pub fn reload_config() {
        let mut config = CONFIG.lock().unwrap();
        *config = Self::create_config();
        drop(config);
    }

    pub fn default() -> Config {
        return Config {
            border_size: 4,
            border_offset: -1,
            border_radius: -1.0,
            active_color: String::from("accent"),
            inactive_color: String::from("accent")
        }
    }
}

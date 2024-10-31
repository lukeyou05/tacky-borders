use serde::Deserialize;
use serde::Serialize;
use dirs::home_dir;
use std::fs;
use std::fs::DirBuilder;
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
        let config_dir = home_dir.join(".config").join("tacky-borders"); 
        let config_path = config_dir.join("config.yaml");

        // If .config/tacky-borders/config.yaml does not exist, create it
        if !fs::exists(&config_path).expect("couldn't check if config path exists") {
            let default = Self::default();
            let default_contents = serde_yaml::to_string(&default).expect("could not generate default config.yaml");

            // If .config/tacky-borders does not exist either, then create the directory too
            if !fs::exists(&config_dir).expect("couldn't check if config directory exists") {
                DirBuilder::new()
                    .recursive(true)
                    .create(&config_dir).expect("could not create config directory!");
            }

            let _ = std::fs::write(&config_path, &default_contents).expect("could not generate default config.yaml");

            println!("generating default config in C:/Users/<username>/.config/tacky-borders");
            return default;
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

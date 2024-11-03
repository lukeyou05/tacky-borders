use serde::Deserialize;
use serde::Serialize;
use dirs::home_dir;
use std::fs;
use std::fs::DirBuilder;
use std::sync::{LazyLock, Mutex};

pub static CONFIG: LazyLock<Mutex<Config>> = LazyLock::new(|| Mutex::new(Config::create_config()));
pub const DEFAULT_CONFIG: &str = include_str!("resources/config.yaml");

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub border_size: i32,
    pub border_offset: i32,
    pub border_radius: f32,
    pub active_color: String,
    pub inactive_color: String,
    pub window_rules: Vec<WindowRule>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WindowRule {
  #[serde(rename = "match")]
  pub rule_match: Kind,
  pub contains: Option<String>,
  pub border_size: Option<i32>,
  pub border_offset: Option<i32>,
  pub border_radius: Option<f32>,
  pub active_color: Option<String>,
  pub inactive_color: Option<String>,
  pub enabled: Option<bool>
}


#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum Kind {
  Title,
  Class,
}

impl Config {
    pub fn create_config() -> Self {
        let home_dir = home_dir().expect("can't find home path");
        let config_dir = home_dir.join(".config").join("tacky-borders"); 
        let config_path = config_dir.join("config.yaml");

        // If .config/tacky-borders/config.yaml does not exist, create it
        if !fs::exists(&config_path).expect("couldn't check if config path exists") {
            let default_contents = DEFAULT_CONFIG.as_bytes();

            // If .config/tacky-borders does not exist either, then create the directory too
            if !fs::exists(&config_dir).expect("couldn't check if config directory exists") {
                DirBuilder::new()
                    .recursive(true)
                    .create(&config_dir).expect("could not create config directory!");
            }

            let _ = std::fs::write(&config_path, &default_contents).expect("could not generate default config.yaml");

            println!(r"generating default config in {}\.config\tacky-borders", home_dir.display());
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
}

impl WindowRule {
    pub fn default() -> Self {
        return WindowRule {
            // rule_match for default() is completely arbitrary. I could replace it with an Option
            // in the future and then I could stop manually implementing default() altogether.
            rule_match: Kind::Title,
            contains: None,
            border_size: None,
            border_offset: None,
            border_radius: None,
            active_color: None,
            inactive_color: None,
            enabled: None 
        };
    }
}

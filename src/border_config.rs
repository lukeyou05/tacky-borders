use serde::Deserialize;
use serde::Serialize;
use std::fs;
use dirs::home_dir;

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub border_size: i32,
    pub border_offset: i32,
    pub border_radius: f32,
    pub active_color: u32,
    pub inactive_color: u32,
}

pub fn create_config() -> Config {
    let home_dir = home_dir().expect("can't find home path");
    let config_path = home_dir.join(".config").join("tacky-borders").join("config.yaml");

    let contents = match fs::read_to_string(&config_path) {
        Ok(contents) => contents,
        _ => panic!("could not read config.yaml in:{}", config_path.display()),
    }; 
    let config: Config = serde_yaml::from_str(&contents).unwrap();
    return config;
}

use serde::Deserialize;
use serde::Serialize;
use std::fs;

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub border_size: i32,
    pub border_offset: i32,
    pub border_radius: f32,
}

pub fn create_config() -> Config {
    let contents = match fs::read_to_string("src/resources/config.yaml") {
        Ok(contents) => contents,
        _ => panic!("could not read config.yaml!"),
    }; 
    let config: Config = serde_yaml::from_str(&contents).unwrap();
    return config;
}

#![allow(dead_code)]
use serde::Deserialize;
use std::sync::Arc;
use std::{fs, panic};

lazy_static::lazy_static! {
    pub static ref APP_CONFIG: Arc<AppConfig> = {
        let config_file = "config.toml";
        let toml_str = fs::read_to_string(config_file)
            .expect(&format!("Failed to read configuration file: {}", config_file));
        let config = match toml::from_str(&toml_str) {
            Ok(t) => t,
            Err(e) => panic!("TOML parse fail {e:?}"),
        };
        Arc::new(config)
    };
}

#[derive(Debug, Deserialize, Clone)]
pub struct MqttConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub client_id: String,
    pub username: String,
    pub password: String,
    pub interval: u32,
    pub topic: String,
    pub sub: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MeterConfig {
    pub address: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub mqtt: MqttConfig,
    pub meter: MeterConfig,
}

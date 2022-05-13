use anyhow::Result;
use serde::Deserialize;
use std::{fs, path::Path, time::Duration};

#[derive(Debug, Deserialize)]
pub(crate) struct Config {
    pub broker: BrokerConfig,
    pub topics: TopicsConfig,
    pub serial: SerialConfig,
}

impl Config {
    pub(crate) fn load(path: &Path) -> Result<Self> {
        Ok(toml::from_str(&fs::read_to_string(path)?)?)
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct BrokerConfig {
    pub address: String,
    pub client_id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TopicsConfig {
    pub transmit: String,
    pub receive: String,
    pub receive_control: String,
    pub availability: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SerialConfig {
    pub device: String,
    pub baud: u32,
    #[serde(with = "humantime_serde")]
    pub timeout: Duration,
}

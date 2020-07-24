use serde::Deserialize;

use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub user: User,
}

#[derive(Debug, Deserialize)]
pub struct User {
    /// Name to be shown in the application
    pub name: String,
    /// Phone number used in Signal
    pub phone_number: String,
}

pub fn load_from(path: impl AsRef<Path>) -> anyhow::Result<Config> {
    let content = std::fs::read_to_string(path)?;
    let config = toml::de::from_str(&content)?;
    Ok(config)
}

pub fn installed_config() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".gurk.toml"))
}

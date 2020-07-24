use serde::Deserialize;

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub user: User,
    #[serde(default)]
    pub signal_cli: SignalCli,
    #[serde(default = "default_data_path")]
    pub data_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    /// Name to be shown in the application
    pub name: String,
    /// Phone number used in Signal
    pub phone_number: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SignalCli {
    /// Path to the signal-cli executable.
    pub path: PathBuf,
}

impl Default for SignalCli {
    fn default() -> Self {
        Self {
            path: PathBuf::from("signal-cli"),
        }
    }
}

pub fn load_from(path: impl AsRef<Path>) -> anyhow::Result<Config> {
    let content = std::fs::read_to_string(path)?;
    let config = toml::de::from_str(&content)?;
    Ok(config)
}

pub fn installed_config() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".gurk.toml"))
}

fn default_data_path() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".gurk.data.json"))
        .expect("could not find home directory")
}

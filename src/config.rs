use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub user: User,
    #[serde(default)]
    pub signal_cli: SignalCli,
    #[serde(default = "default_data_path")]
    pub data_path: PathBuf,
    /// Whether only to show the first name of a contact
    #[serde(default)]
    pub first_name_only: bool,
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
    let config_dir = dirs::config_dir()?.join("gurk");
    fs::create_dir_all(&config_dir)
        .unwrap_or_else(|_| panic!("{:?} did not exist and could not be created", &config_dir));
    Some(config_dir.join("gurk.toml"))
}

fn default_data_path() -> PathBuf {
    let data_dir = match dirs::data_dir() {
        Some(dir) => dir.join("gurk"),
        None => panic!("default data directory not found, $XDG_DATA_HOME and $HOME are unset"),
    };
    fs::create_dir_all(&data_dir)
        .unwrap_or_else(|_| panic!("{:?} did not exist and could not be created", &data_dir));
    data_dir.join("gurk.data.json")
}

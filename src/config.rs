use anyhow::{anyhow, bail};
use serde::{Deserialize, Serialize};

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_data_path")]
    pub data_path: PathBuf,
    #[serde(default = "default_signal_db_path")]
    pub signal_db_path: PathBuf,
    /// Whether only to show the first name of a contact
    #[serde(default)]
    pub first_name_only: bool,
    pub user: User,
}

impl Config {
    /// Create new config with default paths from the given user.
    pub fn with_user(user: User) -> Self {
        Config {
            user,
            data_path: default_data_path(),
            signal_db_path: default_signal_db_path(),
            first_name_only: false,
        }
    }

    pub fn save_new(&self) -> anyhow::Result<()> {
        let config_dir =
            dirs::config_dir().ok_or_else(|| anyhow!("could not find default config directory"))?;
        let config_file = config_dir.join("gurk/gurk.toml");
        if config_file.exists() {
            bail!(
                "will not override config file at: {}",
                config_file.display()
            );
        }
        println!("{:?}", self);
        let content = toml::ser::to_string(self)?;
        std::fs::write(config_file, &content)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Name to be shown in the application
    pub name: String,
    /// Phone number used in Signal
    pub phone_number: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Get the location of the first found default config file paths
/// according to the following order:
///
/// 1. $XDG_CONFIG_HOME/gurk/gurk.toml
/// 2. $XDG_CONFIG_HOME/gurk.yml
/// 3. $HOME/.config/gurk/gurk.toml
/// 4. $HOME/.gurk.toml
pub fn installed_config() -> Option<PathBuf> {
    // case 1, and 3 as fallback (note: case 2 is not possible if 1 is not possible)
    let config_dir = dirs::config_dir()?;
    let config_file = config_dir.join("gurk/gurk.toml");
    if config_file.exists() {
        return Some(config_file);
    }

    // case 2
    let config_file = config_dir.join("gurk.toml");
    if config_file.exists() {
        return Some(config_file);
    }

    // case 4
    let home_dir = dirs::home_dir()?;
    let config_file = home_dir.join(".gurk.toml");
    if config_file.exists() {
        return Some(config_file);
    }

    None
}

pub fn default_data_dir() -> PathBuf {
    let data_dir = match dirs::data_dir() {
        Some(dir) => dir.join("gurk"),
        None => panic!("default data directory not found, $XDG_DATA_HOME and $HOME are unset"),
    };
    fs::create_dir_all(&data_dir)
        .unwrap_or_else(|_| panic!("{:?} did not exist and could not be created", &data_dir));
    data_dir
}

fn default_data_path() -> PathBuf {
    default_data_dir().join("gurk.data.json")
}

pub fn default_signal_db_path() -> PathBuf {
    default_data_dir().join("signal-db")
}

/// Fallback to legacy data path location
pub fn fallback_data_path() -> Option<PathBuf> {
    dirs::home_dir().map(|p| p.join(".gurk.data.json"))
}

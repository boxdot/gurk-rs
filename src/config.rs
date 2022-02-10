use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Path to the JSON file (incl. filename) storing channels and messages.
    #[serde(default = "default_data_path")]
    pub data_path: PathBuf,
    /// Path to the Signal database containing the linked device data.
    #[serde(default = "default_signal_db_path")]
    pub signal_db_path: PathBuf,
    /// Whether only to show the first name of a contact
    #[serde(default)]
    pub first_name_only: bool,
    /// Whether to show receipts (sent, delivered, read) information next to your user name in UI
    #[serde(default = "default_true")]
    pub show_receipts: bool,
    /// User configuration
    pub user: User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// Name to be shown in the application
    pub name: String,
    /// Phone number used in Signal
    pub phone_number: String,
}

impl Config {
    /// Create new config with default paths from the given user.
    pub fn with_user(user: User) -> Self {
        Config {
            user,
            data_path: default_data_path(),
            signal_db_path: default_signal_db_path(),
            first_name_only: false,
            show_receipts: true,
        }
    }

    /// Tries to load configuration from one of the default locations:
    ///
    /// 1. $XDG_CONFIG_HOME/gurk/gurk.toml
    /// 2. $XDG_CONFIG_HOME/gurk.toml
    /// 3. $HOME/.config/gurk/gurk.toml
    /// 4. $HOME/.gurk.toml
    ///
    /// If no config is found returns `None`.
    pub fn load_installed() -> anyhow::Result<Option<Self>> {
        installed_config().map(Self::load).transpose()
    }

    /// Saves a new config file in case it does not exist.
    ///
    /// Also makes sure that the `config.data_path` exists.
    pub fn save_new(&self) -> anyhow::Result<()> {
        let config_dir =
            dirs::config_dir().ok_or_else(|| anyhow!("could not find default config directory"))?;
        let config_file = config_dir.join("gurk/gurk.toml");
        self.save_new_at(config_file)
    }

    fn save_new_at(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        // check that config won't be overridden
        if path.as_ref().exists() {
            bail!(
                "will not override config file at: {}",
                path.as_ref().display()
            );
        }

        // make sure data_path exists
        let data_path = self
            .data_path
            .parent()
            .ok_or_else(|| anyhow!("invalid data path: no parent dir"))?;
        fs::create_dir_all(data_path).context("could not create data dir")?;

        self.save(path)
    }

    fn load(path: impl AsRef<Path>) -> anyhow::Result<Config> {
        let content = std::fs::read_to_string(path)?;
        let config = toml::de::from_str(&content)?;
        Ok(config)
    }

    fn save(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path.as_ref();
        let content = toml::ser::to_string(self)?;
        let parent_dir = path
            .parent()
            .ok_or_else(|| anyhow!("invalid config path {}: no parent dir", path.display()))?;
        fs::create_dir_all(parent_dir).unwrap();
        fs::write(path, &content)?;
        Ok(())
    }
}

/// Get the location of the first found default config file paths
/// according to the following order:
///
/// 1. $XDG_CONFIG_HOME/gurk/gurk.toml
/// 2. $XDG_CONFIG_HOME/gurk.yml
/// 3. $HOME/.config/gurk/gurk.toml
/// 4. $HOME/.gurk.toml
fn installed_config() -> Option<PathBuf> {
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

/// Path to store the signal database containing the data for the linked device.
pub fn default_signal_db_path() -> PathBuf {
    default_data_dir().join("signal-db")
}

/// Fallback to legacy data path location
pub fn fallback_data_path() -> Option<PathBuf> {
    dirs::home_dir().map(|p| p.join(".gurk.data.json"))
}

fn default_data_dir() -> PathBuf {
    match dirs::data_dir() {
        Some(dir) => dir.join("gurk"),
        None => panic!("default data directory not found, $XDG_DATA_HOME and $HOME are unset"),
    }
}

fn default_data_path() -> PathBuf {
    default_data_dir().join("gurk.data.json")
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{tempdir, NamedTempFile, TempDir};

    fn example_user() -> User {
        User {
            name: "Tyler Durden".to_string(),
            phone_number: "+0000000000".to_string(),
        }
    }

    fn example_config_with_random_paths(dir: &TempDir) -> Config {
        let data_path = dir.path().join("some-data-dir/some-other-dir/data.json");
        assert!(!data_path.parent().unwrap().exists());
        let signal_db_path = dir.path().join("some-signal-db-dir/some-other-dir");
        assert!(!signal_db_path.exists());

        Config {
            data_path,
            signal_db_path,
            ..Config::with_user(example_user())
        }
    }

    #[test]
    fn test_save_new_at_non_existent() -> anyhow::Result<()> {
        let dir = tempdir()?;

        let config = example_config_with_random_paths(&dir);
        let config_path = dir.path().join("some-dir/some-other-dir/gurk.toml");

        config.save_new_at(&config_path)?;
        let loaded_config = Config::load(config_path)?;
        assert_eq!(config, loaded_config);

        assert!(config.data_path.parent().unwrap().exists()); // data path parent is created
        assert!(!config.signal_db_path.exists()); // signal path is not touched

        Ok(())
    }

    #[test]
    fn test_save_new_fails_or_existent() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let config = example_config_with_random_paths(&dir);
        let file = NamedTempFile::new()?;

        assert!(config.save_new_at(file.path()).is_err());
        assert!(!config.data_path.parent().unwrap().exists()); // data path parent is not touched
        assert!(!config.signal_db_path.exists()); // signal path is not touched

        Ok(())
    }
}

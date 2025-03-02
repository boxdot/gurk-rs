use anyhow::{Context, anyhow, bail};
use serde::{Deserialize, Serialize};
use url::Url;

use std::fs;
use std::path::{Path, PathBuf};

use crate::command::ModeKeybindingConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Path to the JSON file (incl. filename) storing channels and messages.
    #[serde(default = "default_data_json_path", rename = "data_path")]
    pub deprecated_data_path: PathBuf,
    /// Path to the Signal database containing the linked device data.
    #[serde(default = "default_signal_db_path")]
    pub signal_db_path: PathBuf,
    /// Whether only to show the first name of a contact
    #[serde(default)]
    pub first_name_only: bool,
    /// Whether to show receipts (sent, delivered, read) information next to your user name in UI
    #[serde(default = "default_true")]
    pub show_receipts: bool,
    /// Whether to show system notifications on incoming messages
    #[serde(default = "default_true")]
    pub notifications: bool,
    #[serde(default = "default_true")]
    pub bell: bool,
    /// User configuration
    pub user: User,
    #[cfg(feature = "dev")]
    #[serde(default, skip_serializing_if = "DeveloperConfig::is_default")]
    pub developer: DeveloperConfig,
    #[serde(default)]
    pub sqlite: SqliteConfig,
    #[serde(default)]
    /// If set, enables encryption of the key store and messages database
    pub passphrase: Option<String>,
    /// If set, the full message text will be colored, not only the author name
    #[serde(default)]
    pub colored_messages: bool,
    #[serde(default)]
    /// Keymaps
    pub keybindings: ModeKeybindingConfig,
    /// Whether to enable the default keybindings
    #[serde(default = "default_true")]
    pub default_keybindings: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// Name to be shown in the application
    pub name: String,
    /// Phone number used in Signal
    pub phone_number: String,
}

#[cfg(feature = "dev")]
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeveloperConfig {
    /// Dump raw messages to `messages.json` for collecting debug/benchmark data
    pub dump_raw_messages: bool,
}

#[cfg(feature = "dev")]
impl DeveloperConfig {
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LoadedConfig {
    pub(crate) config: Config,
    pub(crate) deprecated_keys: DeprecatedKeys,
}

#[derive(Debug, Clone)]
pub(crate) struct DeprecatedKeys {
    pub(crate) file_path: PathBuf,
    pub(crate) keys: Vec<DeprecatedConfigKey>,
}

#[derive(Debug, Clone)]
pub(crate) struct DeprecatedConfigKey {
    pub(crate) key: &'static str,
    pub(crate) message: &'static str,
}

impl Config {
    /// Create new config with default paths from the given user.
    pub fn with_user(user: User) -> Self {
        Config {
            user,
            deprecated_data_path: default_data_json_path(),
            signal_db_path: default_signal_db_path(),
            first_name_only: false,
            show_receipts: true,
            notifications: true,
            bell: true,
            #[cfg(feature = "dev")]
            developer: Default::default(),
            sqlite: Default::default(),
            passphrase: None,
            colored_messages: false,
            default_keybindings: true,
            keybindings: ModeKeybindingConfig::default(),
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
    pub(crate) fn load_installed() -> anyhow::Result<Option<LoadedConfig>> {
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
        let data_path = data_dir();
        fs::create_dir_all(data_path).context("could not create data dir")?;

        self.save(path)
    }

    fn load(path: impl AsRef<Path>) -> anyhow::Result<LoadedConfig> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)?;
        let config = toml::de::from_str(&content)?;

        // check for deprecated keys
        let config_value: toml::Value = toml::de::from_str(&content)?;
        let mut keys = Vec::new();
        if config_value
            .get("sqlite")
            .map(|v| v.get("enabled").is_some())
            .unwrap_or(false)
        {
            keys.push(DeprecatedConfigKey {
                key: "sqlite.enabled",
                message: "sqlite is now enabled by default",
            });
        }
        if config_value.get("data_path").is_some() {
            keys.push(DeprecatedConfigKey {
                key: "data_path",
                message: "is not used anymore, and is migrated to sqlite.url",
            });
        }
        let deprecated_keys = DeprecatedKeys {
            file_path: path.to_path_buf(),
            keys,
        };

        Ok(LoadedConfig {
            config,
            deprecated_keys,
        })
    }

    fn save(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path.as_ref();
        let content = toml::ser::to_string(self)?;
        let parent_dir = path
            .parent()
            .ok_or_else(|| anyhow!("invalid config path {}: no parent dir", path.display()))?;
        fs::create_dir_all(parent_dir).unwrap();
        fs::write(path, content)?;
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct SqliteConfig {
    #[serde(default = "SqliteConfig::default_db_url")]
    pub url: Url,
    /// Don't delete the unencrypted db, after applying encryption to it
    ///
    /// Useful for testing.
    #[serde(default, rename = "_preserve_unencryped")]
    pub preserve_unencrypted: bool,
}

impl Default for SqliteConfig {
    fn default() -> Self {
        Self {
            url: Self::default_db_url(),
            preserve_unencrypted: false,
        }
    }
}

impl SqliteConfig {
    fn default_db_url() -> Url {
        let path = data_dir().join("gurk.sqlite");
        format!("sqlite://{}", path.display())
            .parse()
            .expect("invalid default sqlite path")
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
    data_dir().join("signal-db")
}

/// Fallback to legacy data path location
pub fn fallback_data_path() -> Option<PathBuf> {
    dirs::home_dir().map(|p| p.join(".gurk.data.json"))
}

pub(crate) fn data_dir() -> PathBuf {
    let data_dir =
        dirs::data_dir().expect("data directory not found, $XDG_DATA_HOME and $HOME are unset?");
    data_dir.join("gurk")
}

fn default_data_json_path() -> PathBuf {
    data_dir().join("gurk.data.json")
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{NamedTempFile, TempDir, tempdir};

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
            deprecated_data_path: data_path,
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
        let LoadedConfig {
            config: loaded_config,
            deprecated_keys: _,
        } = Config::load(&config_path)?;
        assert_eq!(config, loaded_config);

        assert!(config_path.parent().unwrap().exists()); // data path parent is created
        assert!(!config.signal_db_path.exists()); // signal path is not touched

        Ok(())
    }

    #[test]
    fn test_save_new_fails_or_existent() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let config = example_config_with_random_paths(&dir);
        let file = NamedTempFile::new()?;

        assert!(config.save_new_at(file.path()).is_err());
        assert!(!config.deprecated_data_path.parent().unwrap().exists()); // data path parent is not touched
        assert!(!config.signal_db_path.exists()); // signal path is not touched

        Ok(())
    }
}

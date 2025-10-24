use anyhow::{Context, anyhow, bail};
use serde::{Deserialize, Serialize};
use tracing::warn;
use url::Url;

use std::fs;
use std::path::{Path, PathBuf};

use crate::{command::ModeKeybindingConfig, passphrase::Passphrase};

const GURK_DB_NAME: &str = "gurk.sqlite";
const SIGNAL_DB_NAME: &str = "signal.sqlite";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Directory to store messages and signal database, and attachments.
    #[serde(
        default = "default_data_dir",
        skip_serializing_if = "is_default_data_dir"
    )]
    pub data_dir: PathBuf,
    /// Path to the JSON file (incl. filename) storing channels and messages.
    #[serde(
        default = "default_data_json_path",
        rename = "data_path",
        skip_serializing
    )]
    pub deprecated_data_path: PathBuf,
    /// Path to the Signal database containing the linked device data.
    #[serde(
        rename = "signal_db_path",
        default = "default_signal_db_path",
        skip_serializing_if = "is_default_signal_db_path"
    )]
    pub deprecated_signal_db_path: PathBuf,
    /// Whether only to show the first name of a contact
    #[serde(default)]
    pub first_name_only: bool,
    /// Whether to show receipts (sent, delivered, read) information next to your user name in UI
    #[serde(default = "default_true")]
    pub show_receipts: bool,
    /// Notification settings
    #[serde(default)]
    pub notifications: NotificationConfig,
    #[serde(default = "default_true")]
    pub bell: bool,
    /// User configuration
    pub user: User,
    #[cfg(feature = "dev")]
    #[serde(default, skip_serializing_if = "DeveloperConfig::is_default")]
    pub developer: DeveloperConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sqlite: Option<SqliteConfig>,
    #[serde(default)]
    /// If set, enables encryption of the key store and messages database
    pub passphrase: Option<Passphrase>,
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
    #[serde(alias = "name")]
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationConfig {
    /// Whether to show system notifications on incoming messages
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Whether to show message preview in notifications
    #[serde(default = "default_true")]
    pub show_message_text: bool,
    /// Whether to show message origin in notifications
    #[serde(default = "default_true")]
    pub show_message_chat: bool,
    /// Whether to show reactions in notifications
    #[serde(default = "default_true")]
    pub show_reactions: bool,
    /// Whether to mute reactions bell
    #[serde(default)]
    pub mute_reactions_bell: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_message_text: true,
            show_message_chat: true,
            show_reactions: true,
            mute_reactions_bell: false,
        }
    }
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
pub struct LoadedConfig {
    pub(crate) config: Config,
    pub(crate) deprecated_keys: DeprecatedKeys,
}

impl LoadedConfig {
    pub fn report_deprecated_keys(self) -> Config {
        if !self.deprecated_keys.keys.is_empty() {
            println!("In '{}':", self.deprecated_keys.file_path.display());
            for DeprecatedConfigKey { key, message } in self.deprecated_keys.keys.iter() {
                warn!(key, message, "deprecated config key");
                println!("deprecated config key: {key}, {message}");
            }
        }
        self.config
    }
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
            data_dir: default_data_dir(),
            deprecated_data_path: default_data_json_path(),
            deprecated_signal_db_path: default_signal_db_path(),
            first_name_only: false,
            show_receipts: true,
            notifications: NotificationConfig::default(),
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
    pub fn load_installed() -> anyhow::Result<Option<LoadedConfig>> {
        installed_config().map(Self::load).transpose()
    }

    pub fn load_installed_passphrase() -> anyhow::Result<Option<Passphrase>> {
        let loaded = Self::load_installed()?;
        Ok(loaded.and_then(|c| c.config.passphrase))
    }

    /// Saves a new config file in case it does not exist.
    ///
    /// Also makes sure that the `config.data_path` exists.
    pub fn save_new(&self) -> anyhow::Result<PathBuf> {
        let config_dir =
            dirs::config_dir().ok_or_else(|| anyhow!("could not find default config directory"))?;
        let config_file = config_dir.join("gurk/gurk.toml");
        self.save_new_at(&config_file)
            .with_context(|| format!("failed to save config at {}", config_file.display()))?;
        Ok(config_file)
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
        let data_path = default_data_dir();
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
        if config_value.get("signal_db_path").is_some() {
            keys.push(DeprecatedConfigKey {
                key: "signal_db_path",
                message: "is not used anymore; use `data_dir` instead",
            });
        }
        if config_value.get("sqlite").is_some() {
            keys.push(DeprecatedConfigKey {
                key: "sqlite",
                message: "will be removed in a future version; use `<data_dir>/gurk.sqlite` instead",
            });
        }
        if config_value
            .get("notifications")
            .and_then(|v| v.as_bool())
            .is_some()
        {
            keys.push(DeprecatedConfigKey {
                key: "notifications",
                message: "boolean format is deprecated; use [notifications] section with enabled, show_sender_name, show_message_preview, and show_reactions fields",
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

    pub fn gurk_db_path(&self) -> PathBuf {
        self.data_dir.join(GURK_DB_NAME)
    }

    pub(crate) fn signal_db_path(&self) -> PathBuf {
        self.data_dir.join(SIGNAL_DB_NAME)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct SqliteConfig {
    #[serde(default = "SqliteConfig::default_db_url")]
    pub url: Url,
    /// Don't delete the unencrypted db, after applying encryption to it
    ///
    /// Useful for testing.
    #[serde(default, rename = "_preserve_unencrypted")]
    pub preserve_unencrypted: bool,
}

impl SqliteConfig {
    fn default_db_url() -> Url {
        let path = default_data_dir().join("gurk.sqlite");
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
fn default_signal_db_path() -> PathBuf {
    default_data_dir().join("signal-db")
}

fn is_default_signal_db_path(path: &Path) -> bool {
    path == default_signal_db_path()
}

/// Fallback to legacy data path location
pub fn fallback_data_path() -> Option<PathBuf> {
    dirs::home_dir().map(|p| p.join(".gurk.data.json"))
}

fn default_data_dir() -> PathBuf {
    let data_dir =
        dirs::data_dir().expect("data directory not found, $XDG_DATA_HOME and $HOME are unset?");
    data_dir.join("gurk")
}

fn is_default_data_dir(path: &Path) -> bool {
    path == default_data_dir()
}

fn default_data_json_path() -> PathBuf {
    default_data_dir().join("gurk.data.json")
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{NamedTempFile, TempDir, tempdir};

    fn example_config_with_random_paths(dir: &TempDir) -> Config {
        let data_dir = dir.path().join("some-data-dir/some-other-dir/data.json");
        assert!(!data_dir.parent().unwrap().exists());

        Config {
            data_dir,
            ..Config::with_user(User {
                display_name: "Tyler Durden".to_string(),
            })
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

        Ok(())
    }

    #[test]
    fn test_save_new_fails_or_existent() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let config = example_config_with_random_paths(&dir);
        let file = NamedTempFile::new()?;

        assert!(config.save_new_at(file.path()).is_err());

        Ok(())
    }
}

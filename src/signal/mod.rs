mod r#impl;
mod manager;
#[cfg(test)]
pub mod test;

use std::path::PathBuf;

use anyhow::{bail, Context as _};
use presage::prelude::SignalServers;

use crate::config::{self, Config};

pub use self::manager::{Attachment, ResolvedGroup, SignalManager};
use self::r#impl::PresageManager;

const GROUP_MASTER_KEY_LEN: usize = 32;
const GROUP_IDENTIFIER_LEN: usize = 32;

pub type GroupMasterKeyBytes = [u8; GROUP_MASTER_KEY_LEN];
pub type GroupIdentifierBytes = [u8; GROUP_IDENTIFIER_LEN];

/// Makes sure that we have a linked device.
///
/// Either,
///
/// 1. links a new device (if no config file found), and writes a new config file with username
///    and phone number, or
/// 2. loads the config file and tries to create the Signal manager from configured Signal database
///    path.
pub async fn ensure_linked_device(
    relink: bool,
) -> anyhow::Result<(Box<dyn SignalManager>, Config)> {
    let config = Config::load_installed()?;
    let db_path = config
        .as_ref()
        .map(|c| c.signal_db_path.clone())
        .unwrap_or_else(config::default_signal_db_path);

    let mut manager = get_signal_manager(db_path)?;

    let is_registered = !relink && manager.is_registered();

    if is_registered {
        if let Some(config) = config {
            return Ok((Box::new(PresageManager::new(manager)), config));
        }
    }

    // link device
    let at_hostname = hostname::get()
        .ok()
        .and_then(|hostname| {
            hostname
                .to_string_lossy()
                .split('.')
                .find(|s| !s.is_empty())
                .map(|s| format!("@{}", s))
        })
        .unwrap_or_default();
    let device_name = format!("gurk{}", at_hostname);
    println!("Linking new device with device name: {}", device_name);
    manager
        .link_secondary_device(SignalServers::Production, device_name.clone())
        .await?;

    // get profile
    let phone_number = manager
        .phone_number()
        .expect("no phone number after device was linked")
        .format()
        .mode(phonenumber::Mode::E164)
        .to_string();
    let profile = manager
        .retrieve_profile()
        .await
        .context("failed to get the user profile")?;
    let name = profile
        .name
        .map(|name| name.given_name)
        .unwrap_or_else(whoami::username);

    let config = if let Some(config) = config {
        // check that config fits the profile
        if config.user.phone_number != phone_number {
            bail!("Wrong phone number in the config. Please adjust it.");
        }
        config
    } else {
        let user = config::User { name, phone_number };
        let config = config::Config::with_user(user);
        config.save_new().context("failed to init config file")?;
        config
    };

    Ok((Box::new(PresageManager::new(manager)), config))
}

/// If `db_path` does not exist, it will be created (including parent directories).
fn get_signal_manager(
    db_path: PathBuf,
) -> anyhow::Result<presage::Manager<presage::SledConfigStore>> {
    let store = presage::SledConfigStore::new(db_path)?;
    let manager = presage::Manager::with_store(store)?;
    Ok(manager)
}

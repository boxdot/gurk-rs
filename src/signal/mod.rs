mod r#impl;
mod manager;
#[cfg(test)]
pub mod test;

use anyhow::{bail, Context as _};
use presage::prelude::SignalServers;
use presage::SledConfigStore;

use crate::config::{self, Config};

pub use self::manager::{Attachment, ResolvedGroup, SignalManager};
use self::r#impl::PresageManager;

// TODO: these should be either re-exported from presage/libsignal-service
const PROFILE_KEY_LEN: usize = 32;
const GROUP_MASTER_KEY_LEN: usize = 32;
const GROUP_IDENTIFIER_LEN: usize = 32;

pub type ProfileKey = [u8; PROFILE_KEY_LEN];
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
    let store = SledConfigStore::new(db_path)?;

    if !relink {
        if let Some(config) = config.clone() {
            if let Ok(manager) = presage::Manager::load_registered(store.clone()) {
                // done loading manager from store
                return Ok((Box::new(PresageManager::new(manager)), config));
            }
        }
    }

    // faulty manager, or not config, or explicit relink
    // => link device
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

    let (tx, rx) = futures_channel::oneshot::channel();
    let (manager, _) = tokio::try_join!(
        async move {
            presage::Manager::link_secondary_device(
                store,
                SignalServers::Production,
                device_name.clone(),
                tx,
            )
            .await
            .map_err(anyhow::Error::from)
        },
        async move {
            match rx.await {
                Ok(url) => qr2term::print_qr(url.to_string()).context("failed to generated qr"),
                Err(e) => bail!("error linking device: {}", e),
            }
        }
    )?;

    // get profile
    let phone_number = manager
        .state()
        .phone_number
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

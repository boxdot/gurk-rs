use crate::{
    app::GroupData,
    config::{self, Config},
};

use anyhow::{bail, Context as _};
use log::error;
use presage::prelude::{GroupMasterKey, SignalServers};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use std::path::PathBuf;

pub const GROUP_MASTER_KEY_LEN: usize = 32;
pub const GROUP_IDENTIFIER_LEN: usize = 32;

pub type GroupMasterKeyBytes = [u8; GROUP_MASTER_KEY_LEN];
pub type GroupIdentifierBytes = [u8; GROUP_IDENTIFIER_LEN];

/// Signal Manager backed by a `sled` store.
pub type Manager = presage::Manager<presage::SledConfigStore>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub content_type: String,
    pub filename: PathBuf,
    pub size: u64,
}

/// If `db_path` does not exist, it will be created (including parent directories).
fn get_signal_manager(db_path: PathBuf) -> anyhow::Result<Manager> {
    let store = presage::SledConfigStore::new(db_path)?;
    let manager = presage::Manager::with_store(store)?;
    Ok(manager)
}

/// Makes sure that we have linked device.
///
/// Either,
///
/// 1. links a new device (if no config file found), and writes a new config file with username
///    and phone number, or
/// 2. loads the config file and tries to create the Signal manager from configured Signal database
///    path.
pub async fn ensure_linked_device(relink: bool) -> anyhow::Result<(Manager, Config)> {
    let config = Config::load_installed()?;
    let db_path = config
        .as_ref()
        .map(|c| c.signal_db_path.clone())
        .unwrap_or_else(config::default_signal_db_path);

    let mut manager = get_signal_manager(db_path)?;

    let is_registered = !relink && manager.is_registered();

    if is_registered {
        if let Some(config) = config {
            return Ok((manager, config));
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
        .unwrap_or_else(String::new);
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

    Ok((manager, config))
}

pub async fn contact_name(manager: &Manager, uuid: Uuid, profile_key: [u8; 32]) -> Option<String> {
    match manager.retrieve_profile_by_uuid(uuid, profile_key).await {
        Ok(profile) => Some(profile.name?.given_name),
        Err(e) => {
            error!("failed to retrieve profile for {}: {}", uuid, e);
            None
        }
    }
}

pub async fn try_resolve_group(
    manager: &mut Manager,
    master_key_bytes: GroupMasterKeyBytes,
) -> anyhow::Result<(String, GroupData, Vec<Vec<u8>>)> {
    let master_key = GroupMasterKey::new(master_key_bytes);
    let decrypted_group = manager.get_group_v2(master_key).await?;

    let mut members = Vec::with_capacity(decrypted_group.members.len());
    let mut member_profile_keys = Vec::with_capacity(decrypted_group.members.len());
    for member in decrypted_group.members {
        let uuid = match Uuid::from_slice(&member.uuid) {
            Ok(id) => id,
            Err(_) => continue,
        };
        members.push(uuid);
        member_profile_keys.push(member.profile_key);
    }

    let title = decrypted_group.title;
    let group_data = GroupData {
        master_key_bytes,
        members,
        revision: decrypted_group.revision,
    };

    Ok((title, group_data, member_profile_keys))
}

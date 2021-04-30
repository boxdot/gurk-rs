use crate::{
    app::GroupData,
    config::{self, Config},
};

use anyhow::{anyhow, bail, Context as _};
use libsignal_service::prelude::GroupMasterKey;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use std::path::PathBuf;

/// Signal Manager backed by a `sled` store.
pub type Manager = presage::Manager<presage::config::SledConfigStore>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub content_type: String,
    pub filename: PathBuf,
    pub size: u64,
}

fn get_signal_manager() -> anyhow::Result<Manager> {
    let data_dir = config::default_data_dir();
    let db_path = data_dir.join("signal-db");
    let config_store = presage::config::SledConfigStore::new(db_path)?;
    let signal_context =
        libsignal_protocol::Context::new(libsignal_protocol::crypto::DefaultCrypto::default())?;
    let manager = presage::Manager::with_config_store(config_store, signal_context)?;
    Ok(manager)
}

pub async fn ensure_linked_device(relink: bool) -> anyhow::Result<(Manager, Config)> {
    let mut manager = get_signal_manager()?;

    let config = config::installed_config()
        .map(config::load_from)
        .transpose()?;

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
        .link_secondary_device(
            libsignal_service::configuration::SignalServers::Production,
            device_name.clone(),
        )
        .await?;

    // get profile
    let phone_number = manager
        .phone_number()
        .expect("no phone number after device was linked");
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
        if config.user.phone_number != phone_number.to_string() {
            bail!("Wrong phone number in the config. Please adjust it.");
        }
        config
    } else {
        let user = config::User {
            name,
            phone_number: phone_number.to_string(),
        };
        let config = config::Config::with_user(user);
        config.save_new().context("failed to init config file")?;
        config
    };

    Ok((manager, config))
}

pub async fn contact_name(manager: &Manager, uuid: Uuid, profile_key: [u8; 32]) -> Option<String> {
    let profile = manager
        .retrieve_profile_by_uuid(uuid, profile_key)
        .await
        .ok()?;
    Some(profile.name?.given_name)
}

pub async fn try_resolve_group(
    manager: &mut Manager,
    master_key: Vec<u8>,
) -> anyhow::Result<(String, GroupData, Vec<Vec<u8>>)> {
    use std::convert::TryInto;

    let master_key = master_key
        .try_into()
        .map_err(|_| anyhow!("invalid master key"))?;
    let decrypted_group = manager
        .get_group_v2(GroupMasterKey::new(master_key))
        .await?;

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
        members,
        revision: decrypted_group.revision,
    };

    Ok((title, group_data, member_profile_keys))
}

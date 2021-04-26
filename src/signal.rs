use crate::config::{self, Config};

use anyhow::Context;
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

pub async fn ensure_linked_device() -> anyhow::Result<(Manager, Config)> {
    let mut manager = get_signal_manager()?;
    let config = if let Some(config_path) = config::installed_config() {
        config::load_from(config_path)?
    } else {
        if manager.phone_number().is_none() {
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
        }

        let phone_number = manager
            .phone_number()
            .expect("no phone number after device was linked");
        let profile = manager.retrieve_profile().await?;
        let name = profile
            .name
            .map(|name| name.given_name)
            .unwrap_or_else(whoami::username);

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

mod attachment;
mod r#impl;
mod manager;
pub mod test;

use std::path::Path;

use anyhow::{Context as _, anyhow, bail};
use futures_channel::oneshot;
use image::Luma;
use presage::{libsignal_service::configuration::SignalServers, model::identity::OnNewIdentity};
use presage_store_sled::{MigrationConflictStrategy, SledStore};
use tokio_util::task::LocalPoolHandle;
use tracing::{error, warn};
use url::Url;

use crate::config::{self, Config, DeprecatedConfigKey, DeprecatedKeys, LoadedConfig};

use self::r#impl::PresageManager;
pub use self::manager::{Attachment, ResolvedGroup, SignalManager};

// TODO: these should be either re-exported from presage/libsignal-service
const PROFILE_KEY_LEN: usize = 32;
const GROUP_MASTER_KEY_LEN: usize = 32;
const GROUP_IDENTIFIER_LEN: usize = 32;

pub type ProfileKeyBytes = [u8; PROFILE_KEY_LEN];
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
    local_pool: LocalPoolHandle,
) -> anyhow::Result<(Box<dyn SignalManager + Send>, Config)> {
    let config = Config::load_installed()?;

    // warn about deprecated keys
    let config = config.map(
        |LoadedConfig {
             config,
             deprecated_keys: DeprecatedKeys { file_path, keys },
         }| {
            if !keys.is_empty() {
                println!("In '{}':", file_path.display());
                for DeprecatedConfigKey { key, message } in keys {
                    warn!(key, message, "deprecated config key");
                    println!("deprecated config key: {key}, {message}");
                }
            }
            config
        },
    );

    let db_path = config
        .as_ref()
        .map(|c| c.signal_db_path.clone())
        .unwrap_or_else(config::default_signal_db_path);
    let passphrase = config
        .as_ref()
        .and_then(|config| config.passphrase.as_ref());
    let store = SledStore::open_with_passphrase(
        db_path,
        passphrase,
        MigrationConflictStrategy::BackupAndDrop,
        OnNewIdentity::Trust,
    )
    .await?;

    if !relink {
        if let Some(config) = config.clone() {
            match presage::Manager::load_registered(store.clone()).await {
                Ok(manager) => {
                    // done loading manager from store
                    return Ok((Box::new(PresageManager::new(manager, local_pool)), config));
                }
                Err(e) => {
                    bail!(
                        "error loading manager. Try again later or run with --relink to force relink: {}",
                        e
                    )
                }
            };
        }
    }

    // faulty manager, or no config, or explicit relink
    // => link device
    let at_hostname = hostname::get()
        .ok()
        .and_then(|hostname| {
            hostname
                .to_string_lossy()
                .split('.')
                .find(|s| !s.is_empty())
                .map(|s| format!("@{s}"))
        })
        .unwrap_or_default();
    let device_name = format!("gurk{at_hostname}");
    println!("Linking new device with device name: {device_name}");

    let (tx, rx) = oneshot::channel();

    let link_task = async move {
        presage::Manager::link_secondary_device(
            store,
            SignalServers::Production,
            device_name.clone(),
            tx,
        )
        .await
        .map_err(anyhow::Error::from)
    };

    let tempdir = tempfile::tempdir().context("failed to create tempdir")?;
    let path = tempdir.path().join("qrcode.png");
    let qrcode_task = gen_qr_code(rx, &path);

    let (mut manager, _) = tokio::try_join!(link_task, qrcode_task)?;
    drop(tempdir); // make sure tempdir is dropped *after* qrcode_task

    // get profile
    let phone_number = manager
        .registration_data()
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

    Ok((Box::new(PresageManager::new(manager, local_pool)), config))
}

async fn gen_qr_code(rx: oneshot::Receiver<Url>, path: &Path) -> anyhow::Result<()> {
    let url = rx
        .await
        .map_err(|e| anyhow!("error linking device {}", e))?;

    if let Err(error) = save_qr_code_png(&url, path) {
        error!(%error, "failed to generate PNG QR code");
    } else {
        println!("QR code saved to {}", path.display());
    }

    qr2term::print_qr(url.to_string()).context("failed to generated qr")?;

    Ok(())
}

fn save_qr_code_png(url: &Url, path: &Path) -> anyhow::Result<()> {
    let image = qrcode::QrCode::new(url.to_string())?
        .render::<Luma<u8>>()
        .build();
    image.save(&path)?;
    Ok(())
}

mod attachment;
mod r#impl;
mod manager;
pub mod test;

use std::path::Path;

use anyhow::{Context as _, anyhow};
use futures_channel::oneshot;
use image::Luma;
use presage::{libsignal_service::configuration::SignalServers, model::identity::OnNewIdentity};
use presage_store_sqlite::SqliteStore;
use tokio_util::task::LocalPoolHandle;
use tracing::{error, info};
use url::Url;

use crate::{config::Config, passphrase::Passphrase};

use self::r#impl::PresageManager;
pub use self::manager::{Attachment, ResolvedGroup, SignalManager};

// TODO: these should be either re-exported from presage/libsignal-service
const PROFILE_KEY_LEN: usize = 32;
const GROUP_MASTER_KEY_LEN: usize = 32;
const GROUP_IDENTIFIER_LEN: usize = 32;

pub type ProfileKeyBytes = [u8; PROFILE_KEY_LEN];
pub type GroupMasterKeyBytes = [u8; GROUP_MASTER_KEY_LEN];
pub type GroupIdentifierBytes = [u8; GROUP_IDENTIFIER_LEN];

/// Links a new device if manager fails with `NotYetRegisteredError` or if `relink` is true.
pub async fn ensure_linked_device(
    relink: bool,
    local_pool: LocalPoolHandle,
    config: &Config,
    passphrase: &Passphrase,
) -> anyhow::Result<Box<dyn SignalManager + Send>> {
    let path = config.signal_db_path();
    info!(path =% path.display(), "opening signal storage");
    let url = Url::from_file_path(&path)
        .map_err(|_| anyhow!("failed to convert path '{}' to file url", path.display()))?;
    let store = SqliteStore::open_with_passphrase(
        url.as_str(),
        Some(passphrase.as_ref()),
        OnNewIdentity::Trust,
    )
    .await
    .with_context(|| format!("failed to open signal storage at: {}", path.display()))?;

    if !relink {
        match presage::Manager::load_registered(store.clone()).await {
            Ok(manager) => {
                // done loading manager from store
                Ok(Box::new(PresageManager::new(
                    manager,
                    config.data_dir.clone(),
                    local_pool,
                )))
            }
            Err(presage::Error::NotYetRegisteredError) => {
                relink_device(local_pool, config, store).await
            }
            Err(error) => Err(error).context(
                "error loading manager. Try again later or run with --relink to force relink",
            ),
        }
    } else {
        relink_device(local_pool, config, store).await
    }
}

async fn relink_device(
    local_pool: LocalPoolHandle,
    config: &Config,
    store: SqliteStore,
) -> anyhow::Result<Box<dyn SignalManager + Send>> {
    // explicit relink => link device
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

    let (manager, _) = tokio::try_join!(link_task, qrcode_task)?;
    drop(tempdir);
    // make sure tempdir is dropped *after* qrcode_task

    Ok(Box::new(PresageManager::new(
        manager,
        config.data_dir.clone(),
        local_pool,
    )))
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

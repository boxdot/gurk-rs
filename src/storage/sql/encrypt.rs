use std::fs::File;
use std::io::Read;

use anyhow::Context;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use sqlx::{ConnectOptions, Connection, SqliteConnection};
use tempfile::tempdir;
use tracing::info;
use url::Url;

pub(super) async fn encrypt_db(
    url: &Url,
    passphrase: &str,
    preserve_unencrypted: bool,
) -> anyhow::Result<()> {
    let opts: SqliteConnectOptions = url.as_str().parse()?;
    let opts = opts
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .disable_statement_logging();

    info!(%url, "encrypting db");

    let tempdir = tempdir().context("failed to create temp dir")?;
    let dest = tempdir.path().join("encrypted.db");

    let mut conn = SqliteConnection::connect_with(&opts).await?;
    sqlx::raw_sql(&format!(
        "
           ATTACH DATABASE '{}' AS encrypted KEY '{passphrase}';
           SELECT sqlcipher_export('encrypted');
           DETACH DATABASE encrypted;
        ",
        dest.display(),
    ))
    .execute(&mut conn)
    .await
    .context("failed to encrypt db")?;
    conn.close().await?;

    let origin = url.path();
    if preserve_unencrypted {
        let backup = format!("{origin}.backup");
        std::fs::copy(origin, &backup)
            .with_context(|| format!("failed to backup the unencrypted database at: {backup}"))?;
    }

    std::fs::copy(dest, origin)
        .with_context(|| format!("failed to replace unencrypted db at: {origin}"))?;

    Ok(())
}

pub(super) fn is_sqlite_encrypted_heuristics(url: &Url) -> Option<bool> {
    const SQLITE3_HEADER: &[u8] = b"SQLite format 3\0";

    let mut buf = [0; SQLITE3_HEADER.len()];
    File::open(url.path()).ok()?.read_exact(&mut buf).ok()?;

    Some(buf != SQLITE3_HEADER)
}

#[cfg(test)]
mod tests {
    use crate::storage::SqliteStorage;

    use super::*;

    #[tokio::test]
    async fn test_encrypt_unencrypted() {
        let tempdir = tempdir().unwrap();
        let path = tempdir.path().join("data.sqlite");
        let url: Url = format!("sqlite://{}", path.display()).parse().unwrap();

        let _ = SqliteStorage::open(&url, None).await.unwrap();
        assert!(path.exists());
        assert_eq!(is_sqlite_encrypted_heuristics(&url), Some(false));

        let preserve_unencrypted = true;
        let passphrase = "secret".to_owned();
        encrypt_db(&url, &passphrase, preserve_unencrypted)
            .await
            .unwrap();

        assert!(path.exists());
        assert_eq!(is_sqlite_encrypted_heuristics(&url), Some(true));

        let backup_url: Url = format!("{url}.backup").parse().unwrap();
        assert_eq!(is_sqlite_encrypted_heuristics(&backup_url), Some(false));

        let _ = SqliteStorage::open(&url, Some(passphrase)).await.unwrap();
    }
}

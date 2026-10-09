use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
};
use sqlx::{query, query_as, query_scalar};
use tracing::{error, info};
use url::Url;
use uuid::Uuid;

use crate::receipt::Receipt;
use crate::signal::Attachment;
use crate::storage::{MessageId, Metadata};
use crate::{
    data::{BodyRange, Channel, ChannelId, GroupData, Message, TypingSet},
    passphrase::Passphrase,
};

use super::encoding::BlobData;
use super::encrypt::{encrypt_db, is_sqlite_encrypted_heuristics};
use super::util::ResultExt as _;

const METADATA_ID: i64 = 0;

#[derive(Clone)]
pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    pub async fn maybe_encrypt_and_open(
        url: &Url,
        passphrase: &Passphrase,
        preserve_unencrypted: bool,
    ) -> anyhow::Result<Self> {
        info!(%url, "loading sql app data");

        let db = match is_sqlite_encrypted_heuristics(url) {
            // encrypted or does not exist
            Some(true) | None => Self::open(url, passphrase).await?,
            // not encrypted => encrypt
            Some(false) => {
                encrypt_db(url, passphrase, preserve_unencrypted).await?;
                Self::open(url, passphrase).await?
            }
        };

        Ok(db)
    }

    pub async fn open(url: &Url, passphrase: &Passphrase) -> sqlx::Result<Self> {
        let opts: SqliteConnectOptions = url.as_str().parse()?;
        let opts = opts
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .pragma("key", passphrase.sqlite_pragma_key().to_string());

        let pool = SqlitePool::connect_with(opts.clone()).await?;
        sqlx::migrate!().run(&pool).await?;

        Ok(Self { pool })
    }

    #[cfg(any(test, feature = "dev"))]
    pub async fn open_unencrypted(url: &Url) -> sqlx::Result<Self> {
        let opts: SqliteConnectOptions = url.as_str().parse()?;
        let opts = opts
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full);

        let pool = SqlitePool::connect_with(opts.clone()).await?;
        sqlx::migrate!().run(&pool).await?;

        Ok(Self { pool })
    }
}

struct SqlChannel {
    id: ChannelId,
    name: String,
    group_master_key: Option<Vec<u8>>,
    group_revision: Option<i64>,
    group_members: Option<BlobData<Vec<Uuid>>>,
    muted: bool,
    expire_timer: Option<i64>,
}

impl SqlChannel {
    fn convert(self) -> Result<Channel, ChannelConvertError> {
        let Self {
            id,
            name,
            group_master_key,
            group_revision,
            group_members,
            muted,
            expire_timer,
        } = self;
        use ChannelConvertError::*;
        let group_data = match (group_master_key, group_revision, group_members) {
            (Some(master_key_bytes), Some(revision), Some(members)) => Some(GroupData {
                master_key_bytes: master_key_bytes.try_into().map_err(|_| MasterKeyBytes)?,
                members: members.into_inner(),
                revision: revision.try_into().map_err(|_| Revision)?,
            }),
            _ => None,
        };
        let is_group = group_data.is_some();
        Ok(Channel {
            id,
            name,
            group_data,
            unread_messages: Default::default(),
            muted,
            typing: TypingSet::new(is_group),
            expire_timer: expire_timer.and_then(|t| u32::try_from(t).ok()),
        })
    }
}

struct SqlMessage {
    from_id: Uuid,
    message: Option<String>,
    arrived_at: i64,
    attachments: Option<BlobData<Vec<Attachment>>>,
    reactions: Option<BlobData<Vec<(Uuid, String)>>>,
    receipt: Option<BlobData<Receipt>>,
    body_ranges: Option<BlobData<Vec<BodyRange>>>,
    quote_arrived_at: Option<i64>,
    quote_from_id: Option<Uuid>,
    quote_message: Option<String>,
    quote_attachments: Option<BlobData<Vec<Attachment>>>,
    quote_body_ranges: Option<BlobData<Vec<BodyRange>>>,
    quote_receipt: Option<BlobData<Receipt>>,
    edit: Option<i64>,
    edited: bool,
    deleted: bool,
    expire_timer: Option<i64>,
    expires_at: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
enum MessageConvertError {
    #[error("timestamp out of bounds")]
    InvalidTimestamp,
}

impl SqlMessage {
    fn convert(self) -> Result<Message, MessageConvertError> {
        let SqlMessage {
            from_id,
            message,
            arrived_at,
            attachments,
            reactions,
            receipt,
            body_ranges,
            quote_arrived_at,
            quote_from_id,
            quote_message,
            quote_attachments,
            quote_body_ranges,
            quote_receipt,
            edit,
            edited,
            deleted,
            expire_timer,
            expires_at,
        } = self;

        let quote = quote_arrived_at
            .zip(quote_from_id)
            .and_then(|(arrived_at, from_id)| {
                let mut quote = Message::new(
                    from_id,
                    quote_message,
                    quote_body_ranges
                        .map(BlobData::into_inner)
                        .unwrap_or_default(),
                    arrived_at
                        .try_into()
                        .map_err(|_| MessageConvertError::InvalidTimestamp)
                        .ok_logged()?,
                    quote_attachments
                        .map(BlobData::into_inner)
                        .unwrap_or_default(),
                );
                quote.receipt = quote_receipt.map(BlobData::into_inner).unwrap_or_default();
                Some(quote)
            });

        Ok(Message {
            from_id,
            message,
            arrived_at: arrived_at
                .try_into()
                .map_err(|_| MessageConvertError::InvalidTimestamp)?,
            quote: quote.map(Box::new),
            attachments: attachments.map(BlobData::into_inner).unwrap_or_default(),
            reactions: reactions.map(BlobData::into_inner).unwrap_or_default(),
            receipt: receipt.map(BlobData::into_inner).unwrap_or_default(),
            body_ranges: body_ranges.map(BlobData::into_inner).unwrap_or_default(),
            send_failed: Default::default(),
            edit: edit.and_then(|edit| {
                edit.try_into()
                    .map_err(|_| MessageConvertError::InvalidTimestamp)
                    .ok_logged()
            }),
            edited,
            deleted,
            expire_timer: expire_timer.and_then(|t| u32::try_from(t).ok()),
            expires_at: expires_at.and_then(|t| u64::try_from(t).ok()),
        })
    }
}

#[derive(Debug, thiserror::Error)]
enum ChannelConvertError {
    #[error("invalid master key bytes")]
    MasterKeyBytes,
    #[error("invalid revision")]
    Revision,
}

struct SqlName {
    id: Uuid,
    name: String,
}

impl SqliteStorage {
    pub async fn channels(&self) -> Vec<Channel> {
        let channels = query_as!(
            SqlChannel,
            r#"
                    SELECT
                         c.id AS "id: _",
                         c.name,
                         c.group_master_key,
                         c.group_revision,
                         c.group_members AS "group_members: _",
                         c.muted AS "muted: _",
                         c.expire_timer
                    FROM channels c
                    ORDER BY (
                        SELECT MAX(m.arrived_at)
                        FROM messages m
                        WHERE m.channel_id = c.id AND m.edit IS NULL
                    ) DESC
                "#
        )
        .fetch_all(&self.pool)
        .await;
        channels
            .ok_logged()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|channel| channel.convert().ok_logged())
            .collect()
    }

    pub async fn channel(&self, channel_id: ChannelId) -> Option<Channel> {
        let channel_id = &channel_id;
        let channel = query_as!(
            SqlChannel,
            r#"
                        SELECT
                            id AS "id: _",
                            name,
                            group_master_key,
                            group_revision,
                            group_members AS "group_members: _",
                            muted AS "muted: _",
                            expire_timer
                        FROM channels
                        WHERE id = ?
                    "#,
            channel_id
        )
        .fetch_optional(&self.pool)
        .await
        .ok_logged()?;
        channel?.convert().ok_logged()
    }

    pub async fn store_channel(&self, channel: &Channel) {
        let id = &channel.id;
        let name = &channel.name;
        let (group_master_key, group_revision, group_members) = channel
            .group_data
            .as_ref()
            .map(|group_data| {
                (
                    Some(&group_data.master_key_bytes[..]),
                    Some(group_data.revision),
                    Some(BlobData(group_data.members.as_slice())),
                )
            })
            .unwrap_or_default();
        let muted = channel.muted;
        let expire_timer: Option<i64> = channel.expire_timer.map(|t| t as i64);

        query!(
            r#"
                    INSERT INTO channels(
                        id,
                        name,
                        group_master_key,
                        group_revision,
                        group_members,
                        muted,
                        expire_timer
                    )
                    VALUES (?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT(id) DO UPDATE SET
                        name = excluded.name,
                        group_master_key = excluded.group_master_key,
                        group_revision = excluded.group_revision,
                        group_members = excluded.group_members,
                        muted = excluded.muted,
                        expire_timer = excluded.expire_timer
                "#,
            id,
            name,
            group_master_key,
            group_revision,
            group_members,
            muted,
            expire_timer
        )
        .execute(&self.pool)
        .await
        .ok_logged();
    }

    pub async fn messages_tail(&self, channel_id: ChannelId, limit: usize) -> Vec<Message> {
        let channel_id = &channel_id;
        let limit = i64::try_from(limit).expect("usize overflow");
        let sql_messages = query_as!(
            SqlMessage,
            r#"
                    SELECT
                        m.arrived_at AS "arrived_at!",
                        m.from_id AS "from_id: _",
                        m.message,
                        m.receipt AS "receipt: _",
                        m.body_ranges AS "body_ranges: _",
                        m.attachments AS "attachments: _",
                        m.reactions AS "reactions: _",
                        q.arrived_at AS "quote_arrived_at: _",
                        q.from_id AS "quote_from_id: _",
                        q.message AS quote_message,
                        q.attachments AS "quote_attachments: _",
                        q.body_ranges AS "quote_body_ranges: _",
                        q.receipt AS "quote_receipt: _",
                        NULL AS "edit: _",
                        m.edited AS "edited: _",
                        m.deleted AS "deleted: _",
                        m.expire_timer AS "expire_timer: _",
                        m.expires_at AS "expires_at: _"
                    FROM messages AS m
                    LEFT JOIN messages AS q ON q.channel_id = ?1 AND q.arrived_at = m.quote
                    WHERE m.channel_id = ?1 AND m.edit IS NULL
                    ORDER BY m.arrived_at DESC
                    LIMIT ?2
                "#,
            channel_id,
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .ok_logged()
        .unwrap_or_default();

        sql_messages
            .into_iter()
            .rev()
            .map(SqlMessage::convert)
            .filter_map(|message| message.ok_logged())
            .collect()
    }

    pub async fn messages_before(
        &self,
        channel_id: ChannelId,
        anchor: u64,
        limit: usize,
    ) -> Vec<Message> {
        let channel_id = &channel_id;
        let anchor = i64::try_from(anchor).expect("u64 overflow");
        let limit = i64::try_from(limit).expect("usize overflow");
        let sql_messages = query_as!(
            SqlMessage,
            r#"
                    SELECT
                        m.arrived_at AS "arrived_at!",
                        m.from_id AS "from_id: _",
                        m.message,
                        m.receipt AS "receipt: _",
                        m.body_ranges AS "body_ranges: _",
                        m.attachments AS "attachments: _",
                        m.reactions AS "reactions: _",
                        q.arrived_at AS "quote_arrived_at: _",
                        q.from_id AS "quote_from_id: _",
                        q.message AS quote_message,
                        q.attachments AS "quote_attachments: _",
                        q.body_ranges AS "quote_body_ranges: _",
                        q.receipt AS "quote_receipt: _",
                        NULL AS "edit: _",
                        m.edited AS "edited: _",
                        m.deleted AS "deleted: _",
                        m.expire_timer AS "expire_timer: _",
                        m.expires_at AS "expires_at: _"
                    FROM messages AS m
                    LEFT JOIN messages AS q ON q.channel_id = ?1 AND q.arrived_at = m.quote
                    WHERE m.channel_id = ?1 AND m.arrived_at < ?2 AND m.edit IS NULL
                    ORDER BY m.arrived_at DESC
                    LIMIT ?3
                "#,
            channel_id,
            anchor,
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .ok_logged()
        .unwrap_or_default();

        sql_messages
            .into_iter()
            .rev()
            .map(SqlMessage::convert)
            .filter_map(|message| message.ok_logged())
            .collect()
    }

    pub async fn messages_after(
        &self,
        channel_id: ChannelId,
        anchor: u64,
        limit: usize,
    ) -> Vec<Message> {
        let channel_id = &channel_id;
        let anchor = i64::try_from(anchor).expect("u64 overflow");
        let limit = i64::try_from(limit).expect("usize overflow");
        let sql_messages = query_as!(
            SqlMessage,
            r#"
                    SELECT
                        m.arrived_at AS "arrived_at!",
                        m.from_id AS "from_id: _",
                        m.message,
                        m.receipt AS "receipt: _",
                        m.body_ranges AS "body_ranges: _",
                        m.attachments AS "attachments: _",
                        m.reactions AS "reactions: _",
                        q.arrived_at AS "quote_arrived_at: _",
                        q.from_id AS "quote_from_id: _",
                        q.message AS quote_message,
                        q.attachments AS "quote_attachments: _",
                        q.body_ranges AS "quote_body_ranges: _",
                        q.receipt AS "quote_receipt: _",
                        NULL AS "edit: _",
                        m.edited AS "edited: _",
                        m.deleted AS "deleted: _",
                        m.expire_timer AS "expire_timer: _",
                        m.expires_at AS "expires_at: _"
                    FROM messages AS m
                    LEFT JOIN messages AS q ON q.channel_id = ?1 AND q.arrived_at = m.quote
                    WHERE m.channel_id = ?1 AND m.arrived_at > ?2 AND m.edit IS NULL
                    ORDER BY m.arrived_at ASC
                    LIMIT ?3
                "#,
            channel_id,
            anchor,
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .ok_logged()
        .unwrap_or_default();

        sql_messages
            .into_iter()
            .map(SqlMessage::convert)
            .filter_map(|message| message.ok_logged())
            .collect()
    }

    pub async fn messages(&self, channel_id: ChannelId) -> Vec<Message> {
        let channel_id = &channel_id;
        let messages = query_as!(
            SqlMessage,
            r#"
                    SELECT
                        m.arrived_at AS "arrived_at!",
                        m.from_id AS "from_id: _",
                        m.message,
                        m.receipt AS "receipt: _",
                        m.body_ranges AS "body_ranges: _",
                        m.attachments AS "attachments: _",
                        m.reactions AS "reactions: _",
                        q.arrived_at AS "quote_arrived_at: _",
                        q.from_id AS "quote_from_id: _",
                        q.message AS quote_message,
                        q.attachments AS "quote_attachments: _",
                        q.body_ranges AS "quote_body_ranges: _",
                        q.receipt AS "quote_receipt: _",
                        NULL AS "edit: _",
                        m.edited AS "edited: _",
                        m.deleted AS "deleted: _",
                        m.expire_timer AS "expire_timer: _",
                        m.expires_at AS "expires_at: _"
                    FROM messages AS m
                    LEFT JOIN messages AS q ON q.arrived_at = m.quote AND q.channel_id = ?1
                    WHERE m.channel_id = ?1 AND m.edit IS NULL
                    ORDER BY m.arrived_at ASC
                "#,
            channel_id
        )
        .fetch_all(&self.pool)
        .await;
        messages
            .ok_logged()
            .into_iter()
            .flatten()
            .filter_map(|message| message.convert().ok_logged())
            .collect()
    }

    pub async fn message(&self, message_id: MessageId) -> Option<Message> {
        let channel_id = &message_id.channel_id;
        let arrived_at: i64 = message_id
            .arrived_at
            .try_into()
            .map_err(|_| MessageConvertError::InvalidTimestamp)
            .ok_logged()?;
        let message = query_as!(
            SqlMessage,
            r#"
                    SELECT
                        m.arrived_at,
                        m.from_id AS "from_id: _",
                        m.message,
                        m.receipt AS "receipt: _",
                        m.body_ranges AS "body_ranges: _",
                        m.attachments AS "attachments: _",
                        m.reactions AS "reactions: _",
                        q.arrived_at AS "quote_arrived_at: _",
                        q.from_id AS "quote_from_id: _",
                        q.message AS quote_message,
                        q.attachments AS "quote_attachments: _",
                        q.body_ranges AS "quote_body_ranges: _",
                        q.receipt AS "quote_receipt: _",
                        m.edit,
                        m.edited as "edited: _",
                        m.deleted as "deleted: _",
                        m.expire_timer as "expire_timer: _",
                        m.expires_at as "expires_at: _"
                    FROM messages AS m
                    LEFT JOIN messages AS q ON q.arrived_at = m.quote AND q.channel_id = ?1
                    WHERE m.channel_id = ?1 AND m.arrived_at = ?2
                    GROUP BY m.arrived_at
                    LIMIT 1
                "#,
            channel_id,
            arrived_at
        )
        .fetch_optional(&self.pool)
        .await;
        let message = message.ok_logged()??.convert().ok_logged()?;
        Some(message)
    }

    pub async fn edits(&self, message_id: MessageId) -> Vec<Message> {
        let channel_id = &message_id.channel_id;
        let arrived_at: Option<i64> = message_id
            .arrived_at
            .try_into()
            .map_err(|_| MessageConvertError::InvalidTimestamp)
            .ok_logged();
        let Some(arrived_at) = arrived_at else {
            return Vec::new();
        };
        let messages = query_as!(
            SqlMessage,
            r#"
                    SELECT
                        m.arrived_at AS "arrived_at!",
                        m.from_id AS "from_id: _",
                        m.message,
                        m.receipt AS "receipt: _",
                        m.body_ranges AS "body_ranges: _",
                        m.attachments AS "attachments: _",
                        m.reactions AS "reactions: _",
                        q.arrived_at AS "quote_arrived_at: _",
                        q.from_id AS "quote_from_id: _",
                        q.message AS quote_message,
                        q.attachments AS "quote_attachments: _",
                        q.body_ranges AS "quote_body_ranges: _",
                        q.receipt AS "quote_receipt: _",
                        NULL AS "edit: _",
                        m.edited AS "edited: _",
                        m.deleted AS "deleted: _",
                        m.expire_timer AS "expire_timer: _",
                        m.expires_at AS "expires_at: _"
                    FROM messages AS m
                    LEFT JOIN messages AS q ON q.arrived_at = m.quote AND q.channel_id = ?1
                    WHERE m.channel_id = ?1 AND m.edit == ?2
                    ORDER BY m.arrived_at ASC
                "#,
            channel_id,
            arrived_at,
        )
        .fetch_all(&self.pool)
        .await;
        messages
            .ok_logged()
            .into_iter()
            .flatten()
            .filter_map(|message| message.convert().ok_logged())
            .collect()
    }

    pub async fn messages_count_after(&self, channel_id: ChannelId, arrived_at: u64) -> usize {
        let channel_id = &channel_id;
        let arrived_at = i64::try_from(arrived_at).expect("u64 overflow");

        query_scalar!(
            "
                    SELECT COUNT(*) FROM messages
                    WHERE channel_id = ?1 AND arrived_at > ?2 AND edit IS NULL
                ",
            channel_id,
            arrived_at
        )
        .fetch_one(&self.pool)
        .await
        .ok_logged()
        .and_then(|c| usize::try_from(c).ok())
        .unwrap_or_default()
    }

    pub async fn remove_expired(&self, now_ms: u64) -> Vec<MessageId> {
        let now_ms = i64::try_from(now_ms).expect("u64 overflow");

        query_as!(
            MessageId,
            r#"
                    DELETE FROM messages
                    WHERE expires_at IS NOT NULL AND expires_at <= ?1
                    RETURNING
                        channel_id AS "channel_id: _",
                        arrived_at AS "arrived_at: _"
                "#,
            now_ms,
        )
        .fetch_all(&self.pool)
        .await
        .ok_logged()
        .unwrap_or_default()
    }

    pub async fn next_expiring_at(&self) -> Option<u64> {
        query_scalar!(
            r#"
                    SELECT MIN(expires_at) AS "next_expiring_at: u64"
                    FROM messages
                    WHERE expires_at IS NOT NULL
                "#,
        )
        .fetch_optional(&self.pool)
        .await
        .ok_logged()??
    }

    pub async fn store_message(&self, channel_id: ChannelId, message: &Message) {
        let channel_id = &channel_id;
        let arrived_at: i64 = message
            .arrived_at
            .try_into()
            .map_err(|_| MessageConvertError::InvalidTimestamp)
            .ok_logged()
            .unwrap();
        let from_id = &message.from_id;
        let message_msg = message.message.as_deref();
        let quote: Option<i64> = message.quote.as_ref().and_then(|quote| {
            quote
                .arrived_at
                .try_into()
                .map_err(|_| MessageConvertError::InvalidTimestamp)
                .ok_logged()
        });
        let receipt = BlobData(&message.receipt);
        let body_ranges = BlobData(&message.body_ranges);
        let attachments = BlobData(&message.attachments);
        let reactions = BlobData(&message.reactions);
        let edit: Option<i64> = message.edit.and_then(|edit| {
            edit.try_into()
                .map_err(|_| MessageConvertError::InvalidTimestamp)
                .ok_logged()
        });
        let edited: bool = message.edited;
        let deleted: bool = message.deleted;
        let expire_timer: Option<i64> = message.expire_timer.map(|t| t as i64);
        let expires_at: Option<i64> = message.expires_at.and_then(|t| i64::try_from(t).ok());
        let inserted = query!(
            "
                    INSERT INTO messages(
                        arrived_at,
                        channel_id,
                        from_id,
                        message,
                        quote,
                        receipt,
                        body_ranges,
                        attachments,
                        reactions,
                        edit,
                        edited,
                        deleted,
                        expire_timer,
                        expires_at
                    )
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT(channel_id, arrived_at) DO UPDATE SET
                        from_id = excluded.from_id,
                        message = excluded.message,
                        quote = excluded.quote,
                        receipt = excluded.receipt,
                        body_ranges = excluded.body_ranges,
                        attachments = excluded.attachments,
                        reactions = excluded.reactions,
                        edit = excluded.edit,
                        edited = excluded.edited,
                        deleted = excluded.deleted,
                        expire_timer = excluded.expire_timer,
                        expires_at = excluded.expires_at
                ",
            arrived_at,
            channel_id,
            from_id,
            message_msg,
            quote,
            receipt,
            body_ranges,
            attachments,
            reactions,
            edit,
            edited,
            deleted,
            expire_timer,
            expires_at
        )
        .execute(&self.pool)
        .await;
        if let Err(error) = inserted {
            error!(%error, ?channel_id, arrived_at, "failed to store message");
        }
    }

    pub async fn remove_message(&self, message_id: MessageId) {
        let channel_id = &message_id.channel_id;
        let arrived_at: Option<i64> = message_id
            .arrived_at
            .try_into()
            .map_err(|_| MessageConvertError::InvalidTimestamp)
            .ok_logged();
        let Some(arrived_at) = arrived_at else {
            return;
        };
        let result = query!(
            "DELETE FROM messages WHERE channel_id = ? AND arrived_at = ?",
            channel_id,
            arrived_at
        )
        .execute(&self.pool)
        .await;
        result.ok_logged();
    }

    pub async fn names(&self) -> Vec<(Uuid, String)> {
        let names = query_as!(
            SqlName,
            r#"SELECT id AS "id: _", name AS "name: _" FROM names"#
        )
        .fetch_all(&self.pool)
        .await;
        names
            .ok_logged()
            .into_iter()
            .flatten()
            .map(|SqlName { id, name }| (id, name))
            .collect()
    }

    pub async fn name(&self, id: Uuid) -> Option<String> {
        let name = query_scalar!(r#"SELECT name AS "name: _" FROM names WHERE id = ?"#, id)
            .fetch_optional(&self.pool)
            .await;
        name.ok_logged()?
    }

    pub async fn store_name(&self, id: Uuid, name: &str) {
        query!(
            "
                    INSERT INTO names(
                        id, name
                    ) VALUES (?, ?)
                    ON CONFLICT(id) DO UPDATE SET
                        name = excluded.name
                ",
            id,
            name
        )
        .execute(&self.pool)
        .await
        .ok_logged();
    }

    pub async fn metadata(&self) -> Metadata {
        let metadata = query_as!(
            Metadata,
            r#"
                    SELECT
                        contacts_sync_request_at AS "contacts_sync_request_at: _",
                        fully_migrated AS "fully_migrated: _"
                    FROM metadata WHERE id = 0 LIMIT 1
                "#,
        )
        .fetch_optional(&self.pool)
        .await;
        metadata.ok_logged().flatten().unwrap_or_default()
    }

    pub async fn store_metadata(&self, metadata: &Metadata) {
        query!(
            "REPLACE INTO metadata(id, contacts_sync_request_at, fully_migrated)
                     VALUES (?, ?, ?)",
            METADATA_ID,
            metadata.contacts_sync_request_at,
            metadata.fully_migrated
        )
        .execute(&self.pool)
        .await
        .ok_logged();
    }

    pub async fn last_message(&self, channel_id: ChannelId) -> Option<Message> {
        self.messages_tail(channel_id, 1).await.pop()
    }

    /// Applies an edit and returns the updated original message, or `None` if the message pointed
    /// to by `target_sent_timestamp` does not exist.
    pub async fn store_edited_message(
        &self,
        channel_id: ChannelId,
        target_sent_timestamp: u64,
        message: Message,
    ) -> Option<Message> {
        // Note: target_sent_timestamp points to the previous edit or the original message
        let prev_edited = self
            .message(MessageId::new(channel_id, target_sent_timestamp))
            .await?;

        // get original message
        let mut original = if let Some(arrived_at) = prev_edited.edit {
            // previous edit => get original message
            self.message(MessageId::new(channel_id, arrived_at)).await?
        } else {
            // original message => first edit
            let original = prev_edited;

            // preserve body of the original message; it is replaced below
            let mut preserved = original.clone();
            preserved.arrived_at = original.arrived_at + 1;
            preserved.edit = Some(original.arrived_at);
            self.store_message(channel_id, &preserved).await;

            original
        };

        // store the incoming edit
        let body = message.message.clone();
        self.store_message(
            channel_id,
            &Message {
                edit: Some(original.arrived_at),
                ..message
            },
        )
        .await;

        // override the body of the original message
        original.message = body;
        original.edited = true;
        self.store_message(channel_id, &original).await;
        Some(original)
    }

    /// Marks a message as deleted (remote delete / delete for everyone).
    ///
    /// Clears the message body and attachments, and sets the `deleted` flag.
    /// Returns `true` if the message existed.
    pub async fn delete_message(&self, message_id: MessageId) -> bool {
        let Some(mut message) = self.message(message_id).await else {
            return false;
        };
        message.message = None;
        message.attachments.clear();
        message.body_ranges.clear();
        message.deleted = true;
        self.store_message(message_id.channel_id, &message).await;
        true
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::tempdir;
    use uuid::uuid;

    use super::*;

    async fn fixtures() -> SqliteStorage {
        let url: Url = "sqlite::memory:".parse().unwrap();
        let storage = SqliteStorage::open(&url, &Passphrase::new("secret").unwrap())
            .await
            .unwrap();

        let user_channel = ChannelId::User(uuid!("966960e0-a8cd-43f1-ac7a-2c986dd470cd"));
        storage
            .store_channel(&Channel {
                id: user_channel,
                name: "direct-channel".to_owned(),
                group_data: None,
                unread_messages: 1,
                muted: false,
                typing: TypingSet::new(false),
                expire_timer: None,
            })
            .await;
        storage
            .store_message(
                user_channel,
                &Message {
                    from_id: uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a"),
                    message: Some("hello".to_owned()),
                    arrived_at: 1664832050000,
                    quote: None,
                    attachments: Default::default(),
                    reactions: Default::default(),
                    receipt: Receipt::Nothing,
                    body_ranges: Default::default(),
                    send_failed: Default::default(),
                    edit: Default::default(),
                    edited: Default::default(),
                    deleted: Default::default(),
                    expire_timer: None,
                    expires_at: None,
                },
            )
            .await;

        let group_channel = ChannelId::Group([
            52, 49, 52, 57, 98, 57, 54, 56, 54, 56, 48, 55, 102, 100, 98, 52, 97, 56, 99, 57, 53,
            100, 57, 98, 53, 52, 49, 51, 98, 98, 99, 100,
        ]);
        storage
            .store_channel(&Channel {
                id: group_channel,
                name: "group-channel".to_owned(),
                group_data: None,
                unread_messages: 2,
                muted: false,
                typing: TypingSet::new(true),
                expire_timer: None,
            })
            .await;
        storage
            .store_message(
                group_channel,
                &Message {
                    from_id: uuid!("ac9b8aa1-691a-47e1-a566-d3e942945d07"),
                    message: Some("world".to_owned()),
                    arrived_at: 1664832050001,
                    quote: None,
                    attachments: Default::default(),
                    reactions: Default::default(),
                    receipt: Receipt::Nothing,
                    body_ranges: Default::default(),
                    send_failed: Default::default(),
                    edit: Default::default(),
                    edited: Default::default(),
                    deleted: Default::default(),
                    expire_timer: None,
                    expires_at: None,
                },
            )
            .await;

        storage
            .store_name(uuid!("966960e0-a8cd-43f1-ac7a-2c986dd470cd"), "ellie")
            .await;
        storage
            .store_name(uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a"), "joel")
            .await;

        storage
    }

    /// Storage with a single channel containing messages at arrived_at 10, 20, ..., 90
    async fn windowed_fixtures() -> (SqliteStorage, ChannelId) {
        let url: Url = "sqlite::memory:".parse().unwrap();
        let storage = SqliteStorage::open(&url, &Passphrase::new("secret").unwrap())
            .await
            .unwrap();

        let channel_id = ChannelId::User(uuid!("966960e0-a8cd-43f1-ac7a-2c986dd470cd"));
        storage
            .store_channel(&Channel {
                id: channel_id,
                name: "direct-channel".to_owned(),
                group_data: None,
                unread_messages: 0,
                muted: false,
                typing: TypingSet::new(false),
                expire_timer: None,
            })
            .await;

        let from_id = uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a");
        for arrived_at in (10..=90).step_by(10) {
            storage
                .store_message(
                    channel_id,
                    &Message::text(from_id, arrived_at, format!("message {arrived_at}")),
                )
                .await;
        }

        (storage, channel_id)
    }

    fn arrived_ats(messages: &[Message]) -> Vec<u64> {
        messages.iter().map(|message| message.arrived_at).collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_messages_tail() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let (storage, channel_id) = windowed_fixtures().await;

        // newest `limit` messages, ascending
        assert_eq!(
            arrived_ats(&storage.messages_tail(channel_id, 3).await),
            [70, 80, 90]
        );

        // short result when limit exceeds the number of messages
        assert_eq!(
            arrived_ats(&storage.messages_tail(channel_id, 100).await),
            [10, 20, 30, 40, 50, 60, 70, 80, 90]
        );

        assert!(storage.messages_tail(channel_id, 0).await.is_empty());

        let unknown_channel = ChannelId::User(uuid!("00000000-0000-0000-0000-000000000000"));
        assert!(storage.messages_tail(unknown_channel, 3).await.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_messages_before() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let (storage, channel_id) = windowed_fixtures().await;

        // messages closest to the anchor, ascending; the anchor itself is excluded
        assert_eq!(
            arrived_ats(&storage.messages_before(channel_id, 50, 2).await),
            [30, 40]
        );

        // anchor between two messages
        assert_eq!(
            arrived_ats(&storage.messages_before(channel_id, 55, 2).await),
            [40, 50]
        );

        // short result at the oldest message
        assert_eq!(
            arrived_ats(&storage.messages_before(channel_id, 50, 100).await),
            [10, 20, 30, 40]
        );

        // nothing before the oldest message
        assert!(storage.messages_before(channel_id, 10, 3).await.is_empty());

        assert!(storage.messages_before(channel_id, 50, 0).await.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_messages_after() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let (storage, channel_id) = windowed_fixtures().await;

        // messages closest to the anchor, ascending; the anchor itself is excluded
        assert_eq!(
            arrived_ats(&storage.messages_after(channel_id, 50, 2).await),
            [60, 70]
        );

        // anchor between two messages
        assert_eq!(
            arrived_ats(&storage.messages_after(channel_id, 45, 2).await),
            [50, 60]
        );

        // short result at the newest message
        assert_eq!(
            arrived_ats(&storage.messages_after(channel_id, 50, 100).await),
            [60, 70, 80, 90]
        );

        // nothing after the newest message
        assert!(storage.messages_after(channel_id, 90, 3).await.is_empty());

        assert!(storage.messages_after(channel_id, 50, 0).await.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_windowed_messages_skip_edits() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let (storage, channel_id) = windowed_fixtures().await;
        let from_id = uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a");

        // editing message 50 preserves the original body in an edit row at 51 and stores the edit
        // itself at 95; both must be skipped
        storage
            .store_edited_message(
                channel_id,
                50,
                Message::text(from_id, 95, "message 50 edited".to_owned()),
            )
            .await;

        let messages = storage.messages_tail(channel_id, 100).await;
        assert_eq!(arrived_ats(&messages), [10, 20, 30, 40, 50, 60, 70, 80, 90]);
        let edited = &messages[4];
        assert_eq!(edited.arrived_at, 50);
        assert_eq!(edited.message.as_deref(), Some("message 50 edited"));
        assert!(edited.edited);

        // the edit row at 51 is skipped
        assert_eq!(
            arrived_ats(&storage.messages_before(channel_id, 52, 1).await),
            [50]
        );
        // the edit row at 95 is skipped
        assert!(storage.messages_after(channel_id, 90, 10).await.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_windowed_messages_resolve_quotes() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let (storage, channel_id) = windowed_fixtures().await;
        let from_id = uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a");

        let quoted = storage
            .message(MessageId::new(channel_id, 20))
            .await
            .unwrap();
        let mut reply = Message::text(from_id, 100, "reply".to_owned());
        reply.quote = Some(Box::new(quoted));
        storage.store_message(channel_id, &reply).await;

        let messages = storage.messages_tail(channel_id, 1).await;
        assert_eq!(arrived_ats(&messages), [100]);
        let quote = messages[0].quote.as_ref().unwrap();
        assert_eq!(quote.arrived_at, 20);
        assert_eq!(quote.message.as_deref(), Some("message 20"));
    }

    fn test_channel(id: ChannelId, name: &str) -> Channel {
        Channel {
            id,
            name: name.to_owned(),
            group_data: None,
            unread_messages: 0,
            muted: false,
            typing: TypingSet::new(false),
            expire_timer: None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_channels_sorted_by_recency() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let url: Url = "sqlite::memory:".parse().unwrap();
        let storage = SqliteStorage::open(&url, &Passphrase::new("secret").unwrap())
            .await
            .unwrap();
        let from_id = uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a");

        let recent = ChannelId::User(uuid!("11111111-1111-1111-1111-111111111111"));
        let middle = ChannelId::User(uuid!("22222222-2222-2222-2222-222222222222"));
        let old = ChannelId::User(uuid!("33333333-3333-3333-3333-333333333333"));
        let empty = ChannelId::User(uuid!("44444444-4444-4444-4444-444444444444"));

        // store channels in a deliberately non-recency order
        for (id, name) in [
            (old, "old"),
            (recent, "recent"),
            (empty, "empty"),
            (middle, "middle"),
        ] {
            storage.store_channel(&test_channel(id, name)).await;
        }
        storage
            .store_message(old, &Message::text(from_id, 100, "old".to_owned()))
            .await;
        storage
            .store_message(middle, &Message::text(from_id, 200, "middle".to_owned()))
            .await;
        storage
            .store_message(recent, &Message::text(from_id, 300, "recent".to_owned()))
            .await;

        // most recent first; the channel without messages sorts last
        let order: Vec<ChannelId> = storage.channels().await.into_iter().map(|c| c.id).collect();
        assert_eq!(order, [recent, middle, old, empty]);

        // an edit (newer arrived_at, edit IS NOT NULL) must not change recency
        storage
            .store_edited_message(
                old,
                100,
                Message::text(from_id, 999, "old edited".to_owned()),
            )
            .await;
        let order: Vec<ChannelId> = storage.channels().await.into_iter().map(|c| c.id).collect();
        assert_eq!(order, [recent, middle, old, empty]);
    }

    /// Storing a channel must not delete its messages.
    ///
    /// To guard against using `REPLACE INTO` and deleting messages on cascade.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_store_channel_keeps_messages() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let url: Url = "sqlite::memory:".parse().unwrap();
        let storage = SqliteStorage::open(&url, &Passphrase::new("secret").unwrap())
            .await
            .unwrap();
        let from_id = uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a");
        let channel_id = ChannelId::User(uuid!("11111111-1111-1111-1111-111111111111"));

        storage
            .store_channel(&test_channel(channel_id, "channel"))
            .await;
        for arrived_at in [100, 200, 300] {
            storage
                .store_message(
                    channel_id,
                    &Message::text(from_id, arrived_at, format!("message {arrived_at}")),
                )
                .await;
        }
        assert_eq!(storage.messages_tail(channel_id, 10).await.len(), 3);

        // Re-store the same channel
        let mut channel = test_channel(channel_id, "channel");
        channel.muted = true;
        storage.store_channel(&channel).await;

        let messages = storage.messages_tail(channel_id, 10).await;
        let arrived_ats: Vec<u64> = messages.iter().map(|m| m.arrived_at).collect();
        assert_eq!(
            arrived_ats,
            [100, 200, 300],
            "messages were lost when re-storing the channel"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_messages_count_after() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let (storage, channel_id) = windowed_fixtures().await;
        let from_id = uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a");

        assert_eq!(storage.messages_count_after(channel_id, 50).await, 4);
        assert_eq!(storage.messages_count_after(channel_id, 90).await, 0);
        assert_eq!(storage.messages_count_after(channel_id, 0).await, 9);

        // edit rows (preserved original at 51, edit at 95) must not be counted
        storage
            .store_edited_message(
                channel_id,
                50,
                Message::text(from_id, 95, "message 50 edited".to_owned()),
            )
            .await;
        assert_eq!(storage.messages_count_after(channel_id, 50).await, 4);
        assert_eq!(storage.messages_count_after(channel_id, 90).await, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_last_message() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let (storage, channel_id) = windowed_fixtures().await;

        let last = storage.last_message(channel_id).await.unwrap();
        assert_eq!(last.arrived_at, 90);
        assert_eq!(last.message.as_deref(), Some("message 90"));

        let unknown_channel = ChannelId::User(uuid!("00000000-0000-0000-0000-000000000000"));
        assert_eq!(storage.last_message(unknown_channel).await, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_remove_expired() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let (storage, channel_id) = windowed_fixtures().await;
        let from_id = uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a");

        for (arrived_at, expires_at) in [(101, 100), (102, 200), (103, 300)] {
            let mut message = Message::text(from_id, arrived_at, format!("expiring {arrived_at}"));
            message.expires_at = Some(expires_at);
            storage.store_message(channel_id, &message).await;
        }

        assert_eq!(storage.next_expiring_at().await, Some(100));

        // nothing expired yet
        assert!(storage.remove_expired(99).await.is_empty());
        assert_eq!(storage.next_expiring_at().await, Some(100));

        // expiry at the boundary is inclusive
        let mut removed = storage.remove_expired(200).await;
        removed.sort_unstable();
        assert_eq!(
            removed,
            [
                MessageId::new(channel_id, 101),
                MessageId::new(channel_id, 102)
            ]
        );
        assert_eq!(storage.message(MessageId::new(channel_id, 101)).await, None);
        assert_eq!(storage.message(MessageId::new(channel_id, 102)).await, None);
        assert!(
            storage
                .message(MessageId::new(channel_id, 103))
                .await
                .is_some()
        );
        assert_eq!(storage.next_expiring_at().await, Some(300));

        let removed = storage.remove_expired(10_000).await;
        assert_eq!(removed, [MessageId::new(channel_id, 103)]);
        assert_eq!(storage.next_expiring_at().await, None);

        // messages without expires_at are untouched
        assert_eq!(
            arrived_ats(&storage.messages_tail(channel_id, 100).await),
            [10, 20, 30, 40, 50, 60, 70, 80, 90]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_channels() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let storage = fixtures().await;
        let channels = storage.channels().await;
        assert_eq!(channels.len(), 2);
        assert_eq!(
            storage.channel(channels[0].id).await.unwrap().id,
            channels[0].id
        );
        assert_eq!(
            storage.channel(channels[1].id).await.unwrap().id,
            channels[1].id
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_messages() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let storage = fixtures().await;
        let id: Uuid = "966960e0-a8cd-43f1-ac7a-2c986dd470cd".parse().unwrap();

        let messages = storage.messages(id.into()).await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message.as_deref(), Some("hello"));

        let arrived_at = messages[0].arrived_at;
        let message = storage
            .message(MessageId::new(id.into(), arrived_at))
            .await
            .unwrap();
        assert_eq!(message.arrived_at, arrived_at);
        assert_eq!(message.message.as_deref(), Some("hello"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_store_existing_message() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let storage = fixtures().await;
        let id: Uuid = "966960e0-a8cd-43f1-ac7a-2c986dd470cd".parse().unwrap();
        let arrived_at = 1664832050000;
        let mut message = storage
            .message(MessageId::new(id.into(), arrived_at))
            .await
            .unwrap();
        message.message = Some("changed".to_string());

        let arrived_at = message.arrived_at;
        storage.store_message(id.into(), &message).await;

        let messages = storage.messages(id.into()).await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].arrived_at, arrived_at);
        assert_eq!(messages[0].message.as_deref(), Some("changed"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_store_new_message() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let storage = fixtures().await;

        let id: Uuid = uuid!("966960e0-a8cd-43f1-ac7a-2c986dd470cd");

        // store quote
        let quote_arrived_at = 1664832050000;
        let quote = storage
            .message(MessageId::new(id.into(), quote_arrived_at))
            .await
            .unwrap();

        // store message
        let arrived_at = 1664832050001;
        assert_eq!(
            storage.message(MessageId::new(id.into(), arrived_at)).await,
            None
        );
        let attachments = vec![Attachment {
            id: "some_attachment".to_owned(),
            content_type: "image/png".to_owned(),
            filename: "example.png".into(),
            size: 42,
        }];
        let reactions = vec![(id, "+1".to_owned())];
        let receipt = Receipt::Read;
        let body_ranges = vec![BodyRange {
            start: 0,
            end: 1,
            value: crate::data::AssociatedValue::MentionUuid(id),
        }];
        storage
            .store_message(
                id.into(),
                &Message {
                    from_id: id,
                    message: Some("new msg".to_string()),
                    arrived_at,
                    quote: Some(Box::new(quote.clone())),
                    attachments: attachments.clone(),
                    reactions: reactions.clone(),
                    receipt,
                    body_ranges: body_ranges.clone(),
                    send_failed: Default::default(),
                    edit: Default::default(),
                    edited: Default::default(),
                    deleted: Default::default(),
                    expire_timer: None,
                    expires_at: None,
                },
            )
            .await;

        let messages = storage.messages(id.into()).await;
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].arrived_at, arrived_at);
        assert_eq!(messages[1].message.as_deref(), Some("new msg"));
        assert_eq!(messages[1].quote.as_deref(), Some(&quote));
        assert_eq!(messages[1].attachments, attachments);
        assert_eq!(messages[1].reactions, reactions);
        assert_eq!(messages[1].receipt, receipt);
        assert_eq!(messages[1].body_ranges, body_ranges);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_same_arrived_at_in_different_channels() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let storage = fixtures().await;

        // channel_a already has "hello" at this timestamp
        let channel_a = ChannelId::User(uuid!("966960e0-a8cd-43f1-ac7a-2c986dd470cd"));
        let arrived_at = 1664832050000;

        let channel_b = ChannelId::User(uuid!("91a6315b-027c-44ce-bacb-4d5cf012ba8c"));
        storage
            .store_channel(&Channel {
                id: channel_b,
                name: "channel-b".to_owned(),
                group_data: None,
                unread_messages: 0,
                muted: false,
                typing: TypingSet::new(false),
                expire_timer: None,
            })
            .await;
        storage
            .store_message(
                channel_b,
                &Message::text(
                    uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a"),
                    arrived_at,
                    "world".to_owned(),
                ),
            )
            .await;

        // both rows coexist under the composite primary key
        assert_eq!(
            storage
                .message(MessageId::new(channel_a, arrived_at))
                .await
                .unwrap()
                .message
                .as_deref(),
            Some("hello")
        );
        assert_eq!(
            storage
                .message(MessageId::new(channel_b, arrived_at))
                .await
                .unwrap()
                .message
                .as_deref(),
            Some("world")
        );
        assert_eq!(storage.messages(channel_a).await.len(), 1);
        assert_eq!(storage.messages(channel_b).await.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_delete_channel_cascades_messages() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let storage = fixtures().await;
        let channel_id = ChannelId::User(uuid!("966960e0-a8cd-43f1-ac7a-2c986dd470cd"));
        assert_eq!(storage.messages(channel_id).await.len(), 1);

        sqlx::query("DELETE FROM channels WHERE id = ?")
            .bind(&channel_id)
            .execute(&storage.pool)
            .await
            .unwrap();

        assert_eq!(storage.messages(channel_id).await.len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_store_message_for_missing_channel_is_dropped() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let storage = fixtures().await;
        let missing = ChannelId::User(uuid!("00000000-0000-0000-0000-000000000000"));

        // the foreign key rejects the insert; the message is dropped, not stored
        storage
            .store_message(
                missing,
                &Message::text(
                    uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a"),
                    123,
                    "lost".to_owned(),
                ),
            )
            .await;

        assert_eq!(storage.message(MessageId::new(missing, 123)).await, None);
        assert_eq!(storage.messages(missing).await.len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_names() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let storage = fixtures().await;
        let id1 = uuid!("966960e0-a8cd-43f1-ac7a-2c986dd470cd");
        let id2 = uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a");
        let id3 = uuid!("91a6315b-027c-44ce-bacb-4d5cf012ba8c");

        assert_eq!(storage.names().await.len(), 2);
        assert_eq!(storage.name(id1).await.unwrap(), "ellie");
        assert_eq!(storage.name(id2).await.unwrap(), "joel");

        storage.store_name(id3, "abby").await;
        assert_eq!(storage.names().await.len(), 3);
        assert_eq!(storage.name(id1).await.unwrap(), "ellie");
        assert_eq!(storage.name(id2).await.unwrap(), "joel");
        assert_eq!(storage.name(id3).await.unwrap(), "abby");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_storage_metadata() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let storage = fixtures().await;
        assert_eq!(storage.metadata().await.contacts_sync_request_at, None);

        let dt = Utc::now();
        storage
            .store_metadata(&Metadata {
                contacts_sync_request_at: Some(dt),
                fully_migrated: Some(true),
            })
            .await;

        let Metadata {
            contacts_sync_request_at,
            fully_migrated,
        } = storage.metadata().await;
        assert_eq!(contacts_sync_request_at, Some(dt));
        assert_eq!(fully_migrated, Some(true));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sqlite_created_encrypted_with_passphrase() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();

        let tempdir = tempdir().unwrap();
        let path = tempdir.path().join("data.db");
        let url: Url = format!("sqlite://{}", path.display()).parse().unwrap();

        assert_eq!(is_sqlite_encrypted_heuristics(&url), None);

        let secret = "secret".to_owned();
        SqliteStorage::maybe_encrypt_and_open(&url, &Passphrase::new(secret).unwrap(), false)
            .await
            .unwrap();

        assert_eq!(is_sqlite_encrypted_heuristics(&url), Some(true));
    }
}

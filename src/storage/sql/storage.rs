use std::borrow::Cow;
use std::cell::Cell;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::time::Instant;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use sqlx::{ConnectOptions, Connection, SqliteConnection};
use thread_local::ThreadLocal;
use tokio::runtime::Runtime;
use tracing::{info, instrument, trace};
use url::Url;
use uuid::Uuid;

use crate::data::{BodyRange, Channel, ChannelId, GroupData, Message, TypingSet};
use crate::receipt::Receipt;
use crate::signal::Attachment;
use crate::storage::copy::{self, Stats};
use crate::storage::{MessageId, Metadata, Storage};

use super::encoding::BlobData;
use super::encrypt::{encrypt_db, is_sqlite_encrypted_heuristics};
use super::util::ResultExt as _;

const METADATA_ID: i64 = 0;

pub struct SqliteStorage {
    opts: SqliteConnectOptions,
    thread: rayon::ThreadPool,
    thread_local: ThreadLocal<ThreadLocalResource>,
}

struct ThreadLocalResource {
    rt: Runtime,
    conn_cell: Cell<Option<SqliteConnection>>,
}

struct ExecuteContext<'ctx, 'env: 'ctx> {
    conn: &'ctx mut SqliteConnection,
    _env: PhantomData<&'env ()>,
}

impl<'ctx, 'env: 'ctx> ExecuteContext<'ctx, 'env> {
    fn new(conn: &'ctx mut SqliteConnection) -> Self {
        Self {
            conn,
            _env: PhantomData,
        }
    }
}

impl SqliteStorage {
    pub fn maybe_encrypt_and_open(
        url: &Url,
        passphrase: Option<String>,
        preserve_unencrypted: bool,
    ) -> anyhow::Result<Self> {
        info!("loading app data from: {}", url);
        let db = if let Some(passphrase) = passphrase {
            match is_sqlite_encrypted_heuristics(url) {
                // encrypted or does not exist
                Some(true) | None => Self::open(url, Some(passphrase)),
                // not encrypted => encrypt
                Some(false) => {
                    encrypt_db(url, &passphrase, preserve_unencrypted)?;
                    Self::open(url, Some(passphrase))
                }
            }
        } else {
            // not encrypted without passphrase => stays unencrypted
            Self::open(url, None)
        };
        Ok(db?)
    }

    pub fn open(url: &Url, passphrase: Option<String>) -> sqlx::Result<Self> {
        let opts: SqliteConnectOptions = url.as_str().parse()?;
        let mut opts = opts
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .disable_statement_logging();
        if let Some(passphrase) = passphrase {
            opts = opts.pragma("key", passphrase);
        }

        let thread = rayon::ThreadPoolBuilder::new()
            .thread_name(|_| "sqlite-sync".to_owned())
            .num_threads(1)
            .build()
            .unwrap();
        let thread_local = ThreadLocal::with_capacity(1);

        thread.scope(|_scope| -> sqlx::Result<()> {
            thread_local.get_or_try(|| -> sqlx::Result<ThreadLocalResource> {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let conn = rt.block_on(async {
                    let mut conn = SqliteConnection::connect_with(&opts).await?;
                    sqlx::migrate!().run(&mut conn).await?;
                    Ok::<_, sqlx::Error>(conn)
                })?;
                Ok(ThreadLocalResource {
                    rt,
                    conn_cell: Cell::new(Some(conn)),
                })
            })?;
            Ok(())
        })?;

        Ok(Self {
            opts,
            thread,
            thread_local,
        })
    }

    pub async fn copy_from(&mut self, from: &impl Storage) -> Result<Stats, sqlx::Error> {
        // reconnect without disabled journaling and synchronous mode
        // otherwise copying the data is really slow
        let copy_opts = self
            .opts
            .clone()
            .journal_mode(SqliteJournalMode::Off)
            .synchronous(SqliteSynchronous::Off);

        self.thread.scope(|_scope| {
            let ThreadLocalResource { rt, conn_cell } =
                self.thread_local.get().expect("logic error");
            let conn = conn_cell.take().expect("logic_error");
            rt.block_on(conn.close())?;

            let copy_conn = rt.block_on(SqliteConnection::connect_with(&copy_opts))?;
            conn_cell.replace(Some(copy_conn));
            Ok::<_, sqlx::Error>(())
        })?;

        let stats = copy::copy(from, self);

        self.thread.scope(|_scope| {
            let ThreadLocalResource { rt, conn_cell } =
                self.thread_local.get().expect("logic error");
            let conn = conn_cell.take().expect("logic error");
            rt.block_on(conn.close())?;

            let copy_conn = rt.block_on(SqliteConnection::connect_with(&self.opts))?;
            conn_cell.replace(Some(copy_conn));
            Ok::<_, sqlx::Error>(())
        })?;

        Ok(stats)
    }

    #[instrument(level = "trace", skip_all)]
    fn execute<'env, F, T>(&self, task: F) -> T
    where
        F: for<'conn> FnOnce(
                ExecuteContext<'conn, 'env>,
            ) -> Pin<Box<dyn Future<Output = T> + 'conn>>
            + Send
            + 'env,
        T: Send,
    {
        let now = Instant::now();
        let res = self.thread.scope(|_scope| {
            let ThreadLocalResource { rt, conn_cell } =
                self.thread_local.get().expect("logic error");
            let mut conn = conn_cell.take().expect("logic error");
            let res = rt.block_on(task(ExecuteContext::new(&mut conn)));
            conn_cell.replace(Some(conn));
            res
        });
        trace!(elapsed =? now.elapsed(), "sql executed");
        res
    }
}

impl Drop for SqliteStorage {
    fn drop(&mut self) {
        // drop the runtime on the thread it was created
        // otherwise we might drop it inside another tokio runtime, and tokio does not like it.
        self.thread.scope(|_scope| {
            self.thread_local.clear();
        });
    }
}

struct SqlChannel {
    id: ChannelId,
    name: String,
    group_master_key: Option<Vec<u8>>,
    group_revision: Option<i64>,
    group_members: Option<BlobData<Vec<Uuid>>>,
}

impl SqlChannel {
    fn convert(self) -> Result<Channel, ChannelConvertError> {
        let Self {
            id,
            name,
            group_master_key,
            group_revision,
            group_members,
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
            typing: TypingSet::new(is_group),
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

impl Storage for SqliteStorage {
    fn channels<'s>(&'s self) -> Box<dyn Iterator<Item = Cow<Channel>> + 's> {
        let channels = self.execute(|ctx|
            Box::pin(sqlx::query_as!(
                SqlChannel,
                r#"
                    SELECT id AS "id: _", name, group_master_key, group_revision, group_members AS "group_members: _"
                    FROM channels
                "#
            )
            .fetch_all(ctx.conn))
        );

        Box::new(
            channels
                .ok_logged()
                .into_iter()
                .flatten()
                .filter_map(|channel| channel.convert().ok_logged().map(Cow::Owned)),
        )
    }

    fn channel(&self, channel_id: ChannelId) -> Option<Cow<Channel>> {
        let channel_id = &channel_id;
        let channel = self
            .execute(|ctx|
                Box::pin(sqlx::query_as!(
                    SqlChannel,
                    r#"
                        SELECT id AS "id: _", name, group_master_key, group_revision, group_members AS "group_members: _"
                        FROM channels
                        WHERE id = ?
                    "#,
                    channel_id
                )
                .fetch_optional(ctx.conn))
            )
            .ok_logged()?;
        channel?.convert().ok_logged().map(Cow::Owned)
    }

    fn store_channel(&mut self, channel: Channel) -> Cow<Channel> {
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
        let inserted = self.execute(|ctx| {
            Box::pin(
                sqlx::query!(
                    r#"
                    REPLACE INTO channels(id, name, group_master_key, group_revision, group_members)
                    VALUES (?, ?, ?, ?, ?)
                "#,
                    id,
                    name,
                    group_master_key,
                    group_revision,
                    group_members
                )
                .execute(ctx.conn),
            )
        });

        inserted.ok_logged();
        Cow::Owned(channel)
    }

    fn messages(
        &self,
        channel_id: ChannelId,
    ) -> Box<dyn DoubleEndedIterator<Item = Cow<Message>> + '_> {
        let channel_id = &channel_id;
        let messages = self.execute(|ctx| {
            Box::pin(
                sqlx::query_as!(
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
                        m.edited AS "edited: _"
                    FROM messages AS m
                    LEFT JOIN messages AS q ON q.arrived_at = m.quote AND q.channel_id = ?1
                    WHERE m.channel_id = ?1 AND m.edit IS NULL
                    ORDER BY m.arrived_at ASC
                "#,
                    channel_id
                )
                .fetch_all(ctx.conn),
            )
        });
        Box::new(
            messages
                .ok_logged()
                .into_iter()
                .flatten()
                .filter_map(|message| message.convert().ok_logged().map(Cow::Owned)),
        )
    }

    fn edits(
        &self,
        message_id: MessageId,
    ) -> Box<dyn DoubleEndedIterator<Item = Cow<Message>> + '_> {
        let channel_id = &message_id.channel_id;
        let arrived_at: Option<i64> = message_id
            .arrived_at
            .try_into()
            .map_err(|_| MessageConvertError::InvalidTimestamp)
            .ok_logged();
        let Some(arrived_at) = arrived_at else {
            return Box::new(std::iter::empty());
        };
        let messages = self.execute(|ctx| {
            Box::pin(
                sqlx::query_as!(
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
                        m.edited AS "edited: _"
                    FROM messages AS m
                    LEFT JOIN messages AS q ON q.arrived_at = m.quote AND q.channel_id = ?1
                    WHERE m.channel_id = ?1 AND m.edit == ?2
                    ORDER BY m.arrived_at ASC
                "#,
                    channel_id,
                    arrived_at,
                )
                .fetch_all(ctx.conn),
            )
        });
        Box::new(
            messages
                .ok_logged()
                .into_iter()
                .flatten()
                .filter_map(|message| message.convert().ok_logged().map(Cow::Owned)),
        )
    }

    fn message(&self, message_id: MessageId) -> Option<Cow<Message>> {
        let channel_id = &message_id.channel_id;
        let arrived_at: i64 = message_id
            .arrived_at
            .try_into()
            .map_err(|_| MessageConvertError::InvalidTimestamp)
            .ok_logged()?;
        let message = self.execute(|ctx| {
            Box::pin(
                sqlx::query_as!(
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
                        m.edited as "edited: _"
                    FROM messages AS m
                    LEFT JOIN messages AS q ON q.arrived_at = m.quote AND q.channel_id = ?1
                    WHERE m.channel_id = ?1 AND m.arrived_at = ?2
                    GROUP BY m.arrived_at
                    LIMIT 1
                "#,
                    channel_id,
                    arrived_at
                )
                .fetch_optional(ctx.conn),
            )
        });
        let message = message.ok_logged()??.convert().ok_logged()?;
        Some(Cow::Owned(message))
    }

    fn store_message(&mut self, channel_id: ChannelId, message: Message) -> Cow<Message> {
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
        let inserted = self.execute(|ctx| {
            Box::pin(
                sqlx::query!(
                    "
                    REPLACE INTO messages(
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
                        edited
                    )
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
                    edited
                )
                .execute(ctx.conn),
            )
        });
        inserted.ok_logged();
        Cow::Owned(message)
    }

    fn names(&self) -> Box<dyn Iterator<Item = (Uuid, Cow<str>)> + '_> {
        let names = self.execute(|ctx| {
            Box::pin(
                sqlx::query_as!(
                    SqlName,
                    r#"SELECT id AS "id: _", name AS "name: _" FROM names"#
                )
                .fetch_all(ctx.conn),
            )
        });
        let names = names
            .ok_logged()
            .into_iter()
            .flatten()
            .map(|SqlName { id, name }| (id, Cow::Owned(name)));
        Box::new(names)
    }

    fn name(&self, id: Uuid) -> Option<Cow<str>> {
        struct SqlName {
            name: String,
        }
        let name = self.execute(|ctx| {
            Box::pin(
                sqlx::query_as!(
                    SqlName,
                    r#"SELECT name AS "name: _" FROM names WHERE id = ?"#,
                    id
                )
                .fetch_optional(ctx.conn),
            )
        });
        name.ok_logged()?
            .map(|SqlName { name }| name)
            .map(Cow::Owned)
    }

    fn store_name(&mut self, id: Uuid, name: String) -> Cow<str> {
        self.execute(|ctx| {
            Box::pin(
                sqlx::query!("REPLACE INTO names(id, name) VALUES (?, ?)", id, name)
                    .execute(ctx.conn),
            )
        })
        .ok_logged();
        Cow::Owned(name)
    }

    fn metadata(&self) -> Cow<Metadata> {
        let metadata = self.execute(|ctx| {
            Box::pin(
                sqlx::query_as!(
                    Metadata,
                    r#"
                        SELECT
                            contacts_sync_request_at AS "contacts_sync_request_at: _",
                            fully_migrated AS "fully_migrated: _"
                        FROM metadata WHERE id = 0 LIMIT 1
                    "#,
                )
                .fetch_optional(ctx.conn),
            )
        });
        Cow::Owned(metadata.ok_logged().flatten().unwrap_or_default())
    }

    fn store_metadata(&mut self, metadata: Metadata) -> Cow<Metadata> {
        self.execute(|ctx| {
            Box::pin(
                sqlx::query!(
                    "REPLACE INTO metadata(id, contacts_sync_request_at, fully_migrated)
                     VALUES (?, ?, ?)",
                    METADATA_ID,
                    metadata.contacts_sync_request_at,
                    metadata.fully_migrated
                )
                .execute(ctx.conn),
            )
        })
        .ok_logged();
        Cow::Owned(metadata)
    }

    fn save(&mut self) {}

    fn message_channel(&self, arrived_at: u64) -> Option<ChannelId> {
        struct SqlChannelId {
            channel_id: ChannelId,
        }

        let arrived_at: i64 = arrived_at
            .try_into()
            .map_err(|_| MessageConvertError::InvalidTimestamp)
            .ok_logged()?;

        self.execute(|ctx| {
            Box::pin(
                sqlx::query_as!(
                    SqlChannelId,
                    r#"
                    SELECT
                        m.channel_id AS "channel_id: _"
                    FROM messages AS m
                    WHERE m.arrived_at = ?
                    LIMIT 1
                "#,
                    arrived_at
                )
                .fetch_optional(ctx.conn),
            )
        })
        .ok_logged()?
        .map(|channel_id| channel_id.channel_id)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::tempdir;
    use uuid::uuid;

    use super::*;

    fn fixtures() -> SqliteStorage {
        let url: Url = "sqlite::memory:".parse().unwrap();
        let mut storage = SqliteStorage::open(&url, None).unwrap();

        let user_channel = ChannelId::User(uuid!("966960e0-a8cd-43f1-ac7a-2c986dd470cd"));
        storage.store_channel(Channel {
            id: user_channel,
            name: "direct-channel".to_owned(),
            group_data: None,
            unread_messages: 1,
            typing: TypingSet::new(false),
        });
        storage.store_message(
            user_channel,
            Message {
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
            },
        );

        let group_channel = ChannelId::Group([
            52, 49, 52, 57, 98, 57, 54, 56, 54, 56, 48, 55, 102, 100, 98, 52, 97, 56, 99, 57, 53,
            100, 57, 98, 53, 52, 49, 51, 98, 98, 99, 100,
        ]);
        storage.store_channel(Channel {
            id: group_channel,
            name: "group-channel".to_owned(),
            group_data: None,
            unread_messages: 2,
            typing: TypingSet::new(true),
        });
        storage.store_message(
            group_channel,
            Message {
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
            },
        );

        storage.store_name(
            uuid!("966960e0-a8cd-43f1-ac7a-2c986dd470cd"),
            "ellie".to_owned(),
        );
        storage.store_name(
            uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a"),
            "joel".to_owned(),
        );

        storage
    }

    #[test]
    fn test_sqlite_storage_channels() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let storage = fixtures();
        let channels: Vec<_> = storage.channels().collect();
        assert_eq!(channels.len(), 2);
        assert_eq!(storage.channel(channels[0].id).unwrap().id, channels[0].id);
        assert_eq!(storage.channel(channels[1].id).unwrap().id, channels[1].id);
    }

    #[test]
    fn test_sqlite_storage_messages() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let storage = fixtures();
        let id: Uuid = "966960e0-a8cd-43f1-ac7a-2c986dd470cd".parse().unwrap();

        let messages: Vec<_> = storage.messages(id.into()).collect();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message.as_deref(), Some("hello"));

        let arrived_at = messages[0].arrived_at;
        let message = storage
            .message(MessageId::new(id.into(), arrived_at))
            .unwrap();
        assert_eq!(message.arrived_at, arrived_at);
        assert_eq!(message.message.as_deref(), Some("hello"));
    }

    #[test]
    fn test_sqlite_storage_store_existing_message() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let mut storage = fixtures();
        let id: Uuid = "966960e0-a8cd-43f1-ac7a-2c986dd470cd".parse().unwrap();
        let arrived_at = 1664832050000;
        let mut message = storage
            .message(MessageId::new(id.into(), arrived_at))
            .unwrap()
            .into_owned();
        message.message = Some("changed".to_string());

        let arrived_at = message.arrived_at;
        let stored_message = storage.store_message(id.into(), message);
        assert_eq!(stored_message.arrived_at, arrived_at);
        assert_eq!(stored_message.message.as_deref(), Some("changed"));

        let messages: Vec<_> = storage.messages(id.into()).collect();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].arrived_at, arrived_at);
        assert_eq!(messages[0].message.as_deref(), Some("changed"));
    }

    #[test]
    fn test_sqlite_storage_store_new_message() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let mut storage = fixtures();

        let id: Uuid = uuid!("966960e0-a8cd-43f1-ac7a-2c986dd470cd");

        // store quote
        let quote_arrived_at = 1664832050000;
        let quote = storage
            .message(MessageId::new(id.into(), quote_arrived_at))
            .unwrap()
            .into_owned();

        // store message
        let arrived_at = 1664832050001;
        assert_eq!(storage.message(MessageId::new(id.into(), arrived_at)), None);
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
        let stored_message = storage.store_message(
            id.into(),
            Message {
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
            },
        );

        assert_eq!(stored_message.arrived_at, arrived_at);
        assert_eq!(stored_message.message.as_deref(), Some("new msg"));

        let messages: Vec<_> = storage.messages(id.into()).collect();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].arrived_at, arrived_at);
        assert_eq!(messages[1].message.as_deref(), Some("new msg"));
        assert_eq!(messages[1].quote.as_deref(), Some(&quote));
        assert_eq!(messages[1].attachments, attachments);
        assert_eq!(messages[1].reactions, reactions);
        assert_eq!(messages[1].receipt, receipt);
        assert_eq!(messages[1].body_ranges, body_ranges);
    }

    #[test]
    fn test_sqlite_storage_names() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let mut storage = fixtures();
        let id1 = uuid!("966960e0-a8cd-43f1-ac7a-2c986dd470cd");
        let id2 = uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a");
        let id3 = uuid!("91a6315b-027c-44ce-bacb-4d5cf012ba8c");

        assert_eq!(storage.names().count(), 2);
        assert_eq!(storage.name(id1).unwrap(), "ellie");
        assert_eq!(storage.name(id2).unwrap(), "joel");

        assert_eq!(storage.store_name(id3, "abby".to_string()), "abby");
        assert_eq!(storage.names().count(), 3);
        assert_eq!(storage.name(id1).unwrap(), "ellie");
        assert_eq!(storage.name(id2).unwrap(), "joel");
        assert_eq!(storage.name(id3).unwrap(), "abby");
    }

    #[test]
    fn test_sqlite_storage_metadata() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let mut storage = fixtures();
        assert_eq!(storage.metadata().contacts_sync_request_at, None);

        let dt = Utc::now();
        assert_eq!(
            storage
                .store_metadata(Metadata {
                    contacts_sync_request_at: Some(dt),
                    fully_migrated: Some(true),
                })
                .contacts_sync_request_at,
            Some(dt)
        );

        let Metadata {
            contacts_sync_request_at,
            fully_migrated,
        } = storage.metadata().into_owned();
        assert_eq!(contacts_sync_request_at, Some(dt));
        assert_eq!(fully_migrated, Some(true));
    }

    #[test]
    fn test_sqlite_created_unencrypted_without_passphrase() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();

        let tempdir = tempdir().unwrap();
        let path = tempdir.path().join("data.db");
        let url: Url = format!("sqlite://{}", path.display()).parse().unwrap();

        assert_eq!(is_sqlite_encrypted_heuristics(&url), None);

        SqliteStorage::maybe_encrypt_and_open(&url, None, false).unwrap();

        assert_eq!(is_sqlite_encrypted_heuristics(&url), Some(false));
    }

    #[test]
    fn test_sqlite_created_encrypted_with_passphrase() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();

        let tempdir = tempdir().unwrap();
        let path = tempdir.path().join("data.db");
        let url: Url = format!("sqlite://{}", path.display()).parse().unwrap();

        assert_eq!(is_sqlite_encrypted_heuristics(&url), None);

        let secret = "secret".to_owned();
        SqliteStorage::maybe_encrypt_and_open(&url, Some(secret), false).unwrap();

        assert_eq!(is_sqlite_encrypted_heuristics(&url), Some(true));
    }

    #[test]
    fn test_sqlite_storage_message_channel() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let mut storage = fixtures();
        let from_id = uuid!("966960e0-a8cd-43f1-ac7a-2c986dd470cd");
        let channel_id = ChannelId::User(uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a"));
        storage.store_message(
            channel_id,
            Message::text(from_id, 1664832050000, "hello".to_owned()),
        );
        assert_eq!(storage.message_channel(1664832050000), Some(channel_id));
    }
}

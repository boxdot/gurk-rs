use std::borrow::Cow;
use std::future::Future;
use std::time::Instant;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool};
use thread_local::ThreadLocal;
use tokio::runtime::Runtime;
use tracing::{instrument, trace, Level};
use uuid::Uuid;

use crate::data::{Channel, ChannelId, GroupData, Message, TypingSet};
use crate::storage::{MessageId, Metadata, Storage};

use super::encoding::BlobData;
use super::util::ResultExt as _;

pub struct SqliteStorage {
    pool: SqlitePool,
    thread: rayon::ThreadPool,
    local_rt: ThreadLocal<Runtime>,
}

impl SqliteStorage {
    pub async fn open(url: &str) -> Result<Self, sqlx::Error> {
        let opts: SqliteConnectOptions = url.parse()?;
        let opts = opts
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePool::connect_with(opts).await?;
        sqlx::migrate!("src/storage/migrations").run(&pool).await?;

        let thread = rayon::ThreadPoolBuilder::new()
            .thread_name(|_| "sqlite-sync".to_owned())
            .num_threads(1)
            .build()
            .unwrap();
        Ok(Self {
            pool,
            thread,
            local_rt: ThreadLocal::with_capacity(1),
        })
    }

    #[instrument(level = "trace", skip_all)]
    fn execute<T: Send>(&self, fut: impl Future<Output = T> + Send) -> T {
        let now = Instant::now();
        let res = self.thread.scope(|_scope| {
            let rt = self.local_rt.get_or(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
            });
            rt.block_on(fut)
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
            self.local_rt.clear();
        });
    }
}

struct SqlChannel {
    id: ChannelId,
    name: String,
    group_master_key: Option<Vec<u8>>,
    group_revision: Option<i64>,
    group_members: Option<Vec<u8>>,
}

impl SqlChannel {
    fn convert(self) -> Result<Channel, ChannelConvertError<Vec<Uuid>>> {
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
                members: BlobData::decode(&members).map_err(Blob)?,
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

#[derive(Debug, thiserror::Error)]
enum ChannelConvertError<T: BlobData> {
    #[error("invalid master key bytes")]
    MasterKeyBytes,
    #[error("invalid revision")]
    Revision,
    #[error(transparent)]
    Blob(<T as BlobData>::Error),
}

impl Storage for SqliteStorage {
    fn channels<'s>(&'s self) -> Box<dyn Iterator<Item = Cow<Channel>> + 's> {
        let channels = self.execute(
            sqlx::query_as!(
                SqlChannel,
                r#"
                    SELECT id AS "id: _", name, group_master_key, group_revision, group_members
                    FROM channels
                "#
            )
            .fetch_all(&self.pool),
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
            .execute(
                sqlx::query_as!(
                    SqlChannel,
                    r#"
                        SELECT id AS "id: _", name, group_master_key, group_revision, group_members
                        FROM channels
                        WHERE id = ?
                    "#,
                    channel_id
                )
                .fetch_one(&self.pool),
            )
            .ok_logged()?;
        channel.convert().ok_logged().map(Cow::Owned)
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
                    Some(group_data.members.encode().ok_logged().unwrap_or_default()),
                )
            })
            .unwrap_or_default();
        let inserted = self.execute(
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
            .execute(&self.pool),
        );

        inserted.ok_logged();
        Cow::Owned(channel)
    }

    fn messages<'s>(
        &'s self,
        _channel_id: ChannelId,
    ) -> Box<dyn DoubleEndedIterator<Item = Cow<Message>> + 's> {
        todo!()
    }

    fn message(&self, _message_id: MessageId) -> Option<Cow<Message>> {
        todo!()
    }

    fn store_message(&mut self, _channel_id: ChannelId, _message: Message) -> Cow<Message> {
        todo!()
    }

    fn names<'s>(&'s self) -> Box<dyn Iterator<Item = (Uuid, Cow<str>)> + 's> {
        todo!()
    }

    fn name(&self, _id: Uuid) -> Option<Cow<str>> {
        todo!()
    }

    fn store_name(&mut self, _id: Uuid, _name: String) -> Cow<str> {
        todo!()
    }

    fn metadata(&self) -> Cow<Metadata> {
        todo!()
    }

    fn store_metadata(&mut self, _metadata: Metadata) -> Cow<Metadata> {
        todo!()
    }

    fn save(&mut self) {}
}

#[cfg(test)]
mod tests {
    use uuid::uuid;

    use super::*;

    async fn fixtures() -> SqliteStorage {
        let mut storage = SqliteStorage::open("sqlite::memory:").await.unwrap();
        storage.store_channel(Channel {
            id: ChannelId::User(uuid!("966960e0-a8cd-43f1-ac7a-2c986dd470cd")),
            name: "direct-channel".to_owned(),
            group_data: None,
            unread_messages: 1,
            typing: TypingSet::new(false),
        });
        storage.store_channel(Channel {
            id: ChannelId::Group([
                52, 49, 52, 57, 98, 57, 54, 56, 54, 56, 48, 55, 102, 100, 98, 52, 97, 56, 99, 57,
                53, 100, 57, 98, 53, 52, 49, 51, 98, 98, 99, 100,
            ]),
            name: "group-channel".to_owned(),
            group_data: None,
            unread_messages: 2,
            typing: TypingSet::new(true),
        });
        storage
    }

    #[tokio::test]
    async fn test_sqlite_storage_channels() {
        let _ = tracing_subscriber::fmt::try_init();
        let storage = fixtures().await;
        let channels: Vec<_> = storage.channels().collect();
        assert_eq!(channels.len(), 2);
        assert_eq!(storage.channel(channels[0].id).unwrap().id, channels[0].id);
        assert_eq!(storage.channel(channels[1].id).unwrap().id, channels[1].id);
    }
}

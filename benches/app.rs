use std::io::BufReader;

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use gurk::app::App;
use gurk::config::{Config, User};
use gurk::data::{AppData, Channel, ChannelId, GroupData, Message, TypingSet};
use gurk::dev::ContentBase64;
use gurk::signal::test::SignalManagerMock;
use gurk::signal::GroupMasterKeyBytes;
use gurk::storage::Storage;
use gurk::util::StatefulList;
use presage::prelude::Content;
use uuid::Uuid;

pub struct InMemoryStorage {}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self {}
    }
}

impl Storage for InMemoryStorage {
    fn save_app_data(&self, _data: &AppData) -> anyhow::Result<()> {
        Ok(())
    }

    fn load_app_data(&self) -> anyhow::Result<AppData> {
        Ok(Default::default())
    }
}

fn test_app() -> App {
    let signal_manager = SignalManagerMock::new();

    let mut app = App::try_new(
        Config {
            notifications: false,
            ..Config::with_user(User {
                name: "Tyler Durden".to_string(),
                phone_number: "+0000000000".to_string(),
            })
        },
        Box::new(signal_manager),
        Box::new(InMemoryStorage::new()),
    )
    .unwrap();

    app.data.channels.items.push(Channel {
        id: ChannelId::User(Uuid::new_v4()),
        name: "test".to_string(),
        group_data: Some(GroupData {
            master_key_bytes: GroupMasterKeyBytes::default(),
            members: vec![app.user_id],
            revision: 1,
        }),
        messages: StatefulList::with_items(vec![Message {
            from_id: app.user_id,
            message: Some("First message".to_string()),
            arrived_at: 0,
            quote: Default::default(),
            attachments: Default::default(),
            reactions: Default::default(),
            receipt: Default::default(),
        }]),
        unread_messages: 1,
        typing: TypingSet::GroupTyping(Default::default()),
    });
    app.data.channels.state.select(Some(0));

    app
}

pub fn bench_on_message(c: &mut Criterion) {
    use std::io::BufRead;

    let f = std::fs::File::open("messages.raw.json").unwrap();
    let reader = BufReader::new(f);
    let mut data = Vec::new();
    for line in reader.lines() {
        let line = line.unwrap();
        let content_base64: ContentBase64 = serde_json::from_str(&line).unwrap();
        let content = Content::try_from(content_base64).unwrap();
        data.push(content);
    }

    c.bench_function("on_message", move |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter_batched(
                || (test_app(), data.clone()),
                |(mut app, data)| async move {
                    for content in data {
                        app.on_message(content).await.unwrap();
                    }
                },
                BatchSize::SmallInput,
            )
    });
}

criterion_group!(benches, bench_on_message);
criterion_main!(benches);

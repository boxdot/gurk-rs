use std::io::{BufRead, BufReader};
use std::path::Path;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use gurk::app::App;
use gurk::config::{Config, User};
use gurk::dev::ContentBase64;
use gurk::signal::test::SignalManagerMock;
use gurk::storage::InMemoryStorage;
use presage::prelude::Content;
use tracing::info;

fn test_app() -> App {
    App::try_new(
        Config {
            notifications: false,
            ..Config::with_user(User {
                name: "Tyler Durden".to_string(),
                phone_number: "+0000000000".to_string(),
            })
        },
        Box::new(SignalManagerMock::new()),
        Box::new(InMemoryStorage::new()),
    )
    .unwrap()
}

pub fn bench_on_message(c: &mut Criterion) {
    let _ = tracing_subscriber::fmt::try_init();
    let data = read_input_data("messages.raw.json");
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

fn read_input_data(path: impl AsRef<Path>) -> Vec<Content> {
    let f = std::fs::File::open(path).unwrap();
    let reader = BufReader::new(f);
    let mut data = Vec::new();
    for line in reader.lines() {
        let line = line.unwrap();
        let content_base64: ContentBase64 = serde_json::from_str(&line).unwrap();
        let content = Content::try_from(content_base64).unwrap();
        data.push(content);
    }
    info!(n = %data.len(), "messages");
    data
}

criterion_group!(benches, bench_on_message);
criterion_main!(benches);

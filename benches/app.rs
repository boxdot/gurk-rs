use std::path::Path;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use gurk::app::App;
use gurk::config::{Config, User};
use gurk::signal::test::SignalManagerMock;
use gurk::storage::{ForgetfulStorage, MemCache};
use presage::libsignal_service::content::Content;
use tracing::info;

fn test_app() -> App {
    let (app, _) = App::try_new(
        Config {
            notifications: false,
            ..Config::with_user(User {
                name: "Tyler Durden".to_string(),
                phone_number: "+0000000000".to_string(),
            })
        },
        Box::new(SignalManagerMock::new()),
        Box::new(MemCache::new(ForgetfulStorage)),
    )
    .unwrap();
    app
}

pub fn bench_on_message(c: &mut Criterion) {
    let _ = tracing_subscriber::fmt::try_init();
    let path =
        std::env::var("RAW_MESSAGES_FILE").unwrap_or_else(|_| "messages.raw.json".to_string());
    let data = read_input_data(&path).expect("failed to read data");
    info!(n = data.len(), from = path, "messages");
    c.bench_function("on_message", move |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter_batched(
                || (test_app(), data.clone()),
                |(mut app, data)| async move {
                    for content in data {
                        app.on_message(Box::new(content)).await.unwrap();
                    }
                },
                BatchSize::SmallInput,
            )
    });
}

#[allow(unused_variables)]
fn read_input_data(path: impl AsRef<Path>) -> anyhow::Result<Vec<Content>> {
    #[cfg(feature = "dev")]
    {
        use std::io::{BufRead, BufReader};

        let f = std::fs::File::open(path)?;
        let reader = BufReader::new(f);
        let mut data = Vec::new();
        for line in reader.lines() {
            let line = line?;
            let content_base64: gurk::dev::ContentBase64 = serde_json::from_str(&line)?;
            let content = Content::try_from(content_base64)?;
            data.push(content);
        }
        Ok(data)
    }
    #[cfg(not(feature = "dev"))]
    {
        anyhow::bail!("failed to read data; please enable the cargo 'dev' feature");
    }
}

criterion_group!(benches, bench_on_message);
criterion_main!(benches);

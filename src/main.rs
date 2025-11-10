//! Signal Messenger client for terminal

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use clap::Parser;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event as CEvent, EventStream, KeyEvent,
        KeyEventKind, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use gurk::{app::App, config::Config};
use gurk::{backoff::Backoff, passphrase::Passphrase};
use gurk::{
    onboarding,
    storage::{MemCache, SqliteStorage, Storage, sync_from_signal},
};
use gurk::{signal, ui};
use presage::libsignal_service::content::Content;
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::{runtime, select};
use tokio_stream::StreamExt;
use tokio_util::task::LocalPoolHandle;
use tracing::debug;
use tracing::{error, info};
use url::Url;

const TARGET_FPS: u64 = 144;
const RECEIPT_TICK_PERIOD: u64 = 144;
const FRAME_BUDGET: Duration = Duration::from_millis(1000 / TARGET_FPS);
const RECEIPT_BUDGET: Duration = Duration::from_millis(RECEIPT_TICK_PERIOD * 1000 / TARGET_FPS);

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Enables logging to `gurk.log` in the current working directory according to RUST_LOG
    #[arg(short, long = "verbose")]
    verbosity: bool,
    /// Relinks the device (helpful when device was unlinked)
    #[arg(long)]
    relink: bool,
    /// Passphrase to use for encrypting the database
    ///
    /// When omitted, passphrase is read from the config file, passphrase_command, and if missing, prompted for.
    #[arg(long, short, conflicts_with = "passphrase_command")]
    passphrase: Option<Passphrase>,
    /// Get a passphrase from external command. For example `pass`(password-store)
    ///
    /// When omitted, passphrase_command is read from the env "GURK_PASSPHRASE_COMMAND".
    #[arg(long, conflicts_with = "passphrase")]
    passphrase_command: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let mut args = Args::parse();

    let _guard = if args.verbosity {
        let file_appender = tracing_appender::rolling::never("./", "gurk.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::fmt::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(non_blocking)
            .with_ansi(false)
            .init();
        Some(guard)
    } else {
        None
    };

    log_panics::init();

    let (config, passphrase) = match Config::load_installed().context("failed to load config")? {
        Some(config) => {
            let mut config = config.report_deprecated_keys();
            let passphrase = Passphrase::get(
                args.passphrase.take(),
                args.passphrase_command.take(),
                &mut config,
            )?;
            (config, passphrase)
        }
        None => onboarding::run()?,
    };

    let runtime = runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(run(config, passphrase, args.relink))
}

async fn is_online() -> bool {
    tokio::net::TcpStream::connect("detectportal.firefox.com:80")
        .await
        .is_ok()
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Event {
    Redraw,
    Click(MouseEvent),
    Input(KeyEvent),
    Paste(String),
    Message(Box<Content>),
    Resize { cols: u16, rows: u16 },
    Quit(Option<anyhow::Error>),
    ContactSynced(DateTime<Utc>),
    Tick,
    AppEvent(gurk::event::Event),
}

async fn run(config: Config, passphrase: Passphrase, relink: bool) -> anyhow::Result<()> {
    let local_pool = LocalPoolHandle::new(2);

    let mut signal_manager =
        signal::ensure_linked_device(relink, local_pool.clone(), &config, &passphrase).await?;

    let mut storage: Box<dyn Storage> = {
        let url = match config
            .sqlite
            .as_ref()
            .map(|sqlite_config| sqlite_config.url.clone())
        {
            Some(url) => url,
            None => Url::from_file_path(config.gurk_db_path())
                .map_err(|_| anyhow!("failed to convert gurk db path to url"))?,
        };

        debug!(%url, "opening sqlite data storage");
        let sqlite_storage = SqliteStorage::maybe_encrypt_and_open(&url, &passphrase, false)
            .await
            .with_context(|| format!("failed to open sqlite data storage at: {url}"))?;
        Box::new(MemCache::new(sqlite_storage))
    };

    sync_from_signal(&*signal_manager, &mut *storage).await;

    let (mut app, mut app_events) = App::try_new(config, signal_manager.clone_boxed(), storage)?;
    app.populate_names_cache().await;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Event>(100);
    tokio::spawn({
        let tx = tx.clone();
        async move {
            let mut reader = EventStream::new().fuse();
            while let Some(event) = reader.next().await {
                match event {
                    Ok(CEvent::Key(key)) => tx.send(Event::Input(key)).await.unwrap(),
                    Ok(CEvent::Resize(cols, rows)) => {
                        tx.send(Event::Resize { cols, rows }).await.unwrap()
                    }
                    Ok(CEvent::Mouse(button)) => tx.send(Event::Click(button)).await.unwrap(),
                    Ok(CEvent::Paste(content)) => tx.send(Event::Paste(content)).await.unwrap(),
                    _ => (),
                }
            }
        }
    });

    let inner_tx = tx.clone();

    local_pool.spawn_pinned(|| async move {
        let mut backoff = Backoff::new();
        loop {
            let mut messages = if !is_online().await {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                continue;
            } else {
                match signal_manager.receive_messages().await {
                    Ok(messages) => {
                        info!("connected and listening for incoming messages");
                        messages
                    }
                    Err(e) => {
                        let e = e.context(
                            "failed to initialize the stream of Signal messages.\n\
                            Maybe the device was unlinked? Please try to restart with \
                            '--relink` flag.",
                        );
                        inner_tx
                            .send(Event::Quit(Some(e)))
                            .await
                            .expect("logic error: events channel closed");
                        return;
                    }
                }
            };

            while let Some(message) = messages.next().await {
                backoff.reset();
                inner_tx
                    .send(Event::Message(message))
                    .await
                    .expect("logic error: events channel closed")
            }

            let after = backoff.get();
            error!(?after, "messages channel disconnected. trying to reconnect");
            tokio::time::sleep(after).await;
        }
    });

    enable_raw_mode()?;
    let _raw_mode_guard = scopeguard::guard((), |_| {
        disable_raw_mode().unwrap();
    });

    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut res = Ok(()); // result on quit
    let mut last_render_at = Instant::now();
    let is_render_spawned = Arc::new(AtomicBool::new(false));

    let tick_tx = tx.clone();
    // Tick to trigger receipt sending
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(RECEIPT_BUDGET);
        loop {
            interval.tick().await;
            tick_tx
                .send(Event::Tick)
                .await
                .expect("Cannot tick: events channel closed.");
        }
    });

    loop {
        // render
        let left_frame_budget = FRAME_BUDGET.checked_sub(last_render_at.elapsed());
        if let Some(budget) = left_frame_budget {
            // skip frames that render too fast
            if !is_render_spawned.load(Ordering::Relaxed) {
                let tx = tx.clone();
                let is_render_spawned = is_render_spawned.clone();
                is_render_spawned.store(true, Ordering::Relaxed);
                tokio::spawn(async move {
                    // Redraw message is needed to make sure that we render the skipped frame
                    // if it was the last frame in the rendering budget window.
                    tokio::time::sleep(budget).await;
                    tx.send(Event::Redraw)
                        .await
                        .expect("logic error: events channel closed");
                    is_render_spawned.store(false, Ordering::Relaxed);
                });
            }
        } else {
            terminal.draw(|f| ui::draw(f, &mut app))?;
            last_render_at = Instant::now();
        }

        let event = select! {
            v = rx.recv() => v,
            v = app_events.recv() => v.map(Event::AppEvent),
        };

        match event {
            Some(Event::Tick) => {
                app.step_receipts();
            }
            Some(Event::Click(event)) => match event.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    let col = event.column;
                    let row = event.row;
                    if let Some(channel_idx) =
                        ui::coords_within_channels_view(terminal.get_frame().area(), col, row)
                            .map(|(_, row)| row as usize)
                            .filter(|&idx| idx < app.channels.items.len())
                    {
                        app.channels.state.select(Some(channel_idx));
                        app.reset_unread_messages();
                    }
                }
                MouseEventKind::ScrollUp => {
                    if event.column
                        < terminal.get_frame().area().width / ui::CHANNEL_VIEW_RATIO as u16
                    {
                        app.select_previous_channel()
                    } else {
                        app.on_pgup()
                    }
                }
                MouseEventKind::ScrollDown => {
                    if event.column
                        < terminal.get_frame().area().width / ui::CHANNEL_VIEW_RATIO as u16
                    {
                        app.select_next_channel()
                    } else {
                        app.on_pgdn()
                    }
                }
                _ => {}
            },
            Some(Event::Input(
                event @ KeyEvent {
                    kind: KeyEventKind::Press,
                    ..
                },
            )) => app.on_key(event).await?,
            Some(Event::Input(..)) => {}
            Some(Event::Paste(content)) => {
                let multi_line_state = app.is_multiline_input;
                app.is_multiline_input = true;
                content.chars().for_each(|c| app.get_input().put_char(c));
                app.is_multiline_input = multi_line_state;
            }
            Some(Event::Message(content)) => {
                if let Err(e) = app.on_message(content).await {
                    error!("failed on incoming message: {}", e);
                }
            }
            Some(Event::Resize { .. }) | Some(Event::Redraw) => {
                // will just redraw the app
            }
            Some(Event::Quit(e)) => {
                if let Some(e) = e {
                    res = Err(e);
                };
                break;
            }
            Some(Event::ContactSynced(at)) => {
                let mut metadata = app.storage.metadata().into_owned();
                metadata.contacts_sync_request_at.replace(at);
                app.storage.store_metadata(metadata);
                info!(%at, "synced contacts");
            }
            Some(Event::AppEvent(event)) => {
                if let Err(error) = app.handle_event(event) {
                    error!(%error, "failed to handle app event");
                }
            }
            None => {
                break;
            }
        }

        if app.should_quit {
            break;
        }
    }

    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .unwrap();
    terminal.show_cursor().unwrap();

    res
}

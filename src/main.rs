//! Signal Messenger client for terminal

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use chrono::{DateTime, Utc};
use clap::Parser;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event as CEvent, EventStream, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use gurk::app::App;
use gurk::backoff::Backoff;
use gurk::storage::{sync_from_signal, JsonStorage, MemCache, SqliteStorage, Storage};
use gurk::{config, signal, ui};
use presage::libsignal_service::content::Content;
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::select;
use tokio_stream::StreamExt;
use tracing::debug;
use tracing::{error, info, metadata::LevelFilter};

const TARGET_FPS: u64 = 144;
const RECEIPT_TICK_PERIOD: u64 = 144;
const FRAME_BUDGET: Duration = Duration::from_millis(1000 / TARGET_FPS);
const SAVE_BUDGET: Duration = Duration::from_millis(1000);
const RECEIPT_BUDGET: Duration = Duration::from_millis(RECEIPT_TICK_PERIOD * 1000 / TARGET_FPS);

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Enables logging to `gurk.log` in the current working directory
    #[clap(short, long = "verbose", action = clap::ArgAction::Count)]
    verbosity: u8,
    /// Relinks the device (helpful when device was unlinked)
    #[clap(long)]
    relink: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let _guard = if args.verbosity > 0 {
        let file_appender = tracing_appender::rolling::never("./", "gurk.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::fmt::fmt()
            .with_max_level(match args.verbosity {
                0 => LevelFilter::OFF,
                1 => LevelFilter::INFO,
                2 => LevelFilter::DEBUG,
                _ => LevelFilter::TRACE,
            })
            .with_writer(non_blocking)
            .with_ansi(false)
            .init();
        Some(guard)
    } else {
        None
    };

    log_panics::init();

    tokio::task::LocalSet::new()
        .run_until(run_single_threaded(args.relink))
        .await
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
    Message(Content),
    Resize { cols: u16, rows: u16 },
    Quit(Option<anyhow::Error>),
    ContactSynced(DateTime<Utc>),
    Tick,
    AppEvent(gurk::event::Event),
}

async fn run_single_threaded(relink: bool) -> anyhow::Result<()> {
    let (mut signal_manager, config) = signal::ensure_linked_device(relink).await?;

    let mut storage: Box<dyn Storage> = if config.sqlite.enabled {
        debug!(
            %config.sqlite.url,
            encrypt = config.passphrase.is_some(),
            "opening sqlite"
        );
        let mut sqlite_storage = SqliteStorage::maybe_encrypt_and_open(
            &config.sqlite.url,
            config.passphrase.clone(),
            config.sqlite.preserve_unencrypted,
        )
        .with_context(|| {
            format!(
                "failed to open sqlite data storage at: {}",
                config.sqlite.url
            )
        })?;
        if sqlite_storage.is_empty() || !(sqlite_storage.metadata().fully_migrated.unwrap_or(false))
        {
            if let Ok(json_storage) =
                JsonStorage::new(&config.data_path, config::fallback_data_path().as_deref())
            {
                println!(
                    "converting JSON storage to SQLite storage at {}",
                    config.sqlite.url
                );
                let stats = sqlite_storage.copy_from(&json_storage).await?;
                let mut metadata = sqlite_storage.metadata().into_owned();
                metadata.fully_migrated = Some(true);
                sqlite_storage.store_metadata(metadata);
                info!(?stats, "converted");
            }
        }
        Box::new(MemCache::new(sqlite_storage))
    } else {
        let json_storage =
            JsonStorage::new(&config.data_path, config::fallback_data_path().as_deref())?;
        Box::new(json_storage)
    };

    sync_from_signal(&*signal_manager, &mut *storage);

    let (mut app, mut app_events) = App::try_new(config, signal_manager.clone_boxed(), storage)?;

    // sync task can be only spawned after we start to listen to message, because it relies on
    // message sender to be running
    let mut contact_sync_task = app.request_contacts_sync();

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
    tokio::task::spawn_local(async move {
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
                            Maybe the device was unlinked? Please try to restart with '--relink` flag.",
                        );
                        inner_tx
                            .send(Event::Quit(Some(e)))
                            .await
                            .expect("logic error: events channel closed");
                        return;
                    }
                }
            };

            if let Some(task) = contact_sync_task.take() {
                let inner_tx = inner_tx.clone();
                tokio::task::spawn_local(async move {
                    match task.await {
                        Ok(at) => inner_tx
                            .send(Event::ContactSynced(at))
                            .await
                            .expect("logic error: events channel closed"),
                        Err(error) => {
                            error!(%error, "failed to sync contacts");
                        }
                    }
                });
            }

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
    let mut last_save_at = Instant::now();
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
                        ui::coords_within_channels_view(terminal.get_frame().size(), col, row)
                            .map(|(_, row)| row as usize)
                            .filter(|&idx| idx < app.channels.items.len())
                    {
                        app.channels.state.select(Some(channel_idx));
                        app.reset_unread_messages();
                    }
                }
                MouseEventKind::ScrollUp => {
                    if event.column
                        < terminal.get_frame().size().width / ui::CHANNEL_VIEW_RATIO as u16
                    {
                        app.select_previous_channel()
                    } else {
                        app.on_pgup()
                    }
                }
                MouseEventKind::ScrollDown => {
                    if event.column
                        < terminal.get_frame().size().width / ui::CHANNEL_VIEW_RATIO as u16
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
            )) => match event.code {
                KeyCode::F(1u8) => {
                    // Toggle help panel
                    app.toggle_help();
                }
                KeyCode::Char('c') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    break;
                }
                KeyCode::Left => {
                    if event
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    {
                        app.get_input().move_back_word();
                    } else {
                        app.get_input().on_left();
                    }
                }
                KeyCode::Up if event.modifiers.contains(KeyModifiers::ALT) => app.on_pgup(),
                KeyCode::Right => {
                    if event
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    {
                        app.get_input().move_forward_word();
                    } else {
                        app.get_input().on_right();
                    }
                }
                KeyCode::Down if event.modifiers.contains(KeyModifiers::ALT) => app.on_pgdn(),
                KeyCode::Char('j') if event.modifiers.contains(KeyModifiers::ALT) => app.on_pgdn(),
                KeyCode::PageUp => app.on_pgup(),
                KeyCode::Char('k') if event.modifiers.contains(KeyModifiers::ALT) => app.on_pgup(),
                KeyCode::PageDown => app.on_pgdn(),
                KeyCode::Char('f') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.get_input().move_forward_word();
                }
                KeyCode::Char('b') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.get_input().move_back_word();
                }
                KeyCode::Char('u') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.get_input().on_delete_line();
                }
                KeyCode::Char('w') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.get_input().on_delete_word();
                }
                KeyCode::Down => {
                    if app.is_select_channel_shown() {
                        app.select_channel_next()
                    } else if app.is_multiline_input {
                        app.input.move_line_down();
                    } else {
                        app.select_next_channel();
                    }
                }
                KeyCode::Char('j') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    if app.is_select_channel_shown() {
                        app.select_channel_next()
                    } else if app.is_multiline_input {
                        app.input.move_line_down();
                    } else {
                        app.select_next_channel();
                    }
                }
                KeyCode::Char('y') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.copy_selection();
                }
                KeyCode::Up => {
                    if app.is_select_channel_shown() {
                        app.select_channel_prev()
                    } else if app.is_multiline_input {
                        app.input.move_line_up();
                    } else {
                        app.select_previous_channel();
                    }
                }
                KeyCode::Char('k') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    if app.is_select_channel_shown() {
                        app.select_channel_prev()
                    } else if app.is_multiline_input {
                        app.input.move_line_up();
                    } else {
                        app.select_previous_channel();
                    }
                }
                KeyCode::Backspace
                    if event
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    app.get_input().on_delete_word();
                }
                KeyCode::Char('k') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.get_input().on_delete_suffix();
                }
                _ => app.on_key(event).await?,
            },
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

        if last_save_at.elapsed() > SAVE_BUDGET || app.should_quit {
            app.storage.save();
            last_save_at = Instant::now();
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

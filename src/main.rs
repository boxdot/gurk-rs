//! Signal Messenger client for terminal

mod app;
mod config;
mod cursor;
mod data;
mod input;
mod receipt;
mod shortcuts;
mod signal;
mod storage;
mod storage2;
mod ui;
mod util;

use app::App;

use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event as CEvent, EventStream, KeyCode, KeyEvent,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use presage::prelude::Content;
use structopt::StructOpt;
use tokio_stream::StreamExt;
use tracing::{error, info, metadata::LevelFilter};
use tui::{backend::CrosstermBackend, Terminal};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::storage::JsonStorage;

const TARGET_FPS: u64 = 144;
const RECEIPT_TICK_PERIOD: u64 = 144;
const FRAME_BUDGET: Duration = Duration::from_millis(1000 / TARGET_FPS);
const RECEIPT_BUDGET: Duration = Duration::from_millis(RECEIPT_TICK_PERIOD * 1000 / TARGET_FPS);
const MESSAGE_SCROLL_BACK: bool = false;

#[derive(Debug, StructOpt)]
struct Args {
    /// Enables logging to `gurk.log` in the current working directory
    #[structopt(short, long = "verbose", parse(from_occurrences))]
    verbosity: u8,
    /// Relinks the device (helpful when device was unlinked)
    #[structopt(long)]
    relink: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::from_args();

    let file_appender = tracing_appender::rolling::never("./", "gurk.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_max_level(match args.verbosity {
            0 => LevelFilter::OFF,
            1 => LevelFilter::INFO,
            2 => LevelFilter::DEBUG,
            _ => LevelFilter::TRACE,
        })
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();

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
    Message(Content),
    Resize { cols: u16, rows: u16 },
    Quit(Option<anyhow::Error>),
    Tick,
}

async fn run_single_threaded(relink: bool) -> anyhow::Result<()> {
    let (signal_manager, config) = signal::ensure_linked_device(relink).await?;

    let storage = JsonStorage::new(config.data_path.clone(), config::fallback_data_path());
    let mut app = App::try_new(config, signal_manager.clone_boxed(), Box::new(storage))?;

    app.request_contacts_sync().await?;

    enable_raw_mode()?;
    let _raw_mode_guard = scopeguard::guard((), |_| {
        disable_raw_mode().unwrap();
    });

    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

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
                    _ => (),
                }
            }
        }
    });

    let backend = CrosstermBackend::new(stdout);

    let mut terminal = Terminal::new(backend)?;

    let inner_tx = tx.clone();
    tokio::task::spawn_local(async move {
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

            while let Some(message) = messages.next().await {
                inner_tx
                    .send(Event::Message(message))
                    .await
                    .expect("logic error: events channel closed")
            }
            info!("messages channel disconnected. trying to reconnect.")
        }
    });

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

        match rx.recv().await {
            Some(Event::Tick) => {
                let _ = app.step_receipts();
            }
            Some(Event::Click(event)) => match event.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    let col = event.column;
                    let row = event.row;
                    if let Some(channel_idx) =
                        ui::coords_within_channels_view(&terminal.get_frame(), &app, col, row)
                            .map(|(_, row)| row as usize)
                            .filter(|&idx| idx < app.data.channels.items.len())
                    {
                        app.data.channels.state.select(Some(channel_idx));
                        if app.reset_unread_messages() {
                            app.save().unwrap();
                        }
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
            Some(Event::Input(event)) => match event.code {
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
                KeyCode::PageUp => app.on_pgup(),
                KeyCode::PageDown => app.on_pgdn(),
                KeyCode::Tab if event.modifiers.contains(KeyModifiers::ALT) => app.toggle_search(),
                KeyCode::Char('f') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.get_input().move_forward_word();
                }
                KeyCode::Char('b') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.get_input().move_back_word();
                }
                KeyCode::Char('w') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.get_input().on_delete_word();
                }
                KeyCode::Down => {
                    if app.is_multiline_input {
                        app.input.move_line_down();
                    } else {
                        app.select_next_channel();
                    }
                }
                KeyCode::Char('j') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    if app.is_multiline_input {
                        app.input.move_line_down();
                    } else {
                        app.select_next_channel();
                    }
                }
                KeyCode::Up => {
                    if app.is_multiline_input {
                        app.input.move_line_up();
                    } else {
                        app.select_previous_channel();
                    }
                }
                KeyCode::Char('k') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    if app.is_multiline_input {
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
                _ => app.on_key(event)?,
            },
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

//! Signal Messenger client for terminal

mod app;
mod config;
mod signal;
mod ui;
mod util;

use app::{App, Event};

use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event as CEvent, EventStream, KeyCode,
        KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::{error, info};
use structopt::StructOpt;
use tokio_stream::StreamExt;
use tui::{backend::CrosstermBackend, Terminal};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const TARGET_FPS: u64 = 144;
const FRAME_BUDGET: Duration = Duration::from_millis(1000 / TARGET_FPS);
const MESSAGE_SCROLL_BACK: bool = false;

#[derive(Debug, StructOpt)]
struct Args {
    /// Enables logging to `gurk.log` in the current working directory
    #[structopt(short, long)]
    verbose: bool,
    /// Relinks the device (helpful when device was unlinked)
    #[structopt(long)]
    relink: bool,
}

fn init_file_logger() -> anyhow::Result<()> {
    use log::LevelFilter;
    use log4rs::append::file::FileAppender;
    use log4rs::config::{Appender, Config, Root};
    use log4rs::encode::pattern::PatternEncoder;

    let logfile = FileAppender::builder()
        .encoder(Box::new(PatternEncoder::new("[{d} {l} {M}] {m}\n")))
        .build("gurk.log")?;

    let config = Config::builder()
        .appender(Appender::builder().build("logfile", Box::new(logfile)))
        .build(Root::builder().appender("logfile").build(LevelFilter::Info))?;

    log4rs::init_config(config)?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::from_args();
    if args.verbose {
        init_file_logger()?;
    }
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

async fn run_single_threaded(relink: bool) -> anyhow::Result<()> {
    let mut app = App::try_new(relink).await?;

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

    let inner_manager = app.signal_manager.clone();
    let inner_tx = tx.clone();
    tokio::task::spawn_local(async move {
        loop {
            let messages = if !is_online().await {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                continue;
            } else {
                match inner_manager.receive_messages().await {
                    Ok(messages) => {
                        info!("connected and listening for incoming messages");
                        messages
                    }
                    Err(e) => {
                        let e = anyhow::Error::from(e).context(
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

            tokio::pin!(messages);
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
            Some(Event::Click(event)) => match event.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    let col = event.column;
                    let row = event.row;
                    if let Some(channel_idx) =
                        ui::coords_within_channels_view(&terminal.get_frame(), col, row)
                            .map(|(_, row)| row as usize)
                            .filter(|&idx| idx < app.data.channels.items.len())
                    {
                        app.data.channels.state.select(Some(channel_idx as usize));
                        if app.reset_unread_messages() {
                            app.save().unwrap();
                        }
                    }
                }
                MouseEventKind::ScrollUp => {
                    if event.column
                        < terminal.get_frame().size().width / ui::CHANNEL_VIEW_RATIO as u16
                    {
                        app.on_up()
                    } else {
                        app.on_pgup()
                    }
                }
                MouseEventKind::ScrollDown => {
                    if event.column
                        < terminal.get_frame().size().width / ui::CHANNEL_VIEW_RATIO as u16
                    {
                        app.on_down()
                    } else {
                        app.on_pgdn()
                    }
                }
                _ => {}
            },
            Some(Event::Input(event)) => match event.code {
                KeyCode::Char('c') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    break;
                }
                KeyCode::Left => {
                    if event
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    {
                        app.move_back_word();
                    } else {
                        app.on_left();
                    }
                }
                KeyCode::Up if event.modifiers.contains(KeyModifiers::ALT) => app.on_pgup(),
                KeyCode::Up => app.on_up(),
                KeyCode::Right => {
                    if event
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    {
                        app.move_forward_word();
                    } else {
                        app.on_right();
                    }
                }
                KeyCode::Down if event.modifiers.contains(KeyModifiers::ALT) => app.on_pgdn(),
                KeyCode::Down => app.on_down(),
                KeyCode::PageUp => app.on_pgup(),
                KeyCode::PageDown => app.on_pgdn(),
                KeyCode::Char('f') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.move_forward_word();
                }
                KeyCode::Char('b') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.move_back_word();
                }
                KeyCode::Char('a') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.on_home();
                }
                KeyCode::Char('e') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.on_end();
                }
                KeyCode::Char('w') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.on_delete_word();
                }
                KeyCode::Backspace
                    if event
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    app.on_delete_word();
                }
                KeyCode::Char('k') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.on_delete_suffix();
                }
                KeyCode::Char('1') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.select_channel(0);
                }
                KeyCode::Char('2') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.select_channel(1);
                }
                KeyCode::Char('3') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.select_channel(2);
                }
                KeyCode::Char('4') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.select_channel(3);
                }
                KeyCode::Char('5') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.select_channel(4);
                }
                KeyCode::Char('6') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.select_channel(5);
                }
                KeyCode::Char('7') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.select_channel(6);
                }
                KeyCode::Char('8') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.select_channel(7);
                }
                KeyCode::Char('9') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.select_channel(8);
                }
                KeyCode::Char('0') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.select_channel(9);
                }
                code => app.on_key(code).await,
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

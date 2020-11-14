//! Signal Messenger client for terminal

mod app;
mod config;
mod signal;
mod ui;
mod util;

use app::{App, Event};

use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event as CEvent, EventStream, KeyCode, KeyEvent,
        KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use structopt::StructOpt;
use tokio::stream::StreamExt;
use tui::{backend::CrosstermBackend, Terminal};

use std::io::Write;

#[derive(Debug, StructOpt)]
struct Args {
    /// Enable logging to `gurg.log` in the current working directory.
    #[structopt(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::from_args();

    let mut app = App::try_new(args.verbose)?;

    enable_raw_mode()?;
    let _raw_mode_guard = scopeguard::guard((), |_| {
        disable_raw_mode().unwrap();
    });

    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    tokio::spawn({
        let mut tx = tx.clone();
        async move {
            let mut reader = EventStream::new().fuse();
            while let Some(event) = reader.next().await {
                match event {
                    Ok(CEvent::Key(key)) => tx.send(Event::Input(key)).await.unwrap(),
                    Ok(CEvent::Resize(_, _)) => tx.send(Event::Resize).await.unwrap(),
                    _ => (),
                }
            }
        }
    });

    let backend = CrosstermBackend::new(stdout);

    let mut terminal = Terminal::new(backend)?;

    let local = tokio::task::LocalSet::new();
    let signal_client = signal::SignalClient::from_config(app.config.clone());
    let (mut tx, mut rx) = tokio::sync::mpsc::channel::<Event<KeyEvent>>(32);
    local.spawn_local(async move {
        let mut messages = signal_client.stream_messages();
        for msg in messages.next().await {
            tx.send(Event::Message(msg)).await.unwrap();
        }
    });

    terminal.clear()?;

    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;
        match rx.recv().await {
            Some(Event::Input(event)) => match event.code {
                KeyCode::Char('c') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    break;
                }
                KeyCode::Left => app.on_left(),
                KeyCode::Up => app.on_up(),
                KeyCode::Right => app.on_right(),
                KeyCode::Down => app.on_down(),
                code => app.on_key(code),
            },
            Some(Event::Message(message)) => {
                app.on_message(message).await;
            }
            Some(Event::Resize) => {
                // will just redraw the app
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

    Ok(())
}

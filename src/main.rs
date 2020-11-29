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
        KeyModifiers, MouseEvent,
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

    app.data.chanpos.downside = terminal.get_frame().size().height - 3;

    let signal_client = signal::SignalClient::from_config(app.config.clone());
    tokio::spawn(async move { signal_client.stream_messages(tx).await });

    terminal.clear()?;

    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;
        match rx.recv().await {
            Some(Event::Click(event)) => match event {
                MouseEvent::Down(_, col, row, _) => match row {
                    row if row <= 0 => {}
                    row if row >= terminal.get_frame().size().height - 1 => {}
                    _ => {
                        let jump = row as usize - 1;
                        app.data
                            .channels
                            .state
                            .select(Some(app.data.chanpos.top + jump));
                    }
                },
                _ => {}
            },
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
            Some(Event::Message { payload, message }) => {
                app.on_message(message, payload).await;
            }
            Some(Event::Resize { cols, rows }) => match rows {
                // terminal height decreased
                rows if rows < terminal.get_frame().size().height => {
                    // viewport shrinks at the top
                    if app.data.chanpos.downside == 0 {
                        app.data.chanpos.top += 1;
                        app.data.chanpos.upside -= 1;
                    // viewport shrinks at the bottom
                    } else {
                        app.data.chanpos.downside -= 1;
                    }
                }
                // terminal height increased, viewport grows at the bottom
                rows if rows > terminal.get_frame().size().height => {
                    app.data.chanpos.downside += 1;
                }
                // will just redraw the app
                _ => {}
            },
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

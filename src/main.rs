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
                        if col < terminal.get_frame().size().width / 4 {
                            let target = app.data.chanpos.top + row as usize - 1;
                            if target < app.data.channels.items.len() {
                                app.data.channels.state.select(Some(target));
                                app.data.chanpos.upside =
                                    target as u16 - app.data.chanpos.top as u16;
                                app.data.chanpos.downside = terminal.get_frame().size().height
                                    - app.data.chanpos.upside
                                    - 3;
                            }
                        }
                    }
                },
                MouseEvent::ScrollUp(col, _, _) => match col {
                    col if col < terminal.get_frame().size().width / 4 => app.on_up(),
                    _ => {}
                },
                MouseEvent::ScrollDown(col, _, _) => match col {
                    col if col < terminal.get_frame().size().width / 4 => app.on_down(),
                    _ => {}
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
            Some(Event::Resize { cols: _, rows }) => match rows {
                // terminal too narrow for mouse navigation
                rows if rows < 3 => {}
                // terminal height decreased
                rows if rows < terminal.get_frame().size().height => {
                    let diff = terminal.get_frame().size().height - rows;
                    // decrease of one row
                    if diff == 1 {
                        // viewport shrinks at the top
                        if app.data.chanpos.downside == 0 {
                            app.data.chanpos.top += diff as usize;
                            if app.data.chanpos.upside > 0 {
                                app.data.chanpos.upside -= diff;
                            }
                        // viewport shrinks at the bottom
                        } else {
                            app.data.chanpos.downside -= diff;
                        }
                    // decrease of more than one row
                    } else {
                        // viewport shrinks at the bottom
                        if app.data.chanpos.downside >= diff {
                            app.data.chanpos.downside -= diff;
                        // viewport shrinks at the (bottom and then) top
                        } else {
                            let shorten = diff - app.data.chanpos.downside;
                            app.data.chanpos.downside = 0;
                            if app.data.chanpos.upside as i16 - shorten as i16 >= 0 {
                                app.data.chanpos.upside -= shorten;
                            }
                            app.data.chanpos.top += shorten as usize;
                        }
                    }
                }
                // terminal height increased, viewport grows at the bottom
                rows if rows > terminal.get_frame().size().height => {
                    let diff = rows - terminal.get_frame().size().height;
                    app.data.chanpos.downside += diff;
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

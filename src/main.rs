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
use log::error;
use structopt::StructOpt;
use tokio_stream::StreamExt;
use tui::{backend::CrosstermBackend, Terminal};

use std::io::Write;

#[derive(Debug, StructOpt)]
struct Args {
    /// Enable logging to `gurg.log` in the current working directory.
    #[structopt(short, long)]
    verbose: bool,
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

    let mut app = App::try_new()?;

    enable_raw_mode()?;
    let _raw_mode_guard = scopeguard::guard((), |_| {
        disable_raw_mode().unwrap();
    });

    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
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

    app.data.chanpos.downside = terminal.get_frame().size().height - 3;

    let signal_client = signal::SignalClient::from_config(app.config.clone());
    tokio::spawn(async move {
        // load data from signal asynchronously
        let remote_channels = tokio::task::spawn_blocking({
            let client = signal_client.clone();
            move || {
                app::AppData::init_from_signal(&client)
                    .map(|remote_data| remote_data.channels.items)
            }
        })
        .await;

        match remote_channels {
            Ok(Ok(remote)) => {
                let _ = tx.send(Event::Channels { remote }).await;
            }
            Ok(Err(e)) => error!("failed to load channel from server: {}", e),
            Err(e) => unreachable!(e.to_string()),
        }

        // listen to incoming messages
        signal_client.stream_messages(tx).await
    });

    terminal.clear()?;

    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;
        match rx.recv().await {
            Some(Event::Click(event)) => match event {
                MouseEvent::Down(_, col, row, _) => match row {
                    row if row == 0 => {}
                    row if row >= terminal.get_frame().size().height - 1 => {}
                    _ => {
                        if col < terminal.get_frame().size().width / 4 {
                            let target = app.data.chanpos.top + row as usize - 1;
                            if target < app.data.channels.items.len() {
                                if app.reset_unread_messages() {
                                    app.save().unwrap();
                                }
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
                    col if col > terminal.get_frame().size().width / 4 => app.on_pgup(),
                    _ => {}
                },
                MouseEvent::ScrollDown(col, _, _) => match col {
                    col if col < terminal.get_frame().size().width / 4 => app.on_down(),
                    col if col > terminal.get_frame().size().width / 4 => app.on_pgdn(),
                    _ => {}
                },
                _ => {}
            },
            Some(Event::Input(event)) => match event.code {
                KeyCode::Char('c') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    break;
                }
                KeyCode::Left => {
                    app.on_left();
                }
                KeyCode::Up => app.on_up(),
                KeyCode::Right => {
                    app.on_right();
                }
                KeyCode::Down => app.on_down(),
                KeyCode::PageUp => app.on_pgup(),
                KeyCode::PageDown => app.on_pgdn(),
                KeyCode::Char('f') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.on_alt_right();
                }
                KeyCode::Char('b') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.on_alt_left();
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
                KeyCode::Char('\u{7f}') if event.modifiers.contains(KeyModifiers::ALT) => {
                    app.on_delete_word();
                }
                KeyCode::Char('k') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.on_delete_suffix();
                }
                code => app.on_key(code),
            },
            Some(Event::Message { payload, message }) => {
                app.on_message(message, payload).await;
            }
            Some(Event::Channels { remote }) => app.on_channels(remote),
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

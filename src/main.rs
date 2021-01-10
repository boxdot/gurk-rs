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
        KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::info;
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

#[actix_rt::main(?Send)]
async fn main() -> anyhow::Result<()> {
    let args = Args::from_args();
    if args.verbose {
        init_file_logger()?;
    }

    // events channel
    let (mut tx, mut rx) = tokio::sync::mpsc::channel(100);

    let mut app = App::try_new(tx.clone())?;

    enable_raw_mode()?;
    let _raw_mode_guard = scopeguard::guard((), |_| {
        disable_raw_mode().unwrap();
    });

    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    actix_rt::spawn({
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
    let mut terminal = RestoreGuard(Terminal::new(backend)?);

    let signal_client = app.signal_client.clone();
    tokio::task::spawn_local(async move {
        info!("Listening for incoming signal messages");
        let mut messages = signal_client.stream_messages();
        while let Some(msg) = messages.next().await {
            tx.send(Event::Message(msg))
                .await
                .expect("logic error: main loop closed");
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
                code => app.on_key(code).await,
            },
            Some(Event::Message(message)) => {
                if let Err(e) = app.on_message(message?).await {
                    log::info!("skipping message due to: {}", e);
                }
            }
            Some(Event::Resize) => {
                // will just redraw the app
            }
            Some(Event::Error(e)) => {
                log::error!("error: {}", e);
            }
            None => {
                break;
            }
        }
        if app.should_quit {
            break;
        }
    }

    Ok(())
}

struct RestoreGuard(Terminal<CrosstermBackend<std::io::Stdout>>);

impl std::ops::Deref for RestoreGuard {
    type Target = Terminal<CrosstermBackend<std::io::Stdout>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for RestoreGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for RestoreGuard {
    fn drop(&mut self) {
        if let Err(e) = execute!(
            self.0.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        ) {
            eprintln!("failed to restore screen: {}", e);
        }
        self.0.show_cursor().unwrap();
    }
}

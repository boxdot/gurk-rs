mod app;
mod config;
mod signal;
mod ui;
mod util;

use app::{App, Event};
use tokio::stream::StreamExt;

use anyhow::Context;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event as CEvent, EventStream, KeyCode,
        KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use structopt::StructOpt;
use tui::{backend::CrosstermBackend, Terminal};

use std::fs::File;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, StructOpt)]
struct Args {
    #[structopt(subcommand)]
    cmd: Option<Command>,
}

#[derive(Debug, StructOpt)]
enum Command {
    TestModel {
        #[structopt(short, long)]
        path: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::from_args();
    if let Some(Command::TestModel { path }) = args.cmd {
        // do model testing
        return test_model(path);
    }

    let mut app = App::try_new()?;

    enable_raw_mode()?;

    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    tokio::spawn({
        let mut tx = tx.clone();
        async move {
            let mut reader = EventStream::new().fuse();
            while let Some(event) = reader.next().await {
                if let Ok(CEvent::Key(key)) = event {
                    tx.send(Event::Input(key)).await.unwrap();
                }
            }
        }
    });

    let backend = CrosstermBackend::new(stdout);

    let mut terminal = Terminal::new(backend)?;

    let signal_client = signal::SignalClient::from_config(app.config.clone());
    tokio::spawn(async move { signal_client.stream_messages(tx).await });

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
            Some(Event::Message { payload, message }) => {
                app.on_message(message, payload);
            }
            None => {
                break;
            }
        }
        if app.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

fn test_model(path: impl AsRef<Path>) -> anyhow::Result<()> {
    let f = File::open(path)?;
    for (line_number, json_line) in io::BufReader::new(f).lines().enumerate() {
        let json_line = json_line?;
        if json_line.trim().is_empty() {
            continue;
        }
        let msg: signal::Message = serde_json::from_str(&json_line).context(format!(
            "failed to parse line {}: '{}'",
            line_number + 1,
            json_line
        ))?;
        println!("{:?}", msg);
    }
    Ok(())
}

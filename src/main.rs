mod app;
mod appdata;
mod jami;
mod ui;
mod util;

use crate::jami::Jami;
use crate::util::*;
use app::App;

use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event as CEvent, EventStream, KeyCode,
        KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use structopt::StructOpt;
use tokio::stream::StreamExt;
use tui::{backend::CrosstermBackend, Terminal};

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, StructOpt)]
struct Args {
    #[structopt(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::from_args();

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

    let stop = Arc::new(AtomicBool::new(false));
    let stop_cloned = stop.clone();

    tokio::spawn(async move { Jami::handle_events(tx, stop_cloned).await });

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = App::try_new(args.verbose)?;
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
            Some(Event::Message {
                account_id,
                conversation_id,
                payloads,
            }) => {
                app.on_message(&account_id, &conversation_id, payloads)
                    .await;
            }
            Some(Event::Resize) => {
                // will just redraw the app
            }
            Some(Event::RegistrationStateChanged(account_id, registration_state)) => {
                app.on_registration_state_changed(&account_id, &registration_state)
                    .await;
            }
            Some(Event::ConversationReady(account_id, conversation_id)) => {
                app.on_conversation_ready(account_id, conversation_id).await;
            }
            Some(Event::ConversationRemoved(account_id, conversation_id)) => {
                app.on_conversation_removed(account_id, conversation_id).await;
            }
            Some(Event::ConversationRequest(account_id, conversation_id)) => {
                app.on_conversation_request(account_id, conversation_id)
                    .await;
            }
            Some(Event::RegisteredNameFound(account_id, status, address, name)) => {
                app.on_registered_name_found(account_id, status, address, name)
                    .await;
            }
            Some(Event::ConversationLoaded(id, account_id, conversation_id, messages)) => {
                app.on_conversation_loaded(id, account_id, conversation_id, messages)
                    .await;
            }
            Some(Event::ProfileReceived(account_id, from, path)) => {
                app.on_profile_received(&account_id, &from, &path).await;
            }
            Some(Event::IncomingTrustRequest(account_id, from, payload, receive_time)) => {
                app.on_incoming_trust_request(&account_id, &from, payload, receive_time)
                    .await;
            }
            Some(Event::AccountsChanged()) => {
                app.on_accounts_changed().await;
            }
            None => {
                break;
            }
        }
        if app.should_quit {
            break;
        }
    }

    // Stop handle_events
    stop.store(true, Ordering::Relaxed);

    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .unwrap();
    terminal.show_cursor().unwrap();

    Ok(())
}

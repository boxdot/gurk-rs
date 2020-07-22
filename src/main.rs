mod ui;
mod util;

use util::StatefulList;

use chrono::{DateTime, Utc};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyCode, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use tui::{backend::CrosstermBackend, Terminal};

use std::io::Write;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

pub struct App {
    username: String,
    channels: StatefulList<Channel>,
    current_chat: Chat,
    input: String,
    should_quit: bool,
}

#[derive(Debug)]
struct Channel {
    name: String,
    is_group: bool,
    last_msg: Option<String>,
}

impl Channel {
    fn new(name: impl Into<String>, is_group: bool) -> Self {
        Self {
            name: name.into(),
            is_group,
            last_msg: None,
        }
    }
}

struct Chat {
    msgs: StatefulList<Message>,
}

#[derive(Debug)]
struct Message {
    from: String,
    text: String,
    arrived_at: DateTime<Utc>,
}

enum Event<I> {
    Input(I),
    Tick,
}

impl App {
    fn new() -> Self {
        let now = Utc::now();
        let sample_chat = Chat {
            msgs: StatefulList::with_items(vec![
                Message {
                    from: "Bob".to_string(),
                    text: "Lorem ipsum dolor sit amet,  consectetur adipisicing elit, sed  do"
                        .to_string(),
                    arrived_at: now,
                },
                Message {
                    from: "Bob".to_string(),
                    text: "eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad"
                        .to_string(),
                    arrived_at: now,
                },
                Message {
                    from: "Bob".to_string(),
                    text: "minim veniam, quis nostrud exercitation ullamco laboris nisi ut"
                        .to_string(),
                    arrived_at: now,
                },
                Message {
                    from: "Bob".to_string(),
                    text: "aliquip ex ea commodo consequat. Duis aute irure dolor in".to_string(),
                    arrived_at: now,
                },
                Message {
                    from: "Bob".to_string(),
                    text: "reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla"
                        .to_string(),
                    arrived_at: now,
                },
                Message {
                    from: "Bob".to_string(),
                    text: "pariatur. Excepteur sint occaecat cupidatat non proident, sunt in"
                        .to_string(),
                    arrived_at: now,
                },
                Message {
                    from: "Bob".to_string(),
                    text: "culpa qui officia deserunt mollit anim id est laborum.".to_string(),
                    arrived_at: now,
                },
            ]),
        };

        let mut channels = StatefulList::with_items(vec![
            Channel::new("Basic people", true),
            Channel::new("Flat earth society", true),
            Channel::new("Don't burn", true),
            Channel::new("Small 🦙", true),
            Channel::new("Alice", false),
            Channel::new("Bob", false),
            Channel::new("Note to Self", false),
        ]);
        channels.state.select(Some(0));

        Self {
            username: "boxdot".to_string(),
            channels,
            current_chat: sample_chat,
            input: String::new(),
            should_quit: false,
        }
    }

    fn on_key(&mut self, k: KeyCode) {
        match k {
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            KeyCode::Char(c) => {
                self.input.push(c);
            }
            KeyCode::Enter if !self.input.is_empty() => {
                self.current_chat.msgs.items.push(Message {
                    from: self.username.clone(),
                    text: self.input.drain(..).collect(),
                    arrived_at: Utc::now(),
                });
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            _ => {}
        }
    }

    pub fn on_up(&mut self) {
        self.channels.previous();
    }

    pub fn on_down(&mut self) {
        self.channels.next();
    }

    pub fn on_right(&mut self) {
        // self.tabs.next();
    }

    pub fn on_left(&mut self) {
        // self.tabs.previous();
    }

    pub fn on_tick(&mut self) {}
}

fn main() -> anyhow::Result<()> {
    enable_raw_mode()?;

    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);

    let mut terminal = Terminal::new(backend)?;

    // Setup input handling
    let (tx, rx) = mpsc::channel();

    let tick_rate = Duration::from_millis(250);
    thread::spawn(move || {
        let mut last_tick = Instant::now();
        loop {
            // poll for tick rate duration, if no events, sent tick event.
            if event::poll(tick_rate - last_tick.elapsed()).unwrap() {
                if let CEvent::Key(key) = event::read().unwrap() {
                    tx.send(Event::Input(key)).unwrap();
                }
            }
            if last_tick.elapsed() >= tick_rate {
                tx.send(Event::Tick).unwrap();
                last_tick = Instant::now();
            }
        }
    });

    let mut app = App::new();

    terminal.clear()?;

    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;
        match rx.recv()? {
            Event::Input(event) => match event.code {
                KeyCode::Char('c') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    disable_raw_mode()?;
                    execute!(
                        terminal.backend_mut(),
                        LeaveAlternateScreen,
                        DisableMouseCapture
                    )?;
                    terminal.show_cursor()?;
                    break;
                }
                KeyCode::Left => app.on_left(),
                KeyCode::Up => app.on_up(),
                KeyCode::Right => app.on_right(),
                KeyCode::Down => app.on_down(),
                code => app.on_key(code),
            },
            Event::Tick => {
                app.on_tick();
            }
        }
        if app.should_quit {
            break;
        }
    }

    Ok(())
}

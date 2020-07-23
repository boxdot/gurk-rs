use crate::util::StatefulList;

use chrono::{DateTime, Utc};
use crossterm::event::KeyCode;

pub struct App {
    pub username: String,
    pub channels: StatefulList<Channel>,
    pub current_chat: Chat,
    pub input: String,
    pub should_quit: bool,
}

#[derive(Debug)]
pub struct Channel {
    pub name: String,
    pub is_group: bool,
    pub last_msg: Option<String>,
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

pub struct Chat {
    pub msgs: StatefulList<Message>,
}

#[derive(Debug)]
pub struct Message {
    pub from: String,
    pub text: String,
    pub arrived_at: DateTime<Utc>,
}

pub enum Event<I> {
    Input(I),
    Tick,
}

impl App {
    pub fn new() -> Self {
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

    pub fn on_key(&mut self, k: KeyCode) {
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

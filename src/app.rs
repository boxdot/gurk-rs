use crate::config::{self, Config};
use crate::signal;
use crate::util::StatefulList;

use anyhow::Context;
use chrono::{DateTime, Utc};
use crossterm::event::KeyCode;

use std::fs::File;
use std::io::Write;

pub struct App {
    pub config: Config,
    pub channels: StatefulList<Channel>,
    pub input: String,
    pub should_quit: bool,
    pub log_file: File,
}

#[derive(Debug)]
pub struct Channel {
    pub name: String,
    pub group_info: Option<signal::GroupInfo>,
    pub messages: Vec<Message>,
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
    pub fn try_new() -> anyhow::Result<Self> {
        let config_path = config::installed_config().expect("missing default location for config");
        let config = config::load_from(&config_path)
            .with_context(|| format!("failed to read config from: {}", config_path.display()))?;

        let client = signal::SignalClient::from_config(config.clone());
        let groups = client
            .get_groups()
            .context("failed to get groups from signal")?;
        let channels = groups.into_iter().map(|group_info| {
            let name = group_info
                .name
                .as_ref()
                .unwrap_or_else(|| &group_info.group_id)
                .to_string();
            Channel {
                name,
                group_info: Some(group_info),
                messages: Vec::new(),
            }
        });

        let mut channels = StatefulList::with_items(channels.collect());
        if !channels.items.is_empty() {
            channels.state.select(Some(0));
        }

        Ok(Self {
            config,
            channels,
            input: String::new(),
            should_quit: false,
            log_file: File::create("gurk.log").unwrap(),
        })
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
                if let Some(idx) = self.channels.state.selected() {
                    let channel = &mut self.channels.items[idx];
                    channel.messages.push(Message {
                        from: self.config.user.name.clone(),
                        text: self.input.drain(..).collect(),
                        arrived_at: Utc::now(),
                    })
                }
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

    #[allow(dead_code)]
    pub fn log(&mut self, msg: impl AsRef<str>) {
        writeln!(&mut self.log_file, "{}", msg.as_ref()).unwrap();
    }
}

use crate::account::Account;
use crate::util::StatefulList;
use crate::jami::Jami;

use chrono::{DateTime, Utc};
use crossterm::event::KeyCode;
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthStr;

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, Write};
use std::path::Path;

pub struct App {
    pub should_quit: bool,
    pub log_file: Option<File>,
    pub data: AppData,
    pub jami: Jami,
}

#[derive(Serialize, Deserialize)]
pub struct AppData {
    pub channels: StatefulList<Channel>,
    pub hash2name: HashMap<String, String>,
    pub account: Account,
    pub out_invite: Vec<OutgoingInvite>,
    pub input: String,
    #[serde(skip)]
    pub input_cursor: usize,
}

#[derive(Serialize, Deserialize)]
pub struct OutgoingInvite {
    pub account: String,
    pub channel: String,
    pub member: String,
}

impl AppData {
    fn save(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let f = File::create(path)?;
        serde_json::to_writer(f, self)?;
        Ok(())
    }

    fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let f = File::open(path)?;
        let mut data: Self = serde_json::from_reader(f)?;
        data.input_cursor = data.input.width();
        Ok(data)
    }

    // Move to jami namespace
    fn select_jami_account() -> Account {
        let accounts = Jami::get_account_list();
        // Select first enabled account
        for account in &accounts {
            if account.enabled {
                return account.clone();
            }
        }
        // No valid account found, generate a new one
        Jami::add_account("", "", false);
        return Account::null();
    }

    fn lookup_members(&mut self) {
        // Refresh titles for channel
        for channel in &mut *self.channels.items {
                println!("@@@ {:?}", channel.members);
            for member in &*channel.members {
                println!("@@@");
                Jami::lookup_name(&self.account.id, &String::new(), &member.hash);
            }
        }
    }

    fn channels_for_account(account: &Account) -> Vec<Channel> {
        let mut channels = Vec::new();
        let mut messages = Vec::new();

        // TODO move out welcome
        let file = File::open("rsc/welcome-art");
        if file.is_ok() {
            for line in io::BufReader::new(file.unwrap()).lines() {
                messages.push(Message {
                    from: String::new(),
                    message: Some(String::from(line.unwrap())),
                    arrived_at: Utc::now(),
                });
            }
        }

        channels.push(Channel {
            id: String::from("Welcome"),
            name: String::from("Welcome"),
            members: Vec::new(),
            is_group: false,
            messages,
            unread_messages: 0,
        });
        
        for conversation in Jami::get_conversations(&account.id) {
            let members_from_daemon = Jami::get_members(&account.id, &conversation);
            let mut members = Vec::new();
            for member in members_from_daemon {
                let role : Role;
                if member["role"].to_string() == "admin" {
                    role = Role::Admin;
                } else {
                    role = Role::Member;
                }
                let hash = member["uri"].to_string();
                members.push(Member {
                    hash,
                    role,
                })
            }
            channels.push(Channel {
                id: conversation.clone(),
                name: conversation,
                members: Vec::new(),
                is_group: true,
                messages: Vec::new(),
                unread_messages: 0,
            });
        }
        channels
    }

    fn init_from_jami() -> anyhow::Result<Self> {
        let account = AppData::select_jami_account();
        let mut channels = Vec::new();
        if !account.id.is_empty() {
            channels = AppData::channels_for_account(&account);
        }

        let mut channels = StatefulList::with_items(channels);
        if !channels.items.is_empty() {
            channels.state.select(Some(0));
        }

        Ok(AppData {
            channels,
            input: String::new(),
            hash2name: HashMap::new(),
            out_invite: Vec::new(),
            input_cursor: 0,
            account,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Role {
    Member,
    Admin
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Member {
    hash: String,
    role: Role, // TODO enum
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Channel {
    /// Either phone number or group id
    pub id: String,
    pub name: String,
    pub is_group: bool,
    pub members: Vec<Member>,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub unread_messages: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub from: String,
    #[serde(alias = "text")] // remove
    pub message: Option<String>,
    pub arrived_at: DateTime<Utc>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Event<I> {
    Input(I),
    Message {
        account_id: String,
        conversation_id: String,
        payloads: HashMap<String, String>
    },
    ConversationReady(String, String),
    RegistrationStateChanged(String, String),
    RegisteredNameFound(String, u64, String, String),
    Resize,
}

impl App {

    pub fn try_new(verbose: bool) -> anyhow::Result<Self> {
        let log_file = if verbose {
            Some(File::create("jami-cli.log").unwrap())
        } else {
            None
        };
        let mut data = AppData::init_from_jami()?;
        data.lookup_members();
        if data.channels.state.selected().is_none() && !data.channels.items.is_empty() {
            data.channels.state.select(Some(0));
        }

        let jami = Jami::init().unwrap();

        Ok(Self {
            data,
            should_quit: false,
            log_file,
            jami,
        })
    }


    pub fn on_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char(c) => {
                let mut idx = self.data.input_cursor;
                while !self.data.input.is_char_boundary(idx) {
                    idx += 1;
                }
                self.data.input.insert(idx, c);
                self.data.input_cursor += 1;
            }
            KeyCode::Enter if !self.data.input.is_empty() => {
                if let Some(idx) = self.data.channels.state.selected() {
                    self.send_input(idx)
                }
            }
            KeyCode::Backspace => {
                if self.data.input_cursor > 0
                    && self.data.input_cursor < self.data.input.width() + 1
                {
                    self.data.input_cursor = self.data.input_cursor.saturating_sub(1);
                    let idx = self
                        .data
                        .input
                        .chars()
                        .take(self.data.input_cursor)
                        .map(|c| c.len_utf8())
                        .sum();
                    self.data.input.remove(idx);
                }
            }
            _ => {}
        }
    }

    pub fn on_registration_state_changed(&mut self, _account_id: &String, registration_state: &String) {
        if registration_state == "REGISTERED" && self.data.account == Account::null() {
            self.data.account = AppData::select_jami_account();
        }
    }

    fn send_input(&mut self, channel_idx: usize) {
        let channel = &mut self.data.channels.items[channel_idx];

        let message: String = self.data.input.drain(..).collect();
        self.data.input_cursor = 0;

        if message == "/exit" {
            self.should_quit = true;
            return;
        }

        let mut show_msg = true;

        // TODO enum
        if !channel.is_group {
            if message == "/new" {
                Jami::start_conversation(&self.data.account.id);
            }
            if message == "/list" {
                for account in Jami::get_account_list() {
                    channel.messages.push(Message {
                        from: String::new(),
                        message: Some(String::from(format!("{}", account))),
                        arrived_at: Utc::now(),
                    });
                }
            } else if message.starts_with("/switch ") {
                let account_id = String::from(message.strip_prefix("/switch ").unwrap());
                let account = Jami::get_account(&*account_id);
                if account.id.is_empty() {
                    channel.messages.push(Message {
                        from: String::new(),
                        message: Some(String::from("Invalid account id.")),
                        arrived_at: Utc::now(),
                    });
                } else {
                    self.data.account = account;
                    let channels = AppData::channels_for_account(&self.data.account);
                    self.data.channels = StatefulList::with_items(channels);
                    if !self.data.channels.items.is_empty() {
                        self.data.channels.state.select(Some(0));
                    }
                    self.data.lookup_members();
                }
            } else if message == "/help" {
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/help: Show this help")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/new: start a new conversation")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/list: list accounts")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/switch <id>: switch to an account")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/exit: quit")),
                    arrived_at: Utc::now(),
                });
            }
        } else {
            // TODO simplify
            let account_id = &self.data.account.id;
            if message == "/leave" {
                if Jami::rm_conversation(&account_id, &channel.id) {
                    self.data.channels.items.remove(channel_idx);
                    if !self.data.channels.items.is_empty() {
                        self.data.channels.state.select(Some(0));
                    }
                    return;
                } else {
                    channel.messages.push(Message {
                        from: String::new(),
                        message: Some(String::from("Cannot remove conversation")),
                        arrived_at: Utc::now(),
                    });
                }
            } else if message.starts_with("/invite") {
                let mut member = String::from(message.strip_prefix("/invite ").unwrap());
                if Jami::is_hash(&member) {
                    Jami::add_conversation_member(&account_id, &channel.id, &member);
                } else {
                    let mut ns = String::new();
                    if member.find("@") != None {
                        let member_cloned = member.clone();
                        let split : Vec<&str> = member_cloned.split("@").collect();
                        member = split[0].to_string();
                        ns = split[1].to_string();
                    }
                    self.data.out_invite.push(OutgoingInvite {
                        account: account_id.to_string(),
                        channel: channel.id.clone(),
                        member: member.clone(),
                    });
                    show_msg = false;
                    Jami::lookup_name(&account_id, &ns, &member);
                }
            } else if message == "/help" {
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/help: Show this help")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/leave: Leave this conversation")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/invite [hash|username]: Invite somebody to the conversation")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/exit: quit")),
                    arrived_at: Utc::now(),
                });
            } else {
                show_msg = false;
                Jami::send_conversation_message(&account_id, &channel.id, &message, &String::new());
            }
        }

        if show_msg {
            let channel = &mut self.data.channels.items[channel_idx];
            channel.messages.push(Message {
                from: self.data.account.get_display_name(),
                message: Some(message.clone()),
                arrived_at: Utc::now(),
            });
        }

        self.reset_unread_messages();
        self.bubble_up_channel(channel_idx);
    }

    pub fn on_up(&mut self) {
        self.reset_unread_messages();
        self.data.channels.previous();
    }

    pub fn on_down(&mut self) {
        self.reset_unread_messages();
        self.data.channels.next();
    }

    fn reset_unread_messages(&mut self) -> bool {
        if let Some(selected_idx) = self.data.channels.state.selected() {
            if self.data.channels.items[selected_idx].unread_messages > 0 {
                self.data.channels.items[selected_idx].unread_messages = 0;
                return true;
            }
        }
        false
    }

    pub fn on_left(&mut self) {
        self.data.input_cursor = self.data.input_cursor.saturating_sub(1);
    }

    pub fn on_right(&mut self) {
        if self.data.input_cursor < self.data.input.width() {
            self.data.input_cursor += 1;
        }
    }

    #[allow(dead_code)]
    pub fn log(&mut self, msg: impl AsRef<str>) {
        if let Some(log_file) = &mut self.log_file {
            writeln!(log_file, "{}", msg.as_ref()).unwrap();
        }
    }

    pub async fn on_message(
        &mut self,
        account_id: String,
        conversation_id: String,
        payloads: HashMap<String, String>,
    ) -> Option<()> {
        self.log(format!("incoming: {:?}", payloads));
        if account_id == self.data.account.id {
            let channels = &mut self.data.channels;
            if let Some(idx) = channels.state.selected() {
                let channel = &mut channels.items[idx];
                if channel.id == conversation_id {
                    if payloads.get("type").unwrap() == "text/plain" {
                        channel.messages.push(Message {
                            from: String::from(payloads.get("author").unwrap()),
                            message: Some(String::from(payloads.get("body").unwrap())),
                            arrived_at: Utc::now(), // TODO timestamp
                        });
                    } else if payloads.get("type").unwrap() == "member" {
                        let body = String::from(payloads.get("body").unwrap());
                        if body.starts_with("Add member ") {
                            let enter = format!("--> | {} has been added", body.strip_prefix("Add member ").unwrap());
                            channel.messages.push(Message {
                                from: String::new(),
                                message: Some(String::from(enter)),
                                arrived_at: Utc::now(), // TODO timestamp
                            });
                        } else {
                            channel.messages.push(Message {
                                from: String::new(),
                                message: Some(String::from(payloads.get("body").unwrap())),
                                arrived_at: Utc::now(), // TODO timestamp
                            });
                        }
                    } else {
                        channel.messages.push(Message {
                            from: String::new(),
                            message: Some(String::from(format!("{:?}", payloads))),
                            arrived_at: Utc::now(),
                        });
                    }
                }
            }
        }
        Some(())
    }

    pub fn on_conversation_ready(
        &mut self,
        account_id: String,
        conversation_id: String,
    ) -> Option<()> {
        if account_id == self.data.account.id {
            self.data.channels.items.push(Channel {
                id: conversation_id.clone(),
                name: conversation_id,
                members: Vec::new(),
                is_group: true,
                messages: Vec::new(),
                unread_messages: 0,
            });
            self.bubble_up_channel(self.data.channels.items.len() - 1);
            self.data.channels.state.select(Some(0));
        }
        Some(())
    }

    pub fn on_registered_name_found(
        &mut self,
        account_id: String,
        status: u64,
        address: String,
        name: String,
    ) -> Option<()> {
        println!("on_registered");
        self.data.hash2name.insert(address.clone(), name.clone());
        for i in 0..self.data.out_invite.len() {
            let mut out_invite = &self.data.out_invite[i];
            if out_invite.account == account_id && out_invite.member == name {
                if status == 0 {
                    Jami::add_conversation_member(&out_invite.account, &out_invite.channel, &address);
                } else {
                    let channels = &mut self.data.channels.items;
                    for channel in &mut *channels {
                        if channel.id == out_invite.channel {
                            channel.messages.push(Message {
                                from: String::new(),
                                message: Some(String::from("Cannot invite member")),
                                arrived_at: Utc::now(),
                            });
                        }
                    }
                }
                self.data.out_invite.remove(i);
                break;
            }
        }

        // Refresh titles for channel
        for channel in &mut *self.data.channels.items {
            let mut refresh_name = false;
            let mut name = String::new();
            for member in &*channel.members {
                name += self.data.hash2name.get(&member.hash).unwrap_or(&member.hash);
                if member.hash == address {
                    refresh_name = true;
                }
            }
            if refresh_name {
                channel.name = name;
            }
        }
        Some(())
    }

    fn bubble_up_channel(&mut self, channel_idx: usize) {
        // bubble up channel to the beginning of the list
        let channels = &mut self.data.channels;
        for (prev, next) in (0..channel_idx).zip(1..channel_idx + 1).rev() {
            channels.items.swap(prev, next);
        }
        match channels.state.selected() {
            Some(selected_idx) if selected_idx == channel_idx => channels.state.select(Some(0)),
            Some(selected_idx) if selected_idx < channel_idx => {
                channels.state.select(Some(selected_idx + 1));
            }
            _ => {}
        };
    }
}

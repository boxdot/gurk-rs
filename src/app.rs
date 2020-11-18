use crate::account::Account;
use crate::util::StatefulList;
use crate::jami::{ ImportType, Jami };

use chrono::{DateTime, TimeZone, Utc};
use crossterm::event::KeyCode;
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthStr;
use app_dirs::{AppDataType, get_app_dir, AppInfo};

use std::collections::HashMap;
use std::fs::{copy, File};
use ical;
use std::io::{self, BufRead, BufReader, Write};
use std::time::{Duration, UNIX_EPOCH, SystemTime};

pub struct App {
    pub should_quit: bool,
    pub log_file: Option<File>,
    pub data: AppData,
}

#[derive(Serialize, Deserialize)]
pub struct AppData {
    pub channels: StatefulList<Channel>,
    pub hash2name: HashMap<String, String>,
    pub account: Account,
    #[serde(skip)]
    pub out_invite: Vec<OutgoingInvite>,
    #[serde(skip)]
    pub pending_rm: Vec<PendingRm>,
    pub input: String,
    #[serde(skip)]
    pub input_cursor: usize,
}

pub struct OutgoingInvite {
    pub account: String,
    pub channel: Option<String>,
    pub member: String,
}

pub struct PendingRm {
    pub account: String,
    pub channel: String,
    pub member: String,
}

impl AppData {

    // Move to jami namespace
    fn select_jami_account(create_if_not: bool) -> Account {
        let accounts = Jami::get_account_list();
        // Select first enabled account
        for account in &accounts {
            if account.enabled {
                return account.clone();
            }
        }
        if create_if_not {
            // No valid account found, generate a new one
            Jami::add_account("", "", ImportType::None);
        }
        return Account::null();
    }

    fn lookup_members(&mut self) {
        // Refresh titles for channel
        for channel in &mut *self.channels.items {
            for member in &*channel.members {
                Jami::lookup_address(&self.account.id, &String::new(), &member.hash);
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
            channel_type: ChannelType::Generated,
            messages,
            unread_messages: 0,
        });
        
        for request in Jami::get_conversations_requests(&account.id) {
            channels.push(Channel {
                id: request.get("id").unwrap().clone(),
                name: String::from(format!("r:{}", request.get("id").unwrap())),
                members: Vec::new(),
                channel_type: ChannelType::Invite,
                messages: Vec::new(),
                unread_messages: 0,
            });
        }
        
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
                members,
                channel_type: ChannelType::Group,
                messages: Vec::new(),
                unread_messages: 0,
            });
        }
        channels
    }

    fn init_from_jami() -> anyhow::Result<Self> {
        let account = AppData::select_jami_account(true);
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
            pending_rm: Vec::new(),
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ChannelType {
    Generated,
    Group,
    Invite,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Channel {
    /// Either phone number or group id
    pub id: String,
    pub name: String,
    pub channel_type: ChannelType,
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
    ConversationRequest(String, String),
    RegistrationStateChanged(String, String),
    ProfileReceived(String, String, String),
    RegisteredNameFound(String, u64, String, String),
    AccountsChanged(),
    ConversationLoaded(u32, String, String, Vec<HashMap<String, String>>),
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

        Ok(Self {
            data,
            should_quit: false,
            log_file,
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

    pub async fn on_registration_state_changed(&mut self, _account_id: &String, registration_state: &String) {
        if registration_state == "REGISTERED" && self.data.account == Account::null() {
            self.data.account = AppData::select_jami_account(false);
        }
    }

    pub async fn on_accounts_changed(&mut self) {
        let mut still_there = false;
        for account in Jami::get_account_list() {
            if account.id == self.data.account.id {
                still_there = true;
                break;
            }
        }
        if !still_there {
            // Reselect an account
            self.data.account = AppData::select_jami_account(false);
            if self.data.account.id.is_empty() {
                self.data.channels.state.select(Some(0));
                self.data.channels.items.retain(|channel| channel.id.is_empty());
                self.data.channels.items[0].messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("!!!! No more account left to use")),
                    arrived_at: Utc::now(),
                });
                return;
            }
            let channels = AppData::channels_for_account(&self.data.account);
            self.data.channels = StatefulList::with_items(channels);
            if !self.data.channels.items.is_empty() {
                self.data.channels.state.select(Some(0));
            }
            self.data.lookup_members();
        }
    }

    pub async fn on_profile_received(&mut self, account_id: &String, from: &String, path: &String) {
        let dest = get_app_dir(
                        AppDataType::UserData,
                        &AppInfo{name: "jami", author: "SFL"},
                        &*format!("{}/profiles", account_id)
                    );
        if dest.is_err() {
            return;
        }
        let dest = dest.unwrap().into_os_string().into_string();
        let dest =  format!("{}/{}.vcf", dest.unwrap(), &base64::encode(&*from));
        let result = copy(path, dest.clone());
        if result.is_err() {
            return;
        }
        // TODO parse
        // TODO add store system for vcards
        // TODO improve lookup
//        let buf = BufReader::new(File::open(dest).unwrap());
//        let reader = ical::VcardParser::new(buf);
//        for line in reader {
//            println!("{:?}", line);
//        }
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
        if message.starts_with("/msg ") {
            let account_id = &self.data.account.id;
            let mut member = String::from(message.strip_prefix("/msg ").unwrap());
            if Jami::is_hash(&member) {
                let conversation = Jami::start_conversation(&account_id);
                Jami::add_conversation_member(&account_id, &conversation, &member);
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
                    channel: None,
                    member: member.clone(),
                });
                show_msg = false;
                Jami::lookup_name(&account_id, &ns, &member);
            }
        } else if channel.channel_type == ChannelType::Generated {
            if message == "/new" {
                Jami::start_conversation(&self.data.account.id);
            } else if message == "/list" {
                for account in Jami::get_account_list() {
                    channel.messages.push(Message {
                        from: String::new(),
                        message: Some(String::from(format!("{}", account))),
                        arrived_at: Utc::now(),
                    });
                }
            } else if message == "/get" || message.starts_with("/get ") {
                let parts : Vec<&str> = message.split(" ").collect();
                let filter = parts.get(1).unwrap_or(&"").to_string();
                for (key, value) in Jami::get_account_details(&self.data.account.id) {
                    if filter.is_empty() || filter.to_lowercase() == key.to_lowercase() {
                        channel.messages.push(Message {
                            from: String::new(),
                            message: Some(String::from(format!("{}: {}", key, value))),
                            arrived_at: Utc::now(),
                        });
                    }
                }
                show_msg = false;
            } else if message.starts_with("/set") {
                let parts : Vec<&str> = message.split(" ").collect();
                let key = parts.get(1).unwrap_or(&"").to_string();
                let value = parts.get(2).unwrap_or(&"").to_string();
                let mut details = Jami::get_account_details(&self.data.account.id);
                let mut key_found = String::new();
                for (key2, _) in &details {
                    if key2.to_lowercase() == key.to_lowercase() {
                        key_found = key2.to_string();
                    }
                }
                if !key_found.is_empty() {
                    details.insert(key_found, value.to_string());
                }
                Jami::set_account_details(&self.data.account.id, details);
                show_msg = false;
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
                    //  TODO avoid duplicate code
                    self.data.account = account;
                    let channels = AppData::channels_for_account(&self.data.account);
                    self.data.channels = StatefulList::with_items(channels);
                    if !self.data.channels.items.is_empty() {
                        self.data.channels.state.select(Some(0));
                    }
                    self.data.lookup_members();
                }
            } else if message == "/add" {
                Jami::add_account("", "", ImportType::None);
            } else if message.starts_with("/rm ") {
                let account_id = String::from(message.strip_prefix("/rm ").unwrap());
                Jami::rm_account(&*account_id);
            } else if message.starts_with("/import ") {
                let parts : Vec<&str> = message.split(" ").collect();
                let file = parts.get(1).unwrap_or(&"").to_string();
                let password = parts.get(2).unwrap_or(&"").to_string();
                Jami::add_account(&file, &password, ImportType::BACKUP);
            } else if message.starts_with("/link ") {
                let parts : Vec<&str> = message.split(" ").collect();
                let pin = parts.get(1).unwrap_or(&"").to_string();
                let password = parts.get(2).unwrap_or(&"").to_string();
                Jami::add_account(&pin, &password, ImportType::NETWORK);
            } else if message == "/help" {
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/help: Show this help")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/new: Start a new conversation")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/msg <id|username>: Start a conversation with someone")),
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
                    message: Some(String::from("/add: Add a new account")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/rm <id>: Remove an account")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/link <pin> [password]: Link an account via a PIN")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/import <file> [password]: Import an account from a backup")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/get [key]: get account details (if key specified, only get key)")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/set <key> <value>: set account detail")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/exit: quit")),
                    arrived_at: Utc::now(),
                });
            }
        } else if channel.channel_type == ChannelType::Group {
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
                        channel: Some(channel.id.clone()),
                        member: member.clone(),
                    });
                    show_msg = false;
                    Jami::lookup_name(&account_id, &ns, &member);
                }
            }  else if message.starts_with("/kick") {
                // TODO remove code duplication
                let mut member = String::from(message.strip_prefix("/kick ").unwrap());
                if Jami::is_hash(&member) {
                    Jami::rm_conversation_member(&account_id, &channel.id, &member);
                } else {
                    let mut ns = String::new();
                    if member.find("@") != None {
                        let member_cloned = member.clone();
                        let split : Vec<&str> = member_cloned.split("@").collect();
                        member = split[0].to_string();
                        ns = split[1].to_string();
                    }
                    self.data.pending_rm.push(PendingRm {
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
                    message: Some(String::from("/kick [hash|username]: Kick someone from the conversation")),
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
        } else if channel.channel_type == ChannelType::Invite {
            // TODO simplify
            let account_id = &self.data.account.id;
            if message == "/leave" {
                Jami::decline_request(&account_id, &channel.id);
                self.data.channels.items.remove(channel_idx);
                if !self.data.channels.items.is_empty() {
                    self.data.channels.state.select(Some(0));
                }
            } else if message.starts_with("/join") {
                Jami::accept_request(&account_id, &channel.id);
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("Syncing… the view will update")),
                    arrived_at: Utc::now(),
                });
            } else {
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/help: Show this help")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/leave: Decline this request")),
                    arrived_at: Utc::now(),
                });
                channel.messages.push(Message {
                    from: String::new(),
                    message: Some(String::from("/join: Accepts the request")),
                    arrived_at: Utc::now(),
                });
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
        if let Some(idx) = self.data.channels.state.selected() {
            let channel = &mut self.data.channels.items[idx];
            if channel.channel_type == ChannelType::Group {
                channel.messages.clear();
                Jami::load_conversation(&self.data.account.id, &channel.id, &String::new(), 0);            
            }           
        }
    }

    pub fn on_down(&mut self) {
        self.reset_unread_messages();
        self.data.channels.next();
        if let Some(idx) = self.data.channels.state.selected() {
            let channel = &mut self.data.channels.items[idx];
            if channel.channel_type == ChannelType::Group {
                channel.messages.clear();
                Jami::load_conversation(&self.data.account.id, &channel.id, &String::new(), 0);            
            }
        }
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
            for channel in &mut *self.data.channels.items {
                if channel.id == conversation_id {
                    // Parse timestamp
                    let mut arrived_at = SystemTime::UNIX_EPOCH;
                    let tstr : String = payloads.get("timestamp").unwrap_or(&String::new()).to_string();
                    if tstr.is_empty() {
                        arrived_at = SystemTime::now();
                    } else {
                        arrived_at += Duration::from_secs(tstr.parse::<u64>().unwrap_or(0));
                    }
                    let arrived_at = Utc.timestamp(arrived_at.duration_since(UNIX_EPOCH).unwrap().as_secs() as i64, 0);
                    // author
                    let author_str = payloads.get("author").unwrap_or(&String::new()).to_string();
                    let mut author = self.data.hash2name.get(&author_str).unwrap_or(&author_str).to_string();
                    if author.is_empty() {
                        author = author_str;
                    }
                    // print message
                    if payloads.get("type").unwrap().is_empty() {
                        channel.messages.push(Message {
                            from: author,
                            message: Some(String::from("--> started the conversation")),
                            arrived_at,
                        });
                    } else if payloads.get("type").unwrap() == "text/plain" {
                        channel.messages.push(Message {
                            from: author,
                            message: Some(String::from(payloads.get("body").unwrap())),
                            arrived_at,
                        });
                    } else if payloads.get("type").unwrap() == "merge" {
                        // Do not show merge commits
                    } else if payloads.get("type").unwrap() == "member" {
                        let body = String::from(payloads.get("body").unwrap());
                        if body.starts_with("Add member ") {
                            let uri_str = body.strip_prefix("Add member ").unwrap().to_string();
                            let mut uri = self.data.hash2name.get(&uri_str).unwrap_or(&uri_str).to_string();
                            if uri.is_empty() {
                                uri = uri_str;
                            }
                            let enter = format!("--> | {} has been added", uri);
                            channel.messages.push(Message {
                                from: author,
                                message: Some(String::from(enter)),
                                arrived_at,
                            });
                        } else {
                            channel.messages.push(Message {
                                from: author,
                                message: Some(String::from(payloads.get("body").unwrap())),
                                arrived_at,
                            });
                        }
                    } else {
                        channel.messages.push(Message {
                            from: author,
                            message: Some(String::from(format!("{:?}", payloads))),
                            arrived_at,
                        });
                    }
                }
            }
        }
        Some(())
    }

    pub async fn on_conversation_loaded(
        &mut self,
        _id: u32,
        account_id: String,
        conversation_id: String,
        messages: Vec<HashMap<String, String>>,
    ) -> Option<()> {
        let messages: Vec<_> = messages.into_iter().rev().collect();
        for msg in messages {
            // TODO no need to clone
            let _ = self.on_message(account_id.clone(), conversation_id.clone(), msg).await;
        }
        Some(())
    }

    pub async fn on_conversation_ready(
        &mut self,
        account_id: String,
        conversation_id: String,
    ) -> Option<()> {
        if account_id == self.data.account.id {
            self.data.channels.state.select(Some(0));
            self.data.channels.items.retain(|channel| channel.id != conversation_id);
            self.data.channels.items.push(Channel {
                id: conversation_id.clone(),
                name: conversation_id,
                members: Vec::new(),
                channel_type: ChannelType::Group,
                messages: Vec::new(),
                unread_messages: 0,
            });
            self.bubble_up_channel(self.data.channels.items.len() - 1);
            self.data.channels.state.select(Some(0));
        }
        Some(())
    }

    pub async fn on_conversation_request(
        &mut self,
        account_id: String,
        conversation_id: String,
    ) -> Option<()> {
        if account_id == self.data.account.id {
            self.data.channels.items.push(Channel {
                id: conversation_id.clone(),
                name: String::from(format!("r:{}", conversation_id)),
                members: Vec::new(),
                channel_type: ChannelType::Group,
                messages: Vec::new(),
                unread_messages: 0,
            });
            self.bubble_up_channel(self.data.channels.items.len() - 1);
            self.data.channels.state.select(Some(0));
        }
        Some(())
    }

    pub async fn on_registered_name_found(
        &mut self,
        account_id: String,
        status: u64,
        address: String,
        name: String,
    ) -> Option<()> {
        self.data.hash2name.insert(address.clone(), name.clone());
        // pending invite
        for i in 0..self.data.out_invite.len() {
            let out_invite = &self.data.out_invite[i];
            if out_invite.account == account_id && out_invite.member == name {
                if status == 0 {
                    let conversation : String;
                    if out_invite.channel.as_ref().is_none() {
                        conversation = Jami::start_conversation(&self.data.account.id);
                    } else {
                        conversation = out_invite.channel.clone().unwrap();
                    }
                    Jami::add_conversation_member(&out_invite.account, &conversation, &address);
                } else {
                    let channels = &mut self.data.channels.items;
                    for channel in &mut *channels {
                        if channel.id == out_invite.channel.clone().unwrap_or(String::new()) {
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

        // pending remove
        for i in 0..self.data.pending_rm.len() {
            let pending_rm = &self.data.pending_rm[i];
            if pending_rm.account == account_id && pending_rm.member == name {
                if status == 0 {
                    Jami::rm_conversation_member(&pending_rm.account, &pending_rm.channel, &address);
                }
                self.data.pending_rm.remove(i);
                break;
            }
        }

        // Refresh titles for channel
        for channel in &mut *self.data.channels.items {
            let mut refresh_name = false;
            let mut name = String::new();
            for member in &*channel.members {
                let uri = self.data.hash2name.get(&member.hash).unwrap_or(&member.hash);
                if !uri.is_empty() {
                    name += uri;
                } else {
                    name += &member.hash;
                }
                name += ", ";
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
        for (prev, next) in (1..channel_idx).zip(2..channel_idx + 1).rev() {
            channels.items.swap(prev, next);
        }
        match channels.state.selected() {
            Some(0) if 0 == channel_idx => channels.state.select(Some(0)),
            Some(selected_idx) if selected_idx == channel_idx => channels.state.select(Some(1)),
            Some(selected_idx) if selected_idx < channel_idx => {
                channels.state.select(Some(selected_idx + 1));
            }
            _ => {}
        };
    }
}

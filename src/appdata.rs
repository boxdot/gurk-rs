use crate::jami::account::Account;
use crate::jami::{Jami, ProfileManager};
use crate::util::*;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, BufRead};

#[derive(Serialize, Deserialize)]
pub struct AppData {
    pub channels: StatefulList<Channel>,
    pub account: Account,
    pub profile_manager: ProfileManager,
    #[serde(skip)]
    pub out_invite: Vec<OutgoingInvite>,
    #[serde(skip)]
    pub pending_rm: Vec<PendingRm>,
    pub input: String,
    #[serde(skip)]
    pub input_cursor: usize,
}

impl AppData {
    /**
     * Lookup on nameserver all members
     */
    pub fn lookup_members(&mut self) {
        // Refresh titles for channel
        for channel in &mut *self.channels.items {
            for member in &*channel.members {
                Jami::lookup_address(&self.account.id, &String::new(), &member.hash);
            }
        }
    }

    /**
     * Get channel for account
     * @param account   
     * @return the channels
     */
    pub fn channels_for_account(account: &Account) -> Vec<Channel> {
        let mut channels = Vec::new();
        let mut messages = Vec::new();

        // Create Welcome channel
        let file = File::open("rsc/welcome-art");
        if file.is_ok() {
            for line in io::BufReader::new(file.unwrap()).lines() {
                messages.push(Message::new(
                    String::new(),
                    String::from(line.unwrap()),
                    Utc::now(),
                ));
            }
        }

        channels.push(Channel {
            id: String::from("⚙️ Jami-cli"),
            name: String::from("⚙️ Jami-cli"),
            members: Vec::new(),
            channel_type: ChannelType::Generated,
            messages,
            unread_messages: 0,
        });

        // Get requests
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

        // Get conversations
        for conversation in Jami::get_conversations(&account.id) {
            let members_from_daemon = Jami::get_members(&account.id, &conversation);
            let mut members = Vec::new();
            for member in members_from_daemon {
                let role: Role;
                if member["role"].to_string() == "admin" {
                    role = Role::Admin;
                } else {
                    role = Role::Member;
                }
                let hash = member["uri"].to_string();
                members.push(Member { hash, role })
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

    // Init self
    pub fn init_from_jami() -> anyhow::Result<Self> {
        let account = Jami::select_jami_account(true);
        let mut channels = Vec::new();
        let mut profile_manager = ProfileManager::new();
        if !account.id.is_empty() {
            profile_manager.load_from_account(&account.id);
            channels = AppData::channels_for_account(&account);
        }

        let mut channels = StatefulList::with_items(channels);
        if !channels.items.is_empty() {
            channels.state.select(Some(0));
        }

        Ok(AppData {
            channels,
            input: String::new(),
            profile_manager,
            out_invite: Vec::new(),
            pending_rm: Vec::new(),
            input_cursor: 0,
            account,
        })
    }
}

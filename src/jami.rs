/**
 * Copyright (c) 2020, Sébastien Blin <sebastien.blin@enconn.fr>
 * All rights reserved.
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions are met:
 *
 * * Redistributions of source code must retain the above copyright
 *  notice, this list of conditions and the following disclaimer.
 * * Redistributions in binary form must reproduce the above copyright
 *  notice, this list of conditions and the following disclaimer in the
 *  documentation and/or other materials provided with the distribution.
 * * Neither the name of the University of California, Berkeley nor the
 *  names of its contributors may be used to endorse or promote products
 *  derived from this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE REGENTS AND CONTRIBUTORS ``AS IS'' AND ANY
 * EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
 * WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
 * DISCLAIMED. IN NO EVENT SHALL THE REGENTS AND CONTRIBUTORS BE LIABLE FOR ANY
 * DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
 * (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
 * LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
 * ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 * (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
 * SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 **/

use crate::account::Account;
use dbus::{Connection, ConnectionItem, BusType, Message};
use dbus::arg::{Array, Dict};
use log::{debug, error, log_enabled, info, Level};
use serde_json::{Value, from_str};
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};

/**TODO
 */
pub struct Jami {
    jami_dbus: &'static str,
    configuration_path: &'static str,
    configuration_iface: &'static str,
}

impl Jami {
    /**
     * Init the RORI server, the database and retrieve the RING account linked
     * @param hash to retrieve
     * @return a Manager if success, else an error
     */
    pub fn init() -> Result<Jami, &'static str> {
        let mut manager = Jami {
            jami_dbus: "cx.ring.Ring",
            configuration_path: "/cx/ring/Ring/ConfigurationManager",
            configuration_iface: "cx.ring.Ring.ConfigurationManager",
        };
        Ok(manager)
    }

    /**
     * Listen from interresting signals from dbus and call handlers
     * @param self
     */
    pub fn handle_signals(manager: Arc<Mutex<Jami>>) {
        // Use another dbus connection to listen signals.
        let dbus_listener = Connection::get_private(BusType::Session).unwrap();
        dbus_listener.add_match("interface=cx.ring.Ring.ConfigurationManager,member=incomingAccountMessage").unwrap();
        dbus_listener.add_match("interface=cx.ring.Ring.ConfigurationManager,member=incomingTrustRequest").unwrap();
        dbus_listener.add_match("interface=cx.ring.Ring.ConfigurationManager,member=accountsChanged").unwrap();
        dbus_listener.add_match("interface=cx.ring.Ring.ConfigurationManager,member=registrationStateChanged").unwrap();
        // For each signals, call handlers.
        for i in dbus_listener.iter(100) {

            /*let mut m = manager.lock().unwrap();
            m.handle_accounts_signals(&i);
            m.handle_registration_changed(&i);
            if let Some((account_id, interaction)) = m.handle_interactions(&i) {
                info!("New interation for {}: {}", account_id, interaction);
            };*/
        }
    }

    // Helpers

    /**
     * Add a RING account
     * @param main_info path or alias
     * @param password
     * @param from_archive if main_info is a path
     */
    pub fn add_account(main_info: &str, password: &str, from_archive: bool) {
        let mut details: HashMap<&str, &str> = HashMap::new();
        if from_archive {
            details.insert("Account.archivePath", main_info);
        } else {
            details.insert("Account.alias", main_info);
        }
        details.insert("Account.type", "RING");
        details.insert("Account.archivePassword", password);
        let details = Dict::new(details.iter());
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "addAccount");
        if !dbus_msg.is_ok() {
            error!("addAccount fails. Please verify daemon's API.");
            return;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return;
        }
        let dbus = conn.unwrap();
        let response = dbus.send_with_reply_and_block(dbus_msg.unwrap()
                                                                .append1(details), 2000).unwrap();
        // addAccount returns one argument, which is a string.
        let account_added: &str  = match response.get1() {
            Some(account) => account,
            None => ""
        };
        info!("New account: {:?}", account_added);
    }

    /**
     * Get current ring accounts
     * @return current accounts
     */
    pub fn get_account_list() -> Vec<Account> {
        let mut account_list: Vec<Account> = Vec::new();
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "getAccountList");
        if !dbus_msg.is_ok() {
            error!("getAccountList fails. Please verify daemon's API.");
            return account_list;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return account_list;
        }
        let dbus = conn.unwrap();
        let response = dbus.send_with_reply_and_block(dbus_msg.unwrap(), 2000).unwrap();
        // getAccountList returns one argument, which is an array of strings.
        let accounts: Array<&str, _>  = match response.get1() {
            Some(array) => array,
            None => return account_list
        };
        for account in accounts {
            account_list.push(Jami::build_account(account));
        }
        account_list
    }

    /**
     * Get current contacts for account
     * @param id        Id of the account
     * @return current contacts
     */
    pub fn get_contacts(id: &String) -> Vec<HashMap<String, String>> {
        let mut contacts_list: Vec<HashMap<String, String>> = Vec::new();
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "getContacts");
        if !dbus_msg.is_ok() {
            error!("getAccountList fails. Please verify daemon's API.");
            return contacts_list;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return contacts_list;
        }
        let dbus = conn.unwrap();
        let response = dbus.send_with_reply_and_block(dbus_msg.unwrap().append1(&*id), 2000).unwrap();
        // getAccountList returns one argument, which is an array of strings.
        let contacts: Array<Dict<&str, &str, _>, _> = match response.get1() {
            Some(contacts) => contacts,
            None => return contacts_list
        };
        for contact in contacts {
            let mut details = HashMap::new();
            for (key, value) in contact {
                details.insert(String::from(key), String::from(value));
            }
            contacts_list.push(details);
        }
        contacts_list
    }

    /**
     * Start conversation
     * @param id        Id of the account
     */
    pub fn start_conversation(id: &String) {
        let mut account_list: Vec<Account> = Vec::new();
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "startConversation");
        if !dbus_msg.is_ok() {
            error!("startConversation fails. Please verify daemon's API.");
            return;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return;
        }
        let dbus = conn.unwrap();
        let response = dbus.send_with_reply_and_block(dbus_msg.unwrap().append1(&*id), 2000).unwrap();
    }

    /**
     * Get current conversations for account
     * @param id        Id of the account
     * @return current conversations
     */
    pub fn get_conversations(id: &String) -> Vec<String> {
        let mut conversations: Vec<String> = Vec::new();
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "getConversations");
        if !dbus_msg.is_ok() {
            error!("getAccountList fails. Please verify daemon's API.");
            return conversations;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return conversations;
        }
        let dbus = conn.unwrap();
        let response = dbus.send_with_reply_and_block(dbus_msg.unwrap().append1(&*id), 2000).unwrap();
        // getAccountList returns one argument, which is an array of strings.
        let conv_resp: Array<&str, _> = match response.get1() {
            Some(conversations) => conversations,
            None => return conversations
        };
        for conv in conv_resp {
            conversations.push(String::from(conv));
        }
        conversations
    }

    /**
     * Remove a conversation for an account
     * @param id        Id of the account
     * @param conv_id   Id of the conversation
     * @return if the conversation is removed
     */
    pub fn rm_conversation(id: &String, conv_id: &String) -> bool {
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "removeConversation");
        if !dbus_msg.is_ok() {
            error!("getAccountList fails. Please verify daemon's API.");
            return false;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return false;
        }
        let dbus = conn.unwrap();
        let response = dbus.send_with_reply_and_block(dbus_msg.unwrap().append2(&*id, &*conv_id), 2000).unwrap();
        true
    }

    /**
     * Remove a conversation for an account
     * @param id        Id of the account
     * @param conv_id   Id of the conversation
     * @param hash      Id of the member to invite
     */
    pub fn add_conversation_member(id: &String, conv_id: &String, hash: &String) {
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "addConversationMember");
        if !dbus_msg.is_ok() {
            error!("getAccountList fails. Please verify daemon's API.");
            return;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return;
        }
        let dbus = conn.unwrap();
        let response = dbus.send_with_reply_and_block(dbus_msg.unwrap().append3(&*id, &*conv_id, &*hash), 2000).unwrap();
    }

    /**
     * Remove a conversation for an account
     * @param id        Id of the account
     * @param conv_id   Id of the conversation
     * @param hash      Id of the member to invite
     * @param hash      Id of the member to invite
     */
    pub fn send_conversation_message(id: &String, conv_id: &String, message: &String, parent: &String) -> u64 {
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "sendMessage");
        if !dbus_msg.is_ok() {
            error!("getAccountList fails. Please verify daemon's API.");
            return 0;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return 0;
        }
        let dbus = conn.unwrap();
        let response = dbus.send_with_reply_and_block(dbus_msg.unwrap().append3(&*id, &*conv_id, &*message).append1(&*parent), 2000).unwrap();
        response.get1().unwrap_or(0) as u64
    }

// Private stuff
    /**
     * Build a new account with an id from the daemon
     * @param id the account id to build
     * @return the account retrieven
     */
    fn build_account(id: &str) -> Account {
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "getAccountDetails");
        if !dbus_msg.is_ok() {
            error!("getAccountDetails fails. Please verify daemon's API.");
            return Account::null();
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            error!("connection not ok.");
            return Account::null();
        }
        let dbus = conn.unwrap();
        let response = dbus.send_with_reply_and_block(
                                           dbus_msg.unwrap().append1(id), 2000
                                       ).ok().expect("Is the ring-daemon launched?");
        let details: Dict<&str, &str, _> = match response.get1() {
            Some(details) => details,
            None => {
                return Account::null();
            }
        };

        let mut account = Account::null();
        account.id = id.to_owned();
        for detail in details {
            match detail {
                (key, value) => {
                    if key == "Account.enable" {
                        account.enabled = value == "true";
                    }
                    if key == "Account.alias" {
                        account.alias = String::from(value);
                    }
                    if key == "Account.username" {
                        account.hash = String::from(value).replace("ring:", "");
                    }
                }
            }
        }
        account
    }

    /**
     * Update current RORI account by handling accountsChanged signals from daemon.
     * @param self
     * @param ci
     */
    fn handle_accounts_signals(&mut self, ci: &ConnectionItem) {
        // Check signal
        let msg = if let &ConnectionItem::Signal(ref signal) = ci { signal } else { return };
        if &*msg.interface().unwrap() != "cx.ring.Ring.ConfigurationManager" { return };
        if &*msg.member().unwrap() != "accountsChanged" { return };
        // TODO test if RORI accounts is still exists
    }

    /**
    * Handle new interactions signals
    * @param self
    * @param ci
    * @return (accountId, interaction)
    */
    /*fn handle_interactions(&self, ci: &ConnectionItem) -> Option<(String, Interaction)> {
        // Check signal
        let msg = if let &ConnectionItem::Signal(ref signal) = ci { signal } else { return None };
        if &*msg.interface().unwrap() != "cx.ring.Ring.ConfigurationManager" { return None };
        if &*msg.member().unwrap() != "incomingAccountMessage" { return None };
        // incomingAccountMessage return three arguments
        let (account_id, _msg_id, author_hash, payloads) = msg.get4::<&str, &str, &str, Dict<&str, &str, _>>();
        let author_hash = author_hash.unwrap().to_string();
        let mut body = String::new();
        let mut datatype = String::new();
        let mut metadatas: HashMap<String, String> = HashMap::new();
        for detail in payloads.unwrap() {
            match detail {
                (key, value) => {
                    // TODO for now, text/plain is the only supported datatypes, changes this with key in supported datatypes
                    if key == "text/plain" {
                        datatype = key.to_string();
                        body = value.to_string();
                    } else {
                        metadatas.insert(
                            key.to_string(),
                            value.to_string()
                        );
                    }
                }
            }
        };
        let interaction = Interaction {
            author_hash: author_hash,
            body: body,
            datatype: datatype,
            metadatas: metadatas
        };
        Some((account_id.unwrap().to_string(), interaction))
    }*/

    /**
     * Update current RORI account by handling accountsChanged signals from daemon
     * @param self
     * @param ci
     */
    fn handle_registration_changed(&self, ci: &ConnectionItem) {
        // Check signal
        let msg = if let &ConnectionItem::Signal(ref signal) = ci { signal } else { return };
        if &*msg.interface().unwrap() != "cx.ring.Ring.ConfigurationManager" { return };
        if &*msg.member().unwrap() != "registrationStateChanged" { return };
        // let (account_id, registration_state, _, _) = msg.get4::<&str, &str, u64, &str>();
        // TODO the account can be disabled. Inform UI
    }

    /**
     * Handle new pending requests signals
     * @param self
     * @param ci
     * @return (accountId, from)
     */
    fn handle_requests(&self, ci: &ConnectionItem) -> Option<(String, String)> {
        // Check signal
        let msg = if let &ConnectionItem::Signal(ref signal) = ci { signal } else { return None };
        if &*msg.interface().unwrap() != "cx.ring.Ring.ConfigurationManager" { return None };
        if &*msg.member().unwrap() != "incomingTrustRequest" { return None };
        // incomingTrustRequest return three arguments
        let (account_id, from, _, _) = msg.get4::<&str, &str, Dict<&str, &str, _>, u64>();
        Some((account_id.unwrap().to_string(), from.unwrap().to_string()))
    }
}

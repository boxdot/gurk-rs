use crate::app::Event;
use crate::account::Account;
use dbus::{Connection, ConnectionItem, BusType, Message};
use dbus::arg::{Array, Dict};
use log::{error, info};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/**TODO
 */
pub struct Jami {}

impl Jami {
    pub async fn handle_events<T: std::fmt::Debug>(
        mut tx: tokio::sync::mpsc::Sender<crate::app::Event<T>>,
        stop: Arc<AtomicBool>,
    ) -> Result<(), std::io::Error> {
        
        loop {
            // todo separate + doc
            let mut events = Vec::new();
            {
                let dbus_listener = Connection::get_private(BusType::Session).unwrap();
                dbus_listener.add_match("interface=cx.ring.Ring.ConfigurationManager,member=incomingTrustRequest").unwrap();
                dbus_listener.add_match("interface=cx.ring.Ring.ConfigurationManager,member=accountsChanged").unwrap();
                dbus_listener.add_match("interface=cx.ring.Ring.ConfigurationManager,member=registrationStateChanged").unwrap();
                dbus_listener.add_match("interface=cx.ring.Ring.ConfigurationManager,member=messageReceived").unwrap();
                dbus_listener.add_match("interface=cx.ring.Ring.ConfigurationManager,member=conversationReady").unwrap();
                dbus_listener.add_match("interface=cx.ring.Ring.ConfigurationManager,member=registeredNameFound").unwrap();
                dbus_listener.add_match("interface=cx.ring.Ring.ConfigurationManager,member=conversationRequestReceived").unwrap();
                dbus_listener.add_match("interface=cx.ring.Ring.ConfigurationManager,member=conversationLoaded").unwrap();
                // For each signals, call handlers.
                for ci in dbus_listener.iter(100) {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let msg = if let ConnectionItem::Signal(ref signal) = ci { signal } else { continue };
                    if &*msg.interface().unwrap() != "cx.ring.Ring.ConfigurationManager" { continue };
                    if &*msg.member().unwrap() == "messageReceived" {
                        let (account_id, conversation_id, payloads_dict) = msg.get3::<&str, &str, Dict<&str, &str, _>>();
                        let mut payloads: HashMap<String, String> = HashMap::new();
                        for (key, value) in payloads_dict.unwrap() {
                            payloads.insert(String::from(key), String::from(value));
                        }
                        events.push(Event::Message {
                            account_id: String::from(account_id.unwrap()),
                            conversation_id: String::from(conversation_id.unwrap()),
                            payloads
                        });
                    } else if &*msg.member().unwrap() == "registrationStateChanged" { 
                        let (account_id, registration_state, _, _) = msg.get4::<&str, &str, u64, &str>();
                        events.push(Event::RegistrationStateChanged(String::from(account_id.unwrap()), String::from(registration_state.unwrap())));
                    } else if &*msg.member().unwrap() == "conversationReady" { 
                        let (account_id, conversation_id) = msg.get2::<&str, &str>();
                        events.push(Event::ConversationReady(String::from(account_id.unwrap()), String::from(conversation_id.unwrap())));
                    } else if &*msg.member().unwrap() == "registeredNameFound" { 
                        let (account_id, status, address, name) = msg.get4::<&str, i32, &str, &str>();
                        events.push(Event::RegisteredNameFound(String::from(account_id.unwrap()), status.unwrap() as u64, String::from(address.unwrap()), String::from(name.unwrap())));
                    } else if &*msg.member().unwrap() == "conversationRequestReceived" { 
                        let (account_id, conversation_id, _) = msg.get3::<&str, &str, Dict<&str, &str, _>>();
                        events.push(Event::ConversationRequest(String::from(account_id.unwrap()), String::from(conversation_id.unwrap())));
                    } else if &*msg.member().unwrap() == "conversationLoaded" { 
                        let (id, account_id, conversation_id, messages_dbus) = msg.get4::<u32, &str, &str, Array<Dict<&str, &str, _>, _>>();
                        let mut messages = Vec::new();
                        for message_dbus in messages_dbus.unwrap() {
                            let mut message: HashMap<String, String> = HashMap::new();
                            for (key, value) in message_dbus {
                                message.insert(String::from(key), String::from(value));
                            }
                            messages.push(message);
                        }
                        events.push(Event::ConversationLoaded(id.unwrap(), String::from(account_id.unwrap()), String::from(conversation_id.unwrap()), messages));
                    }
                    
                    // Send events
                    if !events.is_empty() {
                        break;
                    }
                }
            }

            if stop.load(Ordering::Relaxed) {
                break;
            }

            for ev in events {
                if tx.send(ev).await.is_err() {
                    return Ok(()); // receiver closed
                }
            }
        }

        Ok(())
    }

    /**
     * Asynchronously lookup a name
     * @param account
     * @param name_service
     * @param name
     * @return if dbus is ok
     */
    pub fn lookup_name(account: &String, name_service: &String, name: &String) -> bool {
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "lookupName");
        if !dbus_msg.is_ok() {
            error!("lookupName fails. Please verify daemon's API.");
            return false;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return false;
        }
        let dbus = conn.unwrap();
        let _ = dbus.send_with_reply_and_block(dbus_msg.unwrap().append3(&*account, &*name_service, &*name), 2000).unwrap();
        true
    }

    /**
     * Asynchronously lookup an address
     * @param account
     * @param name_service
     * @param address
     * @return if dbus is ok
     */
    pub fn lookup_address(account: &String, name_service: &String, address: &String) -> bool {
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "lookupAddress");
        if !dbus_msg.is_ok() {
            error!("lookupAddress fails. Please verify daemon's API.");
            return false;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return false;
        }
        let dbus = conn.unwrap();
        let _ = dbus.send_with_reply_and_block(dbus_msg.unwrap().append3(&*account, &*name_service, &*address), 2000).unwrap();
        true
    }

    // Helpers

    pub fn is_hash(string: &String) -> bool {
        if string.len() != 40 {
            return false;
        }
        for i in 0..string.len() {
            if "0123456789abcdef".find(string.as_bytes()[i] as char) == None {
                return false;
            }
        }
        true
    }

    /**
     * Add a new account
     * @param main_info path or alias
     * @param password
     * @param from_archive if main_info is a path
     */
    pub fn add_account(main_info: &str, password: &str, from_archive: bool) -> String {
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
            return String::new();
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return String::new();
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
        String::from(account_added)
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
            account_list.push(Jami::get_account(account));
        }
        account_list
    }

    /**
     * Build a new account with an id from the daemon
     * @param id the account id to build
     * @return the account retrieven
     */
    pub fn get_account(id: &str) -> Account {
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
     * Get current members for a conversation
     * @param id        Id of the account
     * @param convid    Id of the conversation
     * @return current members
     */
    pub fn get_members(id: &String, convid: &String) -> Vec<HashMap<String, String>> {
        let mut members: Vec<HashMap<String, String>> = Vec::new();
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "getConversationMembers");
        if !dbus_msg.is_ok() {
            error!("getConversationMembers fails. Please verify daemon's API.");
            return members;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return members;
        }
        let dbus = conn.unwrap();
        let response = dbus.send_with_reply_and_block(dbus_msg.unwrap().append2(&*id, &*convid), 2000).unwrap();
        // getAccountList returns one argument, which is an array of strings.
        let contacts: Array<Dict<&str, &str, _>, _> = match response.get1() {
            Some(contacts) => contacts,
            None => return members
        };
        for contact in contacts {
            let mut details = HashMap::new();
            for (key, value) in contact {
                details.insert(String::from(key), String::from(value));
            }
            members.push(details);
        }
        members
    }

    /**
     * Start conversation
     * @param id        Id of the account
     */
    pub fn start_conversation(id: &String) -> String {
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "startConversation");
        if !dbus_msg.is_ok() {
            error!("startConversation fails. Please verify daemon's API.");
            return String::new();
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return String::new();
        }
        let dbus = conn.unwrap();
        let response = dbus.send_with_reply_and_block(dbus_msg.unwrap().append1(&*id), 2000).unwrap();
        response.get1().unwrap_or(String::new())
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
     * Asynchronously load a conversation
     * @param account
     * @param conversation
     * @param from              "" if latest else the commit id
     * @param size              0 if all else max number of messages to get
     * @return the id of the request
     */
    pub fn load_conversation(account: &String, conversation: &String, from: &String, size: u32) -> u32 {
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "loadConversationMessages");
        if !dbus_msg.is_ok() {
            error!("loadConversationMessages fails. Please verify daemon's API.");
            return 0;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return 0;
        }
        let dbus = conn.unwrap();
        let response = dbus.send_with_reply_and_block(dbus_msg.unwrap().append3(&*account, &*conversation, &*from).append1(size), 2000).unwrap();
        response.get1().unwrap_or(0)
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

        let removed: bool = match response.get1() {
            Some(removed) => removed,
            None => false
        };
        removed
    }

    /**
     * Invite a member to a conversation
     * @param id        Id of the account
     * @param conv_id   Id of the conversation
     * @param hash      Id of the member to invite
     */
    pub fn add_conversation_member(id: &String, conv_id: &String, hash: &String) {
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "addConversationMember");
        if !dbus_msg.is_ok() {
            error!("addConversationMember fails. Please verify daemon's API.");
            return;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return;
        }
        let dbus = conn.unwrap();
        let _ = dbus.send_with_reply_and_block(dbus_msg.unwrap().append3(&*id, &*conv_id, &*hash), 2000).unwrap();
    }

    /**
     * Remove a member from a conversation
     * @param id        Id of the account
     * @param conv_id   Id of the conversation
     * @param hash      Id of the member to invite
     */
    pub fn rm_conversation_member(id: &String, conv_id: &String, hash: &String) {
        let dbus_msg = Message::new_method_call("cx.ring.Ring", "/cx/ring/Ring/ConfigurationManager",
                                                "cx.ring.Ring.ConfigurationManager",
                                                "rmConversationMember");
        if !dbus_msg.is_ok() {
            error!("rmConversationMember fails. Please verify daemon's API.");
            return;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return;
        }
        let dbus = conn.unwrap();
        let _ = dbus.send_with_reply_and_block(dbus_msg.unwrap().append3(&*id, &*conv_id, &*hash), 2000).unwrap();
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

}

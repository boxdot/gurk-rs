use crate::account::Account;
use crate::app::Event;
use app_dirs::{get_app_dir, AppDataType, AppInfo};
use dbus::arg::{Array, Dict};
use dbus::{BusType, Connection, ConnectionItem, Message};
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Serialize, Deserialize)]
pub struct Profile {
    pub uri: String,
    pub username: String,
    pub display_name: String,
}

impl Profile {
    pub fn new() -> Self {
        Self {
            uri: String::new(),
            username: String::new(),
            display_name: String::new(),
        }
    }

    pub fn bestname(&self) -> String {
        if !self.display_name.is_empty() {
            return self.display_name.clone();
        }
        if !self.username.is_empty() {
            return self.username.clone();
        }
        return self.uri.clone();
    }
}

#[derive(Serialize, Deserialize)]
pub struct ProfileManager {
    pub profiles: HashMap<String, Profile>,
}

impl ProfileManager {
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    pub fn load_from_account(&mut self, account_id: &String) {
        let dest = get_app_dir(
            AppDataType::UserData,
            &AppInfo {
                name: "jami",
                author: "SFL",
            },
            &*format!("{}/profiles", account_id),
        );
        if dest.is_err() {
            return;
        }

        let paths = fs::read_dir(dest.unwrap()).unwrap();

        for path in paths {
            self.load_profile(&path.unwrap().path().to_str().unwrap().to_string());
        }

        let account = Jami::get_account(account_id);
        let mut profile = Profile::new();
        profile.uri = account.hash.clone();
        profile.display_name = account.alias;
        profile.username = account.registered_name;
        self.profiles.insert(account.hash, profile);
    }

    pub fn load_profile(&mut self, path: &String) {
        // TODO better parsing?
        let buf = BufReader::new(File::open(path).unwrap());
        let mut profile = Profile::new();

        for line in buf.lines() {
            let line = line.unwrap();
            if line.starts_with("FN:") {
                profile.display_name = String::from(line.strip_prefix("FN:").unwrap());
            } else if line.starts_with("TEL") {
                profile.uri = line[(line.len() - 40)..].to_string();
            }
        }

        if self.profiles.contains_key(&profile.uri) {
            profile.username = self.profiles.get(&profile.uri).unwrap().username.clone();
        }

        if !profile.uri.is_empty() {
            self.profiles.insert(profile.uri.clone(), profile);
        }
    }

    pub fn username_found(&mut self, uri: &String, username: &String) {
        if self.profiles.contains_key(uri) {
            let mut profile = self.profiles.get_mut(uri).unwrap();
            profile.username = username.to_string();
        } else {
            let mut profile = Profile::new();
            profile.uri = uri.to_string();
            profile.username = username.to_string();
            self.profiles.insert(uri.to_string(), profile);
        }
    }

    pub fn display_name(&self, uri: &String) -> String {
        if self.profiles.contains_key(uri) {
            return self.profiles.get(uri).unwrap().bestname();
        }
        uri.to_string()
    }
}

/**TODO
 */
pub struct Jami {}

#[derive(PartialEq)]
pub enum ImportType {
    None,
    BACKUP,
    NETWORK,
}

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
                dbus_listener
                    .add_match(
                        "interface=cx.ring.Ring.ConfigurationManager,member=incomingTrustRequest",
                    )
                    .unwrap();
                dbus_listener
                    .add_match("interface=cx.ring.Ring.ConfigurationManager,member=accountsChanged")
                    .unwrap();
                dbus_listener.add_match("interface=cx.ring.Ring.ConfigurationManager,member=registrationStateChanged").unwrap();
                dbus_listener
                    .add_match("interface=cx.ring.Ring.ConfigurationManager,member=messageReceived")
                    .unwrap();
                dbus_listener
                    .add_match(
                        "interface=cx.ring.Ring.ConfigurationManager,member=conversationReady",
                    )
                    .unwrap();
                dbus_listener
                    .add_match(
                        "interface=cx.ring.Ring.ConfigurationManager,member=registeredNameFound",
                    )
                    .unwrap();
                dbus_listener.add_match("interface=cx.ring.Ring.ConfigurationManager,member=conversationRequestReceived").unwrap();
                dbus_listener
                    .add_match(
                        "interface=cx.ring.Ring.ConfigurationManager,member=conversationLoaded",
                    )
                    .unwrap();
                dbus_listener
                    .add_match("interface=cx.ring.Ring.ConfigurationManager,member=profileReceived")
                    .unwrap();
                dbus_listener
                    .add_match("interface=cx.ring.Ring.ConfigurationManager,member=accountsChanged")
                    .unwrap();
                // For each signals, call handlers.
                for ci in dbus_listener.iter(100) {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let msg = if let ConnectionItem::Signal(ref signal) = ci {
                        signal
                    } else {
                        continue;
                    };
                    if &*msg.interface().unwrap() != "cx.ring.Ring.ConfigurationManager" {
                        continue;
                    };
                    if &*msg.member().unwrap() == "messageReceived" {
                        let (account_id, conversation_id, payloads_dict) =
                            msg.get3::<&str, &str, Dict<&str, &str, _>>();
                        let mut payloads: HashMap<String, String> = HashMap::new();
                        for (key, value) in payloads_dict.unwrap() {
                            payloads.insert(String::from(key), String::from(value));
                        }
                        events.push(Event::Message {
                            account_id: String::from(account_id.unwrap()),
                            conversation_id: String::from(conversation_id.unwrap()),
                            payloads,
                        });
                    } else if &*msg.member().unwrap() == "registrationStateChanged" {
                        let (account_id, registration_state, _, _) =
                            msg.get4::<&str, &str, u64, &str>();
                        events.push(Event::RegistrationStateChanged(
                            String::from(account_id.unwrap()),
                            String::from(registration_state.unwrap()),
                        ));
                    } else if &*msg.member().unwrap() == "conversationReady" {
                        let (account_id, conversation_id) = msg.get2::<&str, &str>();
                        events.push(Event::ConversationReady(
                            String::from(account_id.unwrap()),
                            String::from(conversation_id.unwrap()),
                        ));
                    } else if &*msg.member().unwrap() == "registeredNameFound" {
                        let (account_id, status, address, name) =
                            msg.get4::<&str, i32, &str, &str>();
                        events.push(Event::RegisteredNameFound(
                            String::from(account_id.unwrap()),
                            status.unwrap() as u64,
                            String::from(address.unwrap()),
                            String::from(name.unwrap()),
                        ));
                    } else if &*msg.member().unwrap() == "conversationRequestReceived" {
                        let (account_id, conversation_id, _) =
                            msg.get3::<&str, &str, Dict<&str, &str, _>>();
                        events.push(Event::ConversationRequest(
                            String::from(account_id.unwrap()),
                            String::from(conversation_id.unwrap()),
                        ));
                    } else if &*msg.member().unwrap() == "profileReceived" {
                        let (account_id, from, path) = msg.get3::<&str, &str, &str>();
                        events.push(Event::ProfileReceived(
                            String::from(account_id.unwrap()),
                            String::from(from.unwrap()),
                            String::from(path.unwrap()),
                        ));
                    } else if &*msg.member().unwrap() == "accountsChanged" {
                        events.push(Event::AccountsChanged());
                    } else if &*msg.member().unwrap() == "conversationLoaded" {
                        let (id, account_id, conversation_id, messages_dbus) =
                            msg.get4::<u32, &str, &str, Array<Dict<&str, &str, _>, _>>();
                        let mut messages = Vec::new();
                        for message_dbus in messages_dbus.unwrap() {
                            let mut message: HashMap<String, String> = HashMap::new();
                            for (key, value) in message_dbus {
                                message.insert(String::from(key), String::from(value));
                            }
                            messages.push(message);
                        }
                        events.push(Event::ConversationLoaded(
                            id.unwrap(),
                            String::from(account_id.unwrap()),
                            String::from(conversation_id.unwrap()),
                            messages,
                        ));
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
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "lookupName",
        );
        if !dbus_msg.is_ok() {
            error!("lookupName fails. Please verify daemon's API.");
            return false;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return false;
        }
        let dbus = conn.unwrap();
        let _ = dbus
            .send_with_reply_and_block(
                dbus_msg.unwrap().append3(&*account, &*name_service, &*name),
                2000,
            )
            .unwrap();
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
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "lookupAddress",
        );
        if !dbus_msg.is_ok() {
            error!("lookupAddress fails. Please verify daemon's API.");
            return false;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return false;
        }
        let dbus = conn.unwrap();
        let _ = dbus
            .send_with_reply_and_block(
                dbus_msg
                    .unwrap()
                    .append3(&*account, &*name_service, &*address),
                2000,
            )
            .unwrap();
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
    pub fn add_account(main_info: &str, password: &str, import_type: ImportType) -> String {
        let mut details: HashMap<&str, &str> = HashMap::new();
        if import_type == ImportType::BACKUP {
            details.insert("Account.archivePath", main_info);
        } else if import_type == ImportType::NETWORK {
            details.insert("Account.archivePin", main_info);
        } else {
            details.insert("Account.alias", main_info);
        }
        details.insert("Account.type", "RING");
        details.insert("Account.archivePassword", password);
        let details = Dict::new(details.iter());
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "addAccount",
        );
        if !dbus_msg.is_ok() {
            error!("addAccount fails. Please verify daemon's API.");
            return String::new();
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return String::new();
        }
        let dbus = conn.unwrap();
        let response = dbus
            .send_with_reply_and_block(dbus_msg.unwrap().append1(details), 2000)
            .unwrap();
        // addAccount returns one argument, which is a string.
        let account_added: &str = match response.get1() {
            Some(account) => account,
            None => "",
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
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "getAccountList",
        );
        if !dbus_msg.is_ok() {
            error!("getAccountList fails. Please verify daemon's API.");
            return account_list;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return account_list;
        }
        let dbus = conn.unwrap();
        let response = dbus
            .send_with_reply_and_block(dbus_msg.unwrap(), 2000)
            .unwrap();
        // getAccountList returns one argument, which is an array of strings.
        let accounts: Array<&str, _> = match response.get1() {
            Some(array) => array,
            None => return account_list,
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
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "getAccountDetails",
        );
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
        let response = dbus
            .send_with_reply_and_block(dbus_msg.unwrap().append1(id), 2000)
            .ok()
            .expect("Is the ring-daemon launched?");
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
                    if key == "Account.registeredName" {
                        account.registered_name = String::from(value);
                    }
                }
            }
        }
        account
    }

    /**
     * Remove an account
     * @param id the account id to remove
     */
    pub fn rm_account(id: &str) {
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "removeAccount",
        );
        if !dbus_msg.is_ok() {
            error!("removeAccount fails. Please verify daemon's API.");
            return;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            error!("connection not ok.");
            return;
        }
        let dbus = conn.unwrap();
        let _ = dbus
            .send_with_reply_and_block(dbus_msg.unwrap().append1(id), 2000)
            .ok()
            .expect("Is the ring-daemon launched?");
    }

    /**
     * Get account details
     * @param id the account id to build
     * @return the account details
     */
    pub fn get_account_details(id: &str) -> HashMap<String, String> {
        let mut result = HashMap::new();
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "getAccountDetails",
        );
        if !dbus_msg.is_ok() {
            error!("getAccountDetails fails. Please verify daemon's API.");
            return result;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            error!("connection not ok.");
            return result;
        }
        let dbus = conn.unwrap();
        let response = dbus
            .send_with_reply_and_block(dbus_msg.unwrap().append1(id), 2000)
            .ok()
            .expect("Is the ring-daemon launched?");
        let details: Dict<&str, &str, _> = match response.get1() {
            Some(details) => details,
            None => {
                return result;
            }
        };

        for (key, value) in details {
            result.insert(String::from(key), String::from(value));
        }
        result
    }

    /**
     * Get account details
     * @param id the account id to build
     * @return the account details
     */
    pub fn set_account_details(id: &str, details: HashMap<String, String>) {
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "setAccountDetails",
        );
        if !dbus_msg.is_ok() {
            error!("setAccountDetails fails. Please verify daemon's API.");
            return;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            error!("connection not ok.");
            return;
        }
        let dbus = conn.unwrap();
        let _ = dbus
            .send_with_reply_and_block(dbus_msg.unwrap().append2(id, details), 2000)
            .ok()
            .expect("Is the ring-daemon launched?");
    }

    /**
     * Get current members for a conversation
     * @param id        Id of the account
     * @param convid    Id of the conversation
     * @return current members
     */
    pub fn get_members(id: &String, convid: &String) -> Vec<HashMap<String, String>> {
        let mut members: Vec<HashMap<String, String>> = Vec::new();
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "getConversationMembers",
        );
        if !dbus_msg.is_ok() {
            error!("getConversationMembers fails. Please verify daemon's API.");
            return members;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return members;
        }
        let dbus = conn.unwrap();
        let response = dbus
            .send_with_reply_and_block(dbus_msg.unwrap().append2(&*id, &*convid), 2000)
            .unwrap();
        // getAccountList returns one argument, which is an array of strings.
        let contacts: Array<Dict<&str, &str, _>, _> = match response.get1() {
            Some(contacts) => contacts,
            None => return members,
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
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "startConversation",
        );
        if !dbus_msg.is_ok() {
            error!("startConversation fails. Please verify daemon's API.");
            return String::new();
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return String::new();
        }
        let dbus = conn.unwrap();
        let response = dbus
            .send_with_reply_and_block(dbus_msg.unwrap().append1(&*id), 2000)
            .unwrap();
        response.get1().unwrap_or(String::new())
    }

    /**
     * Get current conversations for account
     * @param id        Id of the account
     * @return current conversations
     */
    pub fn get_conversations(id: &String) -> Vec<String> {
        let mut conversations: Vec<String> = Vec::new();
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "getConversations",
        );
        if !dbus_msg.is_ok() {
            error!("getConversations fails. Please verify daemon's API.");
            return conversations;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return conversations;
        }
        let dbus = conn.unwrap();
        let response = dbus
            .send_with_reply_and_block(dbus_msg.unwrap().append1(&*id), 2000)
            .unwrap();
        // getAccountList returns one argument, which is an array of strings.
        let conv_resp: Array<&str, _> = match response.get1() {
            Some(conversations) => conversations,
            None => return conversations,
        };
        for conv in conv_resp {
            conversations.push(String::from(conv));
        }
        conversations
    }

    /**
     * Get current conversations requests for account
     * @param id        Id of the account
     * @return current conversations requests
     */
    pub fn get_conversations_requests(id: &String) -> Vec<HashMap<String, String>> {
        let mut requests: Vec<HashMap<String, String>> = Vec::new();
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "getConversationRequests",
        );
        if !dbus_msg.is_ok() {
            error!("getConversationRequests fails. Please verify daemon's API.");
            return requests;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return requests;
        }
        let dbus = conn.unwrap();
        let response = dbus
            .send_with_reply_and_block(dbus_msg.unwrap().append1(&*id), 2000)
            .unwrap();
        // getAccountList returns one argument, which is an array of strings.
        let requests_rep: Array<Dict<&str, &str, _>, _> = match response.get1() {
            Some(requests) => requests,
            None => return requests,
        };
        for req in requests_rep {
            let mut request = HashMap::new();
            for (key, value) in req {
                request.insert(String::from(key), String::from(value));
            }
            requests.push(request);
        }
        requests
    }

    /**
     * Decline a conversation request
     * @param id        Id of the account
     * @param conv_id   Id of the conversation
     */
    pub fn decline_request(id: &String, conv_id: &String) {
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "declineConversationRequest",
        );
        if !dbus_msg.is_ok() {
            error!("declineConversationRequest fails. Please verify daemon's API.");
            return;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return;
        }
        let dbus = conn.unwrap();
        let _ = dbus
            .send_with_reply_and_block(dbus_msg.unwrap().append2(&*id, &*conv_id), 2000)
            .unwrap();
    }

    /**
     * Accept a conversation request
     * @param id        Id of the account
     * @param conv_id   Id of the conversation
     */
    pub fn accept_request(id: &String, conv_id: &String) {
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "acceptConversationRequest",
        );
        if !dbus_msg.is_ok() {
            error!("acceptConversationRequest fails. Please verify daemon's API.");
            return;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return;
        }
        let dbus = conn.unwrap();
        let _ = dbus
            .send_with_reply_and_block(dbus_msg.unwrap().append2(&*id, &*conv_id), 2000)
            .unwrap();
    }

    /**
     * Asynchronously load a conversation
     * @param account
     * @param conversation
     * @param from              "" if latest else the commit id
     * @param size              0 if all else max number of messages to get
     * @return the id of the request
     */
    pub fn load_conversation(
        account: &String,
        conversation: &String,
        from: &String,
        size: u32,
    ) -> u32 {
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "loadConversationMessages",
        );
        if !dbus_msg.is_ok() {
            error!("loadConversationMessages fails. Please verify daemon's API.");
            return 0;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return 0;
        }
        let dbus = conn.unwrap();
        let response = dbus
            .send_with_reply_and_block(
                dbus_msg
                    .unwrap()
                    .append3(&*account, &*conversation, &*from)
                    .append1(size),
                2000,
            )
            .unwrap();
        response.get1().unwrap_or(0)
    }

    /**
     * Remove a conversation for an account
     * @param id        Id of the account
     * @param conv_id   Id of the conversation
     * @return if the conversation is removed
     */
    pub fn rm_conversation(id: &String, conv_id: &String) -> bool {
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "removeConversation",
        );
        if !dbus_msg.is_ok() {
            error!("getAccountList fails. Please verify daemon's API.");
            return false;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return false;
        }
        let dbus = conn.unwrap();
        let response = dbus
            .send_with_reply_and_block(dbus_msg.unwrap().append2(&*id, &*conv_id), 2000)
            .unwrap();

        let removed: bool = match response.get1() {
            Some(removed) => removed,
            None => false,
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
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "addConversationMember",
        );
        if !dbus_msg.is_ok() {
            error!("addConversationMember fails. Please verify daemon's API.");
            return;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return;
        }
        let dbus = conn.unwrap();
        let _ = dbus
            .send_with_reply_and_block(dbus_msg.unwrap().append3(&*id, &*conv_id, &*hash), 2000)
            .unwrap();
    }

    /**
     * Remove a member from a conversation
     * @param id        Id of the account
     * @param conv_id   Id of the conversation
     * @param hash      Id of the member to invite
     */
    pub fn rm_conversation_member(id: &String, conv_id: &String, hash: &String) {
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "rmConversationMember",
        );
        if !dbus_msg.is_ok() {
            error!("rmConversationMember fails. Please verify daemon's API.");
            return;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return;
        }
        let dbus = conn.unwrap();
        let _ = dbus
            .send_with_reply_and_block(dbus_msg.unwrap().append3(&*id, &*conv_id, &*hash), 2000)
            .unwrap();
    }

    /**
     * Remove a conversation for an account
     * @param id        Id of the account
     * @param conv_id   Id of the conversation
     * @param hash      Id of the member to invite
     * @param hash      Id of the member to invite
     */
    pub fn send_conversation_message(
        id: &String,
        conv_id: &String,
        message: &String,
        parent: &String,
    ) -> u64 {
        let dbus_msg = Message::new_method_call(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            "cx.ring.Ring.ConfigurationManager",
            "sendMessage",
        );
        if !dbus_msg.is_ok() {
            error!("getAccountList fails. Please verify daemon's API.");
            return 0;
        }
        let conn = Connection::get_private(BusType::Session);
        if !conn.is_ok() {
            return 0;
        }
        let dbus = conn.unwrap();
        let response = dbus
            .send_with_reply_and_block(
                dbus_msg
                    .unwrap()
                    .append3(&*id, &*conv_id, &*message)
                    .append1(&*parent),
                2000,
            )
            .unwrap();
        response.get1().unwrap_or(0) as u64
    }
}

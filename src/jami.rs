use crate::app::Event;
use crate::account::Account;
use dbus::{Connection, ConnectionItem, BusType, Message};
use dbus::arg::{Array, Dict};
use log::{debug, error, log_enabled, info, Level};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::io::Read;

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

    // Helpers

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

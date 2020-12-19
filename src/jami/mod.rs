pub mod account;
pub mod profile;
pub mod profilemanager;

pub use profile::Profile;
pub use profilemanager::ProfileManager;

use crate::util::Event;
use account::Account;

use dbus::blocking::Connection;
use dbus::message::MatchRule;
use dbus_tokio::connection;
use log::info;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::{thread, time};

/**
 * Connect to the jami daemon
 */
pub struct Jami {}

#[derive(PartialEq)]
pub enum ImportType {
    None,
    BACKUP,
    NETWORK,
}

impl Jami {
    /**
     * Retrieve account or create one if necessary.
     * @param   create_if_not   Create if no account found
     * @return the account
     */
    pub fn select_jami_account(create_if_not: bool) -> Account {
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

    /**
     * Listen to daemon's signals
     */
    pub async fn handle_events<T: 'static + std::fmt::Debug + std::marker::Send>(
        tx: tokio::sync::mpsc::Sender<Event<T>>,
        stop: Arc<AtomicBool>,
    ) -> Result<(), std::io::Error> {
        let (resource, conn) = connection::new_session_sync()
            .ok()
            .expect("Lost connection");
        tokio::spawn(async {
            let err = resource.await;
            panic!("Lost connection to D-Bus: {}", err);
        });

        let mr = MatchRule::new_signal("cx.ring.Ring.ConfigurationManager", "accountsChanged");
        let txs = tx.clone();
        let _ic = conn
            .add_match(mr)
            .await
            .ok()
            .expect("Lost connection")
            .cb(move |_, (): ()| {
                let mut txs = txs.clone();
                tokio::spawn(async move { txs.send(Event::AccountsChanged()).await });
                true
            });

        let mr = MatchRule::new_signal("cx.ring.Ring.ConfigurationManager", "messageReceived");
        let txs = tx.clone();
        let _ic = conn.add_match(mr).await.ok().expect("Lost connection").cb(
            move |_,
                  (account_id, conversation_id, payloads): (
                String,
                String,
                HashMap<String, String>,
            )| {
                let mut txs = txs.clone();
                tokio::spawn(async move {
                    txs.send(Event::Message {
                        account_id,
                        conversation_id,
                        payloads,
                    })
                    .await
                });
                true
            },
        );

        let mr = MatchRule::new_signal(
            "cx.ring.Ring.ConfigurationManager",
            "registrationStateChanged",
        );
        let txs = tx.clone();
        let _ic = conn.add_match(mr).await.ok().expect("Lost connection").cb(
            move |_, (account_id, registration_state, _, _): (String, String, u64, String)| {
                let mut txs = txs.clone();
                tokio::spawn(async move {
                    txs.send(Event::RegistrationStateChanged(
                        account_id,
                        registration_state,
                    ))
                    .await
                });
                true
            },
        );

        let mr = MatchRule::new_signal("cx.ring.Ring.ConfigurationManager", "conversationReady");
        let txs = tx.clone();
        let _ic = conn.add_match(mr).await.ok().expect("Lost connection").cb(
            move |_, (account_id, conversation_id): (String, String)| {
                let mut txs = txs.clone();
                tokio::spawn(async move {
                    txs.send(Event::ConversationReady(account_id, conversation_id))
                        .await
                });
                true
            },
        );

        let mr = MatchRule::new_signal(
            "cx.ring.Ring.ConfigurationManager",
            "conversationRequestReceived",
        );
        let txs = tx.clone();
        let _ic = conn.add_match(mr).await.ok().expect("Lost connection").cb(
            move |_, (account_id, conversation_id): (String, String)| {
                let mut txs = txs.clone();
                tokio::spawn(async move {
                    txs.send(Event::ConversationRequest(account_id, conversation_id))
                        .await
                });
                true
            },
        );

        let mr = MatchRule::new_signal("cx.ring.Ring.ConfigurationManager", "registeredNameFound");
        let txs = tx.clone();
        let _ic = conn.add_match(mr).await.ok().expect("Lost connection").cb(
            move |_, (account_id, status, address, name): (String, i32, String, String)| {
                let mut txs = txs.clone();
                tokio::spawn(async move {
                    txs.send(Event::RegisteredNameFound(
                        account_id,
                        status as u64,
                        address,
                        name,
                    ))
                    .await
                });
                true
            },
        );

        let mr = MatchRule::new_signal("cx.ring.Ring.ConfigurationManager", "profileReceived");
        let txs = tx.clone();
        let _ic = conn.add_match(mr).await.ok().expect("Lost connection").cb(
            move |_, (account_id, from, path): (String, String, String)| {
                let mut txs = txs.clone();
                tokio::spawn(async move {
                    txs.send(Event::ProfileReceived(account_id, from, path))
                        .await
                });
                true
            },
        );

        let mr = MatchRule::new_signal("cx.ring.Ring.ConfigurationManager", "incomingTrustRequest");
        let txs = tx.clone();
        let _ic = conn.add_match(mr).await.ok().expect("Lost connection").cb(
            move |_, (account_id, from, payloads, receive_time): (String, String, Vec<u8>, u64)| {
                let mut txs = txs.clone();
                tokio::spawn(async move {
                    txs.send(Event::IncomingTrustRequest(
                        account_id,
                        from,
                        payloads,
                        receive_time,
                    ))
                    .await
                });
                true
            },
        );

        let mr = MatchRule::new_signal("cx.ring.Ring.ConfigurationManager", "conversationLoaded");
        let txs = tx.clone();
        let _ic = conn.add_match(mr).await.ok().expect("Lost connection").cb(
            move |_,
                  (id, account_id, conversation_id, messages): (
                u32,
                String,
                String,
                Vec<HashMap<String, String>>,
            )| {
                let mut txs = txs.clone();
                tokio::spawn(async move {
                    txs.send(Event::ConversationLoaded(
                        id,
                        account_id,
                        conversation_id,
                        messages,
                    ))
                    .await
                });
                true
            },
        );

        let ten_millis = time::Duration::from_millis(10);
        loop {
            thread::sleep(ten_millis);
            if stop.load(Ordering::Relaxed) {
                break;
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
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(bool,), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "lookupName",
            (account, name_service, name),
        );
        if result.is_ok() {
            let result = result.unwrap().0;
            return result;
        }
        false
    }

    /**
     * Asynchronously lookup an address
     * @param account
     * @param name_service
     * @param address
     * @return if dbus is ok
     */
    pub fn lookup_address(account: &String, name_service: &String, address: &String) -> bool {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(bool,), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "lookupAddress",
            (account, name_service, address),
        );
        if result.is_ok() {
            let result = result.unwrap().0;
            return result;
        }
        false
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
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(String,), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "addAccount",
            (details,),
        );
        if result.is_ok() {
            let result = result.unwrap().0;
            info!("New account: {:?}", result);
            return result;
        }

        String::new()
    }

    /**
     * Get current ring accounts
     * @return current accounts
     */
    pub fn get_account_list() -> Vec<Account> {
        let mut account_list: Vec<Account> = Vec::new();
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(Vec<String>,), _> =
            proxy.method_call("cx.ring.Ring.ConfigurationManager", "getAccountList", ());
        if result.is_err() {
            return account_list;
        }
        let accounts = result.unwrap().0;
        for account in accounts {
            account_list.push(Jami::get_account(&*account));
        }
        account_list
    }

    /**
     * Build a new account with an id from the daemon
     * @param id the account id to build
     * @return the account retrieven
     */
    pub fn get_account(id: &str) -> Account {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(HashMap<String, String>,), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "getAccountDetails",
            (id,),
        );
        if result.is_err() {
            return Account::null();
        }
        let details = result.unwrap().0;

        let mut account = Account::null();
        account.id = id.to_owned();
        for detail in details {
            match detail {
                (key, value) => {
                    if key == "Account.enable" {
                        account.enabled = value == "true";
                    }
                    if key == "Account.alias" {
                        account.alias = value.clone();
                    }
                    if key == "Account.username" {
                        account.hash = value.clone().replace("ring:", "");
                    }
                    if key == "Account.registeredName" {
                        account.registered_name = value.clone();
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
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let _: Result<(), _> =
            proxy.method_call("cx.ring.Ring.ConfigurationManager", "removeAccount", (id,));
    }

    /**
     * Get account details
     * @param id the account id to build
     * @return the account details
     */
    pub fn get_account_details(id: &str) -> HashMap<String, String> {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(HashMap<String, String>,), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "getAccountDetails",
            (id,),
        );
        if result.is_ok() {
            let result = result.unwrap().0;
            return result;
        }

        HashMap::new()
    }

    /**
     * Get account details
     * @param id the account id to build
     * @return the account details
     */
    pub fn set_account_details(id: &str, details: HashMap<String, String>) {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let _: Result<(), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "setAccountDetails",
            (id, details),
        );
    }

    /**
     * Add a new contact
     * @param id        Account id
     * @param uri       Uri of the contact
     */
    pub fn add_contact(id: &String, uri: &String) {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let _: Result<(), _> =
            proxy.method_call("cx.ring.Ring.ConfigurationManager", "addContact", (id, uri));
    }

    /**
     * Get trusts requests from an account
     * @param id        Account id
     * @return the list of trusts requests senders
     */
    pub fn get_trust_requests(id: &String) -> Vec<String> {
        let mut res = Vec::new();
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(Vec<HashMap<String, String>>,), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "getTrustRequests",
            (id,),
        );
        if result.is_ok() {
            let result = result.unwrap().0;
            for tr in result {
                if tr.contains_key("from") {
                    res.push(tr.get("from").unwrap().clone());
                }
            }
        }
        return res;
    }

    /**
     * Send a trust request to someone
     * @param id        Account id
     * @param to        Contact uri
     * @param payloads  VCard
     */
    pub fn send_trust_request(id: &String, to: &String, payloads: Vec<u8>) {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let _: Result<(), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "sendTrustRequest",
            (id, to, payloads),
        );
    }

    /**
     * Accept a trust request
     * @param id        Account id
     * @param from      Contact uri
     * @return if successful
     */
    pub fn accept_trust_request(id: &String, from: &String) -> bool {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(bool,), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "acceptTrustRequest",
            (id, from),
        );
        if result.is_ok() {
            let result = result.unwrap().0;
            return result;
        }
        false
    }

    /**
     * Discard a trust request
     * @param id        Account id
     * @param from      Contact uri
     * @return if successful
     */
    pub fn discard_trust_request(id: &String, from: &String) -> bool {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(bool,), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "discardTrustRequest",
            (id, from),
        );
        if result.is_ok() {
            let result = result.unwrap().0;
            return result;
        }
        false
    }

    /**
     * Get current members for a conversation
     * @param id        Id of the account
     * @param convid    Id of the conversation
     * @return current members
     */
    pub fn get_members(id: &String, convid: &String) -> Vec<HashMap<String, String>> {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(Vec<HashMap<String, String>>,), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "getConversationMembers",
            (id, convid),
        );
        if result.is_ok() {
            let result = result.unwrap().0;
            return result;
        }

        Vec::new()
    }

    /**
     * Start conversation
     * @param id        Id of the account
     */
    pub fn start_conversation(id: &String) -> String {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(String,), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "startConversation",
            (id,),
        );
        if result.is_ok() {
            let result = result.unwrap().0;
            return result;
        }

        String::new()
    }

    /**
     * Get current conversations for account
     * @param id        Id of the account
     * @return current conversations
     */
    pub fn get_conversations(id: &String) -> Vec<String> {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(Vec<String>,), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "getConversations",
            (id,),
        );
        if result.is_ok() {
            let result = result.unwrap().0;
            return result;
        }

        Vec::new()
    }

    /**
     * Get current conversations requests for account
     * @param id        Id of the account
     * @return current conversations requests
     */
    pub fn get_conversations_requests(id: &String) -> Vec<HashMap<String, String>> {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(Vec<HashMap<String, String>>,), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "getConversationRequests",
            (id,),
        );
        if result.is_ok() {
            let result = result.unwrap().0;
            return result;
        }

        Vec::new()
    }

    /**
     * Decline a conversation request
     * @param id        Id of the account
     * @param conv_id   Id of the conversation
     */
    pub fn decline_request(id: &String, conv_id: &String) {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let _: Result<(), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "declineConversationRequest",
            (id, conv_id),
        );
    }

    /**
     * Accept a conversation request
     * @param id        Id of the account
     * @param conv_id   Id of the conversation
     */
    pub fn accept_request(id: &String, conv_id: &String) {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let _: Result<(), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "acceptConversationRequest",
            (id, conv_id),
        );
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
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(u32,), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "loadConversationMessages",
            (account, conversation, from, size),
        );
        if result.is_ok() {
            let result = result.unwrap().0;
            return result;
        }
        0
    }

    /**
     * Remove a conversation for an account
     * @param id        Id of the account
     * @param conv_id   Id of the conversation
     * @return if the conversation is removed
     */
    pub fn rm_conversation(id: &String, conv_id: &String) -> bool {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(bool,), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "removeConversation",
            (id, conv_id),
        );
        if result.is_ok() {
            let result = result.unwrap().0;
            return result;
        }
        false
    }

    /**
     * Invite a member to a conversation
     * @param id        Id of the account
     * @param conv_id   Id of the conversation
     * @param hash      Id of the member to invite
     */
    pub fn add_conversation_member(id: &String, conv_id: &String, hash: &String) {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let _: Result<(), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "addConversationMember",
            (id, conv_id, hash),
        );
    }

    /**
     * Remove a member from a conversation
     * @param id        Id of the account
     * @param conv_id   Id of the conversation
     * @param hash      Id of the member to invite
     */
    pub fn rm_conversation_member(id: &String, conv_id: &String, hash: &String) {
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let _: Result<(), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "rmConversationMember",
            (id, conv_id, hash),
        );
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
        let conn = Connection::new_session().unwrap();
        let proxy = conn.with_proxy(
            "cx.ring.Ring",
            "/cx/ring/Ring/ConfigurationManager",
            Duration::from_millis(5000),
        );
        let result: Result<(u64,), _> = proxy.method_call(
            "cx.ring.Ring.ConfigurationManager",
            "sendMessage",
            (id, conv_id, message, parent),
        );
        if result.is_ok() {
            return result.unwrap().0;
        }
        0
    }
}

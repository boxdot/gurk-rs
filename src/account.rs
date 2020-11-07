use std::fmt;
use serde::{Serialize, Deserialize};

/**
 * Represent a Jami account, just here to store informations.
 **/
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Account {
    pub id: String,
    pub hash: String,
    pub alias: String,
    pub enabled: bool,
}

// Used for println!
impl fmt::Display for Account {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[{}]: {} ({}) - Active: {}", self.id, self.hash, self.alias, self.enabled)
    }
}

impl Account {
    pub fn null() -> Account {
        Account {
            id: String::new(),
            hash: String::new(),
            alias: String::new(),
            enabled: false,
        }
    }

    pub fn get_display_name(&self) -> String {
        if !self.alias.is_empty() {
            return self.alias.clone();
        }
        return self.hash.clone();
    }
}
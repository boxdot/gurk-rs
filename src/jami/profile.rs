use serde::{Deserialize, Serialize};

/**
 * Represent a User Profile, just here to store informations. Cf ProfileManager for logic
 **/
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

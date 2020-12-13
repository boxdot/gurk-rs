use super::profile::Profile;

use app_dirs::{get_app_dir, AppDataType, AppInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};

/**
 * A class used to share user profiles for all Jami accounts
 **/
#[derive(Serialize, Deserialize)]
pub struct ProfileManager {
    pub profiles: HashMap<String, Profile>,
}

impl ProfileManager {
    /**
     * Generate a new ProfileManager
     * @return the new manager
     */
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    /**
     * Load all profiles stored for an account (jami::app_data + profiles)
     * @param account   Id of the account
     */
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

        let paths = fs::read_dir(dest.unwrap());
        if paths.is_err() {
            return;
        }
        let paths = paths.unwrap();

        for path in paths {
            self.load_profile(&path.unwrap().path().to_str().unwrap().to_string());
        }
    }

    /**
     * Load one profile
     * @param path   Path to load
     */
    pub fn load_profile(&mut self, path: &String) {
        // TODO better parsing?
        // For now we don't care about the full vcard file
        // and current Rust libs seems bugguy
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

    /**
     * Modify the username stored (after a lookup for example)
     * @param uri       Contact to modify
     * @param username  New username for this user
     */
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

    /**
     * Return the display name for a user
     * @param uri        Id of the user
     * @return The display name
     */
    pub fn display_name(&self, uri: &String) -> String {
        if self.profiles.contains_key(uri) {
            return self.profiles.get(uri).unwrap().bestname();
        }
        uri.to_string()
    }
}

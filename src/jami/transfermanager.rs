use serde::{Deserialize, Serialize};
use std::collections::HashMap;


#[derive(Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub display_name: String,
    pub path: String,
    pub last_event: i32
}

/**
 * A class used to store transfers per account per conversation
 **/
#[derive(Serialize, Deserialize)]
pub struct TransferManager {
    pub account_id: String,
    pub conv_id: String,
    pub transfers: HashMap<String, FileInfo>,
}

impl TransferManager {
    /**
     * Generate a new TransferManager
     * @return the new manager
     */
    pub fn new(account_id: String, conv_id: String) -> Self {
        // TODO load from file
        Self {
            account_id,
            conv_id,
            transfers: HashMap::new(),
        }
    }

    pub fn save(&mut self) {
    }

    pub fn file_info(&mut self, id: String) -> Option<FileInfo> {
        let info = self.transfers.get(&id);
        if info.is_none() {
            return None;
        }
        Some(info.unwrap().clone())
    }

    pub fn add_file_info(&mut self, id: String, info: FileInfo) {
        self.transfers.insert(id, info);
    }

}

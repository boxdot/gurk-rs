use std::collections::{HashMap, HashSet};

use get_size2::GetSize;
use serde::{Deserialize, Serialize};
use tracing::error;
use uuid::Uuid;

use crate::signal::SignalManager;

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, GetSize,
)]
pub enum Receipt {
    Sent = -1,
    Delivered = 0,
    Read = 1,
    #[default]
    #[serde(other)]
    Nothing = -2, // Do not do anything to these receipts in order to avoid spamming receipt messages when an old database is loaded
}

impl Receipt {
    pub fn from_i32(i: i32) -> Self {
        match i {
            0 => Self::Delivered,
            1 => Self::Read,
            _ => Self::Nothing,
        }
    }

    pub fn to_i32(self) -> i32 {
        match self {
            Self::Read => 1,
            _ => 0,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReceiptHandler {
    receipt_set: HashMap<Uuid, ReceiptQueues>,
    time_since_update: u64,
}

/// This get built anywhere in the client and get passed to the App
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReceiptEvent {
    uuid: Uuid,
    /// Timestamp of the messages
    timestamp: u64,
    /// Type : Received, Delivered
    receipt_type: Receipt,
}

impl ReceiptEvent {
    pub fn new(uuid: Uuid, timestamp: u64, receipt_type: Receipt) -> Self {
        Self {
            uuid,
            timestamp,
            receipt_type,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReceiptQueues {
    received_msg: HashSet<u64>,
    read_msg: HashSet<u64>,
}

impl ReceiptQueues {
    pub fn new() -> Self {
        Self {
            received_msg: HashSet::new(),
            read_msg: HashSet::new(),
        }
    }

    pub fn add_received(&mut self, timestamp: u64) {
        if !self.received_msg.insert(timestamp) {
            error!("Somehow got duplicate Received receipt @ {}", timestamp);
        }
    }

    pub fn add_read(&mut self, timestamp: u64) {
        // Ensures we do not send uselessly double the amount of receipts
        // in the case a message is immediatly received and read.
        self.received_msg.remove(&timestamp);
        if !self.read_msg.insert(timestamp) {
            error!("Somehow got duplicate Delivered receipt @ {}", timestamp);
        }
    }

    pub fn add(&mut self, timestamp: u64, receipt: Receipt) {
        match receipt {
            Receipt::Delivered => self.add_received(timestamp),
            Receipt::Read => self.add_read(timestamp),
            _ => {}
        }
    }

    pub fn get_data(&mut self) -> Option<(Vec<u64>, Receipt)> {
        if !self.received_msg.is_empty() {
            let timestamps = self.received_msg.drain().collect::<Vec<u64>>();
            return Some((timestamps, Receipt::Delivered));
        }
        if !self.read_msg.is_empty() {
            let timestamps = self.read_msg.drain().collect::<Vec<u64>>();
            return Some((timestamps, Receipt::Read));
        }
        None
    }

    pub fn is_empty(&self) -> bool {
        self.received_msg.is_empty() && self.read_msg.is_empty()
    }
}

impl ReceiptHandler {
    pub fn new() -> Self {
        Self {
            receipt_set: HashMap::new(),
            time_since_update: 0u64,
        }
    }

    pub fn add_receipt_event(&mut self, event: ReceiptEvent) {
        // Add a new set in the case no receipt had been handled for this contact
        // over the current session
        self.receipt_set
            .entry(event.uuid)
            .or_default()
            .add(event.timestamp, event.receipt_type);
    }

    // Dictates whether receipts should be sent on the current tick
    // Not used for now as
    fn do_tick(&mut self) -> bool {
        true
    }

    pub fn step(&mut self, _signal_manager: &dyn SignalManager) -> bool {
        if !self.do_tick() {
            return false;
        }
        if self.receipt_set.is_empty() {
            return false;
        }

        // For now, receipts are disabled
        self.receipt_set.clear();
        false

        // // Get any key
        // let uuid = *self.receipt_set.keys().next().unwrap();
        //
        // let j = self.receipt_set.entry(uuid);
        // match j {
        //     Entry::Occupied(mut e) => {
        //         let u = e.get_mut();
        //         if let Some((timestamps, receipt)) = u.get_data() {
        //             signal_manager.send_receipt(uuid, timestamps, receipt);
        //             if u.is_empty() {
        //                 e.remove_entry();
        //             }
        //             true
        //         } else {
        //             false
        //         }
        //     }
        //     Entry::Vacant(_) => false,
        // }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_receipt_order() {
        assert!(Receipt::Nothing < Receipt::Sent);
        assert!(Receipt::Sent < Receipt::Delivered);
        assert!(Receipt::Delivered < Receipt::Read);
    }

    #[test]
    fn test_receipt_serde() -> anyhow::Result<()> {
        assert_eq!(serde_json::to_string(&Receipt::Nothing)?, "\"Nothing\"");
        assert_eq!(serde_json::to_string(&Receipt::Sent)?, "\"Sent\"");
        assert_eq!(serde_json::to_string(&Receipt::Delivered)?, "\"Delivered\"");
        assert_eq!(serde_json::to_string(&Receipt::Read)?, "\"Read\"");

        let receipt: Receipt = serde_json::from_str("\"Unknown\"")?;
        assert_eq!(receipt, Receipt::Nothing);

        Ok(())
    }
}

use chrono::{DateTime, NaiveDateTime, TimeZone as _, Utc};
use presage::libsignal_service::prelude::phonenumber::PhoneNumber;
use serde::{Deserialize, Serialize};
use tui::widgets::ListState;

#[derive(Serialize, Deserialize)]
pub struct StatefulList<T> {
    #[serde(skip)]
    pub state: ListState,
    pub items: Vec<T>,
}

impl<T> Default for StatefulList<T> {
    fn default() -> Self {
        Self {
            state: Default::default(),
            items: Vec::new(),
        }
    }
}

impl<T> StatefulList<T> {
    pub fn with_items(items: Vec<T>) -> StatefulList<T> {
        StatefulList {
            state: ListState::default(),
            items,
        }
    }

    pub fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i + 1 >= self.items.len() {
                    0
                } else {
                    i + 1
                }
            }
            None => {
                if !self.items.is_empty() {
                    0
                } else {
                    return; // nothing to select
                }
            }
        };
        self.state.select(Some(i));
    }

    pub fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => {
                if !self.items.is_empty() {
                    0
                } else {
                    return; // nothing to select
                }
            }
        };
        self.state.select(Some(i));
    }
}

pub fn timestamp_msec_to_utc(timestamp: u64) -> DateTime<Utc> {
    let dt = NaiveDateTime::from_timestamp(timestamp as i64 / 1000, (timestamp % 1000) as u32);
    Utc.from_utc_datetime(&dt)
}

pub fn utc_now_timestamp_msec() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis() as u64
}

pub fn is_phone_number(s: impl AsRef<str>) -> bool {
    use std::str::FromStr;
    PhoneNumber::from_str(s.as_ref()).is_ok()
}

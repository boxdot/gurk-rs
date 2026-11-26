use std::sync::LazyLock;

use chrono::{DateTime, Datelike, Local};
use phonenumber::PhoneNumber;
use ratatui::widgets::ListState;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{
    data::{Channel, ChannelId},
    storage::Storage,
};

const MESSAGE_SCROLL_BACK: bool = false;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatefulList<T> {
    #[serde(skip)]
    pub state: ListState,
    pub items: Vec<T>,
    #[serde(skip)]
    pub rendered: Rendered,
}

impl<T: PartialEq> PartialEq for StatefulList<T> {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl<T: Eq> Eq for StatefulList<T> {}

#[derive(Debug, Clone, Default)]
pub struct Rendered {
    pub offset: usize,
}

impl<T> Default for StatefulList<T> {
    fn default() -> Self {
        Self {
            state: Default::default(),
            items: Vec::new(),
            rendered: Default::default(),
        }
    }
}

impl<T> StatefulList<T> {
    pub fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i + 1 >= self.items.len() {
                    if MESSAGE_SCROLL_BACK { 0 } else { i }
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
                    if MESSAGE_SCROLL_BACK {
                        self.items.len() - 1
                    } else {
                        0
                    }
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

    pub(crate) fn selected_item(&self) -> Option<&T> {
        let idx = self.state.selected()?;
        Some(&self.items[idx])
    }
}

impl StatefulList<Channel> {
    /// Modifies the channel at the given index and writes it to the storage.
    ///
    /// If the channel is not found, does nothing. Returns `Some(true)` if the channel was modified.
    /// Returns `None` if the channel does not exist.
    pub(crate) fn modify_channel(
        &mut self,
        storage: &mut dyn Storage,
        idx: usize,
        f: impl FnOnce(&mut Channel) -> bool,
    ) -> Option<bool> {
        let channel = self.items.get_mut(idx)?;
        if f(channel) {
            storage.store_channel(channel);
            Some(true)
        } else {
            Some(false)
        }
    }

    /// Modifies the selected channel and writes it to the storage.
    ///
    /// If the channel is not found, does nothing. Returns `Some(true)` if the channel was modified.
    /// Returns `None` if no channel is selected or the channel does not exist.
    pub(crate) fn modify_selected_channel(
        &mut self,
        storage: &mut dyn Storage,
        f: impl FnOnce(&mut Channel) -> bool,
    ) -> Option<bool> {
        let idx = self.state.selected()?;
        self.modify_channel(storage, idx, f)
    }

    /// Modifies the channel with the given id and writes it to the storage.
    ///
    /// If the channel is not found, does nothing. Returns `Some(true)` if the channel was modified.
    /// Returns `None` if the channel does not exist.
    pub(crate) fn modify_channel_by_id(
        &mut self,
        storage: &mut dyn Storage,
        channel_id: ChannelId,
        f: impl FnOnce(&mut Channel) -> bool,
    ) -> Option<bool> {
        let idx = self
            .items
            .iter()
            .position(|channel| channel.id == channel_id)?;
        self.modify_channel(storage, idx, f)
    }
}

/// Converts a UTC timestamp in milliseconds to local time
///
/// Timestamps come from received messages and are not validated by the sender's client, so a
/// timestamp that cannot be represented falls back to the Unix epoch instead of panicking.
pub fn utc_timestamp_msec_to_local(timestamp: u64) -> DateTime<Local> {
    i64::try_from(timestamp)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
        .filter(|datetime| datetime.year() <= MAX_TIMESTAMP_YEAR)
        .unwrap_or(DateTime::UNIX_EPOCH)
        .with_timezone(&Local)
}

/// Latest year accepted by [`utc_timestamp_msec_to_local`]
///
/// Far below chrono's limit, so that converting to any local time zone cannot overflow.
const MAX_TIMESTAMP_YEAR: i32 = 9999;

pub fn utc_now_timestamp_msec() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis() as u64
}

pub fn is_phone_number(s: impl AsRef<str>) -> bool {
    // Note: previously we formatted phone numbers sometimes incorrectly (not always as E164). So,
    // some users might still have them stored with spaces and dashes. So, we strip them here, even
    // the formatting now is correct.
    let stripped = s.as_ref().replace(&[' ', '-'][..], "");
    stripped.parse::<PhoneNumber>().is_ok()
}

// Based on Alacritty, APACHE-2.0 License
pub(crate) static URL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        "(ipfs:|ipns:|magnet:|mailto:|gemini:|gopher:|https:|http:|news:|file:|git:|ssh:|ftp:)\
     [^\u{0000}-\u{001F}\u{007F}-\u{009F}<>\"\\s{-}\\^⟨⟩`]+",
    )
    .unwrap()
});

// Based on Alacritty, APACHE-2.0 License
pub(crate) static ATTACHMENT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("file:[^\u{0000}-\u{001F}\u{007F}-\u{009F}<>\"\\s{-}\\^⟨⟩`]+").unwrap()
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_phone_number() {
        assert!(is_phone_number("+1 800-000-0000"));
    }

    #[test]
    fn test_utc_timestamp_msec_to_local() {
        let datetime = utc_timestamp_msec_to_local(1_700_000_000_123).to_utc();
        assert_eq!(datetime.timestamp_millis(), 1_700_000_000_123);
    }

    #[test]
    fn test_utc_timestamp_msec_to_local_out_of_range() {
        let max_millis = DateTime::<chrono::Utc>::MAX_UTC.timestamp_millis() as u64;
        for timestamp in [u64::MAX, i64::MAX as u64, max_millis, 253_402_300_800_000] {
            let datetime = utc_timestamp_msec_to_local(timestamp);
            assert_eq!(datetime.to_utc(), DateTime::UNIX_EPOCH, "{timestamp}");
            // the date line and attachment directory use these
            let _ = datetime.num_days_from_ce();
            let _ = datetime.date_naive();
        }
        // last representable millisecond of year 9999 is kept
        let datetime = utc_timestamp_msec_to_local(253_402_300_799_999);
        assert_eq!(datetime.to_utc().year(), 9999);
    }
}

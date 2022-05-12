use crate::app::Channel;

use chrono::{DateTime, Local, NaiveDateTime, TimeZone as _, Utc};
use presage::prelude::PhoneNumber;
use regex_automata::Regex;
use serde::{Deserialize, Serialize};
use tui::widgets::ListState;

use crate::MESSAGE_SCROLL_BACK;

#[derive(Debug, Serialize, Deserialize)]
pub struct StatefulList<T> {
    #[serde(skip)]
    pub state: ListState,
    pub items: Vec<T>,
    #[serde(skip)]
    pub rendered: Rendered,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FilteredStatefulList<T> {
    #[serde(skip)]
    pub state: ListState,
    pub items: Vec<T>,
    #[serde(skip)]
    pub filtered_items: Vec<usize>,
    #[serde(skip)]
    pub rendered: Rendered,
}

impl<T: PartialEq> PartialEq for StatefulList<T> {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl<T: Eq> Eq for StatefulList<T> {}

impl<T: PartialEq> PartialEq for FilteredStatefulList<T> {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl<T: Eq> Eq for FilteredStatefulList<T> {}

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

impl FilteredStatefulList<Channel> {
    pub fn filter(&mut self, filter: impl Fn(&Channel) -> bool) {
        self.filter_elements(filter);
        // Update the selected message to not got past the bound of `self.filtered_items`
        self.state.select(if self.filtered_items.is_empty() {
            None
        } else {
            Some((self.filtered_items.len().max(1) - 1).min(self.state.selected().unwrap_or(0)))
        });
    }
}

impl<T> Default for FilteredStatefulList<T> {
    fn default() -> Self {
        Self {
            state: Default::default(),
            items: Vec::new(),
            rendered: Default::default(),
            filtered_items: Vec::new(),
        }
    }
}

pub struct StatefulIterator<'a, T> {
    index: usize,
    list: &'a FilteredStatefulList<T>,
}

impl<'a, T> Iterator for StatefulIterator<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.list.filtered_items.len() {
            None
        } else {
            let res = self.list.items.get(self.list.filtered_items[self.index]);
            self.index += 1;
            res
        }
    }
}

impl<T> StatefulList<T> {
    pub fn with_items(items: Vec<T>) -> StatefulList<T> {
        StatefulList {
            state: ListState::default(),
            items,
            rendered: Default::default(),
        }
    }

    pub fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i + 1 >= self.items.len() {
                    if MESSAGE_SCROLL_BACK {
                        0
                    } else {
                        i
                    }
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
}

impl<T> FilteredStatefulList<T> {
    pub fn iter(&'_ self) -> StatefulIterator<T> {
        StatefulIterator {
            index: 0,
            list: self,
        }
    }

    pub fn _with_items(items: Vec<T>) -> FilteredStatefulList<T> {
        FilteredStatefulList {
            state: ListState::default(),
            items,
            filtered_items: Default::default(),
            rendered: Default::default(),
        }
    }

    pub fn _get(&self, index: usize) -> Option<&T> {
        if let Some(i) = self.filtered_items.get(index) {
            self.items.get(*i)
        } else {
            None
        }
    }

    pub fn _get_mut(&self, index: usize) -> Option<&T> {
        if let Some(i) = self.filtered_items.get(index) {
            self.items.get(*i)
        } else {
            None
        }
    }

    pub fn filter_elements(&mut self, lambda: impl Fn(&T) -> bool) {
        self.filtered_items = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, c)| lambda(c))
            .map(|(i, _)| i)
            .collect();
    }

    pub fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i + 1 >= self.filtered_items.len() {
                    if MESSAGE_SCROLL_BACK {
                        0
                    } else {
                        i
                    }
                } else {
                    i + 1
                }
            }
            None => {
                if !self.filtered_items.is_empty() {
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
                        self.filtered_items.len() - 1
                    } else {
                        0
                    }
                } else {
                    i - 1
                }
            }
            None => {
                if !self.filtered_items.is_empty() {
                    0
                } else {
                    return; // nothing to select
                }
            }
        };
        self.state.select(Some(i));
    }
}

pub fn utc_timestamp_msec_to_local(timestamp: u64) -> DateTime<Local> {
    let dt = NaiveDateTime::from_timestamp(timestamp as i64 / 1000, (timestamp % 1000) as u32);
    Utc.from_utc_datetime(&dt).with_timezone(&Local)
}

pub fn utc_now_timestamp_msec() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis() as u64
}

pub fn is_phone_number(s: impl AsRef<str>) -> bool {
    use std::str::FromStr;
    // Note: previously we formatted phone numbers sometimes incorrectly (not always as E164). So,
    // some users might still have them stored with spaces and dashes. So, we strip them here, even
    // the formatting now is correct.
    let stripped = s.as_ref().replace(&[' ', '-'][..], "");
    PhoneNumber::from_str(&stripped).is_ok()
}

// Based on Alacritty, APACHE-2.0 License
pub const URL_REGEX: &str =
    "(ipfs:|ipns:|magnet:|mailto:|gemini:|gopher:|https:|http:|news:|file:|git:|ssh:|ftp:)\
     [^\u{0000}-\u{001F}\u{007F}-\u{009F}<>\"\\s{-}\\^⟨⟩`]+";
pub const ATTACHMENT_REGEX: &str = "file:[^\u{0000}-\u{001F}\u{007F}-\u{009F}<>\"\\s{-}\\^⟨⟩`]+";

/// Regex which is compiled on demand, to avoid expensive computations at startup.
///
/// Based on Alacritty, APACHE-2.0 License
#[derive(Clone, Debug)]
pub enum LazyRegex {
    Pattern(&'static str),
    Compiled(Box<Regex>),
}

impl LazyRegex {
    pub fn new(pattern: &'static str) -> Self {
        Self::Pattern(pattern)
    }

    /// Get a reference to the compiled regex.
    ///
    /// Compiles regex on the first call.
    pub fn compiled(&mut self) -> &Regex {
        if let Self::Pattern(pattern) = self {
            let regex = Regex::new(pattern).expect("invalid regex");
            *self = Self::Compiled(Box::new(regex));
        }
        match self {
            Self::Compiled(regex) => regex,
            Self::Pattern(_) => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_phone_number() {
        assert!(is_phone_number("+1 000-000-0000"));
    }
}

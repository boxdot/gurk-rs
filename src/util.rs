use chrono::{DateTime, Local, NaiveDateTime, TimeZone as _, Utc};
use presage::prelude::PhoneNumber;
use regex_automata::Regex;
use serde::{Deserialize, Serialize};
use tui::widgets::ListState;

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

    pub(crate) fn selected_item(&self) -> Option<&T> {
        let idx = self.state.selected()?;
        Some(&self.items[idx])
    }
}

pub fn utc_timestamp_msec_to_local(timestamp: u64) -> DateTime<Local> {
    let dt = NaiveDateTime::from_timestamp_opt(timestamp as i64 / 1000, (timestamp % 1000) as u32)
        .expect("invalid datetime");
    Utc.from_utc_datetime(&dt).with_timezone(&Local)
}

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

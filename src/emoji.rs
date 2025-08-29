use std::{borrow::Cow, sync::LazyLock};

use regex::{Captures, Regex};

static REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r":([a-z1238+-][a-z0-9_-]*):").unwrap());

pub(crate) fn replace_shortcodes(text: &str) -> Cow<'_, str> {
    REGEX.replace_all(text, Replacer)
}

struct Replacer;

impl regex::Replacer for Replacer {
    fn replace_append(&mut self, caps: &Captures, dst: &mut String) {
        match emojis::get_by_shortcode(&caps[1]) {
            Some(emoji) => dst.push_str(emoji.as_str()),
            None => dst.push_str(&caps[1]),
        }
    }
}

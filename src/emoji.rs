use std::{borrow::Cow, sync::LazyLock};

use regex::{Captures, Regex};
use unicode_segmentation::UnicodeSegmentation;

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

/// Replaces every emoji in `text` with its CLDR short name in brackets, e.g. `👍` -> `[thumbs up]`
pub(crate) fn replace_emojis_with_names(text: &str) -> Cow<'_, str> {
    if text.is_ascii() {
        return Cow::Borrowed(text);
    }
    let mut res = String::with_capacity(text.len());
    for grapheme in text.graphemes(true) {
        match emojis::get(grapheme) {
            Some(emoji) => {
                res.push('[');
                res.push_str(emoji.name());
                res.push(']');
            }
            None => res.push_str(grapheme),
        }
    }
    Cow::Owned(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replace_emojis_with_names() {
        assert_eq!(replace_emojis_with_names("hello"), "hello");
        assert_eq!(
            replace_emojis_with_names("nice 👍🎉!"),
            "nice [thumbs up][party popper]!"
        );
        assert_eq!(replace_emojis_with_names("Grüße"), "Grüße");
    }
}

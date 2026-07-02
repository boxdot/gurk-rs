//! Input box

use unicode_width::UnicodeWidthStr;

use crate::cursor::Cursor;

/// Input box with data and a cursor
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub data: String,
    pub cursor: Cursor,
    /// Original partial that started a tab-completion cycle (e.g., "th" for thumbsup).
    /// Set during cycling, cleared on any other input action.
    pub completion_partial: Option<String>,
}

impl Input {
    pub fn put_char(&mut self, c: char) {
        self.cursor.put(c, &mut self.data);
    }

    pub fn new_line(&mut self) {
        self.cursor.new_line(&mut self.data);
    }

    pub fn on_left(&mut self) {
        self.cursor.move_left(&self.data);
    }

    pub fn on_right(&mut self) {
        self.cursor.move_right(&self.data);
    }

    pub fn move_line_down(&mut self) {
        self.cursor.move_line_down(&self.data);
    }

    pub fn move_line_up(&mut self) {
        self.cursor.move_line_up(&self.data);
    }

    pub fn move_back_word(&mut self) {
        self.cursor.move_word_left(&self.data);
    }

    pub fn move_forward_word(&mut self) {
        self.cursor.move_word_right(&self.data);
    }

    pub fn on_home(&mut self) {
        self.cursor.start_of_line(&self.data);
    }

    pub fn on_end(&mut self) {
        self.cursor.end_of_line(&self.data);
    }

    pub fn on_backspace(&mut self) {
        self.cursor.delete_backward(&mut self.data);
    }

    pub fn on_delete(&mut self) {
        self.cursor.delete_forward(&mut self.data);
    }

    pub fn on_delete_line(&mut self) {
        self.cursor.delete_line_backward(&mut self.data);
    }

    pub fn on_delete_word(&mut self) {
        self.cursor.delete_word_backward(&mut self.data);
    }

    pub fn on_delete_suffix(&mut self) {
        self.cursor.delete_suffix(&mut self.data);
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn take(&mut self) -> String {
        self.cursor = Default::default();
        std::mem::take(&mut self.data)
    }

    /// If the character just before the cursor is `:`, look backwards for a
    /// matching opening `:` and check if it's a valid emoji shortcode. If so,
    /// replace `:shortcode:` with the actual emoji and adjust cursor position.
    pub fn convert_emoji_on_colon(&mut self) {
        let idx = self.cursor.idx;
        if idx < 1 || !self.data.is_char_boundary(idx) {
            return;
        }
        if &self.data[idx - 1..idx] != ":" {
            return;
        }

        let before = &self.data[..idx - 1];
        let open_pos = match before.rfind(':') {
            Some(p) => p,
            None => return,
        };

        let shortcode = &self.data[open_pos + 1..idx - 1];
        let Some(emoji) = emojis::get_by_shortcode(shortcode) else {
            return;
        };
        let emoji_str = emoji.as_str();

        let old_width = self.data[open_pos..idx].width();
        let emoji_width = emoji_str.width();

        self.cursor.col = self.cursor.col.saturating_sub(old_width - emoji_width);
        self.cursor.idx = open_pos + emoji_str.len();
        self.data.replace_range(open_pos..idx, emoji_str);
    }

    /// If the cursor is inside a potential emoji shortcode (`:part`),
    /// complete or cycle to the next matching shortcode on each call.
    /// Returns true if a completion was made.
    pub fn try_complete_emoji(&mut self) -> bool {
        let idx = self.cursor.idx;
        if idx < 1 {
            self.completion_partial = None;
            return false;
        }

        let before = &self.data[..idx];
        let open_pos = match before.rfind(':') {
            Some(p) => p,
            None => {
                self.completion_partial = None;
                return false;
            }
        };

        let current = &self.data[open_pos + 1..idx];
        if current.is_empty() {
            self.completion_partial = None;
            return false;
        }

        // Determine the base partial to match against.
        // If we're cycling (completion_partial is set and current text is
        // related), reuse the original partial. Otherwise start fresh.
        let partial = match &self.completion_partial {
            Some(prev) if current.starts_with(prev.as_str()) || prev.starts_with(current) => {
                prev.as_str()
            }
            _ => {
                self.completion_partial = Some(current.to_string());
                current
            }
        };

        let matches: Vec<&str> = emojis::iter()
            .flat_map(|e| e.shortcodes())
            .filter(|sc| sc.starts_with(partial))
            .collect();

        if matches.is_empty() {
            self.completion_partial = None;
            return false;
        }

        let cur_idx = matches.iter().position(|&sc| sc == current);
        let next_idx = match cur_idx {
            Some(i) => (i + 1) % matches.len(),
            None => 0,
        };

        let next = matches[next_idx];
        let old_width = current.width();
        let new_width = next.width();

        self.data.replace_range(open_pos + 1..idx, next);
        if new_width >= old_width {
            self.cursor.col += new_width - old_width;
        } else {
            self.cursor.col = self.cursor.col.saturating_sub(old_width - new_width);
        }
        self.cursor.idx = open_pos + 1 + next.len();
        true
    }
}

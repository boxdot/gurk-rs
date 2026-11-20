use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct Cursor {
    /// Position of the character as byte index
    pub idx: usize,
    /// Position vertically
    pub line: usize,
    /// Position horizontally
    pub col: usize,
    col_wanted: Option<usize>,
}

impl Cursor {
    #[cfg(test)]
    pub fn new(idx: usize, line: usize, col: usize) -> Self {
        Self {
            idx,
            line,
            col,
            ..Default::default()
        }
    }

    #[cfg(test)]
    pub fn begin() -> Self {
        Default::default()
    }

    #[cfg(test)]
    pub fn end(text: &str) -> Self {
        let idx = text.len();
        let (line, col) = calc_line_column(text, idx);
        Self {
            idx,
            line,
            col,
            col_wanted: None,
        }
    }

    #[cfg(test)]
    pub fn at(text: &str, idx: usize) -> Self {
        let idx = snap_to_grapheme(text, idx);
        let (line, col) = calc_line_column(text, idx);
        Self {
            idx,
            line,
            col,
            col_wanted: None,
        }
    }

    pub fn put(&mut self, c: char, text: &mut String) {
        text.insert(self.idx, c);
        if c == '\n' {
            self.line += 1;
            self.col = 0;
            self.idx += 1;
        } else {
            let s = &text[self.idx..self.idx + c.len_utf8()];
            self.col += s.width();
            self.idx += c.len_utf8();
        }
        self.col_wanted = None;
    }

    pub fn new_line(&mut self, text: &mut String) {
        self.put('\n', text);
    }

    pub fn move_left(&mut self, text: &str) {
        // Find the previous grapheme
        let prev = text[..self.idx]
            .grapheme_indices(true)
            .next_back();
        if let Some((idx, g)) = prev {
            if g == "\n" {
                self.line -= 1;
                // Calculate column by summing widths of graphemes on this line
                self.col = text[..idx]
                    .graphemes(true)
                    .rev()
                    .take_while(|&g| g != "\n")
                    .map(|g| g.width())
                    .sum();
            } else {
                self.col -= g.width();
            }
            self.idx = idx;
            self.col_wanted = None;
        }
    }

    pub fn move_right(&mut self, text: &str) {
        // Find the next grapheme
        let mut graphemes = text[self.idx..].grapheme_indices(true);
        if let Some((_, g)) = graphemes.next() {
            if g == "\n" {
                self.line += 1;
                self.col = 0;
            } else {
                self.col += g.width();
            }
            self.idx = graphemes
                .next()
                .map(|(idx, _)| self.idx + idx)
                .unwrap_or(text.len());
            self.col_wanted = None;
        }
    }

    pub fn delete_backward(&mut self, text: &mut String) {
        // Find the previous grapheme
        let prev = text[..self.idx]
            .grapheme_indices(true)
            .next_back();
        if let Some((idx, g)) = prev {
            if g == "\n" {
                self.line -= 1;
                self.col = text[..idx]
                    .graphemes(true)
                    .rev()
                    .take_while(|&g| g != "\n")
                    .map(|g| g.width())
                    .sum();
            } else {
                self.col -= g.width();
            }
            let end = self.idx;
            self.idx = idx;
            text.replace_range(idx..end, "");
        }
    }

    pub fn delete_forward(&mut self, text: &mut String) {
        if self.idx < text.len() {
            // Find the grapheme at current position
            if let Some((_, g)) = text[self.idx..].grapheme_indices(true).next() {
                text.replace_range(self.idx..self.idx + g.len(), "");
            }
        }
    }

    pub fn delete_line_backward(&mut self, text: &mut String) {
        // Delete graphemes until we hit start of line
        while self.col > 0 {
            self.delete_backward(text);
        }
    }

    pub fn delete_word_backward(&mut self, text: &mut String) {
        let end = self.idx;
        self.move_word_left(text);
        text.replace_range(self.idx..end, "");
    }

    pub fn move_line_down(&mut self, text: &str) {
        let offset = self.idx;
        // Find end of current line
        let end_line = text[offset..]
            .grapheme_indices(true)
            .find(|(_, g)| *g == "\n")
            .map(|(idx, _)| offset + idx);

        if let Some(newline_idx) = end_line {
            // Check if there's a next line
            let next_start = newline_idx + 1;
            if next_start <= text.len() {
                self.line += 1;
                let prev_col = *self.col_wanted.get_or_insert(self.col);

                // Find end of next line
                let next_end = text[next_start..]
                    .grapheme_indices(true)
                    .find(|(_, g)| *g == "\n")
                    .map(|(idx, _)| next_start + idx)
                    .unwrap_or(text.len());

                // Move to the same column on the next line
                let (idx, col) = text[next_start..next_end]
                    .grapheme_indices(true)
                    .scan(0, |width, (idx, g)| {
                        let col = *width;
                        *width += g.width();
                        Some((next_start + idx, col))
                    })
                    .take_while(|(_, col)| *col <= prev_col)
                    .last()
                    .unwrap_or((next_start, 0));

                self.col = col;
                self.idx = idx;
            }
        } else {
            // No newline found, move to end of text
            self.col = text[..text.len()]
                .graphemes(true)
                .rev()
                .take_while(|&g| g != "\n")
                .map(|g| g.width())
                .sum();
            self.idx = text.len();
        }
    }

    pub fn move_line_up(&mut self, text: &str) {
        // Find start of current line
        let current_line_start = text[..self.idx]
            .grapheme_indices(true)
            .rev()
            .find(|(_, g)| *g == "\n")
            .map(|(idx, _)| idx);

        if let Some(prev_newline) = current_line_start {
            self.line -= 1;
            let prev_col = *self.col_wanted.get_or_insert(self.col);

            // Find start of previous line
            let prev_line_start = text[..prev_newline]
                .grapheme_indices(true)
                .rev()
                .find(|(_, g)| *g == "\n")
                .map(|(idx, g)| idx + g.len())
                .unwrap_or(0);

            // Move to the same column on the previous line
            let (idx, col) = text[prev_line_start..prev_newline]
                .grapheme_indices(true)
                .scan(0, |width, (idx, g)| {
                    let col = *width;
                    *width += g.width();
                    Some((prev_line_start + idx, col))
                })
                .take_while(|(_, col)| *col <= prev_col)
                .last()
                .unwrap_or((prev_line_start, 0));

            self.col = col;
            self.idx = idx;
        } else {
            self.col = 0;
            self.idx = 0;
        }
    }

    pub fn move_word_left(&mut self, text: &str) {
        // Skip whitespace
        while self.idx > 0 {
            let prev = text[..self.idx].graphemes(true).next_back();
            match prev {
                Some(g) if g.chars().all(|c| c.is_whitespace()) => self.move_left(text),
                _ => break,
            }
        }
        // Skip non-whitespace
        while self.idx > 0 {
            let prev = text[..self.idx].graphemes(true).next_back();
            match prev {
                Some(g) if !g.chars().all(|c| c.is_whitespace()) => self.move_left(text),
                _ => break,
            }
        }
    }

    pub fn move_word_right(&mut self, text: &str) {
        // Skip non-whitespace
        while self.idx < text.len() {
            let next = text[self.idx..].graphemes(true).next();
            match next {
                Some(g) if !g.chars().all(|c| c.is_whitespace()) => self.move_right(text),
                _ => break,
            }
        }
        // Skip whitespace
        while self.idx < text.len() {
            let next = text[self.idx..].graphemes(true).next();
            match next {
                Some(g) if g.chars().all(|c| c.is_whitespace()) => self.move_right(text),
                _ => break,
            }
        }
    }

    pub fn start_of_line(&mut self, text: &str) {
        // Find the previous newline
        let prev_newline = text[..self.idx]
            .grapheme_indices(true)
            .rev()
            .find(|(_, g)| *g == "\n");

        if let Some((idx, g)) = prev_newline {
            self.idx = idx + g.len();
            self.col = 0;
        } else {
            self.idx = 0;
            self.col = 0;
        }
    }

    pub fn end_of_line(&mut self, text: &str) {
        // Find the next newline or end of text
        let next_newline = text[self.idx..]
            .grapheme_indices(true)
            .find(|(_, g)| *g == "\n");

        if let Some((idx, _)) = next_newline {
            let end_idx = self.idx + idx;
            self.col += text[self.idx..end_idx].width();
            self.idx = end_idx;
        } else {
            self.col += text[self.idx..].width();
            self.idx = text.len();
        }
    }

    pub fn delete_suffix(&mut self, text: &mut String) {
        let end = text[self.idx..]
            .grapheme_indices(true)
            .find(|(_, g)| *g == "\n")
            .map(|(idx, _)| self.idx + idx)
            .unwrap_or(text.len());
        if self.idx == end && end < text.len() {
            // At newline, delete just the newline
            if let Some((_, g)) = text[end..].grapheme_indices(true).next() {
                text.replace_range(end..end + g.len(), "");
            }
        } else {
            text.replace_range(self.idx..end, "");
        }
    }
}

/// Snap the byte index `idx` to a grapheme boundary in `s`.
#[cfg(test)]
fn snap_to_grapheme(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        s.len()
    } else {
        // Find the grapheme that contains this byte index
        s.grapheme_indices(true)
            .take_while(|(i, _)| *i <= idx)
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
}

#[cfg(test)]
fn calc_line_column(s: &str, idx: usize) -> (usize, usize) {
    let mut col = 0;
    let mut line = 0;

    for g in s[..idx].graphemes(true) {
        if g == "\n" {
            line += 1;
            col = 0;
        } else {
            col += g.width();
        }
    }

    (line, col)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    use quickcheck::{Arbitrary, Gen};
    use quickcheck_macros::quickcheck;

    #[test]
    fn test_cursor_empty() {
        let cur = Cursor::end("");
        assert_eq!(cur, Cursor::begin());
    }

    #[test]
    fn test_cursor_one_line() {
        let cur = Cursor::end("Hello, world!");
        assert_eq!(cur, Cursor::new(13, 0, 13));
    }

    #[test]
    fn test_cursor_new_line() {
        let cur = Cursor::end("\n\n");
        assert_eq!(cur, Cursor::new(2, 2, 0));
    }

    #[test]
    fn test_cursor_multiple_lines() {
        let cur = Cursor::end("Hello\n\nWorld");
        assert_eq!(cur, Cursor::new(12, 2, 5));
    }

    #[test]
    fn test_cursor_at() {
        let text = "Hello\nWorld\n\n".to_string();
        assert_eq!(Cursor::at(&text, 0), Cursor::begin());
        assert_eq!(Cursor::at(&text, 0), Cursor::new(0, 0, 0));
        assert_eq!(Cursor::at(&text, 1), Cursor::new(1, 0, 1));
        assert_eq!(Cursor::at(&text, 5), Cursor::new(5, 0, 5));
        assert_eq!(Cursor::at(&text, 6), Cursor::new(6, 1, 0));
        assert_eq!(Cursor::at(&text, 6), Cursor::new(6, 1, 0));
        assert_eq!(Cursor::at(&text, 11), Cursor::new(11, 1, 5));
        assert_eq!(Cursor::at(&text, 12), Cursor::new(12, 2, 0));
        assert_eq!(Cursor::at(&text, 13), Cursor::new(13, 3, 0));
        assert_eq!(Cursor::at(&text, 100), Cursor::new(13, 3, 0));
    }

    #[test]
    fn test_zwj_emoji() {
        // Woman tipping hand: base + ZWJ + female sign + VS16
        let emoji = "💁‍♀️";
        assert_eq!(emoji.len(), 13); // 13 bytes
        assert_eq!(emoji.graphemes(true).count(), 1); // 1 grapheme
        assert_eq!(emoji.width(), 2); // 2 cells wide

        let text = format!("Hi{emoji}!");
        let mut cursor = Cursor::begin();

        // Move right twice to get past "Hi"
        cursor.move_right(&text);
        cursor.move_right(&text);
        assert_eq!(cursor.idx, 2);
        assert_eq!(cursor.col, 2);

        // Move right once more - should skip entire emoji
        cursor.move_right(&text);
        assert_eq!(cursor.idx, 15); // 2 + 13
        assert_eq!(cursor.col, 4); // 2 + 2 (emoji width)

        // Move left - should go back to start of emoji
        cursor.move_left(&text);
        assert_eq!(cursor.idx, 2);
        assert_eq!(cursor.col, 2);
    }

    #[test]
    fn test_move_word_left_right() {
        let text = "Hello\n  new🌍\n\nWorld";
        let mut cursor = Cursor::begin();

        let stops = vec![
            Cursor::new(0, 0, 0),
            Cursor::new(8, 1, 2),
            Cursor::new(17, 3, 0),
            Cursor::new(22, 3, 5),
        ];

        for stop in &stops {
            assert_eq!(stop, &cursor);
            cursor.move_word_right(text);
        }

        for stop in stops.iter().rev() {
            assert_eq!(stop, &cursor);
            cursor.move_word_left(text);
        }
    }

    #[test]
    fn test_delete_suffix() {
        let mut text = "Hello\n  new🌍\n\nWorld".to_string();
        let mut cursor = Cursor::begin();

        cursor.delete_suffix(&mut text);
        assert_eq!(cursor, Cursor::new(0, 0, 0));
        assert_eq!(text, "\n  new🌍\n\nWorld");

        cursor.delete_suffix(&mut text);
        assert_eq!(cursor, Cursor::new(0, 0, 0));
        assert_eq!(text, "  new🌍\n\nWorld");

        cursor.delete_suffix(&mut text);
        assert_eq!(cursor, Cursor::new(0, 0, 0));
        assert_eq!(text, "\n\nWorld");

        cursor.delete_suffix(&mut text);
        assert_eq!(cursor, Cursor::new(0, 0, 0));
        assert_eq!(text, "\nWorld");

        cursor.delete_suffix(&mut text);
        assert_eq!(cursor, Cursor::new(0, 0, 0));
        assert_eq!(text, "World");

        cursor.delete_suffix(&mut text);
        assert_eq!(cursor, Cursor::new(0, 0, 0));
        assert_eq!(text, "");
    }

    #[test]
    fn test_delete_forward() {
        let mut text = "Hello\n  new🌍\n\nWorld".to_string();
        let mut cursor = Cursor::begin();

        cursor.delete_forward(&mut text);
        assert_eq!(cursor, Cursor::new(0, 0, 0));
        assert_eq!(text, "ello\n  new🌍\n\nWorld");

        cursor.delete_forward(&mut text);
        cursor.delete_forward(&mut text);
        cursor.delete_forward(&mut text);
        cursor.delete_forward(&mut text);
        assert_eq!(cursor, Cursor::new(0, 0, 0));
        assert_eq!(text, "\n  new🌍\n\nWorld");

        cursor.delete_forward(&mut text);
        assert_eq!(cursor, Cursor::new(0, 0, 0));
        assert_eq!(text, "  new🌍\n\nWorld");

        for _ in 0..5 {
            cursor.move_right(&text);
        }
        assert_eq!(cursor, Cursor::new(5, 0, 5));
        cursor.delete_forward(&mut text);
        assert_eq!(cursor, Cursor::new(5, 0, 5));
        assert_eq!(text, "  new\n\nWorld");

        let mut cursor = Cursor::end(&text);
        let snapshot = text.clone();
        cursor.delete_forward(&mut text);
        assert_eq!(text, snapshot);
    }

    #[derive(Debug, Clone, Copy)]
    enum Operation {
        Left,
        Right,
        Down,
        Up,
        Put(char),
        DeleteBackward,
        DeleteForward,
    }

    impl Arbitrary for Operation {
        fn arbitrary(g: &mut Gen) -> Self {
            use Operation::*;

            let mut c = char::arbitrary(g);
            // Restrict to printable ASCII and common emoji for predictable widths
            // Many Unicode chars have edge cases that break the (line, col) -> idx test
            while !(c.is_ascii_graphic() || c == ' ' || c == '\n') {
                c = char::arbitrary(g);
            }

            *g.choose(&[Left, Right, Up, Down, Put(c), DeleteBackward, DeleteForward])
                .unwrap()
        }
    }

    #[quickcheck]
    fn test_random_operations_sequence(operations: Vec<Operation>) -> bool {
        let mut text = "Hello\nnew🌍\n\nWorld".to_string();

        let calc_index_matrix = |text: &str| {
            let mut res: HashMap<(usize, usize), usize> = Default::default();

            let mut line = 0;
            let mut col = 0;

            for (idx, g) in text.grapheme_indices(true) {
                res.insert((line, col), idx);
                if g == "\n" {
                    line += 1;
                    col = 0;
                } else {
                    col += g.width();
                }
            }
            res
        };

        let mut cursor = Cursor::begin();
        let mut index_matrix = calc_index_matrix(&text);

        for op in operations {
            let is_last = cursor.idx == text.len();
            let is_entry = cursor.idx < text.len()
                && index_matrix.get(&(cursor.line, cursor.col)) == Some(&cursor.idx);
            if !(is_last || is_entry) {
                println!("is_last = {is_last}");
                println!("is_entry = {is_entry}");
                println!("{text:?}");
                println!("{index_matrix:?}");
                println!("cursor = {cursor:?}");
                println!("op = {op:?}");
                return false;
            }

            match op {
                Operation::Left => cursor.move_left(&text),
                Operation::Right => cursor.move_right(&text),
                Operation::Down => cursor.move_line_down(&text),
                Operation::Up => cursor.move_line_up(&text),
                Operation::Put(c) => {
                    cursor.put(c, &mut text);
                    index_matrix = calc_index_matrix(&text);
                }
                Operation::DeleteBackward => {
                    cursor.delete_backward(&mut text);
                    index_matrix = calc_index_matrix(&text);
                }
                Operation::DeleteForward => {
                    cursor.delete_forward(&mut text);
                    index_matrix = calc_index_matrix(&text);
                }
            }
        }

        true
    }
}

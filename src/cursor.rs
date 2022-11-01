use itertools::Itertools;
use unicode_width::UnicodeWidthChar;

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
        let idx = snap_to_char(text, text.len());
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
        let idx = snap_to_char(text, idx);
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
            self.col += c.width().unwrap_or(0);
            self.idx += c.len_utf8();
        }
        self.col_wanted = None;
    }

    pub fn new_line(&mut self, text: &mut String) {
        self.put('\n', text);
    }

    pub fn move_left(&mut self, text: &str) {
        let mut char_indices = text[..self.idx].char_indices();
        if let Some((idx, c)) = char_indices.next_back() {
            if c == '\n' {
                self.line -= 1;
                self.col = char_indices
                    .rev()
                    .take_while(|&(_, c)| c != '\n')
                    .map(|(_, c)| c.width().unwrap_or(0))
                    .sum();
            } else {
                self.col -= c.width().unwrap_or(0);
            }
            self.idx = idx;
            self.col_wanted = None;
        }
    }

    pub fn move_right(&mut self, text: &str) {
        let mut char_indices = text[self.idx..]
            .char_indices()
            .map(|(idx, c)| (idx + self.idx, c));
        if let Some((_idx, c)) = char_indices.next() {
            if c == '\n' {
                self.line += 1;
                self.col = 0;
            } else {
                self.col += c.width().unwrap_or(0);
            }
            self.idx = char_indices
                .next()
                .map(|(idx, _c)| idx)
                .unwrap_or_else(|| text.len());
            self.col_wanted = None;
        }
    }

    pub fn delete_backward(&mut self, text: &mut String) {
        let mut char_indices = text[..self.idx].char_indices();
        if let Some((idx, c)) = char_indices.next_back() {
            if c == '\n' {
                self.line -= 1;
                self.col = char_indices
                    .rev()
                    .take_while(|&(_, c)| c != '\n')
                    .map(|(_, c)| c.width().unwrap_or(0))
                    .sum();
            } else {
                self.col -= c.width().unwrap_or(0);
            }
            self.idx = idx;
            text.remove(idx);
        }
    }

    pub fn delete_word_backward(&mut self, text: &mut String) {
        let end = self.idx;
        self.move_word_left(text);
        text.replace_range(self.idx..end, "");
    }

    pub fn move_line_down(&mut self, text: &str) {
        let offset = self.idx;
        let mut char_indices = text[offset..]
            .char_indices()
            .map(|(idx, c)| (idx + offset, c))
            .skip_while(|&(_, c)| c != '\n')
            .map(|(idx, _)| idx);
        let end_line = char_indices.next();
        if let Some(start) = char_indices.next() {
            self.line += 1;
            let prev_col = *self.col_wanted.get_or_insert(self.col);

            let end = text[start..]
                .char_indices()
                .find(|&(_, c)| c == '\n')
                .map(|(idx, _)| start + idx)
                .unwrap_or_else(|| text.len());

            let (idx, col) = text[start..end]
                .char_indices()
                .take(prev_col + 1)
                .scan(0, |prev_width, (idx, c)| {
                    let col = *prev_width;
                    *prev_width += c.width().unwrap_or(0);
                    Some((start + idx, col))
                })
                .last()
                .unwrap_or((start, 0));

            self.col = col;
            self.idx = idx;
        } else if let Some(idx) = end_line {
            self.col = text[..idx]
                .chars()
                .rev()
                .take_while(|&c| c != '\n')
                .map(|c| c.width().unwrap_or(0))
                .sum();
            self.idx = idx;
        } else {
            self.col = text
                .chars()
                .rev()
                .take_while(|&c| c != '\n')
                .map(|c| c.width().unwrap_or(0))
                .sum();
            self.idx = text.len();
        }
    }

    pub fn move_line_up(&mut self, text: &str) {
        if let Some((end, _)) = text[..self.idx]
            .char_indices()
            .rev()
            .find(|&(_, c)| c == '\n')
        {
            self.line -= 1;
            let prev_col = *self.col_wanted.get_or_insert(self.col);

            let start = text[..end]
                .char_indices()
                .rev()
                .find(|&(_, c)| c == '\n')
                .map(|(idx, _)| idx + 1)
                .unwrap_or(0);

            let (idx, col) = text[start..end]
                .char_indices()
                .take(prev_col + 1)
                .scan(0, |prev_width, (idx, c)| {
                    let col = *prev_width;
                    *prev_width += c.width().unwrap_or(0);
                    Some((start + idx, col))
                })
                .last()
                .unwrap_or((start, 0));

            self.col = col;
            self.idx = idx;
        } else {
            self.col = 0;
            self.idx = 0;
        }
    }

    pub fn move_word_left(&mut self, text: &str) {
        let mut chars = text[..self.idx].chars().rev().peekable();

        while let Some(c) = chars.peek() {
            if !c.is_whitespace() {
                break;
            }
            self.move_left(text);
            chars.next();
        }

        while let Some(c) = chars.peek() {
            if c.is_whitespace() {
                break;
            }
            self.move_left(text);
            chars.next();
        }
    }

    pub fn move_word_right(&mut self, text: &str) {
        let mut chars = text[self.idx..].chars().peekable();

        while let Some(c) = chars.peek() {
            if c.is_whitespace() {
                break;
            }
            self.move_right(text);
            chars.next();
        }

        while let Some(c) = chars.peek() {
            if !c.is_whitespace() {
                break;
            }
            self.move_right(text);
            chars.next();
        }
    }

    pub fn start_of_line(&mut self, text: &str) {
        if let Some((idx, col, c)) = text[..self.idx]
            .char_indices()
            .rev()
            .scan(0, |prev_width, (idx, c)| {
                let col = *prev_width;
                *prev_width += c.width().unwrap_or(0);
                Some((idx, col, c))
            })
            .find(|&(_, _, c)| c == '\n')
        {
            self.idx = idx;
            self.col -= col;
            self.line -= 1;
            if c == '\n' {
                self.move_right(text);
            }
        } else {
            self.idx = 0;
            self.col = 0;
        }
    }

    pub fn end_of_line(&mut self, text: &str) {
        let offset = self.idx;
        if let Some((idx, col, c)) = text[self.idx..]
            .char_indices()
            .scan(0, |prev_width, (idx, c)| {
                let col = *prev_width;
                *prev_width += c.width().unwrap_or(0);
                Some((offset + idx, col, c))
            })
            .find_or_last(|&(_, _, c)| c == '\n')
        {
            self.idx = idx;
            self.col += col;
            if c != '\n' {
                // end of text
                self.move_right(text);
            }
        }
    }

    pub fn delete_suffix(&mut self, text: &mut String) {
        let end = text[self.idx..]
            .char_indices()
            .find(|&(_, c)| c == '\n')
            .map(|(idx, _)| idx)
            .unwrap_or_else(|| text.len());
        if self.idx == end && end < text.len() {
            text.remove(end);
        } else {
            text.replace_range(self.idx..end, "");
        }
    }
}

/// Snap the byte index `idx` to a char boundary in `s`.
///
/// The snapping is always done to left starting at `idx`.
#[cfg(test)]
fn snap_to_char(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        s.len()
    } else {
        while !s.is_char_boundary(idx) {
            if let Some(new_idx) = idx.checked_sub(1) {
                idx = new_idx
            } else {
                break;
            }
        }
        idx
    }
}

#[cfg(test)]
fn calc_line_column(s: &str, idx: usize) -> (usize, usize) {
    let mut col = 0;
    let mut line = 0;

    for c in s[0..idx].chars().rev() {
        if c == '\n' {
            line += 1;
        }
        if line == 0 {
            col += 1;
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

    #[derive(Debug, Clone, Copy)]
    enum Operation {
        Left,
        Right,
        Down,
        Up,
        Put(char),
        DeleteBackward,
    }

    impl Arbitrary for Operation {
        fn arbitrary(g: &mut Gen) -> Self {
            use Operation::*;

            let mut c = char::arbitrary(g);
            while c.width().unwrap_or(0) == 0 {
                c = char::arbitrary(g);
            }

            *g.choose(&[Left, Right, Up, Down, Put(c), DeleteBackward])
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

            for (idx, c) in text.char_indices() {
                res.insert((line, col), idx);
                if c == '\n' {
                    line += 1;
                    col = 0;
                }
                col += c.width().unwrap_or(0);
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
            }
        }

        true
    }
}

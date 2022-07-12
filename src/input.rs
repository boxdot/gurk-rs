//! Input box

use crate::cursor::Cursor;

/// Input box with data and a cursor
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub data: String,
    pub cursor: Cursor,
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

    pub fn on_delete_word(&mut self) {
        self.cursor.delete_word_backward(&mut self.data);
    }

    pub fn on_delete_suffix(&mut self) {
        self.cursor.delete_suffix(&mut self.data);
    }

    pub fn take(&mut self) -> String {
        self.cursor = Default::default();
        std::mem::take(&mut self.data)
    }
}

use ratatui::widgets::ListState;

use crate::data::{ChannelId, Message};
use crate::input::Input;

#[derive(Default)]
pub struct SearchMessages {
    pub is_shown: bool,
    pub input: Input,
    pub state: ListState,
    pub results: Vec<Message>,
    pub last_query: String,
    pub channel_id: Option<ChannelId>,
}

impl SearchMessages {
    pub fn reset(&mut self) {
        self.input.take();
        self.state = Default::default();
        self.results.clear();
        self.last_query.clear();
        self.channel_id = None;
    }

    pub fn query_changed(&self) -> bool {
        self.input.data != self.last_query
    }

    pub fn prev(&mut self) {
        if self.results.is_empty() {
            return;
        }
        let selected = self
            .state
            .selected()
            .map(|idx| idx.saturating_sub(1))
            .unwrap_or(0);
        self.state.select(Some(selected));
    }

    pub fn next(&mut self) {
        if self.results.is_empty() {
            return;
        }
        let len = self.results.len();
        let selected = self
            .state
            .selected()
            .map(|idx| (idx + 1).min(len.saturating_sub(1)))
            .unwrap_or(0);
        self.state.select(Some(selected));
    }

    pub fn selected_message(&self) -> Option<&Message> {
        let idx = self.state.selected()?;
        self.results.get(idx)
    }
}

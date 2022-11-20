use std::cmp::Reverse;

use tui::widgets::ListState;

use crate::data::ChannelId;
use crate::input::Input;
use crate::storage::Storage;

#[derive(Default)]
pub(crate) struct SelectChannel {
    pub is_shown: bool,
    pub input: Input,
    pub items: Vec<ItemData>,
    pub filtered_index: Vec<usize /* index into items */>,
    pub state: ListState,
}

pub(crate) struct ItemData {
    pub channel_id: ChannelId,
    pub name: String,
}

impl SelectChannel {
    pub fn reset(&mut self, storage: &dyn Storage) {
        self.input.take();
        self.state = Default::default();

        self.items = storage
            .channels()
            .map(|channel| ItemData {
                channel_id: channel.id,
                name: channel.name.clone(),
            })
            .collect();
        self.items.sort_unstable_by_key(|item| {
            let last_message_arrived_at = storage
                .messages(item.channel_id)
                .last()
                .map(|message| message.arrived_at);
            (Reverse(last_message_arrived_at), item.name.clone())
        });

        self.filtered_index.clear();
    }

    pub fn prev(&mut self) {
        let selected = self
            .state
            .selected()
            .map(|idx| idx.saturating_sub(1))
            .unwrap_or(0);
        self.state.select(Some(selected));
    }

    pub fn next(&mut self) {
        let selected = self.state.selected().map(|idx| idx + 1).unwrap_or(0);
        self.state.select(Some(selected));
    }
}

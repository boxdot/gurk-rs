use std::cmp::Reverse;

use ratatui::widgets::ListState;

use crate::data::ChannelId;
use crate::input::Input;
use crate::storage::Storage;

#[derive(Default)]
pub(crate) struct SelectChannel {
    pub is_shown: bool,
    pub input: Input,
    pub state: ListState,
    items: Vec<ItemData>,
    filtered_index: Vec<usize /* index into items */>,
}

pub(crate) struct ItemData {
    pub channel_id: ChannelId,
    pub name: String,
}

impl SelectChannel {
    pub fn reset(&mut self, storage: &dyn Storage) {
        self.input.take();
        self.state = Default::default();

        let items = storage.channels().map(|channel| ItemData {
            channel_id: channel.id,
            name: channel.name.clone(),
        });
        self.items.clear();
        self.items.extend(items);

        self.items.sort_unstable_by_key(|item| {
            let last_message_arrived_at = storage
                .message_id_at(item.channel_id, 0)
                .map(|id| id.arrived_at);
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

    fn filter_by_input(&mut self) {
        let index = self.items.iter().enumerate().filter_map(|(idx, item)| {
            if item
                .name
                .to_ascii_lowercase()
                .contains(&self.input.data.to_ascii_lowercase())
            {
                Some(idx)
            } else {
                None
            }
        });
        self.filtered_index.clear();
        self.filtered_index.extend(index);
    }

    pub fn filtered_names(&mut self) -> impl Iterator<Item = String> + '_ {
        self.filter_by_input();
        self.filtered_index
            .iter()
            .map(|&idx| self.items[idx].name.clone())
    }

    pub fn selected_channel_id(&self) -> Option<&ChannelId> {
        let idx = self.state.selected()?;
        let item_idx = self.filtered_index[idx];
        let item = &self.items[item_idx];
        Some(&item.channel_id)
    }
}

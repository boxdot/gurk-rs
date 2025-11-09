use std::{borrow::Cow, cell::RefCell};

use ratatui::widgets::{ListItem, ListItemsBuilder};
use uuid::Uuid;

use crate::{
    config::Config,
    data::{ChannelId, Message},
    storage::Storage,
    ui::{
        draw::{ShowReceipt, display_message},
        name_resolver::NameResolver,
    },
};

pub(super) struct MessageItemsBuilder<'a> {
    pub channel_id: ChannelId,
    pub num_messages: usize,
    pub storage: &'a dyn Storage,
    pub prefix_len: usize,
    pub user_id: Uuid,
    pub name_resolver: RefCell<NameResolver<'a>>,
    pub width: usize,
    pub height: usize,
    pub config: &'a Config,
    pub unread_messages: u32,
}

impl<'a> ListItemsBuilder<'a> for MessageItemsBuilder<'a> {
    fn len(&self) -> usize {
        self.num_messages
    }

    fn build(&self, index: usize) -> Option<Cow<'_, ListItem<'a>>> {
        let message_id = self.storage.message_id_at(self.channel_id, index)?;
        let message = self.storage.message(message_id)?;
        let is_first_unread = self.unread_messages == index as u32 + 1;
        self.render_message(message, is_first_unread)
            .map(Cow::Owned)
    }
}

impl<'a> MessageItemsBuilder<'a> {
    fn render_message(
        &self,
        message: Cow<'_, Message>,
        is_first_unread: bool,
    ) -> Option<ListItem<'a>> {
        let show_receipt = ShowReceipt::from_msg(&message, self.user_id, self.config.show_receipts);
        let prefix = " ".repeat(self.prefix_len);
        let mut name_resolver = self.name_resolver.borrow_mut();
        display_message(
            &mut name_resolver,
            &message,
            &prefix,
            self.width,
            self.height,
            show_receipt,
            self.config.colored_messages,
            is_first_unread,
        )
    }
}

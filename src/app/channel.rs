use uuid::Uuid;

use crate::data::{Channel, ChannelId, Message, TypingSet};
use crate::signal::{GroupMasterKeyBytes, ProfileKeyBytes, ResolvedGroup};
use crate::util;

use super::App;

impl App {
    pub(super) fn reset_message_selection(&mut self) {
        if let Some(channel_id) = self.channels.selected_item()
            && let Some(messages) = self.messages.get_mut(channel_id)
        {
            messages.state.select(None);
            messages.rendered = Default::default();
        }
    }

    pub fn select_previous_channel(&mut self) {
        self.channels.previous();
        self.on_channel_changed();
    }

    pub fn select_next_channel(&mut self) {
        self.channels.next();
        self.on_channel_changed();
    }

    pub fn on_pgup(&mut self) {
        if let Some(channel_id) = self.channels.selected_item() {
            self.messages
                .get_mut(channel_id)
                .expect("non-existent channel")
                .next();
            self.selected_message();
        }
    }

    pub fn on_pgdn(&mut self) {
        if let Some(channel_id) = self.channels.selected_item() {
            self.messages
                .get_mut(channel_id)
                .expect("non-existent channel")
                .previous()
        }
    }

    pub fn reset_unread_messages(&mut self) {
        if let Some(channel_id) = self.channels.selected_item()
            && let Some(channel) = self.storage.channel(*channel_id)
            && channel.unread_messages > 0
        {
            let mut channel = channel.into_owned();
            channel.unread_messages = 0;
            self.storage.store_channel(channel);
        }
    }

    pub(super) async fn ensure_group_channel_exists(
        &mut self,
        master_key: GroupMasterKeyBytes,
        revision: u32,
    ) -> anyhow::Result<usize> {
        let channel_id = ChannelId::from_master_key_bytes(master_key)?;
        if let Some(channel_idx) = self.channels.items.iter().position(|id| id == &channel_id) {
            // existing channel
            let channel = self
                .storage
                .channel(channel_id)
                .expect("non-existent channel");

            let is_stale = match channel.group_data.as_ref() {
                Some(group_data) => group_data.revision != revision,
                None => true,
            };
            if is_stale {
                let ResolvedGroup {
                    name,
                    group_data,
                    profile_keys,
                } = self.signal_manager.resolve_group(master_key).await?;

                let mut channel = channel.into_owned();

                self.ensure_users_are_known(group_data.members.iter().copied().zip(profile_keys))
                    .await;

                channel.name = name;
                channel.group_data = Some(group_data);
                self.storage.store_channel(channel);
            }
            Ok(channel_idx)
        } else {
            // new channel
            let ResolvedGroup {
                name,
                group_data,
                profile_keys,
            } = self.signal_manager.resolve_group(master_key).await?;

            self.ensure_users_are_known(group_data.members.iter().copied().zip(profile_keys))
                .await;

            let channel = Channel {
                id: channel_id,
                name,
                group_data: Some(group_data),
                unread_messages: 0,
                muted: false,
                typing: TypingSet::GroupTyping(Default::default()),
                expire_timer: None,
            };
            self.storage.store_channel(channel);

            let channel_idx = self.channels.items.len();
            self.channels.items.push(channel_id);

            Ok(channel_idx)
        }
    }

    pub(super) async fn ensure_user_is_known(
        &mut self,
        uuid: Uuid,
        profile_key: Option<ProfileKeyBytes>,
    ) {
        // is_known <=>
        //   * in names, and
        //   * is not empty
        //   * is not a phone numbers, and
        //   * is not their uuid
        let is_known = self
            .storage
            .name(uuid)
            .filter(|name| {
                !name.is_empty()
                    && !util::is_phone_number(name)
                    && Uuid::parse_str(name) != Ok(uuid)
            })
            .is_some();
        if !is_known {
            let name = if let Some(name) = self.signal_manager.profile_name(uuid).await {
                name
            } else {
                match profile_key {
                    Some(profile_key) => {
                        // try to resolve from signal service via their profile
                        self.signal_manager
                            .resolve_profile_name(uuid, profile_key)
                            .await
                            .unwrap_or_else(|| uuid.to_string())
                    }
                    None => {
                        // cannot be resolved
                        uuid.to_string()
                    }
                }
            };
            self.storage.store_name(uuid, name);
        }
    }

    async fn ensure_users_are_known(
        &mut self,
        users_with_keys: impl Iterator<Item = (Uuid, ProfileKeyBytes)>,
    ) {
        // TODO: Run in parallel
        for (uuid, profile_key) in users_with_keys {
            self.ensure_user_is_known(uuid, Some(profile_key)).await;
        }
    }

    pub(super) fn ensure_own_channel_exists(&mut self) -> usize {
        let user_id = self.user_id;
        if let Some(channel_idx) = self
            .channels
            .items
            .iter()
            .position(|channel_id| channel_id == &user_id)
        {
            channel_idx
        } else {
            let channel = Channel {
                id: user_id.into(),
                name: self.config.user.display_name.clone(),
                group_data: None,
                unread_messages: 0,
                muted: false,
                typing: TypingSet::SingleTyping(false),
                expire_timer: None,
            };
            let channel = self.storage.store_channel(channel);

            let channel_idx = self.channels.items.len();
            self.channels.items.push(channel.id);

            channel_idx
        }
    }

    pub(crate) async fn ensure_contact_channel_exists(&mut self, uuid: Uuid, name: &str) -> usize {
        if let Some(channel_idx) = self
            .channels
            .items
            .iter()
            .position(|channel_id| channel_id == &uuid)
        {
            let channel = self
                .storage
                .channel(uuid.into())
                .expect("non-existent channel");
            if channel.name != name {
                let mut channel = channel.into_owned();
                channel.name = name.to_string();
                self.storage.store_channel(channel);
            }
            channel_idx
        } else {
            let channel = Channel {
                id: uuid.into(),
                name: name.to_string(),
                group_data: None,
                unread_messages: 0,
                muted: false,
                typing: TypingSet::SingleTyping(false),
                expire_timer: None,
            };
            let channel = self.storage.store_channel(channel);

            let channel_idx = self.channels.items.len();
            self.channels.items.push(channel.id);

            channel_idx
        }
    }

    pub(super) fn add_message_to_channel(&mut self, channel_idx: usize, mut message: Message) {
        let channel_id = self.channels.items[channel_idx];

        // Eagerly activate timer for messages arriving in the currently viewed channel
        if message.expire_timer.is_some_and(|t| t > 0)
            && message.expires_at.is_none()
            && self.timers_activated_for == Some(channel_id)
        {
            let timer = message.expire_timer.unwrap();
            let now_ms = crate::util::utc_now_timestamp_msec();
            message.expires_at = Some(now_ms + u64::from(timer) * 1000);
        }

        let message = self.storage.store_message(channel_id, message);
        let from_current_user = self.user_id == message.from_id;

        let messages = self.messages.entry(channel_id).or_default();
        messages.items.push(message.arrived_at);

        if let Some(idx) = messages.state.selected() {
            // keep selection on the old message
            messages.state.select(Some(idx + 1));
        }

        self.touch_channel(channel_idx, from_current_user);
    }

    pub(super) fn remove_message_from_view(&mut self, channel_id: ChannelId, arrived_at: u64) {
        if let Some(messages) = self.messages.get_mut(&channel_id)
            && let Some(pos) = messages.items.iter().position(|&ts| ts == arrived_at)
        {
            messages.items.remove(pos);
            if let Some(selected) = messages.state.selected() {
                if messages.items.is_empty() {
                    messages.state.select(None);
                } else if selected > 0 && selected >= messages.items.len() {
                    messages.state.select(Some(selected - 1));
                }
            }
        }
    }

    pub(crate) fn touch_channel(&mut self, channel_idx: usize, from_current_user: bool) {
        if !from_current_user && self.channels.state.selected() != Some(channel_idx) {
            let channel_id = self.channels.items[channel_idx];
            let mut channel = self
                .storage
                .channel(channel_id)
                .expect("non-existent channel")
                .into_owned();
            channel.unread_messages += 1;
            self.storage.store_channel(channel);
        } else {
            self.reset_unread_messages();
        }

        self.bubble_up_channel(channel_idx);
    }

    pub(super) fn bubble_up_channel(&mut self, channel_idx: usize) {
        // bubble up channel to the beginning of the list
        let channels = &mut self.channels;
        for (prev, next) in (0..channel_idx).zip(1..channel_idx + 1).rev() {
            channels.items.swap(prev, next);
        }
        match channels.state.selected() {
            Some(selected_idx) if selected_idx == channel_idx => channels.state.select(Some(0)),
            Some(selected_idx) if selected_idx < channel_idx => {
                channels.state.select(Some(selected_idx + 1));
            }
            _ => {}
        }
    }

    pub fn select_channel_prev(&mut self) {
        self.select_channel.prev();
    }

    pub fn select_channel_next(&mut self) {
        self.select_channel.next();
    }

    /// Reset dwell tracking when channel selection changes
    pub fn on_channel_changed(&mut self) {
        self.channel_selected_at = std::time::Instant::now();
        self.timers_activated_for = None;
    }

    pub fn toggle_mute_channel(&mut self) {
        if let Some(&channel_id) = self.channels.selected_item()
            && let Some(channel) = self.storage.channel(channel_id)
        {
            let mut channel = channel.into_owned();
            channel.muted = !channel.muted;
            self.storage.store_channel(channel);
        }
    }

    /// Activate expire timers for messages in the currently viewed channel.
    /// Runs once per channel selection after a 10-second dwell.
    pub fn activate_expire_timers(&mut self) {
        if self.channel_selected_at.elapsed() < std::time::Duration::from_secs(10) {
            return;
        }
        let Some(&channel_id) = self.channels.selected_item() else {
            return;
        };
        if self.timers_activated_for == Some(channel_id) {
            return;
        }
        let has_timer = self
            .storage
            .channel(channel_id)
            .is_some_and(|c| c.expire_timer.is_some_and(|t| t > 0));
        if !has_timer {
            return;
        }

        let now_ms = crate::util::utc_now_timestamp_msec();
        let to_activate: Vec<(u64, u32)> = self
            .storage
            .messages(channel_id)
            .filter_map(|msg| {
                if msg.expires_at.is_none() && msg.expire_timer.is_some_and(|t| t > 0) {
                    Some((msg.arrived_at, msg.expire_timer.unwrap()))
                } else {
                    None
                }
            })
            .collect();

        for (arrived_at, timer) in to_activate {
            let message_id = crate::storage::MessageId::new(channel_id, arrived_at);
            if let Some(mut msg) = self.storage.message(message_id).map(|m| m.into_owned()) {
                msg.expires_at = Some(now_ms + u64::from(timer) * 1000);
                self.storage.store_message(channel_id, msg);
            }
        }

        self.timers_activated_for = Some(channel_id);
    }

    /// Remove messages that have expired.
    /// All timestamps are UTC milliseconds — no locale dependency.
    pub fn expire_messages(&mut self) {
        let now_ms = crate::util::utc_now_timestamp_msec();
        let channel_ids: Vec<ChannelId> = self
            .channels
            .items
            .iter()
            .copied()
            .filter(|&id| {
                self.storage
                    .channel(id)
                    .is_some_and(|c| c.expire_timer.is_some_and(|t| t > 0))
            })
            .collect();

        for channel_id in channel_ids {
            let expired: Vec<u64> = self
                .storage
                .messages(channel_id)
                .filter(|msg| msg.expires_at.is_some_and(|ea| now_ms > ea))
                .map(|msg| msg.arrived_at)
                .collect();

            for arrived_at in expired {
                let message_id = crate::storage::MessageId::new(channel_id, arrived_at);
                self.storage.remove_message(message_id);
                self.remove_message_from_view(channel_id, arrived_at);
            }
        }
    }
}

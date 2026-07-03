use uuid::Uuid;

use crate::util;
use crate::{
    app::window::MessageWindow,
    data::{Channel, ChannelId, Message, TypingSet},
};
use crate::{
    app::window::PAGE,
    signal::{GroupMasterKeyBytes, ProfileKeyBytes, ResolvedGroup},
};

use super::App;

/// Tracks selection of a message in a channel.
#[derive(Default, Clone)]
pub(crate) struct ChannelPosition {
    /// Selected message
    ///
    /// None = tail-follow
    pub(crate) selected: Option<u64>,
    /// Scroll position
    ///
    /// None = newest
    pub(crate) viewport_bottom: Option<u64>,
}

impl App {
    // Channel API: access from memory + write to storage

    pub(crate) fn channels(&self) -> &[Channel] {
        &self.channels.items
    }

    pub(crate) fn channel(&self, channel_id: ChannelId) -> Option<&Channel> {
        self.channels.items.iter().find(|c| c.id == channel_id)
    }

    pub(crate) fn selected_channel(&self) -> Option<&Channel> {
        self.channels.selected_item()
    }

    pub fn selected_channel_id(&self) -> Option<ChannelId> {
        self.channels.selected_item().map(|c| c.id)
    }

    pub(crate) fn store_channel(&mut self, channel: Channel) {
        let channel = self.storage.store_channel(channel).into_owned();
        if let Some(old_channel) = self.channels.items.iter_mut().find(|c| c.id == channel.id) {
            *old_channel = channel;
        } else {
            self.channels.items.push(channel);
        }
    }

    pub(super) fn reset_message_selection(&mut self) {
        if let Some(channel_id) = self.selected_channel_id()
            && let Some(pos) = self.positions.get_mut(&channel_id)
        {
            pos.selected = None;
            pos.viewport_bottom = None;
        }
    }

    pub fn select_previous_channel(&mut self) {
        self.reset_unread_messages();
        let old = self.selected_channel_id();
        self.channels.previous();
        let new = self.selected_channel_id();
        self.swap_channel_draft(old, new);
        self.on_channel_changed();
    }

    pub fn select_next_channel(&mut self) {
        self.reset_unread_messages();
        let old = self.selected_channel_id();
        self.channels.next();
        let new = self.selected_channel_id();
        self.swap_channel_draft(old, new);
        self.on_channel_changed();
    }

    pub fn on_pgup(&mut self) {
        let Some(window) = self.window.as_mut() else {
            return;
        };
        let pos = self.positions.entry(window.channel_id()).or_default();
        let Some(selected) = pos.selected else {
            // first press: select the bottom-most message, don't move yet
            pos.selected = pos.viewport_bottom.or_else(|| window.newest());
            return;
        };
        match window.older(selected) {
            Some(prev) => pos.selected = Some(prev),
            None if !window.at_oldest() => {
                window.extend_older(&*self.storage, PAGE);
                if let Some(older) = window.older(selected) {
                    pos.selected = Some(older);
                }
            }
            None => {} // at the oldest message
        }
    }

    pub fn on_pgdn(&mut self) {
        let Some(window) = self.window.as_mut() else {
            return;
        };
        let pos = self.positions.entry(window.channel_id()).or_default();
        let Some(cur) = pos.selected.or(pos.viewport_bottom).or(window.newest()) else {
            return;
        };
        match window.newer(cur) {
            Some(next) => pos.selected = Some(next),
            None if !window.at_newest() => {
                window.extend_newer(&*self.storage, PAGE);
                if let Some(newer) = window.newer(cur) {
                    pos.selected = Some(newer);
                }
            }
            None => {} // at the newest message
        }
    }

    pub fn reset_unread_messages(&mut self) {
        if let Some(channel) = self.selected_channel()
            && channel.unread_messages > 0
        {
            let mut channel = channel.clone();
            channel.unread_messages = 0;
            self.storage.store_channel(channel);
        }
    }

    pub(super) async fn ensure_group_channel_exists(
        &mut self,
        master_key: GroupMasterKeyBytes,
        revision: u32,
    ) -> anyhow::Result<ChannelId> {
        let channel_id = ChannelId::from_master_key_bytes(master_key)?;
        if let Some(channel) = self.channel(channel_id) {
            let is_stale = match channel.group_data.as_ref() {
                Some(group_data) => group_data.revision != revision,
                None => true,
            };
            if is_stale {
                let mut channel = channel.clone();

                let ResolvedGroup {
                    name,
                    group_data,
                    profile_keys,
                } = self.signal_manager.resolve_group(master_key).await?;

                self.ensure_users_are_known(group_data.members.iter().copied().zip(profile_keys))
                    .await;

                channel.name = name;
                channel.group_data = Some(group_data);
                self.store_channel(channel);
            }
            Ok(channel_id)
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
            self.store_channel(channel);

            Ok(channel_id)
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

    pub(super) fn ensure_own_channel_exists(&mut self) -> ChannelId {
        let user_id = self.user_id;
        let channel_id = ChannelId::User(user_id);
        if self.channel(channel_id).is_none() {
            let channel = Channel {
                id: user_id.into(),
                name: self.config.user.display_name.clone(),
                group_data: None,
                unread_messages: 0,
                muted: false,
                typing: TypingSet::SingleTyping(false),
                expire_timer: None,
            };
            self.store_channel(channel);
        }
        channel_id
    }

    pub(crate) async fn ensure_contact_channel_exists(
        &mut self,
        uuid: Uuid,
        name: &str,
    ) -> ChannelId {
        let channel_id = ChannelId::User(uuid);
        if let Some(channel) = self.channel(channel_id) {
            if channel.name != name {
                let mut channel = channel.clone();
                channel.name = name.to_string();
                self.store_channel(channel);
            }
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
            self.store_channel(channel);
        }
        channel_id
    }

    pub(super) fn add_message_to_channel(&mut self, channel_id: ChannelId, mut message: Message) {
        let Some(channel) = self.channel(channel_id) else {
            return;
        };

        // Eagerly activate timer for messages arriving in the currently viewed channel
        if message.expire_timer.is_some_and(|t| t > 0)
            && message.expires_at.is_none()
            && self.timers_activated_for == Some(channel.id)
        {
            let timer = message.expire_timer.unwrap();
            let now_ms = crate::util::utc_now_timestamp_msec();
            message.expires_at = Some(now_ms + u64::from(timer) * 1000);
            self.schedule_expiry(message.expires_at);
        }

        let from_current_user = self.user_id == message.from_id;
        self.store_message(channel_id, message);
        self.touch_channel(channel_id, from_current_user);
    }

    pub(super) fn remove_message_from_view(&mut self, channel_id: ChannelId, arrived_at: u64) {
        let Some(window) = self.window.as_mut() else {
            return;
        };
        if window.channel_id() != channel_id {
            return;
        }

        let pos = self.positions.entry(channel_id).or_default();
        if pos.selected == Some(arrived_at) {
            pos.selected = window
                .older(arrived_at)
                .or_else(|| window.newer(arrived_at));
        }
        if pos.viewport_bottom == Some(arrived_at) {
            pos.viewport_bottom = window.older(arrived_at);
        }
        window.remove(arrived_at);
    }

    pub(crate) fn touch_channel(&mut self, channel_id: ChannelId, from_current_user: bool) {
        if self.selected_channel_id() == Some(channel_id) {
            return;
        }

        if !from_current_user
            && let Some(channel) = self.channel(channel_id)
            && channel.id != channel_id
        {
            let mut channel = channel.clone();
            channel.unread_messages += 1;
            self.storage.store_channel(channel);
        } else {
            self.reset_unread_messages();
        }

        self.bubble_up_channel(channel_id);
    }

    pub(super) fn bubble_up_channel(&mut self, channel_id: ChannelId) {
        // bubble up channel to the beginning of the list
        let channels = &mut self.channels;
        let Some(channel_idx) = channels
            .items
            .iter()
            .position(|channel| channel.id == channel_id)
        else {
            return;
        };
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
        let Some(channel) = self.channels.selected_item() else {
            return;
        };
        let pos = self.positions.entry(channel.id).or_default();
        let mut window = MessageWindow::new(channel.id);
        match pos.viewport_bottom {
            Some(anchor) => window.load_around(&*self.storage, anchor, PAGE),
            None => window.load_tail(&*self.storage, PAGE),
        }
        self.window = Some(window);

        self.channel_selected_at = std::time::Instant::now();
        self.timers_activated_for = None;
    }

    pub fn toggle_mute_channel(&mut self) {
        if let Some(channel) = self.channels.selected_item() {
            let mut channel = channel.clone();
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
        let Some(channel) = self.selected_channel() else {
            return;
        };
        let channel_id = channel.id;
        if self.timers_activated_for == Some(channel_id) {
            return;
        }
        let has_timer = channel.expire_timer.is_some_and(|t| t > 0);
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
                self.schedule_expiry(msg.expires_at);
                self.store_message(channel_id, msg);
            }
        }

        self.timers_activated_for = Some(channel_id);
    }

    fn schedule_expiry(&mut self, at: Option<u64>) {
        self.next_expiring_at = [self.next_expiring_at, at].into_iter().flatten().min();
    }

    /// Remove messages that have expired.
    ///
    /// This function is called per-frame, therefore it should not hit the storage all the time.
    pub fn expire_messages(&mut self) {
        let now_ms = crate::util::utc_now_timestamp_msec();

        if self.next_expiring_at.is_none_or(|t| now_ms < t) {
            return; // nothing to expire yet
        }

        let removed = self.storage.remove_expired(now_ms);
        for message_id in removed {
            self.remove_message_from_view(message_id.channel_id, message_id.arrived_at);
        }

        self.next_expiring_at = self.storage.next_expiring_at();
    }
}

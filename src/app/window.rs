use std::collections::VecDeque;

use crate::{
    app::App,
    data::{ChannelId, Message},
    storage::{MessageId, Storage},
};

/// Fetch granularity for load_*/extend_*
pub(crate) const PAGE: usize = 64;

/// Trim threshold
///
/// Must be >> viewport rows * 2*PAGE
pub(crate) const MAX_WINDOW: usize = 512;

/// How much to fill the window when ensuring it is filled
const FILL_MARGIN: usize = PAGE;

/// A window of loaded messages in a channel
///
/// # Invariants:
///
/// * `items` is strictly ascending by arrived_at and contiguous
/// * no message exists in storage between `front` and `back` of `items`
/// * `at_oldest` <=> nothing older than `front` exists in the storage
/// * `at_newest` <=> nothing newer than `back` exists in the storage
/// * `is_empty` <=> `at_oldest` and `at_newest` and `items` is empty (empty channel)
pub(crate) struct MessageWindow {
    channel_id: ChannelId,
    /// Contiguous list of messages in the window strictly ascending by arrived_at.
    items: VecDeque<Message>,
    /// If true, there is nothing older than `front` in the storage.
    at_oldest: bool,
    /// If true, there is nothing newer than `back` in the storage.
    at_newest: bool,
}

impl App {
    /// Ensures the message window is filled with enough messages to fill the viewport
    pub fn ensure_message_window_filled(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let channel_id = window.channel_id();
        let anchor = self
            .positions
            .get(&channel_id)
            .and_then(|p| p.selected.or(p.viewport_bottom));
        let target = self.message_view_height + FILL_MARGIN;
        let window = self.window.as_mut().expect("logic error: no window");
        while !window.at_oldest() && window.loaded_above(anchor) < target {
            window.extend_older(&*self.storage, PAGE);
        }
        while !window.at_newest() && window.loaded_below(anchor) < target {
            window.extend_newer(&*self.storage, PAGE);
        }
    }
}

impl MessageWindow {
    /// Creates an empty channel (nothing loaded yet)
    pub(crate) fn new(channel_id: ChannelId) -> Self {
        Self {
            channel_id,
            items: Default::default(),
            at_oldest: true,
            at_newest: true,
        }
    }

    /// Load newest `limit` messages.
    pub(crate) fn load_tail(&mut self, storage: &dyn Storage, limit: usize) {
        self.items = storage.messages_tail(self.channel_id, limit).into();
        self.at_newest = true;
        self.at_oldest = self.items.len() < limit;
    }

    pub(crate) fn load_around(&mut self, storage: &dyn Storage, anchor: u64, limit: usize) {
        self.items = storage
            .messages_before(self.channel_id, anchor, limit)
            .into();
        self.at_oldest = self.items.len() < limit;
        if let Some(message) = storage.message(MessageId::new(self.channel_id, anchor)) {
            self.items.push_back(message.into_owned());
        }
        let suffix = storage.messages_after(self.channel_id, anchor, limit);
        self.at_newest = suffix.len() < limit;
        self.items.extend(suffix);
    }

    // Grow at edges

    pub(crate) fn extend_older(&mut self, storage: &dyn Storage, count: usize) {
        if self.at_oldest {
            return;
        }
        let Some(oldest) = self.oldest() else {
            return;
        };
        let prefix = storage.messages_before(self.channel_id, oldest, count);
        self.at_oldest = prefix.len() < count;
        for message in prefix.into_iter().rev() {
            self.items.push_front(message);
        }
        if self.items.len() > MAX_WINDOW {
            self.items.truncate(MAX_WINDOW);
            self.at_newest = false;
        }
    }

    pub(crate) fn extend_newer(&mut self, storage: &dyn Storage, count: usize) {
        if self.at_newest {
            return;
        }
        let Some(newest) = self.newest() else {
            return;
        };
        let suffix = storage.messages_after(self.channel_id, newest, count);
        self.at_newest = suffix.len() < count;
        self.items.extend(suffix);
        if self.items.len() > MAX_WINDOW {
            let to_remove = self.items.len() - MAX_WINDOW;
            for _ in 0..to_remove {
                self.items.pop_front();
            }
            self.at_oldest = false;
        }
    }

    // Mutate

    pub(crate) fn upsert(&mut self, message: Message) {
        match self
            .items
            .binary_search_by_key(&message.arrived_at, |m| m.arrived_at)
        {
            Ok(pos) => {
                self.items[pos] = message;
            }
            Err(pos) => {
                enum Trim {
                    None,
                    Front,
                    Back,
                }

                let (should_insert, trim) = match (self.oldest(), self.newest()) {
                    (Some(oldest), Some(newest))
                        if oldest < message.arrived_at && message.arrived_at < newest =>
                    {
                        // strictly interior; trimming at front is an arbitrary choice
                        (true, Trim::Front)
                    }
                    (Some(oldest), _) if message.arrived_at < oldest && self.at_oldest => {
                        // at the oldest edge
                        (true, Trim::Back)
                    }
                    (_, Some(newest)) if newest < message.arrived_at && self.at_newest => {
                        // at the newest edge
                        (true, Trim::Front)
                    }
                    (None, None) if self.at_newest => {
                        // empty and at the newest edge
                        (true, Trim::None)
                    }
                    _ => (false, Trim::None),
                };

                if should_insert {
                    self.items.insert(pos, message);
                    if self.items.len() > MAX_WINDOW {
                        match trim {
                            Trim::None => {}
                            Trim::Front => {
                                self.items.pop_front();
                                self.at_oldest = false;
                            }
                            Trim::Back => {
                                self.items.pop_back();
                                self.at_newest = false;
                            }
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn remove(&mut self, arrived_at: u64) {
        let Ok(pos) = self
            .items
            .binary_search_by_key(&arrived_at, |m| m.arrived_at)
        else {
            return;
        };
        self.items.remove(pos);
    }

    // Read

    pub(crate) fn channel_id(&self) -> ChannelId {
        self.channel_id
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty() && self.at_oldest && self.at_newest
    }

    /// Number of loaded messages at or newer than `anchor` (all if `None`)
    pub(crate) fn loaded_below(&self, anchor: Option<u64>) -> usize {
        match anchor {
            Some(anchor) => match self.items.binary_search_by_key(&anchor, |m| m.arrived_at) {
                Ok(pos) => self.items.len() - pos,
                Err(insert_pos) => self.items.len() - insert_pos,
            },
            None => self.items.len(),
        }
    }

    /// Number of loaded messages at or older than `anchor` (all if `None`)
    pub(crate) fn loaded_above(&self, anchor: Option<u64>) -> usize {
        match anchor {
            Some(anchor) => match self.items.binary_search_by_key(&anchor, |m| m.arrived_at) {
                Ok(pos) => pos.saturating_add(1),
                Err(insert_pos) => insert_pos,
            },
            None => self.items.len(),
        }
    }

    pub(crate) fn get(&self, arrived_at: u64) -> Option<&Message> {
        let pos = self
            .items
            .binary_search_by_key(&arrived_at, |m| m.arrived_at)
            .ok()?;
        self.items.get(pos)
    }

    pub(crate) fn newest(&self) -> Option<u64> {
        self.items.back().map(|m| m.arrived_at)
    }

    pub(crate) fn oldest(&self) -> Option<u64> {
        self.items.front().map(|m| m.arrived_at)
    }

    pub(crate) fn at_oldest(&self) -> bool {
        self.at_oldest
    }

    pub(crate) fn at_newest(&self) -> bool {
        self.at_newest
    }

    pub(crate) fn older(&self, arrived_at: u64) -> Option<u64> {
        let pos = self
            .items
            .binary_search_by_key(&arrived_at, |m| m.arrived_at)
            .ok()?;
        let prev_pos = pos.checked_sub(1)?;
        self.items.get(prev_pos).map(|m| m.arrived_at)
    }

    pub(crate) fn newer(&self, arrived_at: u64) -> Option<u64> {
        let pos = self
            .items
            .binary_search_by_key(&arrived_at, |m| m.arrived_at)
            .ok()?;
        let next_pos = pos.checked_add(1)?;
        self.items.get(next_pos).map(|m| m.arrived_at)
    }

    pub(crate) fn iter_rev_from(
        &self,
        anchor: Option<u64>,
    ) -> impl DoubleEndedIterator<Item = &Message> {
        let end = match anchor {
            Some(anchor) => self
                .items
                .binary_search_by_key(&anchor, |m| m.arrived_at)
                .ok()
                .and_then(|pos| pos.checked_add(1))
                .unwrap_or(self.items.len()),
            None => self.items.len(),
        };
        self.items.range(..end).rev()
    }
}

#[cfg(test)]
mod tests {
    use uuid::{Uuid, uuid};

    use crate::data::{Channel, TypingSet};
    use crate::storage::SqliteStorage;

    use super::*;

    const FROM: Uuid = uuid!("a955d20f-6b83-4e69-846e-a99b1779ff7a");

    fn ch() -> ChannelId {
        ChannelId::User(uuid!("966960e0-a8cd-43f1-ac7a-2c986dd470cd"))
    }

    /// arrived_at values currently in the window, oldest -> newest
    fn ats(w: &MessageWindow) -> Vec<u64> {
        w.items.iter().map(|m| m.arrived_at).collect()
    }

    /// Build a window directly (no storage) for testing the in-memory operations.
    fn window(arrived_ats: &[u64], at_oldest: bool, at_newest: bool) -> MessageWindow {
        MessageWindow {
            channel_id: ch(),
            items: arrived_ats
                .iter()
                .map(|&a| Message::text(FROM, a, format!("m{a}")))
                .collect(),
            at_oldest,
            at_newest,
        }
    }

    async fn storage_with(arrived_ats: &[u64]) -> SqliteStorage {
        let url = "sqlite::memory:".parse().unwrap();
        let mut storage = SqliteStorage::open_unencrypted(&url).await.unwrap();
        storage.store_channel(Channel {
            id: ch(),
            name: "test".to_owned(),
            group_data: None,
            unread_messages: 0,
            muted: false,
            typing: TypingSet::new(false),
            expire_timer: None,
        });
        for &a in arrived_ats {
            storage.store_message(ch(), Message::text(FROM, a, format!("m{a}")));
        }
        storage
    }

    // --- read accessors (no storage) ---

    #[test]
    fn empty_window_accessors() {
        let w = window(&[], true, true);
        assert!(w.is_empty());
        assert_eq!(w.newest(), None);
        assert_eq!(w.oldest(), None);
        assert_eq!(w.get(10), None);
        assert_eq!(w.older(10), None);
        assert_eq!(w.newer(10), None);
        assert_eq!(w.iter_rev_from(None).count(), 0);
    }

    #[test]
    fn neighbors_and_get() {
        let w = window(&[10, 20, 30, 40, 50], true, true);
        assert_eq!(w.newest(), Some(50));
        assert_eq!(w.oldest(), Some(10));
        assert_eq!(w.get(30).map(|m| m.arrived_at), Some(30));
        assert_eq!(w.get(35), None);
        assert_eq!(w.older(30), Some(20));
        assert_eq!(w.newer(30), Some(40));
        assert_eq!(w.older(10), None); // nothing older in the window
        assert_eq!(w.newer(50), None); // nothing newer in the window
    }

    #[test]
    fn iter_rev_from_yields_newest_first() {
        let w = window(&[10, 20, 30, 40, 50], true, true);
        let all: Vec<u64> = w.iter_rev_from(None).map(|m| m.arrived_at).collect();
        assert_eq!(all, [50, 40, 30, 20, 10]);
        // anchor is inclusive, then older
        let from_30: Vec<u64> = w.iter_rev_from(Some(30)).map(|m| m.arrived_at).collect();
        assert_eq!(from_30, [30, 20, 10]);
    }

    #[test]
    fn loaded_above_counts_anchor_and_older() {
        let w = window(&[10, 20, 30, 40, 50], true, true);
        assert_eq!(w.loaded_above(None), 5); // tail: everything is above the newest
        assert_eq!(w.loaded_above(Some(50)), 5); // newest + all older
        assert_eq!(w.loaded_above(Some(30)), 3); // 30, 20, 10
        assert_eq!(w.loaded_above(Some(10)), 1); // just the oldest
    }

    #[test]
    fn loaded_above_non_member_counts_older() {
        let w = window(&[10, 20, 30, 40, 50], true, true);
        // a non-member anchor counts everything strictly older than it
        assert_eq!(w.loaded_above(Some(35)), 3); // 10, 20, 30
        assert_eq!(w.loaded_above(Some(5)), 0); // nothing at or older
        assert_eq!(w.loaded_above(Some(99)), 5); // all older
    }

    #[test]
    fn loaded_above_empty() {
        let w = window(&[], true, true);
        assert_eq!(w.loaded_above(None), 0);
        assert_eq!(w.loaded_above(Some(10)), 0);
    }

    // upsert (no storage); the four placement branches + replace

    #[test]
    fn upsert_replaces_existing_in_place() {
        let mut w = window(&[10, 20, 30], true, true);
        w.upsert(Message::text(FROM, 20, "edited".to_owned()));
        assert_eq!(ats(&w), [10, 20, 30]);
        assert_eq!(w.get(20).unwrap().message.as_deref(), Some("edited"));
    }

    #[test]
    fn upsert_appends_newer_only_when_at_newest() {
        let mut at_newest = window(&[10, 20, 30], true, true);
        at_newest.upsert(Message::text(FROM, 40, "m40".to_owned()));
        assert_eq!(ats(&at_newest), [10, 20, 30, 40]);

        // not at newest: a newer-than-back message would create a gap -> dropped
        let mut scrolled = window(&[10, 20, 30], true, false);
        scrolled.upsert(Message::text(FROM, 40, "m40".to_owned()));
        assert_eq!(ats(&scrolled), [10, 20, 30]);
    }

    #[test]
    fn upsert_prepends_older_only_when_at_oldest() {
        let mut at_oldest = window(&[20, 30], true, true);
        at_oldest.upsert(Message::text(FROM, 10, "m10".to_owned()));
        assert_eq!(ats(&at_oldest), [10, 20, 30]);

        let mut not_oldest = window(&[20, 30], false, true);
        not_oldest.upsert(Message::text(FROM, 10, "m10".to_owned()));
        assert_eq!(ats(&not_oldest), [20, 30]); // dropped
    }

    #[test]
    fn upsert_inserts_interior_regardless_of_flags() {
        let mut w = window(&[10, 30], false, false);
        w.upsert(Message::text(FROM, 20, "m20".to_owned()));
        assert_eq!(ats(&w), [10, 20, 30]);
    }

    #[test]
    fn upsert_into_empty_at_newest() {
        let mut w = window(&[], true, true);
        w.upsert(Message::text(FROM, 10, "m10".to_owned()));
        assert_eq!(ats(&w), [10]);
    }

    // remove (no storage)

    #[test]
    fn remove_interior_front_back_and_absent() {
        let mut w = window(&[10, 20, 30], true, true);
        w.remove(25); // absent -> no-op
        assert_eq!(ats(&w), [10, 20, 30]);
        w.remove(20); // interior
        assert_eq!(ats(&w), [10, 30]);
        w.remove(10); // front
        w.remove(30); // back
        assert!(ats(&w).is_empty());
    }

    // load_tail (storage)

    #[tokio::test(flavor = "multi_thread")]
    async fn load_tail_returns_newest_ascending() {
        let storage = storage_with(&[10, 20, 30, 40, 50, 60, 70, 80, 90]).await;
        let mut w = MessageWindow::new(ch());
        w.load_tail(&storage, 3);
        assert_eq!(ats(&w), [70, 80, 90]);
        assert!(w.at_newest());
        assert!(!w.at_oldest());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn load_tail_short_result_sets_at_oldest() {
        let storage = storage_with(&[10, 20, 30]).await;
        let mut w = MessageWindow::new(ch());
        w.load_tail(&storage, 10);
        assert_eq!(ats(&w), [10, 20, 30]);
        assert!(w.at_newest());
        assert!(w.at_oldest());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn load_tail_empty_channel() {
        let storage = storage_with(&[]).await;
        let mut w = MessageWindow::new(ch());
        w.load_tail(&storage, 10);
        assert!(w.is_empty());
        assert!(w.at_oldest() && w.at_newest());
    }

    // load_around (storage); `limit` = margin on each side of the anchor

    #[tokio::test(flavor = "multi_thread")]
    async fn load_around_centers_on_anchor() {
        let storage = storage_with(&[10, 20, 30, 40, 50, 60, 70, 80, 90]).await;
        let mut w = MessageWindow::new(ch());
        w.load_around(&storage, 50, 2);
        assert_eq!(ats(&w), [30, 40, 50, 60, 70]);
        assert!(!w.at_oldest());
        assert!(!w.at_newest());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn load_around_at_oldest_edge() {
        let storage = storage_with(&[10, 20, 30, 40, 50]).await;
        let mut w = MessageWindow::new(ch());
        w.load_around(&storage, 10, 2);
        assert_eq!(ats(&w), [10, 20, 30]);
        assert!(w.at_oldest());
        assert!(!w.at_newest());
    }

    // extend_older / extend_newer (storage)

    #[tokio::test(flavor = "multi_thread")]
    async fn extend_older_pages_in_and_stops_at_oldest() {
        let storage = storage_with(&[10, 20, 30, 40, 50]).await;
        let mut w = MessageWindow::new(ch());
        w.load_tail(&storage, 2);
        assert_eq!(ats(&w), [40, 50]);

        w.extend_older(&storage, 2);
        assert_eq!(ats(&w), [20, 30, 40, 50]);
        assert!(!w.at_oldest());

        w.extend_older(&storage, 2); // only 10 left -> short -> at_oldest
        assert_eq!(ats(&w), [10, 20, 30, 40, 50]);
        assert!(w.at_oldest());

        w.extend_older(&storage, 2); // no-op at the oldest
        assert_eq!(ats(&w), [10, 20, 30, 40, 50]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn extend_newer_pages_in_and_stops_at_newest() {
        let storage = storage_with(&[10, 20, 30, 40, 50]).await;
        let mut w = MessageWindow::new(ch());
        w.load_around(&storage, 20, 1); // [10, 20, 30], not at newest
        assert_eq!(ats(&w), [10, 20, 30]);
        assert!(!w.at_newest());

        w.extend_newer(&storage, 1);
        assert_eq!(ats(&w), [10, 20, 30, 40]);
        assert!(!w.at_newest());

        w.extend_newer(&storage, 5); // only 50 left -> short -> at_newest
        assert_eq!(ats(&w), [10, 20, 30, 40, 50]);
        assert!(w.at_newest());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn extend_older_trims_the_far_edge_past_max_window() {
        let arrived_ats: Vec<u64> = (1..=(MAX_WINDOW as u64 + 100)).collect();
        let storage = storage_with(&arrived_ats).await;
        let mut w = MessageWindow::new(ch());

        w.load_tail(&storage, MAX_WINDOW);
        assert_eq!(w.items.len(), MAX_WINDOW);
        assert!(w.at_newest());
        assert!(!w.at_oldest());
        let newest_before = w.newest();

        w.extend_older(&storage, PAGE); // pushes over MAX_WINDOW -> trim the newest edge
        assert_eq!(w.items.len(), MAX_WINDOW);
        assert!(!w.at_newest()); // newest end was trimmed
        assert!(!w.at_oldest());
        assert!(w.newest() < newest_before); // bottom moved older after trim
    }
}

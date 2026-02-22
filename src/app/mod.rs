use std::borrow::Cow;
use std::cell::Cell;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context as _;
use itertools::Itertools;
use regex::Regex;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};
use uuid::Uuid;

use crate::channels::SelectChannel;
use crate::command::{ModeKeybinding, get_keybindings};
use crate::config::Config;
use crate::data::{Channel, ChannelId, Message, TypingSet};
use crate::event::Event;
use crate::input::Input;
use crate::receipt::ReceiptHandler;
use crate::signal::{Attachment, SignalManager};
use crate::storage::{MessageId, Storage};
use crate::util::StatefulList;

use presage::proto::data_message::Sticker;

mod channel;
mod input;
mod message;

pub struct App {
    pub config: Config,
    signal_manager: Box<dyn SignalManager>,
    pub storage: Box<dyn Storage>,
    pub channels: StatefulList<ChannelId>,
    pub messages: BTreeMap<ChannelId, StatefulList<u64 /* arrived at*/>>,
    pub help_scroll: (u16, u16),
    pub user_id: Uuid,
    pub should_quit: bool,
    display_help: bool,
    receipt_handler: ReceiptHandler,
    pub input: Input,
    pub is_multiline_input: bool,
    editing: Option<MessageId>,
    pub(crate) select_channel: SelectChannel,
    clipboard: Option<arboard::Clipboard>,
    event_tx: mpsc::UnboundedSender<Event>,
    // It is expensive to hit the signal manager contacts storage, so we cache it
    names_cache: Cell<Option<BTreeMap<Uuid, String>>>,
    pub mode_keybindings: ModeKeybinding,
}

impl App {
    pub fn try_new(
        config: Config,
        signal_manager: Box<dyn SignalManager>,
        storage: Box<dyn Storage>,
    ) -> anyhow::Result<(Self, mpsc::UnboundedReceiver<Event>)> {
        let user_id = signal_manager.user_id();

        // build index of channels and messages for using them as lists content
        let mut channels: StatefulList<ChannelId> = Default::default();
        let mut messages: BTreeMap<_, StatefulList<_>> = BTreeMap::new();
        for channel in storage.channels() {
            channels.items.push(channel.id);
            let channel_messages = &mut messages.entry(channel.id).or_default().items;
            for message in storage.messages(channel.id) {
                channel_messages.push(message.arrived_at);
            }
        }
        channels.items.sort_unstable_by_key(|channel_id| {
            let last_message_arrived_at = storage
                .messages(*channel_id)
                .next_back()
                .map(|msg| msg.arrived_at);
            let channel_name = storage
                .channel(*channel_id)
                .map(|channel| channel.name.clone());
            (Reverse(last_message_arrived_at), channel_name)
        });
        channels.next();

        let clipboard = arboard::Clipboard::new()
            .map_err(|error| warn!(%error, "clipboard disabled"))
            .ok();

        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let mode_keybindings = get_keybindings(&config.keybindings, config.default_keybindings)
            .expect("keybinding configuration failed");

        let app = Self {
            config,
            signal_manager,
            user_id,
            storage,
            channels,
            messages,
            help_scroll: (0, 0),
            should_quit: false,
            display_help: false,
            receipt_handler: ReceiptHandler::new(),
            input: Default::default(),
            is_multiline_input: false,
            editing: None,
            select_channel: Default::default(),
            clipboard,
            event_tx,
            names_cache: Default::default(),
            mode_keybindings,
        };
        Ok((app, event_rx))
    }

    /// Resolve and cache all user names for the known user channels
    pub async fn populate_names_cache(&self) {
        let mut names_cache = BTreeMap::new();
        for user_id in self
            .storage
            .channels()
            .filter_map(|channel| channel.id.user())
        {
            if let Some(name) = self.resolve_name(user_id).await {
                names_cache.insert(user_id, name);
            }
        }
        let mut cache = self.names_cache.take().unwrap_or_default();
        cache.extend(names_cache);
        self.names_cache.replace(Some(cache));
    }

    pub fn get_input(&mut self) -> &mut Input {
        if self.select_channel.is_shown {
            &mut self.select_channel.input
        } else {
            &mut self.input
        }
    }

    pub fn writing_people(&self, channel: &Channel) -> Option<String> {
        if channel.is_writing() {
            let uuids: Box<dyn Iterator<Item = Uuid>> = match &channel.typing {
                TypingSet::GroupTyping(uuids) => Box::new(uuids.iter().copied()),
                TypingSet::SingleTyping(a) => {
                    if *a {
                        Box::new(std::iter::once(channel.user_id().unwrap()))
                    } else {
                        Box::new(std::iter::empty())
                    }
                }
            };
            Some(format!(
                "[{}] writing...",
                uuids.map(|id| self.name_by_id_cached(id)).format(", ")
            ))
        } else {
            None
        }
    }

    /// Returns the name of a user by their ID from the cache without resolving it.
    pub fn name_by_id_cached(&self, id: Uuid) -> String {
        if self.user_id == id {
            // it's me
            return self.config.user.display_name.clone();
        };

        let cache = self.names_cache.take().unwrap_or_default();
        let name = cache.get(&id).cloned().unwrap_or_else(|| id.to_string());
        self.names_cache.replace(Some(cache));
        name
    }

    /// Resolves name of a user by their id
    ///
    /// The resolution is done from the following places:
    ///
    /// 1. signal's profile name storage
    /// 2. signal's contacts storage
    /// 3. internal gurk's user name table
    async fn resolve_name(&self, user_id: Uuid) -> Option<String> {
        if let Some(name) = self.signal_manager.profile_name(user_id).await {
            debug!(name, "resolved name as profile name");
            return Some(name);
        }
        if let Some(contact) = self.signal_manager.contact(user_id).await {
            if !contact.name.trim().is_empty() {
                debug!(name = contact.name, "resolved name from contacts");
                return Some(contact.name);
            } else {
                debug!(%user_id, "resolved empty name from contacts, skipping");
            }
        }
        if let Some(name) = self
            .storage
            .name(user_id)
            .filter(|name| !name.trim().is_empty())
        {
            debug!(%name, "resolved name from storage");
            return Some(name.into_owned());
        }
        None
    }

    // Resolves name of a user by their id
    pub async fn name_by_id(&self, id: Uuid) -> String {
        if self.user_id == id {
            // it's me
            self.config.user.display_name.clone()
        } else {
            self.name_by_id_or_cache(id, |id| self.resolve_name(id))
                .await
        }
    }

    async fn name_by_id_or_cache<F>(&self, id: Uuid, on_miss: impl FnOnce(Uuid) -> F) -> String
    where
        F: Future<Output = Option<String>>,
    {
        let cache = self.names_cache.take().unwrap_or_default();
        let name = cache.get(&id).cloned();
        self.names_cache.replace(Some(cache));

        if let Some(name) = name {
            name
        } else if let Some(name) = on_miss(id).await {
            let mut cache = self.names_cache.take().unwrap_or_default();
            cache.insert(id, name.clone());
            self.names_cache.replace(Some(cache));
            name
        } else {
            id.to_string()
        }
    }

    pub fn channel_name<'a>(&self, channel: &'a Channel) -> Cow<'a, str> {
        if let Some(id) = channel.user_id() {
            self.name_by_id_cached(id).into()
        } else {
            (&channel.name).into()
        }
    }

    pub fn toggle_help(&mut self) {
        self.display_help = !self.display_help;
    }

    pub fn is_help(&self) -> bool {
        self.display_help
    }

    pub fn is_select_channel_shown(&self) -> bool {
        self.select_channel.is_shown
    }

    pub fn handle_event(&mut self, event: Event) -> anyhow::Result<()> {
        match event {
            Event::SentTextResult { message_id, result } => {
                if let Err(error) = result {
                    let mut message = self
                        .storage
                        .message(message_id)
                        .context("no message")?
                        .into_owned();
                    message.send_failed = Some(error.to_string());
                    self.storage.store_message(message_id.channel_id, message);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn is_editing(&self) -> bool {
        self.editing.is_some()
    }
}

#[derive(Debug, Default)]
pub(super) struct HandleReactionOptions {
    pub(super) remove: bool,
    pub(super) notify: bool,
    pub(super) bell: bool,
}

impl HandleReactionOptions {
    pub(super) fn new() -> Self {
        Default::default()
    }

    pub(super) fn remove(self, remove: bool) -> Self {
        Self { remove, ..self }
    }

    pub(super) fn notify(self, notify: bool) -> Self {
        Self { notify, ..self }
    }

    pub(super) fn bell(self, bell: bool) -> Self {
        Self { bell, ..self }
    }
}

/// Returns an emoji string if `s` is an emoji or if `s` is a GitHub emoji shortcode.
pub fn to_emoji(s: &str) -> Option<&str> {
    let s = s.trim();
    if emojis::get(s).is_some() {
        Some(s)
    } else {
        let s = s.strip_prefix(':')?.strip_suffix(':')?;
        Some(emojis::get_by_shortcode(s)?.as_str())
    }
}

pub(super) fn open_url(message: &Message, url_regex: &Regex) -> Option<()> {
    let text = message.message.as_ref()?;
    let m = url_regex.find(text)?;
    let url = m.as_str();
    let result = if let Some(path) = url.strip_prefix("file://") {
        opener::open(Path::new(path))
    } else {
        opener::open(url)
    };
    if let Err(error) = result {
        error!(url, %error, "failed to open");
    }
    Some(())
}

pub(super) fn open_file(message: &Message) -> Option<()> {
    let attachment = message.attachments.first()?;
    let file: &Path = attachment.filename.as_ref();
    if let Err(error) = opener::open(file) {
        let path = file.display().to_string();
        error!(path, %error, "failed to open");
    }
    Some(())
}

pub(super) fn notification_text_for_attachments(attachments: &[Attachment]) -> Option<String> {
    match attachments.len() {
        0 => None,
        1 => Some("<attachment>".into()),
        n => Some(format!("<attachments ({n})>")),
    }
}

pub(super) fn add_emoji_from_sticker(body: &mut Option<String>, sticker: Option<Sticker>) {
    if let Some(Sticker { emoji: Some(e), .. }) = sticker {
        *body = Some(format!("<{e}>"));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use chrono::{DateTime, FixedOffset};

    use arboard::ImageData;

    use crate::config::User;
    use crate::data::GroupData;
    use crate::signal::GroupMasterKeyBytes;
    use crate::signal::test::SignalManagerMock;
    use crate::storage::{ForgetfulStorage, MemCache};

    use super::*;

    pub(crate) fn test_app() -> (
        App,
        mpsc::UnboundedReceiver<Event>,
        Rc<RefCell<Vec<Message>>>,
    ) {
        let signal_manager = SignalManagerMock::new();
        let sent_messages = signal_manager.sent_messages.clone();

        let mut storage = MemCache::new(ForgetfulStorage);

        let channel_id = ChannelId::User(Uuid::new_v4());
        let channel = Channel {
            id: channel_id,
            name: "test".to_string(),
            group_data: Some(GroupData {
                master_key_bytes: GroupMasterKeyBytes::default(),
                members: vec![signal_manager.user_id()],
                revision: 1,
            }),
            unread_messages: 1,
            typing: TypingSet::GroupTyping(Default::default()),
        };
        storage.store_channel(channel);
        storage.store_message(
            channel_id,
            Message {
                from_id: signal_manager.user_id(),
                message: Some("First message".to_string()),
                arrived_at: 0,
                quote: Default::default(),
                attachments: Default::default(),
                reactions: Default::default(),
                receipt: Default::default(),
                body_ranges: Default::default(),
                send_failed: Default::default(),
                edit: Default::default(),
                edited: Default::default(),
            },
        );

        let user = User {
            display_name: "Tyler Durden".to_string(),
        };
        let (mut app, events) = App::try_new(
            Config::with_user(user),
            Box::new(signal_manager),
            Box::new(storage),
        )
        .unwrap();
        app.channels.state.select(Some(0));

        (app, events, sent_messages)
    }

    #[tokio::test]
    async fn test_send_input() {
        let (mut app, mut events, sent_messages) = test_app();
        let input = "Hello, World!";
        for c in input.chars() {
            app.get_input().put_char(c);
        }
        app.send_input(0);

        assert_eq!(sent_messages.borrow().len(), 1);
        let msg = sent_messages.borrow()[0].clone();
        assert_eq!(msg.message.as_ref().unwrap(), input);

        let channel_id = app.channels.items[0];
        let channel = app.storage.channel(channel_id).unwrap();
        assert_eq!(channel.unread_messages, 0);

        assert_eq!(app.get_input().data, "");

        match events.recv().await.unwrap() {
            Event::SentTextResult { message_id, result } => {
                assert_eq!(message_id.arrived_at, msg.arrived_at);
                assert!(result.is_ok());
            }
        }
    }

    #[tokio::test]
    async fn test_send_input_with_emoji() {
        let (mut app, mut events, sent_messages) = test_app();
        let input = "\u{1F47B}";
        for c in input.chars() {
            app.get_input().put_char(c);
        }

        app.send_input(0);

        assert_eq!(sent_messages.borrow().len(), 1);
        let msg = sent_messages.borrow()[0].clone();
        assert_eq!(msg.message.as_ref().unwrap(), input);

        assert_eq!(app.get_input().data, "");

        match events.recv().await.unwrap() {
            Event::SentTextResult { message_id, result } => {
                assert_eq!(message_id.arrived_at, msg.arrived_at);
                assert!(result.is_ok());
            }
        }
    }

    #[tokio::test]
    async fn test_send_input_with_emoji_codepoint() {
        let (mut app, mut events, sent_messages) = test_app();
        let input = ":thumbsup:";
        for c in input.chars() {
            app.get_input().put_char(c);
        }

        app.send_input(0);

        assert_eq!(sent_messages.borrow().len(), 1);
        let msg = sent_messages.borrow()[0].clone();
        assert_eq!(msg.message.as_ref().unwrap(), "\u{1F44D}");

        match events.recv().await.unwrap() {
            Event::SentTextResult { message_id, result } => {
                assert_eq!(message_id.arrived_at, msg.arrived_at);
                assert!(result.is_ok());
            }
        }
    }

    #[tokio::test]
    async fn test_add_reaction_with_emoji() {
        let (mut app, _events, _sent_messages) = test_app();

        let channel_id = app.channels.items[0];
        app.messages
            .get_mut(&channel_id)
            .unwrap()
            .state
            .select(Some(0));

        app.get_input().put_char('\u{1F44D}');
        app.add_reaction(0, None).await;

        let arrived_at = app.messages[&channel_id].items[0];
        let reactions = &app
            .storage
            .message(MessageId::new(channel_id, arrived_at))
            .unwrap()
            .reactions;
        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0], (app.user_id, "\u{1F44D}".to_string()));
    }

    #[tokio::test]
    async fn test_add_reaction_with_emoji_codepoint() {
        let (mut app, _events, _sent_messages) = test_app();

        let channel_id = app.channels.items[0];
        app.messages
            .get_mut(&channel_id)
            .unwrap()
            .state
            .select(Some(0));

        for c in ":thumbsup:".chars() {
            app.get_input().put_char(c);
        }
        app.add_reaction(0, None).await;

        let arrived_at = app.messages[&channel_id].items[0];
        let reactions = &app
            .storage
            .message(MessageId::new(channel_id, arrived_at))
            .unwrap()
            .reactions;
        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0], (app.user_id, "\u{1F44D}".to_string()));
    }

    #[tokio::test]
    async fn test_remove_reaction() {
        let (mut app, _events, _sent_messages) = test_app();

        let channel_id = app.channels.items[0];
        app.messages
            .get_mut(&channel_id)
            .unwrap()
            .state
            .select(Some(0));

        let arrived_at = app.messages[&channel_id].items[0];
        let mut message = app
            .storage
            .message(MessageId::new(channel_id, arrived_at))
            .unwrap()
            .into_owned();
        message
            .reactions
            .push((app.user_id, "\u{1F44D}".to_string()));
        app.storage.store_message(channel_id, message);
        app.add_reaction(0, None).await;

        let reactions = &app
            .storage
            .message(MessageId::new(channel_id, arrived_at))
            .unwrap()
            .reactions;
        assert!(reactions.is_empty());
    }

    #[tokio::test]
    async fn test_add_invalid_reaction() {
        let (mut app, _events, _sent_messages) = test_app();
        let channel_id = app.channels.items[0];
        app.messages
            .get_mut(&channel_id)
            .unwrap()
            .state
            .select(Some(0));

        for c in ":thumbsup".chars() {
            app.get_input().put_char(c);
        }
        app.add_reaction(0, None).await;

        assert_eq!(app.get_input().data, ":thumbsup");
        let arrived_at = app.messages[&channel_id].items[0];
        let reactions = &app
            .storage
            .message(MessageId::new(channel_id, arrived_at))
            .unwrap()
            .reactions;
        assert!(reactions.is_empty());
    }

    #[test]
    fn test_to_emoji() {
        assert_eq!(to_emoji("\u{1F680}"), Some("\u{1F680}"));
        assert_eq!(to_emoji("  \u{1F680}   "), Some("\u{1F680}")); // trimmed
        assert_eq!(to_emoji(":rocket:"), Some("\u{1F680}"));
        assert_eq!(to_emoji("\u{261D}\u{1F3FF}"), Some("\u{261D}\u{1F3FF}"));
        assert_eq!(to_emoji("a"), None);
    }

    #[test]
    fn test_extract_attachments() {
        let tempdir = tempfile::tempdir().unwrap();
        let image_png = tempdir.path().join("image.png");
        let image_jpg = tempdir.path().join("image.jpg");

        std::fs::write(&image_png, b"some png data").unwrap();
        std::fs::write(&image_jpg, b"some jpg data").unwrap();

        let clipboard_image = ImageData {
            width: 1,
            height: 1,
            bytes: vec![0, 0, 0, 0].into(), // RGBA single pixel
        };

        let message = format!(
            "Hello, file://{} file://{} World! file://clip",
            image_png.display(),
            image_jpg.display(),
        );

        let at_str = "2023-01-01T00:00:00+0200";
        let at: DateTime<FixedOffset> = at_str.parse().unwrap();

        let (cleaned_message, specs) =
            App::extract_attachments(&message, at, || Some(Ok(clipboard_image.clone())));
        assert_eq!(cleaned_message, "Hello, World!");
        dbg!(&specs);

        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].0.content_type, "image/png");
        assert_eq!(specs[0].0.file_name, Some("image.png".into()));
        assert_eq!(specs[1].0.content_type, "image/jpeg");
        assert_eq!(specs[1].0.file_name, Some("image.jpg".into()));
        assert_eq!(specs[2].0.content_type, "image/png");
        assert_eq!(
            specs[2].0.file_name,
            Some(format!("screenshot-{at_str}.png"))
        );
    }
}

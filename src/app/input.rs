use std::borrow::Cow;
use std::io::Cursor;
use std::path::Path;

use anyhow::Context as _;
use arboard::ImageData;
use chrono::{DateTime, Local, TimeZone};
use crokey::Combiner;
use crossterm::event::{KeyCode, KeyEvent};
use image::codecs::png::PngEncoder;
use image::{ImageBuffer, ImageEncoder, Rgba};
use presage::libsignal_service::sender::AttachmentSpec;
use tracing::{error, info};

use crate::command::{
    Command, DirectionVertical, MoveAmountText, MoveAmountVisual, MoveDirection, Widget, WindowMode,
};
use crate::data::Message;
use crate::storage::MessageId;
use crate::util::{ATTACHMENT_REGEX, URL_REGEX};

use super::{App, HandleReactionOptions, open_file, open_url, to_emoji};

impl App {
    pub async fn on_command(&mut self, command: Command) -> anyhow::Result<()> {
        match command {
            Command::Help => self.toggle_help(),
            Command::MoveText(MoveDirection::Previous, MoveAmountText::Word) => {
                self.get_input().move_back_word()
            }
            Command::MoveText(MoveDirection::Previous, MoveAmountText::Character) => {
                self.get_input().on_left()
            }
            Command::MoveText(MoveDirection::Previous, MoveAmountText::Line) => {
                self.get_input().move_line_up()
            }
            Command::MoveText(MoveDirection::Next, MoveAmountText::Word) => {
                self.get_input().move_forward_word()
            }
            Command::MoveText(MoveDirection::Next, MoveAmountText::Character) => {
                self.get_input().on_right()
            }
            Command::MoveText(MoveDirection::Next, MoveAmountText::Line) => {
                self.get_input().move_line_down()
            }
            Command::SelectMessage(MoveDirection::Previous, MoveAmountVisual::Entry) => {
                self.on_pgup()
            }
            Command::SelectMessage(MoveDirection::Next, MoveAmountVisual::Entry) => self.on_pgdn(),
            Command::KillBackwardLine => self.get_input().on_delete_line(),
            Command::KillWord => self.get_input().on_delete_word(),
            Command::CopyMessage(_) => self.copy_selection(),
            Command::KillLine => self.get_input().on_delete_suffix(),
            Command::SelectChannel(MoveDirection::Previous) => self.select_previous_channel(),
            Command::SelectChannel(MoveDirection::Next) => self.select_next_channel(),
            Command::SelectChannelModal(MoveDirection::Previous) => self.select_channel_prev(),
            Command::SelectChannelModal(MoveDirection::Next) => self.select_channel_next(),
            Command::KillWholeLine => self.get_input().on_delete_line(),
            Command::BeginningOfLine => self.get_input().on_home(),
            Command::EndOfLine => self.get_input().on_end(),
            Command::EditMessage => {
                self.start_editing();
            }
            // Command::ReplyMessage => unimplemented!("{command:?}"),
            // Command::DeleteMessage => unimplemented!("{command:?}"),
            Command::ToggleChannelModal => {
                if !self.select_channel.is_shown {
                    self.select_channel.reset(&*self.storage);
                }
                self.select_channel.is_shown = !self.select_channel.is_shown;
            }
            Command::ToggleMultiline => {
                self.is_multiline_input = !self.is_multiline_input;
            }
            Command::React(reaction) => {
                if let Some(idx) = self.channels.state.selected() {
                    self.add_reaction(idx, reaction).await;
                }
            }
            Command::OpenUrl => {
                self.try_open_url();
            }
            Command::OpenFile => {
                self.try_open_file();
            }
            Command::DeleteCharacter(MoveDirection::Previous) => {
                self.get_input().on_backspace();
            }
            Command::DeleteCharacter(MoveDirection::Next) => {} // unimplemented!("{command:?}")},
            Command::Quit => {
                self.should_quit = true;
            }
            Command::Scroll(Widget::Help, DirectionVertical::Up, MoveAmountVisual::Entry) => {
                if self.help_scroll.0 >= 1 {
                    self.help_scroll.0 -= 1
                }
            }
            Command::Scroll(Widget::Help, DirectionVertical::Down, MoveAmountVisual::Entry) => {
                // TODO: prevent overscrolling
                self.help_scroll.0 += 1
            }
            Command::NoOp => {}
        }
        Ok(())
    }

    pub async fn on_key(&mut self, key: KeyEvent) -> anyhow::Result<()> {
        if let Some(cmd) = self.event_to_command(&key) {
            self.on_command(cmd.clone()).await?;
        } else {
            match key.code {
                KeyCode::Char('\r') => self.get_input().put_char('\n'),
                KeyCode::Enter => {
                    if !self.select_channel.is_shown {
                        if self.is_multiline_input {
                            self.get_input().new_line();
                        } else if !self.input.data.is_empty() {
                            if let Some(idx) = self.channels.state.selected() {
                                self.send_input(idx);
                            }
                        } else {
                            // input is empty
                            self.try_open_url_or_file();
                        }
                    } else if self.select_channel.is_shown
                        && let Some(channel_id) = self.select_channel.selected_channel_id().copied()
                    {
                        self.select_channel.is_shown = false;
                        let (idx, _) = self
                            .channels
                            .items
                            .iter()
                            .enumerate()
                            .find(|(_, id)| **id == channel_id)
                            .context("channel disappeared during channel select popup")?;
                        self.channels.state.select(Some(idx));
                    }
                }
                KeyCode::Esc => {
                    if !self.reset_editing() {
                        self.reset_message_selection();
                    }
                }
                KeyCode::Char(c) => self.get_input().put_char(c),
                _ => {}
            }
        }
        Ok(())
    }

    fn try_open_url_or_file(&mut self) -> Option<()> {
        self.try_open_url().or_else(|| self.try_open_file())
    }

    /// Tries to open the first url in the selected message.
    ///
    /// Does nothing if no message is selected and no url is contained in the message.
    fn try_open_url(&mut self) -> Option<()> {
        let message = self.selected_message()?;
        open_url(&message, &URL_REGEX)?;
        self.reset_message_selection();
        Some(())
    }

    /// Tries to open the first file attachment in the selected message.
    ///
    /// Does nothing if no message is selected and the message contains no attachments.
    fn try_open_file(&mut self) -> Option<()> {
        let message = self.selected_message()?;
        open_file(&message)?;
        self.reset_message_selection();
        Some(())
    }

    fn selected_message_id(&self) -> Option<MessageId> {
        let channel_id = self.channels.selected_item()?;
        let messages = self.messages.get(channel_id)?;
        let message_idx = messages.state.selected()?;
        let arrived_at = messages.items[messages
            .items
            .len()
            .checked_sub(message_idx)?
            .checked_sub(1)?];
        Some(MessageId::new(*channel_id, arrived_at))
    }

    pub(super) fn selected_message(&self) -> Option<Cow<'_, Message>> {
        let message_id = self.selected_message_id()?;
        let message = self.storage.message(message_id);
        info!("selected message: {message:?}");
        message
    }

    /// Returns Some(_) reaction if input is a reaction.
    ///
    /// Inner is None, if the reaction should be removed.
    fn take_reaction(&mut self) -> Option<Option<String>> {
        let input_box = self.get_input();
        if input_box.data.is_empty() {
            Some(None)
        } else {
            let emoji = to_emoji(&input_box.data)?.to_string();
            self.take_input();
            Some(Some(emoji))
        }
    }

    pub async fn add_reaction(
        &mut self,
        channel_idx: usize,
        reaction: Option<String>,
    ) -> Option<()> {
        let reaction = reaction.or_else(|| self.take_reaction()?);
        let channel = self.storage.channel(self.channels.items[channel_idx])?;
        let message = self.selected_message()?;
        let remove = reaction.is_none();
        let emoji = reaction.or_else(|| {
            // find emoji which should be removed
            // if no emoji found => there is no reaction from us => nothing to remove
            message.reactions.iter().find_map(|(id, emoji)| {
                if id == &self.signal_manager.user_id() {
                    Some(emoji.clone())
                } else {
                    None
                }
            })
        })?;

        self.signal_manager
            .send_reaction(&channel, &message, emoji.clone(), remove);

        let channel_id = channel.id;
        let arrived_at = message.arrived_at;
        self.handle_reaction(
            channel_id,
            arrived_at,
            self.signal_manager.user_id(),
            emoji,
            HandleReactionOptions::new().remove(remove),
        )
        .await;

        self.reset_unread_messages();
        self.bubble_up_channel(channel_idx);
        self.reset_message_selection();

        Some(())
    }

    fn take_input(&mut self) -> String {
        self.get_input().take()
    }

    pub(super) fn send_input(&mut self, channel_idx: usize) {
        let input = self.take_input();
        let (input, attachments) = Self::extract_attachments(&input, Local::now(), || {
            self.clipboard.as_mut().map(|c| c.get_image())
        });
        let channel_id = self.channels.items[channel_idx];
        let channel = self
            .storage
            .channel(channel_id)
            .expect("non-existent channel");
        let editing = self.editing.take();
        let quote = editing.is_none().then(|| self.selected_message()).flatten();
        let (sent_message, response) = self.signal_manager.send_text(
            &channel,
            input,
            quote.as_deref(),
            editing.map(|id| id.arrived_at),
            attachments,
        );

        let message_id = MessageId::new(channel_id, sent_message.arrived_at);
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            if let Ok(result) = response.await {
                tx.send(crate::event::Event::SentTextResult { message_id, result })
                    .expect("event sender gone");
            } else {
                error!(?message_id, "response for sending message was lost");
            }
        });

        if let Some(id) = editing {
            self.storage
                .store_edited_message(channel_id, id.arrived_at, sent_message);
        } else {
            let sent_message = self.storage.store_message(channel_id, sent_message);
            self.messages
                .get_mut(&channel_id)
                .expect("non-existent channel")
                .items
                .push(sent_message.arrived_at);
        };

        self.reset_message_selection();
        self.reset_unread_messages();
        self.bubble_up_channel(channel_idx);
    }

    pub fn copy_selection(&mut self) {
        if let Some(message) = self.selected_message()
            && let Some(text) = message.message.as_ref()
        {
            let text = text.clone();
            if let Some(clipboard) = self.clipboard.as_mut() {
                if let Err(error) = clipboard.set_text(text) {
                    error!(%error, "failed to copy text to clipboard");
                } else {
                    info!("copied selected text to clipboard");
                }
            }
        }
    }

    /// Returns `true` if editing was reset, otherwise `false`
    fn reset_editing(&mut self) -> bool {
        let is_reset = self.editing.take().is_some();
        if is_reset {
            self.take_input();
        }
        is_reset
    }

    fn start_editing(&mut self) -> Option<()> {
        if !self.input.is_empty() {
            return None;
        }

        let message_id = self.selected_message_id()?;
        let message = self.storage.message(message_id)?;
        if message.from_id != self.user_id {
            return None;
        }

        let target_sent_timestamp = self
            .storage
            .edits(message_id)
            .last()
            .map(|last_edit| last_edit.arrived_at)
            .unwrap_or(message.arrived_at);
        let message_id = MessageId::new(message_id.channel_id, target_sent_timestamp);
        let text = message.message.clone()?;

        self.editing.replace(message_id);
        self.input.data = text;
        self.input.on_end();

        Some(())
    }

    pub fn event_to_command<'r>(&'r self, event: &KeyEvent) -> Option<&'r Command> {
        let mut combiner = Combiner::default();
        let keys_pressed = combiner.transform(*event)?;
        let modes = if self.is_help() {
            vec![WindowMode::Anywhere, WindowMode::Help]
        } else if self.is_select_channel_shown() {
            vec![WindowMode::Anywhere, WindowMode::ChannelModal]
        } else if self.is_multiline_input {
            vec![
                WindowMode::Anywhere,
                WindowMode::Multiline,
                WindowMode::Normal,
            ]
        } else if self.input.is_empty() {
            vec![
                WindowMode::Anywhere,
                WindowMode::MessageSelected,
                WindowMode::Normal,
            ]
        } else {
            vec![WindowMode::Anywhere, WindowMode::Normal]
        };
        for mode in modes {
            if let Some(kb) = self.mode_keybindings.get(&mode)
                && let Some(cmd) = kb.get(&keys_pressed)
            {
                return Some(cmd);
            }
        }
        if self.is_help() {
            // Swallow event
            Some(&Command::NoOp)
        } else {
            None
        }
    }

    pub(super) fn extract_attachments<Tz: TimeZone>(
        input: &str,
        at: DateTime<Tz>,
        mut get_clipboard_img: impl FnMut() -> Option<Result<ImageData<'static>, arboard::Error>>,
    ) -> (String, Vec<(AttachmentSpec, Vec<u8>)>)
    where
        Tz::Offset: std::fmt::Display,
    {
        let mut offset = 0;
        let mut clean_input = String::new();

        let attachments = ATTACHMENT_REGEX.find_iter(input).filter_map(|m| {
            let path_str = m.as_str().strip_prefix("file://")?;

            clean_input.push_str(input[offset..m.start()].trim_end());
            offset = m.end();

            Some(if path_str.starts_with("clip") {
                // clipboard
                let img = get_clipboard_img()?
                    .inspect_err(|error| error!(%error, "failed to get clipboard image"))
                    .ok()?;

                let width: u32 = img.width.try_into().ok()?;
                let height: u32 = img.height.try_into().ok()?;

                let mut bytes = Vec::new();
                let mut cursor = Cursor::new(&mut bytes);
                let encoder = PngEncoder::new(&mut cursor);

                let png: ImageBuffer<Rgba<_>, _> = ImageBuffer::from_raw(width, height, img.bytes)?;
                let data: Vec<_> = png.into_raw().iter().map(|b| b.swap_bytes()).collect();
                encoder
                    .write_image(
                        &data,
                        img.width as _,
                        img.height as _,
                        image::ExtendedColorType::Rgba8,
                    )
                    .inspect_err(|error| error!(%error, "failed to encode image"))
                    .ok()?;

                let file_name = format!("screenshot-{}.png", at.format("%Y-%m-%dT%H:%M:%S%z"));

                let spec = AttachmentSpec {
                    content_type: "image/png".to_owned(),
                    length: bytes.len(),
                    file_name: Some(file_name),
                    width: Some(width),
                    height: Some(height),
                    ..Default::default()
                };
                (spec, bytes)
            } else {
                // path

                // TODO: Show error to user if the file does not exist. This would prevent not
                // sending the attachment in the end.

                let path = Path::new(path_str);
                let bytes = std::fs::read(path).ok()?;
                let content_type = mime_guess::from_path(path)
                    .first()
                    .map(|mime| mime.essence_str().to_string())
                    .unwrap_or_default();
                let file_name = path.file_name().map(|f| f.to_string_lossy().into());
                let spec = AttachmentSpec {
                    content_type,
                    length: bytes.len(),
                    file_name,
                    ..Default::default()
                };

                (spec, bytes)
            })
        });

        let attachments = attachments.collect();
        clean_input.push_str(&input[offset..]);
        let clean_input = clean_input.trim().to_string();

        (clean_input, attachments)
    }
}

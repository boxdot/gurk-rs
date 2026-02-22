//! Draw the UI

use std::fmt;

use chrono::Datelike;
use itertools::Itertools;
use ratatui::Frame;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListDirection, ListItem, Paragraph};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    widgets::Padding,
};
use ratatui::{
    style::{Color, Modifier, Style},
    widgets::Wrap,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use uuid::Uuid;

use crate::app::App;
use crate::channels::SelectChannel;
use crate::command::{Command, WindowMode};
use crate::cursor::Cursor;
use crate::data::{AssociatedValue, Message};
use crate::receipt::{Receipt, ReceiptEvent};
use crate::storage::MessageId;
use crate::util::utc_timestamp_msec_to_local;

use super::CHANNEL_VIEW_RATIO;
use super::name_resolver::NameResolver;

/// The main function drawing the UI for each frame
pub fn draw(f: &mut Frame, app: &mut App) {
    if app.is_help() {
        // Display shortcut panel
        let chunks = Layout::default()
            .constraints([
                Constraint::Percentage(5),
                Constraint::Percentage(90),
                Constraint::Percentage(5),
            ])
            .direction(Direction::Horizontal)
            .split(f.area());
        draw_help(f, app, chunks[1]);
        return;
    }
    let chunks = Layout::default()
        .constraints(
            [
                Constraint::Ratio(1, CHANNEL_VIEW_RATIO),
                Constraint::Ratio(3, CHANNEL_VIEW_RATIO),
            ]
            .as_ref(),
        )
        .direction(Direction::Horizontal)
        .split(f.area());

    draw_channels(f, app, chunks[0]);
    draw_chat(f, app, chunks[1]);

    if app.select_channel.is_shown {
        draw_select_channel_popup(f, &mut app.select_channel);
    }
}

fn draw_select_channel_popup(f: &mut Frame, select_channel: &mut SelectChannel) {
    let area = centered_rect(60, 60, f.area());
    let chunks = Layout::default()
        .constraints([Constraint::Length(1 + 2), Constraint::Min(0)].as_ref())
        .direction(Direction::Vertical)
        .split(area);
    f.render_widget(Clear, area);
    let input = Paragraph::new(Text::from(select_channel.input.data.clone())).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Select channel"),
    );
    f.render_widget(input, chunks[0]);
    let cursor = &select_channel.input.cursor;
    f.set_cursor_position((
        chunks[0].x + cursor.col as u16 + 1,
        chunks[0].y + cursor.line as u16 + 1,
    ));
    let items: Vec<_> = select_channel.filtered_names().map(ListItem::new).collect();
    match select_channel.state.selected() {
        Some(idx) if items.len() <= idx => {
            select_channel.state.select(items.len().checked_sub(1));
        }
        None if !items.is_empty() => {
            select_channel.state.select(Some(0));
        }
        _ => (),
    }
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Gray));
    f.render_stateful_widget(list, chunks[1], &mut select_channel.state);
}

fn draw_channels(f: &mut Frame, app: &mut App, area: Rect) {
    let channel_list_width = area.width.saturating_sub(2) as usize;
    let channels = app
        .channels
        .items
        .iter()
        .filter_map(|&channel_id| app.storage.channel(channel_id))
        .map(|channel| {
            let unread_messages_label = if channel.unread_messages != 0 {
                format!(" ({})", channel.unread_messages)
            } else {
                String::new()
            };
            let label = format!("{}{}", app.channel_name(&channel), unread_messages_label);
            let label_width = label.width();
            let label = if label.width() <= channel_list_width || unread_messages_label.is_empty() {
                label
            } else {
                let diff = label_width - channel_list_width;
                let mut end = channel.name.width().saturating_sub(diff);
                while !channel.name.is_char_boundary(end) {
                    end += 1;
                }
                format!("{}{}", &channel.name[0..end], unread_messages_label)
            };
            ListItem::new(vec![Line::from(Span::raw(label))])
        });

    let channels = List::new(channels)
        .block(Block::default().borders(Borders::ALL).title("Channels"))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Gray));
    let no_channels = channels.is_empty();
    f.render_stateful_widget(channels, area, &mut app.channels.state);

    if no_channels {
        f.render_widget(
            Paragraph::new("No channels\n(Channels will be added on incoming messages)")
                .wrap(Wrap { trim: true })
                .centered()
                .block(Block::default().padding(Padding::top(area.height / 2))),
            area,
        );
    };
}

fn wrap(text: &str, mut cursor: Cursor, width: usize) -> (String, Cursor, usize) {
    let mut res = String::new();

    let mut line = 0;
    let mut col = 0;

    for c in text.chars() {
        // current line too long => wrap
        if col > 0 && col % width == 0 {
            res.push('\n');

            // adjust cursor
            if line < cursor.line {
                cursor.line += 1;
                cursor.idx += 1;
            } else if line == cursor.line && col <= cursor.col {
                cursor.line += 1;
                cursor.col -= col;
                cursor.idx += 1;
            }

            line += 1;
            col = 0;
        }

        if c == '\n' {
            line += 1;
            col = 0;
        } else {
            col += c.width().unwrap_or(0);
        }
        res.push(c);
    }

    // special case: cursor is at the end of the text and overflows `width`
    if cursor.idx == res.len() && cursor.col == width {
        res.push('\n');
        cursor.line += 1;
        cursor.col = 0;
        cursor.idx += 1;
        line += 1;
    }

    (res, cursor, line + 1)
}

fn draw_chat(f: &mut Frame, app: &mut App, area: Rect) {
    let text_width = area.width.saturating_sub(2) as usize;
    let (wrapped_input, cursor, num_input_lines) =
        wrap(&app.input.data, app.input.cursor.clone(), text_width);

    let chunks = Layout::default()
        .constraints(
            [
                Constraint::Min(0),
                Constraint::Length(num_input_lines as u16 + 2),
            ]
            .as_ref(),
        )
        .direction(Direction::Vertical)
        .split(area);

    draw_messages(f, app, chunks[0]);

    let title = match (app.is_editing(), app.is_multiline_input) {
        (true, true) => "Input (Editing, Multiline)",
        (true, false) => "Input (Editing)",
        (false, true) => "Input (Multiline)",
        (false, false) => "Input",
    };

    let input = Paragraph::new(Text::from(wrapped_input))
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(input, chunks[1]);
    if !app.select_channel.is_shown {
        f.set_cursor_position((
            chunks[1].x + cursor.col as u16 + 1,  // +1 for frame
            chunks[1].y + cursor.line as u16 + 1, // +1 for frame
        ));
    }
}

fn prepare_receipts(app: &mut App, height: usize) {
    let user_id = app.user_id;
    let channel_id = match app.channels.selected_item() {
        Some(channel_id) => *channel_id,
        None => return,
    };
    let messages = match app.messages.get(&channel_id) {
        Some(messages) => messages,
        None => return,
    };
    if messages.items.is_empty() {
        return;
    }

    let offset = if let Some(selected) = messages.state.selected() {
        messages
            .rendered
            .offset
            .clamp(selected.saturating_sub(height), selected)
    } else {
        messages.rendered.offset
    };

    let read_messages: Vec<Message> = app
        .storage
        .messages(channel_id)
        .rev()
        .skip(offset)
        .filter_map(|message| {
            if let Receipt::Delivered = message.receipt
                && message.from_id != user_id
            {
                let mut message = message.into_owned();
                message.receipt = Receipt::Read;
                return Some(message);
            }
            None
        })
        .collect();

    for message in read_messages {
        let from_id = message.from_id;
        let arrived_at = message.arrived_at;
        app.storage.store_message(channel_id, message);
        app.add_receipt_event(ReceiptEvent::new(from_id, arrived_at, Receipt::Read));
    }
}

fn draw_messages(f: &mut Frame, app: &mut App, area: Rect) {
    // area without borders
    let height = area.height.saturating_sub(2) as usize;
    if height == 0 {
        return;
    }
    let width = area.width.saturating_sub(2) as usize;

    prepare_receipts(app, height);

    let Some(&channel_id) = app.channels.selected_item() else {
        f.render_widget(
            Paragraph::new("No Channel selected")
                .block(
                    Block::bordered()
                        .title("Messages")
                        .padding(Padding::top(area.height / 2)),
                )
                .centered(),
            area,
        );
        return;
    };

    let channel = app
        .storage
        .channel(channel_id)
        .expect("non-existent channel");

    let writing_people = app.writing_people(&channel);

    // Calculate the offset in messages we start rendering with.
    // `offset` includes the selected message (if any), and is at most height-many messages to
    // the selected message, since we can't render more than height-many of them.
    let messages = &app.messages[&channel_id];
    let offset = if let Some(selected) = messages.state.selected() {
        messages
            .rendered
            .offset
            .clamp(selected.saturating_sub(height), selected)
    } else {
        messages.rendered.offset
    };
    let messages_to_render = messages
        .items
        .iter()
        .rev()
        .skip(offset)
        .take(height)
        .copied();

    let names = NameResolver::compute(
        app,
        messages_to_render
            .clone()
            .map(|arrived_at| MessageId::new(channel_id, arrived_at)),
    );

    // message display options
    const TIME_WIDTH: usize = 6; // width of "00:00 "
    let mut prefix_width = TIME_WIDTH;
    if app.config.show_receipts {
        prefix_width += RECEIPT_WIDTH;
    }
    let prefix = " ".repeat(prefix_width);

    // The day of the message at the bottom of the viewport
    let first_msg_timestamp = messages_to_render.clone().next().unwrap_or_default();
    let mut previous_msg_timestamp = first_msg_timestamp;
    let mut previous_msg_day = utc_timestamp_msec_to_local(first_msg_timestamp).num_days_from_ce();

    let messages_from_offset = messages_to_render
        .enumerate()
        .flat_map(|(idx, arrived_at)| {
            let msg = app
                .storage
                .message(MessageId::new(channel_id, arrived_at))?;
            let date_division = display_date_line(
                msg.arrived_at,
                previous_msg_timestamp,
                &mut previous_msg_day,
                width,
            );

            let unread_messages = channel.unread_messages as usize;
            let new_messages_division =
                (unread_messages > 0 && unread_messages == idx + 1).then(|| {
                    "-".repeat(prefix_width)
                        + "new messages"
                        + &"-".repeat(width.saturating_sub(prefix_width))
                });

            previous_msg_timestamp = msg.arrived_at;
            let show_receipt = ShowReceipt::from_msg(&msg, app.user_id, app.config.show_receipts);
            display_message(
                &names,
                &msg,
                &prefix,
                width,
                height,
                show_receipt,
                date_division,
                new_messages_division,
                app.config.colored_messages,
            )
        });

    // counters to accumulate messages as long they fit into the list height,
    // or up to the selected message
    let mut items_height = 0;
    let selected = messages.state.selected().unwrap_or(0);

    let mut items: Vec<ListItem<'static>> = messages_from_offset
        .enumerate()
        .take_while(|(idx, item)| {
            items_height += item.height();
            items_height <= height || offset + *idx <= selected
        })
        .map(|(_, item)| item)
        .collect();

    // calculate the new offset by counting the messages down:
    // we known that we either stopped at the last fitting message or at the selected message
    let mut items_height = height;
    let mut first_idx = 0;
    for (idx, item) in items.iter().enumerate().rev() {
        if item.height() <= items_height {
            items_height -= item.height();
            first_idx = idx;
        } else {
            break;
        }
    }
    let offset = offset + first_idx;
    items = items.split_off(first_idx);

    let title: String = if let Some(writing_people) = writing_people {
        format!("Messages {writing_people}")
    } else {
        "Messages".to_string()
    };

    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Gray))
        .direction(ListDirection::BottomToTop);

    // re-borrow channel messages mutably
    let messages = app
        .messages
        .get_mut(&channel_id)
        .expect("non-existent channel");

    // update selected state to point within `items`
    let state = &mut messages.state;
    let selected_global = state.selected();
    if let Some(selected) = selected_global {
        state.select(Some(selected - offset));
    }

    f.render_stateful_widget(list, area, state);

    // restore selected state and update offset
    state.select(selected_global);
    messages.rendered.offset = offset;
}

fn display_time(timestamp: u64) -> String {
    utc_timestamp_msec_to_local(timestamp)
        .format("%R ")
        .to_string()
}

const RECEIPT_WIDTH: usize = 2;

/// Ternary state whether to show receipt for a message
enum ShowReceipt {
    // show receipt for this message
    Yes,
    // don't show receipt for this message
    No,
    // don't show receipt for any message
    Never,
}

impl ShowReceipt {
    fn from_msg(msg: &Message, user_id: Uuid, config_show_receipts: bool) -> Self {
        if config_show_receipts {
            if user_id == msg.from_id {
                Self::Yes
            } else {
                Self::No
            }
        } else {
            Self::Never
        }
    }
}

fn display_receipt(receipt: Receipt, show: ShowReceipt) -> &'static str {
    use ShowReceipt::*;
    match (show, receipt) {
        (Yes, Receipt::Nothing) => "  ",
        (Yes, Receipt::Sent) => "○ ",
        (Yes, Receipt::Delivered) => "◉ ",
        (Yes, Receipt::Read) => "● ",
        (No, _) => "  ",
        (Never, _) => "",
    }
}

#[allow(clippy::too_many_arguments)]
fn display_message(
    names: &NameResolver,
    msg: &Message,
    prefix: &str,
    width: usize,
    height: usize,
    show_receipt: ShowReceipt,
    date_division: Option<String>,
    unread_messages_division: Option<String>,
    colored_messages: bool,
) -> Option<ListItem<'static>> {
    let receipt = Span::styled(
        display_receipt(msg.receipt, show_receipt),
        Style::default().fg(Color::Yellow),
    );

    let time = Span::styled(
        display_time(msg.arrived_at),
        Style::default().fg(Color::Yellow),
    );

    let (from, from_color) = names.resolve(msg.from_id);

    let from = Span::styled(from.into_owned(), Style::default().fg(from_color));
    let delimiter = Span::from(": ");

    let wrap_opts = textwrap::Options::new(width)
        .initial_indent(prefix)
        .subsequent_indent(prefix);

    // collect message text
    let text = strip_ansi_escapes::strip_str(msg.message.as_deref().unwrap_or_default());
    let mut text = replace_mentions(msg, names, text);
    add_attachments(msg, &mut text);
    if text.is_empty() {
        return None; // no text => nothing to render
    }
    add_reactions(msg, &mut text);
    add_edited(msg, &mut text);

    let mut spans: Vec<Line> = vec![];
    if let Some(date_division) = date_division {
        spans.push(Line::from(date_division));
    }
    if let Some(unread_messages_division) = unread_messages_division {
        spans.push(Line::from(unread_messages_division));
    }

    // prepend quote if any
    let quote_text = msg
        .quote
        .as_ref()
        .and_then(|quote| displayed_quote(names, quote));
    if let Some(quote_text) = quote_text.as_ref() {
        let quote_prefix = format!("{prefix}> ");
        let quote_wrap_opts = textwrap::Options::new(width.saturating_sub(2))
            .initial_indent(&quote_prefix)
            .subsequent_indent(&quote_prefix);
        let quote_style = Style::default().fg(Color::Rgb(150, 150, 150));
        spans.extend(
            textwrap::wrap(quote_text, quote_wrap_opts)
                .into_iter()
                .enumerate()
                .map(|(idx, line)| {
                    let res = if idx == 0 {
                        vec![
                            receipt.clone(),
                            time.clone(),
                            from.clone(),
                            delimiter.clone(),
                            Span::styled(
                                line.strip_prefix(prefix).unwrap().to_owned(),
                                quote_style,
                            ),
                        ]
                    } else {
                        vec![Span::styled(line.into_owned(), quote_style)]
                    };
                    Line::from(res)
                }),
        );
    }

    let add_time = quote_text.is_none();
    let message_style = if colored_messages {
        Style::default().fg(from_color)
    } else {
        Style::default()
    };
    spans.extend(
        textwrap::wrap(&text, &wrap_opts)
            .into_iter()
            .enumerate()
            .map(|(idx, line)| {
                let res = if add_time && idx == 0 {
                    vec![
                        receipt.clone(),
                        time.clone(),
                        from.clone(),
                        delimiter.clone(),
                        Span::styled(line.strip_prefix(prefix).unwrap().to_owned(), message_style),
                    ]
                } else {
                    vec![Span::styled(line.into_owned(), message_style)]
                };
                Line::from(res)
            }),
    );

    if let Some(reason) = msg.send_failed.as_deref() {
        let error = format!("[Could not send: {reason}]");
        let error_style = Style::default().fg(Color::Red);
        spans.extend(
            textwrap::wrap(&error, &wrap_opts)
                .into_iter()
                .map(|line| Span::styled(line.into_owned(), error_style).into()),
        );
    }

    if spans.len() > height {
        // span is too big to be shown fully
        spans.resize(height - 1, Line::from(""));
        spans.push(Line::from(format!("{prefix}[...]")));
    }
    Some(ListItem::new(Text::from(spans)))
}

fn replace_mentions(msg: &Message, names: &NameResolver, text: String) -> String {
    if msg.body_ranges.is_empty() {
        return text;
    }

    let ac = aho_corasick::AhoCorasickBuilder::new()
        .build(std::iter::repeat_n("￼", msg.body_ranges.len())) // TODO: cache
        .expect("failed to build obj replacer");
    let mut buf = String::with_capacity(text.len());
    let mut ranges = msg.body_ranges.iter();
    ac.replace_all_with(&text, &mut buf, |_, _, dst| {
        // TODO: check ranges?
        for range in &mut ranges {
            let (name, _color) = match range.value {
                AssociatedValue::MentionUuid(id) => names.resolve(id),
                AssociatedValue::Style(_) => continue, // not supported yet
            };
            dst.push('@');
            dst.push_str(&name);
            return true;
        }
        false
    });

    buf
}

fn display_date_line(
    msg_timestamp: u64,
    previous_msg_timestamp: u64,
    previous_msg_day: &mut i32,
    width: usize,
) -> Option<String> {
    let local_time = utc_timestamp_msec_to_local(msg_timestamp);
    let current_msg_day = local_time.num_days_from_ce();

    if current_msg_day != *previous_msg_day {
        // Show the date of the previous section (the day we're leaving)
        let previous_local_time = utc_timestamp_msec_to_local(previous_msg_timestamp);
        let date = format!("{:=^width$}", previous_local_time.format(" %A, %x "));
        *previous_msg_day = current_msg_day;
        Some(date)
    } else {
        None
    }
}

fn add_attachments(msg: &Message, out: &mut String) {
    if !msg.attachments.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }

        fmt::write(
            out,
            format_args!(
                "{}",
                msg.attachments
                    .iter()
                    .format_with("\n", |attachment, f| f(&format_args!(
                        "<file://{}>",
                        attachment.filename.display()
                    )))
            ),
        )
        .expect("formatting attachments failed");
    }
}

fn add_reactions(msg: &Message, out: &mut dyn fmt::Write) {
    if !msg.reactions.is_empty() {
        fmt::write(
            out,
            format_args!(
                " [{}]",
                msg.reactions.iter().map(|(_, emoji)| emoji).format("")
            ),
        )
        .expect("formatting reactions failed");
    }
}

fn add_edited(msg: &Message, out: &mut dyn fmt::Write) {
    if msg.edited {
        write!(out, " [edited]").expect("formatting edited failed")
    }
}

fn help_commands<'a>() -> Vec<Line<'a>> {
    let commands = <Command as strum::IntoEnumIterator>::iter()
        .map(|cmd| {
            (
                strum::EnumProperty::get_str(&cmd, "usage")
                    .unwrap_or(&cmd.to_string())
                    .to_string(),
                strum::EnumProperty::get_str(&cmd, "desc")
                    .unwrap_or("Undocumented")
                    .to_string(),
            )
        })
        .collect_vec();
    let usage_len = commands.iter().map(|inf| inf.0.len()).max().unwrap_or(0);
    let commands = commands
        .iter()
        .map(|inf| Line::raw(format!("{: <usage_len$}   {}", inf.0, inf.1)));
    let mut v = vec![
        Line::styled("Commands", Style::default().add_modifier(Modifier::BOLD)),
        Line::default(),
    ];
    v.extend(commands);
    v
}

fn bindings(app: &App) -> Vec<Line<'_>> {
    [
        WindowMode::Normal,
        WindowMode::Anywhere,
        WindowMode::Help,
        WindowMode::ChannelModal,
        WindowMode::Multiline,
        WindowMode::MessageSelected,
    ]
    .iter()
    .map(|mode| bindings_mode(app, mode))
    .concat()
}

fn bindings_mode<'a>(app: &App, mode: &WindowMode) -> Vec<Line<'a>> {
    let bindings = if let Some(kb) = app.mode_keybindings.get(mode) {
        kb.iter()
            .map(|(kc, cmd)| {
                (
                    kc.to_string(),
                    cmd.to_string(),
                    strum::EnumProperty::get_str(cmd, "desc")
                        .unwrap_or("Undocumented")
                        .to_string(),
                )
            })
            .sorted()
            .collect_vec()
    } else {
        Vec::default()
    };
    let kc_len = bindings.iter().map(|inf| inf.0.len()).max().unwrap_or(0);
    let cmd_len = bindings.iter().map(|inf| inf.1.len()).max().unwrap_or(0);
    let bindings = bindings.iter().map(|inf| {
        Line::raw(format!(
            "{: <kc_len$}  {: <cmd_len$}  {}",
            inf.0, inf.1, inf.2
        ))
    });
    let mut v = vec![
        Line::default(),
        Line::styled(
            format!("Bindings for {mode} mode"),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::default(),
    ];
    v.extend(bindings);
    v
}

fn draw_help(f: &mut Frame, app: &mut App, area: Rect) {
    let mut command_bindings = help_commands();
    command_bindings.extend(bindings(app));
    let command_bindings = Paragraph::new(Text::from(command_bindings))
        .block(Block::bordered().title("Available commands and configured shortcuts"))
        .scroll(app.help_scroll);
    f.render_widget(command_bindings, area);
}

fn displayed_quote(names: &NameResolver, quote: &Message) -> Option<String> {
    let (name, _) = names.resolve(quote.from_id);
    let text = format!("({}) {}", name, quote.message.as_ref()?);
    Some(replace_mentions(quote, names, text))
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ]
            .as_ref(),
        )
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use crate::data::{AssociatedValue, BodyRange};
    use crate::signal::Attachment;

    use super::*;

    const USER_ID: Uuid = Uuid::nil();

    // formatting test options
    const PREFIX: &str = "                  ";
    const WIDTH: usize = 60;
    const HEIGHT: usize = 10;

    fn test_attachment() -> Attachment {
        Attachment {
            id: "2022-01-16T11:59:58.405665+00:00".to_string(),
            content_type: "image/jpeg".into(),
            filename: "/tmp/gurk/signal-2022-01-16T11:59:58.405665+00:00.jpg".into(),
            size: 238987,
        }
    }

    fn name_resolver() -> NameResolver<'static> {
        NameResolver::single_user(USER_ID, "boxdot".to_string(), Color::Green)
    }

    fn test_message() -> Message {
        Message {
            from_id: USER_ID,
            message: None,
            arrived_at: 1642334397421,
            quote: None,
            attachments: Default::default(),
            reactions: Default::default(),
            receipt: Receipt::Sent,
            body_ranges: Default::default(),
            send_failed: Default::default(),
            edit: Default::default(),
            edited: Default::default(),
        }
    }

    #[test]
    fn test_display_attachment_only_message() {
        let names = name_resolver();
        let msg = Message {
            attachments: vec![test_attachment()],
            ..test_message()
        };
        let rendered = display_message(
            &names,
            &msg,
            PREFIX,
            WIDTH,
            HEIGHT,
            ShowReceipt::Never,
            None,
            None,
            false,
        );

        let expected = ListItem::new(Text::from(vec![
            Line::from(vec![
                Span::styled("", Style::default().fg(Color::Yellow)),
                Span::styled(
                    display_time(msg.arrived_at),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled("boxdot", Style::default().fg(Color::Green)),
                Span::raw(": "),
                Span::raw("<file:///tmp/gurk/signal-2022-01-"),
            ]),
            Line::from(vec![Span::raw(
                "                  16T11:59:58.405665+00:00.jpg>",
            )]),
        ]));
        assert_eq!(rendered, Some(expected));
    }

    #[test]
    fn test_display_text_and_attachment_message() {
        let names = name_resolver();
        let msg = Message {
            message: Some("Hello, World!".into()),
            attachments: vec![test_attachment()],
            ..test_message()
        };
        let rendered = display_message(
            &names,
            &msg,
            PREFIX,
            WIDTH,
            HEIGHT,
            ShowReceipt::Never,
            None,
            None,
            false,
        );

        let expected = ListItem::new(Text::from(vec![
            Line::from(vec![
                Span::styled("", Style::default().fg(Color::Yellow)),
                Span::styled(
                    display_time(msg.arrived_at),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled("boxdot", Style::default().fg(Color::Green)),
                Span::raw(": "),
                Span::raw("Hello, World!"),
            ]),
            Line::from(vec![Span::raw(
                "                  <file:///tmp/gurk/signal-2022-01-",
            )]),
            Line::from(vec![Span::raw(
                "                  16T11:59:58.405665+00:00.jpg>",
            )]),
        ]));
        assert_eq!(rendered, Some(expected));
    }

    #[test]
    fn test_display_sent_receipt() {
        let names = name_resolver();
        let msg = Message {
            message: Some("Hello, World!".into()),
            receipt: Receipt::Sent,
            ..test_message()
        };
        let show_receipt = ShowReceipt::from_msg(&msg, USER_ID, true);
        let rendered = display_message(
            &names,
            &msg,
            PREFIX,
            WIDTH,
            HEIGHT,
            show_receipt,
            None,
            None,
            false,
        );

        let expected = ListItem::new(Text::from(vec![Line::from(vec![
            Span::styled("○ ", Style::default().fg(Color::Yellow)),
            Span::styled(
                display_time(msg.arrived_at),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("boxdot", Style::default().fg(Color::Green)),
            Span::raw(": "),
            Span::raw("Hello, World!"),
        ])]));
        assert_eq!(rendered, Some(expected));
    }

    #[test]
    fn test_display_received_receipt() {
        let names = name_resolver();
        let msg = Message {
            message: Some("Hello, World!".into()),
            receipt: Receipt::Delivered,
            ..test_message()
        };
        let show_receipt = ShowReceipt::from_msg(&msg, USER_ID, true);
        let rendered = display_message(
            &names,
            &msg,
            PREFIX,
            WIDTH,
            HEIGHT,
            show_receipt,
            None,
            None,
            false,
        );

        let expected = ListItem::new(Text::from(vec![Line::from(vec![
            Span::styled("◉ ", Style::default().fg(Color::Yellow)),
            Span::styled(
                display_time(msg.arrived_at),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("boxdot", Style::default().fg(Color::Green)),
            Span::raw(": "),
            Span::raw("Hello, World!"),
        ])]));
        assert_eq!(rendered, Some(expected));
    }

    #[test]
    fn test_display_delivered_receipt() {
        let names = name_resolver();
        let msg = Message {
            message: Some("Hello, World!".into()),
            receipt: Receipt::Read,
            ..test_message()
        };
        let show_receipt = ShowReceipt::from_msg(&msg, USER_ID, true);
        let rendered = display_message(
            &names,
            &msg,
            PREFIX,
            WIDTH,
            HEIGHT,
            show_receipt,
            None,
            None,
            false,
        );

        let expected = ListItem::new(Text::from(vec![Line::from(vec![
            Span::styled("● ", Style::default().fg(Color::Yellow)),
            Span::styled(
                display_time(msg.arrived_at),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("boxdot", Style::default().fg(Color::Green)),
            Span::raw(": "),
            Span::raw("Hello, World!"),
        ])]));
        assert_eq!(rendered, Some(expected));
    }

    #[test]
    fn test_display_show_receipts_disabled() {
        let names = name_resolver();
        let msg = Message {
            message: Some("Hello, World!".into()),
            receipt: Receipt::Read,
            ..test_message()
        };
        let show_receipt = ShowReceipt::from_msg(&msg, USER_ID, false);
        let rendered = display_message(
            &names,
            &msg,
            PREFIX,
            WIDTH,
            HEIGHT,
            show_receipt,
            None,
            None,
            false,
        );

        let expected = ListItem::new(Text::from(vec![Line::from(vec![
            Span::styled("", Style::default().fg(Color::Yellow)),
            Span::styled(
                display_time(msg.arrived_at),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("boxdot", Style::default().fg(Color::Green)),
            Span::raw(": "),
            Span::raw("Hello, World!"),
        ])]));
        assert_eq!(rendered, Some(expected));
    }

    #[test]
    fn test_display_receipts_for_incoming_message() {
        let user_id = Uuid::from_u128(1);
        let names = NameResolver::single_user(user_id, "boxdot".to_string(), Color::Green);
        let msg = Message {
            from_id: user_id,
            message: Some("Hello, World!".into()),
            receipt: Receipt::Read,
            ..test_message()
        };
        let show_receipt = ShowReceipt::from_msg(&msg, USER_ID, true);
        let rendered = display_message(
            &names,
            &msg,
            PREFIX,
            WIDTH,
            HEIGHT,
            show_receipt,
            None,
            None,
            false,
        );

        let expected = ListItem::new(Text::from(vec![Line::from(vec![
            Span::styled("  ", Style::default().fg(Color::Yellow)),
            Span::styled(
                display_time(msg.arrived_at),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("boxdot", Style::default().fg(Color::Green)),
            Span::raw(": "),
            Span::raw("Hello, World!"),
        ])]));
        assert_eq!(rendered, Some(expected));
    }

    #[test]
    fn test_display_mention() {
        let user_id = Uuid::from_u128(1);
        let names = NameResolver::single_user(user_id, "boxdot".to_string(), Color::Green);
        let msg = Message {
            from_id: user_id,
            message: Some("Mention ￼  and even more ￼ . End".into()),
            receipt: Receipt::Read,
            body_ranges: vec![
                BodyRange {
                    start: 8,
                    end: 9,
                    value: AssociatedValue::MentionUuid(user_id),
                },
                BodyRange {
                    start: 25,
                    end: 26,
                    value: AssociatedValue::MentionUuid(user_id),
                },
            ],
            ..test_message()
        };
        let show_receipt = ShowReceipt::from_msg(&msg, USER_ID, true);
        let rendered = display_message(
            &names,
            &msg,
            PREFIX,
            WIDTH,
            HEIGHT,
            show_receipt,
            None,
            None,
            false,
        );

        let expected = ListItem::new(Text::from(vec![
            Line::from(vec![
                Span::styled("  ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    display_time(msg.arrived_at),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled("boxdot", Style::default().fg(Color::Green)),
                Span::raw(": "),
                Span::raw("Mention @boxdot  and even more @boxdot ."),
            ]),
            Line::from(vec![Span::raw("                  End")]),
        ]));
        assert_eq!(rendered, Some(expected));
    }

    #[test]
    fn test_display_long_message_wraps() {
        let names = name_resolver();
        let msg = Message {
            message: Some(
                "This is a very long message that should wrap across multiple lines in the display"
                    .into(),
            ),
            ..test_message()
        };
        let rendered = display_message(
            &names,
            &msg,
            PREFIX,
            WIDTH,
            HEIGHT,
            ShowReceipt::Never,
            None,
            None,
            false,
        );

        let expected = ListItem::new(Text::from(vec![
            Line::from(vec![
                Span::styled("", Style::default().fg(Color::Yellow)),
                Span::styled(
                    display_time(msg.arrived_at),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled("boxdot", Style::default().fg(Color::Green)),
                Span::raw(": "),
                Span::raw("This is a very long message that should"),
            ]),
            Line::from(vec![Span::raw(
                "                  wrap across multiple lines in the display",
            )]),
        ]));
        assert_eq!(rendered, Some(expected));
    }

    #[test]
    fn test_display_unread_messages_division() {
        let names = name_resolver();
        let msg = Message {
            message: Some("Hello, World!".into()),
            ..test_message()
        };
        let division = "--new messages--".to_owned();
        let rendered = display_message(
            &names,
            &msg,
            PREFIX,
            WIDTH,
            HEIGHT,
            ShowReceipt::Never,
            None,
            Some(division.clone()),
            false,
        );

        let expected = ListItem::new(Text::from(vec![
            Line::from(division),
            Line::from(vec![
                Span::styled("", Style::default().fg(Color::Yellow)),
                Span::styled(
                    display_time(msg.arrived_at),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled("boxdot", Style::default().fg(Color::Green)),
                Span::raw(": "),
                Span::raw("Hello, World!"),
            ]),
        ]));
        assert_eq!(rendered, Some(expected));
    }
}

use crate::app::ReceiptEvent;
use crate::cursor::Cursor;
use crate::shortcuts::{ShortCut, SHORTCUTS};
use crate::util;
use crate::{app, App};
use app::Receipt;

use chrono::{Datelike, Timelike};
use itertools::Itertools;
use tui::backend::Backend;
use tui::layout::{Constraint, Corner, Direction, Layout, Rect};
use tui::style::{Color, Style};
use tui::text::{Span, Spans, Text};
use tui::widgets::{Block, Borders, List, ListItem, Paragraph};
use tui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use uuid::Uuid;

use std::fmt;

pub const CHANNEL_VIEW_RATIO: u32 = 4;

pub fn coords_within_channels_view<B: Backend>(
    f: &Frame<B>,
    app: &App,
    x: u16,
    y: u16,
) -> Option<(u16, u16)> {
    let rect = f.size();

    // Compute the offset due to the lines in the search bar
    let text_width = app.channel_text_width;
    let lines: Vec<String> =
        app.data
            .search_box
            .data
            .chars()
            .enumerate()
            .fold(Vec::new(), |mut lines, (idx, c)| {
                if idx % text_width == 0 {
                    lines.push(String::new());
                }
                match c {
                    '\n' => {
                        lines.last_mut().unwrap().push('\n');
                        lines.push(String::new())
                    }
                    _ => lines.last_mut().unwrap().push(c),
                }
                lines
            });
    let num_input_lines = lines.len().max(1);

    if y < 3 + num_input_lines as u16 {
        return None;
    }
    // 1 offset around the view for taking the border into account
    if 0 < x && x < rect.width / CHANNEL_VIEW_RATIO as u16 && 0 < y && y + 1 < rect.height {
        Some((x - 1, y - (3 + num_input_lines as u16)))
    } else {
        None
    }
}

pub fn draw<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    if app.is_help() {
        // Display shortcut panel
        let chunks = Layout::default()
            .constraints([
                Constraint::Percentage(15),
                Constraint::Percentage(70),
                Constraint::Percentage(15),
            ])
            .direction(Direction::Horizontal)
            .split(f.size());
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
        .split(f.size());

    draw_channels_column(f, app, chunks[0]);
    draw_chat(f, app, chunks[1]);
}

fn draw_channels_column<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let text_width = area.width.saturating_sub(2) as usize;
    let (wrapped_input, cursor, num_input_lines) = wrap(
        &app.data.search_box.data,
        app.data.search_box.cursor.clone(),
        text_width,
    );

    let chunks = Layout::default()
        .constraints(
            [
                Constraint::Length(num_input_lines as u16 + 2),
                Constraint::Min(0),
            ]
            .as_ref(),
        )
        .direction(Direction::Vertical)
        .split(area);

    draw_channels(f, app, chunks[1]);

    let input = Paragraph::new(Text::from(wrapped_input))
        .block(Block::default().borders(Borders::ALL).title("Search"));
    f.render_widget(input, chunks[0]);
    if app.is_searching {
        f.set_cursor(
            chunks[0].x + cursor.col as u16 + 1,
            chunks[0].y + cursor.line as u16 + 1,
        );
    }
}

fn draw_channels<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let channel_list_width = area.width.saturating_sub(2) as usize;
    let pattern = app.data.search_box.data.as_str();
    app.channel_text_width = channel_list_width;
    app.data.channels.filter_channels(pattern, &app.data.names);
    let channels: Vec<ListItem> = app
        .data
        .channels
        .iter()
        .map(|channel| {
            let unread_messages_label = if channel.unread_messages != 0 {
                format!(" ({})", channel.unread_messages)
            } else {
                String::new()
            };
            let label = format!("{}{}", channel.name, unread_messages_label);
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
            ListItem::new(vec![Spans::from(Span::raw(label))])
        })
        .collect();
    let channels = List::new(channels)
        .block(Block::default().borders(Borders::ALL).title("Channels"))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Gray));
    f.render_stateful_widget(channels, area, &mut app.data.channels.state);
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

fn draw_chat<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let text_width = area.width.saturating_sub(2) as usize;
    let (wrapped_input, cursor, num_input_lines) = wrap(
        &app.data.input.data,
        app.data.input.cursor.clone(),
        text_width,
    );

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

    let title = if app.data.is_multiline_input {
        "Input (Multiline)"
    } else {
        "Input"
    };

    let input = Paragraph::new(Text::from(wrapped_input))
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(input, chunks[1]);
    if !app.is_searching {
        f.set_cursor(
            chunks[1].x + cursor.col as u16 + 1,  // +1 for frame
            chunks[1].y + cursor.line as u16 + 1, // +1 for frame
        );
    }
    // completion needs to set_cursor to the new input
    // and make this input widget render
}

fn prepare_receipts(app: &mut App, height: usize) {
    let mut to_send = Vec::new();
    let user_id = app.user_id;
    let channel = app
        .data
        .channels
        .state
        .selected()
        .and_then(|idx| app.data.channels.items.get_mut(idx));
    let channel = match channel {
        Some(c) if !c.messages.items.is_empty() => c,
        _ => return,
    };

    let offset = if let Some(selected) = channel.messages.state.selected() {
        channel
            .messages
            .rendered
            .offset
            .min(selected)
            .max(selected.saturating_sub(height))
    } else {
        channel.messages.rendered.offset
    };

    let messages = &mut channel.messages.items[..];

    let _ = messages
        .iter_mut()
        .rev()
        .skip(offset)
        .for_each(|msg| match msg.receipt {
            Receipt::Delivered | Receipt::Nothing | Receipt::Sent => (),
            Receipt::Received => {
                if msg.from_id != user_id {
                    to_send.push((msg.from_id, msg.arrived_at));
                    msg.receipt = Receipt::Delivered
                }
            }
        });
    if !to_send.is_empty() {
        to_send
            .into_iter()
            .for_each(|(u, t)| app.add_receipt_event(ReceiptEvent::new(u, t, Receipt::Delivered)))
    }
}

fn draw_messages<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    // area without borders
    let height = area.height.saturating_sub(2) as usize;
    if height == 0 {
        return;
    }
    let width = area.width.saturating_sub(2) as usize;

    prepare_receipts(app, height);

    let channel = app.data.channels.state.selected().and_then(|idx| {
        app.data
            .channels
            .items
            .get(*app.data.channels.filtered_items.get(idx).unwrap())
    });
    let channel = match channel {
        Some(c) if !c.messages.items.is_empty() => c,
        _ => return,
    };

    let writing_people = app.writing_people(channel);

    // Calculate the offset in messages we start rendering with.
    // `offset` includes the selected message (if any), and is at most height-many messages to
    // the selected message, since we can't render more than height-many of them.
    let offset = if let Some(selected) = channel.messages.state.selected() {
        channel
            .messages
            .rendered
            .offset
            .min(selected)
            .max(selected.saturating_sub(height))
    } else {
        channel.messages.rendered.offset
    };

    let messages = &channel.messages.items[..];

    let names = NameResolver::compute_for_channel(app, channel);
    let max_username_width = names.max_name_width();

    // message display options
    const TIME_WIDTH: usize = 10;
    const DELIMITER_WIDTH: usize = 2;
    let prefix_width = TIME_WIDTH + max_username_width + DELIMITER_WIDTH;
    let prefix = " ".repeat(prefix_width);

    let messages_from_offset = messages.iter().rev().skip(offset).filter_map(|msg| {
        let print_receipt = app.user_id == msg.from_id;
        display_message(&names, msg, &prefix, width as usize, height, print_receipt)
    });

    // counters to accumulate messages as long they fit into the list height,
    // or up to the selected message
    let mut items_height = 0;
    let selected = channel.messages.state.selected().unwrap_or(0);

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

    // add unread messages line
    let unread_messages = channel.unread_messages;
    if unread_messages > 0 && unread_messages < items.len() {
        let new_message_line = "-".repeat(prefix_width)
            + "new messages"
            + &"-".repeat(width.saturating_sub(prefix_width));
        items.insert(unread_messages, ListItem::new(Span::from(new_message_line)));
    }

    let title = format!("Messages {}", writing_people);

    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Gray))
        .start_corner(Corner::BottomLeft);

    // re-borrow channel mutably
    let channel_idx = app.data.channels.state.selected().unwrap_or_default();
    let channel = &mut app.data.channels.items[channel_idx];

    // update selected state to point within `items`
    let state = &mut channel.messages.state;
    let selected_global = state.selected();
    if let Some(selected) = selected_global {
        state.select(Some(selected - offset));
    }

    f.render_stateful_widget(list, area, state);

    // restore selected state and update offset
    state.select(selected_global);
    channel.messages.rendered.offset = offset;
}

fn display_datetime(timestamp: u64) -> String {
    let dt = util::utc_timestamp_msec_to_local(timestamp);
    format!("{} {:02}:{:02} ", dt.weekday(), dt.hour(), dt.minute())
}

#[allow(clippy::too_many_arguments)]
fn display_message(
    names: &NameResolver,
    msg: &app::Message,
    prefix: &str,
    width: usize,
    height: usize,
    print_receipt: bool,
) -> Option<ListItem<'static>> {
    let time = Span::styled(
        display_datetime(msg.arrived_at),
        Style::default().fg(Color::Yellow),
    );

    let (from, from_color) = names.resolve(msg.from_id);

    let from = Span::styled(
        textwrap::indent(
            from,
            &" ".repeat(
                names
                    .max_name_width()
                    .checked_sub(from.width())
                    .unwrap_or_default(),
            ),
        ),
        Style::default().fg(from_color),
    );
    let delimiter = Span::from(": ");

    let wrap_opts = textwrap::Options::new(width)
        .initial_indent(prefix)
        .subsequent_indent(prefix);

    // collect message text
    let mut text = msg.message.clone().unwrap_or_default();
    add_attachments(msg, &mut text);
    if text.is_empty() {
        return None; // no text => nothing to render
    }
    add_reactions(msg, &mut text);
    if print_receipt {
        add_receipt(msg, &mut text);
    }

    let mut spans: Vec<Spans> = vec![];

    // prepend quote if any
    let quote_text = msg
        .quote
        .as_ref()
        .and_then(|quote| displayed_quote(names, quote));
    if let Some(quote_text) = quote_text.as_ref() {
        let quote_prefix = format!("{}> ", prefix);
        let quote_wrap_opts = textwrap::Options::new(width.saturating_sub(2))
            .initial_indent(&quote_prefix)
            .subsequent_indent(&quote_prefix);
        let quote_style = Style::default().fg(Color::Rgb(150, 150, 150));
        spans = textwrap::wrap(quote_text, quote_wrap_opts)
            .into_iter()
            .enumerate()
            .map(|(idx, line)| {
                let res = if idx == 0 {
                    vec![
                        time.clone(),
                        from.clone(),
                        delimiter.clone(),
                        Span::styled(line.strip_prefix(prefix).unwrap().to_string(), quote_style),
                    ]
                } else {
                    vec![Span::styled(line.to_string(), quote_style)]
                };
                Spans::from(res)
            })
            .collect();
    }

    let add_time = spans.is_empty();
    spans.extend(
        textwrap::wrap(&text, &wrap_opts)
            .into_iter()
            .enumerate()
            .map(|(idx, line)| {
                let res = if add_time && idx == 0 {
                    vec![
                        time.clone(),
                        from.clone(),
                        delimiter.clone(),
                        Span::from(line.strip_prefix(prefix).unwrap().to_string()),
                    ]
                } else {
                    vec![Span::from(line.to_string())]
                };
                Spans::from(res)
            }),
    );

    if spans.len() > height {
        // span is too big to be shown fully
        spans.resize(height - 1, Spans::from(""));
        spans.push(Spans::from(format!("{}[...]", prefix)));
    }
    Some(ListItem::new(Text::from(spans)))
}

fn add_attachments(msg: &app::Message, out: &mut String) {
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

fn add_reactions(msg: &app::Message, out: &mut String) {
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

fn add_receipt(msg: &app::Message, out: &mut String) {
    out.push(' ');
    out.push_str(msg.receipt.write());
}

fn draw_help<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let max_event_width = SHORTCUTS
        .iter()
        .map(
            |ShortCut {
                 event: ev,
                 description: _,
             }| { (**ev).width() },
        )
        .max()
        .unwrap_or(0);

    let width = area.width.saturating_sub(2) as usize;
    let delimiter = Span::from(": ");
    const DELIMITER_WIDTH: usize = 2;
    let prefix_width = max_event_width + DELIMITER_WIDTH + 1; // +1 because we add 1 extra " " between the event and the delimiter
    let prefix = " ".repeat(prefix_width);

    let wrap_opts = textwrap::Options::new(width)
        .initial_indent(&prefix)
        .subsequent_indent(&prefix);

    let shorts: Vec<ListItem> = SHORTCUTS
        .iter()
        .map(
            |ShortCut {
                 event: ev,
                 description: desc,
             }| {
                let event = ev;
                let description = desc;

                let wrapped = textwrap::wrap(description, &wrap_opts);

                let mut res = Vec::new();

                wrapped.into_iter().enumerate().for_each(|(i, line)| {
                    let mut truc = Vec::new();
                    if i == 0 {
                        let event_span = Span::from(textwrap::indent(
                            &" ".repeat(
                                max_event_width
                                    .checked_sub(event.width())
                                    .unwrap_or_default()
                                    + 1,
                            ),
                            event,
                        ));
                        truc.push(event_span);
                        truc.push(delimiter.clone());
                        truc.push(Span::from(line.strip_prefix(&prefix).unwrap().to_string()));
                    } else {
                        truc.push(Span::from(line.to_string()))
                    };

                    let spans = Spans::from(truc);
                    res.push(spans);
                });

                //let spans = Spans::from(res);

                ListItem::new(Text::from(res))
            },
        )
        .collect();

    let shorts_widget =
        List::new(shorts).block(Block::default().borders(Borders::ALL).title("Help"));
    f.render_stateful_widget(shorts_widget, area, &mut app.data.channels.state);
}

fn displayed_name(name: &str, first_name_only: bool) -> &str {
    if first_name_only {
        let space_pos = name.find(' ').unwrap_or_else(|| name.len());
        &name[0..space_pos]
    } else {
        name
    }
}

const USER_COLORS: &[Color] = &[
    Color::Red,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::Gray,
];

// Randomly but deterministically choose a color for a username
fn user_color(username: &str) -> Color {
    let idx = username
        .bytes()
        .fold(0, |sum, b| (sum + usize::from(b)) % USER_COLORS.len());
    USER_COLORS[idx]
}

/// Resolves names in a channel
struct NameResolver<'a> {
    app: Option<&'a App>,
    names_and_colors: Vec<(Uuid, &'a str, Color)>,
    max_name_width: usize,
}

impl<'a> NameResolver<'a> {
    fn compute_for_channel<'b>(app: &'a app::App, channel: &'b app::Channel) -> Self {
        let first_name_only = app.config.first_name_only;
        let mut names_and_colors = if let Some(group_data) = channel.group_data.as_ref() {
            group_data
                .members
                .iter()
                .map(|&uuid| {
                    let name = app.name_by_id(uuid);
                    let color = user_color(name);
                    let name = displayed_name(name, first_name_only);
                    (uuid, name, color)
                })
                .collect()
        } else {
            let user_id = app.user_id;
            let user_name = app.name_by_id(user_id);
            let mut self_color = user_color(user_name);
            let user_name = displayed_name(user_name, first_name_only);

            let contact_uuid = match channel.id {
                app::ChannelId::User(uuid) => uuid,
                _ => unreachable!("logic error"),
            };

            if contact_uuid == user_id {
                vec![(user_id, user_name, self_color)]
            } else {
                let contact_name = app.name_by_id(contact_uuid);
                let contact_color = user_color(contact_name);
                let contact_name = displayed_name(contact_name, first_name_only);

                if self_color == contact_color {
                    // use differnt color for our user name
                    if let Some(idx) = USER_COLORS.iter().position(|&c| c == self_color) {
                        self_color = USER_COLORS[(idx + 1) % USER_COLORS.len()];
                    }
                }

                vec![
                    (user_id, user_name, self_color),
                    (contact_uuid, contact_name, contact_color),
                ]
            }
        };
        names_and_colors.sort_unstable_by_key(|&(id, _, _)| id);

        let max_name_width = names_and_colors
            .iter()
            .map(|(_, name, _)| name.width())
            .max()
            .unwrap_or(0);

        Self {
            app: Some(app),
            names_and_colors,
            max_name_width,
        }
    }

    fn resolve(&self, id: Uuid) -> (&str, Color) {
        match self
            .names_and_colors
            .binary_search_by_key(&id, |&(id, _, _)| id)
        {
            Ok(idx) => {
                let (_, from, from_color) = self.names_and_colors[idx];
                (from, from_color)
            }
            Err(_) => (
                app::App::name_by_id(self.app.expect("logic error"), id),
                Color::Magenta,
            ),
        }
    }

    fn max_name_width(&self) -> usize {
        self.max_name_width
    }
}

fn displayed_quote(names: &NameResolver, quote: &app::Message) -> Option<String> {
    let (name, _) = names.resolve(quote.from_id);
    Some(format!("({}) {}", name, quote.message.as_ref()?))
}

#[cfg(test)]
mod tests {
    use crate::app::{Message, Receipt};
    use crate::signal::Attachment;

    use super::*;

    // formatting test options
    const PREFIX: &str = "                  ";
    const WIDTH: usize = 60;
    const HEIGHT: usize = 10;
    const PRINT_RECEIPT: bool = true;

    fn test_message() -> Message {
        Message {
            from_id: Uuid::nil(),
            message: None,
            arrived_at: 1642334397421,
            quote: None,
            attachments: vec![],
            reactions: vec![],
            receipt: Receipt::Sent,
        }
    }

    fn test_attachment() -> Attachment {
        Attachment {
            id: "2022-01-16T11:59:58.405665+00:00".to_string(),
            content_type: "image/jpeg".into(),
            filename: "/tmp/gurk/signal-2022-01-16T11:59:58.405665+00:00.jpg".into(),
            size: 238987,
        }
    }

    #[test]
    fn test_display_attachment_only_message() {
        let names = NameResolver {
            app: None,
            names_and_colors: vec![(Uuid::nil(), "boxdot", Color::Green)],
            max_name_width: 6,
        };

        let msg = Message {
            attachments: vec![test_attachment()],
            ..test_message()
        };
        let rendered = display_message(&names, &msg, PREFIX, WIDTH, HEIGHT, PRINT_RECEIPT);

        let expected = ListItem::new(Text::from(vec![
            Spans(vec![
                Span::styled(
                    display_datetime(msg.arrived_at),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled("boxdot", Style::default().fg(Color::Green)),
                Span::raw(": "),
                Span::raw("<file:///tmp/gurk/signal-2022-01-"),
            ]),
            Spans(vec![Span::raw(
                "                  16T11:59:58.405665+00:00.jpg> (x)",
            )]),
        ]));
        assert_eq!(rendered, Some(expected));
    }

    #[test]
    fn test_display_text_and_attachment_message() {
        let names = NameResolver {
            app: None,
            names_and_colors: vec![(Uuid::nil(), "boxdot", Color::Green)],
            max_name_width: 6,
        };

        let msg = Message {
            message: Some("Hello, World!".into()),
            attachments: vec![test_attachment()],
            ..test_message()
        };
        let rendered = display_message(&names, &msg, PREFIX, WIDTH, HEIGHT, PRINT_RECEIPT);

        let expected = ListItem::new(Text::from(vec![
            Spans(vec![
                Span::styled(
                    display_datetime(msg.arrived_at),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled("boxdot", Style::default().fg(Color::Green)),
                Span::raw(": "),
                Span::raw("Hello, World!"),
            ]),
            Spans(vec![Span::raw(
                "                  <file:///tmp/gurk/signal-2022-01-",
            )]),
            Spans(vec![Span::raw(
                "                  16T11:59:58.405665+00:00.jpg> (x)",
            )]),
        ]));
        assert_eq!(rendered, Some(expected));
    }
}

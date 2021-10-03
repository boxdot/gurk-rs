use crate::util;
use crate::{app, App};

use chrono::{Datelike, Timelike};
use itertools::Itertools;
use tui::backend::Backend;
use tui::layout::{Constraint, Corner, Direction, Layout, Rect};
use tui::style::{Color, Style};
use tui::text::{Span, Spans, Text};
use tui::widgets::{Block, Borders, List, ListItem, Paragraph};
use tui::Frame;
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use std::borrow::Cow;

pub const CHANNEL_VIEW_RATIO: u32 = 4;

pub fn coords_within_channels_view<B: Backend>(f: &Frame<B>, x: u16, y: u16) -> Option<(u16, u16)> {
    let rect = f.size();
    // 1 offset around the view for taking the border into account
    if 0 < x && x < rect.width / CHANNEL_VIEW_RATIO as u16 && 0 < y && y + 1 < rect.height {
        Some((x - 1, y - 1))
    } else {
        None
    }
}

pub fn draw<B: Backend>(f: &mut Frame<B>, app: &mut App) {
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

    let channel_list_width = chunks[0].width.saturating_sub(2) as usize;
    let channels: Vec<ListItem> = app
        .data
        .channels
        .items
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
    f.render_stateful_widget(channels, chunks[0], &mut app.data.channels.state);

    draw_chat(f, app, chunks[1]);
}

fn draw_chat<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let text_width = area.width.saturating_sub(2) as usize;
    let lines: Vec<String> =
        app.data
            .input
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
    // chars since newline on `cursor_y` line
    let mut cursor_x = app.data.input_cursor_chars;
    // line selected by `app.data.input_cursor`
    let mut cursor_y = 0;
    for string in &lines {
        cursor_y += 1;
        match string.len().cmp(&cursor_x) {
            std::cmp::Ordering::Less => cursor_x -= string.width(),
            _ => break,
        };
    }
    let num_input_lines = lines.len().max(1);
    let input: Vec<Spans> = lines.into_iter().map(Spans::from).collect();
    let extra_cursor_line = if cursor_x > 0 && cursor_x % text_width == 0 {
        1
    } else {
        0
    };
    let chunks = Layout::default()
        .constraints(
            [
                Constraint::Min(0),
                Constraint::Length(num_input_lines as u16 + 2 + extra_cursor_line),
            ]
            .as_ref(),
        )
        .direction(Direction::Vertical)
        .split(area);

    draw_messages(f, app, chunks[0]);

    let input = Paragraph::new(Text::from(input))
        .block(Block::default().borders(Borders::ALL).title("Input"));
    f.render_widget(input, chunks[1]);
    f.set_cursor(
        // Put cursor past the end of the input text
        chunks[1].x + ((cursor_x as u16) % text_width as u16) + 1,
        // Move one line down, from the border to the input line
        chunks[1].y + (cursor_x as u16 / (text_width as u16)) + cursor_y.max(1) as u16,
    );
}

fn draw_messages<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let channel = app
        .data
        .channels
        .state
        .selected()
        .and_then(|idx| app.data.channels.items.get(idx));
    let channel = match channel {
        Some(c) if !c.messages.items.is_empty() => c,
        _ => return,
    };

    // area without borders
    let height = area.height.saturating_sub(2) as usize;
    if height == 0 {
        return;
    }
    let width = area.width.saturating_sub(2) as usize;

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

    let names_and_colors = compute_names_and_colors(app, channel);
    let max_username_width = names_and_colors
        .iter()
        .map(|(_, name, _)| name.width())
        .max()
        .unwrap_or(0);

    // message display options
    let first_name_only = app.config.first_name_only;
    const TIME_WIDTH: usize = 10;
    const DELIMITER_WIDTH: usize = 2;
    let prefix_width = TIME_WIDTH + max_username_width + DELIMITER_WIDTH;
    let prefix = " ".repeat(prefix_width);

    let messages_from_offset = messages.iter().rev().skip(offset).filter_map(|msg| {
        display_message(
            app,
            msg,
            &names_and_colors,
            max_username_width,
            first_name_only,
            &prefix,
            width as usize,
            height,
        )
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

    let list = List::new(items)
        .block(Block::default().title("Messages").borders(Borders::ALL))
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

#[allow(clippy::too_many_arguments)]
fn display_message(
    app: &App,
    msg: &app::Message,
    names_and_colors: &[(Uuid, &str, Color)],
    max_username_width: usize,
    first_name_only: bool,
    prefix: &str,
    width: usize,
    height: usize,
) -> Option<ListItem<'static>> {
    let arrived_at = util::utc_timestamp_msec_to_local(msg.arrived_at);

    let time = Span::styled(
        format!(
            "{} {:02}:{:02} ",
            arrived_at.weekday(),
            arrived_at.hour(),
            arrived_at.minute()
        ),
        Style::default().fg(Color::Yellow),
    );

    let result = names_and_colors.binary_search_by_key(&msg.from_id, |&(id, _, _)| id);
    let from;
    let from_color;
    match result {
        Ok(idx) => {
            let (_, f, c) = names_and_colors[idx];
            from = f;
            from_color = c;
        }
        Err(_) => {
            from = app::App::name_by_id(app, msg.from_id);
            from_color = Color::Magenta;
        }
    }

    let from = Span::styled(
        textwrap::indent(
            from,
            &" ".repeat(
                max_username_width
                    .checked_sub(from.width())
                    .unwrap_or_default(),
            ),
        ),
        Style::default().fg(from_color),
    );
    let delimeter = Span::from(": ");

    let wrap_opts = textwrap::Options::new(width)
        .initial_indent(prefix)
        .subsequent_indent(prefix);

    let text = if msg.reactions.is_empty() {
        Cow::from(format!("{} {}", msg.message.as_ref()?, msg.status))
    } else {
        Cow::from(format!(
            "{} [{}] {}",
            msg.message.as_ref()?,
            msg.reactions.iter().map(|(_, emoji)| emoji).format(""),
            msg.status
        ))
    };

    let mut spans: Vec<Spans> = vec![];

    // prepend quote if any
    let quote_text = msg
        .quote
        .as_ref()
        .and_then(|quote| displayed_quote(quote, names_and_colors, first_name_only));
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
                        delimeter.clone(),
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
        textwrap::wrap(&text, wrap_opts)
            .into_iter()
            .enumerate()
            .map(|(idx, line)| {
                let res = if add_time && idx == 0 {
                    vec![
                        time.clone(),
                        from.clone(),
                        delimeter.clone(),
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

/// Returns a sorted vector of `(id, name, color)` by id.
fn compute_names_and_colors<'a, 'b>(
    app: &'a app::App,
    channel: &'b app::Channel,
) -> Vec<(Uuid, &'a str, Color)> {
    let first_name_only = app.config.first_name_only;
    let mut res = if let Some(group_data) = channel.group_data.as_ref() {
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
    res.sort_unstable_by_key(|&(id, _, _)| id);
    res
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

fn displayed_quote(
    quote: &app::Message,
    names_and_colors: &[(Uuid, &str, Color)],
    first_name_only: bool,
) -> Option<String> {
    let idx = names_and_colors.binary_search_by_key(&quote.from_id, |&(id, _, _)| id);
    if let Ok(idx) = idx {
        let name = names_and_colors[idx].1;
        let name = displayed_name(name, first_name_only);
        Some(format!("({}) {}", name, quote.message.as_ref()?))
    } else {
        quote.message.clone()
    }
}

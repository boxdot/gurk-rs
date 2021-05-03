use crate::util;
use crate::{app, App};

use chrono::{Datelike, Timelike};
use tui::backend::Backend;
use tui::layout::{Constraint, Corner, Direction, Layout, Rect};
use tui::style::{Color, Style};
use tui::text::{Span, Spans, Text};
use tui::widgets::{Block, Borders, List, ListItem, Paragraph};
use tui::Frame;
use unicode_width::UnicodeWidthStr;

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
    let messages = app
        .data
        .channels
        .state
        .selected()
        .and_then(|idx| app.data.channels.items.get(idx))
        .map(|channel| &channel.messages.items[..])
        .unwrap_or(&[]);

    let max_username_width = messages
        .iter()
        .map(|msg| displayed_name(app.name_by_id(msg.from_id), app.config.first_name_only).width())
        .max()
        .unwrap_or(0);

    let width = area.width - 2; // without borders

    let time_style = Style::default().fg(Color::Yellow);
    let messages = messages.iter().rev().filter_map(|msg| {
        let arrived_at = util::utc_timestamp_msec_to_local(msg.arrived_at);

        let time = Span::styled(
            format!(
                "{:02} {:02}:{:02} ",
                arrived_at.weekday(),
                arrived_at.hour(),
                arrived_at.minute()
            ),
            time_style,
        );
        let from = displayed_name(app.name_by_id(msg.from_id), app.config.first_name_only);
        let from = Span::styled(
            textwrap::indent(&from, &" ".repeat(max_username_width - from.width())),
            Style::default().fg(user_color(app.name_by_id(msg.from_id))),
        );
        let delimeter = Span::from(": ");

        let prefix_width = (time.width() + from.width() + delimeter.width()) as u16;
        let mut indent = " ".repeat(prefix_width.into());

        let wrap_opts = textwrap::Options::new(width.into())
            .initial_indent(&indent)
            .subsequent_indent(&indent);
        let mut lines = textwrap::wrap(msg.message.as_ref()?, wrap_opts);

        // prepend quote if any
        let quote = if let Some(displayed_quote) = msg
            .quote
            .as_ref()
            .and_then(|quote| displayed_quote(app, quote))
        {
            displayed_quote
        } else {
            String::new()
        };

        if !quote.is_empty() {
            indent.push_str("> ");
            let wrap_opts = textwrap::Options::new(width as usize + 2)
                .initial_indent(&indent)
                .subsequent_indent(&indent);
            let mut quote_lines = textwrap::wrap(quote.as_str(), wrap_opts);
            quote_lines.extend(lines.into_iter());
            lines = quote_lines;
        }

        let spans: Vec<Spans> = lines
            .into_iter()
            .enumerate()
            .map(|(idx, line)| {
                let res = if idx == 0 {
                    vec![
                        time.clone(),
                        from.clone(),
                        delimeter.clone(),
                        Span::from(line.strip_prefix(&indent).unwrap().to_string()),
                    ]
                } else {
                    vec![Span::from(line.to_string())]
                };
                Spans::from(res)
            })
            .collect();
        Some(spans)
    });

    let mut items: Vec<_> = messages.map(|s| ListItem::new(Text::from(s))).collect();

    if let Some(selected_idx) = app.data.channels.state.selected() {
        let unread_messages = app.data.channels.items[selected_idx].unread_messages;
        if unread_messages > 0 && unread_messages < items.len() {
            let prefix_width = max_username_width + 8;
            let new_message_line = "-".repeat(prefix_width)
                + "new messages"
                + &"-".repeat((width as usize).saturating_sub(prefix_width));

            items.insert(unread_messages, ListItem::new(Span::from(new_message_line)));
        }
    }

    let list = List::new(items)
        .block(Block::default().title("Messages").borders(Borders::ALL))
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Gray))
        .start_corner(Corner::BottomLeft);

    let selected = app.data.channels.state.selected().unwrap_or_default();

    let init = &mut app::Channel::empty();

    let state = &mut app
        .data
        .channels
        .items
        .get_mut(selected)
        .unwrap_or(init)
        .messages
        .state;

    f.render_stateful_widget(list, area, state);
}

// Randomly but deterministically choose a color for a username
fn user_color(username: &str) -> Color {
    use Color::*;
    const COLORS: &[Color] = &[Red, Green, Yellow, Blue, Magenta, Cyan, Gray];
    let idx = username
        .bytes()
        .map(|b| usize::from(b) % COLORS.len())
        .sum::<usize>()
        % COLORS.len();
    COLORS[idx]
}

fn displayed_name(name: &str, first_name_only: bool) -> &str {
    if first_name_only {
        let space_pos = name.find(' ').unwrap_or_else(|| name.len());
        &name[0..space_pos]
    } else {
        &name
    }
}

fn displayed_quote(app: &App, quote: &app::Message) -> Option<String> {
    if let Some(name) = app.get_name_by_id(quote.from_id) {
        let name = displayed_name(name, app.config.first_name_only);
        Some(format!("> ({}) {}", name, quote.message.as_ref()?))
    } else {
        quote.message.as_ref().map(|s| format!("> {}", s))
    }
}

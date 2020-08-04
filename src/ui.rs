use crate::signal;
use crate::{app, App};

use anyhow::Context;
use chrono::Timelike;
use tui::backend::Backend;
use tui::layout::{Constraint, Corner, Direction, Layout, Rect};
use tui::style::{Color, Style};
use tui::text::{Span, Spans, Text};
use tui::widgets::{Block, Borders, List, ListItem, Paragraph};
use tui::Frame;
use unicode_width::UnicodeWidthStr;

use std::path::PathBuf;

pub fn draw<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    let chunks = Layout::default()
        .constraints([Constraint::Ratio(1, 4), Constraint::Ratio(3, 4)].as_ref())
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
    let mut cursor_x = app.data.input_cursor;
    // line selected by `app.data.input_cursor`
    let mut cursor_y = 0;
    for string in &lines {
        cursor_y += 1;
        match string.len().cmp(&cursor_x) {
            std::cmp::Ordering::Less => cursor_x -= string.len(),
            _ => break,
        };
    }
    let num_input_lines = lines.len().max(1);
    let input: Vec<Spans> = lines.into_iter().map(Spans::from).collect();
    let extra_cursor_line = if app.data.input_cursor > 0 && app.data.input_cursor % text_width == 0
    {
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

fn draw_messages<B: Backend>(f: &mut Frame<B>, app: &App, area: Rect) {
    let messages = app
        .data
        .channels
        .state
        .selected()
        .and_then(|idx| app.data.channels.items.get(idx))
        .map(|channel| &channel.messages[..])
        .unwrap_or(&[]);

    let max_username_width = messages
        .iter()
        .map(|msg| displayed_name(&msg.from, app.config.first_name_only).width())
        .max()
        .unwrap_or(0);

    let width = area.width - 2; // without borders
    let max_lines = area.height;

    let time_style = Style::default().fg(Color::Yellow);
    let messages = messages
        .iter()
        .rev()
        // we can't show more messages atm and don't have messages navigation
        .take(max_lines as usize)
        .map(|msg| {
            let arrived_at = msg.arrived_at.with_timezone(&chrono::Local);

            let time = Span::styled(
                format!("{:02}:{:02} ", arrived_at.hour(), arrived_at.minute()),
                time_style,
            );
            let from = displayed_name(&msg.from, app.config.first_name_only);
            let from = Span::styled(
                textwrap::indent(&from, &" ".repeat(max_username_width - from.width())),
                Style::default().fg(user_color(&msg.from)),
            );
            let delimeter = Span::from(": ");

            let displayed_message = displayed_message(&msg);

            let prefix_width = (time.width() + from.width() + delimeter.width()) as u16;
            let indent = " ".repeat(prefix_width.into());
            let lines = textwrap::wrap_iter(
                displayed_message.as_str(),
                width.saturating_sub(prefix_width).into(),
            );

            let spans: Vec<Spans> = lines
                .enumerate()
                .map(|(idx, line)| {
                    let res = if idx == 0 {
                        vec![
                            time.clone(),
                            from.clone(),
                            delimeter.clone(),
                            Span::from(line.to_string()),
                        ]
                    } else {
                        vec![Span::from(format!("{}{}", indent, line))]
                    };
                    Spans::from(res)
                })
                .collect();
            spans
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
        .start_corner(Corner::BottomLeft);
    f.render_widget(list, area);
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

fn displayed_message(msg: &app::Message) -> String {
    let symlinks = symlink_attachments(&msg.attachments).unwrap();
    let displayed_attachments = symlinks
        .into_iter()
        .map(|path| format!("[file://{}]", path.display()));
    let message = msg.message.as_deref().unwrap_or_default();
    if !message.is_empty() {
        itertools::join(
            std::iter::once(message.to_string()).chain(displayed_attachments),
            "\n",
        )
    } else {
        itertools::join(displayed_attachments, "\n")
    }
}

/// Creates symlinks to attachments in default tmp dir with short random file names.
fn symlink_attachments(attachments: &[signal::Attachment]) -> anyhow::Result<Vec<PathBuf>> {
    let signal_cli_data_dir = std::env::var("XDG_DATA_HOME")
        .map(|s| PathBuf::from(s).join("signal-cli"))
        .or_else(|_| {
            std::env::var("HOME").map(|s| PathBuf::from(s).join(".local/share/signal-cli"))
        })
        .context("could not find signal-cli data path")?;

    let tmp_attachments_dir = std::env::temp_dir().join("gurk");
    std::fs::create_dir_all(&tmp_attachments_dir)
        .with_context(|| format!("failed to create {}", tmp_attachments_dir.display()))?;

    let tmp_attachments_symlinks: anyhow::Result<Vec<_>> = attachments
        .iter()
        .map(|attachment| {
            let source = signal_cli_data_dir.join("attachments").join(&attachment.id);

            let mut filename = id_to_short_random_filename(&attachment.id);
            if let Some(ext) = attachment.filename.extension() {
                filename += ".";
                filename += &ext.to_string_lossy();
            };
            let dest = tmp_attachments_dir.join(filename);

            let _ = std::fs::remove_file(&dest);
            std::os::unix::fs::symlink(&source, &dest).with_context(|| {
                format!(
                    "failed to create symlink: {} -> {}",
                    source.display(),
                    dest.display(),
                )
            })?;
            Ok(dest)
        })
        .collect();
    Ok(tmp_attachments_symlinks?)
}

fn xorshift32(mut x: u32) -> u32 {
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

fn id_to_short_random_filename(id: &str) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut seed = id
        .chars()
        .fold(0u32, |acc, c| acc.wrapping_add(c as u32))
        .max(1); // must be != 0
    (0..6)
        .map(move |_| {
            seed = xorshift32(seed);
            CHARSET[seed as usize % CHARSET.len()] as char
        })
        .collect()
}

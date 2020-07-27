use crate::App;

use chrono::Timelike;
use tui::backend::Backend;
use tui::layout::{Constraint, Corner, Direction, Layout, Rect};
use tui::style::{Color, Style};
use tui::text::Text;
use tui::text::{Span, Spans};
use tui::widgets::{Block, Borders, List, ListItem, Paragraph};
use tui::Frame;

use unicode_width::UnicodeWidthStr;

pub fn draw<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    let chunks = Layout::default()
        .constraints([Constraint::Ratio(1, 4), Constraint::Ratio(3, 4)])
        .direction(Direction::Horizontal)
        .split(f.size());

    let channels: Vec<ListItem> = app
        .data
        .channels
        .items
        .iter()
        .map(|channel| ListItem::new(vec![Spans::from(Span::raw(&channel.name))]))
        .collect();
    let channels = List::new(channels)
        .block(Block::default().borders(Borders::ALL).title("Channels"))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Gray));
    f.render_stateful_widget(channels, chunks[0], &mut app.data.channels.state);

    draw_chat(f, app, chunks[1]);
}

fn draw_chat<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .direction(Direction::Vertical)
        .split(area);

    draw_messages(f, app, chunks[0]);

    let input = Paragraph::new(app.data.input.as_ref())
        .block(Block::default().borders(Borders::ALL).title("Input"));
    f.render_widget(input, chunks[1]);
    f.set_cursor(
        // Put cursor past the end of the input text
        chunks[1].x + app.data.input.width() as u16 + 1,
        // Move one line down, from the border to the input line
        chunks[1].y + 1,
    );
}

fn draw_messages<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let messages = app
        .data
        .channels
        .state
        .selected()
        .map(|idx| &app.data.channels.items[idx].messages[..])
        .unwrap_or(&[]);

    let max_username_width = messages
        .iter()
        .map(|msg| displayed_name(&msg.from, app.config.first_name_only).width())
        .max()
        .unwrap_or(0);

    let width = area.right() - area.left() - 2; // without borders

    let time_style = Style::default().fg(Color::Yellow);
    let messages: Vec<Vec<Spans>> = messages
        .iter()
        .rev()
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

            let prefix_width = (time.width() + from.width() + delimeter.width()) as u16;
            let indent = " ".repeat(prefix_width.into());
            let lines =
                textwrap::wrap_iter(msg.text.as_str(), width.saturating_sub(prefix_width).into());

            lines
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
                .collect()
        })
        .collect();

    let items: Vec<_> = messages
        .into_iter()
        .map(|s| ListItem::new(Text::from(s)))
        .collect();
    let list = List::new(items)
        .block(Block::default().title("Messages").borders(Borders::ALL))
        .style(Style::default().fg(Color::White))
        .start_corner(Corner::BottomLeft);
    f.render_stateful_widget(list, area, &mut app.data.channels.state);
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

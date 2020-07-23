use crate::App;

use chrono::Timelike;
use tui::backend::Backend;
use tui::layout::{Constraint, Direction, Layout, Rect};
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
        .channels
        .items
        .iter()
        .map(|channel| ListItem::new(vec![Spans::from(Span::raw(&channel.name))]))
        .collect();
    let channels = List::new(channels)
        .block(Block::default().borders(Borders::ALL).title("Channels"))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Gray));
    f.render_stateful_widget(channels, chunks[0], &mut app.channels.state);

    draw_chat(f, app, chunks[1]);
}

fn draw_chat<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .direction(Direction::Vertical)
        .split(area);

    draw_messages(f, app, chunks[0]);

    let input = Paragraph::new(app.input.as_ref())
        .block(Block::default().borders(Borders::ALL).title("Input"));
    f.render_widget(input, chunks[1]);
    f.set_cursor(
        // Put cursor past the end of the input text
        chunks[1].x + app.input.width() as u16 + 1,
        // Move one line down, from the border to the input line
        chunks[1].y + 1,
    );
}

fn draw_messages<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let max_username_width = app
        .current_chat
        .msgs
        .items
        .iter()
        .map(|msg| msg.from.width())
        .max()
        .unwrap_or(0);
    let prefix_width = max_username_width + 8;

    let width = area.right() - area.left() - 2; // without borders

    let messages: Vec<_> = app
        .current_chat
        .msgs
        .items
        .iter()
        .map(|msg| {
            let prefix = format!(
                "{}:{} {}{}: ",
                msg.arrived_at.hour(),
                msg.arrived_at.minute(),
                " ".repeat(max_username_width - msg.from.width()),
                msg.from
            );

            let wrapped_msg = textwrap::fill(msg.text.as_str(), width as usize - prefix_width);
            let wrapped_msg = textwrap::indent(&wrapped_msg, &" ".repeat(prefix_width));
            format!("{}{}", prefix, &wrapped_msg[prefix_width..])
        })
        .collect();

    let items: Vec<_> = messages
        .iter()
        .map(|s| ListItem::new(Text::from(s.as_ref())))
        .collect();
    let list = List::new(items)
        .block(Block::default().title("Messages").borders(Borders::ALL))
        .style(Style::default().fg(Color::White));
    f.render_widget(list, area);
}

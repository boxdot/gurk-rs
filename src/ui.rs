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

    let width = area.right() - area.left() - 2; // without borders

    let time_style = Style::default().fg(Color::Yellow);
    let from_style = Style::default().fg(Color::Green);
    let messages: Vec<Vec<Spans>> = app
        .current_chat
        .msgs
        .items
        .iter()
        .rev()
        .map(|msg| {
            let time = Span::styled(
                format!("{}:{} ", msg.arrived_at.hour(), msg.arrived_at.minute()),
                time_style,
            );
            let from = Span::styled(
                textwrap::indent(
                    &msg.from,
                    &" ".repeat(max_username_width - msg.from.width()),
                ),
                from_style,
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
    f.render_widget(list, area);
}

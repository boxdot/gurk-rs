use crate::App;

use tui::backend::Backend;
use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::style::{Color, Modifier, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
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
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");
    f.render_stateful_widget(channels, chunks[0], &mut app.channels.state);

    draw_chat(f, app, chunks[1]);
}

fn draw_chat<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .direction(Direction::Vertical)
        .split(area);

    let from_style = Style::default().fg(Color::Green);
    let messages: Vec<Spans> = app
        .current_chat
        .msgs
        .items
        .iter()
        .map(|msg| {
            Spans::from(vec![
                Span::styled(format!("{:<7}", msg.from), from_style),
                Span::raw(&msg.text),
            ])
        })
        .collect();
    let block = Block::default().borders(Borders::ALL).title("Messages");
    let paragraph = Paragraph::new(messages)
        .block(block)
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, chunks[0]);

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

use tui::backend::Backend;
use tui::Frame;

use crate::app::App;

use super::CHANNEL_VIEW_RATIO;

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

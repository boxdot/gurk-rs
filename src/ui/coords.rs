use tui::layout::Rect;

use super::CHANNEL_VIEW_RATIO;

pub fn coords_within_channels_view(area: Rect, x: u16, y: u16) -> Option<(u16, u16)> {
    if y < 1 {
        None
    }
    // 1 offset around the view for taking the border into account
    else if 0 < x && x < area.width / CHANNEL_VIEW_RATIO as u16 && 0 < y && y + 1 < area.height {
        Some((x - 1, y - 1))
    } else {
        None
    }
}

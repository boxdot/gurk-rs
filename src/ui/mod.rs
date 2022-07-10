//! Display UI as a function of the app's state
//!
//! Also contains helpers for computing coordinates (for clicking)

mod coords;
mod draw;
mod name_resolver;

pub use coords::coords_within_channels_view;
pub use draw::draw;

pub const CHANNEL_VIEW_RATIO: u32 = 4;

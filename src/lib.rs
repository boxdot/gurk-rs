//! Signal Messenger client for terminal

pub mod app;
pub mod backoff;
mod channels;
pub mod config;
pub mod cursor;
pub mod data;
#[cfg(feature = "dev")]
pub mod dev;
pub(crate) mod emoji;
pub mod event;
mod handlers;
pub mod input;
pub mod receipt;
pub mod shortcuts;
pub mod signal;
pub mod storage;
pub mod ui;
pub mod util;

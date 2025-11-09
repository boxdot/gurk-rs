use std::borrow::Cow;
use std::collections::HashMap;

use ratatui::style::Color;
use uuid::Uuid;

use crate::storage::Storage;

/// Once constructed for a channel, resolves uuid to name and color
///
/// Construction takes time, lookup (resolving) is fast
pub struct NameResolver<'a> {
    storage: Option<&'a dyn Storage>,
    names_and_colors: HashMap<Uuid, (String, Color)>,
    first_name_only: bool,
}

impl<'a> NameResolver<'a> {
    pub(crate) fn new(storage: &'a dyn Storage, first_name_only: bool) -> Self {
        Self {
            storage: Some(storage),
            names_and_colors: Default::default(),
            first_name_only,
        }
    }

    /// Returns name and color for the given id
    pub(crate) fn resolve_and_cache(&mut self, id: Uuid) -> (Cow<'_, str>, Color) {
        let (name, color) = self.names_and_colors.entry(id).or_insert_with(|| {
            name_and_color(self.storage.expect("logic error"), id, self.first_name_only)
        });
        (Cow::Borrowed(name), *color)
    }

    /// Resolver with a single user
    #[cfg(test)]
    pub fn single_user(user_id: Uuid, username: String, color: Color) -> NameResolver<'static> {
        NameResolver {
            storage: None,
            names_and_colors: [(user_id, (username, color))].into_iter().collect(),
            first_name_only: false,
        }
    }
}

fn name_and_color(storage: &dyn Storage, id: Uuid, first_name_only: bool) -> (String, Color) {
    let name = storage
        .name(id)
        .unwrap_or_else(|| Cow::Owned(id.to_string()));
    let color = user_color(&name);
    let name = strip_ansi_escapes::strip_str(displayed_name(&name, first_name_only));
    (name, color)
}

fn displayed_name(name: &str, first_name_only: bool) -> &str {
    if first_name_only {
        let space_pos = name.find(' ').unwrap_or(name.len());
        &name[0..space_pos]
    } else {
        name
    }
}

const USER_COLORS: &[Color] = &[
    Color::Red,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::Gray,
];

// Randomly but deterministically choose a color for a username
pub(crate) fn user_color(username: &str) -> Color {
    let idx = username
        .bytes()
        .fold(0, |sum, b| (sum + usize::from(b)) % USER_COLORS.len());
    USER_COLORS[idx]
}

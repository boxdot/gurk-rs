use std::borrow::Cow;
use std::collections::HashMap;

use ratatui::style::Color;
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use crate::app::App;
use crate::storage::MessageId;

/// Once constructed for a channel, resolves uuid to name and color
///
/// Construction takes time, lookup (resolving) is fast
pub struct NameResolver<'a> {
    app: Option<&'a App>,
    names_and_colors: HashMap<Uuid, (String, Color)>,
    max_name_width: usize,
}

impl<'a> NameResolver<'a> {
    pub fn compute(
        app: &'a App,
        relevant_message_ids: impl IntoIterator<Item = MessageId>,
    ) -> Self {
        let mut names_and_colors: HashMap<Uuid, (String, Color)> = Default::default();
        names_and_colors.insert(app.user_id, app.name_and_color(app.user_id));
        for message_id in relevant_message_ids {
            if let Some(message) = app.storage.message(message_id) {
                names_and_colors
                    .entry(message.from_id)
                    .or_insert_with(|| app.name_and_color(message.from_id));
                if message_id.channel_id.is_user() && message_id.channel_id != app.user_id {
                    break; // ammortize direct contacts
                }
            }
        }

        let max_name_width = names_and_colors
            .values()
            .map(|(name, _)| name.width())
            .max()
            .unwrap_or(0);

        Self {
            app: Some(app),
            names_and_colors,
            max_name_width,
        }
    }

    /// Returns name and color for the given id
    pub fn resolve(&self, id: Uuid) -> (Cow<str>, Color) {
        self.names_and_colors
            .get(&id)
            .map(|(name, color)| (name.into(), *color))
            .unwrap_or_else(|| {
                let name = self.app.expect("logic error").name_by_id(id).into();
                (name, Color::Magenta)
            })
    }

    /// Returns the char width of the longest name
    pub(super) fn max_name_width(&self) -> usize {
        self.max_name_width
    }

    /// Resolver with a single user
    #[cfg(test)]
    pub fn single_user(user_id: Uuid, username: String, color: Color) -> NameResolver<'static> {
        NameResolver {
            app: None,
            names_and_colors: [(user_id, (username, color))].into_iter().collect(),
            max_name_width: 6,
        }
    }
}

impl App {
    fn name_and_color(&self, id: Uuid) -> (String, Color) {
        let name = self.name_by_id(id);
        let color = user_color(&name);
        let name = displayed_name(name, self.config.first_name_only);
        (name, color)
    }
}

fn displayed_name(name: String, first_name_only: bool) -> String {
    if first_name_only {
        let space_pos = name.find(' ').unwrap_or(name.len());
        name[0..space_pos].to_string()
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
fn user_color(username: &str) -> Color {
    let idx = username
        .bytes()
        .fold(0, |sum, b| (sum + usize::from(b)) % USER_COLORS.len());
    USER_COLORS[idx]
}

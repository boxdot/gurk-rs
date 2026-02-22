use std::borrow::Cow;
use std::collections::HashMap;

use ratatui::style::Color;
use uuid::Uuid;

use crate::app::App;
use crate::storage::MessageId;

/// Once constructed for a channel, resolves uuid to name and color
///
/// Construction takes time, lookup (resolving) is fast
pub struct NameResolver<'a> {
    app: Option<&'a App>,
    names_and_colors: HashMap<Uuid, (String, Color)>,
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
                if message_id.channel_id.is_user() {
                    if message_id.channel_id == app.user_id {
                        break; // amortize notes channel
                    } else if message.from_id != app.user_id {
                        // use different color for our user name
                        let &(_, contact_color) =
                            names_and_colors.get(&message.from_id).expect("logic error");
                        let (_, self_color) =
                            names_and_colors.get_mut(&app.user_id).expect("logic error");
                        if self_color == &contact_color
                            && let Some(idx) = USER_COLORS.iter().position(|&c| c == *self_color)
                        {
                            *self_color = USER_COLORS[(idx + 1) % USER_COLORS.len()];
                        }
                        break; // amortize direct channel
                    }
                }
            }
        }

        Self {
            app: Some(app),
            names_and_colors,
        }
    }

    /// Returns name and color for the given id
    pub fn resolve(&self, id: Uuid) -> (Cow<'_, str>, Color) {
        self.names_and_colors
            .get(&id)
            .map(|(name, color)| (name.into(), *color))
            .unwrap_or_else(|| {
                let name = self.app.expect("logic error").name_by_id_cached(id).into();
                (name, Color::Magenta)
            })
    }

    /// Resolver with a single user
    #[cfg(test)]
    pub fn single_user(user_id: Uuid, username: String, color: Color) -> NameResolver<'static> {
        NameResolver {
            app: None,
            names_and_colors: [(user_id, (username, color))].into_iter().collect(),
        }
    }
}

impl App {
    fn name_and_color(&self, id: Uuid) -> (String, Color) {
        let name = self.name_by_id_cached(id);
        let color = user_color(&name);
        let name =
            strip_ansi_escapes::strip_str(displayed_name(&name, self.config.first_name_only));
        (name, color)
    }
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
fn user_color(username: &str) -> Color {
    let idx = username
        .bytes()
        .fold(0, |sum, b| (sum + usize::from(b)) % USER_COLORS.len());
    USER_COLORS[idx]
}

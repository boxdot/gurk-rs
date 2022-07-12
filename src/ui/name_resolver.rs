use std::borrow::Cow;

use tui::style::Color;
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use crate::app::App;
use crate::data::{Channel, ChannelId};

/// Once constructed for a channel, resolves uuid to name and color
///
/// Construction takes time, lookup (resolving) is fast
// TODO: Cache in the app
pub struct NameResolver<'a> {
    app: Option<&'a App>,
    // invariant: sorted by Uuid
    names_and_colors: Vec<(Uuid, String, Color)>,
    max_name_width: usize,
}

impl<'a> NameResolver<'a> {
    /// Constructs the resolver for channel
    pub fn compute_for_channel<'b>(app: &'a App, channel: &'b Channel) -> Self {
        let first_name_only = app.config.first_name_only;
        let mut names_and_colors: Vec<(Uuid, String, Color)> =
            if let Some(group_data) = channel.group_data.as_ref() {
                // group channel
                group_data
                    .members
                    .iter()
                    .map(|&uuid| {
                        let name = app.name_by_id(uuid);
                        let color = user_color(&name);
                        let name = displayed_name(name, first_name_only);
                        (uuid, name, color)
                    })
                    .collect()
            } else {
                // direct message channel
                let user_id = app.user_id;
                let user_name = app.name_by_id(user_id);
                let mut self_color = user_color(&user_name);
                let user_name = displayed_name(user_name, first_name_only);

                let contact_uuid = match channel.id {
                    ChannelId::User(uuid) => uuid,
                    _ => unreachable!("logic error"),
                };

                if contact_uuid == user_id {
                    vec![(user_id, user_name, self_color)]
                } else {
                    let contact_name = app.name_by_id(contact_uuid);
                    let contact_color = user_color(&contact_name);
                    let contact_name = displayed_name(contact_name, first_name_only);

                    if self_color == contact_color {
                        // use different color for our user name
                        if let Some(idx) = USER_COLORS.iter().position(|&c| c == self_color) {
                            self_color = USER_COLORS[(idx + 1) % USER_COLORS.len()];
                        }
                    }

                    vec![
                        (user_id, user_name, self_color),
                        (contact_uuid, contact_name, contact_color),
                    ]
                }
            };
        names_and_colors.sort_unstable_by_key(|&(id, _, _)| id);

        let max_name_width = names_and_colors
            .iter()
            .map(|(_, name, _)| name.width())
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
        match self
            .names_and_colors
            .binary_search_by_key(&id, |&(id, _, _)| id)
        {
            Ok(idx) => {
                let (_, from, from_color) = &self.names_and_colors[idx];
                (from.into(), *from_color)
            }
            Err(_) => (
                self.app.expect("logic error").name_by_id(id).into(),
                Color::Magenta,
            ),
        }
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
            names_and_colors: vec![(user_id, username, color)],
            max_name_width: 6,
        }
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

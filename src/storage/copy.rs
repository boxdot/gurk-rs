use tracing::{debug, error};
use uuid::Uuid;

use crate::data::{Channel, ChannelId, GroupData, TypingSet};
use crate::signal::SignalManager;

use super::Storage;

#[derive(Debug, Default)]
pub struct Stats {
    pub channels: usize,
    pub messages: usize,
    pub names: usize,
}

pub fn copy(from: &dyn Storage, to: &mut dyn Storage) -> Stats {
    let mut stats = Stats::default();

    // to.store_metadata(from.metadata().into_owned());
    //
    // for channel in from.channels() {
    //     let channel_id = channel.id;
    //     to.store_channel(channel.into_owned());
    //     stats.channels += 1;
    //     for message_id in from.messages(channel_id) {
    //         let Some(message) = from.message(message_id) else {
    //             continue;
    //         };
    //         to.store_message(channel_id, message.into_owned());
    //         stats.messages += 1;
    //     }
    // }
    //
    // for (id, name) in from.names() {
    //     to.store_name(id, name.into_owned());
    //     stats.names += 1;
    // }

    stats
}

/// Copies contacts and groups from the signal manager into the storages
///
/// If contact/group is not in the storage, a new one is created. Group channels are updated,
/// existing contacts are skipped. Contacts with empty name are also skipped.
///
/// Note: At the moment, there is no group sync implemented in presage, so only contacts are
/// synced fully.
pub async fn sync_from_signal(manager: &dyn SignalManager, storage: &mut dyn Storage) {
    for contact in manager.contacts().await {
        if contact.name.is_empty() {
            // not sure what to do with contacts without a name
            continue;
        }
        let channel_id = contact.uuid.into();
        if storage.channel(channel_id).is_none() {
            debug!(
                name =% contact.name,
                "storing new contact from signal manager"
            );
            storage.store_channel(Channel {
                id: channel_id,
                name: contact.name.trim().to_owned(),
                group_data: None,
                unread_messages: 0,
                typing: TypingSet::new(false),
            });
        }
    }

    for (master_key_bytes, group) in manager.groups().await {
        let channel_id = match ChannelId::from_master_key_bytes(master_key_bytes) {
            Ok(channel_id) => channel_id,
            Err(error) => {
                error!(%error, "failed to derive group id from master key bytes");
                continue;
            }
        };
        let new_group_data = || GroupData {
            master_key_bytes,
            members: group
                .members
                .iter()
                .map(|member| member.aci.into())
                .collect(),
            revision: group.revision,
        };
        match storage.channel(channel_id) {
            Some(mut channel) => {
                let mut is_changed = false;
                if channel.name != group.title {
                    channel.to_mut().name = group.title;
                    is_changed = true;
                }
                if channel.group_data.as_ref().map(|d| d.revision) != Some(group.revision) {
                    let group_data = channel
                        .to_mut()
                        .group_data
                        .get_or_insert_with(new_group_data);
                    group_data.revision = group.revision;
                    is_changed = true;
                }
                if channel
                    .group_data
                    .as_ref()
                    .map(|d| d.members.iter().copied())
                    .into_iter()
                    .flatten()
                    .ne(group.members.iter().map(|m| Uuid::from(m.aci)))
                {
                    let group_data = channel
                        .to_mut()
                        .group_data
                        .get_or_insert_with(new_group_data);
                    group_data.members = group.members.iter().map(|m| m.aci.into()).collect();
                    is_changed = true;
                }
                if is_changed {
                    debug!(?channel_id, "storing modified channel from signal manager");
                    storage.store_channel(channel.into_owned());
                }
            }
            None => {
                debug!(?channel_id, "storing new channel from signal manager");
                storage.store_channel(Channel {
                    id: channel_id,
                    name: group.title,
                    group_data: Some(new_group_data()),
                    unread_messages: 0,
                    typing: TypingSet::new(true),
                });
            }
        }
    }
}

use super::Storage;

#[derive(Debug, Default)]
pub struct Stats {
    pub channels: usize,
    pub messages: usize,
    pub names: usize,
}

pub fn copy(from: &dyn Storage, to: &mut dyn Storage) -> Stats {
    let mut stats = Stats::default();

    to.store_metadata(from.metadata().into_owned());

    for channel in from.channels() {
        let channel_id = channel.id;
        to.store_channel(channel.into_owned());
        stats.channels += 1;
        for message in from.messages(channel_id) {
            to.store_message(channel_id, message.into_owned());
            stats.messages += 1;
        }
    }

    for (id, name) in from.names() {
        to.store_name(id, name.into_owned());
        stats.names += 1;
    }

    stats
}

use crate::app::AppData;

use anyhow::Context;
use log::info;
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

/// Data storage abstraction
///
/// Note: at the moment, we only support the full saving and loading of app data. Later, we plan
/// to split it in more granular operations.
pub trait Storage {
    fn save_app_data(&self, data: &AppData) -> anyhow::Result<()>;

    /// Loads the app data.
    ///
    /// In case, the app data exists, but can't be deserialized/loaded, this method should fail with
    /// an error, instead of returning a *new* app data which would override the old incompatible
    /// one.
    ///
    /// After the app data is loaded, this method must ensure that the user with the given`user_id`
    /// and `user_name` is indexed in the app data names.
    fn load_app_data(&self, user_id: Uuid, user_name: String) -> anyhow::Result<AppData>;
}

/// Storage based on a single JSON file.
pub struct JsonStorage {
    data_path: PathBuf,
    fallback_data_path: Option<PathBuf>,
}

impl Storage for JsonStorage {
    fn save_app_data(&self, data: &AppData) -> anyhow::Result<()> {
        Self::save_to(data, &self.data_path)
    }

    fn load_app_data(&self, user_id: Uuid, user_name: String) -> anyhow::Result<AppData> {
        let mut data = self.load_app_data_impl()?;

        // ensure that our name is up to date
        data.names.insert(user_id, user_name);

        // select the first channel if none is selected
        if data.channels.state.selected().is_none() && !data.channels.items.is_empty() {
            data.channels.state.select(Some(0));
        }

        Ok(data)
    }
}

impl JsonStorage {
    /// Create a new json storage at the data path.
    ///
    /// As a `Storage`, it will save the app data into the data path. When loading, json storage
    /// will try to load the data from the data path. However, if it does not exist and a fallback
    /// data path is provided, it will also try to load the data from the  fallback path.
    pub fn new(data_path: PathBuf, fallback_data_path: Option<PathBuf>) -> Self {
        Self {
            data_path,
            fallback_data_path,
        }
    }

    fn save_to(data: &AppData, data_path: impl AsRef<Path>) -> anyhow::Result<()> {
        let f = std::io::BufWriter::new(File::create(data_path)?);
        serde_json::to_writer(f, data)?;
        Ok(())
    }

    fn load_app_data_impl(&self) -> anyhow::Result<AppData> {
        let mut data_path = &self.data_path;
        if !data_path.exists() {
            // try also to load from a fallback (legacy) data path
            if let Some(fallback_data_path) = self.fallback_data_path.as_ref() {
                data_path = fallback_data_path;
            }
        }

        // if data file exists, be conservative and fail rather than overriding and losing the messages
        if data_path.exists() {
            Self::load_app_data_from(&data_path).with_context(|| {
                format!(
                    "failed to load stored data from '{}':\n\
            This might happen due to incompatible data model when Gurk is upgraded.\n\
            Please consider to backup your messages and then remove the store.",
                    data_path.display()
                )
            })
        } else {
            Ok(Self::load_app_data_from(data_path).unwrap_or_default())
        }
    }

    fn load_app_data_from(data_path: impl AsRef<Path>) -> anyhow::Result<AppData> {
        info!("loading app data from: {}", data_path.as_ref().display());
        let f = BufReader::new(File::open(data_path)?);
        let mut data: AppData = serde_json::from_reader(f)?;
        data.input_cursor = data.input.len();
        data.input_cursor_chars = data.input.width();
        Ok(data)
    }
}

#[cfg(test)]
pub mod test {
    use super::Storage;

    use crate::app::AppData;

    /// In-memory storage used for testing.
    pub struct InMemoryStorage {}

    impl InMemoryStorage {
        pub fn new() -> Self {
            Self {}
        }
    }

    impl Storage for InMemoryStorage {
        fn save_app_data(&self, _data: &crate::app::AppData) -> anyhow::Result<()> {
            Ok(())
        }

        fn load_app_data(&self, user_id: uuid::Uuid, user_name: String) -> anyhow::Result<AppData> {
            Ok(AppData {
                names: IntoIterator::into_iter([(user_id, user_name)]).collect(),
                ..Default::default()
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        app::{Channel, ChannelId, TypingSet},
        util::StatefulList,
    };

    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_json_storage_load_existing_app_data() -> anyhow::Result<()> {
        let user_id = Uuid::new_v4();
        let user_name = "Tyler Durden".to_string();
        let app_data = AppData {
            input: "some input".to_string(),
            input_cursor: 10,
            input_cursor_chars: 10,
            names: [(user_id, user_name.clone())].iter().cloned().collect(),
            ..Default::default()
        };

        let file = NamedTempFile::new()?;
        let storage = JsonStorage::new(file.path().to_owned(), None);
        storage.save_app_data(&app_data)?;
        let loaded_app_data = storage.load_app_data(user_id, user_name)?;

        assert_eq!(loaded_app_data, app_data);
        assert_eq!(loaded_app_data.channels.state.selected(), None);

        Ok(())
    }

    #[test]
    fn test_json_storage_load_non_existent_app_data() -> anyhow::Result<()> {
        let data_path = PathBuf::from("/tmp/some-non-existent-file.json");

        let storage = JsonStorage::new(data_path, None);

        let user_id = Uuid::new_v4();
        let user_name = "Tyler Durden".to_string();
        let app_data = storage.load_app_data(user_id, user_name.clone())?;

        assert_eq!(
            app_data,
            AppData {
                names: [(user_id, user_name)].iter().cloned().collect(),
                ..Default::default()
            }
        );

        Ok(())
    }

    #[test]
    fn test_json_storage_load_app_data_from_fallback() -> anyhow::Result<()> {
        let user_id = Uuid::new_v4();
        let user_name = "Tyler Durden".to_string();
        let app_data = AppData {
            input: "some input".to_string(),
            input_cursor: 10,
            input_cursor_chars: 10,
            names: [(user_id, user_name.clone())].iter().cloned().collect(),
            ..Default::default()
        };

        let data_path = PathBuf::from("/tmp/some-non-existent-file.json");
        let fallback_data_path = NamedTempFile::new()?;
        JsonStorage::save_to(&app_data, fallback_data_path.path())?;

        let storage = JsonStorage::new(data_path, Some(fallback_data_path.path().to_owned()));

        let loaded_app_data = storage.load_app_data(user_id, user_name)?;

        assert_eq!(loaded_app_data, app_data);

        Ok(())
    }

    #[test]
    fn test_json_storage_selected_channel() -> anyhow::Result<()> {
        let user_id = Uuid::new_v4();
        let user_name = "Tyler Durden".to_string();
        let app_data = AppData {
            input: "some input".to_string(),
            input_cursor: 10,
            input_cursor_chars: 10,
            names: [(user_id, user_name.clone())].iter().cloned().collect(),
            channels: StatefulList::with_items(vec![Channel {
                id: ChannelId::User(user_id),
                name: user_name.clone(),
                group_data: None,
                messages: Default::default(),
                unread_messages: 0,
                typing: TypingSet::SingleTyping(false),
            }]),
        };

        let file = NamedTempFile::new()?;
        let storage = JsonStorage::new(file.path().to_owned(), None);
        storage.save_app_data(&app_data)?;
        let app_data = storage.load_app_data(user_id, user_name)?;

        assert_eq!(app_data.channels.state.selected(), Some(0));

        Ok(())
    }
}

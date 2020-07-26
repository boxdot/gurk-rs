use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tui::widgets::ListState;

#[derive(Serialize, Deserialize)]
pub struct StatefulList<T> {
    #[serde(
        default,
        serialize_with = "serialize_list_state",
        deserialize_with = "deserialize_list_state"
    )]
    pub state: ListState,
    pub items: Vec<T>,
}

impl<T> StatefulList<T> {
    pub fn with_items(items: Vec<T>) -> StatefulList<T> {
        StatefulList {
            state: ListState::default(),
            items,
        }
    }

    pub fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i + 1 >= self.items.len() {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }
}

fn serialize_list_state<S>(list_state: &ListState, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    list_state.selected().serialize(serializer)
}

fn deserialize_list_state<'de, D>(deserializer: D) -> Result<ListState, D::Error>
where
    D: Deserializer<'de>,
{
    let selected = Option::<usize>::deserialize(deserializer)?;
    let mut list_state = ListState::default();
    list_state.select(selected);
    Ok(list_state)
}

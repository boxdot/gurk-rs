//! Here are all shortcuts documented

pub struct ShortCut {
    pub event: &'static str,
    pub description: &'static str,
}

pub static SHORTCUTS: &[ShortCut] = &[
    ShortCut {
        event: "F1",
        description: "Toggle help panel.",
    },
    ShortCut {
        event: "Tab",
        description: "When a message is selected, proceeds to try and read an emoji code (e.g. `:blush:`) from the input box and send the corresponding reaction on the selected message.",
    },
    ShortCut {
        event: "ALT+Enter",
        description: "Add a newline when editing a message.",
    },
    ShortCut {
        event: "Alt+Tab",
        description: "Switch between the message input box and the search bar.",
    },
];

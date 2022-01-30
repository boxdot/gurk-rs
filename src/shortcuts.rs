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
        description: "Sends emoji from input line as reaction on selected message.",
    },
    ShortCut {
        event: "Alt+Enter",
        description: "Add newline.",
    },
    ShortCut {
        event: "Alt+Tab",
        description: "Switch between message input box and search bar.",
    },
];

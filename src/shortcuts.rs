//! Here are all shortcuts documented

pub struct ShortCut {
    pub event: &'static str,
    pub description: &'static str,
}

pub static SHORTCUTS: &[ShortCut] = &[
    ShortCut {
        event: "f1",
        description: "Toggle help panel.",
    },
    ShortCut {
        event: "tab",
        description: "Sends emoji from input line as reaction on selected message.",
    },
    ShortCut {
        event: "alt+enter",
        description: "Add newline.",
    },
    ShortCut {
        event: "alt+tab",
        description: "Switch between message input box and search bar.",
    },
    ShortCut {
        event: "ctrl+w / ctrl+backspace / alt+backspace",
        description: "Delete last word.",
    },
    ShortCut {
        event: "enter, when input box empty",
        description: "Open URL from selected message.",
    },
    ShortCut {
        event: "enter, otherwise",
        description: "Send message.",
    },
    ShortCut {
        event: "alt+f / alt+Right / ctrl+Right",
        description: "Move forward one word.",
    },
    ShortCut {
        event: "alt+b / alt+Left / ctrl+Left",
        description: "Move backward one word.",
    },
    ShortCut {
        event: "ctrl+a / home",
        description: "Move cursor to the beginning of the text.",
    },
    ShortCut {
        event: "ctrl+e / end",
        description: "Move cursor the the end of the text.",
    },
    ShortCut {
        event: "Esc",
        description: "Reset message selection.",
    },
    ShortCut {
        event: "alt+Up / PgUp",
        description: "Select previous message.",
    },
    ShortCut {
        event: "alt+Down / PgDown",
        description: "Select next message.",
    },
    ShortCut {
        event: "ctrl+j / Up",
        description: "Select previous channel.",
    },
    ShortCut {
        event: "ctrl+k / Down",
        description: "Select next channel.",
    },
];

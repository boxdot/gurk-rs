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
        description: "Switch between single-line and multi-line modes.",
    },
    ShortCut {
        event: "ctrl+p",
        description: "Open pop-up for selecting a channel",
    },
    ShortCut {
        event: "ctrl+w / ctrl+backspace / alt+backspace",
        description: "Delete last word.",
    },
    ShortCut {
        event: "enter, when input box empty in single-line mode",
        description: "Open URL from selected message.",
    },
    ShortCut {
        event: "enter, single-line mode",
        description: "Send message.",
    },
    ShortCut {
        event: "enter, multi-line mode",
        description: "New line message.",
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
        description: "Reset message selection / Close popup.",
    },
    ShortCut {
        event: "alt+Up / alt+k / PgUp",
        description: "Select previous message.",
    },
    ShortCut {
        event: "alt+Down / alt+j / PgDown",
        description: "Select next message.",
    },
    ShortCut {
        event: "ctrl+j / Up, single-line mode",
        description: "Select previous channel.",
    },
    ShortCut {
        event: "ctrl+j / Up, multi-line mode",
        description: "Previous line",
    },
    ShortCut {
        event: "ctrl+k / Down, single-line mode",
        description: "Select next channel.",
    },
    ShortCut {
        event: "ctrl+k / Down, multi-line mode",
        description: "Next line",
    },
    ShortCut {
        event: "alt+y",
        description: "Copy selected message to clipboard",
    },
];

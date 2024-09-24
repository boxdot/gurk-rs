use std::collections::HashMap;

use crokey::KeyCombination;
use serde::{Deserialize, Serialize};

pub type KeybindingConfig = HashMap<KeyCombination, String>;
pub type ModeKeybindingConfig = HashMap<WindowMode, KeybindingConfig>;
pub type Keybinding = HashMap<KeyCombination, Command>;
pub type ModeKeybinding = HashMap<WindowMode, Keybinding>;

#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MoveDirection {
    Previous,
    Next,
}

impl MoveDirection {
    fn parse(dir: &str) -> Result<Self, CommandParseError> {
        match dir {
            "previous" => Ok(Self::Previous),
            "next" => Ok(Self::Next),
            _ => Err(CommandParseError::BadEnumArg {
                arg: dir.to_string(),
                accept: vec!["previous".into(), "next".into()],
                optional: false,
            }),
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MoveAmountText {
    Character,
    Word,
    Line,
    // Sentence,
}

impl MoveAmountText {
    fn parse(amount: &str) -> Result<Self, CommandParseError> {
        match amount {
            "character" | "c" => Ok(Self::Character),
            "word" | "w" => Ok(Self::Word),
            "line" | "l" => Ok(Self::Line),
            _ => Err(CommandParseError::BadEnumArg {
                arg: amount.to_string(),
                accept: vec!["character".into(), "word".into(), "line".into()],
                optional: false,
            }),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MoveAmountVisual {
    Entry,
    // HalfScreen,
    // Screen,
}

impl MoveAmountVisual {
    fn parse(amount: &str) -> Result<Self, CommandParseError> {
        match amount {
            "entry" => Ok(Self::Entry),
            // "half_screen" => Ok(Self::HalfScreen),
            // "screen" => Ok(Self::Screen),
            _ => Err(CommandParseError::BadEnumArg {
                arg: amount.to_string(),
                accept: vec!["entry".into()],
                optional: false,
            }),
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MessageSelector {
    Selected,
    Marked,
}

impl MessageSelector {
    fn parse(input: &str) -> Result<Self, CommandParseError> {
        match input {
            "selected" => Ok(Self::Selected),
            "marked" => Ok(Self::Marked),
            _ => Err(CommandParseError::BadEnumArg {
                arg: input.to_string(),
                accept: vec!["selected".into(), "marked".into()],
                optional: false,
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash, Copy)]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    Anywhere,
    Help,
    ChannelModal,
    Multiline,
    MessageSelected,
    Normal,
}

#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    /// Toggle help panel
    Help,
    /// Quit application
    Quit,
    /// Open pop-up for selecting a channel
    ToggleChannelModal,
    /// Switch between single-line and multi-line modes.
    ToggleMultiline,
    /// Sends emoji from input line as reaction on selected message.
    React,
    /// Move forward/backward one character/word/line
    MoveText(MoveDirection, MoveAmountText),
    // MoveChannel(MoveDirectionVert, MoveAmountVisual),
    /// Select next/previous channel in sidebar
    SelectChannel(MoveDirection),
    /// Select next/previous channel in channel modal
    SelectChannelModal(MoveDirection),
    /// Select next/previous message
    SelectMessage(MoveDirection, MoveAmountVisual),
    /// Delete to the end of the line.
    KillLine,
    /// Delete from the start to the end of the line.
    KillWholeLine,
    /// Delete to the start of the line.
    KillBackwardLine,
    /// Delete last word.
    KillWord,
    /// Copy selected message to clipboard
    CopyMessage(MessageSelector),
    /// Move cursor to the beginning of the text.
    BeginningOfLine,
    /// Move cursor the the end of the text.
    EndOfLine,
    /// Delete previous character.
    DeleteCharacter(MoveDirection),
    /// Edit selected message
    EditMessage,
    /// Try to open the first url in the selected message
    OpenUrl,
    // ReplyMessage,
    // DeleteMessage,
}

#[derive(Clone, Debug)]
pub enum CommandParseError {
    NoSuchCommand {
        cmd: String,
    },
    InsufficientArgs {
        cmd: String,
        hint: Option<String>,
    },
    BadEnumArg {
        arg: String,
        accept: Vec<String>,
        optional: bool,
    },
    ArgParseError {
        arg: String,
        err: String,
    },
}

pub fn parse(input: &str) -> Result<Command, CommandParseError> {
    let words: Vec<_> = input.split_whitespace().collect();
    use CommandParseError as E;

    if let Some((cmd, args)) = words.split_first() {
        match *cmd {
            "help" => Ok(Command::Help),
            "quit" => Ok(Command::Quit),
            "toggle_channel_modal" => Ok(Command::ToggleChannelModal),
            "react" => Ok(Command::React),
            "move_text" => {
                let usage = E::InsufficientArgs {
                    cmd: cmd.to_string(),
                    hint: Some("previous|next character|word|line".into()),
                };
                let &dir = args.first().ok_or(usage.clone())?;
                let &amount = args.get(1).ok_or(usage)?;
                let dir = MoveDirection::parse(dir)?;
                let amount = MoveAmountText::parse(amount)?;
                Ok(Command::MoveText(dir, amount))
            }
            // MoveDirectionVert, MoveAmountVisual
            "select_channel" => {
                let usage = E::InsufficientArgs {
                    cmd: cmd.to_string(),
                    hint: Some("previous|next".into()),
                };
                let &dir = args.first().ok_or(usage)?;
                let dir = MoveDirection::parse(dir)?;
                Ok(Command::SelectChannel(dir))
            }
            "select_channel_modal" => {
                let usage = E::InsufficientArgs {
                    cmd: cmd.to_string(),
                    hint: Some("previous|next".into()),
                };
                let &dir = args.first().ok_or(usage)?;
                let dir = MoveDirection::parse(dir)?;
                Ok(Command::SelectChannelModal(dir))
            }
            "select_message" => {
                let usage = E::InsufficientArgs {
                    cmd: cmd.to_string(),
                    hint: Some("previous|next entry".into()),
                };
                let &dir = args.first().ok_or(usage.clone())?;
                let &amount = args.get(1).ok_or(usage)?;
                let dir = MoveDirection::parse(dir)?;
                let amount = MoveAmountVisual::parse(amount)?;
                Ok(Command::SelectMessage(dir, amount))
            }
            "kill_line" => Ok(Command::KillLine),
            "kill_whole_line" => Ok(Command::KillWholeLine),
            "kill_backward_line" => Ok(Command::KillBackwardLine),
            "kill_word" => Ok(Command::KillWord),
            "copy_message" => Ok(Command::CopyMessage(MessageSelector::parse(
                args.first().unwrap_or(&"selected"),
            )?)),
            "beginning_of_line" => Ok(Command::BeginningOfLine),
            "end_of_line" => Ok(Command::EndOfLine),
            "delete_character" => {
                let usage = E::InsufficientArgs {
                    cmd: cmd.to_string(),
                    hint: Some("previous|next".into()),
                };
                let &dir = args.first().ok_or(usage.clone())?;
                let dir = MoveDirection::parse(dir)?;
                Ok(Command::DeleteCharacter(dir))
            }
            "edit_message" => Ok(Command::EditMessage),
            "open_url" => Ok(Command::OpenUrl),
            // "reply_message" => Ok(Command::ReplyMessage),
            // "delete_message" => Ok(Command::DeleteMessage),
            _ => Err(CommandParseError::NoSuchCommand {
                cmd: cmd.to_string(),
            }),
        }
    } else {
        Err(CommandParseError::NoSuchCommand { cmd: input.into() })
    }
}

pub const DEFAULT_KEYBINDINGS: &str = r#"
[anywhere]
F1 = "help"
ctrl-c = "quit"

[normal]
ctrl-p = "toggle_channel_modal"
ctrl-left = "move_text previous character"
ctrl-right = "move_text next character"
left = "move_text previous character"
right = "move_text next character"
alt-left = "move_text previous word"
alt-right = "move_text next word"
alt-up = "select_message previous entry"
alt-down = "select_message next entry"
alt-j = "select_message next entry"
alt-k = "select_message previous entry"
pagedown = "select_message next entry"
pageup = "select_message previous entry"
alt-f = "move_text next word"
ctrl-f = "move_text next character"
alt-b = "move_text previous word"
ctrl-b = "move_text previous character"
ctrl-u = "kill_backward_line"
ctrl-w = "kill_word"
ctrl-j = "select_channel next"
ctrl-k = "select_channel previous"
down = "select_channel next"
up = "select_channel previous"
alt-backspace = "kill_word"
home = "beginning_of_line"
ctrl-a = "beginning_of_line"
end = "end_of_line"
ctrl-e = "end_of_line"
backspace = "delete_character previous"
tab = "react"

[message_selected]
alt-y = "copy_message selected"
ctrl-e = "edit_message"

[channel_modal]
down = "select_channel_modal next"
up = "select_channel_modal previous"
ctrl-j = "select_channel_modal next"
ctrl-k = "select_channel_modal previous"

[multiline]
down = "move_text next line"
up = "move_text previous line"
ctrl-j = "move_text next line"
ctrl-k = "move_text previous line"

[help]
"#;

#[cfg(test)]
mod tests {
    use toml;

    use super::{get_keybindings, ModeKeybindingConfig, DEFAULT_KEYBINDINGS};

    #[test]
    fn default_keybindings_deserialize() {
        let _keybindings: ModeKeybindingConfig = toml::from_str(DEFAULT_KEYBINDINGS).unwrap();
    }

    #[test]
    fn default_keybindings_parse() {
        get_keybindings(&ModeKeybindingConfig::new(), true).unwrap();
    }

    #[test]
    fn custom_keybindings() {
        let bindings: ModeKeybindingConfig =
            toml::from_str("[normal]\n  F1 = \"\"\n  ctrl-h = \"help\"\n").unwrap();
        get_keybindings(&bindings, true).unwrap();
        get_keybindings(&bindings, false).unwrap();
    }
}

fn merge_keybinding_configs(mkb1: &mut ModeKeybindingConfig, mkb2: ModeKeybindingConfig) {
    for (mode, kb2) in mkb2 {
        mkb1.entry(mode).or_insert(HashMap::new()).extend(kb2);
    }
}

pub fn get_keybindings(
    keybinding_config: &ModeKeybindingConfig,
    default_bindings: bool,
) -> Result<ModeKeybinding, CommandParseError> {
    let mut keybindings = if default_bindings {
        toml::from_str(DEFAULT_KEYBINDINGS).unwrap()
    } else {
        HashMap::new()
    };
    merge_keybinding_configs(&mut keybindings, keybinding_config.clone());
    parse_mode_keybindings(&keybindings)
}

fn parse_mode_keybindings(
    mkbc: &ModeKeybindingConfig,
) -> Result<ModeKeybinding, CommandParseError> {
    let mut mode_keybindings = ModeKeybinding::new();
    for (&mode, kbc) in mkbc {
        mode_keybindings.insert(mode, parse_keybindings(kbc)?);
    }
    Ok(mode_keybindings)
}

fn parse_keybindings(kbc: &KeybindingConfig) -> Result<Keybinding, CommandParseError> {
    let mut keybindings = Keybinding::new();
    for (&k, cmd) in kbc {
        // Allows removing bindings
        if !cmd.trim().is_empty() {
            keybindings.insert(k, parse(cmd)?);
        }
    }
    Ok(keybindings)
}

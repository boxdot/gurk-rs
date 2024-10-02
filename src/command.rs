use std::collections::HashMap;
use std::str::FromStr;

use crokey::KeyCombination;
use serde::{Deserialize, Serialize};
use strum::{EnumIter, EnumProperty, EnumString, VariantNames};

pub type KeybindingConfig = HashMap<KeyCombination, String>;
pub type ModeKeybindingConfig = HashMap<WindowMode, KeybindingConfig>;
pub type Keybinding = HashMap<KeyCombination, Command>;
pub type ModeKeybinding = HashMap<WindowMode, Keybinding>;

#[derive(
    Clone,
    Default,
    Debug,
    Eq,
    PartialEq,
    strum_macros::Display,
    strum_macros::VariantNames,
    EnumString,
)]
#[strum(serialize_all = "snake_case")]
pub enum Widget {
    #[default]
    Help,
}

#[derive(
    Clone,
    Default,
    Debug,
    Eq,
    PartialEq,
    strum_macros::Display,
    strum_macros::VariantNames,
    EnumString,
)]
#[strum(serialize_all = "snake_case")]
pub enum DirectionVertical {
    #[default]
    Up,
    Down,
}

#[derive(
    Clone,
    Default,
    Debug,
    Eq,
    PartialEq,
    strum_macros::Display,
    strum_macros::VariantNames,
    EnumString,
)]
#[strum(serialize_all = "snake_case")]
pub enum MoveDirection {
    #[default]
    Previous,
    Next,
}

#[derive(
    Clone,
    Default,
    Debug,
    Eq,
    PartialEq,
    strum_macros::Display,
    strum_macros::VariantNames,
    EnumString,
)]
#[strum(serialize_all = "snake_case")]
pub enum MoveAmountText {
    #[default]
    Character,
    Word,
    Line,
    // Sentence,
}

#[derive(
    Clone,
    Default,
    Debug,
    Eq,
    PartialEq,
    strum_macros::Display,
    strum_macros::VariantNames,
    EnumString,
)]
#[strum(serialize_all = "snake_case")]
pub enum MoveAmountVisual {
    #[default]
    Entry,
    // HalfScreen,
    // Screen,
}

#[derive(
    Clone,
    Default,
    Debug,
    Eq,
    PartialEq,
    strum_macros::Display,
    strum_macros::VariantNames,
    EnumString,
)]
#[strum(serialize_all = "snake_case")]
pub enum MessageSelector {
    #[default]
    Selected,
    Marked,
}

#[derive(
    Clone,
    Debug,
    Eq,
    PartialEq,
    Deserialize,
    Serialize,
    Hash,
    Copy,
    strum_macros::Display,
    strum_macros::VariantNames,
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    Anywhere,
    Help,
    ChannelModal,
    Multiline,
    MessageSelected,
    Normal,
}

#[derive(
    Clone,
    Debug,
    Eq,
    PartialEq,
    strum_macros::Display,
    strum_macros::VariantNames,
    EnumString,
    EnumIter,
    EnumProperty,
)]
#[strum(serialize_all = "snake_case")]
pub enum Command {
    #[strum(props(desc = "Toggle help panel"))]
    Help,
    #[strum(props(desc = "Quit application"))]
    Quit,
    #[strum(props(desc = "Open pop-up for selecting a channel"))]
    ToggleChannelModal,
    #[strum(props(desc = "Switch between single-line and multi-line modes."))]
    ToggleMultiline,
    #[strum(props(desc = "Sends emoji from input line as reaction on selected message."))]
    React,
    #[strum(props(desc = "Scroll a widget", usage = "scroll help up|down entry"))]
    #[strum(serialize = "scroll", to_string = "scroll {0} {1} {2}")]
    Scroll(Widget, DirectionVertical, MoveAmountVisual),
    #[strum(props(
        desc = "Move forward/backward one character/word/line",
        usage = "move_text previous|next character|word|line"
    ))]
    #[strum(serialize = "move_text", to_string = "move_text {0} {1}")]
    MoveText(MoveDirection, MoveAmountText),
    // MoveChannel(MoveDirectionVert, MoveAmountVisual),
    #[strum(props(
        desc = "Select next/previous channel in sidebar",
        usage = "select_channel previous|next"
    ))]
    #[strum(serialize = "select_channel", to_string = "select_channel {0}")]
    SelectChannel(MoveDirection),
    #[strum(props(
        desc = "Select next/previous channel in channel modal",
        usage = "select_channel_modal previous|next"
    ))]
    #[strum(
        serialize = "select_channel_modal",
        to_string = "select_channel_modal {0}"
    )]
    SelectChannelModal(MoveDirection),
    #[strum(props(
        desc = "Select next/previous message",
        usage = "select_message previous|next entry"
    ))]
    #[strum(serialize = "select_message", to_string = "select_message {0} {1}")]
    SelectMessage(MoveDirection, MoveAmountVisual),
    #[strum(props(desc = "Delete to the end of the line."))]
    KillLine,
    #[strum(props(desc = "Delete from the start to the end of the line."))]
    KillWholeLine,
    #[strum(props(desc = "Delete to the start of the line."))]
    KillBackwardLine,
    #[strum(props(desc = "Delete last word."))]
    KillWord,
    #[strum(props(
        desc = "Copy selected message to clipboard",
        usage = "copy_message selected"
    ))]
    #[strum(serialize = "copy_message", to_string = "copy_message {0}")]
    CopyMessage(MessageSelector),
    #[strum(props(desc = "Move cursor to the beginning of the text."))]
    BeginningOfLine,
    #[strum(props(desc = "Move cursor the the end of the text."))]
    EndOfLine,
    #[strum(props(
        desc = "Delete previous character.",
        usage = "delete_character previous|next"
    ))]
    #[strum(serialize = "delete_character", to_string = "delete_character {0}")]
    DeleteCharacter(MoveDirection),
    #[strum(props(desc = "Edit selected message"))]
    EditMessage,
    #[strum(props(desc = "Try to open the first url in the selected message"))]
    OpenUrl,
    // ReplyMessage,
    // DeleteMessage,
}

#[derive(Clone, Debug)]
pub enum CommandParseError {
    NoSuchCommand {
        cmd: String,
        accept: &'static [&'static str],
    },
    InsufficientArgs {
        cmd: String,
        hint: Option<String>,
    },
    BadEnumArg {
        arg: String,
        accept: &'static [&'static str],
        optional: bool,
    },
}

fn parse(input: &str) -> Result<Command, CommandParseError> {
    let words: Vec<_> = input.trim().split_whitespace().collect();
    use CommandParseError as E;

    let (cmd_str, args) = words.split_first().unwrap();
    let cmd = Command::from_str(cmd_str).map_err(|_e| E::NoSuchCommand {
        cmd: cmd_str.to_string(),
        accept: Command::VARIANTS,
    })?;
    match cmd {
        Command::Scroll(_, _, _) => {
            let usage = E::InsufficientArgs {
                cmd: cmd_str.to_string(),
                hint: Some(
                    [
                        Widget::VARIANTS.join("|"),
                        DirectionVertical::VARIANTS.join("|"),
                        MoveAmountVisual::VARIANTS.join("|"),
                    ]
                    .join(" "),
                ),
            };
            let widget = args.first().ok_or(usage.clone())?;
            let dir = args.get(1).ok_or(usage.clone())?;
            let amount = args.get(2).ok_or(usage)?;
            let widget = Widget::from_str(widget).map_err(|_e| E::BadEnumArg {
                arg: widget.to_string(),
                accept: Widget::VARIANTS,
                optional: false,
            })?;
            let dir = DirectionVertical::from_str(dir).map_err(|_e| E::BadEnumArg {
                arg: dir.to_string(),
                accept: DirectionVertical::VARIANTS,
                optional: false,
            })?;
            let amount = MoveAmountVisual::from_str(amount).map_err(|_e| E::BadEnumArg {
                arg: amount.to_string(),
                accept: MoveAmountVisual::VARIANTS,
                optional: false,
            })?;
            Ok(Command::Scroll(widget, dir, amount))
        }
        Command::MoveText(_, _) => {
            let usage = E::InsufficientArgs {
                cmd: cmd_str.to_string(),
                hint: Some(
                    [
                        MoveDirection::VARIANTS.join("|"),
                        MoveAmountText::VARIANTS.join("|"),
                    ]
                    .join(" "),
                ),
            };
            let direction = args.first().ok_or(usage.clone())?;
            let amount = args.get(1).ok_or(usage)?;
            let direction = MoveDirection::from_str(direction).map_err(|_e| E::BadEnumArg {
                arg: direction.to_string(),
                accept: MoveDirection::VARIANTS,
                optional: false,
            })?;
            let amount = MoveAmountText::from_str(amount).map_err(|_e| E::BadEnumArg {
                arg: amount.to_string(),
                accept: MoveAmountText::VARIANTS,
                optional: false,
            })?;
            Ok(Command::MoveText(direction, amount))
        }
        Command::SelectChannel(_) => {
            let usage = E::InsufficientArgs {
                cmd: cmd_str.to_string(),
                hint: Some(MoveDirection::VARIANTS.join("|")),
            };
            let direction = args.first().ok_or(usage)?;
            let direction = MoveDirection::from_str(direction).map_err(|_e| E::BadEnumArg {
                arg: direction.to_string(),
                accept: MoveDirection::VARIANTS,
                optional: false,
            })?;
            Ok(Command::SelectChannel(direction))
            // Ok(Command::SelectChannel(MoveDirection::from_str(args.first().unwrap_or(&""))?))
        }
        Command::SelectChannelModal(_) => {
            let usage = E::InsufficientArgs {
                cmd: cmd_str.to_string(),
                hint: Some(MoveDirection::VARIANTS.join("|")),
            };
            let direction = args.first().ok_or(usage)?;
            let direction = MoveDirection::from_str(direction).map_err(|_e| E::BadEnumArg {
                arg: direction.to_string(),
                accept: MoveDirection::VARIANTS,
                optional: false,
            })?;
            Ok(Command::SelectChannelModal(direction))
            // Ok(Command::SelectChannelModal(MoveDirection::from_str(args.first().unwrap_or(&""))?))
        }
        Command::SelectMessage(_, _) => {
            let usage = E::InsufficientArgs {
                cmd: cmd_str.to_string(),
                hint: Some(
                    [
                        MoveDirection::VARIANTS.join("|"),
                        MoveAmountVisual::VARIANTS.join("|"),
                    ]
                    .join(" "),
                ),
            };
            let direction = args.first().ok_or(usage.clone())?;
            let amount = args.get(1).ok_or(usage)?;
            let direction = MoveDirection::from_str(direction).map_err(|_e| E::BadEnumArg {
                arg: direction.to_string(),
                accept: MoveDirection::VARIANTS,
                optional: false,
            })?;
            let amount = MoveAmountVisual::from_str(amount).map_err(|_e| E::BadEnumArg {
                arg: amount.to_string(),
                accept: MoveAmountText::VARIANTS,
                optional: false,
            })?;
            Ok(Command::SelectMessage(direction, amount))
        }
        Command::CopyMessage(_) => {
            let usage = E::InsufficientArgs {
                cmd: cmd_str.to_string(),
                hint: Some(MessageSelector::VARIANTS.join("|")),
            };
            let selector = args.first().ok_or(usage)?;
            let selector = MessageSelector::from_str(selector).map_err(|_e| E::BadEnumArg {
                arg: selector.to_string(),
                accept: MessageSelector::VARIANTS,
                optional: false,
            })?;
            Ok(Command::CopyMessage(selector))
            // Ok(Command::CopyMessage(MessageSelector::from_str(args.first().unwrap_or(&""))?))
        }
        _ => Ok(cmd),
    }
}

pub const DEFAULT_KEYBINDINGS: &str = r#"
[anywhere]
F1 = "help"
ctrl-c = "quit"

[normal]
ctrl-p = "toggle_channel_modal"
alt-enter = "toggle_multiline"
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
esc = "toggle_channel_modal"
ctrl-p = "toggle_channel_modal"
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
esc = "help"
ctrl-j = "scroll help down entry"
ctrl-k = "scroll help up entry"
down = "scroll help down entry"
up = "scroll help up entry"
pagedown = "scroll help down entry"
pageup = "scroll help up entry"
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

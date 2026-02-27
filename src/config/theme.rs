use ratatui::{
    layout::HorizontalAlignment,
    style::Style,
    text::Line,
    widgets::{Block, BorderType, List, ListItem, Padding},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeConfig {
    #[serde(default)]
    pub channel_popup: ChannelPopupConfig,
    #[serde(default = "ListThemeConfig::default_channel_list")]
    pub channels: ListThemeConfig,
    #[serde(default = "BlockConfig::default_input")]
    pub input: BlockConfig,
    #[serde(default)]
    pub messages: MessagesThemeConfig,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            channel_popup: ChannelPopupConfig::default(),
            channels: ListThemeConfig::default_channel_list(),
            input: BlockConfig::default_input(),
            messages: MessagesThemeConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelPopupConfig {
    pub input: BlockConfig,
    pub list: ListThemeConfig,
}

impl Default for ChannelPopupConfig {
    fn default() -> ChannelPopupConfig {
        ChannelPopupConfig {
            input: BlockConfig::default_channel_popup(),
            list: ListThemeConfig::default_style_unamed(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagesThemeConfig {
    #[serde(default = "BlockConfig::default_messages")]
    pub block: BlockConfig,
    #[serde(default)]
    pub receipts: ReceiptsConfig,
    #[serde(default = "default_time_style")]
    pub time: Style,
    #[serde(default)]
    pub list: ListThemeConfig,
    #[serde(default = "default_user_styles")]
    pub user_styles: Vec<Style>,
}

fn default_time_style() -> Style {
    Style::new().yellow()
}

impl Default for MessagesThemeConfig {
    fn default() -> Self {
        Self {
            block: BlockConfig::default_messages(),
            receipts: ReceiptsConfig::default(),
            list: ListThemeConfig::default_style_unamed(),
            user_styles: default_user_styles(),
            time: default_time_style(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ThemedText {
    pub text: String,
    pub style: Style,
}

impl ThemedText {
    pub fn new<S>(text: S, style: Style) -> Self
    where
        String: From<S>,
    {
        Self {
            text: text.into(),
            style,
        }
    }

    pub fn unstyled<S>(text: S) -> Self
    where
        String: From<S>,
    {
        Self::new(text, Style::new())
    }

    pub fn widget<'a>(&'a self) -> Line<'a> {
        Line::from(self.text.as_str()).style(self.style)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptsConfig {
    #[serde(default = "default_receipt_nothing")]
    pub nothing: ThemedText,
    #[serde(default = "default_receipt_sent")]
    pub sent: ThemedText,
    #[serde(default = "default_receipt_delivered")]
    pub delivered: ThemedText,
    #[serde(default = "default_receipt_read")]
    pub read: ThemedText,
}

impl Default for ReceiptsConfig {
    fn default() -> ReceiptsConfig {
        Self {
            nothing: default_receipt_nothing(),
            sent: default_receipt_sent(),
            delivered: default_receipt_delivered(),
            read: default_receipt_read(),
        }
    }
}

const RECEIPT_STYLE: Style = Style::new().yellow();

pub fn default_receipt_nothing() -> ThemedText {
    ThemedText::new(String::from("  "), RECEIPT_STYLE)
}
pub fn default_receipt_sent() -> ThemedText {
    ThemedText::new(String::from("○ "), RECEIPT_STYLE)
}
pub fn default_receipt_delivered() -> ThemedText {
    ThemedText::new(String::from("◉ "), RECEIPT_STYLE)
}
pub fn default_receipt_read() -> ThemedText {
    ThemedText::new(String::from("● "), RECEIPT_STYLE)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListThemeConfig {
    pub style: Style,
    pub highlight_style: Style,
    pub block: BlockConfig,
}

impl Default for ListThemeConfig {
    fn default() -> Self {
        Self::default_style_unamed()
    }
}

impl ListThemeConfig {
    fn default_style_unamed() -> Self {
        Self::default_style("")
    }

    fn default_style<S>(title: S) -> Self
    where
        String: From<S>,
    {
        Self {
            style: Style::new(),
            highlight_style: Style::new().reversed(),
            block: BlockConfig::unstyled(title),
        }
    }

    fn default_channel_list() -> Self {
        Self::default_style("Channels")
    }

    pub fn widget<'a, T>(&'a self, items: T) -> List<'a>
    where
        T: IntoIterator,
        <T as IntoIterator>::Item: Into<ListItem<'a>>,
    {
        List::new(items)
            .style(self.style)
            .highlight_style(self.highlight_style)
            .block(self.block.widget())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BlockConfig {
    pub border: BorderType,
    pub border_style: Style,
    pub title: BlockTitleConfig,
    pub padding: Padding,
}

impl BlockConfig {
    fn unstyled<S>(title: S) -> BlockConfig
    where
        String: From<S>,
    {
        BlockConfig {
            title: BlockTitleConfig::unstyled(title),
            ..Default::default()
        }
    }

    fn default_channel_popup() -> Self {
        Self::unstyled("Select channel")
    }

    fn default_input() -> BlockConfig {
        Self::unstyled("Input")
    }

    fn default_messages() -> BlockConfig {
        Self::unstyled("Messages")
    }

    pub fn widget<'a>(&'a self) -> Block<'a> {
        Block::new()
            .border_type(self.border)
            .border_style(self.border_style)
            .title(self.title.widget())
            .padding(self.padding)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BlockTitleConfig {
    #[serde(flatten)]
    pub themed_text: ThemedText,
    pub alignment: HorizontalAlignment,
}

impl BlockTitleConfig {
    pub fn new(themed_text: ThemedText, alignment: HorizontalAlignment) -> Self {
        Self {
            themed_text,
            alignment,
        }
    }

    pub fn unstyled<S>(title: S) -> Self
    where
        String: From<S>,
    {
        Self::new(ThemedText::unstyled(title), HorizontalAlignment::default())
    }

    pub fn widget<'a>(&'a self) -> Line<'a> {
        self.themed_text.widget().alignment(self.alignment)
    }
}

fn default_user_styles() -> Vec<Style> {
    vec![
        Style::new().red(),
        Style::new().green(),
        Style::new().yellow(),
        Style::new().blue(),
        Style::new().magenta(),
        Style::new().cyan(),
        Style::reset(),
    ]
}

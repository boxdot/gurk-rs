use ratatui::{
    layout::HorizontalAlignment,
    style::Style,
    text::{Line, Text},
    widgets::{Block, BorderType, Borders, List, ListItem, Padding, Paragraph},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeConfig {
    #[serde(default)]
    pub channel_popup: ChannelPopupConfig,
    #[serde(default = "ListThemeConfig::default_channel_list")]
    pub channels: ListThemeConfig,
    #[serde(default)]
    pub input: InputConfig,
    #[serde(default)]
    pub messages: MessagesThemeConfig,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            channel_popup: ChannelPopupConfig::default(),
            channels: ListThemeConfig::default_channel_list(),
            input: InputConfig::default(),
            messages: MessagesThemeConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelPopupConfig {
    #[serde(default = "InputConfig::default_channel_popup")]
    pub input: InputConfig,
    #[serde(default)]
    pub list: ListThemeConfig,
}

impl Default for ChannelPopupConfig {
    fn default() -> ChannelPopupConfig {
        ChannelPopupConfig {
            input: InputConfig::default_channel_popup(),
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

    pub fn line<'a>(&'a self) -> Line<'a> {
        Line::from(self.text.as_str()).style(self.style)
    }

    pub fn text<'a>(&'a self) -> Text<'a> {
        Text::from(self.text.as_str()).style(self.style)
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
    #[serde(default)]
    pub style: Style,
    #[serde(default = "default_highlight_style")]
    pub highlight_style: Style,
    #[serde(default)]
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
            highlight_style: default_highlight_style(),
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

fn default_highlight_style() -> Style {
    Style::new().reversed()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockConfig {
    #[serde(default = "default_block_border")]
    pub border: Option<BorderType>,
    #[serde(default)]
    pub border_style: Style,
    #[serde(default)]
    pub title: BlockTitleConfig,
    #[serde(default)]
    pub padding: Padding,
}

impl Default for BlockConfig {
    fn default() -> Self {
        Self {
            border: default_block_border(),
            border_style: Default::default(),
            title: Default::default(),
            padding: Default::default(),
        }
    }
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

    fn default_messages() -> BlockConfig {
        Self::unstyled("Messages")
    }

    pub fn widget<'a>(&'a self) -> Block<'a> {
        dbg!(self);
        let mut block = Block::new()
            .border_style(self.border_style)
            .title(self.title.widget())
            .padding(self.padding);
        if let Some(border) = self.border {
            block = block.border_type(border).borders(Borders::ALL);
        }
        block
    }
}

fn default_block_border() -> Option<BorderType> {
    Some(BorderType::Plain)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BlockTitleConfig {
    #[serde(flatten)]
    pub themed_text: ThemedText,
    #[serde(default)]
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
        self.themed_text.line().alignment(self.alignment)
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputConfig {
    pub block: BlockConfig,
    #[serde(default)]
    pub text: Style,
}

impl InputConfig {
    pub fn unstyled<S>(title: S) -> Self
    where
        String: From<S>,
    {
        Self {
            block: BlockConfig::unstyled(title),
            text: Style::new(),
        }
    }

    fn default_channel_popup() -> InputConfig {
        Self::unstyled("Select channel")
    }

    pub fn widget<'a, S>(&'a self, content: S) -> Paragraph<'a>
    where
        Text<'a>: From<S>,
    {
        Paragraph::new(Text::from(content).style(self.text)).block(self.block.widget())
    }
}

impl Default for InputConfig {
    fn default() -> Self {
        Self::unstyled("Input")
    }
}

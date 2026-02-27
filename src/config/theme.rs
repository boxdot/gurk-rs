use ratatui::{
    layout::{HorizontalAlignment, Rect},
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, List, ListItem, Padding, Paragraph},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeConfig {
    #[serde(default)]
    pub channel_popup: ChannelPopupConfig,
    #[serde(default = "ListThemeConfig::default_channel_list")]
    pub channels: ListThemeConfig,
    #[serde(default)]
    pub input: InputConfig,
    #[serde(default = "InputConfig::default_editing")]
    pub input_editing: InputConfig,
    #[serde(default = "InputConfig::default_editing_multiline")]
    pub input_editing_multiline: InputConfig,
    #[serde(default = "InputConfig::default_multiline")]
    pub input_multiline: InputConfig,
    #[serde(default)]
    pub messages: MessagesThemeConfig,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            channel_popup: ChannelPopupConfig::default(),
            channels: ListThemeConfig::default_channel_list(),
            input: InputConfig::default(),
            input_editing: InputConfig::default_editing(),
            input_editing_multiline: InputConfig::default_editing_multiline(),
            input_multiline: InputConfig::default_multiline(),
            messages: MessagesThemeConfig::default(),
        }
    }
}

impl ThemeConfig {
    pub fn input_config(&self, is_editing: bool, is_multiline: bool) -> &InputConfig {
        match (is_editing, is_multiline) {
            (true, true) => &self.input_editing_multiline,
            (true, false) => &self.input_editing,
            (false, true) => &self.input_multiline,
            (false, false) => &self.input,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
    pub user_styles: Vec<UserStyle>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Copy)]
#[serde(deny_unknown_fields)]
pub struct UserStyle {
    pub username: Style,
    #[serde(default)]
    pub message: Style,
}

impl UserStyle {
    pub fn from_color(color: Color) -> UserStyle {
        Self {
            username: Style::new().fg(color),
            message: Style::new(),
        }
    }

    pub fn logic_error() -> UserStyle {
        UserStyle {
            username: Style::new().magenta(),
            message: Style::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ThemedText {
    pub text: String,
    #[serde(default)]
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
        Line::styled(self.text.as_str(), self.style)
    }

    pub fn text<'a>(&'a self) -> Text<'a> {
        Text::styled(self.text.as_str(), self.style)
    }

    pub fn span(&self) -> Span<'_> {
        Span::styled(self.text.as_str(), self.style)
    }

    pub fn span_owned(&self) -> Span<'static> {
        Span::styled(self.text.clone(), self.style)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
        let mut block = Block::new()
            .border_style(self.border_style)
            .title(self.title.widget())
            .padding(self.padding);
        if let Some(border) = self.border {
            block = block.border_type(border).borders(Borders::ALL);
        }
        block
    }

    /// Appends a string to the title
    pub fn append_title(self, s: &str) -> Self {
        let mut this = self.clone();
        this.title.themed_text.text.push_str(s);
        this
    }

    fn border_width(&self) -> u16 {
        if self.border.is_some() { 1 } else { 0 }
    }

    /// Gets the area minus the padding and borders
    pub fn internal_area(&self, area: Rect) -> Rect {
        let x_offset = self.border_width() + self.padding.left;
        let y_offset = self.border_width() + self.padding.top;

        let m_height = y_offset + self.border_width() + self.padding.bottom;
        let height = area.height.saturating_sub(m_height);
        let m_width = x_offset + self.border_width() + self.padding.right;
        let width = area.width.saturating_sub(m_width);
        let x = area.x + x_offset;
        let y = area.y + y_offset;

        Rect::new(x, y, width, height)
    }
}

fn default_block_border() -> Option<BorderType> {
    Some(BorderType::Plain)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
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

fn default_user_styles() -> Vec<UserStyle> {
    vec![
        UserStyle {
            username: Style::new().red(),
            message: Style::new(),
        },
        UserStyle {
            username: Style::new().green(),
            message: Style::new(),
        },
        UserStyle {
            username: Style::new().yellow(),
            message: Style::new(),
        },
        UserStyle {
            username: Style::new().blue(),
            message: Style::new(),
        },
        UserStyle {
            username: Style::new().magenta(),
            message: Style::new(),
        },
        UserStyle {
            username: Style::new().cyan(),
            message: Style::new(),
        },
        UserStyle {
            username: Style::new(),
            message: Style::new(),
        },
    ]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

    fn default_editing() -> InputConfig {
        Self::unstyled("Input (Editing)")
    }

    fn default_editing_multiline() -> InputConfig {
        Self::unstyled("Input (Editing, Multiline)")
    }

    fn default_multiline() -> InputConfig {
        Self::unstyled("Input (Multiline)")
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

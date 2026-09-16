//! Stub color/face definitions consumed by the ui components. Plain
//! Rust (no iocraft) so the theme can be unit-tested in isolation; the
//! ui layer converts `Color` to `iocraft::Color`.

/// Stub palette of terminal colors. Some variants are not referenced
/// yet by the issue-01 faces (later views use the full set).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum Color {
    /// Terminal default (no explicit color).
    #[default]
    Default,
    Black,
    DarkGrey,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Grey,
    White,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Face {
    pub foreground: Color,
    pub background: Color,
    pub bold: bool,
}

impl Face {
    pub const fn new(foreground: Color, background: Color, bold: bool) -> Self {
        Self {
            foreground,
            background,
            bold,
        }
    }
}

/// All faces the ui can ask for. Issue 01 ships stub values; the
/// theme-selection config option maps onto this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub status_line: Face,
    pub status_line_active: Face,
    pub minibuffer: Face,
    pub prompt: Face,
    pub list_item: Face,
    pub list_item_selected: Face,
    pub view: Face,
    pub view_title: Face,
    pub preview: Face,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            status_line: Face::new(Color::Grey, Color::Blue, true),
            status_line_active: Face::new(Color::White, Color::Blue, true),
            minibuffer: Face::new(Color::Grey, Color::Black, false),
            prompt: Face::new(Color::Yellow, Color::Black, true),
            list_item: Face::new(Color::White, Color::Black, false),
            list_item_selected: Face::new(Color::Black, Color::Grey, true),
            view: Face::new(Color::White, Color::Black, false),
            view_title: Face::new(Color::Cyan, Color::Black, true),
            preview: Face::new(Color::Grey, Color::Black, false),
        }
    }
}

/// Process-wide current theme, set once at startup from config. The
/// ui components read it without plumbing the theme through props.
static CURRENT: std::sync::OnceLock<std::sync::Mutex<Theme>> = std::sync::OnceLock::new();

fn current_slot() -> &'static std::sync::Mutex<Theme> {
    CURRENT.get_or_init(|| std::sync::Mutex::new(Theme::default()))
}

pub fn set_current(theme: Theme) {
    *current_slot().lock().unwrap() = theme;
}

pub fn current() -> Theme {
    *current_slot().lock().unwrap()
}

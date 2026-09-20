//! All iocraft code lives in this module tree (plus `main.rs`, which
//! only starts/stops the render loop and restores the terminal). The
//! app layer (`src/app`) and the theme stay plain Rust.

pub mod blame_view;
pub mod commit_editor;
pub mod diff_view;
pub mod event_loop;
pub mod file_view;
pub mod home_view;
pub mod log_view;
pub mod magit_status;
pub mod picker;
pub mod results_view;
pub mod root;
pub mod rows_view;
pub mod tree;
pub mod transient_menu;
pub mod views;

use iocraft::prelude::*;

use crate::theme;

/// Convert a theme color to its iocraft color (single home for the
/// theme→iocraft mapping, keeping `theme.rs` free of iocraft imports).
pub(crate) fn color(c: theme::Color) -> Color {
    use theme::Color as TC;
    match c {
        TC::Default => Color::Grey,
        TC::Black => Color::Black,
        TC::DarkGrey => Color::DarkGrey,
        TC::Red => Color::Red,
        TC::Green => Color::Green,
        TC::Yellow => Color::Yellow,
        TC::Blue => Color::Blue,
        TC::Magenta => Color::Magenta,
        TC::Cyan => Color::Cyan,
        TC::Grey => Color::Grey,
        TC::White => Color::White,
    }
}

pub(crate) fn face_color(face: theme::Face) -> Color {
    color(face.foreground)
}

pub(crate) fn face_bg(face: theme::Face) -> Color {
    color(face.background)
}

pub(crate) fn face_weight(face: theme::Face) -> Weight {
    if face.bold {
        Weight::Bold
    } else {
        Weight::Normal
    }
}

/// Whether the terminal advertises 24-bit (truecolor) color. When set, the
/// selected-row bar emits its background as SGR `48;2;r;g;b` with the theme's
/// exact RGB instead of the 256-color palette index, which user terminal
/// themes can remap to near-background (plan-004 issue 05).
fn truecolor_enabled() -> bool {
    std::env::var("COLORTERM")
        .is_ok_and(|v| v.eq_ignore_ascii_case("truecolor") || v.eq_ignore_ascii_case("24bit"))
}

/// The theme's exact RGB for the selected-row bar's background color, matching
/// the xterm-256 palette index iocraft emits for that face (`Color::Blue` ->
/// `48;5;12`, nominal bright blue (0,0,255)). `None` when the color has no
/// truecolor mapping (falls back to the 256-color path).
fn bar_rgb(c: theme::Color) -> Option<(u8, u8, u8)> {
    use theme::Color as TC;
    Some(match c {
        TC::Blue => (0, 0, 255),
        _ => return None,
    })
}

/// The shared selected-row bar escape: under `COLORTERM=truecolor` the bar's
/// background is the theme's exact RGB (`48;2;r;g;b`); otherwise it falls back
/// to the 256-color path (`face_bg`). Used by every cursor-bar emitter so the
/// truecolor variant lives in one place (the face model is unchanged — only
/// the escape strategy gains the truecolor variant).
pub(crate) fn bar_bg(face: theme::Face) -> Color {
    if truecolor_enabled()
        && let Some((r, g, b)) = bar_rgb(face.background)
    {
        return Color::Rgb {
            r,
            g,
            b,
        };
    }
    face_bg(face)
}

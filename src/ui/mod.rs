//! All iocraft code lives in this module tree (plus `main.rs`, which
//! only starts/stops the render loop and restores the terminal). The
//! app layer (`src/app`) and the theme stay plain Rust.

pub mod diff_view;
pub mod file_view;
pub mod magit_status;
pub mod picker;
pub mod root;
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

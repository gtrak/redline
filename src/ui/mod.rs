//! All iocraft code lives in this module tree (plus `main.rs`, which
//! only starts/stops the render loop and restores the terminal). The
//! app layer (`src/app`) and the theme stay plain Rust.

pub mod blame_view;
pub mod buffer_view;
pub mod commit_editor;
pub mod diff_view;
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

/// One canvas text style. `invert` (reverse-video) is the picker's cursor
/// highlight: it swaps the face's foreground (which becomes the bar) with
/// the parent background (which becomes the text color), so a bright
/// foreground face yields a bright, clearly-visible selection bar.
pub(crate) fn text_style(foreground: theme::Color, invert: bool, bold: bool) -> CanvasTextStyle {
    let mut style = CanvasTextStyle::default();
    style.color = Some(color(foreground));
    if invert {
        style.invert = true;
    }
    if bold {
        style.weight = Weight::Bold;
    }
    style
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
pub(crate) fn truecolor_enabled() -> bool {
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

/// The theme's exact RGB for the landing-highlight band (jump-highlight),
/// matching the xterm-256 palette index iocraft emits for the face's
/// background (`Color::Yellow` -> `48;5;6`, nominal (255,255,0)). `None`
/// when the color has no truecolor mapping (falls back to the 256-color
/// path — the same adaptive strategy as `bar_rgb`).
fn jump_highlight_rgb(c: theme::Color) -> Option<(u8, u8, u8)> {
    use theme::Color as TC;
    Some(match c {
        TC::Yellow => (255, 255, 0),
        _ => return None,
    })
}

/// The theme's exact RGB for the VIEW's base background (the fade's
/// interpolation target: black in the dark theme, white in the light).
/// `None` when the color has no truecolor mapping.
fn view_bg_rgb(c: theme::Color) -> Option<(u8, u8, u8)> {
    use theme::Color as TC;
    Some(match c {
        TC::Black => (0, 0, 0),
        TC::White => (255, 255, 255),
        _ => return None,
    })
}

/// jump-highlight: the landing band's color at a given fade intensity
/// (in `[0, 1]`, the snapshot's pure curve). Under truecolor the face's
/// background RGB is INTERPOLATED toward the view's base background as
/// `(1 - intensity)` (a genuine fade: full face color at `1.0`, the base
/// background at `0.0`). Without truecolor the 16-color palette cannot
/// interpolate, so the band holds the face's palette color for the whole
/// duration and then clears — the hold-then-clear lives in the intensity
/// curve (`jump_highlight_intensity`), never in a faked RGB step here.
pub(crate) fn jump_band_bg(t: &theme::Theme, intensity: f32) -> Color {
    if truecolor_enabled()
        && let (Some((fr, fg, fb)), Some((br, bg, bb))) = (
            jump_highlight_rgb(t.jump_highlight.background),
            view_bg_rgb(t.view.background),
        )
    {
        let w = (intensity.clamp(0.0, 1.0) * 255.0) as u32; // face-weight
        let mix = |face: u8, base: u8| -> u8 {
            ((face as u32 * w + base as u32 * (255 - w)) / 255) as u8
        };
        return Color::Rgb {
            r: mix(fr, br),
            g: mix(fg, bg),
            b: mix(fb, bb),
        };
    }
    face_bg(t.jump_highlight)
}

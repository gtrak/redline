//! The transient menu (issue 002): a bottom-of-frame overlay listing the
//! active view's bindings as `[key] description` (leaf commands) and
//! `KEY …` (prefix submenus), grouped by registry category. The store owns
//! the menu state and derives the rows from the keymap × registry (no
//! hand-written table); this component is a pure renderer over the store's
//! [`TransientMenuRow`]s.
//!
//! Drawn with a canvas component (like the picker) for exact row/column
//! placement of the key column and the description column.

use iocraft::{prelude::*, Component, ComponentDrawer, ComponentUpdater};

use crate::app::store::TransientMenuRow;
use crate::model::text_width::{char_display_width, display_width};
use crate::theme;
use crate::ui::color;

#[derive(Default, Props)]
struct TransientMenuCanvasProps {
    pub rows: Vec<TransientMenuRow>,
    pub height: u32,
}

struct TransientMenuCanvas {
    rows: Vec<TransientMenuRow>,
    height: u32,
}

impl TransientMenuCanvas {
    fn from_props(props: &TransientMenuCanvasProps) -> Self {
        Self {
            rows: props.rows.clone(),
            height: props.height,
        }
    }
}

impl Component for TransientMenuCanvas {
    type Props<'a> = TransientMenuCanvasProps;

    fn new(props: &Self::Props<'_>) -> Self {
        Self::from_props(props)
    }

    fn update(
        &mut self,
        props: &mut Self::Props<'_>,
        _hooks: Hooks,
        updater: &mut ComponentUpdater,
    ) {
        *self = Self::from_props(props);
        updater.set_layout_style(iocraft::taffy::style::Style {
            size: iocraft::taffy::geometry::Size {
                width: iocraft::taffy::style::Dimension::Percent(1.0),
                height: iocraft::taffy::style::Dimension::Length(self.height as f32),
            },
            ..Default::default()
        });
    }

    fn draw(&mut self, drawer: &mut ComponentDrawer<'_>) {
        let layout = drawer.layout();
        let mut canvas = drawer.canvas();
        let t = theme::current();
        let w = layout.size.width.max(1.0) as usize;
        let h = layout.size.height.max(1.0) as usize;

        // Row 0: the menu's title bar.
        canvas.set_text(
            0,
            0,
            &truncate_ellipsis(" Transient menu · C-g to close ", w),
            text_style(t.view_title.foreground, false, true),
        );

        // Two-column flow (magit transients lay out key + description pairs
        // across the width), with a reserved gutter so the two columns never
        // abut. On narrow terminals (w < MIN_TWO_COL) fall back to a single
        // full-width column so descriptions stay legible. Category headers
        // occupy their own full-width row and start the following entries on
        // a fresh row.
        const COLS: usize = 2;
        const GUTTER: usize = 1;
        const MIN_TWO_COL: usize = 40;
        let cols = if w < MIN_TWO_COL { 1 } else { COLS };
        // Each cell's region width (cells). In two-column mode the text is
        // drawn into `cell_w - GUTTER` of it, reserving a gap on the right so
        // adjacent cells never touch; in single-column mode the whole width
        // is the text region (no gutter needed).
        let cell_w = (w / cols).max(1);
        let text_w = if cols == 1 { cell_w } else { (cell_w - GUTTER).max(1) };
        let mut y: isize = 1;
        let mut x: usize = 0;
        for row in &self.rows {
            if y as usize >= h {
                break;
            }
            if row.is_header {
                // A new category never shares a row with the last entry of
                // the previous group (which can leave the row half-filled
                // when that group has an odd number of entries).
                if x != 0 {
                    y += 1;
                }
                if y as usize >= h {
                    break;
                }
                // The category label spans the full width of its own row.
                canvas.set_text(
                    0,
                    y,
                    &truncate_ellipsis(&format!("[{row_label}]", row_label = row.label), w),
                    text_style(t.diff_hunk_header.foreground, false, true),
                );
                y += 1;
                x = 0;
                continue;
            }
            let text = if row.is_prefix {
                format!("{} …", row.key_display)
            } else {
                format!("[{}] {}", row.key_display, row.label)
            };
            let face = if row.is_prefix {
                t.section_heading
            } else {
                t.view
            };
            canvas.set_text(
                (x * cell_w) as isize,
                y,
                &truncate_ellipsis(&text, text_w),
                text_style(face.foreground, false, row.is_prefix),
            );
            x += 1;
            if x >= cols {
                x = 0;
                y += 1;
            }
        }
    }
}

fn text_style(foreground: theme::Color, invert: bool, bold: bool) -> CanvasTextStyle {
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

/// Truncate `s` to fit at most `max` terminal cells, appending a trailing
/// `…` when it does not fit so the cut reads as intentionally elided rather
/// than a mid-word hard cut. A string that fits exactly (or has room) is
/// returned unchanged (no ellipsis). `max` is in cells, not chars: wide
/// (CJK) chars are 2 cells and the ellipsis reserves exactly one cell, so
/// the result never exceeds `max` cells. `max == 0` yields an empty string;
/// `max == 1` yields just `…`.
pub(crate) fn truncate_ellipsis(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if display_width(s) <= max {
        return s.to_string();
    }
    // Reserve one cell for the ellipsis and fill the rest with content, never
    // straddling the boundary on a wide char.
    let budget = max - 1;
    let mut used = 0usize;
    let mut out = String::new();
    for c in s.chars() {
        let w = char_display_width(c);
        if used + w > budget {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::truncate_ellipsis;
    use crate::model::text_width::display_width;

    // ── plan 004 issue 05f: ellipsis-truncation helper ────────────────

    #[test]
    fn fits_exactly_is_not_ellipsized() {
        assert_eq!(truncate_ellipsis("abcd", 4), "abcd", "exact fit keeps the whole string");
        assert_eq!(truncate_ellipsis("abcd", 5), "abcd", "room to spare keeps it whole");
        assert_eq!(truncate_ellipsis("", 3), "", "empty string");
    }

    #[test]
    fn one_over_ellipsizes() {
        // 5 cells into a 4-cell box: 3 content cells + 1 ellipsis cell.
        let out = truncate_ellipsis("abcde", 4);
        assert_eq!(out, "abc…");
        assert_eq!(display_width(&out), 4, "never exceeds the cell");
    }

    #[test]
    fn much_over_stays_within_cell() {
        let out = truncate_ellipsis("Point forward one word, wrapping across lines", 20);
        assert!(out.ends_with('…'), "trailing ellipsis: {out:?}");
        assert_eq!(display_width(&out), 20, "exactly fills the cell");
    }

    #[test]
    fn wide_and_emoji_never_straddle() {
        // 中 is 2 cells. Budget for a 5-cell box is 4 content cells.
        let out = truncate_ellipsis("a中b中c中", 5);
        assert!(out.ends_with('…'));
        assert_eq!(display_width(&out), 5, "wide chars counted as 2 cells");
        // A wide char straddling the budget boundary is dropped whole.
        let out2 = truncate_ellipsis("ab中", 3);
        assert_eq!(display_width(&out2), 3);
        // Emoji (2 cells) never produces a half-rendered glyph.
        let out3 = truncate_ellipsis("a🙂b🙂", 4);
        assert_eq!(display_width(&out3), 4);
        assert!(out3.ends_with('…'));
    }

    #[test]
    fn degenerate_cell_widths() {
        assert_eq!(truncate_ellipsis("abc", 0), "", "zero cell → nothing");
        assert_eq!(truncate_ellipsis("abc", 1), "…", "one cell → just the ellipsis");
        // One cell but the input also fits in one cell: no ellipsis.
        assert_eq!(truncate_ellipsis("a", 1), "a");
        assert_eq!(truncate_ellipsis("中", 2), "中", "wide char fits its 2 cells");
    }
}

#[derive(Default, Props)]
pub struct TransientMenuViewProps {
    pub rows: Vec<TransientMenuRow>,
    pub height: u32,
}

/// Renders the transient menu overlay from the store's derived rows.
#[component]
pub fn TransientMenuView(
    props: &TransientMenuViewProps,
    mut _hooks: Hooks,
) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(flex_shrink: 0.0, background_color: color(t.view.background)) {
            TransientMenuCanvas(rows: props.rows.clone(), height: props.height)
        }
    }
}

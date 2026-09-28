//! The isearch list (plan 018 issue 02 — the helm-occur shape): while
//! isearch is active, the buffer's live match rows render as an overlay
//! in place of the buffer content (the picker-overlay precedent: a
//! content-sized canvas, NOT a new `ViewId` — isearch keeps living inside
//! the Buffer-view modal, and the store's `IsearchState` keeps
//! `pre_search_line/col` and the view stack untouched).
//!
//! The store owns the list (`AppStore::isearch_list`: the rows in search
//! order + the selection + activity); this component only renders. The
//! prompt stays in the MINIBUFFER (`I-search: {query} [{idx}/{count}]` —
//! this surface keeps its existing prompt, plan 018 §3), so the rows are
//! prompt-less: `line number + line text + the match's column`, one row
//! per match, selected-row bar per the shared cursor treatment (the
//! picker's one-contiguous-run rule — `text_style`'s `invert` on the text
//! cells, the gap painted with the face foreground so the bar never
//! breaks).
//!
//! Layout per row (cells):
//! - cols 1..=4: the 1-based line number, right-aligned in 4 cells;
//! - col 5: the blank gutter;
//! - cols 6..: the line text (left-anchored, cell-aware truncated);
//! - the `(col N)` detail right-aligned at the column's right edge — the
//!   match's CHAR column (`IsearchMatchRow.match_col`'s unit, the point's
//!   `col` unit — what RET lands on), verbatim.
//!
//! The line text OWNS the space: it is the detail (`(col N)`) that
//! right-aligns, and the text truncates to leave it room — the
//! picker's name-first rule (the name owns the space; the context
//! truncates and keeps its tail… here it is short enough to never cut).
//!
//! Windowing is content-sized exactly like the picker: the canvas is
//! `min(rows, viewport - 1)` tall, the visible window follows the
//! selection (`start = selected.saturating_sub(win - 1)`). An empty
//! result set is a 0-row canvas — the overlay is not rendered at all
//! (the store's rows are empty and the frame assembly skips it).
//!
//! All truncation is CELL-AWARE (wide/CJK chars are 2 cells,
//! `model::text_width`) — a wide char in the line text can never
//! straddle a column boundary (the picker's rule, issue picker-density).

use iocraft::{prelude::*, Component, ComponentDrawer, ComponentUpdater};

use crate::app::store::IsearchMatchRow;
use crate::model::text_width::{char_display_width, display_width};
use crate::theme;
use crate::ui::text_style;

#[derive(Default, Props)]
struct IsearchListCanvasProps {
    /// The match rows in search order (the store's `IsearchState.rows`).
    pub rows: Vec<IsearchMatchRow>,
    /// The selected row (index into `rows`).
    pub selected: usize,
    /// The terminal viewport height (rows) — sizes the canvas to content.
    pub viewport: u32,
}

/// Canvas-backed isearch list: the match rows, the selected-row bar.
/// (`viewport` is deliberately NOT stored — `update` reads it from the
/// props to size the layout; a stored copy would be dead state.)
struct IsearchListCanvas {
    rows: Vec<IsearchMatchRow>,
    selected: usize,
}

/// The canvas height: the match rows, capped to the viewport (minus the
/// minibuffer row the picker's cap leaves). No prompt row, no count row —
/// the prompt lives in the minibuffer; 0 rows → 0 height (the overlay is
/// not rendered for an empty result set).
fn canvas_height(rows: usize, viewport: u32) -> u32 {
    let cap = (viewport as usize).saturating_sub(1).max(1);
    rows.min(cap) as u32
}

impl IsearchListCanvas {
    fn from_props(props: &IsearchListCanvasProps) -> Self {
        Self {
            rows: props.rows.clone(),
            selected: props.selected,
        }
    }
}

impl Component for IsearchListCanvas {
    type Props<'a> = IsearchListCanvasProps;

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
        // Size the canvas to the content: `min(rows, viewport - 1)` rows.
        updater.set_layout_style(iocraft::taffy::style::Style {
            size: iocraft::taffy::geometry::Size {
                width: iocraft::taffy::style::Dimension::Percent(1.0),
                height: iocraft::taffy::style::Dimension::Length(canvas_height(
                    props.rows.len(),
                    props.viewport,
                ) as f32),
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
        if self.rows.is_empty() {
            return;
        }
        // The visible window follows the selection (the picker's window):
        // every canvas row is a list row here (no prompt/count rows).
        // `h` IS `canvas_height(rows, viewport)` — the layout was sized in
        // `update` from the same props, so the window is content-sized.
        let win = h.min(self.rows.len());
        let start = self.selected.saturating_sub(win.saturating_sub(1));
        for (row, i) in (start..start + win).enumerate() {
            let Some(entry) = self.rows.get(i) else {
                continue;
            };
            let face = if i == self.selected {
                t.list_item_selected
            } else {
                t.list_item
            };
            draw_match_row(
                &mut canvas,
                row as isize,
                w,
                entry,
                face,
                i == self.selected,
            );
        }
    }
}

/// The line number's fixed gutter (cells 1..=4, right-aligned 1-based) and
/// the text's start column (the picker keeps col 0 as the margin).
const LINE_NO_GUTTER: usize = 4;
const TEXT_X: usize = 1 + LINE_NO_GUTTER + 1;

/// Draw one match row (see the module doc for the layout). `pub(crate)`:
/// the store-tier pins quote the rows AS RENDERED (the store builds the
/// rows; this draws them).
pub(crate) fn draw_match_row(
    canvas: &mut CanvasSubviewMut,
    y: isize,
    w: usize,
    entry: &IsearchMatchRow,
    face: theme::Face,
    selected: bool,
) {
    let style = text_style(face.foreground, selected, false);
    // 1-based line number, right-aligned in the gutter (the store's
    // `line_no` is 0-based; the list reads as the buffer's line numbers).
    let number = format!("{:>width$}", entry.line_no + 1, width = LINE_NO_GUTTER);
    canvas.set_text(1, y, &number, style);
    // The match's column detail, right-aligned at the column's right edge
    // (exclusive `w` — the last cell is `w - 1`).
    let detail = format!("(col {})", entry.match_col);
    let detail_x = (w as i32).saturating_sub(display_width(&detail) as i32) as isize;
    // The line text owns the space: it truncates to leave the detail (and
    // one blank cell before it) room — the picker's name-first rule.
    let text_budget = (detail_x.max(0) as usize).saturating_sub(TEXT_X + 1);
    let text = truncate(&entry.line_text, text_budget);
    canvas.set_text(TEXT_X as isize, y, &text, style);
    canvas.set_text(detail_x, y, &detail, style);
    // The selected-row bar is one CONTIGUOUS run (the picker's rule): the
    // text cells carry the invert; the blank cells between the text and
    // the detail keep the default background, which breaks the bar —
    // paint that gap with the bar color (the face foreground, which is
    // what `invert` visually becomes), from the gutter's start to the
    // detail's start.
    if selected {
        let gap_start = TEXT_X as isize + display_width(&text) as isize;
        // Paint to the detail's start (inclusive-exclusive end `detail_x`)
        // — the picker's rule: the bar run extends all the way to the
        // detail, which itself carries the invert, so the run is
        // contiguous with no default-background hole before it.
        let gap_w = (detail_x - gap_start).max(0);
        if gap_w > 0 {
            canvas.set_background_color(
                gap_start,
                y,
                gap_w as usize,
                1,
                crate::ui::color(face.foreground),
            );
        }
    }
}

/// Truncate `s` to fit at most `max` terminal cells (not chars): a wide
/// (CJK) char occupies 2 cells, so a wide char never straddles a column
/// boundary. Hard cut (no ellipsis) to preserve the picker's existing
/// left-truncation shape, now measured in cells. `max == 0` yields empty.
fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if display_width(s) <= max {
        return s.to_string();
    }
    let mut used = 0usize;
    let mut out = String::new();
    for c in s.chars() {
        let cw = char_display_width(c);
        if used + cw > max {
            break;
        }
        used += cw;
        out.push(c);
    }
    out
}

#[derive(Default, Props)]
pub struct IsearchListProps {
    pub rows: Vec<IsearchMatchRow>,
    pub selected: usize,
    pub viewport: u32,
}

/// Renders the isearch list overlay from the store's isearch state.
#[component]
pub fn IsearchList(props: &IsearchListProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(
            flex_shrink: 0.0,
            background_color: crate::ui::color(t.view.background),
        ) {
            IsearchListCanvas(
                rows: props.rows.clone(),
                selected: props.selected,
                viewport: props.viewport,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(line_no: usize, text: &str, col: usize) -> IsearchMatchRow {
        IsearchMatchRow {
            line_no,
            line_text: text.to_string(),
            match_col: col,
        }
    }

    /// A match row renders line number + line text + the match's column:
    /// the 1-based number in the gutter, the text at the text column, the
    /// `(col N)` detail right-aligned at the column's right edge.
    #[test]
    fn match_row_renders_number_text_and_column() {
        let face = theme::current().list_item;
        let w = 40;
        let mut canvas = iocraft::Canvas::new(w, 1);
        {
            let mut sv = canvas.subview_mut(0, 0, 0, 0, w, 1);
            draw_match_row(&mut sv, 0, w, &row(3, "café omega", 5), face, false);
        }
        // The line number is 1-based and right-aligned in the gutter:
        // line_no 3 renders as "4" in the gutter's last cell (cell 4).
        assert_eq!(canvas.cell(4, 0).unwrap().text(), Some("4"));
        assert_eq!(canvas.cell(1, 0).unwrap().text(), Some(" "), "gutter lead is blank");
        // The text starts at the text column (cell 6).
        assert_eq!(canvas.cell(6, 0).unwrap().text(), Some("c"));
        assert_eq!(canvas.cell(7, 0).unwrap().text(), Some("a"));
        // The detail is right-aligned: its last cell is the column's last
        // cell (w - 1, the ')'), and it starts w - display_width cells in.
        let detail = "(col 5)";
        assert_eq!(canvas.cell(w - 1, 0).unwrap().text(), Some(")"));
        assert_eq!(
            canvas.cell(w - display_width(detail), 0).unwrap().text(),
            Some("("),
            "the detail's first cell is right-aligned at the column edge"
        );
    }

    /// A long line text truncates to leave the `(col N)` detail room: the
    /// detail is never cut, the text is (cell-aware: a wide char
    /// straddling the budget boundary is dropped whole).
    #[test]
    fn long_line_text_truncates_to_leave_the_detail() {
        let face = theme::current().list_item;
        let w = 20;
        let detail = "(col 12)";
        let mut canvas = iocraft::Canvas::new(w, 1);
        {
            let mut sv = canvas.subview_mut(0, 0, 0, 0, w, 1);
            draw_match_row(&mut sv, 0, w, &row(9, &"a".repeat(100), 12), face, false);
        }
        // The detail is pinned to the right edge (its last cell is w - 1).
        assert_eq!(
            canvas.cell(w - display_width(detail), 0).unwrap().text(),
            Some("("),
            "the detail's first cell is right-aligned at the column edge"
        );
        assert_eq!(canvas.cell(w - 1, 0).unwrap().text(), Some(")"));
        // The text fills every cell up to the one-cell gap before the detail.
        assert_eq!(canvas.cell(6, 0).unwrap().text(), Some("a"));
        assert_eq!(canvas.cell(w - display_width(detail) - 2, 0).unwrap().text(), Some("a"));
        assert_eq!(
            canvas.cell(w - display_width(detail) - 1, 0).unwrap().text(),
            None,
            "one cell separates the text from the detail (never written)"
        );
    }

    /// The selected row's bar is CONTIGUOUS (the picker's one-contiguous-
    /// run rule): the blank cells between the text and the detail carry
    /// the bar color (the face foreground — what `invert` visually
    /// becomes), while the text cells keep the invert (no explicit
    /// background). A mutation that drops the gap paint redden
    /// this: the gap cells read the default background and the bar
    /// breaks into two.
    #[test]
    fn selected_row_bar_is_contiguous_across_the_gap() {
        let face = theme::current().list_item_selected;
        let w = 40;
        let detail = "(col 5)";
        let mut canvas = iocraft::Canvas::new(w, 1);
        {
            let mut sv = canvas.subview_mut(0, 0, 0, 0, w, 1);
            draw_match_row(&mut sv, 0, w, &row(0, "omega", 5), face, true);
        }
        // "omega" occupies TEXT_X..TEXT_X+5 (cells 6..=10); the detail
        // occupies w - detail_w..w - 1 — the gap is everything between.
        let text_end = TEXT_X + display_width("omega");
        let gap_start = text_end;
        let gap_end = w - display_width(detail); // exclusive
        assert!(
            gap_start < gap_end,
            "expected a gap to paint: start {gap_start}, end {gap_end}"
        );
        for x in gap_start..gap_end {
            assert_eq!(
                canvas.cell(x, 0).unwrap().background_color,
                Some(crate::ui::color(face.foreground)),
                "gap cell {x} must carry the bar color (face foreground)"
            );
        }
        // The text cells keep the invert (no explicit background) — the
        // bar color comes from `invert`, exactly like the picker.
        let text_cell = canvas.cell(6, 0).unwrap();
        assert_eq!(text_cell.background_color, None);
        assert!(text_cell.text_style().unwrap().invert);
    }
}

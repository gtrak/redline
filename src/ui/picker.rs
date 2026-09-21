//! Picker (helm-style) component: a prompt line, a nucleo-filtered
//! candidate list, and a live preview pane (helm follow-mode, always
//! on in redline v1). Consumers: the `M-x` command palette, the file
//! pickers (find-file / recent-files — preview is the file's first
//! page as plain text), the buffer pickers (switch/kill — preview is
//! the buffer's head), and the project switcher (preview is the root
//! path). The store owns the picker state (query, filtered list,
//! selection, preview); this component only renders it.
//!
//! The list is drawn with a canvas component: element! `Text` children
//! are flex-positioned, and we need exact row/column placement for the
//! list and the preview column.
//!
//! Layout (issue picker-density):
//! - Rows are NAME-FIRST. When a candidate carries a `detail` string the
//!   `label` (the name) is drawn left-anchored and `detail` is right-aligned
//!   at the candidate column's right edge, so the paths form a scannable
//!   column. The name OWNS the space: the label is truncated only if the
//!   name alone exceeds the whole candidate column, and it is the DETAIL that
//!   truncates (keeping its tail — the file name — so the repetitive path
//!   prefix is what gets dropped). On rows with very long names the detail
//!   column's left edge moves (its fixed right-alignment is preserved, its
//!   width shrinks). Candidates with no `detail` draw a single
//!   left-anchored string: the `label` when it is non-empty (the
//!   non-current branch rows — their `display` keeps the `*` marker slot's
//!   leading space for matching, but the name must align with the current
//!   branch row's name column), or `display` when the label is empty (the
//!   palette — a single command identifier has no context to right-align —
//!   and the scratch buffer row, where label == display).
//! - The selected-row bar is one CONTIGUOUS run. A name-first row issues two
//!   `set_text` calls (label, then detail); the default-background gap between
//!   the inverted cells is painted with the bar color (the face foreground,
//!   which is what `invert` visually becomes) so the bar never breaks.
//! - The canvas is sized to its content and the viewport:
//!   `height = min(candidates + 2, viewport - 1)` (prompt + rows + count), so
//!   a 3-candidate list is a 5-row box, not a fixed 12-row one.
//! - When the selected candidate's preview is empty the candidate rows take
//!   the full width (no dead 2/3 preview split).
//!
//! All truncation here is CELL-AWARE (wide/CJK chars are 2 cells,
//! `model::text_width`), so a wide char in a fixed column can never overflow
//! its column.

use iocraft::{prelude::*, Component, ComponentDrawer, ComponentUpdater};

use crate::app::store::PickerCandidate;
use crate::model::text_width::{char_display_width, display_width};
use crate::theme;
use crate::ui::{color, text_style};

#[derive(Default, Props)]
struct PickerCanvasProps {
    pub prompt: String,
    pub query: String,
    pub selected: usize,
    pub candidates: Vec<PickerCandidate>,
    pub total: usize,
    pub preview: String,
    /// The terminal viewport height (rows) — sizes the canvas to content
    /// (issue picker-density).
    pub viewport: u32,
}

/// Canvas-backed picker surface: prompt row, candidate list rows,
/// preview column (selected candidate), and a count row.
struct PickerCanvas {
    prompt: String,
    query: String,
    selected: usize,
    candidates: Vec<PickerCandidate>,
    total: usize,
    preview: String,
}

/// The canvas height: prompt + rows + count, capped to the viewport. A
/// 3-candidate list is a 5-row box; a long list fills (nearly) the popup.
/// Floored so a zero/near-zero viewport still leaves prompt + count.
fn canvas_height(candidates: usize, viewport: u32) -> u32 {
    let cap = (viewport as usize).saturating_sub(1).max(1);
    (candidates + 2).min(cap).max(3) as u32
}

impl PickerCanvas {
    fn from_props(props: &PickerCanvasProps) -> Self {
        Self {
            prompt: props.prompt.clone(),
            query: props.query.clone(),
            selected: props.selected,
            candidates: props.candidates.clone(),
            total: props.total,
            preview: props.preview.clone(),
        }
    }
}

impl Component for PickerCanvas {
    type Props<'a> = PickerCanvasProps;

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
        // Size the canvas to the content + viewport (issue picker-density B):
        // `min(candidates + 2, viewport - 1)` rows.
        let h = canvas_height(props.candidates.len(), props.viewport);
        updater.set_layout_style(iocraft::taffy::style::Style {
            size: iocraft::taffy::geometry::Size {
                width: iocraft::taffy::style::Dimension::Percent(1.0),
                height: iocraft::taffy::style::Dimension::Length(h as f32),
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

        // Row 0: prompt + query.
        let prompt = format!("{}{}", self.prompt, self.query);
        canvas.set_text(0, 0, &truncate(&prompt, w), text_style(t.prompt.foreground, false, true));
        // Rows 1..h-2: candidate list (left) + preview (right) for the
        // selected candidate; last row: count.
        let list_h = h.saturating_sub(2);
        if list_h > 0 {
            let win = list_h.min(self.candidates.len());
            let start = self.selected.saturating_sub(win.saturating_sub(1));
            // D: no dead preview space — when the selected candidate has no
            // preview the candidate rows take the full width (no 2/3 split).
            let has_preview = !self.preview.is_empty();
            let split = (w as i32 * 2 / 3) as usize;
            let cand_w = if has_preview { split } else { w };
            for (row, i) in (start..start + win).enumerate() {
                if let Some(candidate) = self.candidates.get(i) {
                    let selected = i == self.selected;
                    let face = if selected {
                        t.list_item_selected
                    } else {
                        t.list_item
                    };
                    let y = 1 + row as isize;
                    draw_candidate_row(&mut canvas, y, cand_w, candidate, face, selected);
                }
            }
            // Preview pane: the selected candidate's preview text, one line
            // per row (clipped to the visible rows). Only when there is a
            // preview to show.
            if has_preview {
                let preview_x = (split + 1) as isize;
                let preview_w = (w as i32 - split as i32).saturating_sub(1);
                if preview_w > 1 {
                    for (row, line) in self.preview.lines().take(list_h).enumerate() {
                        canvas.set_text(
                            preview_x,
                            1 + row as isize,
                            &truncate(line, preview_w as usize),
                            text_style(t.preview.foreground, false, false),
                        );
                    }
                }
            }
        }

        // Last row: candidate count, right-aligned.
        if h > 2 {
            let count = format!("{} of {}", self.candidates.len(), self.total);
            // Cell-aware offset (like every other measure here); `count` is
            // ASCII so this is a no-op in practice, but keeps the one
            // exception to cell-measurement gone.
            let x = (w as i32).saturating_sub(display_width(&count) as i32 + 1) as isize;
                canvas.set_text(x, h as isize - 1, &count, text_style(t.minibuffer.foreground, false, false));
        }
    }
}

/// Draw one candidate row on the picker canvas (see the module doc for the
/// row layout). A candidate with no `detail` is a single left-anchored
/// string (the `label` when it is non-empty — the non-current branch
/// rows' `display` keeps the `*` marker slot's leading space for matching
/// — otherwise `display`); otherwise it is a name-first row (label left,
/// detail right-aligned at the column's right edge).
fn draw_candidate_row(
    canvas: &mut CanvasSubviewMut,
    y: isize,
    cand_w: usize,
    candidate: &PickerCandidate,
    face: theme::Face,
    selected: bool,
) {
    if candidate.detail.is_empty() {
        // Single left-anchored string. When the candidate carries a
        // label, draw the LABEL: the non-current branch rows keep
        // `display` as `" name"` (the leading space is the `*` marker
        // slot, retained for matching), so drawing `display` would push
        // the branch name one column right of the current branch row's
        // name column — the label aligns it. For the other
        // empty-detail candidates the label is either empty (the
        // palette — a single command identifier has no context to
        // right-align; `display` is drawn, unchanged) or identical to
        // `display` (the scratch buffer row), so nothing is lost.
        let text = if candidate.label.is_empty() {
            &candidate.display
        } else {
            &candidate.label
        };
        let label = truncate(text, cand_w);
        canvas.set_text(1, y, &label, text_style(face.foreground, selected, selected));
    } else {
        // A: name-first — the NAME owns the space (issue picker-density):
        // the label truncates only if the name ALONE exceeds the whole
        // candidate column; the DETAIL is what truncates, and it keeps its
        // tail (the file name) so the repetitive path prefix is dropped. The
        // detail stays right-aligned at the column's right edge, so the
        // paths form a scannable column.
        let right_edge = 1 + cand_w; // exclusive right boundary of the column
        let label = if display_width(&candidate.label) > cand_w {
            truncate(&candidate.label, cand_w)
        } else {
            candidate.label.clone()
        };
        let label_end = 1 + display_width(&label);
        let detail = truncate_left(&candidate.detail, right_edge.saturating_sub(label_end));
        let detail_x = (right_edge - display_width(&detail)) as isize;
        if selected {
            // Fix 1: the name-first row issues two set_text calls, so the
            // inverted (bar) cells leave a default-background gap between the
            // label and the detail. Paint that gap with the bar color — the
            // face foreground, which is what `invert` visually becomes — so
            // the selected row stays one contiguous bar, exactly like the
            // single-string path.
            let gap_w = (detail_x - label_end as isize).max(0);
            if gap_w > 0 {
                canvas.set_background_color(
                    label_end as isize,
                    y,
                    gap_w as usize,
                    1,
                    color(face.foreground),
                );
            }
        }
        canvas.set_text(1, y, &label, text_style(face.foreground, selected, selected));
        canvas.set_text(detail_x, y, &detail, text_style(face.foreground, selected, selected));
    }
}

/// Truncate `s` to fit at most `max` terminal CELLS, keeping the RIGHTMOST
/// cells (the tail) instead of the leftmost. Used for the name-first row's
/// detail column: when it is squeezed by a long name, the repetitive path
/// prefix (left) is what gets dropped, not the informative file name (right).
/// `max == 0` yields empty.
fn truncate_left(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if display_width(s) <= max {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut used = 0usize;
    let mut start = chars.len();
    for (i, c) in chars.iter().enumerate().rev() {
        let cw = char_display_width(*c);
        if used + cw > max {
            start = i + 1;
            break;
        }
        used += cw;
        start = i;
    }
    chars[start..].iter().collect()
}

/// Truncate `s` to fit at most `max` terminal CELLS (not chars): a wide
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
pub struct PickerProps {
    pub prompt: String,
    pub query: String,
    pub selected: usize,
    pub candidates: Vec<PickerCandidate>,
    pub total: usize,
    pub preview: String,
    /// The terminal viewport height (rows) — sizes the canvas (issue
    /// picker-density).
    pub viewport: u32,
}

/// Renders the picker overlay from the store's picker state.
#[component]
pub fn Picker(props: &PickerProps, mut _hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let t = theme::current();
    element! {
        View(
            flex_shrink: 0.0,
            background_color: color(t.view.background),
        ) {
            PickerCanvas(
                prompt: props.prompt.clone(),
                query: props.query.clone(),
                selected: props.selected,
                candidates: props.candidates.clone(),
                total: props.total,
                preview: props.preview.clone(),
                viewport: props.viewport,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::text_width::display_width;

    /// The picker's truncation is CELL-AWARE (not char-counting): a wide
    /// (CJK) char is 2 cells and never straddles a column boundary. This is
    /// the intentional behavior change from the old char-based `truncate`
    /// (issue picker-density: name-first rows sit in fixed columns, so a
    /// wide char must not overflow the column silently).
    #[test]
    fn truncate_counts_cells_not_chars() {
        // 4 wide chars = 8 cells; a 6-cell budget keeps exactly 3 wide chars.
        let wide = "\u{4e00}\u{4e01}\u{4e02}\u{4e03}"; // 中好世界
        assert_eq!(display_width(wide), 8);
        let cut = truncate(wide, 6);
        assert_eq!(cut, "\u{4e00}\u{4e01}\u{4e02}");
        assert_eq!(display_width(&cut), 6, "never exceeds the cell budget");

        // ASCII is unaffected by the cell/char switch.
        assert_eq!(truncate("abcdef", 3), "abc");
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abc", 0), "");

        // A wide char that would straddle the boundary is dropped whole
        // (no half-cell bleed into the next column).
        assert_eq!(truncate("ab\u{4e00}", 3), "ab", "wide char straddling col 2-3 is dropped");
    }

    /// `truncate_left` keeps the rightmost cells (the tail), dropping the
    /// left prefix — the opposite of `truncate`. A wide char straddling the
    /// boundary is dropped whole from the left.
    #[test]
    fn truncate_left_keeps_the_tail() {
        assert_eq!(truncate_left("abcdefgh", 3), "fgh");
        assert_eq!(truncate_left("abc", 5), "abc");
        assert_eq!(truncate_left("abc", 0), "");
        // Keeps the tail; a wide char that straddles is dropped from the left.
        assert_eq!(truncate_left("ab\u{4e00}", 3), "b\u{4e00}", "keeps 'b' + wide char (3 cells)");
    }

    /// Fix 1 (discriminating): the selected name-first row's bar is CONTIGUOUS
    /// across the label→detail gap. The name-first row issues two set_text
    /// calls, so without the bar-color paint the gap cells carry NO background
    /// (default background) and the inverted bar breaks into two. This test
    /// asserts the gap cells DO carry the bar color (the face foreground, what
    /// `invert` visually becomes) while the label/detail cells keep the invert
    /// (no explicit background) — so it FAILS before the fix (gap bg = None)
    /// and passes after.
    #[test]
    fn selected_name_first_row_bar_is_contiguous() {
        let face = theme::current().list_item_selected;
        let cand_w = 53;
        let candidate = PickerCandidate {
            name: String::new(),
            display: String::new(),
            label: "some_symbol_name".to_string(),
            detail: "[fn] src/app/store.rs:1234".to_string(),
            docs: String::new(),
            category: String::new(),
        };
        let w = 80;
        let mut canvas = iocraft::Canvas::new(w, 1);
        {
            let mut sv = canvas.subview_mut(0, 0, 0, 0, w, 1);
            draw_candidate_row(&mut sv, 0, cand_w, &candidate, face, true);
        }
        // Recompute the layout exactly as the row draws it.
        let label_end = 1 + display_width(&candidate.label);
        let right_edge = 1 + cand_w;
        let detail = truncate_left(&candidate.detail, right_edge.saturating_sub(label_end));
        let detail_x = right_edge - display_width(&detail);
        // There must be a gap between label and detail for this to be a
        // discriminating case (no gap => nothing to break, nothing to paint).
        assert!(
            detail_x > label_end,
            "expected a label→detail gap to paint: label_end {label_end}, detail_x {detail_x}"
        );
        // The gap cells carry the bar color so the inverted bar is contiguous.
        for x in label_end..detail_x {
            assert_eq!(
                canvas.cell(x, 0).unwrap().background_color,
                Some(color(face.foreground)),
                "gap cell {x} must carry the bar color (face foreground)"
            );
        }
        // The label and detail cells keep the invert (no explicit background)
        // — the bar color comes from `invert`, exactly like the single-string
        // path, so the text stays visible on the bar.
        let label_cell = canvas.cell(1, 0).unwrap();
        assert_eq!(label_cell.background_color, None);
        assert!(label_cell.text_style().unwrap().invert);
        let detail_cell = canvas.cell(detail_x, 0).unwrap();
        assert_eq!(detail_cell.background_color, None);
        assert!(detail_cell.text_style().unwrap().invert);
    }

    /// Gate follow-up (alignment): a non-current branch row's `display`
    /// carries a leading space (the `*` marker slot, kept for matching).
    /// The empty-detail fallback must draw the LABEL (the bare branch
    /// name), not `display`, so the branch name starts in the same
    /// column as the current branch row's name. This test fails if the
    /// fallback regresses to drawing `display` (the name lands one column
    /// right of the current branch row).
    #[test]
    fn non_current_branch_rows_align_with_the_current() {
        let face = theme::current().list_item;
        let current = PickerCandidate {
            name: "main".into(),
            display: "*main".into(),
            label: "main".into(),
            detail: "*".into(),
            docs: String::new(),
            category: "branch".into(),
        };
        let other = PickerCandidate {
            name: "feature".into(),
            display: " feature".into(), // marker slot: leading space
            label: "feature".into(),
            detail: String::new(),
            docs: String::new(),
            category: "branch".into(),
        };
        let w = 80;
        let cand_w = w - 1; // right edge 1 + cand_w = 80, inside the canvas
        let mut canvas = iocraft::Canvas::new(w, 2);
        {
            let mut sv = canvas.subview_mut(0, 0, 0, 0, w, 2);
            draw_candidate_row(&mut sv, 0, cand_w, &current, face, false);
            draw_candidate_row(&mut sv, 1, cand_w, &other, face, false);
        }
        // Both branch names start at column 1 — the current branch row's
        // label and the non-current branch row's label. Before the fix the
        // non-current row drew `display` (" feature"), so cell 1 was a
        // blank and "feature" started at column 2.
        assert_eq!(
            canvas.cell(1, 0).unwrap().text(),
            Some("m"),
            "current branch name starts at column 1"
        );
        assert_eq!(
            canvas.cell(1, 1).unwrap().text(),
            Some("f"),
            "non-current branch name must start in the same column (no marker-slot offset)"
        );
        assert_eq!(canvas.cell(2, 1).unwrap().text(), Some("e"));
        // The `*` marker still rides the current branch row, right-aligned
        // at the column's right edge.
        let marker = (0..w)
            .find(|&x| canvas.cell(x, 0).and_then(|c| c.text()) == Some("*"))
            .expect("the current branch's * marker is right-aligned");
        assert_eq!(marker, 1 + cand_w - 1, "the * sits on the column's right edge");
    }
}

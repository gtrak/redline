//! Cursor / click geometry: the hardware-cursor cell for the current
//! view and the tree-sidebar click split (pure math over a Snapshot).

use crate::app::store::{FileViewRow, ViewId};
use crate::model::tree_layout::TREE_WIDTH;

use super::snapshot::Snapshot;

/// The hardware-cursor position (0-based column, 0-based row — crossterm's
/// `MoveTo(col, row)` order) for the current view. The row is the view's cursor
/// row — the blue-bar (selected) row for list views, the point's screen row for
/// the read-focused buffer view (clamped to the viewport, plan 004 issue 05c)
/// — and the column is the point's DISPLAY column (char-index → display-
/// column conversion, plan 004 issue 05d) for the read-focused buffer view, 0
/// for list views).
///
/// Layout (0-based terminal rows): the main view's title is row 0 and its first
/// content row is row 1, so content row `i` (0-based within the window) is at
/// terminal row `1 + i`. With the tree sidebar visible the view's column is
/// PANE-relative while the terminal cursor is in ABSOLUTE screen coordinates
/// (the two panes are laid out by the flex row: the tree occupies terminal
/// columns `[0, TREE_WIDTH)`), so the column is offset by `TREE_WIDTH`
/// (clamped to the terminal width) and the row left unchanged (plan 004
/// issue 05g). Tree hidden: the offset is 0 (pane col 0 == absolute col 0).
pub(super) fn cursor_cell(snap: &Snapshot) -> Option<(u16, u16)> {
    let cell = |i: usize| (0u16, 1 + i as u16);
    let pos = match snap.view {
        // 06a: home has no selectable row — the cursor stays where the
        // frame park put it (the status line).
        ViewId::Home => None,
        ViewId::BufferList => Some(cell(
            snap.buffer_list_selected.min(snap.buffer_rows.len().saturating_sub(1)),
        )),
        ViewId::Search => snap.search_selected_row.map(cell).or_else(|| Some(cell(0))),
        ViewId::MagitStatus => Some(cell(
            snap.magit_rows.iter().position(|r| r.selected).unwrap_or(0),
        )),
        ViewId::Log => Some(cell(snap.log_rows.iter().position(|r| r.selected).unwrap_or(0))),
        ViewId::Blame => Some(cell(snap.blame_rows.iter().position(|r| r.selected).unwrap_or(0))),
        ViewId::CommitDiff => Some(cell(
            snap.commit_diff_rows.iter().position(|r| r.selected).unwrap_or(0),
        )),
        ViewId::CommitEditor => Some(cell(0)),
        ViewId::Buffer => {
            // The "changed on disk" banner (when present) pushes the content
            // down one row; account for it so the cursor lands on the
            // intended content row, not the banner.
            let banner = u16::from(snap.file_view_changed_on_disk);
            let total = snap.file_view_total_lines;
            // The canvas gets `viewport - banner` rows; the cursor row must
            // never be placed outside the file area (plan 004 issue 05c —
            // the window can sit off the point, e.g. after a recenter clamp
            // or a resize, and C-l no longer moves the window off the point
            // in the first place).
            // The target BUFFER line: the point (read-focused) or the
            // insertion row (the last line, editable buffers — the cursor
            // stays there; plan 004 05b/05c). plan 005 issue 02: that
            // buffer line is translated to its RENDERED row through the row
            // list (note rows are extra rows; the dense `line - top`
            // offset is gone). When the target line is outside the visible
            // slice (the window can sit off the point), fall back to the
            // pre-annotation clamp.
            let target_line = if snap.file_view_current_buffer_editable {
                total.saturating_sub(1)
            } else {
                snap.file_view_point_line.min(total.saturating_sub(1))
            };
            let rows = &snap.file_view_rows;
            let content_row = match FileViewRow::row_for_line(rows, target_line) {
                Some(i) => i,
                None => target_line.saturating_sub(snap.file_view_top_line),
            }
            // plan 005 issue 02b: clamp against the RENDERED slice (which is
            // capped to <= viewport_lines by file_view_rows), not the raw
            // buffer-line viewport.
            .min(rows.len().saturating_sub(1));
            let col = if snap.file_view_current_buffer_editable {
                0
            } else {
                // The terminal cursor is positioned in CELLS, not char
                // indexes: the display column is the width of the point
                // line's prefix [0, point_col) (plan 004 issue 05d).
                // issue-annotations-symbol-precise: an annotated line's
                // code sits at display column `code_start` (the leading
                // run's display width, or column 1 for the column-0
                // exact-1 shift); the row's `text` has that leading run
                // stripped, so the point column (a full-line char offset)
                // is translated onto the stripped text by `indent_chars`,
                // then offset by the code's start column. A non-annotated
                // line keeps its full
                // text at column 0. The fallback is the char index when the
                // point line is outside the pre-computed visible slice.
                FileViewRow::row_for_line(rows, target_line)
                    .and_then(|i| rows.get(i))
                    .map(|r| {
                        if r.annotated {
                            let code_start = r.code_start;
                            let code_col =
                                snap.file_view_point_col.saturating_sub(r.indent_chars);
                            code_start
                                + crate::model::text_width::char_index_to_display_col(
                                    &r.text, code_col,
                                )
                        } else {
                            crate::model::text_width::char_index_to_display_col(
                                &r.text,
                                snap.file_view_point_col,
                            )
                        }
                    })
                    .unwrap_or(snap.file_view_point_col)
            };
            Some((col as u16, 1 + banner + content_row as u16))
        }
    };
    // Plan 004 issue 05g: the terminal cursor is in ABSOLUTE screen
    // coordinates, but the view column computed above is PANE-relative —
    // the two panes are laid out by the flex row, and the tree sidebar (when
    // visible) occupies terminal columns `[0, TREE_WIDTH)`. Offset the column
    // by the tree width (row unchanged) so the hardware cursor does not land
    // inside the sidebar, and clamp to the terminal width so a huge column
    // cannot address outside the screen. Tree hidden: offset 0, unchanged.
    pos.map(|(col, row)| {
        let col = if snap.tree_visible {
            col.saturating_add(TREE_WIDTH).min(snap.terminal_width.saturating_sub(1))
        } else {
            col
        };
        (col, row)
    })
}
/// Plan 004 issue 05e: with the tree sidebar visible the code pane's column 0
/// is terminal column `TREE_WIDTH` (the tree occupies terminal columns
/// `[0, TREE_WIDTH)`). Splits a 0-based terminal click column into
/// `(in_tree, pane_col)`: a click inside the tree's columns must NOT move the
/// code point (the tree row under it is selected instead — the store maps the
/// terminal row); every other click passes the column shifted by the tree
/// width to the file-view click mapping. Tree hidden: the shift is 0 and no
/// click lands in the tree.
pub(super) fn click_pane(tree_visible: bool, col: usize) -> (bool, usize) {
    let offset = if tree_visible { TREE_WIDTH as usize } else { 0 };
    (tree_visible && col < offset, col.saturating_sub(offset))
}
#[cfg(test)]
mod tests {
    use super::*;

    // ── plan 004 issue 05d: cursor_cell display-column conversion ───────

    /// A `Snapshot` shaped for `cursor_cell`: a read-focused Buffer view
    /// with the given lines at window top 0. All fields `cursor_cell` does
    /// not read stay at inert defaults.
    fn buffer_snapshot(lines: &[&str], point_line: usize, point_col: usize) -> Snapshot {
        let rows: Vec<FileViewRow> = lines
            .iter()
            .enumerate()
            .map(|(i, t)| FileViewRow {
                line: i,
                is_note: false,
                annotated: false,
                anchors: Vec::new(),
                code_start: 0,
                indent_chars: 0,
                text: (*t).to_string(),
                spans: Vec::new(),
                matches: Vec::new(),
                highlight: None,
            })
            .collect();
        Snapshot {
            view: ViewId::Buffer,
            file_view_rows: rows.clone(),
            file_view_total_rows: lines.len(),
            file_view_top_line: 0,
            file_view_total_lines: lines.len(),
            file_view_viewport_lines: 21,
            file_view_point_line: point_line,
            file_view_point_col: point_col,
            quit: false,
            project: String::new(),
            view_name: String::new(),
            pending: String::new(),
            activity: String::new(),
            resolving: String::new(),
            crate_indexing: String::new(),
            message: String::new(),
            home_title: String::new(),
            home_rows: Vec::new(),
            buffer_rows: Vec::new(),
            buffer_list_selected: 0,
            picker: false,
            prompt: String::new(),
            query: String::new(),
            selected: 0,
            candidates: Vec::new(),
            total: 0,
            preview: String::new(),
            magit_rows: Vec::new(),
            magit_top_row: 0,
            magit_total_rows: 0,
            menu_open: false,
            menu_rows: Vec::new(),
            menu_height: 0,
            log_title: String::new(),
            log_rows: Vec::new(),
            blame_title: String::new(),
            blame_rows: Vec::new(),
            commit_diff_title: String::new(),
            commit_diff_rows: Vec::new(),
            commit_diff_top_row: 0,
            commit_diff_total_rows: 0,
            commit_editor_title: String::new(),
            commit_editor_rows: Vec::new(),
            dirty: None,
            file_view_title: String::new(),
            file_view_changed_on_disk: false,
            file_view_current_buffer_editable: false,
            buffer_mode: String::new(),
            tree_visible: false,
            tree_rows: Vec::new(),
            tree_selected: 0,
            terminal_width: 80,
            which_function: String::new(),
            indexing: String::new(),
            search_title: String::new(),
            search_rows: Vec::new(),
            search_top_row: 0,
            search_total_rows: 0,
            search_selected_row: None,
            search_running: false,
            search_error: None,
            searching: String::new(),
            position: String::new(),
            annotations: String::new(),
            region_lines: None,
            region_size: None,
            file_view_notes_folded: false,
        }
    }

    /// The cursor column is the point's DISPLAY column (terminal cells),
    /// not its char index (plan 004 issue 05d): char 13 of
    /// `CJK: abcd中 efgh` sits at display col 14 (中 is 2 cells).
    #[test]
    fn cursor_cell_uses_display_column_not_char_index() {
        let snap = buffer_snapshot(&["CJK: abcd中 efgh", "bb"], 0, 13);
        assert_eq!(
            cursor_cell(&snap),
            Some((14, 1)),
            "char 13 → display col 14 (0-based); content row 0 → row 1 (0-based)"
        );
        let snap = buffer_snapshot(&["CJK: abcd中 efgh", "bb"], 0, 0);
        assert_eq!(cursor_cell(&snap), Some((0, 1)));
        // ASCII lines are unchanged: display col == char index.
        let snap = buffer_snapshot(&["abcd", "bb"], 1, 1);
        assert_eq!(cursor_cell(&snap), Some((1, 2)));
        // Combining marks add no cell: char 2 of e+U+0301+x is at col 1.
        let snap = buffer_snapshot(&["e\u{301}x"], 0, 2);
        assert_eq!(cursor_cell(&snap), Some((1, 1)));
    }

    /// A point line outside the pre-computed visible slice falls back to
    /// the char index (the cursor row is clamped off the point there too).
    #[test]
    fn cursor_cell_falls_back_to_char_index_off_slice() {
        let mut snap = buffer_snapshot(&["bb", "bb"], 4, 2);
        snap.file_view_total_lines = 5;
        // plan 005 issue 02b: the clamp is against the RENDERED slice
        // (rows.len() = 2), not the buffer-line viewport. Off-slice point
        // line 4 falls back to char index 2, row clamped to last rendered
        // row (row 1) → terminal row 1 + 0 + 1 = 2.
        assert_eq!(cursor_cell(&snap), Some((2, 2)), "char index 2, clamped to rendered-slice row 1 → terminal row 2");
    }

    // ── plan 005 issue 02b: gutter + note-row overflow regression ────────

    /// A snapshot with annotated rows and interleaved note rows, shaped for
    /// the 02b regression: the point's line is at the window bottom, and a
    /// note row above it must not push the point off-canvas.
    /// (annotations-render-fold: the note rows emit BEFORE their code row —
    /// the helper mirrors the emitter's ordering.)
    fn annotated_snapshot(
        lines: &[(&str, bool)], // (text, annotated)
        note_lines: &[usize],   // buffer lines that have a note row above
        point_line: usize,
        point_col: usize,
        total_lines: usize,
    ) -> Snapshot {
        let rows: Vec<FileViewRow> = lines
            .iter()
            .enumerate()
            .flat_map(|(i, (text, annotated))| {
                let notes: Vec<FileViewRow> = note_lines
                    .iter()
                    .filter(|&&l| l == i)
                    .map(|&l| FileViewRow {
                        line: l,
                        is_note: true,
                        annotated: false,
                        anchors: vec![0],
                        code_start: 0,
                        indent_chars: 0,
                        text: format!("note {l}"),
                        spans: Vec::new(),
                        matches: Vec::new(),
                        highlight: None,
                    })
                    .collect();
                let code_row = FileViewRow {
                    line: i,
                    is_note: false,
                    annotated: *annotated,
                    // The column-0 shift shape (issue-annotations-symbol-
                    // precise): full text intact, the column-0 indicator
                    // takes cell 0, the code starts at cell 1.
                    anchors: vec![0],
                    code_start: 1,
                    indent_chars: 0,
                    text: (*text).to_string(),
                    spans: Vec::new(),
                    matches: Vec::new(),
                    highlight: None,
                };
                notes.into_iter().chain(std::iter::once(code_row))
            })
            .collect();
        Snapshot {
            view: ViewId::Buffer,
            file_view_rows: rows.clone(),
            file_view_total_rows: total_lines + note_lines.len(),
            file_view_top_line: 0,
            file_view_total_lines: total_lines,
            file_view_viewport_lines: 21,
            file_view_point_line: point_line,
            file_view_point_col: point_col,
            quit: false,
            project: String::new(),
            view_name: String::new(),
            pending: String::new(),
            activity: String::new(),
            resolving: String::new(),
            crate_indexing: String::new(),
            message: String::new(),
            home_title: String::new(),
            home_rows: Vec::new(),
            buffer_rows: Vec::new(),
            buffer_list_selected: 0,
            picker: false,
            prompt: String::new(),
            query: String::new(),
            selected: 0,
            candidates: Vec::new(),
            total: 0,
            preview: String::new(),
            magit_rows: Vec::new(),
            magit_top_row: 0,
            magit_total_rows: 0,
            menu_open: false,
            menu_rows: Vec::new(),
            menu_height: 0,
            log_title: String::new(),
            log_rows: Vec::new(),
            blame_title: String::new(),
            blame_rows: Vec::new(),
            commit_diff_title: String::new(),
            commit_diff_rows: Vec::new(),
            commit_diff_top_row: 0,
            commit_diff_total_rows: 0,
            commit_editor_title: String::new(),
            commit_editor_rows: Vec::new(),
            dirty: None,
            file_view_title: String::new(),
            file_view_changed_on_disk: false,
            file_view_current_buffer_editable: false,
            buffer_mode: String::new(),
            tree_visible: false,
            tree_rows: Vec::new(),
            tree_selected: 0,
            terminal_width: 80,
            which_function: String::new(),
            indexing: String::new(),
            search_title: String::new(),
            search_rows: Vec::new(),
            search_top_row: 0,
            search_total_rows: 0,
            search_selected_row: None,
            search_running: false,
            search_error: None,
            searching: String::new(),
            position: String::new(),
            annotations: String::new(),
            region_lines: None,
            region_size: None,
            file_view_notes_folded: false,
        }
    }

    /// issue-annotations-anchor-at-symbol: the 2-cell gutter is GONE — the
    /// cursor column on an annotated line is the code's own start column
    /// (`code_start`) plus the point's display offset within the code,
    /// NOT a fixed 2-cell add. The fixtures here are COLUMN-0 lines (no
    /// indentation to borrow): the anchor is column 0, the code shifts right
    /// by exactly one cell, so the cursor sits at column 1 + the point's
    /// display col. (The store builds these rows with the full text intact,
    /// `code_start = 1`, `indent_chars = 0` — the column-0 shape.)
    /// (annotations-render-fold: the note row emits BEFORE
    /// the code row, so an annotated point's code row sits at slice row 1
    /// — terminal row 2.)
    #[test]
    fn cursor_cell_annotated_line_starts_at_anchor_not_gutter() {
        // One annotated column-0 line, point at char 3 (display col 3 in the
        // code). The code sits at cell 1 (the anchor shifted it right by
        // one), so the terminal cursor is at col 1 + 3 = 4; the note row
        // above occupies slice row 0, so the code row is at terminal row
        // 1 (title) + 1 = 2.
        let snap = annotated_snapshot(
            &[("fn target_one() {}", true)],
            &[0],
            0, 3, 1,
        );
        assert_eq!(
            cursor_cell(&snap),
            Some((4, 2)),
            "annotated column-0: code start (1) + display_col(3) = 4; note row above → terminal row 2"
        );
        // Point at char 0 → col 1 (the anchor shifted the line right by one;
        // NOT the old 2-cell gutter).
        let snap = annotated_snapshot(&[("fn target_one() {}", true)], &[0], 0, 0, 1);
        assert_eq!(cursor_cell(&snap), Some((1, 2)), "char 0 → col 1 (the 1-cell shift); note row above");
        // Non-annotated line: code at column 0, no shift.
        let snap = annotated_snapshot(&[("plain line", false)], &[], 0, 3, 1);
        assert_eq!(cursor_cell(&snap), Some((3, 1)), "non-annotated: code at column 0");
    }

    /// issue-annotations-anchor-at-symbol: an INDENTED annotated line's code
    /// does NOT move — the anchor borrows a cell of the line's own
    /// indentation, so the cursor's code-start column is the line's own
    /// indentation width (here 4 spaces → the code at display col 4), not a
    /// fixed gutter. This is the mirror of the column-0 test above and is
    /// what catches a refactor that re-introduces a constant leading width.
    /// issue-annotations-symbol-precise: the fixture now carries the
    /// MID-LINE anchor shape (the record's symbol is mid-line: anchors [7],
    /// the cell before the symbol — code_start stays 4, the cursor math
    /// reads only `code_start`, never the anchor set).
    #[test]
    fn cursor_cell_annotated_indented_line_code_does_not_move() {
        // `    fn deep() {}` (4 leading spaces): the store strips the run,
        // sets code_start = 4, indent_chars = 4, and the cursor is
        // code_start (4) + the point's display offset within the code.
        // The record's indicator sits at the MID-LINE anchor 7 (one cell
        // left of its symbol at display 8) — a fact the cursor math must
        // ignore (it is the renderer's business, not the cursor's).
        let rows = vec![
            FileViewRow {
                line: 0,
                is_note: true,
                annotated: false,
                anchors: vec![7],
                code_start: 0,
                indent_chars: 0,
                text: "a note".to_string(),
                spans: Vec::new(),
                matches: Vec::new(),
                highlight: None,
            },
            FileViewRow {
                line: 0,
                is_note: false,
                annotated: true,
                anchors: vec![7],
                code_start: 4,
                indent_chars: 4,
                text: "fn deep() {}".to_string(), // stripped of the 4 spaces
                spans: Vec::new(),
                matches: Vec::new(),
                highlight: None,
            },
        ];
        let mut snap = buffer_snapshot(&["    fn deep() {}"], 0, 0);
        snap.file_view_rows = rows;
        // Point at char 4 (the `f`, display col 4 of the FULL line) → within
        // the stripped code that's char 0 → cursor at code_start (4) + 0 = 4:
        // the code did not move. Row 2 (note above).
        snap.file_view_point_col = 4;
        assert_eq!(cursor_cell(&snap), Some((4, 2)), "indented: code stays at display col 4");
        // Point at char 5 (the `n`) → stripped char 1 → cursor at 4 + 1 = 5.
        snap.file_view_point_col = 5;
        assert_eq!(cursor_cell(&snap), Some((5, 2)), "indented: point one code char in → col 5");
    }

    /// plan 005 issue 02b regression: a note row above the point must not
    /// push the point's rendered row past the canvas. The rendered slice is
    /// capped so the point is always drawn, and the cursor row equals the
    /// point's rendered row.
    #[test]
    fn cursor_cell_note_row_does_not_overflow_point() {
        // Simulate the capped slice: 20 code rows (lines 1-20), no notes.
        // The original slice [0, 21) had a note on line 0, pushing the total
        // to 22 > 21. The cap reduces to 20 code rows + 0 notes = 20 rows.
        // The point at line 20 is at rendered row 19 (line 1 → row 0, ...
        // line 20 → row 19). Terminal row = 1 + 0 + 19 = 20.
        let mut rows: Vec<FileViewRow> = Vec::new();
        for line in 1..=20 {
            rows.push(FileViewRow {
                line,
                is_note: false,
                annotated: false,
                anchors: Vec::new(),
                code_start: 0,
                indent_chars: 0,
                text: format!("line {}", line),
                spans: Vec::new(),
                matches: Vec::new(),
                highlight: None,
            });
        }
        let snap = Snapshot {
            view: ViewId::Buffer,
            file_view_rows: rows,
            file_view_total_rows: 21,
            file_view_top_line: 1,
            file_view_total_lines: 60,
            file_view_viewport_lines: 21,
            file_view_point_line: 20,
            file_view_point_col: 0,
            quit: false,
            project: String::new(),
            view_name: String::new(),
            pending: String::new(),
            activity: String::new(),
            resolving: String::new(),
            crate_indexing: String::new(),
            message: String::new(),
            home_title: String::new(),
            home_rows: Vec::new(),
            buffer_rows: Vec::new(),
            buffer_list_selected: 0,
            picker: false,
            prompt: String::new(),
            query: String::new(),
            selected: 0,
            candidates: Vec::new(),
            total: 0,
            preview: String::new(),
            magit_rows: Vec::new(),
            magit_top_row: 0,
            magit_total_rows: 0,
            menu_open: false,
            menu_rows: Vec::new(),
            menu_height: 0,
            log_title: String::new(),
            log_rows: Vec::new(),
            blame_title: String::new(),
            blame_rows: Vec::new(),
            commit_diff_title: String::new(),
            commit_diff_rows: Vec::new(),
            commit_diff_top_row: 0,
            commit_diff_total_rows: 0,
            commit_editor_title: String::new(),
            commit_editor_rows: Vec::new(),
            dirty: None,
            file_view_title: String::new(),
            file_view_changed_on_disk: false,
            file_view_current_buffer_editable: false,
            buffer_mode: String::new(),
            tree_visible: false,
            tree_rows: Vec::new(),
            tree_selected: 0,
            terminal_width: 80,
            which_function: String::new(),
            indexing: String::new(),
            search_title: String::new(),
            search_rows: Vec::new(),
            search_top_row: 0,
            search_total_rows: 0,
            search_selected_row: None,
            search_running: false,
            search_error: None,
            searching: String::new(),
            position: String::new(),
            annotations: String::new(),
            region_lines: None,
            region_size: None,
            file_view_notes_folded: false,
        };
        // The point (line 20) is at rendered row 19 (0-indexed in the rows
        // slice). Terminal row = 1 (title) + 0 (banner) + 19 = 20.
        let result = cursor_cell(&snap);
        assert_eq!(result, Some((0, 20)), "point's line IS drawn; cursor on it");
    }

    // ── plan 004 issue 05g: cursor_cell tree-sidebar column offset ────

    /// plan 004 issue 05g: the terminal cursor is in ABSOLUTE screen
    /// coordinates while the view column is PANE-relative — with the tree
    /// visible the column is offset by `TREE_WIDTH` (row unchanged); tree
    /// hidden the column is unchanged (pane col 0 == absolute col 0).
    #[test]
    fn cursor_cell_tree_visible_offsets_column_by_tree_width() {
        let mut snap = buffer_snapshot(&["abcd"], 0, 3);
        snap.tree_visible = true;
        assert_eq!(
            cursor_cell(&snap),
            Some((TREE_WIDTH + 3, 1)),
            "pane col 3 → terminal col 34 + 3 (0-based)"
        );
        // Tree hidden: the column is exactly the pane column.
        let snap = buffer_snapshot(&["abcd"], 0, 3);
        assert_eq!(cursor_cell(&snap), Some((3, 1)), "tree hidden: unchanged");
    }

    /// plan 004 issue 05g: with the tree visible a huge pane column is
    /// clamped to the last terminal column so it cannot address outside
    /// the screen (80-col terminal: max 0-based col 79).
    #[test]
    fn cursor_cell_tree_visible_clamps_to_terminal_width() {
        let long = "A".repeat(200);
        let mut snap = buffer_snapshot(&[&long], 0, 100);
        snap.tree_visible = true;
        assert_eq!(
            cursor_cell(&snap),
            Some((79, 1)),
            "pane col 100 + tree 34 → clamped to terminal col 79"
        );
        // Tree hidden: no clamp (a beyond-pane column may land off-screen,
        // the pre-05g behavior).
        let snap = buffer_snapshot(&[&long], 0, 100);
        assert_eq!(cursor_cell(&snap), Some((100, 1)), "tree hidden: unchanged");
    }

    // ── plan 004 issue 05e: tree-sidebar click offset ──────────────────

    /// Tree hidden: the terminal column maps 1:1 to the code pane's column
    /// and no click lands in the tree.
    #[test]
    fn click_pane_tree_hidden_is_identity() {
        for col in [0usize, 1, 33, 34, 40, 79] {
            assert_eq!(click_pane(false, col), (false, col), "col {col}");
        }
    }

    /// Tree visible: columns `[0, TREE_WIDTH)` are the tree (no code-pane
    /// click); columns `>= TREE_WIDTH` pass through shifted by the tree
    /// width (34 → pane col 0, 40 → pane col 6).
    #[test]
    fn click_pane_tree_visible_offsets_by_tree_width() {
        for col in 0..TREE_WIDTH as usize {
            assert_eq!(click_pane(true, col), (true, 0), "col {col} is the tree");
        }
        assert_eq!(click_pane(true, 34), (false, 0), "pane col 0");
        assert_eq!(click_pane(true, 40), (false, 6), "pane col 6");
        assert_eq!(click_pane(true, 79), (false, 45), "pane col 45");
    }
}

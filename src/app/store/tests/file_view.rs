use super::*;

    #[test]
    fn scroll_line_down_increments_top() {
        let (mut s, _dir) = store_with_lines(100);
        assert_eq!(s.scroll_top(), 0);
        s.scroll_line_down();
        assert_eq!(s.scroll_top(), 1);
        s.scroll_line_down();
        assert_eq!(s.scroll_top(), 2);
    }

    #[test]
    fn scroll_line_up_saturates_at_zero() {
        let (mut s, _dir) = store_with_lines(100);
        s.scroll_line_up();
        assert_eq!(s.scroll_top(), 0, "cannot scroll above top");
    }

    // ── plan 004 issue 05b: file-view point (line, col) + emacs motion ──

    #[test]
    fn point_down_up_preserves_goal_column() {
        let (mut s, _dir) = store_with_varied_lines();
        // Start at line 0, col 5 (goal 5).
        s.set_point(0, 5, 5);
        // C-n to line 1 ("bb", len 2): col clamps to 2, goal stays 5.
        s.point_down();
        assert_eq!((s.point_line(), s.point_col()), (1, 2));
        // C-p back to line 0 (len 12): the goal column (5) is restored.
        s.point_up();
        assert_eq!((s.point_line(), s.point_col()), (0, 5));
    }

    #[test]
    fn point_forward_wraps_at_eol() {
        let (mut s, _dir) = store_with_varied_lines();
        s.set_point(0, 12, 12); // line 0 end (col == len 12)
        s.point_forward(); // wrap to line 1, col 0
        assert_eq!((s.point_line(), s.point_col()), (1, 0));
    }

    #[test]
    fn point_backward_wraps_at_bol() {
        let (mut s, _dir) = store_with_varied_lines();
        s.set_point(1, 0, 0); // line 1, col 0 (BOL)
        s.point_backward(); // wrap to line 0 end (col 12)
        assert_eq!((s.point_line(), s.point_col()), (0, 12));
    }

    #[test]
    fn point_line_start_end() {
        let (mut s, _dir) = store_with_varied_lines();
        s.set_point(2, 5, 5);
        s.point_line_start();
        assert_eq!((s.point_line(), s.point_col()), (2, 0));
        s.point_line_end();
        assert_eq!((s.point_line(), s.point_col()), (2, 12));
    }

    #[test]
    fn point_buffer_start_end() {
        let (mut s, _dir) = store_with_varied_lines();
        s.set_point(1, 1, 1);
        s.point_buffer_end();
        assert_eq!((s.point_line(), s.point_col()), (2, 12));
        s.point_buffer_start();
        assert_eq!((s.point_line(), s.point_col()), (0, 0));
    }

    #[test]
    fn motion_saturates_at_buffer_bounds() {
        let (mut s, _dir) = store_with_varied_lines();
        s.point_buffer_start();
        s.point_up(); // at line 0: no-op
        assert_eq!(s.point_line(), 0);
        s.point_buffer_end();
        s.point_down(); // at the last line: no-op
        assert_eq!(s.point_line(), 2);
    }

    #[test]
    fn motion_window_follows_point() {
        let (mut s, _dir) = store_with_lines(100);
        // Point far from the window: the window must follow to keep it in view.
        s.set_point(90, 0, 0);
        assert!(
            s.scroll_top() <= 90 && 90 < s.scroll_top() + 10,
            "window keeps the point in view: top={}",
            s.scroll_top()
        );
        // Moving back to line 0 scrolls the window up.
        s.set_point(0, 0, 0);
        assert_eq!(s.scroll_top(), 0);
    }

    #[test]
    fn window_scroll_keeps_point_screen_row() {
        let (mut s, _dir) = store_with_lines(100);
        s.set_point(5, 3, 3);
        // The point's screen row is 5 (window top 0). C-v keeps that row.
        let screen_row_before = s.point_line().saturating_sub(s.scroll_top());
        s.scroll_page_down();
        assert_eq!(
            s.point_line().saturating_sub(s.scroll_top()),
            screen_row_before,
            "C-v keeps the point's screen row fixed"
        );
    }

    // ── plan 004 issue 05c: mouse (line,col) click, wheel parity, words ──

    #[test]
    fn mouse_click_sets_point_line_and_col() {
        let (mut s, _dir) = store_with_lines(100);
        // Lines are "lineN" — line 2 is "line2" (5 chars). The click row is
        // 0-based within the visible area (window top 0 here).
        s.mouse_click_position(2, 3);
        assert_eq!((s.point_line(), s.point_col()), (2, 3), "click lands (line, col)");
        // The window stays put: the clicked row is visible.
        assert_eq!(s.scroll_top(), 0);
    }

    #[test]
    fn mouse_click_past_eol_clamps_to_eol() {
        let (mut s, _dir) = store_with_varied_lines();
        // Line 1 is "bb" (len 2): a click far past EOL lands at EOL.
        s.mouse_click_position(1, 99);
        assert_eq!((s.point_line(), s.point_col()), (1, 2), "clamped to EOL");
    }

    #[test]
    fn mouse_click_wide_chars_convert_display_col_to_char_index() {
        // plan 004 issue 05d: the clicked column is a terminal (display)
        // column; 中 (char 9) occupies display cols 9-10, so display col
        // 14 is char 13 ('g'), not char 14.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/wide.rs"), "CJK: abcd中 efgh\nbb\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/wide.rs");
        s.set_viewport_lines(10);
        // A click inside the wide char (either of its cells) maps to it.
        s.mouse_click_position(0, 9);
        assert_eq!((s.point_line(), s.point_col()), (0, 9), "click inside 中 → char 9");
        s.mouse_click_position(0, 10);
        assert_eq!((s.point_line(), s.point_col()), (0, 9), "second cell of 中 → char 9");
        // 'g' sits at display col 14 (one extra cell for 中).
        s.mouse_click_position(0, 14);
        assert_eq!((s.point_line(), s.point_col()), (0, 13), "display col 14 → char 13");
        // A click past the line's total WIDTH (16) clamps to EOL (char 15).
        s.mouse_click_position(0, 99);
        assert_eq!((s.point_line(), s.point_col()), (0, 15), "past total width clamps to EOL");
    }

    #[test]
    fn mouse_click_annotated_line_pins_gutter_column_mapping() {
        // issue-annotations-anchor-at-symbol (re-expressed from the ed7a524
        // gutter pin, NOT deleted): the 2-cell gutter is GONE. `c1` is a
        // COLUMN-0 symbol, so the indicator borrows nothing, takes column 0,
        // and shifts the code right by exactly one — `c` at cell 1, `1` at
        // cell 2. The click mapping's inverse of that shift is the
        // load-bearing math (mirrored in src/ui/root/geometry.rs::
        // cursor_cell): a click on the code's OWN column (cell 1) maps to
        // char 0; a click one cell past it (cell 2) maps to char 1; a click
        // on the indicator cell (cell 0) maps to the symbol's char (char 0,
        // since it is a column-0 line). A mutation that re-introduces a
        // constant leading width (cell 1 -> char 0 still, but cell 2 ->
        // char 0) desyncs the click from the rendered column; cell 2 -> char
        // 1 and cell 3 -> EOL discriminate it. Unannotated lines keep
        // cell 0 -> char 0.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/annpin.rs", "c0\nc1\nc2\n");
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/annpin.rs".to_string(),
            line: 1,
            col: 0,
            anchor: "c1".to_string(),
            text: "note".to_string(),
            orphaned: false,
        }));
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);
        // Rendered rows: 0=c0, 1=note row (above c1), 2=c1 (annotated code
        // row, code at cell 1), 3=c2, 4=trailing empty line.
        // The code's OWN column (cell 1, the `c`) maps to char 0.
        s.mouse_click_position(2, 1);
        assert_eq!((s.point_line(), s.point_col()), (1, 0), "annotated code's own column (cell 1) -> char 0");
        // One cell past it (cell 2, the `1`) maps to char 1 — the mutation
        // that would still treat cell 2 as the leading width (-> char 0) is
        // caught here.
        s.mouse_click_position(2, 2);
        assert_eq!((s.point_line(), s.point_col()), (1, 1), "annotated cell 2 -> char 1");
        // Past the code's EOL (cell 3) clamps to the line end (char 2).
        s.mouse_click_position(2, 3);
        assert_eq!((s.point_line(), s.point_col()), (1, 2), "annotated cell 3 -> EOL char 2");
        // The indicator cell (cell 0) maps to the symbol's char (char 0 for
        // this column-0 line).
        s.mouse_click_position(2, 0);
        assert_eq!((s.point_line(), s.point_col()), (1, 0), "indicator cell 0 -> the symbol's char (0)");
        // Unannotated line: the full-line mapping is unchanged, cell 0 -> char 0.
        s.mouse_click_position(0, 0);
        assert_eq!((s.point_line(), s.point_col()), (0, 0), "unannotated cell 0 -> char 0");
    }

    #[test]
    fn mouse_click_indented_annotated_line_pins_indented_branch() {
        // issue-annotations-anchor-at-symbol P2-1 (re-expressed for
        // issue-annotations-symbol-precise): pin the INDENTED branch of
        // mouse_click_position (the `tail = &t[indent..]` / `indent +
        // display_col_to_char_index(tail, col - code_start)` path). The
        // column-0 pin above
        // (mouse_click_annotated_line_pins_gutter_column_mapping) never
        // enters that branch — for a column-0 line `indent` is 0 and the
        // tail is the whole line, so an indent-BUG there is invisible. This
        // fixture is `    c1` (4-space indent) with a record AT CHAR 0 (the
        // record's symbol is the line's start; it falls back to the indent
        // anchor, display col 3 — the same cell the landed rule used), the
        // code tail `c1` starts at display col 4, and the mapping must add
        // the indent back on for the code region. Expected mapping:
        // display cells 0 / 3 / 4 / 5 / 9 -> chars 4 / 0 / 4 / 5 / 6.
        //
        // issue-annotations-symbol-precise re-verification: the INDICATOR
        // cell (display 3) now maps to the record's OWN column (char 0 —
        // where the annotation was made), not to the line's first token:
        // the indicator guards the record's symbol, and the record's
        // symbol here is the line's start. A blank cell left of the
        // indicator (cell 0) keeps the landed first-token mapping (char 4).
        //
        // MUTATION the pin exists to catch: replacing the indented path
        // with the indent-blind `display_col_to_char_index(&t, col -
        // code_start)` (no `indent +`, no tail) makes THIS test RED — a
        // click on the code's own cell (display col 4) lands at char 0
        // instead of char 4 (the leading spaces get mapped) — while the
        // whole rest of the suite stays GREEN (the column-0 pin still
        // passes, because there `indent` is 0 and the two forms coincide).
        // That contrast is exactly why the pin uses an indented line.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/annpinind.rs", "c0\n    c1\nc2\n");
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/annpinind.rs".to_string(),
            line: 1,
            col: 0,
            anchor: "c1".to_string(),
            text: "note".to_string(),
            orphaned: false,
        }));
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);
        // Rendered rows: 0=c0, 1=note (above c1), 2=c1 (annotated code row,
        // code tail `c1` starts at display col 4, indicator at col 3), 3=c2,
        // 4=trailing empty line. Full line is `    c1` (6 chars, first token
        // at char 4; the record's symbol is char 0).
        // A blank cell LEFT of the indicator (cell 0) keeps the landed
        // first-token mapping: the line's own indentation maps to the first
        // non-whitespace char (char 4).
        s.mouse_click_position(2, 0);
        assert_eq!((s.point_line(), s.point_col()), (1, 4), "indented cell 0 (left of every indicator) -> the first token char 4");
        // The INDICATOR cell (cell 3) maps to the record's OWN column (char
        // 0 — the annotation's column, re-verified for the moved anchor):
        // clicking the indicator lands where the annotation was made.
        s.mouse_click_position(2, 3);
        assert_eq!((s.point_line(), s.point_col()), (1, 0), "indented cell 3 (the indicator cell) -> the annotation's column (char 0)");
        // The code's OWN cell (display col 4, the `c`) maps to char 4 — this
        // is the cell the indent-blind mutation mis-maps to char 0.
        s.mouse_click_position(2, 4);
        assert_eq!((s.point_line(), s.point_col()), (1, 4), "indented cell 4 (the code's own cell) -> char 4, NOT char 0");
        // One cell past it (display col 5, the `1`) maps to char 5.
        s.mouse_click_position(2, 5);
        assert_eq!((s.point_line(), s.point_col()), (1, 5), "indented cell 5 -> char 5");
        // Past the code's EOL (display col 9) clamps to the line end (char 6).
        s.mouse_click_position(2, 9);
        assert_eq!((s.point_line(), s.point_col()), (1, 6), "indented cell 9 -> EOL char 6");
    }

    #[test]
    fn file_view_rows_multi_record_same_line_share_one_anchor() {
        // issue-annotations-symbol-precise (re-expressed from P2-2's
        // "several records on one line share the line's single anchor": that
        // rule is GONE — one indicator per record, at its own anchor). The
        // surviving shared-anchor case: two records on one line that BOTH
        // fall back to the SAME indent anchor (both at char 0 on an
        // indented line) share that ONE indicator — the code row's anchor
        // set is deduplicated, so the renderer draws a single ▴/▸ and
        // folding leaves exactly one ▸. The `A` key path dedupes/edits to
        // one record per line, so this fixture is built BY HAND (two
        // NotesEntry::Record pushes on the same line), not by what the UI
        // happens to allow.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/annmulti.rs", "c0\n    c1\nc2\n");
        // TWO records on line 1 (the indented `    c1`, both at char 0 —
        // both fall back to the indent anchor, display col 3).
        for text in ["first note", "second note"] {
            s.notes_doc.entries.push(NotesEntry::Record(Annotation {
                syntax: None,
                path: "src/annmulti.rs".to_string(),
                line: 1,
                col: 0,
                anchor: "c1".to_string(),
                text: text.to_string(),
                orphaned: false,
            }));
        }
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);

        let rows = s.file_view_rows();
        // EXACTLY ONE annotated code row for the line (the renderer draws
        // one indicator per DISTINCT anchor; both records fall back to the
        // same indent anchor, so there is exactly one).
        let code_rows: Vec<_> = rows.iter().filter(|r| !r.is_note && r.line == 1).collect();
        assert_eq!(code_rows.len(), 1, "one line -> one annotated code row: {rows:?}");
        assert!(code_rows[0].annotated, "the code row is annotated: {rows:?}");
        // `    c1` is 4-space indented: both records fall back to the
        // indent anchor (display col 3) and SHARE it — the code row's
        // anchor set dedups to the single entry.
        assert_eq!(code_rows[0].anchors, vec![3], "both fallback records share the indent anchor (col 3): {rows:?}");
        assert_eq!(code_rows[0].code_start, 4, "the code keeps its source column (4): {rows:?}");

        // TWO note rows, both for line 1, EACH carrying its own record's
        // anchor — here both records resolved to the same fallback anchor,
        // so both ╭ corners sit at col 3 (they stack at the shared
        // indicator).
        let note_rows: Vec<_> = rows.iter().filter(|r| r.is_note && r.line == 1).collect();
        assert_eq!(note_rows.len(), 2, "two records on one line -> two note rows: {rows:?}");
        for n in &note_rows {
            assert_eq!(n.anchors, vec![3], "each note row carries its record's (shared) anchor col 3: {rows:?}");
        }
        // Both note rows sit DIRECTLY ABOVE the code row (record order).
        let code_idx = rows.iter().position(|r| !r.is_note && r.line == 1).unwrap();
        assert!(code_idx >= 2, "both note rows must precede the code row: {rows:?}");
        assert!(rows[code_idx - 2].is_note && rows[code_idx - 2].line == 1, "first-above row is a note for line 1: {rows:?}");
        assert!(rows[code_idx - 1].is_note && rows[code_idx - 1].line == 1, "second-above row is a note for line 1: {rows:?}");

        // Fold: BOTH note rows vanish, but the single annotated code row
        // remains with its single (shared) ▸.
        s.annotate_toggle();
        assert!(s.note_rows_folded(), "toggled to folded");
        let folded = s.file_view_rows();
        let folded_notes: Vec<_> = folded.iter().filter(|r| r.is_note && r.line == 1).collect();
        assert!(folded_notes.is_empty(), "folded: both note rows gone: {folded:?}");
        let folded_code: Vec<_> = folded.iter().filter(|r| !r.is_note && r.line == 1).collect();
        assert_eq!(folded_code.len(), 1, "folded: still exactly one annotated code row: {folded:?}");
        assert!(folded_code[0].annotated, "folded code row stays annotated: {folded:?}");
        assert_eq!(folded_code[0].anchors, vec![3], "folded: the shared anchor (one ▸) survives: {folded:?}");
    }

    #[test]
    fn file_view_rows_mid_line_record_anchors_at_the_symbol() {
        // issue-annotations-symbol-precise: the indicator sits before the
        // SYMBOL the annotation was made on, at its own display column —
        // NOT before the line's first token. A record on a MID-LINE symbol
        // (char 8 of `    let x = 1;` — the `x`) anchors at display col 7
        // (the space before `x`), not at the indent anchor col 3. The code
        // row carries the anchor SET (here the single entry 7), the code
        // keeps its source column (code_start 4), and the note row's ╭
        // carries the same anchor.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/annmid.rs", "c0\n    let x = 1;\nc2\n");
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/annmid.rs".to_string(),
            line: 1,
            col: 8, // char offset of `x` (issue: a CHAR offset, not a display col)
            anchor: "    let x = 1;".to_string(),
            text: "mid note".to_string(),
            orphaned: false,
        }));
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);

        let rows = s.file_view_rows();
        let code_rows: Vec<_> = rows.iter().filter(|r| !r.is_note && r.line == 1).collect();
        assert_eq!(code_rows.len(), 1, "one annotated code row: {rows:?}");
        assert_eq!(code_rows[0].anchors, vec![7], "the mid-line anchor is display col 7 (the cell before `x`), not the indent anchor 3: {rows:?}");
        assert_eq!(code_rows[0].code_start, 4, "the code keeps its source column (4): {rows:?}");
        assert_eq!(code_rows[0].indent_chars, 4, "the leading run is stripped: {rows:?}");
        // The note row carries the SAME anchor (the ╭ sits at col 7, the
        // same cell as the ▴ below it).
        let note_rows: Vec<_> = rows.iter().filter(|r| r.is_note && r.line == 1).collect();
        assert_eq!(note_rows.len(), 1, "one note row: {rows:?}");
        assert_eq!(note_rows[0].anchors, vec![7], "the note row's ╭ anchors at the record's own col 7: {rows:?}");
        // The note row sits directly above the code row.
        let code_idx = rows.iter().position(|r| !r.is_note && r.line == 1).unwrap();
        assert_eq!(rows[code_idx - 1].line, 1, "the note row is directly above the code row: {rows:?}");

        // Fold: the note row vanishes; the code row keeps its mid-line
        // anchor (one ▸ at col 7).
        s.annotate_toggle();
        let folded = s.file_view_rows();
        assert!(folded.iter().all(|r| !r.is_note), "folded: note rows gone: {folded:?}");
        let folded_code: Vec<_> = folded.iter().filter(|r| !r.is_note && r.line == 1).collect();
        assert_eq!(folded_code[0].anchors, vec![7], "folded: the mid-line ▸ survives at col 7: {folded:?}");
    }

    #[test]
    fn file_view_rows_two_records_two_distinct_anchors_two_indicators() {
        // issue-annotations-symbol-precise: one indicator PER ANNOTATION.
        // Two records on one line at TWO DIFFERENT symbols get two
        // indicators at two columns and two note rows, each ╭ at its own
        // anchor — replacing the landed "several records on one line share
        // one anchor" rule. `    a b`: record 1 on `a` (char 4 — first
        // token, anchor 3), record 2 on `b` (char 6 — mid-line, anchor 5).
        // The fixture is a hand-built pair of records (the test pushes the
        // entries directly); the `A` key path now addresses per symbol (not
        // per line), so a line MAY host several records.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann2.rs", "c0\n    a b\nc2\n");
        for (col, text) in [(4usize, "note a"), (6usize, "note b")] {
            s.notes_doc.entries.push(NotesEntry::Record(Annotation {
                syntax: None,
                path: "src/ann2.rs".to_string(),
                line: 1,
                col,
                anchor: "    a b".to_string(),
                text: text.to_string(),
                orphaned: false,
            }));
        }
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);

        let rows = s.file_view_rows();
        let code_rows: Vec<_> = rows.iter().filter(|r| !r.is_note && r.line == 1).collect();
        assert_eq!(code_rows.len(), 1, "one annotated code row: {rows:?}");
        assert_eq!(code_rows[0].anchors, vec![3, 5], "TWO indicators at TWO columns (the anchor SET): {rows:?}");
        assert_eq!(code_rows[0].code_start, 4, "the code keeps its source column: {rows:?}");
        let note_rows: Vec<_> = rows.iter().filter(|r| r.is_note && r.line == 1).collect();
        assert_eq!(note_rows.len(), 2, "two note rows: {rows:?}");
        assert_eq!(note_rows[0].anchors, vec![3], "the first note's ╭ at its record's anchor (3): {rows:?}");
        assert_eq!(note_rows[1].anchors, vec![5], "the second note's ╭ at its record's anchor (5): {rows:?}");
        let code_idx = rows.iter().position(|r| !r.is_note && r.line == 1).unwrap();
        assert!(rows[code_idx - 2].is_note && rows[code_idx - 1].is_note, "both note rows directly above the code row: {rows:?}");

        // Fold: both note rows vanish; the code row keeps BOTH ▸ (one per
        // annotation, at its own anchor).
        s.annotate_toggle();
        let folded = s.file_view_rows();
        let folded_code: Vec<_> = folded.iter().filter(|r| !r.is_note && r.line == 1).collect();
        assert_eq!(folded_code[0].anchors, vec![3, 5], "folded: one ▸ PER ANNOTATION (two, at 3 and 5): {folded:?}");
    }

    /// issue-annotations-layout — the PACKING: two records on one line
    /// whose display-cell footprints do not collide emit ONE note row
    /// (not two), each slot still at its own anchor, and the rendered-row
    /// total counts the packed row once. By char count these notes also
    /// fit (short texts) — the discriminator is the cell-space decision
    /// the packer must take in the wide-char test below.
    #[test]
    fn file_view_rows_disjoint_notes_pack_onto_one_row() {
        // Line 1: `    a` + 10 spaces + `b` — record 1 on `a` (char 4,
        // anchor 3), record 2 on `b` (char 15, anchor 14). Slot 1's
        // footprint is [3, 7) (╭ @3, ─ @4, "aa" @5-6); slot 2's is
        // [14, 18) — disjoint.
        let line = "    a".to_string() + &" ".repeat(10) + "b";
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/annpack.rs", &format!("c0\n{line}\nc2\n"));
        for (col, text) in [(4usize, "aa"), (15usize, "bb")] {
            s.notes_doc.entries.push(NotesEntry::Record(Annotation {
                syntax: None,
                path: "src/annpack.rs".to_string(),
                line: 1,
                col,
                anchor: line.clone(),
                text: text.to_string(),
                orphaned: false,
            }));
        }
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);

        let rows = s.file_view_rows();
        let note_rows: Vec<_> = rows.iter().filter(|r| r.is_note && r.line == 1).collect();
        assert_eq!(note_rows.len(), 1, "disjoint footprints -> ONE packed note row (not two): {rows:?}");
        let packed = &note_rows[0];
        assert_eq!(
            packed.note_slots,
            vec![
                crate::app::store::NoteSlot {
                    anchor: 3,
                    leader: 1,
                    text: "aa".to_string()
                },
                crate::app::store::NoteSlot {
                    anchor: 14,
                    leader: 1,
                    text: "bb".to_string()
                }
            ],
            "each slot at its own anchor, plain single-bend leaders: {rows:?}"
        );
        assert_eq!(packed.anchors, vec![3, 14], "one anchor entry per slot, slot order: {rows:?}");
        assert_eq!(packed.text, "aa", "the row's text is the first slot's text: {rows:?}");
        // The row still sits directly above its code row, and the code row
        // carries both indicators.
        let code_idx = rows.iter().position(|r| !r.is_note && r.line == 1).unwrap();
        assert!(rows[code_idx - 1].is_note, "the packed row is directly above the code row: {rows:?}");
        assert_eq!(rows[code_idx].anchors, vec![3, 14], "the code row keeps its two indicators: {rows:?}");
        // Total rendered rows: 3 buffer lines + 1 packed note row (NOT +2).
        let lines = s.buffers.current_buffer().unwrap().line_count();
        assert_eq!(s.file_view_total_rows(), lines + 1, "the packed row counts ONCE in the total: {rows:?}");

        // Fold: the packed row vanishes (a fold emits no note rows at all),
        // the code row keeps both ▸.
        s.annotate_toggle();
        let folded = s.file_view_rows();
        assert!(
            folded.iter().all(|r| !r.is_note),
            "folded: the packed note row is gone: {folded:?}"
        );
        let folded_code: Vec<_> = folded.iter().filter(|r| !r.is_note && r.line == 1).collect();
        assert_eq!(folded_code[0].anchors, vec![3, 14], "folded: one ▸ per annotation: {folded:?}");
    }

    /// issue-annotations-layout — the FURTHER-OUT rule, pinned at the
    /// store level: when the footprints collide the notes stack, and the
    /// LARGER-display-anchor note (the one further right on the line — the
    /// note whose connector the other note's text can actually reach) is
    /// the one whose leader extends, to start strictly to the right of the
    /// colliding note's text end. Line 1 `    a b`: note 1 on `a`
    /// (anchor 3, "aaaaaaaaa" spans [3, 14)); note 2 on `b` (anchor 5,
    /// "bb" base [5, 9)) collides with note 1's text -> note 2's text
    /// starts at 14 (leader 8), note 1 keeps the plain leader (its text
    /// cannot reach left into note 2's cells).
    #[test]
    fn file_view_rows_overlapping_notes_stack_and_further_out_leader_extends() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/annstack.rs", "c0\n    a b\nc2\n");
        for (col, text) in [(4usize, "aaaaaaaaa"), (6usize, "bb")] {
            s.notes_doc.entries.push(NotesEntry::Record(Annotation {
                syntax: None,
                path: "src/annstack.rs".to_string(),
                line: 1,
                col,
                anchor: "    a b".to_string(),
                text: text.to_string(),
                orphaned: false,
            }));
        }
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);

        let rows = s.file_view_rows();
        let note_rows: Vec<_> = rows.iter().filter(|r| r.is_note && r.line == 1).collect();
        assert_eq!(note_rows.len(), 2, "colliding footprints -> two stacked rows: {rows:?}");
        // The shallower note keeps the plain single-bend leader.
        assert_eq!(
            note_rows[0].note_slots,
            vec![crate::app::store::NoteSlot {
                anchor: 3,
                leader: 1,
                text: "aaaaaaaaa".to_string()
            }],
            "the shallower note is NOT the further-out one — its leader stays plain: {rows:?}"
        );
        // The further-out note: anchor 5, text pushed to 14 (note 1's text
        // end) -> leader = 14 - 5 - 1 = 8.
        assert_eq!(
            note_rows[1].note_slots,
            vec![crate::app::store::NoteSlot {
                anchor: 5,
                leader: 8,
                text: "bb".to_string()
            }],
            "the larger-anchor note's leader extends past the colliding note's text end: {rows:?}"
        );
        assert_eq!(note_rows[1].text, "bb", "the stacked row's text is its slot's text: {rows:?}");
        // Stacked rows still sit directly above the code row (record order:
        // the later record's row is closest to the code).
        let code_idx = rows.iter().position(|r| !r.is_note && r.line == 1).unwrap();
        assert!(rows[code_idx - 1].is_note && rows[code_idx - 2].is_note, "both stacked rows above the code row: {rows:?}");
        let lines = s.buffers.current_buffer().unwrap().line_count();
        assert_eq!(s.file_view_total_rows(), lines + 2, "stacked notes count as two rows: {rows:?}");
    }

    /// issue-annotations-layout — an ORPHANED record packs/stacks by the
    /// same stated rule: the `(orphaned)` suffix is part of the footprint,
    /// and the orphan's anchor still lands per `record_anchor` (orphaned
    /// records never insert a marker cell — gate P3-1 — so this line's
    /// orphan keeps a plain anchor and the rule is the same display-cell
    /// arithmetic for everyone).
    #[test]
    fn file_view_rows_orphaned_note_packs_by_the_same_rule() {
        // Line 1 `    a b`: note 1 on `a` (anchor 3, "aaaaaa" spans
        // [3, 11)); the ORPHAN record on `b` (anchor 5, "cc" + " (orphaned)"
        // = 13 cells -> base [5, 19)) collides -> it stacks, and its text
        // starts at 11 (note 1's text end) with leader 5.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/annorph.rs", "c0\n    a b\nc2\n");
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/annorph.rs".to_string(),
            line: 1,
            col: 4,
            anchor: "    a b".to_string(),
            text: "aaaaaa".to_string(),
            orphaned: false,
        }));
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/annorph.rs".to_string(),
            line: 1,
            col: 6,
            // A non-matching anchor keeps the record orphaned through any
            // re-anchor pass (the exact-text match is what would clear it).
            anchor: "    a bb".to_string(),
            text: "cc".to_string(),
            orphaned: true,
        }));
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);

        let rows = s.file_view_rows();
        let note_rows: Vec<_> = rows.iter().filter(|r| r.is_note && r.line == 1).collect();
        assert_eq!(note_rows.len(), 2, "the orphan's suffixed text collides -> two rows: {rows:?}");
        assert_eq!(
            note_rows[1].note_slots,
            vec![crate::app::store::NoteSlot {
                anchor: 5,
                leader: 5,
                text: "cc (orphaned)".to_string()
            }],
            "the orphan packs/stacks by the same display-cell rule, suffix included: {rows:?}"
        );
        let orphan_rec = ann_records(&s).into_iter().find(|a| a.text == "cc");
        assert!(orphan_rec.is_some_and(|a| a.orphaned), "the record is still flagged orphaned: {rows:?}");
    }

    /// issue-annotations-layout — the SPAN CAP counts PACKED rows, not
    /// records, pinned by the gate's half-blank-pane measurement. 25 lines
    /// each carrying TWO disjoint notes in a 24-row viewport: each line
    /// emits ONE packed note row, so the largest fitting span is
    /// 12 code + 12 note = 24 (the canvas fills). The cap must not count
    /// records — under `line_records.len()` the window reserves two rows
    /// per line, the span shrinks to 8, and the emitter (which packs the
    /// two notes onto one row) emits 8 code + 8 note = 16 rows in a 24-row
    /// pane: the same arithmetic, a half-blank pane. This test is the
    /// mutation's discriminator: reverting the cap to record counts
    /// reddens `code_rows == 12` / `note_rows == 12`.
    ///
    /// The colliding twin pins the other end of the same arithmetic: two
    /// COLLIDING notes per line emit two rows, so the span lands at
    /// 8 code + 16 note = 24 — the packed-row count and the record count
    /// agree there, and the pin keeps the cap from "helping" the colliding
    /// case by under-counting it.
    ///
    /// Rationale, not invariant (gate P3-6 correction): counting packed
    /// rows fills the canvas where the packing allows — it is not a
    /// guarantee that `code_rows + note_rows == viewport_lines`; the
    /// invariant is `<= viewport_lines` and the emitted tail can still be
    /// short of the pane (a past-EOF record counts a row it never draws).
    #[test]
    fn file_view_span_cap_counts_packed_rows_not_records() {
        // 25 lines, each `    a` + 10 spaces + `b` (disjoint note pair,
        // the `file_view_rows_disjoint_notes_pack_onto_one_row` shape,
        // one packed row per line).
        let line = "    a".to_string() + &" ".repeat(10) + "b";
        let content = (0..25).map(|_| line.clone()).collect::<Vec<_>>().join("\n") + "\n";
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/spanpack.rs", &content);
        for l in 0..25 {
            for (col, text) in [(4usize, "aa"), (15usize, "bb")] {
                s.notes_doc.entries.push(NotesEntry::Record(Annotation {
                    syntax: None,
                    path: "src/spanpack.rs".to_string(),
                    line: l,
                    col,
                    anchor: line.clone(),
                    text: text.to_string(),
                    orphaned: false,
                }));
            }
        }
        s.sync_notes_from_doc();
        s.set_viewport_lines(24);
        s.set_scroll_top(0);

        let rows = s.file_view_rows();
        let code_rows = rows.iter().filter(|r| !r.is_note).count();
        let note_rows = rows.iter().filter(|r| r.is_note).count();
        // 24 lines in the window, one packed row each: the largest span
        // with `s + s <= 24` is s = 12 -> 12 code + 12 note = 24 rows.
        assert_eq!(code_rows, 12, "the cap counts the 24 packed rows, reserving 12: the span is 12 code rows: {rows:?}");
        assert_eq!(note_rows, 12, "each of the 12 emitted lines carries its ONE packed row: {rows:?}");
        assert_eq!(rows.len(), 24, "the canvas fills: 12 code + 12 note == the 24-row viewport: {rows:?}");
        // Every emitted note row carries BOTH slots (packing, not stacking):
        // the record-count mutant emits 8 code + 8 note and leaves the
        // lower half of the pane blank.
        assert!(
            rows.iter().filter(|r| r.is_note).all(|r| r.note_slots.len() == 2),
            "every emitted note row is a two-slot packed row: {rows:?}"
        );

        // The colliding twin: the same 25-line shape with notes that
        // COLLIDE (each line packs to TWO rows) -> the largest span with
        // `s + 2s <= 24` is s = 8 -> 8 code + 16 note = 24 rows.
        let mut s = store_with_project();
        let content = (0..25).map(|_| "    a b".to_string()).collect::<Vec<_>>().join("\n") + "\n";
        open_ann_file(&mut s, "src/spancollide.rs", &content);
        for l in 0..25 {
            for (col, text) in [(4usize, "aaaaaaaaaa"), (6usize, "bb")] {
                s.notes_doc.entries.push(NotesEntry::Record(Annotation {
                    syntax: None,
                    path: "src/spancollide.rs".to_string(),
                    line: l,
                    col,
                    anchor: "    a b".to_string(),
                    text: text.to_string(),
                    orphaned: false,
                }));
            }
        }
        s.sync_notes_from_doc();
        s.set_viewport_lines(24);
        s.set_scroll_top(0);

        let rows = s.file_view_rows();
        let code_rows = rows.iter().filter(|r| !r.is_note).count();
        let note_rows = rows.iter().filter(|r| r.is_note).count();
        assert_eq!(code_rows, 8, "colliding notes: two packed rows per line, so the span is 8 code rows: {rows:?}");
        assert_eq!(note_rows, 16, "colliding notes: 8 lines x 2 stacked rows = 16 note rows: {rows:?}");
        assert_eq!(rows.len(), 24, "the canvas fills: 8 code + 16 note == the 24-row viewport: {rows:?}");
    }

    /// issue-annotations-layout — the LEADER-EXTENSION CASCADE, pinned at
    /// the store level: when THREE notes all collide, the third note's
    /// extension runs against the second note's ACTUAL (already-extended)
    /// text end, not its base end. Line 1 `    a b` + 6 spaces + `c`
    /// (a@char 4, b@char 6, c@char 13; all anchor cells are spaces, so
    /// display col == char col and no marker cell is inserted): note 1
    /// on `a` (anchor 3, "aaaaaaaaaa" spans [3, 15)); note 2 on `b`
    /// (anchor 5, "b" base [5, 8)) collides with note 1 -> note 2's text
    /// starts at 15 (note 1's text end), so note 2's ACTUAL end is
    /// 16 — not its base end 8; note 3 on `c` (anchor 12, "cc" base
    /// [12, 16)) collides with both -> its text starts at 16 (note 2's
    /// actual end), leader 3. The emitted slots are [(3,1),(5,9),(12,3)]
    /// (the gate's independent arithmetic). The base-end mutant (extending
    /// against `anchor_j + 2 + width_j`) starts note 3 at 15 — leader 2,
    /// slot (12,2) — and this test reddens on `leader == 3` / text at 16.
    #[test]
    fn file_view_rows_leader_extension_cascades_against_the_actual_end() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/anncascade.rs", "c0\n    a b      c\nc2\n");
        for (col, text) in [(4usize, "aaaaaaaaaa"), (6usize, "b"), (13usize, "cc")] {
            s.notes_doc.entries.push(NotesEntry::Record(Annotation {
                syntax: None,
                path: "src/anncascade.rs".to_string(),
                line: 1,
                col,
                anchor: "    a b      c".to_string(),
                text: text.to_string(),
                orphaned: false,
            }));
        }
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);

        let rows = s.file_view_rows();
        let note_rows: Vec<_> = rows.iter().filter(|r| r.is_note && r.line == 1).collect();
        assert_eq!(note_rows.len(), 3, "all three footprints collide -> three stacked rows: {rows:?}");
        assert_eq!(
            note_rows[0].note_slots,
            vec![crate::app::store::NoteSlot {
                anchor: 3,
                leader: 1,
                text: "aaaaaaaaaa".to_string()
            }],
            "note 1 keeps the plain leader: {rows:?}"
        );
        assert_eq!(
            note_rows[1].note_slots,
            vec![crate::app::store::NoteSlot {
                anchor: 5,
                leader: 9,
                text: "b".to_string()
            }],
            "note 2's text starts at note 1's text end (15) -> leader 9: {rows:?}"
        );
        // The cascade pin: the extension is against the colliding note's
        // ACTUAL end — note 2's text ends at 16 (15 + width 1), so note 3
        // starts at 16: leader = 16 - 12 - 1 = 3, text at 16.
        assert_eq!(
            note_rows[2].note_slots,
            vec![crate::app::store::NoteSlot {
                anchor: 12,
                leader: 3,
                text: "cc".to_string()
            }],
            "note 3 extends against note 2's ACTUAL (extended) end 16, not its base end — a base-end extension gives leader 2 / text at 15 (slot (12,2)): {rows:?}"
        );
        let lines = s.buffers.current_buffer().unwrap().line_count();
        assert_eq!(s.file_view_total_rows(), lines + 3, "three stacked rows count as three: {rows:?}");
    }

    #[test]
    fn file_view_rows_wide_and_tab_records_use_display_columns() {
        // issue-annotations-symbol-precise, the UNITS TRAP pinned at the
        // store level: the record's `col` is a CHAR offset, and the anchor
        // is a DISPLAY column. A symbol after two CJK chars is 2 display
        // columns further right than its char offset; after a tab, up to 7.
        // A char-vs-display error is invisible on ASCII lines and lands the
        // indicator one cell off the symbol only here.
        let mut s = store_with_project();
        // Line 1: `  中中 x = 1;` — `x` at char 5, display col 7 (two CJK =
        // 4 cells). Line 2: `\tz = 3;` — `z` at char 1, display
        // col 8 (the tab advances to the 8-column stop). Line 3: `x\ty = 1;`
        // — `y` at char 2, display col 8 (the MID-LINE tab: x = 1 cell, tab
        // to col 8).
        open_ann_file(&mut s, "src/annwide.rs", "c0\n  中中 x = 1;\n\tz = 3;\nx\ty = 1;\nc4\n");
        for (line, col) in [(1usize, 5usize), (2usize, 1usize), (3usize, 2usize)] {
            s.notes_doc.entries.push(NotesEntry::Record(Annotation {
                syntax: None,
                path: "src/annwide.rs".to_string(),
                line,
                col,
                anchor: "pin".to_string(),
                text: format!("w{line}").to_string(),
                orphaned: false,
            }));
        }
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);

        let rows = s.file_view_rows();
        // Line 1: `x` at display col 7 (char 5) → anchor 6 (the space
        // before it). A char-offset implementation would anchor at 4 — the
        // SECOND CJK char's first cell, one the CJK glyph itself occupies
        // (the units trap, invisible on ASCII lines).
        let l1: Vec<_> = rows.iter().filter(|r| !r.is_note && r.line == 1).collect();
        assert_eq!(l1[0].anchors, vec![6], "CJK-preceded symbol: anchor at DISPLAY col 6 (char-offset 4 would overwrite the second 中 — the units trap): {rows:?}");
        assert_eq!(l1[0].code_start, 2, "the leading run (2 spaces) is the only indent — the CJK chars are code: {rows:?}");
        // Line 2: `z` at display col 8 (the tab stop; char 1) → anchor 7 —
        // the same cell the landed indent-anchor rule would pick, but here
        // it is a rule-1 anchor (the tab IS the whitespace before the
        // symbol).
        let l2: Vec<_> = rows.iter().filter(|r| !r.is_note && r.line == 2).collect();
        assert_eq!(l2[0].anchors, vec![7], "tab-indented symbol: anchor at display col 7 (the last tab cell): {rows:?}");
        assert_eq!(l2[0].code_start, 8, "the code sits at the 8-column tab stop: {rows:?}");
        // Line 3: `y` at display col 8 (x = 1 cell, the mid-line tab to 8;
        // char 2) → anchor 7, and NO shift (the line is unindented but the
        // indicator overwrites a blank cell — the tab's trailing cells).
        let l3: Vec<_> = rows.iter().filter(|r| !r.is_note && r.line == 3).collect();
        assert_eq!(l3[0].anchors, vec![7], "mid-line tab: anchor at display col 7 (char-offset 1 would be inside the tab's blank region — the units trap): {rows:?}");
        assert_eq!(l3[0].code_start, 0, "mid-line tab: NO shift (a blank cell was overwritten, not column 0 taken): {rows:?}");
    }

    #[test]
    fn mouse_click_after_an_inserted_marker_cell_maps_to_the_rendered_char() {
        // gate P2-1: the inserted-cell accounting in the click mapping was
        // COMPLETELY UNPINNED — deleting its subtraction loop left all 945 tests
        // green, because every existing click pin uses `syntax: None`, which can
        // never insert. The code was right; the guard was vacuous.
        // Fixture is the live reproduction, `    map: HashMap<String, u32>,`
        // with a TIED record on the inner `String` (char 17), so the line
        // inserts one marker cell and every later char renders one cell right:
        //   cell 16 = '<' · 17 = the MARKER · 18 = 'S' (char 17) · 24 = ',' (char 23).
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/annins.rs", "c0\n    map: HashMap<String, u32>,\nc2\n");
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: Some(SyntaxAnchor {
                kind: "type_identifier".to_string(),
                name: "String".to_string(),
                scope: None,
            }),
            path: "src/annins.rs".to_string(),
            line: 1,
            col: 17,
            anchor: "    map: HashMap<String, u32>,".to_string(),
            text: "tied".to_string(),
            orphaned: false,
        }));
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);
        // Rows: 0 = c0, 1 = the note row, 2 = the code row.
        s.mouse_click_position(2, 18); // the rendered `S`
        assert_eq!(
            (s.point_line(), s.point_col()),
            (1, 17),
            "one cell right of the marker is the symbol's own char — the inserted-cell offset is subtracted"
        );
        s.mouse_click_position(2, 24); // the rendered `,`
        assert_eq!(
            (s.point_line(), s.point_col()),
            (1, 23),
            "every later cell is exactly one text char behind its rendered cell"
        );
        s.mouse_click_position(2, 17); // the marker's own cell
        assert_eq!(
            (s.point_line(), s.point_col()),
            (1, 17),
            "the marker cell maps to the record's char"
        );
    }

    #[test]
    fn mouse_click_mid_line_indicator_maps_to_the_symbol() {
        // issue-annotations-symbol-precise: re-verify the click/cursor
        // mapping at the moved indicator — clicking an indicator cell maps
        // to the SYMBOL's char (the record's column, the landed semantics),
        // which the indicator now guards MID-LINE, where the plain
        // code-tail mapping would land on its preceding space. Fixture:
        // `    let x = 1;` with a record on `x` (char 8, mid-line anchor 7;
        // code at display col 4, no shift).
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/annclick.rs", "c0\n    let x = 1;\nc2\n");
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/annclick.rs".to_string(),
            line: 1,
            col: 8,
            anchor: "    let x = 1;".to_string(),
            text: "click me".to_string(),
            orphaned: false,
        }));
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);
        // Rendered rows: 0=c0, 1=note row, 2=code row (the annotated line).
        // The indicator cell (display col 7) maps to the symbol's char 8
        // (NOT char 7, the space — the tail mapping's answer; this is the
        // cell the moved indicator newly guards, mid-code).
        s.mouse_click_position(2, 7);
        assert_eq!((s.point_line(), s.point_col()), (1, 8), "mid-line indicator cell 7 -> the symbol's char 8");
        // A blank cell left of EVERY indicator (col 3) keeps the landed
        // first-token semantics: the line's own indentation maps to the
        // first non-whitespace char (char 4, the `l`), not to the record's
        // symbol.
        s.mouse_click_position(2, 3);
        assert_eq!((s.point_line(), s.point_col()), (1, 4), "blank cell 3 (left of every indicator) -> the line's first token char 4");
        // The code's own cell (display col 8, the `x`) maps to char 8 via
        // the code-tail path (col - code_start 4 -> char 4 + 4).
        s.mouse_click_position(2, 8);
        assert_eq!((s.point_line(), s.point_col()), (1, 8), "code cell 8 -> char 8");
        // One cell past it (display col 9, the space) maps to char 9.
        s.mouse_click_position(2, 9);
        assert_eq!((s.point_line(), s.point_col()), (1, 9), "code cell 9 -> char 9 (the space after x)");
        // The code's start cell (display col 4, the `l`) maps to char 4.
        s.mouse_click_position(2, 4);
        assert_eq!((s.point_line(), s.point_col()), (1, 4), "code cell 4 -> char 4");
        // Past the code's EOL (display col 15) clamps to the line end (char 14).
        s.mouse_click_position(2, 15);
        assert_eq!((s.point_line(), s.point_col()), (1, 14), "past EOL -> char 14");
    }

    #[test]
    fn mouse_click_first_token_record_indicator_maps_to_its_column() {
        // issue-annotations-symbol-precise: a record on the line's FIRST
        // TOKEN (char 4 of `    let x = 1;` — the anchor is display col 3,
        // coinciding with the landed indent anchor: the two rules agree on
        // first-token records). Its indicator cell maps to the record's
        // column (char 4), and cells left of it keep the landed mapping
        // (the first non-whitespace char — the same char 4 here).
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/annclickft.rs", "c0\n    let x = 1;\nc2\n");
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/annclickft.rs".to_string(),
            line: 1,
            col: 4,
            anchor: "    let x = 1;".to_string(),
            text: "first token".to_string(),
            orphaned: false,
        }));
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);
        // Rendered rows: 0=c0, 1=note row, 2=code row.
        s.mouse_click_position(2, 3);
        assert_eq!((s.point_line(), s.point_col()), (1, 4), "first-token indicator cell 3 -> the record's column (char 4)");
        s.mouse_click_position(2, 2);
        assert_eq!((s.point_line(), s.point_col()), (1, 4), "blank cell 2 (left of the indicator) -> the first token char 4");
        s.mouse_click_position(2, 4);
        assert_eq!((s.point_line(), s.point_col()), (1, 4), "code cell 4 -> char 4");
    }

    #[test]
    fn mouse_click_column_zero_mid_line_indicator_no_shift() {
        // issue-annotations-symbol-precise: a COLUMN-0 line whose record is
        // MID-LINE does NOT shift (the indicator overwrites a blank cell),
        // and the click mapping must honor that: the line's own cells keep
        // their source columns (col 0 -> char 0, unchanged by any shift),
        // while the indicator cell still maps to the symbol's char.
        // Fixture: `let x = 1;`, record on `x` (char 4, anchor 3, no
        // shift — no column-0 indicator present).
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/annclick0.rs", "let x = 1;\nc2\n");
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/annclick0.rs".to_string(),
            line: 0,
            col: 4,
            anchor: "let x = 1;".to_string(),
            text: "no shift".to_string(),
            orphaned: false,
        }));
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);
        // The row model: anchor 3, code_start 0 (NO shift).
        let rows = s.file_view_rows();
        let code_row = rows.iter().find(|r| !r.is_note && r.line == 0).unwrap();
        assert_eq!(code_row.anchors, vec![3], "mid-line record on a column-0 line: anchor 3");
        assert_eq!(code_row.code_start, 0, "NO shift (the blank cell is overwritten, not column 0 taken)");
        // Rendered rows: 0=note row (above), 1=code row.
        // The indicator cell (3) maps to the symbol's char 4 (the tail
        // mapping would give char 3, the space — the discriminator).
        s.mouse_click_position(1, 3);
        assert_eq!((s.point_line(), s.point_col()), (0, 4), "indicator cell 3 -> the symbol's char 4");
        // Cell 0 is the line's own (unmoved) `l` → char 0 (a shifted line
        // would have the code at 1 and cell 0 as the indicator — the
        // no-shift case keeps source columns).
        s.mouse_click_position(1, 0);
        assert_eq!((s.point_line(), s.point_col()), (0, 0), "cell 0 -> char 0 (the line did not move)");
    }

    #[test]
    fn mouse_click_empty_line_lands_col_zero() {
        let dir = tempfile::tempdir().unwrap();
        // Line 1 is empty.
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/e.rs"), "aaaa\n\nbbbb").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/e.rs");
        s.set_viewport_lines(10);
        s.mouse_click_position(1, 40);
        assert_eq!((s.point_line(), s.point_col()), (1, 0), "empty line → col 0");
    }

    #[test]
    fn mouse_click_maps_through_scroll_top() {
        let (mut s, _dir) = store_with_lines(100);
        // Scroll down, then click the last visible row: row + top = line.
        s.set_scroll_top(50);
        s.mouse_click_position(9, 2);
        assert_eq!(s.point_line(), 59, "click row maps through the window top");
    }

    #[test]
    fn mouse_click_preserves_goal_column() {
        let (mut s, _dir) = store_with_varied_lines();
        // Goal 7 on the 12-char line; the click sets (line 1, col 1) but
        // keeps the goal column (emacs: a mouse set-point does not touch it).
        s.set_point(0, 7, 7);
        s.mouse_click_position(1, 1);
        assert_eq!((s.point_line(), s.point_col()), (1, 1));
        // C-p back to line 0 restores the goal column (7), not the click's 1.
        s.point_up();
        assert_eq!((s.point_line(), s.point_col()), (0, 7), "goal column preserved across the click");
    }

    #[test]
    fn mouse_click_position_noop_outside_buffer_view() {
        let (mut s, _dir) = store_with_lines(100);
        s.push_view(ViewId::BufferList);
        s.mouse_click_position(3, 4);
        assert_eq!((s.point_line(), s.point_col()), (0, 0), "click outside the file view is a no-op");
    }

    // ── plan 004 issue 05e: tree-sidebar click-to-select ────────────────

    #[test]
    fn mouse_wheel_is_a_window_scroll_with_the_screen_row_pinned() {
        // File view: the wheel is the C-v primitive at a 3-line step — the
        // point's screen row is pinned and its buffer line advances only
        // because the window moved under it.
        let (mut s, _dir) = store_with_lines(100);
        s.set_point(5, 2, 2);
        s.mouse_scroll_down();
        assert_eq!(s.scroll_top(), 3, "wheel down scrolls the window 3 lines");
        assert_eq!(s.point_line(), 8, "point line advanced under the window");
        assert_eq!(s.point_line() - s.scroll_top(), 5, "screen row pinned");
        s.mouse_scroll_up();
        assert_eq!(s.scroll_top(), 0, "wheel up scrolls back");
        assert_eq!(s.point_line(), 5, "point line advanced back");
        assert_eq!(s.point_line() - s.scroll_top(), 5, "screen row still pinned");
        // At the top, wheel-up saturates: the window cannot go above 0.
        s.mouse_scroll_up();
        assert_eq!(s.scroll_top(), 0);
        assert_eq!(s.point_line(), 5);
    }

    #[test]
    fn mouse_wheel_clamps_the_point_col_to_the_new_line() {
        // A short line sits three rows down: the screen-row-pinned wheel
        // drag moves the point onto it and re-clamps the col.
        let dir = tempfile::tempdir().unwrap();
        let content = "aaaaaaaaaaaa\naaaaaaaaaaaa\naaaaaaaaaaaa\nbb\ncccccccccccc\ncccccccccccc";
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/wl.rs"), content).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/wl.rs");
        s.set_viewport_lines(10);
        s.set_point(0, 5, 5);
        s.mouse_scroll_down(); // top 3: the point's row 0 drags onto line 3 ("bb")
        assert_eq!((s.point_line(), s.point_col()), (3, 2), "point line advanced, col clamped to the short line");
        s.mouse_scroll_up(); // top 0: back to line 0, col clamped from 2 (≤ 12)
        assert_eq!((s.point_line(), s.point_col()), (0, 2), "wheel back: point line advanced back, col re-clamped");
    }

    #[test]
    fn word_forward_walks_words_and_punctuation() {
        // emacs `forward-word` lands at the END of each word (not its first
        // char). Line 0: "hello world_foo!!" (len 17), line 1: "x",
        // line 2: "ab cd".
        let (mut s, _dir) = store_with_words();
        s.set_point(0, 0, 0);
        s.point_word_forward();
        assert_eq!((s.point_line(), s.point_col()), (0, 5), "end of `hello`");
        s.point_word_forward();
        assert_eq!((s.point_line(), s.point_col()), (0, 15), "skip the space, walk to the end of `world_foo`");
        s.point_word_forward();
        // "!!" + newline are one non-word run: crosses to line 1, then
        // walks `x` to its end (col 1).
        assert_eq!((s.point_line(), s.point_col()), (1, 1), "skip the punctuation run across the newline, end of `x`");
        s.point_word_forward();
        assert_eq!((s.point_line(), s.point_col()), (2, 2), "wrap across lines, end of `ab`");
        s.point_word_forward();
        assert_eq!((s.point_line(), s.point_col()), (2, 5), "skip the space, end of `cd` (buffer end)");
        // At the buffer end M-f is a no-op.
        s.point_word_forward();
        assert_eq!((s.point_line(), s.point_col()), (2, 5), "no-op at buffer end");
    }

    #[test]
    fn word_backward_walks_words_and_punctuation() {
        // emacs `backward-word` lands at the START of each word (not its
        // end). Line 0: "hello world_foo!!", line 1: "x", line 2: "ab cd".
        let (mut s, _dir) = store_with_words();
        // From the end of `cd` (line 2, col 5):
        s.set_point(2, 5, 5);
        s.point_word_backward();
        assert_eq!((s.point_line(), s.point_col()), (2, 3), "start of `cd`");
        s.point_word_backward();
        assert_eq!((s.point_line(), s.point_col()), (2, 0), "skip the space, start of `ab`");
        s.point_word_backward();
        // Cross the newline into line 1, back to the start of `x`.
        assert_eq!((s.point_line(), s.point_col()), (1, 0), "cross the newline, start of `x`");
        s.point_word_backward();
        // Cross to line 0, skip "!!" back to the start of `world_foo` (col 6).
        assert_eq!((s.point_line(), s.point_col()), (0, 6), "cross the newline, skip `!!`, start of `world_foo`");
        s.point_word_backward();
        assert_eq!((s.point_line(), s.point_col()), (0, 0), "skip the space, start of `hello` (buffer start)");
        // At the buffer start M-b is a no-op.
        s.point_word_backward();
        assert_eq!((s.point_line(), s.point_col()), (0, 0), "no-op at buffer start");
    }

    #[test]
    fn word_motion_over_empty_lines() {
        let dir = tempfile::tempdir().unwrap();
        // "word\n\n\nnext" — two empty lines between the words.
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/e2.rs"), "word\n\n\nnext").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/e2.rs");
        s.set_viewport_lines(10);
        s.set_point(0, 0, 0);
        s.point_word_forward();
        assert_eq!((s.point_line(), s.point_col()), (0, 4), "end of `word`");
        s.point_word_forward();
        assert_eq!((s.point_line(), s.point_col()), (3, 4), "M-f skips blank lines to the end of `next`");
        // M-b from the end of `next`: the char before point is a word char,
        // so it lands on `next`'s start (line 3, col 0).
        s.point_word_backward();
        assert_eq!((s.point_line(), s.point_col()), (3, 0), "M-b back to the start of `next`");
        s.point_word_backward();
        assert_eq!((s.point_line(), s.point_col()), (0, 0), "M-b back across the blank lines to the start of `word`");
    }

    #[test]
    fn word_motion_sets_goal_column_to_landing_col() {
        let (mut s, _dir) = store_with_words();
        // Landing on `world_foo`'s end sets the goal column to 15; C-n onto
        // the short line clamps col to 1, and C-p back restores goal 15
        // (clamped to line 0's length — 17 — so 15).
        s.set_point(0, 0, 0);
        for _ in 0..2 {
            s.point_word_forward(); // → (0,15) end of `world_foo`
        }
        s.point_down();
        assert_eq!((s.point_line(), s.point_col()), (1, 1), "C-n clamps to the short line");
        s.point_up();
        assert_eq!((s.point_line(), s.point_col()), (0, 15), "C-p restores the word's landing column");
        // M-b lands on `cd`'s start (line 2, col 3) with goal 3.
        s.set_point(2, 5, 5);
        s.point_word_backward();
        assert_eq!((s.point_line(), s.point_col()), (2, 3));
    }

    #[test]
    fn goto_line_lands_point_at_col_zero() {
        let (mut s, _dir) = store_with_lines(100);
        s.goto_line_start();
        s.goto_line_digit('7');
        s.goto_line_confirm();
        assert_eq!((s.point_line(), s.point_col()), (6, 0));
    }

    #[test]
    fn scroll_page_down_keeps_two_line_overlap() {
        // PART A fix (item 5): a full page keeps a 2-line context overlap
        // (emacs `next-screen-context-lines`), so it advances by
        // `viewport - 2`, not the full viewport.
        let (mut s, _dir) = store_with_lines(100); // viewport 10 → step 8
        s.scroll_page_down();
        assert_eq!(s.scroll_top(), 8);
        s.scroll_page_down();
        assert_eq!(s.scroll_top(), 16);
    }

    #[test]
    fn scroll_page_up_saturates_at_zero() {
        let (mut s, _dir) = store_with_lines(100);
        s.scroll_page_up();
        assert_eq!(s.scroll_top(), 0);
    }

    #[test]
    fn scroll_half_page_down_advances_by_half_viewport() {
        let (mut s, _dir) = store_with_lines(100);
        s.scroll_half_page_down();
        assert_eq!(s.scroll_top(), 5);
        s.scroll_half_page_down();
        assert_eq!(s.scroll_top(), 10);
    }

    #[test]
    fn scroll_half_page_up_saturates_at_zero() {
        let (mut s, _dir) = store_with_lines(100);
        s.scroll_half_page_up();
        assert_eq!(s.scroll_top(), 0);
    }

    #[test]
    fn scroll_to_bottom_clamps_to_last_visible_line() {
        let (mut s, _dir) = store_with_lines(100);
        let total = s.buffers.current_buffer().unwrap().line_count();
        s.scroll_to_bottom();
        assert_eq!(s.scroll_top(), total - 10, "top = total - viewport");
    }

    #[test]
    fn scroll_top_resets_to_zero() {
        let (mut s, _dir) = store_with_lines(100);
        s.scroll_page_down();
        s.scroll_to_top();
        assert_eq!(s.scroll_top(), 0);
    }

    // ── plan 004 issue 05c: recenter-top-bottom contract ─────────────
    // store_with_lines(100) writes 100 "lineN\n" lines → ropey 101 lines;
    // viewport=10 → max_scroll=91, mid=5, top row=0, bottom row=9.
    // `recenter_cycle` selects the position in the emacs `(middle top
    // bottom)` order: index 0 → middle, 1 → top, 2 → bottom, then repeats.
    // It is reset to 0 on any non-recenter command (in `dispatch`), so a
    // fresh C-l goes to MIDDLE. The point's line never moves.

    #[test]
    fn recenter_fresh_goes_to_middle() {
        // A C-l that does not follow another C-l lands the point on the
        // middle row (cycle index 0 → viewport/2).
        let (mut s, _dir) = store_with_lines(100);
        s.set_point(50, 0, 0);
        s.recenter();
        assert_eq!(s.scroll_top(), 45, "fresh C-l → middle (row 5)");
        assert_eq!(s.point_line(), 50, "the point does not move");
    }

    #[test]
    fn recenter_consecutive_cycles_middle_top_bottom() {
        // Consecutive C-l advance middle → top → bottom, purely by cycle
        // index (no zone-derived guess).
        let (mut s, _dir) = store_with_lines(100);
        s.set_point(50, 0, 0);
        s.recenter(); // middle (row 5) → top 45
        assert_eq!(s.scroll_top(), 45);
        s.recenter(); // top (row 0) → top 50
        assert_eq!(s.scroll_top(), 50);
        s.recenter(); // bottom (row 9) → top 41
        assert_eq!(s.scroll_top(), 41);
        assert_eq!(s.point_line(), 50, "the point never moved");
    }

    #[test]
    fn recenter_resets_to_middle_after_other_command() {
        // Any intervening non-recenter command resets the cycle (in
        // `dispatch`), so the next C-l goes to MIDDLE even though the
        // previous C-l had already advanced the cycle.
        let (mut s, _dir) = store_with_lines(100);
        s.set_point(50, 0, 0);
        s.recenter(); // middle → top 45
        s.recenter(); // top → top 50
        assert_eq!(s.scroll_top(), 50);
        s.recenter_cycle = 0; // simulate an intervening command
        s.recenter(); // fresh again → middle → top 45
        assert_eq!(s.scroll_top(), 45);
    }

    #[test]
    fn recenter_resets_cycle_through_dispatch() {
        // The reset happens in `dispatch`: a non-recenter command between
        // two C-l's forces the next C-l back to MIDDLE.
        let (mut s, _dir) = store_with_lines(100);
        s.set_point(50, 0, 0);
        s.dispatch("recenter", None).unwrap(); // fresh → middle
        assert_eq!(s.scroll_top(), 45);
        s.dispatch("recenter", None).unwrap(); // top
        assert_eq!(s.scroll_top(), 50);
        // A non-recenter command resets the cycle to 0.
        s.dispatch("re-walk", None).unwrap();
        s.set_point(50, 0, 0); // restore the point for a clean read
        s.dispatch("recenter", None).unwrap(); // fresh again → middle
        assert_eq!(s.scroll_top(), 45);
    }

    #[test]
    fn recenter_full_cycle_returns_to_start() {
        let (mut s, _dir) = store_with_lines(100);
        s.set_point(50, 0, 0);
        s.recenter(); // middle (row 5)
        assert_eq!(s.scroll_top(), 45);
        s.recenter(); // top (row 0)
        assert_eq!(s.scroll_top(), 50);
        s.recenter(); // bottom (row 9)
        assert_eq!(s.scroll_top(), 41);
        s.recenter(); // middle again (row 5) — the cycle repeats
        assert_eq!(s.scroll_top(), 45);
        s.recenter(); // top again (row 0)
        assert_eq!(s.scroll_top(), 50);
        assert_eq!(s.point_line(), 50, "the point never moved");
    }

    #[test]
    fn recenter_tiny_scroll_ranges_keep_the_point_in_view() {
        // Regression (plan-004-02 review, reworked to the 05c contract):
        // when the buffer barely scrolls (max_scroll in {1,2}) the
        // top/middle/bottom screen rows are unreachable, so the clamps pin
        // the point where it is — C-l must never leave the window off the
        // point and must keep the scroll in range (no dead ends, no
        // out-of-range writes, no oscillation). store_with_lines(n) writes
        // n lines + trailing \n → ropey n+1 lines; viewport=10.
        for n in [10, 11] { // max_scroll in {1, 2}
            let (mut s, _dir) = store_with_lines(n);
            let total = s.buffers.current_buffer().unwrap().line_count();
            let max_scroll = total - 10;
            s.set_point(5, 0, 0);
            for _ in 0..6 {
                s.recenter();
                assert!(
                    s.scroll_top() <= max_scroll,
                    "top in [0, {max_scroll}] (n={n})"
                );
                // The point's screen row must stay inside the viewport.
                let row = s.point_line().saturating_sub(s.scroll_top());
                assert!(row < 10, "point screen row {row} in the viewport (n={n})");
            }
        }
    }

    #[test]
    fn recenter_tiny_viewports_still_cycle() {
        // A 3-row viewport (mid=1, top=0, bottom=2) must cycle without a
        // dead end by index: middle (row 1) → top (row 0) → bottom (row 2)
        // → middle. Rows that collide after clamping simply repeat.
        let (mut s, _dir) = store_with_lines(100);
        s.set_viewport_lines(3);
        s.set_point(50, 0, 0);
        s.recenter(); // middle (row 1) → top 49
        assert_eq!(s.scroll_top(), 49);
        s.recenter(); // top (row 0) → top 50
        assert_eq!(s.scroll_top(), 50);
        s.recenter(); // bottom (row 2) → top 48
        assert_eq!(s.scroll_top(), 48);
        s.recenter(); // middle (row 1) → top 49
        assert_eq!(s.scroll_top(), 49);
        assert_eq!(s.point_line(), 50);
    }

    #[test]
    fn recenter_noop_when_buffer_fits_viewport() {
        // total=5, viewport=10 → max_scroll=0 → no-op.
        let dir = tempfile::tempdir().unwrap();
        let mut c = String::new();
        for i in 0..5 {
            c.push_str(&format!("line_{i}\n"));
        }
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/a.rs"), &c).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.set_viewport_lines(10);
        s.open_path("src/a.rs");
        s.recenter();
        assert_eq!(s.scroll_top(), 0, "no-op when buffer fits viewport");
    }

    // ── plan 004 issue 07: jump-landing recenter (middle row) ───────────
    // These must FAIL against a landing that only minimal-scrolls
    // (`keep_cursor_visible`): that puts a below-window target on the LAST
    // row and an above-window target on the FIRST row; the jump landing
    // recenters to the MIDDLE row (emacs `xref-after-jump-hook` =
    // `(recenter xref-pulse-momentarily)`) without touching `recenter_cycle`.

    #[test]
    fn position_display_top() {
        let (s, _dir) = store_with_lines(100);
        assert_eq!(s.file_view_position_display(), "Top");
    }

    #[test]
    fn position_display_bot() {
        let (mut s, _dir) = store_with_lines(100);
        // M-END (point-buffer-end; jump-ambiguity moved it from M->,
        // which now forces the Xref candidate list): the point lands on
        // the last line; the window follows, so the position display
        // reports "Bot".
        s.point_buffer_end();
        assert_eq!(s.file_view_position_display(), "Bot");
    }

    #[test]
    fn point_buffer_end_reachable_via_mend_and_g() {
        // (jump-ambiguity, test f) the rebind through the KEY PATH:
        // `point-buffer-end` is now `M-END` (it used to be `M->`, which
        // forces the Xref candidate list), and `G` still binds it —
        // nothing becomes unreachable.
        let (mut s, _dir) = store_with_lines(100);
        s.key_event(crate::app::keymap::parse_key("M-END").unwrap());
        assert_eq!(s.point_line(), 100, "M-END moves the point to the last line");
        s.set_point_line(0);
        s.key_event(crate::app::keymap::parse_key("G").unwrap());
        assert_eq!(s.point_line(), 100, "G still binds point-buffer-end");
    }

    #[test]
    fn position_display_middle() {
        let (mut s, _dir) = store_with_lines(100);
        s.set_point_line(50); // line 51 (1-based), 50/101 → 50% (rounded)
        assert_eq!(s.file_view_position_display(), "L51,50%");
    }

    #[test]
    fn position_display_single_line_buffer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "hello\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/a.rs");
        assert_eq!(s.file_view_position_display(), "Top");
    }

    #[test]
    fn position_display_empty_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        // 06a: no current buffer at boot → no position display.
        assert_eq!(s.file_view_position_display(), "");
        // The explicit scratch buffer has an empty rope: line_count()=1,
        // scroll_top=0 → "Top".
        s.open_scratch();
        assert_eq!(s.file_view_position_display(), "Top");
    }

    #[test]
    fn scroll_preserved_across_buffer_switch() {
        let dir = tempfile::tempdir().unwrap();
        let mut c1 = String::new();
        let mut c2 = String::new();
        for i in 0..100 {
            c1.push_str(&format!("a{}\n", i));
            c2.push_str(&format!("b{}\n", i));
        }
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/a.rs"), &c1).unwrap();
        std::fs::write(dir.path().join("src/b.rs"), &c2).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.set_viewport_lines(10);
        s.open_path("src/a.rs");
        s.scroll_page_down();
        assert_eq!(s.scroll_top(), 8, "page down keeps a 2-line overlap");
        s.open_path("src/b.rs");
        assert_eq!(s.scroll_top(), 0);
        s.open_path("src/a.rs");
        assert_eq!(s.scroll_top(), 8, "scroll preserved on return");
    }

    #[test]
    fn goto_line_confirm_jumps_to_line() {
        // PART A fix (item 5): `M-g g` input is 1-based (line N → 0-based
        // scroll top N-1), matching the error message and emacs.
        let (mut s, _dir) = store_with_lines(100);
        s.goto_line_start();
        assert!(s.goto_line_active());
        s.goto_line_digit('5');
        s.goto_line_digit('0');
        assert_eq!(s.goto_line_input(), "50");
        s.goto_line_confirm();
        assert!(!s.goto_line_active());
        assert_eq!(s.point_line(), 49, "line 50 (1-based) → point line 49");

        // Line 1 is the very top.
        s.goto_line_start();
        s.goto_line_digit('1');
        s.goto_line_confirm();
        assert_eq!(s.point_line(), 0, "line 1 → point line 0");
    }

    #[test]
    fn goto_line_zero_is_out_of_range() {
        // 1-based: 0 is below the valid range (1..=total).
        let (mut s, _dir) = store_with_lines(100);
        s.goto_line_start();
        s.goto_line_digit('0');
        s.goto_line_confirm();
        assert!(!s.goto_line_active());
        assert!(s.message.contains("out of range"), "msg: {}", s.message);
    }

    #[test]
    fn goto_line_out_of_range_rejects() {
        let (mut s, _dir) = store_with_lines(100);
        s.goto_line_start();
        s.goto_line_digit('9');
        s.goto_line_digit('9');
        s.goto_line_digit('9');
        s.goto_line_confirm();
        assert!(!s.goto_line_active());
        assert!(s.message.contains("out of range"), "msg: {}", s.message);
    }

    #[test]
    fn goto_line_cancel() {
        let (mut s, _dir) = store_with_lines(100);
        s.scroll_line_down();
        let pre = s.scroll_top();
        s.goto_line_start();
        s.goto_line_digit('5');
        s.goto_line_cancel();
        assert!(!s.goto_line_active());
        assert_eq!(s.scroll_top(), pre, "cancel must not change scroll");
    }

    // ── C15: the unified word-char rule (Unicode, `model::buffer::is_word_char`) ──

    /// Multibyte symbol pin: word motion (M-f/M-b) and the M-?/references
    /// symbol extraction must AGREE on the extent of `café` — `é` is a
    /// word character on both paths, so M-f lands past it (char col 4),
    /// M-b returns to 0, and `symbol_under_point` extracts the full
    /// identifier (not the ASCII prefix `caf`).
    #[test]
    fn multibyte_symbol_word_motion_and_references_agree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/mb.rs"), "café\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/mb.rs");
        s.set_point(0, 0, 0);
        s.point_word_forward();
        assert_eq!((s.point_line(), s.point_col()), (0, 4), "M-f lands past `é`");
        s.point_word_backward();
        assert_eq!((s.point_line(), s.point_col()), (0, 0), "M-b returns to the word start");
        // The M-? path: a point on the word (col 2, on `f`) extracts the
        // same full multibyte identifier the word motion just covered.
        let symbol = crate::search::references::symbol_under_point("café", 2, |_| false)
            .expect("line has an identifier");
        assert_eq!(symbol, "café", "references extraction agrees with word motion");
        // CJK identifier: the same agreement holds for a CJK word.
        let symbol = crate::search::references::symbol_under_point("漢字", 1, |_| false)
            .expect("line has an identifier");
        assert_eq!(symbol, "漢字");
    }


    // ── issue match-highlight: per-row range clipping ─────────────────

    /// (e) A match range that runs past a line's end clips to the line's
    /// byte length (no panic, no overhang), and a match that lives only on
    /// an OFF-SCREEN line produces no range at all — the per-row
    /// computation covers the visible lines only, never a whole-buffer
    /// scan per frame.
    #[test]
    fn match_ranges_beyond_line_end_clip_and_offscreen_lines_are_unscanned() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        // line 0 "aaaa" (bytes 0..4), line 1 "bbbb" (5..9), filler
        // lines 2..19 ("fill\n" ×18, bytes 10..99), line 20 "aaaa"
        // (bytes 100..104).
        let mut content = String::from("aaaa\nbbbb\n");
        for _ in 0..18 {
            content.push_str("fill\n");
        }
        content.push_str("aaaa\n");
        std::fs::write(dir.path().join("src/t.rs"), &content).unwrap();
        let mut s = store(dir.path());
        s.set_viewport_lines(2);
        s.open_path("src/t.rs");
        let key = s.buffers.current().unwrap().to_string();
        // A range that starts in line 0 and runs 8 bytes — overhanging
        // the newline into line 1: it must clip to line 0's length.
        s.match_context = MatchContext {
            buffer_key: key.clone(),
            query: "aaaa\nbb".into(),
            ranges: vec![(0, 8)],
            selected: 0,
        };
        let rows = s.file_view_rows();
        assert_eq!(rows.len(), 2, "the viewport is 2 lines");
        assert_eq!(
            rows[0].matches,
            vec![LineMatch { start: 0, end: 4, selected: true }],
            "the overhanging range clips to the line's byte length"
        );
        assert!(
            rows[1].matches.is_empty(),
            "the range's tail on line 1 never highlights (a range anchors at its start)"
        );
        // The same query's match on the OFF-SCREEN line 20: no range is
        // emitted for it (per-row, visible lines only).
        s.match_context = MatchContext {
            buffer_key: key,
            query: "aaaa".into(),
            ranges: vec![(0, 4), (100, 104)],
            selected: 1,
        };
        let rows = s.file_view_rows();
        let total: usize = rows.iter().map(|r| r.matches.len()).sum();
        assert_eq!(
            total, 1,
            "only the visible line's range is emitted, not the off-screen match"
        );
        assert_eq!(
            rows[0].matches,
            vec![LineMatch { start: 0, end: 4, selected: false }],
            "the SELECTED match is the off-screen one: the visible range stays plain"
        );
    }

    // ── issue-clipboard-and-selection part 2: left drag-select ───────────

    /// The drag path: Down at row 2 (the unchanged click point-set) arms
    /// the drag; DRAG to row 5 rebuilds the WHOLE-LINE region 2..=5 (mark
    /// at the press line's start, point at the drag line's EOL); the
    /// release PERSISTS the region (the emacs mark survives mouse-up) and
    /// disarms the drag; M-w then copies exactly the highlighted lines —
    /// kill ring AND the matching OSC 52.
    #[test]
    fn drag_press_drag_releases_and_mw_copies_the_highlighted_lines() {
        let (mut s, _dir) = store_with_lines(10);
        let mut rx = s.take_clipboard_rx().expect("clipboard rx");
        // Down at row 2, col 3: the point-set (the existing click
        // contract) + the drag arm.
        s.mouse_click_position(2, 3);
        assert_eq!((s.point_line(), s.point_col()), (2, 3), "the press sets the point");
        s.mouse_drag_begin(2);
        // DRAG to row 5: whole lines 2..=5, both ends inclusive.
        s.mouse_drag_position(5);
        assert_eq!(
            s.region_line_range(),
            Some((2, 5)),
            "the region is the whole lines between press and drag"
        );
        assert_eq!((s.point_line(), s.point_col()), (5, 5), "point at line 5's EOL");
        let bkey = s.buffers.current().unwrap().to_string();
        let start = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(2).unwrap();
        // The drag line's EOL: line 5's start + its 5 chars — the region
        // ends BEFORE line 5's trailing newline (the standard EOL point).
        let end = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(5).unwrap() + 5;
        assert_eq!(
            s.region_byte_range(),
            Some((start, end)),
            "mark = press line start; point = drag line EOL (before its newline)"
        );
        // Up: disarms the drag; the region PERSISTS — a late drag is inert.
        s.mouse_drag_end();
        s.mouse_drag_position(8);
        assert_eq!(
            s.region_line_range(),
            Some((2, 5)),
            "release persists the region; a post-release drag must not extend it"
        );
        // M-w: exactly the highlighted region, into BOTH sinks.
        s.key_event(key("M-w"));
        let region_text = "line2\nline3\nline4\nline5";
        assert_eq!(s.kill_ring.top(), Some(region_text), "kill ring: the drag's region");
        assert_eq!(
            rx.try_recv().ok(),
            Some("\u{1b}]52;c;bGluZTIKbGluZTMKbGluZTQKbGluZTU=\u{7}".to_string()),
            "OSC 52: the base64 of exactly the region's UTF-8"
        );
    }

    /// An upward drag selects the same lines (the mark/point normalise via
    /// the region byte range).
    #[test]
    fn drag_upward_selects_the_same_lines() {
        let (mut s, _dir) = store_with_lines(10);
        s.mouse_click_position(5, 0);
        s.mouse_drag_begin(5);
        s.mouse_drag_position(2);
        assert_eq!(s.region_line_range(), Some((2, 5)), "upward drag: same line span");
        s.mouse_drag_end();
    }

    /// The pinned click contract: Down + Up with no DRAG in between is a
    /// plain point-set — no region (the drag arm alone creates nothing).
    /// A plain click after a drag sets the point; the region is the LIVE
    /// mark..point span (the mark stays where the drag put it, so the
    /// region re-spans to the new point — standard emacs region
    /// semantics, and the highlight always matches what M-w will copy).
    #[test]
    fn plain_left_click_sets_point_and_never_a_region() {
        let (mut s, _dir) = store_with_lines(10);
        s.mouse_click_position(4, 2);
        s.mouse_drag_begin(4);
        s.mouse_drag_end();
        assert_eq!((s.point_line(), s.point_col()), (4, 2), "the click sets the point");
        assert_eq!(s.region_byte_range(), None, "no mark → no region");
        // A click after a drag: the point moves; the region is now the
        // live mark..point span (mark on line 2's start, point on line 7).
        s.mouse_click_position(2, 0);
        s.mouse_drag_begin(2);
        s.mouse_drag_position(3);
        s.mouse_drag_end();
        assert_eq!(s.region_line_range(), Some((2, 3)));
        s.mouse_click_position(7, 1);
        s.mouse_drag_begin(7);
        s.mouse_drag_end();
        assert_eq!((s.point_line(), s.point_col()), (7, 1), "the plain click sets the point");
        assert_eq!(
            s.region_line_range(),
            Some((2, 7)),
            "live mark..point region: the mark survived the click, the point moved"
        );
    }

    /// Tree/picker drags must not create a region. The root hook's tree
    /// branch disarms (mouse_drag_end + tree_click_row, never
    /// mouse_drag_begin); a drag that arrives after that disarm is inert.
    #[test]
    fn drag_in_tree_does_not_create_region() {
        let (mut s, _dir) = store_with_lines(10);
        // A code press armed the drag, then a press in the tree (the hook's
        // tree branch) disarmed it; the drag that follows creates nothing.
        s.mouse_click_position(2, 0);
        s.mouse_drag_begin(2);
        s.mouse_drag_end();
        s.mouse_drag_position(6);
        assert_eq!(
            s.region_byte_range(),
            None,
            "the tree press disarms; the later drag creates no region"
        );
        assert_eq!(s.point_line(), 2, "the point stays where the press put it");
    }

    /// With the picker open, a press must NOT arm the drag — a drag over
    /// the picker's candidate rows must not build a code region behind the
    /// overlay.
    #[test]
    fn drag_with_picker_open_does_not_arm() {
        let (mut s, _dir) = store_with_lines(10);
        s.mouse_click_position(2, 0);
        s.open_find_file();
        s.mouse_drag_begin(3);
        s.mouse_drag_position(6);
        s.mouse_drag_end();
        assert_eq!(
            s.region_byte_range(),
            None,
            "picker up: no arm, no region"
        );
        assert!(s.picker_open(), "the picker is still open (untouched)");
    }

    /// A view switch mid-drag disarms (the store guard in
    /// mouse_drag_position): no region, and no re-arm after the switch.
    #[test]
    fn view_switch_mid_drag_disarms() {
        let (mut s, _dir) = store_with_lines(10);
        s.mouse_click_position(2, 0);
        s.mouse_drag_begin(2);
        s.push_view(ViewId::BufferList);
        s.mouse_drag_position(6);
        assert_eq!(
            s.region_byte_range(),
            None,
            "a view switch mid-drag disarms"
        );
        assert_eq!(s.drag_line, None, "the armed state is cleared");
        s.view_stack.pop();
        s.mouse_drag_position(6);
        assert_eq!(s.region_byte_range(), None, "back in the buffer: still unarmed");
    }

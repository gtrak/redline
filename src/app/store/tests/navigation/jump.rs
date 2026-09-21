use super::*;

    #[test]
    fn jump_landing_far_down_lands_on_middle_row() {
        let (mut s, _dir) = store_with_lines(200);
        s.set_viewport_lines(21);
        // Jump DOWN from the top of the file to line 150.
        s.set_point_line(150);
        s.recenter_landing();
        assert_eq!(
            s.scroll_top(),
            140,
            "top = 150 - vp/2; a minimal scroll would leave the point on the last row (top 130)"
        );
    }

    #[test]
    fn jump_landing_far_up_lands_on_middle_row() {
        let (mut s, _dir) = store_with_lines(200);
        s.set_viewport_lines(21);
        // Scroll to the bottom, then jump UP to line 50.
        s.set_point_line(199); // minimal scroll → top 179
        assert_eq!(s.scroll_top(), 179);
        s.set_point_line(50);
        s.recenter_landing();
        assert_eq!(
            s.scroll_top(),
            40,
            "top = 50 - vp/2; a minimal scroll would leave the point on row 0 (top 50)"
        );
    }

    #[test]
    fn jump_landing_short_file_clamps_without_panic() {
        // The buffer barely exceeds the 21-row viewport (the last line is
        // the trailing empty line rope counts after the final newline).
        let (mut s, _dir) = store_with_lines(25);
        s.set_viewport_lines(21);
        s.set_point_line(0);
        s.recenter_landing();
        assert_eq!(s.scroll_top(), 0, "clamped at the top");
        let max_scroll = s.current_line_count() - 21;
        s.set_point_line(s.current_line_count() - 2); // last content line
        s.recenter_landing();
        assert_eq!(
            s.scroll_top(),
            max_scroll,
            "clamped at max_scroll (a middle-row landing is unreachable there)"
        );
        // A buffer that FITS the viewport is a no-op (nothing to recenter):
        // whole file visible, top stays 0, no panic.
        let (mut s2, _dir2) = store_with_lines(10);
        s2.set_viewport_lines(21);
        s2.set_point_line(9);
        s2.recenter_landing();
        assert_eq!(s2.scroll_top(), 0, "fits the viewport: top stays 0");
    }

    #[test]
    fn jump_landing_recenters_even_when_point_is_visible() {
        // emacs recenter repositions the window unconditionally — a jump
        // to a line already in view still lands it on the middle row.
        let (mut s, _dir) = store_with_lines(200);
        s.set_viewport_lines(21);
        s.set_scroll_top(5);
        s.set_point_line(12); // visible at row 7
        s.recenter_landing();
        assert_eq!(
            s.scroll_top(),
            2,
            "window repositioned to the middle row; minimal scroll would keep top 5"
        );
    }

    #[test]
    fn jump_landing_does_not_perturb_recenter_cycle() {
        // `recenter_landing` ITSELF does not touch the cycle: it must not
        // advance it (a jump is not a `C-l`) and must not rely on resetting
        // it. NOTE: in production every key dispatches through `dispatch()`,
        // which resets `recenter_cycle` for any command that is not literally
        // `recenter` (the pre-existing 05c `recenter-last-op` behavior, which
        // matches emacs keying `recenter-top-bottom` off `last-command`) — so
        // this test drives the helpers directly, below `dispatch`, to isolate
        // the helper's own contract.
        let (mut s, _dir) = store_with_lines(200);
        s.set_viewport_lines(21);
        s.set_point_line(50);
        s.recenter(); // fresh → middle: top 40, cycle 1
        assert_eq!(s.scroll_top(), 40);
        s.recenter(); // top: top 50, cycle 2
        assert_eq!(s.scroll_top(), 50);
        // A jump landing in between:
        s.set_point_line(150);
        s.recenter_landing();
        assert_eq!(s.scroll_top(), 140);
        assert_eq!(
            s.recenter_cycle, 2,
            "the jump neither reset nor advanced the cycle"
        );
        s.recenter(); // the cycle continues where it left off: bottom row
        assert_eq!(
            s.scroll_top(),
            130,
            "3rd C-l is the BOTTOM position (150 - (vp-1)); a reset cycle would give middle (140)"
        );
    }

    // ── plan 004 row 11: position display ─────────────────────────────

    #[test]
    fn jump_back_restores_recorded_column() {
        // Regression (user report): M-, must restore the prior position as
        // line + column (its own doc promises it; `current_jump_entry`
        // records `col: point_col()`). The fixture's column is intentionally
        // NONZERO — a col-0 fixture cannot discriminate (the same class of
        // defect that let the isearch col-0 landing survive).
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/t.rs"),
            "first line\nsecond line with words\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        // Record the point at line 1, col 4 (inside "second").
        s.set_point(1, 4, 4);
        let origin = s.current_jump_entry().expect("current buffer has a jump entry");
        assert_eq!(
            (origin.line, origin.col),
            (1, 4),
            "the recorded entry keeps the column"
        );
        // Jump away, then M-, back.
        s.set_point_line(0);
        assert_eq!(s.point_col(), 0, "the forward landing zeroed the column");
        s.record_jump(Some(origin), "M-.");
        s.jump_back();
        assert_eq!(s.point_line(), 1, "M-, restores the recorded line");
        assert_eq!(s.point_col(), 4, "M-, restores the recorded column");
    }

    #[test]
    fn jump_stack_back_forward_round_trip() {
        let mut stack = JumpStack::default();
        let p0 = jump_entry("a.rs", 0);
        let d1 = jump_entry("b.rs", 10);
        let d2 = jump_entry("c.rs", 20);

        // First jump: P0 → D1.
        stack.record_jump(&p0, &d1);
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.back().unwrap().line, 0, "back: P0");
        assert_eq!(stack.forward().unwrap().line, 10, "forward: D1");

        // Second jump: D1 → D2 (from D1, which is the current position).
        // We need to simulate being at D1: pos is now 1 (after forward).
        // record_jump truncates to pos+1 = 2, so it keeps [P0, D1] and adds D2.
        stack.record_jump(&d1, &d2);
        assert_eq!(stack.len(), 3);

        // Back twice: D1, then P0.
        assert_eq!(stack.back().unwrap().line, 10, "back: D1");
        assert_eq!(stack.back().unwrap().line, 0, "back: P0");

        // Forward twice: D1, then D2.
        assert_eq!(stack.forward().unwrap().line, 10, "forward: D1");
        assert_eq!(stack.forward().unwrap().line, 20, "forward: D2");

        // At the end: forward is None.
        assert!(stack.forward().is_none());
    }

    #[test]
    fn jump_stack_back_at_start_returns_none() {
        let mut stack = JumpStack::default();
        let p0 = jump_entry("a.rs", 0);
        let d1 = jump_entry("b.rs", 10);
        stack.record_jump(&p0, &d1);
        // Go back to the start.
        stack.back();
        assert!(stack.back().is_none(), "no further back");
    }

    #[test]
    fn jump_stack_new_jump_truncates_forward_history() {
        let mut stack = JumpStack::default();
        let p0 = jump_entry("a.rs", 0);
        let d1 = jump_entry("b.rs", 10);
        let d2 = jump_entry("c.rs", 20);
        let d3 = jump_entry("d.rs", 30);

        // P0 → D1 → D2.
        stack.record_jump(&p0, &d1);
        // Now at D1 (pos=1). Jump D1 → D2.
        stack.record_jump(&d1, &d2);
        assert_eq!(stack.len(), 3);

        // Go back to D1 (pos=1).
        stack.back();
        // New jump from D1: D1 → D3. Truncates D2 from forward history.
        stack.record_jump(&d1, &d3);
        assert_eq!(stack.len(), 3, "D2 was truncated: [P0, D1, D3]");

        // Back: D1, then P0.
        assert_eq!(stack.back().unwrap().line, 10);
        assert_eq!(stack.back().unwrap().line, 0);
        // Forward: D1, then D3 (not D2).
        assert_eq!(stack.forward().unwrap().line, 10);
        assert_eq!(stack.forward().unwrap().line, 30);
    }

    // ── issue 05: xref tests (finding #2) ─────────────────────────────

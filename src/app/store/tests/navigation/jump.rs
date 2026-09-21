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

    // ── jump-highlight: the transient landing highlight ────────────────

    /// (a) M-. sets a landing highlight whose range covers the DEFINITION
    /// symbol: a unique M-. jump lands on the definition's name column and
    /// the recorded range is exactly that name's byte extent on the line
    /// (a char-based or whole-line extent would fail this).
    #[test]
    fn m_dot_sets_landing_highlight_over_definition() {
        let content = "fn alpha() {}\nfn beta() {\n    alpha();\n}\n";
        let (mut s, _dir) = store_with_index(&[("src/a.rs", content)]);
        s.open_path("src/a.rs");
        // Point on the `alpha` reference inside beta (line 2, col 5).
        s.set_point(2, 5, 5);
        let key = s.buffers.current().map(String::from).unwrap();
        s.dispatch("xref-find-definitions", None).unwrap();

        let h = s
            .jump_highlight()
            .expect("M-. landing must set a landing highlight");
        assert_eq!(h.buffer_key, key, "the highlight belongs to the landed buffer");
        assert_eq!(h.line, 0, "the highlight is on the definition line");
        // Line 0 is `fn alpha() {}`: the name `alpha` is bytes 3..8.
        let line = s.buffers.get(&key).unwrap().line_text(0).unwrap();
        assert_eq!(&line[h.start..h.end], "alpha");
        assert_eq!((h.start, h.end), (3, 8), "the range is the symbol's byte extent");
    }

    /// (b) M-, sets a landing highlight covering the ORIGIN symbol (the
    /// named case: on M-, the symbol highlighted is the one you RETURN
    /// TO — the reference you jumped from, not the definition you leave).
    #[test]
    fn m_comma_sets_landing_highlight_over_origin() {
        let content = "fn alpha() {}\nfn beta() {\n    alpha();\n}\n";
        let (mut s, _dir) = store_with_index(&[("src/a.rs", content)]);
        s.open_path("src/a.rs");
        s.set_point(2, 5, 5); // on the `alpha` reference
        s.dispatch("xref-find-definitions", None).unwrap();
        // Back to the reference: the origin symbol must be highlighted.
        s.dispatch("jump-back", None).unwrap();

        let h = s
            .jump_highlight()
            .expect("M-, landing must set a landing highlight");
        assert_eq!(h.line, 2, "the highlight is on the ORIGIN line (the reference)");
        let key = h.buffer_key.clone();
        let line = s.buffers.get(&key).unwrap().line_text(2).unwrap();
        // Line 2 is `    alpha();`: the name is bytes 4..9.
        assert_eq!((h.start, h.end), (4, 9));
        assert_eq!(&line[h.start..h.end], "alpha");
    }

    /// (c) The highlight lives ONE command: the next command dispatch
    /// CLEARS it, and a second jump REPLACES it (same rule, uniform).
    #[test]
    fn landing_highlight_cleared_by_next_command_and_replaced_by_jump() {
        let content = "fn alpha() {}\nfn beta() {\n    alpha();\n}\n";
        let (mut s, _dir) = store_with_index(&[("src/a.rs", content)]);
        s.open_path("src/a.rs");
        s.set_point(2, 5, 5);
        s.dispatch("xref-find-definitions", None).unwrap();
        assert!(s.jump_highlight().is_some(), "M-. sets the landing highlight");
        // M-, lands back on the reference (the origin symbol).
        s.dispatch("jump-back", None).unwrap();
        let h0 = s
            .jump_highlight()
            .expect("M-, sets a highlight on the origin landing");
        assert_eq!(h0.line, 2, "the origin (reference) line");
        // Any other command clears it (the point-down dispatch).
        s.dispatch("point-down", None).unwrap();
        assert!(
            s.jump_highlight().is_none(),
            "the next command dispatch must clear the landing highlight"
        );
        // A second jump replaces it: forward to the definition again.
        s.dispatch("jump-forward", None).unwrap();
        let h = s.jump_highlight().expect("the second jump replaces the highlight");
        assert_eq!(h.line, 0, "replaced by the definition landing");
    }

    /// (d) Multibyte: the landing range is the right BYTE extent for the
    /// right CHARS. `café` is the symbol (é is a word char); JumpEntry.col
    /// is a 0-based CHAR index while the span layer is byte offsets — the
    /// recorded range must be the symbol's byte range, verified against the
    /// actual bytes of the line (a byte-as-char or char-as-byte mix-up
    /// lands off-by-N).
    #[test]
    fn multibyte_landing_highlight_is_the_right_byte_extent() {
        let content = "fn caf\u{e9}() {}\ncaf\u{e9}();\n";
        let (mut s, _dir) = store_with_index(&[("src/mb.rs", content)]);
        s.open_path("src/mb.rs");
        // Point on the `café` reference (line 1, char col 2).
        s.set_point(1, 2, 2);
        s.dispatch("xref-find-definitions", None).unwrap();

        let h = s
            .jump_highlight()
            .expect("multibyte M-. landing must set a landing highlight");
        assert_eq!(h.line, 0);
        let key = h.buffer_key.clone();
        let line = s.buffers.get(&key).unwrap().line_text(0).unwrap();
        // Line 0 is `fn café() {}`: `café` starts at char 3 / byte 3 and
        // spans 5 bytes (c a f + 2-byte é) -> bytes 3..8.
        assert_eq!((h.start, h.end), (3, 8), "byte extent of the multibyte symbol");
        assert_eq!(&line[h.start..h.end], "caf\u{e9}", "the highlighted bytes ARE the symbol");
        // And the range must be strictly inside the line (never whole-line).
        assert!(h.end < line.len());
    }

    /// (e) The search sentinel landing (SEARCH_JUMP_KEY) returns to the
    /// results view — there is no buffer symbol there, so it must set NO
    /// buffer highlight.
    #[test]
    fn search_sentinel_landing_sets_no_buffer_highlight() {
        let content = "fn alpha() {}\nfn beta() {\n    alpha();\n}\n";
        let (mut s, _dir) = store_with_index(&[("src/a.rs", content)]);
        s.open_path("src/a.rs");
        s.set_point(2, 5, 5);
        let cur = s.current_jump_entry().unwrap();
        let sentinel = JumpEntry {
            buffer_key: SEARCH_JUMP_KEY.to_string(),
            line: 0,
            col: 0,
            label: "*search*".to_string(),
        };
        // The search-RET stack shape (two records: pre-search -> sentinel,
        // then sentinel -> the file landing), so the first M-, lands the
        // sentinel.
        s.jump_stack.record_jump(&cur, &sentinel);
        let file = s.current_jump_entry().unwrap();
        s.jump_stack.record_jump(&sentinel, &file);
        // M-, through the sentinel lands in the results view.
        s.jump_back();
        assert_eq!(s.top_view(), ViewId::Search, "the sentinel landing opens the results view");
        assert!(
            s.jump_highlight().is_none(),
            "the sentinel landing must set no buffer highlight"
        );
    }

    /// (f) A landing at a point with no symbol (an empty line) sets NO
    /// highlight — never an empty or whole-line range.
    #[test]
    fn no_symbol_landing_sets_no_highlight() {
        let (mut s, _dir) = store_with_index(&[("src/a.rs", "fn alpha() {}\n\n")]);
        s.open_path("src/a.rs");
        let cur = s.current_jump_entry().unwrap();
        let key = s.buffers.current().map(String::from).unwrap();
        // A jump landing on the EMPTY line (1, 0).
        let dest = JumpEntry {
            buffer_key: key,
            line: 1,
            col: 0,
            label: "test".to_string(),
        };
        // Recorded as a forward jump FROM the empty line, so M-, lands
        // the empty line (the back target).
        s.jump_stack.record_jump(&cur, &dest);
        s.jump_stack.record_jump(&dest, &cur);
        s.jump_back();
        assert!(
            s.jump_highlight().is_none(),
            "a whitespace/no-symbol landing sets no highlight (no empty range)"
        );
    }

    /// (g) The intensity curve is a PURE function (no rendering needed):
    /// 1.0 at the start, monotone non-increasing, and 0.0 at or before the
    /// duration — sampled across the whole window.
    #[test]
    fn jump_highlight_curve_is_pure_monotone_and_bounded() {
        // 1.0 at the start.
        assert_eq!(jump_highlight_intensity(std::time::Duration::ZERO, true), 1.0);
        // Monotone non-increasing across 20 samples over the duration.
        let mut prev = f32::INFINITY;
        for i in 0..=20 {
            let t = JUMP_HIGHLIGHT_DURATION * i / 20;
            let v = jump_highlight_intensity(t, true);
            assert!(v <= prev, "curve must be monotone non-increasing at sample {i}");
            prev = v;
        }
        // 0.0 at the duration and past it.
        assert_eq!(jump_highlight_intensity(JUMP_HIGHLIGHT_DURATION, true), 0.0);
        assert_eq!(
            jump_highlight_intensity(JUMP_HIGHLIGHT_DURATION + std::time::Duration::from_millis(50), true),
            0.0
        );
        // A midpoint sample is strictly between (a genuine fade, not a step).
        let mid = jump_highlight_intensity(JUMP_HIGHLIGHT_DURATION / 2, true);
        assert!((0.4..0.6).contains(&mid), "midpoint intensity ~0.5, got {mid}");
    }

    /// (i) Without truecolor the palette cannot fade: the highlight HOLDS
    /// at full intensity for the whole duration, then clears (an honest
    /// flash) — instead of a bogus intermediate RGB.
    #[test]
    fn jump_highlight_no_truecolor_holds_then_clears() {
        for i in 1..4 {
            let t = JUMP_HIGHLIGHT_DURATION * i / 4;
            assert_eq!(
                jump_highlight_intensity(t, false),
                1.0,
                "without truecolor the highlight holds at full intensity at {t:?}"
            );
        }
        assert_eq!(
            jump_highlight_intensity(JUMP_HIGHLIGHT_DURATION, false),
            0.0,
            "…then clears exactly at the duration"
        );
    }

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
        // M-> (point-buffer-end): the point lands on the last line; the
        // window follows, so the position display reports "Bot".
        s.point_buffer_end();
        assert_eq!(s.file_view_position_display(), "Bot");
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


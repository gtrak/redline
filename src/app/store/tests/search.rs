use super::*;

    #[test]
    fn isearch_forward_incremental_and_count() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/t.rs"),
            "foo world\nfoo there\nfoo again\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        assert!(s.isearch_active());
        s.isearch_query_char('f');
        assert_eq!(s.isearch_match_count(), 3);
        s.isearch_query_char('o');
        assert_eq!(s.isearch_match_count(), 3);
        s.isearch_query_char('o');
        assert_eq!(s.isearch_match_count(), 3);
    }

    #[test]
    fn isearch_next_prev_wrap() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/t.rs"),
            "aaa\nbbb\naaa\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('a');
        let total = s.isearch_match_count();
        assert!(total >= 3);
        let idx = s.isearch_match_index();
        for _ in 0..total {
            s.isearch_next();
        }
        assert_eq!(s.isearch_match_index(), idx, "next wraps to start");
        s.isearch_prev();
        assert_eq!(s.isearch_match_index(), total, "prev wraps to end");
    }

    #[test]
    fn isearch_cancel_restores_position() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/t.rs"),
            "alpha\nbeta\ngamma\ndelta\nomega\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.set_viewport_lines(10);
        s.scroll_line_down();
        s.scroll_line_down();
        let pre_point = s.point_line();
        assert_eq!(pre_point, 2);
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('o'); // only in "omega" (line 4)
        assert_eq!(s.point_line(), 4, "search must land the point on the match");
        s.isearch_cancel();
        assert!(!s.isearch_active());
        assert_eq!(s.point_line(), pre_point, "cancel must restore the point");
    }

    #[test]
    fn isearch_multibyte_no_panic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "é\né\né\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('\u{e9}'); // é
        assert_eq!(s.isearch_match_count(), 3);
    }

    #[test]
    fn isearch_lands_point_on_match_column() {
        // Regression (user report): isearch moved the cursor to the match's
        // LINE but not the match — `set_point_line` zeroed the column. The
        // match here is at a NONZERO column ("xx omega" → col 3); the
        // existing fixture's match ("omega" at col 0) cannot discriminate.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/t.rs"),
            "alpha\nbeta\ngamma\ndelta\nxx omega\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('o'); // only in "xx omega" (line 4)
        assert_eq!(s.isearch_match_count(), 1);
        assert_eq!(s.point_line(), 4, "the match's line");
        assert_eq!(s.point_col(), 3, "the match's column — not the line start");
        assert_eq!(
            s.file_point().goal_col,
            3,
            "the landing column becomes the goal column (C-n/C-p hold it)"
        );
    }

    #[test]
    fn isearch_lands_on_multibyte_match_column() {
        // Pins the byte→char conversion: in "café omega" the 'o' of
        // "omega" sits at BYTE 6 of the line (é is 2 bytes) but CHAR 5.
        // A byte-based conversion would land the point at col 6 (the 'm').
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "café omega\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('o');
        assert_eq!(s.isearch_match_count(), 1);
        assert_eq!(s.point_line(), 0);
        assert_eq!(s.point_col(), 5, "char index 5 — not byte index 6");
    }

    #[test]
    fn isearch_cancel_restores_column() {
        // Regression (column-landings): C-g restored only the pre-search
        // LINE — `IsearchState` stored no column, so the landing was
        // `set_point_line` (col 0). The pre-search point here is at a
        // NONZERO column, so a line-only restore cannot pass (the
        // existing cancel fixture's point was at col 0).
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/t.rs"),
            "alpha\nxy omega\ngamma\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.set_point(1, 3, 3); // "xy omega": col 3 is the 'o'
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('y'); // only in "xy omega" (line 1)
        assert_eq!(s.isearch_match_count(), 1);
        assert_eq!(s.point_line(), 1, "the search moved the point (col 3 → 0)");
        s.isearch_cancel();
        assert!(!s.isearch_active());
        assert_eq!(s.point_line(), 1, "the pre-search line");
        assert_eq!(s.point_col(), 3, "the pre-search column — not the line start");
        assert_eq!(
            s.file_point().goal_col,
            3,
            "the restored column becomes the goal column"
        );
    }

    #[test]
    fn isearch_cancel_restores_multibyte_column() {
        // Pins the CHAR unit of the recorded column: the pre-search
        // point is "café| omega" — CHAR 4 (just after the é) but BYTE 5.
        // A byte recorded and landed verbatim would put the cursor at
        // col 5 (the 'o'); the old line-only landing at col 0.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "café omega\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.set_point(0, 4, 4);
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('o'); // the match moves the point to col 5
        assert_eq!(s.isearch_match_count(), 1);
        s.isearch_cancel();
        assert_eq!(s.point_line(), 0);
        assert_eq!(s.point_col(), 4, "char index 4 — a recorded byte (5) would land on the 'o'");
    }

    #[test]
    fn isearch_bound_command_letters_extend_query() {
        // Regression: keys that are depth-1 leaf commands in the file view
        // (n, p, l, g, q) must extend the isearch query, not dispatch.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        // Content with "line_5" on line 5 so the full query matches.
        let lines: Vec<String> = (1..=10).map(|i| format!("fn line_{}() {{\n", i)).collect();
        std::fs::write(root.join("src/t.rs"), lines.join("")).unwrap();
        let mut s = store(root);
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        // Type each character of "line_5" via key_event (the bug path).
        for c in "line_5".chars() {
            s.key_event(Key::char(c));
        }
        assert_eq!(s.isearch.query, "line_5", "all printables must extend the query");
        assert!(s.isearch.active);
    }

    #[test]
    fn isearch_match_count_continuity_while_typing() {
        // Match count must update as each character is typed, including
        // bound-command letters (n, p, l, g, q) inside the query.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        // "gamma" is the only line with a 'g'; typing "gamma" one char at a
        // time (including 'n'... wait, no 'n' in gamma). Use "gnome" style:
        // "gnome" appears once; 'g' matches, 'gn' matches, 'gno' matches,
        // 'gnom' matches, 'gnome' matches – all with count 1, and 'n'
        // (the 2nd char) must extend the query, not navigate.
        std::fs::write(
            root.join("src/t.rs"),
            "gnome\nalpha\nbeta\n",
        )
        .unwrap();
        let mut s = store(root);
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.key_event(Key::char('g'));
        assert_eq!(s.isearch.query, "g");
        assert_eq!(s.isearch_match_count(), 1);
        // 'n' is the key that was dropped by the old code – it must extend
        // the query, not call isearch_next().
        s.key_event(Key::char('n'));
        assert_eq!(s.isearch.query, "gn");
        assert_eq!(s.isearch_match_count(), 1);
        s.key_event(Key::char('o'));
        assert_eq!(s.isearch.query, "gno");
        assert_eq!(s.isearch_match_count(), 1);
        s.key_event(Key::char('m'));
        assert_eq!(s.isearch.query, "gnom");
        assert_eq!(s.isearch_match_count(), 1);
        s.key_event(Key::char('e'));
        assert_eq!(s.isearch.query, "gnome");
        assert_eq!(s.isearch_match_count(), 1);
    }

    #[test]
    fn isearch_c_s_c_r_ret_c_g_unchanged() {
        // Chords (C-s, C-r, RET, C-g) keep their isearch semantics after
        // the printable-interception fix.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/t.rs"),
            "foo bar foo baz foo qux\n",
        )
        .unwrap();
        let mut s = store(root);
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.key_event(Key::char('f'));
        s.key_event(Key::char('o'));
        s.key_event(Key::char('o'));
        assert!(s.isearch.active);
        assert_eq!(s.isearch_match_count(), 3);
        // C-s advances to next match.
        let before = s.isearch.current;
        s.key_event(Key::ctrl_char('s'));
        assert_eq!(s.isearch.current, (before + 1) % 3, "C-s must advance");
        assert!(s.isearch.active);
        // C-r moves to previous match.
        s.key_event(Key::ctrl_char('r'));
        assert_eq!(s.isearch.current, before, "C-r must go back");
        assert!(s.isearch.active);
        // RET confirms and deactivates.
        s.key_event(Key::new(KeyCode::Enter));
        assert!(!s.isearch.active);
        // C-g cancels (start a new search first).
        s.isearch_start(IsearchDirection::Forward);
        s.key_event(Key::char('f'));
        assert!(s.isearch.active);
        s.key_event(Key::ctrl_char('g'));
        assert!(!s.isearch.active);
    }

    #[test]
    fn isearch_regression_guard_n_is_not_navigation() {
        // Discriminating test: with the old code, key('n') during isearch
        // called isearch_next() and left the query unchanged. With the fix,
        // it appends 'n' to the query.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/t.rs"), "line_5\nline_5\n").unwrap();
        let mut s = store(root);
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.key_event(Key::char('l'));
        assert_eq!(s.isearch.query, "l");
        s.key_event(Key::char('i'));
        assert_eq!(s.isearch.query, "li");
        // This is the character that was dropped by the old code.
        s.key_event(Key::char('n'));
        assert_eq!(s.isearch.query, "lin", "'n' must extend the query, not navigate");
        s.key_event(Key::char('e'));
        s.key_event(Key::char('_'));
        s.key_event(Key::char('5'));
        assert_eq!(s.isearch.query, "line_5");
        assert_eq!(s.isearch_match_count(), 2);
    }

    /// Watchlist item 3 (the pre-011 artifact): a search-RET whose hit
    /// file cannot be opened (deleted after the walk) keeps the results
    /// view OPEN and reports that the jump did not happen — no jump
    /// entry recorded, no view closed, the current buffer unchanged.
    #[test]
    fn search_jump_failed_open_keeps_the_results_view() {
        let (dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        // The pre-search position (a different file: the failure must
        // not move the current buffer).
        store.open_path("src/main.rs");
        let main_key = store.buffers.current().unwrap().to_string();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);
        assert_eq!(store.search.hits[0].file, "src/lib.rs");
        // Delete the hit file AFTER the walk finished.
        std::fs::remove_file(dir.path().join("src/lib.rs")).unwrap();
        let stack_len_before = store.jump_stack.len();

        store.key_event(key("RET"));
        assert_eq!(store.top_view(), ViewId::Search, "the results view stays open");
        assert_eq!(
            store.buffers.current().map(String::from),
            Some(main_key.clone()),
            "the current buffer is unchanged"
        );
        assert!(store.message.contains("cannot open"), "{:?}", store.message);
        assert!(
            store.message.contains("the jump did not happen"),
            "{:?}",
            store.message
        );
        assert_eq!(
            store.jump_stack.len(),
            stack_len_before,
            "no jump entry on a failed open"
        );
    }

    #[test]
    fn search_jump_lands_match_column() {
        // Regression (column-landings): project-search RET landed via
        // `set_point_line` — zeroing the hit's column although
        // `Hit.col` carries it. The hit here sits at a NONZERO column
        // ("xx omega" → col 3), so the old landing cannot pass.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "xx omega\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        let mut rx = s.search_rx().unwrap();
        s.start_project_search("omega".into());
        drain_search_finished(&mut s, &mut rx);
        assert_eq!(s.search.hits.len(), 1);
        assert_eq!(s.search.hits[0].col, Some(3), "the pipeline's byte column");
        s.key_event(key("RET"));
        assert_eq!(s.point_line(), 0, "the hit's line");
        assert_eq!(s.point_col(), 3, "the hit's column — not the line start");
        assert_eq!(
            s.file_point().goal_col,
            3,
            "the landing column becomes the goal column"
        );
    }

    #[test]
    fn search_jump_lands_multibyte_match_column() {
        // Pins the byte→char conversion: in "café omega" rg reports a
        // BYTE column of 6 (é is 2 bytes); the char column is 5. A
        // byte landing would put the cursor at col 6 (the 'm'); the old
        // line-only landing at col 0.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "café omega\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        let mut rx = s.search_rx().unwrap();
        s.start_project_search("omega".into());
        drain_search_finished(&mut s, &mut rx);
        assert_eq!(s.search.hits.len(), 1);
        assert_eq!(
            s.search.hits[0].col,
            Some(6),
            "byte column after the multibyte prefix"
        );
        s.key_event(key("RET"));
        assert_eq!(s.point_line(), 0);
        assert_eq!(s.point_col(), 5, "char index 5 — not byte index 6 or col 0");
    }

    /// Watchlist item 4: `M-,` under the Search view — the jump-back
    /// pops through the sentinel in ONE step and lands the pre-search
    /// position (the results view closes with the landing — emacs
    /// `xref-pop-marker-stack`); before, the landing moved the buffer
    /// and point underneath the results view: a no-op until the view
    /// was closed by hand.
    #[test]
    fn search_mcomma_pops_the_sentinel_to_the_pre_search_position() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        // The pre-search position: main.rs, line 2.
        store.open_path("src/main.rs");
        store.set_point_line(2);
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);

        // RET: the first hit (lib.rs:1), the results view closes.
        store.key_event(key("RET"));
        assert_eq!(store.top_view(), ViewId::Buffer);
        assert_eq!(store.view_name_display(), "src/lib.rs");

        // M-,: the sentinel — back to the results (selection restored).
        store.key_event(key("M-,"));
        assert_eq!(store.top_view(), ViewId::Search, "the first M-, returns to the results");

        // M-, again: one step through the sentinel to the pre-search
        // position — and the results view closes with the landing.
        store.key_event(key("M-,"));
        assert_eq!(
            store.top_view(),
            ViewId::Buffer,
            "the results view closes with the landing"
        );
        assert_eq!(store.view_name_display(), "src/main.rs", "the pre-search buffer");
        assert_eq!(store.point_line(), 2, "the pre-search line");
    }

    /// The command registry carries the issue 06 commands, and the
    /// keymap resolves the plan's keys (and the freed `C-c p s` prefix).
    #[test]
    fn search_keybindings_resolve() {
        let (_dir, store) = search_project();
        for name in [
            "project-search",
            "references-at-point",
            "occur",
            "search-next",
            "search-prev",
            "search-jump",
            "search-rerun",
            "search-cancel",
            "close-search-view",
        ] {
            assert!(store.registry.get(name).is_some(), "missing `{name}`");
        }
        let resolve = |seq: &[crate::app::keymap::Key]| {
            store
                .engine
                .resolve(seq)
                .and_then(|l| match l {
                    crate::app::keymap::Lookup::Command(c) => Some(c.to_string()),
                    _ => None,
                })
        };
        use crate::app::keymap::Key;
        assert_eq!(
            resolve(&[Key::ctrl_char('c'), Key::char('p'), Key::char('s'), Key::char('s')]),
            Some("project-search".into())
        );
        assert_eq!(resolve(&[Key::alt_char('?')]), Some("references-at-point".into()));
        assert_eq!(
            resolve(&[Key::alt_char('s'), Key::char('o')]),
            Some("occur".into())
        );
        // The strict prefix `C-c p s` is freed for the longer binding.
        assert_eq!(
            resolve(&[Key::ctrl_char('c'), Key::char('p'), Key::char('s')]),
            None
        );
    }

    /// `C-c p s s` opens the prompt; typing + RET runs the search and
    /// lands in the results view with grouped, counted rows.
    #[test]
    fn search_prompt_flow_and_grouped_results() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();

        store.key_event(key("C-c"));
        store.key_event(key("p"));
        store.key_event(key("s"));
        store.key_event(key("s"));
        // Prompt is active: the minibuffer echoes "Search: ".
        assert_eq!(store.message, "Search: ");
        store.key_event(key("t"));
        store.key_event(key("a"));
        store.key_event(key("r"));
        store.key_event(key("g"));
        store.key_event(key("e"));
        store.key_event(key("t"));
        assert_eq!(store.message, "Search: target");
        store.key_event(key("RET"));

        assert_eq!(store.top_view(), ViewId::Search);
        assert!(store.search_running(), "the search must be running");
        drain_search_finished(&mut store, &mut rx);
        assert!(!store.search_running());

        // 4 hits in 2 files: src/main.rs (3) and src/lib.rs (1).
        let (rows, _top, _total, _sel) = store.search_view_info();
        let headers: Vec<&crate::app::store::ResultRow> = rows
            .iter()
            .filter(|r| matches!(r, crate::app::store::ResultRow::Header { .. }))
            .collect();
        assert_eq!(headers.len(), 2, "two file groups: {rows:?}");
        let counts: Vec<u64> = headers
            .iter()
            .map(|h| match h {
                crate::app::store::ResultRow::Header { count, final_count, .. } => {
                    assert!(*final_count);
                    *count
                }
                _ => unreachable!(),
            })
            .collect();
        assert!(counts.contains(&3) && counts.contains(&1), "per-file counts: {counts:?}");
        assert!(store.search_title().contains("4 matches in 2 files"));
    }

    /// n/p move between matches (wrapping); the selection indexes the
    /// flat hit list.
    #[test]
    fn search_next_prev_move_selection() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);
        assert_eq!(store.search.selected, 0);

        store.key_event(key("n"));
        assert_eq!(store.search.selected, 1);
        // Wrap from the last hit back to the first.
        store.search.selected = 3;
        store.key_event(key("n"));
        assert_eq!(store.search.selected, 0);
        store.key_event(key("p"));
        assert_eq!(store.search.selected, 3);
    }

    /// RET jumps to the match (exact line + column recorded on the jump
    /// stack) and `M-,` returns to the results view with the selection
    /// restored.
    #[test]
    fn search_ret_jump_and_mcomma_returns_to_results() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);

        // Hits are sorted deterministically on Finished: (path, line, col).
        // "src/lib.rs" < "src/main.rs", so the first hit is lib.rs:1
        // ("pub fn target() {}", "target" at col 7).
        let first = store.search.hits[0].clone();
        assert_eq!(first.file, "src/lib.rs");
        assert_eq!(first.line_no, 1);
        assert_eq!(first.col, Some(7));

        store.key_event(key("RET"));
        // Landed in the buffer view on the hit's file/line.
        assert_eq!(store.top_view(), ViewId::Buffer);
        assert_eq!(store.view_name_display(), "src/lib.rs");
        assert_eq!(store.scroll_top(), 0); // line 1 (0-based)

        // `M-,` returns to the results view, selection restored.
        store.key_event(key("M-,"));
        assert_eq!(store.top_view(), ViewId::Search, "M-, must return to the results");
        assert_eq!(store.search.selected, 0);
        // The stack is [Buffer, Search] again: q closes back to the buffer.
        store.key_event(key("q"));
        assert_eq!(store.top_view(), ViewId::Buffer);
    }

    /// `g` re-runs the search: results reset, a new generation streams
    /// in, and the same hits reappear.
    #[test]
    fn search_rerun_restarts_the_search() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);
        let old_gen = store.search.generation;

        store.key_event(key("g"));
        assert_eq!(store.search.generation, old_gen + 1, "re-run bumps the generation");
        assert!(store.search_running(), "the re-run must be running");
        drain_search_finished(&mut store, &mut rx);
        let n = store.search.hits.len();
        assert_eq!(n, 4, "the same 4 hits reappear: {n}");
    }

    /// `q` closes the results view (and cancels the in-flight job); the
    /// terminal event reports a cancelled finish.
    #[test]
    fn search_close_cancels_and_closes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        // Enough files that the walk takes real time.
        for i in 0..300 {
            std::fs::write(dir.path().join(format!("f{i:03}.txt")), "needle\n").unwrap();
        }
        let base = tempfile::tempdir().unwrap();
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("needle".into());

        // Close mid-flight.
        store.key_event(key("q"));
        assert_ne!(store.top_view(), ViewId::Search, "q must close the results view");
        // The job's terminal event must report a cancelled finish.
        let start = std::time::Instant::now();
        loop {
            let mut done = false;
            while let Ok(ev) = rx.try_recv() {
                if matches!(ev, crate::search::rg::SearchEvent::Finished { cancelled: true, .. }) {
                    done = true;
                    break;
                }
            }
            if done {
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// `C-g` in the results view cancels the in-flight job WITHOUT
    /// closing the view (the partial results stay on screen); the
    /// terminal event reports a cancelled finish and `search_running`
    /// flips.
    #[test]
    fn search_c_g_cancels_without_closing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        // Enough files that the walk takes real time.
        for i in 0..300 {
            std::fs::write(dir.path().join(format!("f{i:03}.txt")), "needle\n").unwrap();
        }
        let base = tempfile::tempdir().unwrap();
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("needle".into());
        assert!(store.search_running(), "the search must be in flight");

        // C-g cancels mid-flight: the view stays open, the job stops.
        store.key_event(key("C-g"));
        assert_eq!(store.top_view(), ViewId::Search, "C-g must keep the results view open");
        assert_eq!(store.message, "search cancelled");

        // The worker's terminal event must report a cancelled finish.
        let start = std::time::Instant::now();
        loop {
            let mut done = false;
            while let Ok(ev) = rx.try_recv() {
                store.apply_search_event(&ev);
                if matches!(ev, crate::search::rg::SearchEvent::Finished { cancelled: true, .. }) {
                    done = true;
                }
            }
            if done {
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(5), "cancel not observed in 5s");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!store.search_running(), "the job must be stopped");
    }

    /// `C-g` while a picker is open over the results view still closes
    /// the picker (the global intercept), not cancel the search.
    #[test]
    fn search_c_g_with_picker_open_closes_the_picker() {
        let (_dir, mut store) = search_project();
        let _rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        assert_eq!(store.top_view(), ViewId::Search);

        store.open_palette();
        assert!(store.picker_open());
        store.key_event(key("C-g"));
        assert!(!store.picker_open(), "C-g must close the palette");
        // The search itself is untouched (still running or already
        // finished — either way C-g did not cancel it: the message is
        // the generic "cancel", not "search cancelled").
        assert_eq!(store.message, "cancel");
        assert_eq!(store.top_view(), ViewId::Search);
    }

    /// `M-s o` (the two-key sequence) opens the occur prompt; RET runs
    /// the in-buffer regex search, grouped under the buffer's display
    /// name, with the per-file (group) count.
    #[test]
    fn occur_prompt_flow_lists_in_file_matches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("a.txt"), "foo world\nbar foo\nfoo again\nbaz\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());
        let mut rx = store.search_rx().unwrap();
        store.open_path("a.txt");

        store.key_event(key("M-s"));
        store.key_event(key("o"));
        assert_eq!(store.message, "Occur: ");
        store.key_event(key("f"));
        store.key_event(key("o"));
        store.key_event(key("o"));
        store.key_event(key("RET"));

        assert_eq!(store.top_view(), ViewId::Search);
        drain_search_finished(&mut store, &mut rx);
        assert_eq!(store.search.hits.len(), 3, "all in-file matches");
        // One group, named after the buffer's display name.
        let (rows, _, _, _) = store.search_view_info();
        let headers: Vec<_> = rows
            .iter()
            .filter_map(|r| match r {
                crate::app::store::ResultRow::Header { file, count, .. } => {
                    Some((file.clone(), *count))
                }
                _ => None,
            })
            .collect();
        assert_eq!(headers, vec![("a.txt".to_string(), 3)],
            "one group named after the buffer: {headers:?}");
    }

    /// `M-?` searches the identifier under point (line-level point; col 0
    /// until the buffer model has a column cursor).
    #[test]
    fn references_at_point_searches_the_symbol() {
        let (dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        // A plain-text file whose line 1 starts with the symbol (col 0 is
        // on it, so no known-identifier fallback is needed).
        std::fs::write(dir.path().join("src/plain.txt"), "target alpha\n").unwrap();
        store.open_path("src/plain.txt");

        store.key_event(key("M-?"));
        assert_eq!(store.top_view(), ViewId::Search);
        assert_eq!(store.search.query, "target");
        assert_eq!(store.search.kind, crate::app::store::SearchKind::References);
        drain_search_finished(&mut store, &mut rx);
        // The plain-text hit is kept (fallback: .txt has no grammar);
        // the .rs code hits survive the token filter.
        assert!(
            store.search.hits.iter().any(|h| h.file == "src/plain.txt"),
            "the plain-text hit must be kept: {:?}",
            store.search.hits
        );
    }

    /// Events from a stale generation (a superseded job) are discarded;
    /// events from the current generation apply.
    #[test]
    fn search_stale_generation_events_discarded() {
        let (_dir, mut store) = search_project();
        store.start_project_search("one".into());
        let stale_gen = store.search.generation;
        store.start_project_search("two".into());
        let cur_gen = store.search.generation;
        assert_eq!(cur_gen, stale_gen + 1);

        let stale = crate::search::rg::SearchEvent::Hit {
            file: "stale.txt".into(),
            line_no: 1,
            col: None,
            line: "stale".into(),
            generation: stale_gen,
        };
        store.apply_search_event(&stale);
        assert!(store.search.hits.is_empty(), "stale hit must be discarded");

        let fresh = crate::search::rg::SearchEvent::Hit {
            file: "fresh.txt".into(),
            line_no: 1,
            col: None,
            line: "fresh".into(),
            generation: cur_gen,
        };
        store.apply_search_event(&fresh);
        assert_eq!(store.search.hits.len(), 1, "current-generation hit applies");
    }

    // ── issue 08: commit editor tests ───────────────────────────────────────

    #[test]
    fn isearch_c_s_repeats_next_match() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("test.rs"), "foo bar foo baz foo qux\n").unwrap();
        let mut store = store(root);
        store.open_path("test.rs");
        // Start isearch forward.
        store.isearch_start(crate::app::store::IsearchDirection::Forward);
        // Type "foo".
        store.key_event(key("f"));
        store.key_event(key("o"));
        store.key_event(key("o"));
        assert!(store.isearch.active);
        assert_eq!(store.isearch.current, 0, "first match should be current");
        // C-s repeats: next match.
        store.key_event(key("C-s"));
        assert_eq!(store.isearch.current, 1, "C-s must advance to next match");
        store.key_event(key("C-s"));
        assert_eq!(store.isearch.current, 2, "C-s must advance to third match");
    }

    #[test]
    fn isearch_c_r_repeats_prev_match() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("test.rs"), "foo bar foo baz foo qux\n").unwrap();
        let mut store = store(root);
        store.open_path("test.rs");
        // Start isearch backward.
        store.isearch_start(crate::app::store::IsearchDirection::Backward);
        // Type "foo".
        store.key_event(key("f"));
        store.key_event(key("o"));
        store.key_event(key("o"));
        assert!(store.isearch.active);
        // C-r repeats: previous match.
        let current_after_start = store.isearch.current;
        store.key_event(key("C-r"));
        assert!(store.isearch.current != current_after_start || store.isearch.matches.len() == 1,
            "C-r must move the match position");
    }


    // ── issue match-highlight: the store's match context ─────────────

    /// (a) A visible line with two matches produces two match ranges, the
    /// selected one flagged; the flag follows the cursor as isearch
    /// navigates. Discriminates: without the context the rows carry no
    /// ranges at all; with it, ranges stay put while only `selected`
    /// moves.
    #[test]
    fn isearch_visible_line_two_matches_selected_flagged() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "foo a foo b\nzzz\n").unwrap();
        let mut s = store(dir.path());
        s.set_viewport_lines(10);
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('f');
        s.isearch_query_char('o');
        s.isearch_query_char('o');
        let rows = s.file_view_rows();
        assert_eq!(rows[0].line, 0);
        assert_eq!(
            rows[0].matches,
            vec![
                LineMatch { start: 0, end: 3, selected: true },
                LineMatch { start: 6, end: 9, selected: false },
            ],
            "two ranges; the cursor's match (the first) is selected"
        );
        assert!(rows[1].matches.is_empty(), "the matchless line has no ranges");
        // C-s moves the selection: the ranges stay, the flag follows.
        s.isearch_next();
        let rows = s.file_view_rows();
        assert_eq!(
            rows[0].matches,
            vec![
                LineMatch { start: 0, end: 3, selected: false },
                LineMatch { start: 6, end: 9, selected: true },
            ],
            "the selected flag follows the cursor"
        );
    }

    /// (d) A multibyte line: the per-row ranges are BYTE offsets relative
    /// to the line start (the renderer's overlay does the byte→char
    /// conversion) — a byte/char mix-up here would be off-by-N on the
    /// `é` (the same bug class as the isearch column fix). The point
    /// lands at the CHAR column inside the selected range.
    #[test]
    fn isearch_multibyte_line_ranges_are_byte_offsets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        // "café omega": bytes c0 a1 f2 é3-4 ' '5 o6 m7 e8 g9 a10.
        std::fs::write(dir.path().join("src/t.rs"), "café omega\n").unwrap();
        let mut s = store(dir.path());
        s.set_viewport_lines(10);
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('o');
        s.isearch_query_char('m');
        let rows = s.file_view_rows();
        assert_eq!(
            rows[0].matches,
            vec![LineMatch { start: 6, end: 8, selected: true }],
            "byte range 6..8 (not char 5..7) — the line-relative byte domain"
        );
        assert_eq!(s.point_col(), 5, "the point lands at CHAR column 5");
        assert_eq!(s.point_line(), 0);
    }

    /// (b) After a search-results jump the match context covers the hits
    /// in THIS buffer only: a hit in another file must not highlight.
    #[test]
    fn search_jump_context_covers_only_the_current_buffer() {
        let (_dir, mut s) = search_project();
        let mut rx = s.search_rx().unwrap();
        s.open_path("src/main.rs");
        s.set_viewport_lines(10);
        s.start_project_search("target".into());
        drain_search_finished(&mut s, &mut rx);
        assert_eq!(s.search.hits.len(), 4, "3 hits in main.rs + 1 in lib.rs");
        // Jump into main.rs (lib.rs sorts first in the deterministic order).
        let sel = s
            .search
            .hits
            .iter()
            .position(|h| h.file == "src/main.rs")
            .unwrap();
        s.search.selected = sel;
        s.key_event(key("RET"));
        // The context is keyed by the BUFFER KEY (the absolute path — the
        // buffer table's key space), while the hits stay file-relative.
        let key = s.buffers.current().unwrap().to_string();
        assert_eq!(s.match_context.buffer_key, key);
        assert_eq!(s.match_context.query, "target");
        assert_eq!(
            s.match_context.ranges.len(),
            3,
            "only THIS buffer's hits — lib.rs's hit must not highlight"
        );
        // The visible rows carry exactly those 3 ranges (no more).
        let rows = s.file_view_rows();
        let total: usize = rows.iter().map(|r| r.matches.len()).sum();
        assert_eq!(total, 3, "the 3 main.rs hits, none from lib.rs");
    }

    /// (c) The selected (prominent) range is exactly the hit that was
    /// jumped to: the point lands inside it (the user's actual complaint
    /// — the cursor being hard to see where the match highlight is).
    #[test]
    fn search_jump_selected_range_is_the_jumped_hit() {
        let (_dir, mut s) = search_project();
        let mut rx = s.search_rx().unwrap();
        s.open_path("src/main.rs");
        s.set_viewport_lines(10);
        s.start_project_search("target".into());
        drain_search_finished(&mut s, &mut rx);
        // Jump to the main.rs line-3 hit ("target();" at col 0).
        let sel = s
            .search
            .hits
            .iter()
            .position(|h| h.file == "src/main.rs" && h.line_no == 3)
            .unwrap();
        s.search.selected = sel;
        s.key_event(key("RET"));
        assert_eq!(s.point_line(), 2, "landed on the hit's line (0-based 2)");
        assert_eq!(s.point_col(), 0, "the hit's char column");
        let rows = s.file_view_rows();
        let row = &rows[FileViewRow::row_for_line(&rows, 2).unwrap()];
        let sel_match = row
            .matches
            .iter()
            .find(|m| m.selected)
            .expect("a selected range on the jumped line");
        assert_eq!(sel_match.start, 0, "the selected range starts at the hit's byte column");
        assert!(sel_match.end > sel_match.start);
        // The other main.rs hits are flagged plain.
        let others: Vec<&LineMatch> = rows
            .iter()
            .flat_map(|r| r.matches.iter())
            .filter(|m| !m.selected)
            .collect();
        assert_eq!(others.len(), 2, "the 2 non-jumped hits are plain matches");
    }

    /// (P2, gate finding) The jumped hit can carry NO column: a literal
    /// search that matched a line case-insensitively without the literal
    /// spelling makes rg emit `col: None` (rg.rs) while sibling lines in
    /// the same buffer still carry columns. The cursor then sits on no
    /// range at all, so NO range may wear the prominent face — the
    /// contract is "the selected match must be the one the cursor is on".
    /// Regression: `selected` was seeded from the (still empty) `ranges`,
    /// so it defaulted to 0 and the FIRST sibling range took the
    /// prominent face.
    #[test]
    fn search_jump_with_no_column_selects_no_range() {
        use crate::search::rg::Hit;
        let (_dir, mut s) = search_project();
        let mut rx = s.search_rx().unwrap();
        s.open_path("src/main.rs");
        s.set_viewport_lines(10);
        s.start_project_search("target".into());
        drain_search_finished(&mut s, &mut rx);
        // Two hits in the current buffer: a sibling WITH a column, and the
        // jumped-to hit with none (the case-insensitive literal case).
        s.search.query = "target".into();
        s.search.hits = vec![
            Hit {
                file: "src/main.rs".into(),
                line_no: 3,
                col: Some(0),
                line: "target();".into(),
            },
            Hit {
                file: "src/main.rs".into(),
                line_no: 5,
                col: None,
                line: "TARGET".into(),
            },
        ];
        s.search.selected = 1;
        s.key_event(key("RET"));
        let rows = s.file_view_rows();
        let matches: Vec<&LineMatch> = rows.iter().flat_map(|r| r.matches.iter()).collect();
        assert!(
            matches.iter().all(|m| !m.selected),
            "no range may be prominent when the jumped hit has no column: {matches:?}"
        );
        assert_eq!(
            matches.len(),
            1,
            "the sibling hit is still highlighted (plainly): {matches:?}"
        );
    }

    /// (f) The pinned lifetime rule: the highlight tracks the active
    /// isearch; CANCEL clears it; CONFIRM keeps it (the context after the
    /// search ends is the point of the feature); point motion keeps it; a
    /// DIFFERENT search clears it; a same-query re-run keeps it; leaving
    /// the results view ends the session; C-g outside isearch clears it.
    /// Match-highlight lifetime rule (issue match-highlight, corrected
    /// rule): isearch confirm (RET) — like cancel — CLEARS the context
    /// (emacs `isearch-exit` removes the lazy-highlight faces when the
    /// search ends), while the isearch STATE survives (a repeat search
    /// works). The context that PERSISTS is the one owned by a
    /// search-results jump: it survives point motion, is cleared by a
    /// different query, kept by a same-query re-run, and cleared by
    /// closing the results view or the global C-g.
    #[test]
    fn match_context_lifetime_rule() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/t.rs"),
            "foo world\nfoo there\nbar\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/u.rs"), "foo lib\n").unwrap();
        let mut s = store(dir.path());
        s.set_viewport_lines(10);
        s.open_path("src/t.rs");
        let mut rx = s.search_rx().unwrap();
        let type_foo = |s: &mut AppStore| {
            s.isearch_start(IsearchDirection::Forward);
            s.isearch_query_char('f');
            s.isearch_query_char('o');
            s.isearch_query_char('o');
        };
        // 1. Active isearch: the context is live.
        type_foo(&mut s);
        assert_eq!(s.match_context.ranges.len(), 2, "active isearch drives the highlight");
        // 2. Cancel (C-g) clears.
        s.isearch_cancel();
        assert!(s.match_context.ranges.is_empty(), "C-g cancel clears the highlight");
        // 3. Confirm (RET) now CLEARS too: emacs `isearch-exit` removes the
        //    lazy-highlight faces when the search ends. The old rule kept
        //    the context here — that rationale ("the context after jumping
        //    from results") belongs to the results-jump path (step 4),
        //    which sets its own context and is unaffected.
        type_foo(&mut s);
        s.isearch_confirm();
        assert!(
            s.match_context.ranges.is_empty(),
            "confirm clears the highlight (faces vanish when the search ends)"
        );
        // 3b. The isearch STATE survives confirm (only the highlight
        //     lifetime changed): re-running the same search finds the same
        //     matches.
        type_foo(&mut s);
        assert_eq!(s.isearch_match_count(), 2, "the search state survived confirm");
        s.isearch_cancel();
        // 4. The jump-from-results context is the one that PERSISTS: a
        //    RET from the results view sets it (this buffer's hits only),
        //    and it survives point motion.
        s.start_project_search("foo".into());
        drain_search_finished(&mut s, &mut rx);
        s.search.selected = 0; // t.rs line 1 ("src/t.rs" < "src/u.rs")
        s.search_jump();
        assert_eq!(s.top_view(), ViewId::Buffer);
        assert_eq!(
            s.match_context.ranges.len(),
            2,
            "the jump sets this buffer's match context (t.rs only)"
        );
        s.point_down();
        s.point_up();
        s.point_forward();
        s.point_line_start();
        assert_eq!(
            s.match_context.ranges.len(),
            2,
            "the jump context survives point motion"
        );
        // 5. A same-query re-run keeps it.
        s.start_project_search("foo".into());
        drain_search_finished(&mut s, &mut rx);
        assert_eq!(
            s.match_context.ranges.len(),
            2,
            "a same-query re-run keeps the highlight"
        );
        // 6. A different search clears it.
        s.start_project_search("bar".into());
        assert!(
            s.match_context.ranges.is_empty(),
            "a different search clears the old highlight"
        );
        // Consume the "bar" job's events so the results view is clean before the
        // next search. (The stale-generation early-return hazard this used to
        // guard against is gone: `drain_search_finished` now terminates only on
        // the CURRENT generation's `Finished`.)
        drain_search_finished(&mut s, &mut rx);
        // 7. Leaving the results view (q / ESC) ends the session.
        s.start_project_search("foo".into());
        drain_search_finished(&mut s, &mut rx);
        s.search_close();
        assert!(
            s.match_context.ranges.is_empty(),
            "closing the results view clears the highlight"
        );
        // 8. Global C-g (buffer view, no isearch active) clears it.
        s.start_project_search("foo".into());
        drain_search_finished(&mut s, &mut rx);
        s.search.selected = 0;
        s.search_jump();
        assert!(!s.match_context.ranges.is_empty(), "jump re-established the context");
        s.key_event(Key::ctrl_char('g'));
        assert!(
            s.match_context.ranges.is_empty(),
            "C-g outside isearch clears the highlight"
        );
    }

    /// jump-highlight P2-1: the `search_jump` landing hook — the third of
    /// three `record_landing_highlight` call-sites (the other two, in
    /// `record_jump` and `navigate_to_entry`, have coverage in
    /// `tests/navigation/jump.rs`). `search_jump` records the jump stack
    /// directly (not through `record_jump`), so it calls the shared
    /// helper itself: this test pins that the search-RET landing sets the
    /// transient landing highlight on the landed hit's symbol.
    #[test]
    fn search_jump_sets_the_landing_highlight() {
        let (_dir, mut s) = search_project();
        let mut rx = s.search_rx().unwrap();
        s.open_path("src/main.rs");
        s.set_viewport_lines(10);
        s.start_project_search("target".into());
        drain_search_finished(&mut s, &mut rx);
        // The deterministic sort puts lib.rs first: "pub fn target() {}",
        // "target" at byte column 7.
        assert_eq!(s.search.hits[0].file, "src/lib.rs");
        assert_eq!(s.search.hits[0].line_no, 1);
        assert_eq!(s.search.hits[0].col, Some(7));
        s.search.selected = 0;
        s.search_jump();
        let h = s
            .jump_highlight()
            .expect("search_jump must set the landing highlight");
        let key = s.buffers.current().unwrap().to_string();
        assert_eq!(h.buffer_key, key, "the highlight belongs to the landed buffer");
        assert_eq!(h.line, 0, "the highlight is on the hit's line (0-based)");
        // Line 0 is `pub fn target() {}`: the word `target` spans bytes
        // 7..13 (all ASCII).
        assert_eq!(h.start, 7, "the symbol's byte start");
        assert_eq!(h.end, 13, "the symbol's byte end (exclusive)");
    }

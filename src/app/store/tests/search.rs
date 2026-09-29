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

    // ── plan 018 issue 02: the isearch list (helm-occur shape) ─────────

    /// The list rows are 1:1 with the match set, in search order: each row
    /// carries the match's line number (0-based), the line's text (the row's
    /// display projection), and the match's CHAR column (not byte, not
    /// display cell — the point's `col` unit; on "café omega" the 'o' of
    /// "omega" is CHAR 5, byte 6). A forward search lists ascending, a
    /// backward one descending (search order IS list order — FilterOnly,
    /// no re-ranking).
    #[test]
    fn isearch_list_rows_follow_the_matches_in_search_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/t.rs"),
            "café omega\nalpha\ncafé omyga\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");

        // Forward: the rows run ascending (line 0 before line 2).
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('o');
        s.isearch_query_char('m');
        assert_eq!(s.isearch.matches.len(), 2, "one match per line");
        assert_eq!(
            s.isearch.rows,
            vec![
                crate::app::store::IsearchMatchRow {
                    line_no: 0,
                    line_text: "café omega".to_string(),
                    match_col: 5, // char index, not byte 6 (the é is 2 bytes)
                },
                crate::app::store::IsearchMatchRow {
                    line_no: 2,
                    line_text: "café omyga".to_string(),
                    match_col: 5,
                },
            ],
            "forward search order IS the list order"
        );
        s.isearch_cancel();

        // Backward: the same rows, reversed (the search runs backward from
        // the pre-search point, so the farthest match leads the list).
        s.isearch_start(IsearchDirection::Backward);
        s.isearch_query_char('o');
        s.isearch_query_char('m');
        assert_eq!(
            s.isearch.rows,
            vec![
                crate::app::store::IsearchMatchRow {
                    line_no: 2,
                    line_text: "café omyga".to_string(),
                    match_col: 5,
                },
                crate::app::store::IsearchMatchRow {
                    line_no: 0,
                    line_text: "café omega".to_string(),
                    match_col: 5,
                },
            ],
            "backward search order IS the list order (reversed matches)"
        );
        // The selection indexes both lists: row[current] is the match
        // the point sits on.
        let sel = s.isearch.current;
        assert_eq!(s.isearch.rows[sel].line_no, s.point_line());
    }

    /// On every recompute the selection RE-DERIVES to the first match in
    /// the search direction from the point (today's `isearch_recompute`
    /// rule, `search.rs` — the session's "re-derive" policy; for a fresh
    /// query the point is the pre-search point, so this IS the
    /// first-in-direction-from-the-pre-search-point rule). It does NOT keep
    /// the old index across recomputes — the picker's clamp is the wrong
    /// rule here. The discriminating step places the selection AWAY from
    /// the point (an index the point's re-derivation does not pick), then
    /// extends the query: a "keep old index" mutation reddens this, the
    /// re-derive rule lands the point's own match line.
    #[test]
    fn isearch_recompute_rederives_forward_selection_from_pre_search_point() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        // Four "aa" lines with a no-hit line at 1: the pre-search point
        // sits BETWEEN the first and last match lines, so the fresh
        // query's first-in-direction match is not the first or last row.
        std::fs::write(
            dir.path().join("src/t.rs"),
            "aa zero\nno hit\naa two\naa three\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.set_point(1, 0, 0); // pre-search point: line 1 (the no-hit line)
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('a'); // 6 matches: 2 on each of lines 0, 2, 3
        assert_eq!(s.isearch.matches.len(), 6);
        // Fresh query: the first match in the search direction from the
        // pre-search point — line 2's first 'a' (index 2), NOT the list's
        // head (line 0) and not the tail.
        assert_eq!(s.isearch.current, 2, "fresh query: first-in-direction from the pre-search point");
        assert_eq!(s.point_line(), 2, "the buffer view follows the fresh selection");
        // Walk the selection to the LAST match (line 3's second 'a'); the
        // point follows to line 3.
        for _ in 0..3 {
            s.isearch_next();
        }
        assert_eq!(s.isearch.current, 5, "C-s moved the selection to the end");
        assert_eq!(s.point_line(), 3);
        // The discriminating step: place the selection on the FIRST match
        // (line 0) while the point stays on line 3 — a state the recompute
        // must re-derive out of, because the point, not the old index,
        // drives the rule.
        s.isearch.current = 0;
        // Extend the query: "aa" narrows to 3 matches (one per "aa" line).
        s.isearch_query_char('a');
        assert_eq!(s.isearch.matches.len(), 3);
        assert_eq!(
            s.isearch.current,
            2,
            "the selection re-derives to the point's own match (line 3) — it does not keep the old index (line 0)"
        );
        assert_eq!(s.point_line(), 3, "the buffer view follows the re-derived selection");
    }

    /// The backward mirror: the selection re-derives to the
    /// backward-direction match at or before the point on every recompute
    /// (today's `isearch_recompute` rule — the re-derive policy, not the
    /// picker's clamp). A "keep old index" mutation reddens this: the
    /// discriminating step places the selection past the point, then the
    /// extension must re-derive to the point's own match, not clamp.
    #[test]
    fn isearch_recompute_rederives_backward_selection_from_pre_search_point() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        // "aa" lines at 0 and 1 (before the point) and at 3 (after it);
        // the pre-search point is line 2.
        std::fs::write(
            dir.path().join("src/t.rs"),
            "aa zero\naa one\nno hit\naa two\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.set_point(2, 0, 0); // pre-search point: line 2
        s.isearch_start(IsearchDirection::Backward);
        s.isearch_query_char('a'); // matches on lines 0, 1, 3
        // Fresh backward query: the FAR-EST match at or before the point —
        // line 0's earliest 'a' (the tail of the reversed match list).
        assert_eq!(s.isearch.matches.len(), 6);
        assert_eq!(
            s.isearch.rows[s.isearch.current].line_no,
            0,
            "fresh backward query selects the farthest-back match at or before the point"
        );
        // C-r moves the selection through the list (toward the point):
        // line 1's first match; the point follows.
        for _ in 0..3 {
            s.isearch_prev();
        }
        assert_eq!(
            s.isearch.rows[s.isearch.current].line_no,
            1,
            "C-r moved the selection through the list (to line 1)"
        );
        assert_eq!(s.point_line(), 1);
        // The discriminating step: place the selection PAST the point
        // (line 3's first match) while the point stays on line 1 — the
        // recompute must re-derive out of it from the point, not keep it.
        s.isearch.current = 0;
        // Extend the query: "aa" narrows to 3 matches (lines 0, 1, 3).
        s.isearch_query_char('a');
        assert_eq!(s.isearch.matches.len(), 3);
        assert_eq!(
            s.isearch.rows[s.isearch.current].line_no,
            0,
            "the selection re-derives to the backward match at or before the point (line 0) — it does not keep the old index (line 3)"
        );
        assert_eq!(s.point_line(), 0, "the buffer view follows the re-derived selection");
    }

    /// RET confirms the SELECTED match (the list's selection, moved by
    /// C-s) and the point lands ON the match's column — pinned on a
    /// multibyte line: in "café omyga" the 'o' of "omyga" is at BYTE 6
    /// (the é is 2 bytes) but CHAR 5; a byte-based landing would put the
    /// point at col 6 (the 'm'), the issue-isearch-column class.
    #[test]
    fn isearch_ret_lands_on_the_selected_multibyte_match_column() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/t.rs"),
            "café omega\ncafé omyga\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('o');
        s.isearch_query_char('m');
        assert_eq!(s.isearch.matches.len(), 2);
        // The fresh selection is the first match (line 0); C-s moves the
        // list's selection to the SECOND match (line 1).
        assert_eq!(s.isearch.current, 0);
        s.isearch_next();
        assert_eq!(s.isearch.current, 1, "C-s selects the second match in the list");
        s.isearch_confirm();
        assert!(!s.isearch.active);
        assert_eq!(s.point_line(), 1, "RET lands the point on the SELECTED match's line");
        assert_eq!(
            s.point_col(),
            5,
            "the match's CHAR column — not byte 6 (the multibyte é before it)"
        );
        assert_eq!(
            s.file_point().goal_col,
            5,
            "the landing column becomes the goal column"
        );
    }

    /// Backspacing the query to empty clears the LIST (the rows go with
    /// the result set) and keeps the empty-query echo byte-for-byte (the
    /// existing `I-search: ` prompt — this surface keeps its prompt in
    /// the minibuffer); a no-match query keeps the `[no matches]` echo
    /// byte-for-byte with an empty list.
    #[test]
    fn isearch_backspace_to_empty_clears_the_list_and_keeps_the_echo() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "foo a\nfoo b\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('f');
        s.isearch_query_char('o');
        assert_eq!(s.isearch.rows.len(), 2, "two matches: the list has two rows");
        assert_eq!(s.message, "I-search: fo [1/2]");
        s.isearch_backspace();
        assert_eq!(s.isearch.rows.len(), 2, "one match per line survives 'f'");
        assert_eq!(s.message, "I-search: f [1/2]");
        s.isearch_backspace();
        assert!(s.isearch.matches.is_empty(), "the empty query has no matches");
        assert!(s.isearch.rows.is_empty(), "the list disappears with the result set");
        assert_eq!(s.isearch.current, 0);
        assert_eq!(s.message, "I-search: ", "the empty-query echo, byte-for-byte");
        // A no-match query: the echo keeps its `[no matches]` shape and the
        // list stays empty.
        s.isearch_query_char('q');
        assert!(s.isearch.matches.is_empty());
        assert!(s.isearch.rows.is_empty(), "a no-match query has no list");
        assert_eq!(s.message, "I-search: q [no matches]");
        s.isearch_backspace();
        assert_eq!(s.message, "I-search: ", "back to the empty echo, byte-for-byte");
    }

    /// 018-02 measured this and reported it rather than fixing it (out of that
    /// issue's scope): `isearch_recompute`'s EMPTY-query early return skipped
    /// `isearch_sync_match_context()`, so backspacing a query to empty left the
    /// PREVIOUS query's highlight painted on the buffer — a highlight for text
    /// that is no longer being searched. Measured before the fix: after
    /// "fo" -> "f" -> "", `match_context` still held query="f" with ranges
    /// [(0, 1), (6, 7)].
    ///
    /// WHY IT SURVIVED: the pin below asserts the matches, the rows, `current`
    /// and the echo byte-for-byte — four of the five things this operation
    /// changes — and never looks at `match_context` at all.
    #[test]
    fn isearch_backspace_to_empty_clears_the_match_highlight() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "foo a\nfoo b\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('f');
        s.isearch_query_char('o');
        assert!(
            !s.match_context.ranges.is_empty(),
            "precondition: a live query paints the match highlight"
        );
        assert_eq!(s.match_context.query, "fo");
        s.isearch_backspace();
        s.isearch_backspace();
        assert!(s.isearch.matches.is_empty(), "the empty query has no matches");
        assert!(
            s.match_context.ranges.is_empty() && s.match_context.query.is_empty(),
            "the highlight must NOT survive the empty query (a stale paint for text that is no longer searched); got query={:?} ranges={:?}",
            s.match_context.query,
            s.match_context.ranges
        );
    }


    /// C-g makes the list disappear with the session (the rows and the
    /// match set both clear; the pre-search line AND column restore and
    /// the highlight-lifetime rule are pinned by the existing isearch
    /// cancel pins, unmodified).
    #[test]
    fn isearch_cancel_disappears_the_list_with_the_session() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "foo a\nfoo b\nfoo c\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('f');
        s.isearch_query_char('o');
        assert_eq!(s.isearch.rows.len(), 3, "the list is live");
        s.isearch_cancel();
        assert!(!s.isearch.active);
        assert!(s.isearch.matches.is_empty());
        assert!(s.isearch.rows.is_empty(), "the list disappears with the session");
        assert!(s.isearch.query.is_empty());
        assert_eq!(s.message, "cancel");
        // Confirm (RET) likewise leaves no list behind: the overlay
        // renders only while active.
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('f');
        s.isearch_query_char('o');
        assert_eq!(s.isearch.rows.len(), 3);
        s.isearch_confirm();
        assert!(!s.isearch.active);
        let (_, _, session_live) = s.isearch_list();
        assert!(
            !session_live,
            "the accessor reports the session closed — the overlay does not render"
        );
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

    /// issue-clipboard-and-selection part 3: a pasted non-ASCII char
    /// reaches the i-search prompt through the input gate (monotonic
    /// widening) and narrows the match count.
    #[test]
    fn isearch_accepts_a_pasted_nonascii_query_char() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/t.rs"),
            "café line\nplain line\ncafé again\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.key_event(key("C-s"));
        assert!(s.isearch_active());
        s.key_event(crate::app::keymap::Key::new(
            crate::app::keymap::KeyCode::Char('\u{e9}'),
        ));
        assert_eq!(
            s.isearch_match_count(),
            2,
            "the pasted é must reach the query and match both lines"
        );
    }

    // ── U-E13: isearch's second query dimension ─────────────────────────────
    //
    // The literal search is the FIRST dimension (mechanism=own, the plan's
    // corrected criterion — untouched by this section). The SECOND is a
    // nucleo filter inside the match list: C-o toggles the filter input,
    // the shared core filters the rows' line-text projection FilterOnly
    // (source order — match order IS the search), and C-g is layered
    // (first C-g clears the filter, second cancels isearch). Every pin
    // here REDS on the pre-U-E13 tree: C-o was swallowed there (the
    // guard had no filter branch), so the "filter" chars extended the
    // literal query and the echoed/row state diverged at the first
    // C-o-dependent assert.

    fn isearch_store_with(text: &str) -> (tempfile::TempDir, AppStore) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), text).unwrap();
        // A throwaway sibling base (the store helper's rule: never the
        // project dir itself — the walk must not see the persistence
        // files). Both tempdirs stay alive for the caller's duration.
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        (dir, s)
    }

    /// C-o arms the filter input; the filter narrows the match rows
    /// FilterOnly (the shared core's survivor set, SOURCE order) and the
    /// selection CLAMPS (the seam's rule) rather than resetting: the
    /// cursor's match is narrowed out, its old POSITION clamps into the
    /// surviving set. The query "l a" is a SUBSEQUENCE of "foo alpha"
    /// (l@5 … a@7) and "foo delta" (l@6 … a@8) and a contiguous substring
    /// of NONE of the rows: a contains-filter recompute keeps zero rows,
    /// the core keeps rows 0 and 2.
    #[test]
    fn isearch_filter_narrows_the_rows_and_clamps_the_selection() {
        let (_dir, mut s) = isearch_store_with(
            "foo alpha\nfoo gamma\nfoo delta\nfoo gamma\nfoo theta\n",
        );
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('f');
        s.isearch_query_char('o');
        s.isearch_query_char('o');
        assert_eq!(s.message, "I-search: foo [1/5]");
        // Walk the canonical selection AWAY from the head (line 3).
        for _ in 0..3 {
            s.isearch_next();
        }
        assert_eq!(s.message, "I-search: foo [4/5]");
        // C-o arms the filter input (the guard's new branch — a
        // pre-U-E13 tree swallows the key here).
        s.key_event(key("C-o"));
        assert_eq!(
            s.message,
            "I-search: foo (filter: ) [4/5]",
            "armed empty filter: the clause shows, the full set is still the list (the count is the list's)"
        );
        // The filter is typed in the guard's filter branch.
        s.key_event(key("l"));
        s.key_event(Key::space());
        s.key_event(key("a"));
        assert_eq!(
            s.message,
            "I-search: foo (filter: l a) [2/2]",
            "the filter clause + the FILTERED count, byte-for-byte"
        );
        let (rows, selected, active) = s.isearch_list();
        assert!(active);
        let order: Vec<usize> = rows.iter().map(|r| r.line_no).collect();
        assert_eq!(
            order,
            vec![0, 2],
            "FilterOnly: the core's survivors stay in SOURCE (search) order"
        );
        assert_eq!(
            selected,
            1,
            "the cursor's match (line 3) is NARROWED OUT: the old position 4 clamps to 1 — never a reset"
        );
        assert_eq!(
            s.isearch.current, 2,
            "the canonical selection follows the clamped position (line 2's match)"
        );
        assert_eq!(s.point_line(), 2, "the buffer view follows the clamped selection");
    }

    /// The filter SURVIVES further literal-search characters (and
    /// backspaces): the literal extension re-derives the match set and
    /// the set the filter projects re-derives with it. C-o off keeps the
    /// filter too (the toggle is the INPUT TARGET, not the filter's
    /// lifetime). C-g is layered: the first clears the filter (the
    /// selection follows its match — clamped, not lost), the second
    /// cancels isearch.
    #[test]
    fn isearch_filter_survives_a_literal_extension() {
        let (_dir, mut s) = isearch_store_with("foo alpha\nfoo gamma\nfoxtrot delta\n");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('f');
        s.isearch_query_char('o');
        assert_eq!(s.message, "I-search: fo [1/3]");
        s.isearch_next();
        s.isearch_next();
        assert_eq!(s.message, "I-search: fo [3/3]");
        s.key_event(key("C-o"));
        // "d e": a subsequence of "foxtrot delta" (d@8 … e@9), a
        // substring of none — only line 2 survives.
        s.key_event(key("d"));
        s.key_event(Key::space());
        s.key_event(key("e"));
        assert_eq!(
            s.message,
            "I-search: fo (filter: d e) [1/1]",
            "the filter narrows to line 2's row; the cursor's match survives"
        );
        // C-o off: the filter SURVIVES (only the input target moves).
        s.key_event(key("C-o"));
        assert_eq!(
            s.message,
            "I-search: fo (filter: d e) [1/1]",
            "C-o off keeps the filter (the list stays narrowed)"
        );
        // A further literal character: the match set re-derives, the
        // filter survives, and projects onto the new set.
        s.key_event(key("x"));
        assert_eq!(s.isearch_match_count(), 1, "'fox' matches only line 2");
        assert_eq!(
            s.message,
            "I-search: fox (filter: d e) [1/1]",
            "the filter clause survives the literal extension"
        );
        let (rows, _, _) = s.isearch_list();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].line_no, 2);
        // Backspace: the re-derived set is re-filtered again; the
        // re-derived selection (line 0) is narrowed out and the
        // position clamps to the survivor (line 2).
        s.key_event(key("C-h"));
        assert_eq!(
            s.message,
            "I-search: fo (filter: d e) [1/1]",
            "the backspace re-derives the set, the filter survives"
        );
        assert_eq!(s.point_line(), 2, "the clamped selection (line 2) is where the point sits");
        // Layered C-g: FIRST clears the filter (the selection follows
        // its match — line 2, clamped into the full set), SECOND cancels.
        s.key_event(key("C-g"));
        assert_eq!(
            s.message,
            "I-search: fo [3/3]",
            "the first C-g clears the filter; the selection followed its match (line 2), not a reset to line 0"
        );
        assert_eq!(s.point_line(), 2);
        s.key_event(key("C-g"));
        assert_eq!(s.message, "cancel");
        assert!(!s.isearch_active(), "the second C-g cancels isearch");
        assert_eq!(s.point_line(), 0, "the pre-search line restores");
        assert!(s.isearch.rows.is_empty(), "the list disappears with the session");
    }

    /// A filter matching NOTHING shows an honest empty state: no rows,
    /// no stale highlight, the `[no matches]` echo with the clause, and
    /// RET is an honest `[not found]` (the point does not teleport onto
    /// a match the list does not show).
    #[test]
    fn isearch_filter_matching_nothing_is_an_honest_empty_state() {
        let (_dir, mut s) = isearch_store_with("bar b\nfoo a\n");
        s.set_point(1, 1, 1); // pre-search point: line 1, char 1
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('f');
        assert_eq!(s.message, "I-search: f [1/1]");
        s.key_event(key("C-o"));
        s.key_event(key("z"));
        s.key_event(key("z"));
        assert_eq!(
            s.message,
            "I-search: f (filter: zz) [no matches]",
            "the clause stays, the count is the honest [no matches]"
        );
        let (rows, _, active) = s.isearch_list();
        assert!(active);
        assert!(rows.is_empty(), "no stale rows");
        assert!(
            s.match_context.ranges.is_empty(),
            "no stale highlight for a match the list does not show"
        );
        assert_eq!(
            (s.point_line(), s.point_col()),
            (1, 0),
            "the point stays where it was (no jump onto the unlisted match)"
        );
        s.isearch_confirm();
        assert!(!s.isearch_active());
        assert_eq!(
            s.message,
            "I-search: f [not found]",
            "RET on an empty filtered set is an honest not-found"
        );
    }

    /// C-s / C-r wrap WITHIN the filtered set (never over the full
    /// match set) while the filter is active.
    #[test]
    fn isearch_c_s_c_r_wrap_within_the_filtered_set() {
        let (_dir, mut s) = isearch_store_with("foo beta\nfoo beta\nfoo gamma\n");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('f');
        assert_eq!(s.isearch_match_count(), 3);
        s.key_event(key("C-o"));
        s.key_event(key("b"));
        s.key_event(key("e"));
        assert_eq!(
            s.message,
            "I-search: f (filter: be) [1/2]",
            "lines 0 and 1 survive 'be'; line 2 drops out"
        );
        s.key_event(key("C-s"));
        assert_eq!(s.message, "I-search: f (filter: be) [2/2]");
        assert_eq!(s.isearch_match_index(), 2, "C-s moved to line 1's match (source index 1)");
        s.key_event(key("C-s"));
        assert_eq!(
            s.message,
            "I-search: f (filter: be) [1/2]",
            "C-s wraps WITHIN the filtered set (not to line 2, which is filtered out)"
        );
        assert_eq!(s.isearch_match_index(), 1, "back to line 0's match (source index 0)");
        s.key_event(key("C-r"));
        assert_eq!(s.message, "I-search: f (filter: be) [2/2]");
        assert_eq!(s.isearch_match_index(), 2, "C-r wraps back to line 1's match");
    }

    /// With a filter active, a literal extension still re-derives the
    /// FIRST-dimension selection from the pre-search point (the pinned
    /// re-derive rule — `isearch_recompute_rederives_*`'s regime) and
    /// then re-projects the surviving filter onto the new set.
    #[test]
    fn isearch_first_dimension_rederives_when_the_filter_is_active() {
        let (_dir, mut s) = isearch_store_with("aa zero\nno hit\naa two\naa three\n");
        s.set_point(1, 0, 0); // pre-search point: line 1 (the no-hit line)
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('a');
        assert_eq!(s.isearch_match_count(), 6);
        assert_eq!(s.message, "I-search: a [3/6]");
        s.key_event(key("C-o"));
        // 'h': only line 3's text ("aa three") holds it.
        s.key_event(key("h"));
        assert_eq!(
            s.message,
            "I-search: a (filter: h) [2/2]",
            "the filter keeps line 3's two matches; the re-derived selection (line 2) is narrowed out and clamps to them"
        );
        assert_eq!(s.point_line(), 3);
        // Toggle the filter input OFF, then extend the literal query:
        // "aa" — the first dimension re-derives to the point's own match
        // (line 3), and the filter re-projects (the set it filters is
        // the new one).
        s.key_event(key("C-o"));
        s.key_event(key("a"));
        assert_eq!(s.isearch_match_count(), 3, "'aa': one match per 'aa' line");
        assert_eq!(
            s.message,
            "I-search: aa (filter: h) [1/1]",
            "the re-derived selection (line 3) survives the re-projection"
        );
        assert_eq!(
            s.isearch.current, 2,
            "the re-derive rule still drives the canonical selection (line 3's match)"
        );
        assert_eq!(s.point_line(), 3);
    }

    /// Backspacing the literal query to empty with a filter active:
    /// the filter goes with the source set (there is nothing left to
    /// filter), the empty-query echo stays byte-for-byte, and the
    /// highlight clears (the `isearch_backspace_to_empty_*` regime
    /// extended to the filter state).
    #[test]
    fn isearch_backspace_to_empty_with_a_filter_clears_both_and_keeps_the_echo() {
        let (_dir, mut s) = isearch_store_with("foo a\nfoo b\n");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('f');
        s.isearch_query_char('o');
        assert_eq!(s.message, "I-search: fo [1/2]");
        s.key_event(key("C-o"));
        s.key_event(key("a"));
        assert_eq!(
            s.message,
            "I-search: fo (filter: a) [1/1]",
            "only 'foo a' holds 'a'"
        );
        s.isearch_backspace();
        assert_eq!(
            s.message,
            "I-search: f (filter: a) [1/1]",
            "'f' re-derives the set (2 matches); the filter survives and the count is the filtered one"
        );
        s.isearch_backspace();
        assert!(s.isearch.matches.is_empty(), "the empty query has no matches");
        assert!(s.isearch.rows.is_empty(), "the list disappears with the result set");
        assert!(
            s.match_context.ranges.is_empty(),
            "the highlight clears (the pre-existing regime)"
        );
        assert_eq!(s.message, "I-search: ", "the empty-query echo, byte-for-byte");
        // The filter went with the source set: the next char is a
        // LITERAL char, not a filter char.
        s.key_event(key("q"));
        assert_eq!(s.message, "I-search: q [no matches]");
        assert_eq!(s.isearch.query, "q", "the char extended the literal query");
    }

    /// RET confirms the SELECTED filtered match — the landing column is
    /// the match's CHAR column on a multibyte line (the
    /// `issue-isearch-column` regime through the second dimension):
    /// in "café omyga" the 'o' of "omyga" is at BYTE 6 but CHAR 5.
    #[test]
    fn isearch_ret_lands_on_the_selected_filtered_match_column() {
        let (_dir, mut s) = isearch_store_with("café omega\ncafé omyga\n");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('o');
        s.isearch_query_char('m');
        assert_eq!(s.isearch_match_count(), 2);
        s.key_event(key("C-o"));
        // 'yg': only "café omyga" holds it — the filter narrows to line
        // 1, whose match (the cursor's, line 0) is narrowed out: the
        // position clamps and the canonical selection follows to line 1.
        s.key_event(key("y"));
        s.key_event(key("g"));
        assert_eq!(s.message, "I-search: om (filter: yg) [1/1]");
        assert_eq!(s.isearch.current, 1, "the clamped selection is line 1's match");
        s.isearch_confirm();
        assert!(!s.isearch_active());
        assert_eq!(s.point_line(), 1, "RET lands the point on the SELECTED filtered match's line");
        assert_eq!(
            s.point_col(),
            5,
            "the match's CHAR column — not byte 6 (the multibyte é before it)"
        );
        assert_eq!(s.file_point().goal_col, 5, "the landing column becomes the goal column");
    }

    /// A literal extension that leaves ZERO matches drops the filter
    /// (no source set — the filter goes with it, the empty-literal-
    /// query rule extended): the echo is the plain `[no matches]` (no
    /// stale clause), the highlight clears, and nothing panics (a stale
    /// filtered session read against zero rows panicked the highlight
    /// sync — measured, and this is the pin that caught it).
    #[test]
    fn isearch_zero_matches_drops_the_filter() {
        let (_dir, mut s) = isearch_store_with("foo a\nfoo b\n");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('f');
        s.isearch_query_char('o');
        s.key_event(key("C-o"));
        s.key_event(key("a"));
        assert_eq!(s.message, "I-search: fo (filter: a) [1/1]");
        s.key_event(key("C-o")); // off — the filter survives
        s.key_event(key("z")); // literal "foz": zero matches, filter in play
        assert!(s.isearch.matches.is_empty(), "'foz' matches nothing");
        assert!(s.isearch.rows.is_empty());
        assert!(
            s.isearch.filter.query.is_empty(),
            "the filter went with the source set (no stale session)"
        );
        assert!(!s.isearch.filter_mode);
        assert!(s.match_context.ranges.is_empty(), "no stale highlight");
        assert_eq!(
            s.message,
            "I-search: foz [no matches]",
            "the plain [no matches] echo — no stale filter clause"
        );
    }

    // ── plan 018 issue 03: results-view narrowing ────────────────────

    /// Narrow to a file subset through the prompt (the user's typing
    /// path — printable chars route through the guard, not the keymap):
    /// the canonical `hits`/`rows` are NEVER narrowed away (the
    /// projection, not a mutation, plan 018 §2.3-3), the view window
    /// keeps only that file's header + hits (the other file's header is
    /// DROPPED — 0 surviving children), the title carries both numbers,
    /// `n`/`p` wrap WITHIN the subset, and RET lands on a hit in that
    /// file. On the pre-018-03 tree the typed chars echo "unbound key"
    /// and every one of these asserts is red (the window is still the
    /// full 6-row list and the title still says "4 matches").
    #[test]
    fn results_narrow_to_file_subset_ret_and_wrap() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);
        assert_eq!(store.search.hits.len(), 4); // 1 lib.rs + 3 main.rs

        // "lib" (no decision-key chars — n/p/g/q are the view's own
        // keys and fall through to the keymap, not the query).
        for c in "lib".chars() {
            store.key_event(Key::char(c));
        }
        // Canonical stream state untouched by the narrowing:
        assert_eq!(store.search.hits.len(), 4, "hits are never narrowed away");
        assert_eq!(store.search.rows.len(), 6, "canonical rows (2 headers + 4 hits) untouched");

        // The window: the lib.rs header (SURVIVING count 1, final_count
        // unchanged) + its one hit — no main.rs rows at all.
        let (rows, _top, total, sel_row) = store.search_view_info();
        assert_eq!(total, 2, "narrowed row list: 1 header + 1 hit");
        match &rows[0] {
            crate::app::store::ResultRow::Header { file, count, final_count } => {
                assert_eq!(file, "src/lib.rs");
                assert_eq!(*count, 1, "the surviving count for the file");
                assert!(*final_count, "final_count unchanged from the canonical header");
            }
            other => panic!("expected the lib.rs header, got {other:?}"),
        }
        assert!(matches!(
            &rows[1],
            crate::app::store::ResultRow::Hit { hit_index: 0, .. }
        ));
        assert_eq!(sel_row, Some(1), "the selection (lib.rs hit 0) is in the window");

        // The title carries both numbers, visible together.
        assert!(store.search_title().contains("1 of 4 matches"));

        // n/p wrap within the 1-hit subset (they do NOT step to the
        // narrowed-out main.rs hits).
        store.key_event(key("n"));
        assert_eq!(store.search.selected, 0, "n wraps within the 1-hit subset");
        store.key_event(key("p"));
        assert_eq!(store.search.selected, 0, "p wraps within the 1-hit subset");

        // RET lands on a hit in that file (the un-narrowed RET semantics,
        // on the canonical hit index).
        store.key_event(key("RET"));
        assert_eq!(store.top_view(), ViewId::Buffer);
        assert_eq!(store.view_name_display(), "src/lib.rs");
    }

    /// Plan 018 issue 03 — the pending-sequence discipline (class-bug pin,
    /// sibling of the magit/buffer-list narrow guards): a chord that arms a
    /// prefix must NOT be stranded by the results-view narrow-prompt guard.
    /// With the results view on top, `C-x` reaches the engine (no char
    /// value — the guard cannot swallow it) and arms the `C-x` prefix; the
    /// follow-up `2` — which completes the bound `C-x 2` — must reach the
    /// engine too. On the pre-fix guard the `2` was silently edited into
    /// `search.narrow.query` (the guard had NO pending-sequence check) and
    /// the sequence was stranded. The guard now consults `self.pending`:
    /// a non-empty pending means the key MUST reach the engine, so the
    /// filter query stays empty and the unbound echo lands.
    #[test]
    fn results_narrow_guard_reaches_the_engine_for_a_pending_chord_c_x_2() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);
        assert_eq!(store.search.hits.len(), 4);
        // No narrow query yet.
        assert_eq!(store.search.narrow.query, "");
        store.key_event(key("C-x"));
        assert_eq!(
            store.search.narrow.query,
            "",
            "a bare C-x must not feed the narrow query"
        );
        store.key_event(key("2"));
        assert_eq!(
            store.search.narrow.query,
            "",
            "the `2` must NOT be silently edited into the narrow query"
        );
        assert!(
            store.message.contains("unbound key: 2"),
            "the `2` reaches the engine and dead-ends to the unbound echo: {}",
            store.message
        );
        assert_eq!(store.top_view(), ViewId::Search, "no view change on the echo");
    }

    /// n/p wrap within a MULTI-hit subset: the wrap target is the
    /// subset's first hit, not the full list's. On the pre-018-03 tree
    /// `n` from the last main hit wraps to hit 0 (the lib.rs hit) —
    /// red; with the narrowing it lands on the subset's first hit.
    #[test]
    fn results_n_p_wrap_within_the_narrowed_subset() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);
        // Narrow to main.rs via "mai" (hits 1, 2, 3) — "main" cannot be
        // typed: the final `n` is the view's next-match key and falls
        // through to the keymap. The cursor's old position 0 (the
        // lib.rs hit, hit index 0) clamps to position 0 of the surviving
        // set -> hit 1.
        for c in "mai".chars() {
            store.key_event(Key::char(c));
        }
        assert_eq!(store.search.narrow.query, "mai");
        assert_eq!(store.search.selected, 1, "the clamped cursor sits on the subset's first hit");
        assert_eq!(store.search.narrow.selected, 0, "the prompt cursor indexes the NARROWED list");
        store.key_event(key("n"));
        assert_eq!(store.search.selected, 2);
        store.key_event(key("n"));
        assert_eq!(store.search.selected, 3);
        store.key_event(key("n"));
        assert_eq!(
            store.search.selected,
            1,
            "n wraps to the SUBSET's first hit — not the full list's hit 0"
        );
        store.key_event(key("p"));
        assert_eq!(store.search.selected, 3, "p wraps back within the subset");
    }

    /// Narrowing away the selected hit CLAMPS (issue 01's rule — the
    /// cursor's old POSITION survives into the shrunk set) and clearing
    /// the query restores the full list with that clamped selection:
    /// clamped, not lost, and never a reset to 0. On the pre-018-03
    /// tree the typed chars are unbound, the selection never moves, and
    /// the narrowed-view asserts are red.
    #[test]
    fn results_narrow_clamp_survives_the_clear_not_reset() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);
        // Park the cursor on main.rs line 1 (hit 1, position 1).
        store.key_event(key("n"));
        assert_eq!(store.search.selected, 1);
        // ";" survives on main.rs lines 2+3 only (hits 2, 3) — the
        // selected hit 1 ("fn target() {}", no semicolon) is narrowed
        // out. (The obvious "target();" cannot be typed: its `g` is the
        // view's re-run key.) The cursor's old position 1 clamps into
        // the 2-hit set -> position 1 -> hit 3. A "reset selection to 0"
        // implementation would land on hit 2.
        store.key_event(Key::char(';'));
        assert_eq!(
            store.search.selected,
            3,
            "the clamped position keeps the selection on a survivor — never a reset"
        );
        let (_rows, _top, total, sel_row) = store.search_view_info();
        assert_eq!(total, 3, "1 header + 2 surviving hits");
        assert_eq!(sel_row, Some(2), "the clamped hit's row in the narrowed window");
        // C-g (job not running) CLEARS the query: the full list
        // re-derives and the clamped selection survives (not lost).
        store.key_event(key("C-g"));
        assert_eq!(store.search.narrow.query, "", "C-g cleared the query (job not running)");
        assert_eq!(store.top_view(), ViewId::Search, "the view stays open");
        assert!(store.message.contains("filter cleared"), "{}", store.message);
        let (rows, _top, total, sel_row) = store.search_view_info();
        assert_eq!(total, 6, "the full list: 2 headers + 4 hits");
        assert_eq!(store.search.selected, 3, "the clamped selection is not lost");
        assert_eq!(sel_row, Some(5));
        let _ = &rows;
        // C-g again with an empty query: today's no-op message (the C-g
        // split's third case).
        store.key_event(key("C-g"));
        assert_eq!(store.message, "nothing to cancel");
    }

    /// A query typed while the job is STILL RUNNING applies to the hits
    /// arrived so far (the projection is at view time — nothing
    /// buffers, nothing re-spawns the job, the generation guard is
    /// untouched): arriving non-matching hits stay out of the window,
    /// matching ones appear, and NOTHING is buffered — the full result
    /// set arrives after `Finished` while the narrow holds. On the
    /// pre-018-03 tree the typed chars are unbound and the window still
    /// lists every arrived hit — red.
    #[test]
    fn results_narrow_mid_stream_applies_to_hits_so_far() {
        // A slow-enough walk: 300 files, half named alpha_* (the narrow
        // target), half beta_* (the drops).
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        for i in 0..150 {
            let name = if i < 75 { "alpha" } else { "beta" };
            std::fs::write(
                dir.path().join(format!("src/{name}_{i:03}.txt")),
                "needle\n",
            )
            .unwrap();
        }
        let base = tempfile::tempdir().unwrap();
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("needle".into());
        // Apply the bus's Hit events (and FileDones) — but apply the
        // Finished event ONLY at the end, so the job is still `running`
        // while the narrow is typed. Bounded.
        let start = std::time::Instant::now();
        let mut have_alpha = false;
        let mut have_beta = false;
        'pull: loop {
            while let Ok(ev) = rx.try_recv() {
                if matches!(ev, crate::search::rg::SearchEvent::Finished { .. }) {
                    continue; // park Finished for the final drain
                }
                if let crate::search::rg::SearchEvent::Hit { file, .. } = &ev {
                    if file.starts_with("src/alpha_") {
                        have_alpha = true;
                    }
                    if file.starts_with("src/beta_") {
                        have_beta = true;
                    }
                }
                store.apply_search_event(&ev);
                if have_alpha && have_beta {
                    break 'pull;
                }
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "both file classes did not arrive"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(store.search_running(), "the job is still in flight (Finished unapplied)");
        // Type the narrow query MID-STREAM.
        for c in "alpha".chars() {
            store.key_event(Key::char(c));
        }
        // The window shows only arriving alpha hits; beta hits (already
        // in `hits`) stay OUT of the window.
        let (_rows, _top, total, _) = store.search_view_info();
        assert!(total > 0, "arriving alpha hits are in the narrowed list");
        for &(h, _) in &store.search.narrow.filtered {
            assert!(store.search.hits[h].file.starts_with("src/alpha_"));
        }
        assert!(
            store.search.hits.iter().any(|h| h.file.starts_with("src/beta_")),
            "beta hits arrived and are in the canonical set (narrowing is a projection)"
        );
        // The title carries both numbers mid-stream (narrowed / total).
        let narrowed = store.search.narrow.filtered.len();
        let hits = store.search.hits.len();
        assert!(
            store.search_title().contains(&format!("{narrowed} of {hits} matches")),
            "title: {}", store.search_title()
        );
        // Finish: the FULL result set arrived (nothing was buffered by
        // the narrowing) and the narrow still holds. The projection is
        // re-derived at VIEW time (the stored filtered is the last
        // keystroke's cache — view_info re-scores the live hits).
        drain_search_finished(&mut store, &mut rx);
        assert_eq!(store.search.hits.len(), 150, "all hits arrived despite the mid-stream narrow");
        let (_rows, _top, total, _) = store.search_view_info();
        assert_eq!(store.search.narrow.filtered.len(), 75, "the narrow holds after Finished");
        for &(h, _) in &store.search.narrow.filtered {
            assert!(store.search.hits[h].file.starts_with("src/alpha_"));
        }
        // The headers re-derive at view time: 75 surviving files
        // (1 header + 1 hit each) = 150 narrowed rows.
        assert_eq!(total, 150, "75 surviving headers + 75 surviving hits");
    }

    /// `g` re-run clears the narrow query (a new job is a new result set
    /// — stated, not accidental: `begin_search` replaces the
    /// `SearchState`, and the session lives inside it). On the
    /// pre-018-03 tree there is no query to clear; the post-run window
    /// assert (6 rows) is the leg that only passes once the prompt
    /// exists and is cleared by the re-run.
    #[test]
    fn results_rerun_clears_the_query() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);
        for c in "lib".chars() {
            store.key_event(Key::char(c));
        }
        assert_eq!(store.search.narrow.query, "lib");
        let old_gen = store.search.generation;
        store.key_event(key("g"));
        assert_eq!(store.search.generation, old_gen + 1);
        assert_eq!(
            store.search.narrow.query,
            "",
            "g re-run cleared the narrow query (a new job is a new result set)"
        );
        assert!(store.search_running());
        drain_search_finished(&mut store, &mut rx);
        let (_rows, _top, total, _) = store.search_view_info();
        assert_eq!(total, 6, "the full list re-derives (2 headers + 4 hits)");
    }

    /// RET under an active query records the jump-stack sentinel on the
    /// canonical hit index, and `M-,` returns to the results WITH the
    /// selection restored — under the still-active query (the narrow
    /// survives the jump/return round trip; the existing
    /// `search_jump` sentinel behaviour is unbroken).
    #[test]
    fn results_narrowed_ret_sentinel_restores_selection_on_mcomma() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);
        // Walk the cursor to main.rs line 1 (hit 1, position 1), then
        // narrow to main.rs via "mai" (the subset's hits 1, 2, 3 —
        // "main" cannot be typed: the `n` is the view's next key): the
        // cursor's hit 1 survives the narrowing, so the cursor stays on
        // it (the position follows the hit).
        store.key_event(key("n"));
        for c in "mai".chars() {
            store.key_event(Key::char(c));
        }
        assert_eq!(store.search.selected, 1, "the cursor's hit survives the narrowing");
        store.key_event(key("RET"));
        assert_eq!(store.top_view(), ViewId::Buffer);
        assert_eq!(store.view_name_display(), "src/main.rs");
        assert_eq!(store.point_line(), 0, "the jumped hit's line (0-based)");
        // M-,: the sentinel — back to the results, selection restored
        // (hit 1), the narrow still active.
        store.key_event(key("M-,"));
        assert_eq!(store.top_view(), ViewId::Search, "M-, must return to the results");
        assert_eq!(store.search.selected, 1, "the selection restored on the canonical hit index");
        let (_rows, _top, total, sel_row) = store.search_view_info();
        assert_eq!(total, 4, "1 header + 3 main hits — the narrow survived the round trip");
        assert_eq!(sel_row, Some(1), "the restored selection is the window's selected row");
    }

    /// The prompt row is ONE NoWrap row and the DECISION KEYS LEAD
    /// (PLAN §5.2, the `7f0090a` clip pin): at 80 cols with a 200-char
    /// query the row is filled to the edge by the query tail, every
    /// decision group is still present and left of the query — a clip
    /// eats the query tail (recognisable), never the keys (unguessable).
    /// A mutation that reorders the row (query first) reddens this.
    #[test]
    fn results_prompt_row_keys_lead_at_80_with_200_char_query() {
        use crate::ui::root::render_at_width;

        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);
        for _ in 0..200 {
            store.key_event(Key::char('x'));
        }
        let frame = render_at_width(store, 80);
        let lines: Vec<&str> = frame.lines().collect();
        let prompt_idx = lines
            .iter()
            .position(|l| l.contains("filter:"))
            .unwrap_or_else(|| panic!("prompt row missing\n{frame}"));
        let prompt = lines[prompt_idx];
        // One row, NoWrap: the row is exactly 80 cells, filled to the
        // edge with the query (200 chars -> clipped, tail gone). (The
        // `·` separators are 2 bytes each — count CELLS, not bytes.)
        assert_eq!(prompt.chars().count(), 80, "the prompt row is one 80-col row: {prompt:?}");
        assert!(prompt.trim_end().ends_with('x'), "the row is filled by the query");
        let query_at = prompt.find('x').unwrap();
        for group in ["RET jump", "n/p", "g re-run", "C-g clear", "q close"] {
            assert!(
                prompt.contains(group),
                "decision group {group:?} must survive the 80-col clip: {prompt:?}"
            );
            assert!(
                prompt.find(group).unwrap() < query_at,
                "decision group {group:?} must LEAD the query: {prompt:?}"
            );
        }
        // The query tail is clipped (the row holds ~22 of the 200 chars).
        assert!(
            !prompt.contains(&"x".repeat(25)),
            "the 200-char query cannot fit: its tail is the clipped part: {prompt:?}"
        );
    }

    /// The prompt row at rest shows the keys + the `type to narrow`
    /// placeholder (ONE NoWrap row under the title); Backspacing the
    /// query back to empty restores the FULL list with the selection
    /// preserved (the clear path's Backspace half — C-g's is pinned in
    /// `results_narrow_clamp_survives_the_clear_not_reset`).
    #[test]
    fn results_prompt_placeholder_and_backspace_to_empty() {
        use crate::ui::root::render_at_width;

        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);
        // Walk the cursor to main.rs line 2 (hit 2), then narrow to a
        // query that keeps hit 2 among survivors ("mai" — hits 1, 2, 3:
        // "main" cannot be typed, the `n` is the view's next key): the
        // cursor's hit 2 survives, so it stays under the cursor.
        store.key_event(key("n"));
        store.key_event(key("n"));
        assert_eq!(store.search.selected, 2);
        for c in "mai".chars() {
            store.key_event(Key::char(c));
        }
        assert_eq!(store.search.narrow.query, "mai");
        assert_eq!(store.search.narrow.filtered.len(), 3, "hits 1, 2, 3 survive 'mai'");
        assert_eq!(store.search.selected, 2, "the cursor's hit survives the narrowing");
        // Backspace the query away (3 chars): the full list re-derives
        // and the selection (hit 2) is preserved — not lost, not reset.
        for _ in 0..3 {
            store.key_event(Key::new(KeyCode::Backspace));
        }
        assert_eq!(store.search.narrow.query, "");
        let (rows, _top, total, sel_row) = store.search_view_info();
        assert_eq!(total, 6, "the full list: 2 headers + 4 hits");
        assert_eq!(store.search.selected, 2, "the selection survived the Backspace-away");
        assert_eq!(sel_row, Some(4));
        let _ = &rows;
        // The at-rest prompt row: keys + placeholder, one row under the
        // title (the render consumes the store, so it runs last).
        let frame = render_at_width(store, 80);
        let lines: Vec<&str> = frame.lines().collect();
        let title_idx = lines
            .iter()
            .position(|l| l.contains("Search: 'target'"))
            .unwrap_or_else(|| panic!("title row missing\n{frame}"));
        let prompt = lines.get(title_idx + 1).copied().unwrap_or("");
        assert!(prompt.contains("filter:"), "the prompt row sits under the title: {prompt:?}");
        assert!(prompt.contains("type to narrow"), "the empty-query placeholder: {prompt:?}");
        for group in ["RET jump", "n/p", "g re-run", "C-g clear", "q close"] {
            assert!(prompt.contains(group), "decision group {group:?}: {prompt:?}");
        }
        let _ = &rows;
    }

    // ── R3 agreement test, moved from `redline_model::files::tests`
    // (plan 014 stage 3): it exercises the search pipeline (bin-only)
    // against the file-list walk (now a workspace crate), and an
    // extracted crate cannot reach the bin's `search` module — the bin
    // side next to the search tests is the honest home ────────────────
    /// R3: the file-list walk and the search pipeline must agree on
    /// gitignore semantics. This fixture exercises a nested `.gitignore`
    /// (root + subdir), a negation (`!important.log`), and a directory
    /// rule (`build/`). The only expected difference is the documented
    /// `graft/` asymmetry (Item 2): the file-list walk prunes `graft/`,
    /// the search pipeline does not. Both walkers call the same
    /// `load_gitignore` + `gitignore_matches` helpers (R3 extraction).
    #[test]
    fn finder_and_search_agree_on_gitignore() {
        use crate::search::rg::{SearchBus, SearchConfig, SearchEvent, spawn};
        use std::sync::atomic::AtomicBool;

        fn wfile(path: impl AsRef<std::path::Path>, content: &str) {
            let path = path.as_ref();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }

        // Single-path test shim over the model's shared predicate
        // (identical to the model's own tests' `ignored` twin).
        fn ignored(root: &std::path::Path, path: &std::path::Path, is_dir: bool) -> bool {
            let memo: redline_model::files::GitignoreMemo =
                std::sync::Mutex::new(std::collections::HashMap::new());
            redline_model::files::is_gitignored(root, path, is_dir, &memo)
        }

        let dir = tempfile::tempdir().unwrap();
        wfile(dir.path().join("Cargo.toml"), "[package]\n");
        // Root .gitignore: ignore *.log (except important.log), ignore build/
        std::fs::write(
            dir.path().join(".gitignore"),
            "*.log\n!important.log\nbuild/\n",
        )
        .unwrap();
        wfile(dir.path().join("kept.txt"), "hello world\n");
        wfile(dir.path().join("crash.log"), "hello world\n");
        wfile(dir.path().join("important.log"), "hello world\n");
        wfile(dir.path().join("build/out.bin"), "hello world\n");
        // Subdirectory .gitignore: ignore gen/
        wfile(dir.path().join("src/.gitignore"), "gen/\n");
        wfile(dir.path().join("src/keep.rs"), "hello world\n");
        wfile(dir.path().join("src/gen/out.rs"), "hello world\n");
        wfile(dir.path().join("src/visible.rs"), "hello world\n");
        // graft/ directory (the documented asymmetry)
        wfile(dir.path().join("graft/card.md"), "hello world\n");

        // FileList walk (the finder side)
        let list = redline_model::files::FileList::build(dir.path()).unwrap();
        let finder_files: std::collections::BTreeSet<&str> =
            list.files.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            finder_files,
            std::collections::BTreeSet::from([
                "Cargo.toml", "important.log", "kept.txt", "src/keep.rs", "src/visible.rs"
            ]),
            "FileList should contain exactly the non-ignored, non-graft files"
        );

        // Search pipeline (the grep side)
        let cfg = SearchConfig {
            root: dir.path().to_path_buf(),
            pattern: "hello world".to_string(),
            word: false,
            fixed: true,
            case_smart: false,
            case_insensitive: false,
            glob: None,
            file_type: None,
            filter: None,
            cancel: std::sync::Arc::new(AtomicBool::new(false)),
        };
        let (bus, mut rx) = SearchBus::new();
        spawn(cfg, &bus, 0);
        drop(bus);

        // Drain until Finished
        let mut search_files: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        let start = std::time::Instant::now();
        loop {
            match rx.try_recv() {
                Ok(SearchEvent::Hit { file, .. }) => {
                    search_files.insert(file);
                }
                Ok(SearchEvent::Finished { .. }) => break,
                Ok(_) => {}
                Err(_) => {
                    if start.elapsed() > std::time::Duration::from_secs(5) {
                        panic!(
                            "search did not finish within 5s; files so far: {search_files:?}"
                        );
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }

        // The only expected difference is graft/card.md (Item 2: the
        // file-list walk prunes graft/, the search pipeline does not).
        // All files except Cargo.toml (whose content doesn't match the
        // search pattern) contain "hello world".
        let expected_search: std::collections::BTreeSet<String> =
            ["graft/card.md", "important.log", "kept.txt", "src/keep.rs", "src/visible.rs"]
                .into_iter()
                .map(|s| s.to_string())
                .collect();
        assert_eq!(
            search_files, expected_search,
            "search and finder must agree on gitignore (modulo the documented graft/ asymmetry)"
        );

        // Index filter (the THIRD walker, this issue): the incremental
        // reindex must keep exactly the walk's file set over the same
        // fixture — the nested `.gitignore` (`src/gen/`), the root rules
        // (`*.log`, `build/`), the negation (`!important.log`), and the
        // graft/ prune all agree with the walk.
        let every_path = [
            "Cargo.toml", "kept.txt", "important.log", "crash.log", "build/out.bin",
            "src/keep.rs", "src/gen/out.rs", "src/visible.rs", "graft/card.md",
        ]
        .map(|rel| dir.path().join(rel));
        let index_kept: std::collections::BTreeSet<String> = every_path
            .iter()
            .filter(|p| {
                !ignored(dir.path(), p, false) && !redline_model::files::under_graft(dir.path(), p, false)
            })
            .map(|p| {
                p.strip_prefix(dir.path())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            index_kept,
            list.files.iter().cloned().collect::<std::collections::BTreeSet<_>>(),
            "the index's changed-path filter must agree with the file walk"
        );
        // Pin the shared predicate on each rule kind (the one that
        // discriminates a root-only check is the nested one).
        assert!(
            ignored(dir.path(), &dir.path().join("src/gen/out.rs"), false),
            "nested .gitignore"
        );
        assert!(
            ignored(dir.path(), &dir.path().join("crash.log"), false),
            "root .gitignore file rule"
        );
        assert!(
            ignored(dir.path(), &dir.path().join("build/out.bin"), false),
            "directory rule (build/)"
        );
        assert!(
            !ignored(dir.path(), &dir.path().join("important.log"), false),
            "negation (!important.log)"
        );
        assert!(
            !ignored(dir.path(), &dir.path().join("src/visible.rs"), false),
            "non-ignored path stays"
        );
    }

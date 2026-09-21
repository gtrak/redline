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


use super::*;

    #[test]
    fn commit_editor_prefill_shows_staged_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = git_store_with_staged(dir.path());
        store.open_commit_editor();
        let ed = store.commit_editor.as_ref().unwrap();
        let text = ed.rope.to_string();
        assert!(text.starts_with("# Please enter the commit message"), "prefill: {text}");
        assert!(text.contains("#   M a.txt"), "staged file listed: {text}");
        assert!(text.contains("# Staged changes:"), "section header: {text}");
        // Cursor is at end of prefill.
        assert_eq!(ed.cursor, text.len());
    }

    #[test]
    fn commit_editor_text_edits_land_in_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = git_store_with_staged(dir.path());
        store.open_commit_editor();
        // Type a commit message after the prefill.
        for c in "hello".chars() {
            store.commit_editor_insert(c);
        }
        let ed = store.commit_editor.as_ref().unwrap();
        let text = ed.rope.to_string();
        assert!(text.ends_with("#\nhello"), "text: {text}");
        // Backspace removes the last char.
        store.commit_editor_backspace();
        let text = store.commit_editor.as_ref().unwrap().rope.to_string();
        assert!(text.ends_with("#\nhell"), "after backspace: {text}");
    }

    #[test]
    fn commit_editor_cc_cc_extracts_message_and_commits() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut store = git_store_with_staged(root);
        store.open_commit_editor();
        // Type the message.
        for c in "my commit msg".chars() {
            store.commit_editor_insert(c);
        }
        // C-c C-c through the keymap engine.
        store.key_event(key("C-c"));
        assert_eq!(store.pending.len(), 1, "C-c arms the prefix");
        store.key_event(key("C-c"));
        // The commit should have been made.
        assert!(store.commit_editor.is_none(), "editor closed after commit");
        assert!(store.message.contains("committed"), "message: {}", store.message);
        // Verify via git CLI that the commit was created with the right message.
        let out = std::process::Command::new("git")
            .arg("-C").arg(root)
            .args(["log", "-1", "--pretty=%s"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output().unwrap();
        let msg = String::from_utf8_lossy(&out.stdout);
        assert_eq!(msg.trim(), "my commit msg", "git log: {msg}");
    }

    #[test]
    fn commit_editor_cc_k_discards() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut store = git_store_with_staged(root);
        // Capture HEAD after repo creation.
        let head_before = {
            let out = std::process::Command::new("git")
                .arg("-C").arg(root)
                .args(["rev-parse", "HEAD"])
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output().unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        store.open_commit_editor();
        for c in "should not commit".chars() {
            store.commit_editor_insert(c);
        }
        // C-c C-k aborts.
        store.key_event(key("C-c"));
        store.key_event(key("C-k"));
        assert!(store.commit_editor.is_none(), "editor closed after abort");
        assert!(store.message.contains("aborted"), "message: {}", store.message);
        // HEAD unchanged.
        let out = std::process::Command::new("git")
            .arg("-C").arg(root)
            .args(["rev-parse", "HEAD"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output().unwrap();
        let head_after = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(head_before, head_after, "HEAD unchanged after abort");
    }

    #[test]
    fn commit_editor_keybindings_resolve_through_engine() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = git_store_with_staged(dir.path());
        store.open_commit_editor();
        // C-c alone is a pending prefix (not a command by itself).
        store.key_event(key("C-c"));
        assert!(!store.pending.is_empty(), "C-c must be a pending prefix");
        // C-k completes the abort sequence.
        store.key_event(key("C-k"));
        assert!(store.pending.is_empty(), "C-c C-k resolves");
        assert!(store.commit_editor.is_none(), "editor closed by C-c C-k");
        // Re-open and test C-c C-c.
        // (We can't easily re-stage here, so just verify the engine accepts
        // the prefix again without error.)
        store.open_commit_editor();
        assert!(store.commit_editor.is_some(), "editor re-opened");
        store.key_event(key("C-c"));
        assert!(!store.pending.is_empty(), "second C-c prefix");
        store.key_event(key("C-c"));
        // This will attempt to commit (message is empty after prefill only →
        // the "empty commit message" guard fires, which is fine).
        assert!(store.message.contains("empty commit"), "guard: {}", store.message);
        store.commit_editor_abort();
        assert!(store.commit_editor.is_none());
    }

    /// Regression (review round 3): arrow keys must clear an armed `C-c`
    /// prefix. Old code let `C-c -> arrow -> C-c` resolve as the full
    /// `commit-editor-commit` sequence -- an unintended commit from a
    /// navigation reflex.
    #[test]
    fn commit_editor_arrow_clears_armed_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = git_store_with_staged(dir.path());
        store.open_commit_editor();
        let head_before = store
            .git
            .as_ref()
            .and_then(|g| g.log(None, 0, 1).ok())
            .and_then(|l| l.into_iter().next())
            .map(|e| e.short_id);

        // Arm the C-c prefix.
        store.key_event(key("C-c"));
        assert!(!store.pending.is_empty(), "C-c arms the prefix");

        // An arrow navigates the editor AND disarms the prefix.
        store.key_event(key("DOWN"));
        assert!(store.pending.is_empty(), "arrow must clear the armed prefix");
        assert!(store.commit_editor.is_some(), "editor stays open");

        // The next C-c only RE-ARMS; the commit must not fire.
        store.key_event(key("C-c"));
        assert!(!store.pending.is_empty(), "C-c re-arms after an arrow");
        assert!(store.commit_editor.is_some(), "no commit fired");
        let head_after = store
            .git
            .as_ref()
            .and_then(|g| g.log(None, 0, 1).ok())
            .and_then(|l| l.into_iter().next())
            .map(|e| e.short_id);
        assert_eq!(head_before, head_after, "HEAD must be unchanged");
    }

    #[test]
    fn commit_editor_refuses_when_nothing_staged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // A repo with a committed file but nothing staged.
        fn git_cli(dir: &std::path::Path, args: &[&str]) {
            let out = std::process::Command::new("git")
                .arg("-C").arg(dir)
                .args(args)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
        git_cli(root, &["init", "-q", "-b", "main"]);
        git_cli(root, &["config", "user.name", "Test"]);
        git_cli(root, &["config", "user.email", "test@example.com"]);
        git_cli(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git_cli(root, &["add", "a.txt"]);
        git_cli(root, &["commit", "-q", "-m", "init"]);

        let base = tempfile::tempdir().unwrap();
        let mut store = crate::app::store::AppStore::at(
            root,
            base.path().to_path_buf(),
        );
        // Ensure git is initialized in the store.
        store.open_commit_editor();
        // Unstage everything (should be nothing staged anyway).
        // Type a message.
        for c in "try to commit".chars() {
            store.commit_editor_insert(c);
        }
        // C-c C-c attempts the commit.
        store.key_event(key("C-c"));
        store.key_event(key("C-c"));
        // The editor must still be open (commit refused).
        assert!(store.commit_editor.is_some(), "editor must stay open");
        assert!(
            store.message.contains("nothing staged"),
            "message should say nothing staged: {}", store.message
        );
    }

    #[test]
    fn commit_editor_cg_clears_armed_prefix_not_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = git_store_with_staged(dir.path());
        store.open_commit_editor();

        // Type some text.
        for c in "my msg".chars() {
            store.commit_editor_insert(c);
        }

        // Arm the C-c prefix.
        store.key_event(key("C-c"));
        assert!(!store.pending.is_empty(), "C-c arms the prefix");

        // C-g must clear the pending prefix, NOT abort the editor.
        store.key_event(key("C-g"));
        assert!(store.pending.is_empty(), "C-g must clear the armed prefix");
        assert!(store.commit_editor.is_some(), "C-g must NOT abort the editor");
        // The text is preserved.
        let text = store.commit_editor.as_ref().unwrap().rope.to_string();
        assert!(text.contains("my msg"), "text preserved after C-g: {text}");
    }

    // ── issue 08: snapshot tests ────────────────────────────────────────────

    #[test]
    fn snapshot_log_entry_display() {
        let entries = [
            LogEntry { short_id: "abc1234".into(), subject: "fix: handle edge case".into(), author: "Alice".into(), time: 1_699_992_800, date: "2 hours ago".into() },
            LogEntry { short_id: "def5678".into(), subject: "feat: add new module".into(), author: "Bob".into(), time: 1_699_734_400, date: "3 days ago".into() },
        ];
        let rows: Vec<String> = entries.iter().map(log_entry_display).collect();
        insta::assert_debug_snapshot!(rows);
    }

    #[test]
    fn snapshot_blame_line_display() {
        let now = 1_700_000_000_i64;
        let lines = [
            BlameLine { line_no: 1, short_id: "aaa1111".into(), author: "Alice".into(), time: now - 7200, text: "fn main() {".into() },
            BlameLine { line_no: 2, short_id: "bbb2222".into(), author: "Bob".into(), time: now - 86400, text: "    println!(\"hi\");".into() },
            BlameLine { line_no: 3, short_id: "ccc3333".into(), author: "Alice".into(), time: now - 3600, text: "}".into() },
        ];
        let author_w = lines.iter().map(|l| l.author.chars().count()).max().unwrap_or(0).max(4);
        let rows: Vec<String> = lines.iter().map(|l| blame_line_display(l, now, author_w)).collect();
        insta::assert_debug_snapshot!(rows);
    }

    // ── issue 09 blocking finding #3: regression tests ────────────────────

    /// Commit-diff (issue 003-02): the window is bounded by the pane window,
    /// `M->` lands on the last row (top clamps), `M-<` round-trips to the top,
    /// and C-n/C-p move the window one row.
    #[test]
    fn commit_diff_window_bounded_and_motions_clamp() {
        let dir = tall_commit_repo();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.project = Some(crate::model::project::Project::new(dir.path().to_path_buf()));
        s.set_viewport_lines(8); // pane window == 6
        s.open_log();
        s.log_open_commit();
        assert_eq!(s.top_view(), ViewId::CommitDiff);

        let total = s.commit_diff_rows().len();
        assert!(total > 6, "commit diff must overflow the window: {total} rows");

        // Window bounded at the top.
        let (win, top, tot) = s.commit_diff_view_info();
        assert_eq!(tot, total);
        assert!(win.len() <= 6, "window must be bounded: {} rows", win.len());
        assert_eq!(top, 0);

        // C-n advances the top one row; C-p steps back.
        s.key_event(key("C-n"));
        assert_eq!(s.commit_diff_view_info().1, 1, "C-n must advance the top");
        s.key_event(key("C-p"));
        assert_eq!(s.commit_diff_view_info().1, 0, "C-p must step back");

        // M-> (scroll to bottom) lands the top on the full last page; the
        // last visible row is the diff's final row.
        let w = pane_window(s.viewport_lines);
        s.key_event(key("M->"));
        let (win_b, top_b, _) = s.commit_diff_view_info();
        assert_eq!(top_b, total - w, "M-> must land on the full last page: got {top_b}");
        assert_eq!(
            win_b[win_b.len() - 1].text,
            s.commit_diff_rows()[total - 1].text,
            "the last visible row must be the diff's final row"
        );

        // M-< (scroll to top) round-trips.
        s.key_event(key("M-<"));
        assert_eq!(s.commit_diff_view_info().1, 0, "M-< must round-trip to top");

        // C-v / M-v page: a page-down advances the top by more than a line
        // (the pane window minus a 2-line overlap), and M-v brings it back.
        s.key_event(key("C-v"));
        let top_page = s.commit_diff_view_info().1;
        assert!(top_page > 1, "C-v must page down (more than one row): got {top_page}");
        s.key_event(key("M-v"));
        assert_eq!(s.commit_diff_view_info().1, 0, "M-v must page back to top");
    }

    /// Blame (issue 003-02): the cursor-following window keeps `b.selected`
    /// in view on every move; the top advances as the cursor passes it and
    /// clamps so the last row stays visible.
    #[test]
    fn blame_cursor_stays_in_window_on_moves() {
        let dir = tempfile::tempdir().unwrap();
        git_test_cli(dir.path(), &["init", "-q", "-b", "main"]);
        git_test_cli(dir.path(), &["config", "user.name", "Test"]);
        git_test_cli(dir.path(), &["config", "user.email", "t@e.com"]);
        git_test_cli(dir.path(), &["config", "commit.gpgsign", "false"]);
        let mut content = String::new();
        for i in 1..=60 {
            content.push_str(&format!("line {i}\n"));
        }
        std::fs::write(dir.path().join("big.txt"), content).unwrap();
        git_test_cli(dir.path(), &["add", "-A"]);
        git_test_cli(dir.path(), &["commit", "-q", "-m", "one"]);

        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.project = Some(crate::model::project::Project::new(dir.path().to_path_buf()));
        s.set_viewport_lines(8); // pane window == 6
        s.open_path("big.txt");
        s.open_blame();
        assert_eq!(s.top_view(), ViewId::Blame);

        let total = s.blame_rows().len(); // header + 60 lines
        assert!(total > 6, "blame must overflow the window: {total} rows");

        // Initially the cursor is on the first line (row 1); the window top is 0.
        let (win, top, tot) = s.blame_view_info();
        assert_eq!(tot, total);
        assert!(win.len() <= 6);
        assert_eq!(top, 0);
        assert!(s.blame.as_ref().unwrap().selected + 1 < top + win.len());

        // Move the cursor down past the first window; it must stay in view and
        // the top must advance.
        for _ in 0..10 {
            s.key_event(key("C-n"));
        }
        let (win2, top2, _) = s.blame_view_info();
        let sel = s.blame.as_ref().unwrap().selected + 1;
        assert!(
            (top2..top2 + win2.len()).contains(&sel),
            "cursor row {sel} must be in window [{top2},{}): top={top2}",
            top2 + win2.len()
        );
        assert!(top2 > 0, "window must have scrolled down: {top2}");

        // M-> moves the cursor to the last line; the top clamps so the last row
        // is the last visible row.
        s.key_event(key("M->"));
        let (win3, top3, _) = s.blame_view_info();
        let last = s.blame.as_ref().unwrap().selected + 1;
        assert_eq!(last, total - 1, "M-> must move the cursor to the last row");
        assert!(
            (top3..top3 + win3.len()).contains(&last),
            "last row {last} must be in window [{top3},{}): top={top3}",
            top3 + win3.len()
        );
        assert_eq!(top3, 55, "top must clamp to show the last row: got {top3}");

        // M-< round-trips the cursor to the first row.
        s.key_event(key("M-<"));
        let (win4, top4, _) = s.blame_view_info();
        let first = s.blame.as_ref().unwrap().selected + 1;
        assert_eq!(first, 1, "M-< must move the cursor to the first row");
        assert!((top4..top4 + win4.len()).contains(&first));
    }

    /// Log in-page (issue 003-02): the in-page motion (arrows / j / k) keeps
    /// the selection inside the visible window; paging (`n`/`p`) resets the
    /// window to the top and is unchanged.
    #[test]
    fn log_in_page_selection_stays_in_window() {
        let dir = tempfile::tempdir().unwrap();
        git_test_cli(dir.path(), &["init", "-q", "-b", "main"]);
        git_test_cli(dir.path(), &["config", "user.name", "Test"]);
        git_test_cli(dir.path(), &["config", "user.email", "t@e.com"]);
        git_test_cli(dir.path(), &["config", "commit.gpgsign", "false"]);
        for i in 0..30 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), "x\n").unwrap();
            git_test_cli(dir.path(), &["add", "-A"]);
            git_test_cli(dir.path(), &["commit", "-q", "-m", &format!("commit {i}")]);
        }

        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.project = Some(crate::model::project::Project::new(dir.path().to_path_buf()));
        s.set_viewport_lines(8); // pane window == 6
        s.open_log();
        assert_eq!(s.top_view(), ViewId::Log);

        let total = s.log_rows().len(); // header + up-to-25 entries + footer
        assert!(total > 6, "log page must overflow the window: {total} rows");

        // Move the in-page selection down with `j`; it must stay in view and
        // the top must advance.
        for _ in 0..20 {
            s.key_event(key("j"));
        }
        let (win, top, _) = s.log_view_info();
        let sel = s.log.as_ref().unwrap().selected + 1;
        assert!(
            (top..top + win.len()).contains(&sel),
            "log selection row {sel} must be in window [{top},{}): top={top}",
            top + win.len()
        );
        assert!(top > 0, "log window must have scrolled: {top}");

        // Paging (`n`) resets the in-page window to the top.
        s.key_event(key("n"));
        let (_, top_paged, _) = s.log_view_info();
        assert_eq!(top_paged, 0, "n (next page) must reset the window to the top");
    }


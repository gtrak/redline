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
        git_repo_init(root, "Test", "test@example.com", true);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git_cli(root, &["add", "a.txt"], "Test", "test@example.com");
        git_cli(root, &["commit", "-q", "-m", "init"], "Test", "test@example.com");

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
        // The original windowing fixture identity ("Test"/"t@e.com"), kept verbatim.
        git_repo_init(dir.path(), "Test", "t@e.com", true);
        let mut content = String::new();
        for i in 1..=60 {
            content.push_str(&format!("line {i}\n"));
        }
        std::fs::write(dir.path().join("big.txt"), content).unwrap();
        git_cli(dir.path(), &["add", "-A"], "Test", "t@e.com");
        git_cli(dir.path(), &["commit", "-q", "-m", "one"], "Test", "t@e.com");

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
        // The original windowing fixture identity ("Test"/"t@e.com"), kept verbatim.
        git_repo_init(dir.path(), "Test", "t@e.com", true);
        for i in 0..30 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), "x\n").unwrap();
            git_cli(dir.path(), &["add", "-A"], "Test", "t@e.com");
            git_cli(dir.path(), &["commit", "-q", "-m", &format!("commit {i}")], "Test", "t@e.com");
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


    // ── issue-commit-editor-pasted-newline: a pasted newline is C-j ──────

    /// issue-commit-editor-pasted-newline: a terminal paste arrives as
    /// ordinary key bytes (the app requests no bracketed paste). Delegates to
    /// the shared `crate::test_support::paste_keys` (single source of truth
    /// for the measured paste decode — the LF byte arrives as C-j, not
    /// `Enter`, not a literal `Char('\\n')`).
    fn paste_keys(paste: &str) -> Vec<crate::app::keymap::Key> {
        crate::test_support::paste_keys(paste)
    }

    /// issue-commit-editor-pasted-newline: pasting a multi-line commit
    /// message must insert REAL newlines, byte-exactly, into the editor
    /// buffer. The pre-fix build dropped every LF (it arrived as C-j,
    /// unbound in this modal), so the two lines concatenated with no
    /// separator. Asserted as ONE whole-value equality on the buffer's
    /// exact bytes — an `in`/`contains` check could not falsify a dropped
    /// newline (the probe that missed the notes bug is the cautionary
    /// example): the newlines are present (positive) AND the surrounding
    /// prefill text is undisturbed (negative).
    #[test]
    fn pasted_newline_in_commit_editor_lands_byte_exact_in_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = git_store_with_staged(dir.path());
        store.open_commit_editor();
        let prefill = store.commit_editor.as_ref().unwrap().rope.to_string();
        for k in paste_keys("line1\nline2\n") {
            store.key_event(k);
        }
        let text = store.commit_editor.as_ref().unwrap().rope.to_string();
        assert_eq!(
            text,
            format!("{prefill}line1\nline2\n"),
            "pasted newlines must land byte-exact in the commit message buffer (no dropped LF, no glued lines)"
        );
    }

    /// issue-commit-editor-pasted-newline: typing a newline by hand (RET
    /// -> Enter) and pasting one (LF -> C-j) must produce BYTE-IDENTICAL
    /// commit-message buffers — the acceptance criterion is "a paste
    /// behaves the same as typing the same characters by hand".
    #[test]
    fn typed_and_pasted_newline_are_byte_identical_in_commit_editor() {
        // Typed: "line1", RET (Enter), "line2", RET.
        let dir = tempfile::tempdir().unwrap();
        let mut typed = git_store_with_staged(dir.path());
        typed.open_commit_editor();
        let prefill = typed.commit_editor.as_ref().unwrap().rope.to_string();
        for c in "line1".chars() {
            typed.key_event(crate::app::keymap::Key::new(
                crate::app::keymap::KeyCode::Char(c),
            ));
        }
        typed.key_event(key("RET"));
        for c in "line2".chars() {
            typed.key_event(crate::app::keymap::Key::new(
                crate::app::keymap::KeyCode::Char(c),
            ));
        }
        typed.key_event(key("RET"));
        // Pasted: the same characters, with the LFs decoded as C-j.
        let dir2 = tempfile::tempdir().unwrap();
        let mut pasted = git_store_with_staged(dir2.path());
        pasted.open_commit_editor();
        for k in paste_keys("line1\nline2\n") {
            pasted.key_event(k);
        }
        let typed_text = typed.commit_editor.as_ref().unwrap().rope.to_string();
        let pasted_text = pasted.commit_editor.as_ref().unwrap().rope.to_string();
        assert_eq!(
            typed_text, pasted_text,
            "a pasted newline must be byte-identical to a typed one"
        );
        assert_eq!(
            typed_text,
            format!("{prefill}line1\nline2\n"),
            "and both must be byte-exact (surrounding prefill undisturbed)"
        );
    }

    /// issue-commit-editor-pasted-newline: end-to-end on disk — a pasted
    /// multi-line message survives the commit, and the committed message
    /// read back from git is byte-exact. `git log --pretty=%B` returns the
    /// full multi-line body (a subject-only `%s` read-back could not see a
    /// dropped second line the way the notes probe's `in` check could not).
    #[test]
    fn pasted_multiline_commit_message_commits_byte_exact() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut store = git_store_with_staged(root);
        store.open_commit_editor();
        for k in paste_keys("line1\nline2\n") {
            store.key_event(k);
        }
        store.key_event(key("C-c"));
        store.key_event(key("C-c"));
        assert!(store.commit_editor.is_none(), "editor closed after commit");
        let out = std::process::Command::new("git")
            .arg("-C").arg(root)
            .args(["log", "-1", "--pretty=%B"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output().unwrap();
        assert_eq!(
            &out.stdout[..],
            b"line1\nline2\n",
            "committed message must be byte-exact (the pasted LF must survive)"
        );
    }

    // ── U-E11: log query-level narrowing (018-fu-log-query-narrow) ──────

    /// The U-E11 fixture: `n` commits, even-index subjects `omga {i}`,
    /// odd-index `beta {i}` (15 each at n = 30). Neither word carries a
    /// `LOG_BINDINGS` key (n/p/j/k/q), so the whole query reaches the
    /// prompt guard. `omga` is a subsequence of NO `beta {i}` subject (no
    /// `o` in it), so the shared core's scoring is exact on this fixture.
    fn log_narrow_repo(dir: &std::path::Path, n: usize) {
        git_repo_init(dir, "Test", "test@example.com", true);
        for i in 0..n {
            std::fs::write(dir.join("f.txt"), format!("rev {i}\n")).unwrap();
            git_cli(dir, &["add", "f.txt"], "Test", "test@example.com");
            let subj = if i % 2 == 0 { format!("omga {i}") } else { format!("beta {i}") };
            git_cli(dir, &["commit", "-q", "-m", &subj], "Test", "test@example.com");
        }
    }

    /// A store rooted in `dir` with the project set (the U-E11 pins'
    /// fixture root).
    fn log_narrow_store(dir: &std::path::Path) -> AppStore {
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir, base.path().to_path_buf());
        s.project = Some(crate::model::project::Project::new(dir.to_path_buf()));
        s
    }

    /// U-E11: the log narrow-prompt guard — printable chars extend the
    /// query live, Backspace pops, and the view's own bound keys (every
    /// single-char `LOG_BINDINGS` binding — the n/p page keys AND the j/k
    /// in-page motion keys, keymap-derived, never copied) fall through to
    /// the keymap instead of the query.
    #[test]
    fn log_narrow_guard_extends_the_query_and_bound_keys_fall_through() {
        let dir = tempfile::tempdir().unwrap();
        log_narrow_repo(dir.path(), 30);
        let mut s = log_narrow_store(dir.path());
        s.open_log();
        assert_eq!(s.top_view(), ViewId::Log);
        assert_eq!(s.log.as_ref().unwrap().total, 30, "the unfiltered total");

        for c in "omga".chars() {
            s.key_event(Key::char(c));
        }
        assert_eq!(s.log_narrow_query(), "omga", "the typed chars must reach the log narrow query (the prompt guard)");

        // `n` pages (the filtered set), not the query: at offset 0 the
        // page holds all 15 matches (15 < LOG_PAGE), so `n` is the
        // end-of-log boundary — its echo lands and the query stays put.
        s.key_event(key("n"));
        assert_eq!(s.log_narrow_query(), "omga", "`n` must fall through, not extend the query");
        assert_eq!(s.message, "end of log", "`n` reached the keymap: {}", s.message);
        // `p` at offset 0: start-of-log boundary echo, query untouched.
        s.key_event(key("p"));
        assert_eq!(s.log_narrow_query(), "omga", "`p` must fall through, not extend the query");
        assert_eq!(s.message, "start of log", "`p` reached the keymap: {}", s.message);
        // `j`/`k` move the in-page selection (the query keys must not
        // steal the motion keys).
        s.key_event(key("j"));
        assert_eq!(s.log.as_ref().unwrap().selected, 1, "`j` must move the selection, not extend the query");
        s.key_event(key("k"));
        assert_eq!(s.log.as_ref().unwrap().selected, 0, "`k` must move the selection back");
        assert_eq!(s.log_narrow_query(), "omga", "`j`/`k` must not extend the query");
        // `q` closes the view; the query state persists store-side (the
        // magit precedent — a re-open re-derives under the SAME query).
        s.key_event(key("q"));
        assert_ne!(s.top_view(), ViewId::Log, "`q` must close the log view");
        assert_eq!(s.log_narrow_query(), "omga", "the query persists across the close");
        // The re-open re-derives the page under the same query (open_log's
        // re-derive), and Backspace then pops the last character.
        s.open_log();
        assert_eq!(s.log.as_ref().unwrap().total, 15, "the re-open re-derives under the same query");
        s.key_event(Key::new(KeyCode::Backspace));
        assert_eq!(s.log_narrow_query(), "omg");
    }

    /// U-E11 (the load-bearing invariant): the query re-walks with the
    /// filter BEFORE offset/limit — `total` is the FILTERED count (15 of
    /// 30, asserted as the number, not "changed"), and `n`/`p` page WITHIN
    /// the filtered set (a client-side filter of the fetched page would
    /// keep `total` = 30 and could page onto `beta` rows — PLAN 018
    /// §2.3-4's forbidden shape).
    #[test]
    fn log_narrow_total_is_the_filtered_count_and_paging_walks_within_the_set() {
        let dir = tempfile::tempdir().unwrap();
        // 60 commits: 30 `omga` (even) + 30 `beta` (odd) — the filtered
        // set (30) overflows LOG_PAGE (25), so paging inside it is real.
        log_narrow_repo(dir.path(), 60);
        let mut s = log_narrow_store(dir.path());
        s.open_log();
        assert_eq!(s.log.as_ref().unwrap().total, 60, "the unfiltered total");

        for c in "omga".chars() {
            s.key_event(Key::char(c));
        }
        let log = s.log.as_ref().unwrap();
        assert_eq!(log.total, 30, "total is the FILTERED count (15+15 omga commits): got {}", log.total);
        assert_eq!(log.entries.len(), 25, "the first filtered page fills LOG_PAGE");
        assert_eq!(log.entries[0].subject, "omga 58", "newest match first: {:?}", log.entries.iter().map(|e| e.subject.as_str()).collect::<Vec<_>>());
        assert_eq!(log.entries[24].subject, "omga 10");
        assert!(
            log.entries.iter().all(|e| e.subject.starts_with("omga")),
            "no beta row may leak into the filtered page"
        );
        // The footer measures the filtered set on screen.
        let footer = s.log_rows().last().unwrap().text.clone();
        assert!(footer.contains("(1–25/30"), "the footer counts the filtered set: {footer}");

        // `n`: the NEXT filtered page (offset 25 — positions 26–30 of the
        // filtered set), not an unfiltered-range fetch.
        s.key_event(key("n"));
        let log = s.log.as_ref().unwrap();
        assert_eq!(log.offset, 25);
        assert_eq!(log.total, 30, "the filtered total survives the page step");
        assert_eq!(log.entries.len(), 5, "the tail page holds the remaining 5 matches");
        assert_eq!(log.entries[0].subject, "omga 8");
        assert_eq!(log.entries[4].subject, "omga 0");
        let footer = s.log_rows().last().unwrap().text.clone();
        assert!(footer.contains("(26–30/30"), "the footer counts the filtered set: {footer}");

        // The boundaries are the filtered set's: `n` at the filtered end,
        // `p`/`p` back to the top and the unfiltered offset 0.
        s.key_event(key("n"));
        assert_eq!(s.message, "end of log", "the filtered set's end, not the full walk's: {}", s.message);
        s.key_event(key("p"));
        assert_eq!(s.log.as_ref().unwrap().offset, 0, "`p` steps back within the filtered set");
        assert_eq!(s.log.as_ref().unwrap().entries.len(), 25);
        s.key_event(key("p"));
        assert_eq!(s.message, "start of log", "the filtered set's start: {}", s.message);
    }

    /// U-E11: the query matches the AUTHOR field too (the `git log
    /// --author` surface) — `alice` keeps exactly Alice's commits, in the
    /// walk's order (FilterOnly: never re-ranked).
    #[test]
    fn log_narrow_author_query_matches_the_author_not_the_subject() {
        let dir = tempfile::tempdir().unwrap();
        git_repo_init(dir.path(), "Alice", "alice@example.com", true);
        for (i, who, email) in [
            (0, "Alice", "alice@example.com"),
            (1, "Bob", "bob@example.com"),
            (2, "Alice", "alice@example.com"),
            (3, "Bob", "bob@example.com"),
        ] {
            std::fs::write(dir.path().join("f.txt"), format!("rev {i}\n")).unwrap();
            git_cli(dir.path(), &["add", "f.txt"], who, email);
            git_cli(dir.path(), &["commit", "-q", "-m", &format!("change {i}")], who, email);
        }
        let mut s = log_narrow_store(dir.path());
        s.open_log();
        assert_eq!(s.log.as_ref().unwrap().total, 4);

        for c in "alice".chars() {
            s.key_event(Key::char(c));
        }
        let log = s.log.as_ref().unwrap();
        assert_eq!(log.total, 2, "total is the author-filtered count: {:?}", log.entries.iter().map(|e| (e.subject.as_str(), e.author.as_str())).collect::<Vec<_>>());
        assert_eq!(log.entries[0].subject, "change 2", "newest Alice commit first");
        assert_eq!(log.entries[1].subject, "change 0");
        assert!(log.entries.iter().all(|e| e.author == "Alice"), "every surviving row is Alice's");
    }

    /// U-E11: a query matching nothing — total 0, the view renders empty
    /// without panicking, and the cursor/selected state is sane (the
    /// stale-index class U-E10 caught: `selected` stays 0, in-page motion
    /// is a no-op, RET has no commit at point, and the paging boundaries
    /// echo on the EMPTY set). C-g restores the full unfiltered log.
    #[test]
    fn log_narrow_no_match_renders_empty_and_state_is_sane() {
        let dir = tempfile::tempdir().unwrap();
        log_narrow_repo(dir.path(), 30);
        let mut s = log_narrow_store(dir.path());
        s.open_log();

        for c in "zzz".chars() {
            s.key_event(Key::char(c));
        }
        let log = s.log.as_ref().unwrap();
        assert_eq!(log.total, 0, "nothing matches `zzz`");
        assert!(log.entries.is_empty());
        assert_eq!(log.selected, 0, "selected stays 0 on the empty set (the stale-index class)");
        // The view renders the empty page without panicking: header +
        // footer only, no selected row, one sane window.
        let rows = s.log_rows();
        assert_eq!(rows.len(), 2, "the empty page is header + footer: {rows:?}");
        assert!(!rows.iter().any(|r| r.selected), "no blue row on the empty set: {rows:?}");
        let (win, top, total) = s.log_view_info();
        assert_eq!((win.len(), top, total), (2, 0, 2), "the windowing reads the empty page sanely");
        // In-page motion is a no-op on the empty page (no panic, no
        // stale index).
        s.key_event(key("j"));
        s.key_event(key("k"));
        assert_eq!(s.log.as_ref().unwrap().selected, 0, "motion is a no-op on the empty set");
        // RET has no commit at point (the page holds none).
        s.key_event(key("RET"));
        assert_eq!(s.top_view(), ViewId::Log, "RET must not push a diff on the empty set");
        assert_eq!(s.message, "no commit at point", "the empty-set echo: {}", s.message);
        // The paging boundaries echo on the EMPTY set (0 + 0 >= 0 and
        // offset == 0).
        s.key_event(key("n"));
        assert_eq!(s.message, "end of log");
        s.key_event(key("p"));
        assert_eq!(s.message, "start of log");
        // C-g clears: the full unfiltered log re-derives, total back to 30.
        s.key_event(key("C-g"));
        let log = s.log.as_ref().unwrap();
        assert_eq!(s.log_narrow_query(), "");
        assert_eq!(s.message, "filter cleared");
        assert_eq!(log.total, 30, "the unfiltered total returns");
        assert_eq!(log.entries.len(), 25, "the first full page re-derives");
        assert_eq!(s.top_view(), ViewId::Log, "C-g must not close the view");
    }

    /// U-E11: C-g CLEARS the query (the full unfiltered log re-derives —
    /// total back to the full count, offset 0) — NOT cancel/close (the
    /// v1 decision, the buffer-list/magit precedent; closing stays on q).
    /// C-g on an empty query is the same clear path (the magit pin's
    /// shape), not a view close.
    #[test]
    fn log_narrow_c_g_clears_and_total_returns_to_the_unfiltered_count() {
        let dir = tempfile::tempdir().unwrap();
        log_narrow_repo(dir.path(), 30);
        let mut s = log_narrow_store(dir.path());
        s.open_log();
        for c in "omga".chars() {
            s.key_event(Key::char(c));
        }
        assert_eq!(s.log.as_ref().unwrap().total, 15, "narrowed before the clear");
        s.key_event(key("C-g"));
        assert_eq!(s.log_narrow_query(), "");
        assert_eq!(s.message, "filter cleared");
        let log = s.log.as_ref().unwrap();
        assert_eq!(log.total, 30, "the full unfiltered total re-derives");
        assert_eq!(log.offset, 0, "the re-derive starts at the walk's top");
        assert_eq!(log.entries.len(), 25, "the first full page re-derives");
        assert_eq!(s.top_view(), ViewId::Log, "C-g must not close the view");
        // C-g on the now-empty query: the clear no-op path, not a close.
        s.key_event(key("C-g"));
        assert_eq!(s.message, "filter cleared", "C-g on an empty query is the clear path, not a view close");
        assert_eq!(s.top_view(), ViewId::Log);
        assert_eq!(s.log.as_ref().unwrap().total, 30);
    }

    /// U-E11 class-bug pin (the pending-sequence discipline, the C-x 2
    /// twin): a chord that arms a prefix must NOT be stranded by the
    /// narrow-prompt guard. With the log view on top, `C-x` reaches the
    /// engine (arms the `C-x` prefix — it carries no char value, so the
    /// guard cannot swallow it), and the follow-up `2` must reach the
    /// engine too, echoing the unbound-key dead end. On the pre-fix guard
    /// (key tested in isolation) the `2` would be consumed into the query,
    /// stranding the sequence. The guard now composes the armed prefix
    /// with the key: a non-empty `self.pending` means the key MUST reach
    /// the engine, so the query stays empty and the echo lands.
    #[test]
    fn log_narrow_guard_reaches_the_engine_for_a_pending_chord_c_x_2() {
        let dir = tempfile::tempdir().unwrap();
        log_narrow_repo(dir.path(), 30);
        let mut s = log_narrow_store(dir.path());
        s.open_log();
        s.key_event(key("C-x"));
        assert!(s.log_narrow_query().is_empty(), "a bare C-x must not feed the narrow query");
        s.key_event(key("2"));
        assert_eq!(s.log_narrow_query(), "", "the `2` must NOT be swallowed into the narrow query");
        assert!(
            s.message.contains("unbound key: 2"),
            "the `2` reaches the engine and dead-ends to the unbound echo: {}",
            s.message
        );
        assert_eq!(s.top_view(), ViewId::Log, "no view change on the echo");
    }

    /// U-E11 (the `7f0090a` keys-first rule): the prompt row is ONE NoWrap
    /// row directly under the title, and EVERY decision group survives the
    /// 80-col clip AND leads the query — a right-edge clip eats the query
    /// tail, never the keys.
    #[test]
    fn log_narrow_prompt_row_keys_lead_at_80_with_long_query() {
        use crate::ui::root::render_at_width;

        let dir = tempfile::tempdir().unwrap();
        log_narrow_repo(dir.path(), 6);
        let mut s = log_narrow_store(dir.path());
        s.open_log();
        // "omga" + 200 x's (`x` is not a decision key, so the whole run
        // reaches the query).
        for c in "omga".chars() {
            s.key_event(Key::char(c));
        }
        for _ in 0..200 {
            s.key_event(Key::char('x'));
        }
        let frame = render_at_width(s, 80);
        let lines: Vec<&str> = frame.lines().collect();
        let title_idx = lines
            .iter()
            .position(|l| l.contains("log —"))
            .unwrap_or_else(|| panic!("title row missing\n{frame}"));
        let prompt_idx = lines
            .iter()
            .position(|l| l.contains("n/p page"))
            .unwrap_or_else(|| panic!("prompt row missing\n{frame}"));
        assert_eq!(
            prompt_idx,
            title_idx + 1,
            "the prompt row sits directly under the title\n{frame}"
        );
        let prompt = lines[prompt_idx];
        // One row, NoWrap: at a 204-char query the row is filled to the
        // edge with the query tail (a wrap would leave the edge empty).
        assert!(prompt.trim_end().ends_with('x'), "the prompt row is one NoWrap row filled by the query: {prompt:?}");
        let query_at = prompt.find('x').unwrap();
        for group in ["n/p page", "RET diff", "C-g clear", "q close"] {
            assert!(
                prompt.contains(group),
                "decision group {group:?} must survive the 80-col clip: {prompt:?}"
            );
            assert!(
                prompt.find(group).unwrap() < query_at,
                "decision group {group:?} must LEAD the query: {prompt:?}"
            );
        }
        // The query tail is the clipped part: a 50-run of x's cannot fit
        // after the 43-char decision-key prefix (at most 33 cells remain).
        assert!(
            !prompt.contains(&"x".repeat(50)),
            "the 204-char query cannot fit: its tail is the clipped part: {prompt:?}"
        );
    }

    /// U-E11: at rest (no query) the prompt row shows the decision keys +
    /// the `type to narrow` placeholder — the shape pin of the at-rest
    /// frame (the live-frame assertions live in the drive).
    #[test]
    fn log_narrow_prompt_row_at_rest_shows_keys_and_placeholder() {
        use crate::ui::root::render_at_width;

        let dir = tempfile::tempdir().unwrap();
        log_narrow_repo(dir.path(), 6);
        let mut s = log_narrow_store(dir.path());
        s.open_log();
        let frame = render_at_width(s, 80);
        let lines: Vec<&str> = frame.lines().collect();
        let title_idx = lines
            .iter()
            .position(|l| l.contains("log —"))
            .unwrap_or_else(|| panic!("title row missing\n{frame}"));
        let prompt = lines.get(title_idx + 1).copied().unwrap_or("");
        assert!(prompt.contains("type to narrow"), "the empty-query placeholder: {prompt:?}");
        for group in ["n/p page", "RET diff", "C-g clear", "q close"] {
            assert!(prompt.contains(group), "decision group {group:?} missing at rest: {prompt:?}");
        }
        // The un-narrowed list renders below the prompt row (both
        // omga rows of the 6-commit fixture).
        assert!(
            lines.iter().any(|l| l.contains("omga 4")),
            "the newest commit row renders: {frame}"
        );
        assert!(
            lines.iter().any(|l| l.contains("beta 5")),
            "the beta row renders: {frame}"
        );
    }

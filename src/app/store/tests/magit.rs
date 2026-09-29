use super::*;

    #[test]
    fn tree_click_row_selects_visible_row_and_leaves_point_alone() {
        // With the tree visible, a click in the tree's columns selects the
        // tree row under it and NEVER moves the code point.
        let (mut s, _dir) = store_with_lines(100);
        s.ensure_files();
        s.toggle_tree();
        assert!(s.tree_visible());
        let n = s.tree_rows().len();
        assert!(n >= 2, "need >=2 tree rows: {n}");

        // The code point starts away from (0,0) so any movement is visible.
        s.set_point(3, 2, 0);

        // Terminal row 0 is the tree title and row 1 is the U-E12 narrow
        // prompt: both are no-ops (selection + point).
        s.tree_click_row(0);
        assert_eq!(s.tree_selected(), 0);
        assert_eq!((s.point_line(), s.point_col()), (3, 2), "title-row click must not move the code point");
        s.tree_click_row(1);
        assert_eq!(s.tree_selected(), 0, "prompt-row click is a no-op (U-E12)");
        assert_eq!((s.point_line(), s.point_col()), (3, 2), "prompt-row click must not move the code point");

        // Terminal row 3 → visible row 1 (window top is `selected-5`, 0
        // here; the window starts at row 2 under the U-E12 prompt row):
        // the selection moves, the point does not.
        s.tree_click_row(3);
        assert_eq!(s.tree_selected(), 1, "terminal row 3 → tree row 1");
        assert_eq!((s.point_line(), s.point_col()), (3, 2), "tree-row click must not move the code point");

        // The help row (row TREE_VISIBLE_ROWS+2 = 10, title + prompt above
        // the 8-row window) is a no-op.
        s.tree_click_row(10);
        assert_eq!(s.tree_selected(), 1);
        // A far row (past the window) is a no-op.
        s.tree_click_row(99);
        assert_eq!(s.tree_selected(), 1);

        // With the tree hidden every tree click is a no-op.
        s.toggle_tree();
        assert!(!s.tree_visible());
        s.tree_click_row(3);
        assert_eq!(s.tree_selected(), 1, "hidden tree: click is a no-op");
    }

    #[test]
    fn tree_click_row_selects_within_a_scrolled_window() {
        // Window top is `selected - 5`: after moving the selection to row 7
        // (of 8 rows) the visible window starts at row 2, so terminal row 2
        // (the first visible row, under the U-E12 prompt row) selects tree
        // row 2.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        for i in 0..8 {
            std::fs::write(root.join(format!("f{i}.rs")), "x\n").unwrap();
        }
        let mut s = store(root);
        s.ensure_files();
        s.toggle_tree();
        let n = s.tree_rows().len();
        assert!(n >= 8, "need >=8 tree rows: {n}");
        s.tree.selected = 7;
        s.tree_click_row(2);
        assert_eq!(s.tree_selected(), 2, "terminal row 2 → window row 0 → tree row 2");
        assert_eq!((s.point_line(), s.point_col()), (0, 0), "selection only; point untouched");
    }

    #[test]
    fn apply_project_change_tracked_path_refreshes_magit_counts() {
        // F4 carried leg: the classifier (any_tracked) is tested separately;
        // this drives the magit-REFRESH leg inside apply_project_change —
        // a tracked-path change must update the dirty counts.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // The magit windowing test's short "T"/"t@e.com" identity, kept
        // verbatim (it only sets commit metadata).
        git_repo_init(root, "T", "t@e.com", false);
        std::fs::write(root.join("tracked.rs"), "fn a() {}\n").unwrap();
        git_cli(root, &["add", "tracked.rs"], "T", "t@e.com");
        git_cli(root, &["commit", "-q", "-m", "init"], "T", "t@e.com");

        let mut s = store(root);
        // Prime the repo (git=Some) and refresh (clean repo → all-zero counts).
        s.open_magit_status();
        let clean = s.dirty_counts().expect("magit status must populate dirty counts");
        assert_eq!(
            clean.staged + clean.unstaged + clean.untracked,
            0,
            "clean repo must report zero dirty: {clean:?}"
        );

        // A tracked file changes on disk → an unstaged modification appears.
        let proj_root = s.project.as_ref().unwrap().root.clone();
        std::fs::write(proj_root.join("tracked.rs"), "fn a() {}\nfn b() {}\n").unwrap();

        // The watcher's apply_project_change must run the refresh leg.
        s.apply_project_change(&change(vec![proj_root.join("tracked.rs")]));
        let d = s.dirty_counts().expect("refresh leg must keep dirty counts populated");
        assert!(d.unstaged >= 1, "tracked-path change must show an unstaged count: {d:?}");
    }

    #[test]
    fn tree_toggle_builds_rows_and_navigates() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        project_with_files(root);
        let mut store = store(root);
        // Ensure the file list is cached.
        store.ensure_files();
        // Toggle on: builds rows.
        store.toggle_tree();
        assert!(store.tree_visible());
        let rows = store.tree_rows();
        assert!(!rows.is_empty(), "tree rows must be non-empty");
        assert!(rows.iter().any(|r| r.name == "main.rs"));
        assert!(rows.iter().any(|r| r.name == "lib.rs"));
        // Navigate down.
        let initial = store.tree_selected();
        store.tree_move_down();
        assert_eq!(store.tree_selected(), initial + 1);
        // Navigate up (wraps to 0).
        store.tree_move_up();
        store.tree_move_up();
        assert_eq!(store.tree_selected(), 0);
        // Toggle off.
        store.toggle_tree();
        assert!(!store.tree_visible());
    }

    #[test]
    fn tree_reset_on_project_switch() {
        // Two projects in separate tempdirs.
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let root1 = dir1.path();
        let root2 = dir2.path();
        std::fs::write(root1.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root2.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root1.join("file1.rs"), "one\n").unwrap();
        std::fs::write(root2.join("file2.rs"), "two\n").unwrap();
        let mut store = store(root1);
        store.ensure_files();
        store.toggle_tree();
        assert!(store.tree_visible());
        assert!(!store.tree_rows().is_empty());
        // Switch to project 2.
        let root2_str = root2.to_string_lossy().to_string();
        store.switch_project_root(&root2_str);
        // Tree rows must be cleared (not stale from project 1).
        assert!(store.tree_rows().is_empty(), "tree rows must be empty after project switch");
    }

    // ── plan 007 issue 04: incremental parse retention ────────────

    #[test]
    fn tree_rebuild_on_re_walk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.join("a.rs"), "a\n").unwrap();
        let mut store = store(root);
        store.ensure_files();
        store.toggle_tree();
        let count_before = store.tree_rows().len();
        // Add a new file and re-walk.
        std::fs::write(root.join("b.rs"), "b\n").unwrap();
        store.re_walk();
        let count_after = store.tree_rows().len();
        assert!(count_after > count_before, "re-walk must add the new file to the tree");
        assert!(store.tree_rows().iter().any(|r| r.name == "b.rs"));
    }

    #[test]
    fn tree_buffer_follow_moves_cursor_on_open_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        project_with_files(root);
        let mut store = store(root);
        store.ensure_files();
        store.toggle_tree();
        assert!(store.tree_visible());
        let lib_row = store
            .tree_rows()
            .iter()
            .position(|r| r.rel_path == "src/lib.rs")
            .expect("src/lib.rs must be a tree row");
        // Off by default: opening a file must not move the tree cursor.
        store.open_path("src/main.rs");
        assert_eq!(store.tree_selected(), 0);
        // Enable follow via the M-x command (the shipped UI path).
        store.dispatch("toggle-tree-follow", None).unwrap();
        assert!(store.message.contains("tree buffer-follow: on"));
        // Now opening src/lib.rs moves the cursor to its row.
        store.open_path("src/lib.rs");
        assert_eq!(store.tree_selected(), lib_row);
        // Toggle back off: the cursor no longer follows.
        store.dispatch("toggle-tree-follow", None).unwrap();
        assert!(store.message.contains("tree buffer-follow: off"));
        store.open_path("README.md");
        assert_eq!(store.tree_selected(), lib_row);
    }

    #[test]
    fn any_tracked_skips_untracked_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git_repo_init(root, "Test", "test@example.com", true);
        std::fs::write(root.join("tracked.txt"), "x\n").unwrap();
        git_cli(root, &["add", "tracked.txt"], "Test", "test@example.com");
        git_cli(root, &["commit", "-q", "-m", "init"], "Test", "test@example.com");
        // An untracked file.
        std::fs::write(root.join("untracked.txt"), "y\n").unwrap();
        // Use the git repo directly.
        let repo = redline_git::GitRepo::discover(root).unwrap();
        let tracked = vec![root.join("tracked.txt")];
        let untracked = vec![root.join("untracked.txt")];
        assert!(repo.any_tracked(&tracked, root), "tracked file must be detected");
        assert!(!repo.any_tracked(&untracked, root), "untracked file must NOT be detected");
    }

    #[test]
    fn unstage_hunk_fully_staged_add_removes_index_entry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git_repo_init(root, "Test", "test@example.com", true);
        std::fs::write(root.join("initial.txt"), "init\n").unwrap();
        git_cli(root, &["add", "initial.txt"], "Test", "test@example.com");
        git_cli(root, &["commit", "-q", "-m", "init"], "Test", "test@example.com");
        // Create a new file and stage it (fully-staged addition).
        std::fs::write(root.join("newfile.txt"), "line1\nline2\nline3\n").unwrap();
        git_cli(root, &["add", "newfile.txt"], "Test", "test@example.com");
        // The hunk new_start for a new file is 1 (first line).
        let repo = redline_git::GitRepo::discover(root).unwrap();
        repo.unstage_hunk("newfile.txt", 1).unwrap();
        // After unstaging a fully-staged addition, the index entry is
        // removed (the file is back to untracked, not an empty blob).
        let status = repo.status().unwrap();
        let newfile_entry = status.files.iter().find(|f| f.path == "newfile.txt");
        assert!(newfile_entry.is_some(), "newfile.txt must appear in status after unstage");
        // It must be untracked (not staged).
        let entry = newfile_entry.unwrap();
        assert!(entry.untracked, "fully-staged addition after unstage must be untracked");
        assert_eq!(entry.staged, redline_git::status::StatusKind::None,
            "staged kind must be None after unstage");
    }

    /// A store rooted in a fresh single-commit git repo (a.txt committed),
    /// with the project set so the git ops resolve the repo.
    /// Windowing (issue 002-02): a status buffer taller than the viewport
    /// keeps the cursor row inside the visible window while pressing `n`, and
    /// the window's top advances (scrolls) as the cursor moves past it. This
    /// is the store-level regression guard for the cursor-following window
    /// (the pyte PTY check drives the same path end-to-end).
    #[test]
    fn magit_status_window_keeps_cursor_visible() {
        let dir = tempfile::tempdir().unwrap();
        git_repo_init(dir.path(), "Test", "test@example.com", true);
        for i in 0..20 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), "x\n").unwrap();
        }
        git_cli(dir.path(), &["add", "-A"], "Test", "test@example.com");
        git_cli(dir.path(), &["commit", "-q", "-m", "init"], "Test", "test@example.com");
        for i in 0..20 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), format!("x\nchanged {i}\n")).unwrap();
        }
        git_cli(dir.path(), &["add", "-A"], "Test", "test@example.com");

        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.project = Some(redline_model::project::Project::new(dir.path().to_path_buf()));
        s.set_viewport_lines(8); // small viewport -> magit_window() == 6
        s.open_magit_status();

        let total = s.magit_rows().len();
        assert!(total > 6, "status buffer must overflow the window: {total} rows");

        // The cursor (first changed file) is inside the window, and the window
        // is bounded by magit_window().
        let (win, top, tot) = s.magit_view_info();
        assert_eq!(tot, total);
        assert!(win.len() <= 6, "window must be bounded: {} rows", win.len());
        let cursor = s.magit_rows().iter().position(|r| r.selected).unwrap();
        assert!((top..top + win.len()).contains(&cursor),
            "cursor row {cursor} must be in window [{top},{}): top={top}", top + win.len());

        // Press `n` until the cursor moves past the first window; the window's
        // top must advance and the cursor must stay visible.
        let first_top = s.magit_view_info().1;
        for _ in 0..12 {
            s.key_event(key("n"));
        }
        let (win2, top2, _) = s.magit_view_info();
        assert!(top2 > first_top, "window must scroll down: {first_top} -> {top2}");
        let cursor2 = s.magit_rows().iter().position(|r| r.selected).unwrap();
        assert!((top2..top2 + win2.len()).contains(&cursor2),
            "cursor row {cursor2} must stay in window [{top2},{}): top={top2}", top2 + win2.len());
    }

    // ── issue 003-02: shared windowing (commit-diff / blame / log / notes) ──

    #[test]
    fn discard_unstaged_file_confirmation_flow() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = git_store(dir.path());
        std::fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
        s.open_magit_status();
        assert_eq!(s.top_view(), ViewId::MagitStatus);
        // k arms the confirmation.
        s.key_event(key("k"));
        assert!(s.discard_armed());
        assert_eq!(s.message, "discard a.txt? y/n");
        // n cancels — nothing changes.
        s.key_event(key("n"));
        assert!(!s.discard_armed());
        assert_eq!(s.message, "discard cancelled");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "changed\n"
        );
        // C-g also cancels — nothing changes.
        s.key_event(key("k"));
        assert!(s.discard_armed());
        s.key_event(key("C-g"));
        assert!(!s.discard_armed());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "changed\n"
        );
        // y confirms — the file is restored to HEAD.
        s.key_event(key("k"));
        assert!(s.discard_armed());
        s.key_event(key("y"));
        assert!(!s.discard_armed());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "keep\n"
        );
        assert!(
            s.message.contains("discarded a.txt"),
            "got `{}`",
            s.message
        );
    }

    /// Magit 4.7.1 boundary messages (`lisp/magit-section.el:805-831`):
    /// `n` at the last visible section and `p` at the first stay put and
    /// echo `No next section` / `No previous section` through the
    /// minibuffer, while mid-list moves stay silent.
    #[test]
    fn magit_cursor_boundaries_echo_magit_messages() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = git_store(dir.path());
        std::fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
        s.open_magit_status();
        // Visible sections: header, staged (empty), unstaged, unstaged:a.txt,
        // untracked (empty). The cursor starts on the first addressable
        // section: unstaged:a.txt.
        assert_eq!(
            s.status_tree
                .as_ref()
                .unwrap()
                .cursor_section()
                .unwrap()
                .id,
            "unstaged:a.txt"
        );
        // Mid-list `n`: moves to the last visible section, no message.
        s.message.clear();
        s.magit_cursor_down();
        assert_eq!(
            s.status_tree
                .as_ref()
                .unwrap()
                .cursor_section()
                .unwrap()
                .id,
            "untracked"
        );
        assert!(
            s.message != "No next section",
            "mid-list move must not report a boundary: `{}`",
            s.message
        );
        // `n` at the last visible section: stays put, `No next section`.
        s.message.clear();
        s.magit_cursor_down();
        assert_eq!(
            s.status_tree
                .as_ref()
                .unwrap()
                .cursor_section()
                .unwrap()
                .id,
            "untracked",
            "no wrap: the cursor must not jump to the first section"
        );
        assert_eq!(s.message, "No next section");
        // `p` back to the first visible section (header), step by step.
        s.magit_cursor_up();
        s.magit_cursor_up();
        s.magit_cursor_up();
        s.magit_cursor_up();
        assert_eq!(
            s.status_tree
                .as_ref()
                .unwrap()
                .cursor_section()
                .unwrap()
                .id,
            "header"
        );
        // `p` at the first visible section: stays put, `No previous section`.
        s.message.clear();
        s.magit_cursor_up();
        assert_eq!(
            s.status_tree
                .as_ref()
                .unwrap()
                .cursor_section()
                .unwrap()
                .id,
            "header"
        );
        assert_eq!(s.message, "No previous section");
    }

    #[test]
    fn discard_acts_on_cursor_row() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git_repo_init(root, "Test", "test@example.com", true);
        std::fs::write(root.join("a.txt"), "A\n").unwrap();
        std::fs::write(root.join("b.txt"), "B\n").unwrap();
        git_cli(root, &["add", "a.txt", "b.txt"], "Test", "test@example.com");
        git_cli(root, &["commit", "-q", "-m", "init"], "Test", "test@example.com");
        std::fs::write(root.join("a.txt"), "A2\n").unwrap();
        std::fs::write(root.join("b.txt"), "B2\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(root, base.path().to_path_buf());
        s.project = Some(redline_model::project::Project::new(root.to_path_buf()));
        s.open_magit_status();
        // Cursor starts on a.txt (first unstaged file). Move to b.txt.
        s.key_event(key("n"));
        s.key_event(key("k"));
        assert!(s.discard_armed());
        assert!(s.message.contains("b.txt"), "got `{}`", s.message);
        s.key_event(key("y"));
        // b.txt restored; a.txt untouched.
        assert_eq!(std::fs::read_to_string(root.join("b.txt")).unwrap(), "B\n");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "A2\n");
    }

    #[test]
    fn discard_menu_leaf_arms_confirmation() {
        // k pressed FROM the open menu closes the menu and arms the gate.
        let dir = tempfile::tempdir().unwrap();
        let mut s = git_store(dir.path());
        std::fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
        s.open_magit_status();
        s.key_event(key("h")); // open the menu (magit dispatch)
        assert!(s.menu_open());
        s.key_event(key("k")); // k is a menu leaf → magit-discard
        assert!(!s.menu_open(), "menu must close on the leaf");
        assert!(s.discard_armed(), "confirmation must be armed");
        s.key_event(key("n"));
        assert!(!s.discard_armed());
    }

    #[test]
    fn branch_and_stash_candidates_are_name_first() {
        // picker-density: the branch row carries the branch name left and
        // the HEAD marker right; the stash row carries the stash ref left
        // and the subject right. `display` (the match target) keeps its
        // pre-conversion shape.
        let dir = tempfile::tempdir().unwrap();
        let mut s = git_store(dir.path());
        let root = dir.path();
        git_cli(root, &["branch", "feature"], "Test", "test@example.com");
        std::fs::write(root.join("a.txt"), "a\nb\n").unwrap();
        git_cli(root, &["stash", "push", "-m", "wip subject"], "Test", "test@example.com");
        s.open_magit_status(); // prime the repo handle

        let branches = s.branch_candidates();
        let main = branches.iter().find(|b| b.name == "main").unwrap();
        assert_eq!(main.display, "*main", "{}", main.display);
        assert_eq!(main.label, "main", "the branch name is the label: {main:?}");
        assert_eq!(main.detail, "*", "the HEAD marker is the detail: {main:?}");
        let feature = branches.iter().find(|b| b.name == "feature").unwrap();
        assert_eq!(feature.label, "feature", "{}", feature.label);
        assert!(feature.detail.is_empty(), "non-current: no detail: {feature:?}");

        let stashes = s.stash_candidates();
        assert_eq!(stashes.len(), 1, "{stashes:?}");
        let st = &stashes[0];
        assert_eq!(st.display, "stash@{0} On main: wip subject", "{}", st.display);
        assert_eq!(st.label, "stash@{0}", "the stash ref is the label: {st:?}");
        assert_eq!(st.detail, "On main: wip subject", "the subject is the detail: {st:?}");
    }

    // ── issue 05 (finding 1): editable-buffer key order ─────────────


    // ── U-E10: magit section narrowing ──────────────────────────────────
    //
    // The deferred shape of plan 018 §2.3-2: a magit-native SECTION filter
    // over the Section tree (filter children, keep the section structure —
    // a heading survives iff a surviving descendant exists). NOT a
    // row-level narrowing of the shared core: the diff payload
    // (context/add/delete lines) is one document, so a score-reorder
    // would corrupt it and even a filter must keep context lines glued
    // to their hunk headers.

    /// The U-E10 narrow fixture: alpha (STAGED change) + omega (UNSTAGED
    /// change) + beta (clean), all committed first. The discriminating
    /// query is `"oa"`: a SUBSEQUENCE of exactly one tree text —
    /// `  M omega.txt (+1 -0)` (o@2…a@6) — and a contiguous substring of
    /// NONE (the `M`/`#` letters and the `(+1 -0)` counts keep every other
    /// heading letter-free of `o…a` in order; the diff lines — `hello`,
    /// `world`, `staged line`, `work line` — hold no `o` followed by an
    /// `a`). A hand-rolled contains-filter recompute therefore keeps ZERO
    /// sections where the shared core keeps the omega chain — the
    /// discriminating case.
    fn magit_narrow_repo(dir: &std::path::Path) {
        git_repo_init(dir, "Test", "test@example.com", true);
        for name in ["alpha.txt", "beta.txt", "omega.txt"] {
            std::fs::write(dir.join(name), "hello\nworld\n").unwrap();
        }
        git_cli(dir, &["add", "-A"], "Test", "test@example.com");
        git_cli(dir, &["commit", "-q", "-m", "init"], "Test", "test@example.com");
        std::fs::write(dir.join("alpha.txt"), "hello\nworld\nstaged line\n").unwrap();
        git_cli(dir, &["add", "alpha.txt"], "Test", "test@example.com");
        std::fs::write(dir.join("omega.txt"), "hello\nworld\nwork line\n").unwrap();
    }

    /// A store rooted in `dir` with the project set (the U-E10 pins'
    /// fixture root; `git_store`'s a.txt fixture is wrong for narrowing).
    fn magit_narrow_store(dir: &std::path::Path) -> AppStore {
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir, base.path().to_path_buf());
        s.project = Some(redline_model::project::Project::new(dir.to_path_buf()));
        s
    }

    /// U-E10: the magit status narrow guard — printable chars extend the
    /// section-narrow query live, Backspace pops, and the view's own
    /// own keys (every single-char `MAGIT_STATUS_BINDINGS` binding — the
    /// advertised `s`/`u`/`n`/`p`/`g`/`q` decision keys AND the context
    /// keys `l`/`b`/`c`/`y`/`z`/`h`/`k`, keymap-derived, never copied)
    /// fall through to the keymap instead of the query. On the pre-fix tree the typed chars were unbound
    /// (minibuffer echoes, no query state) and `g`/`n`/`q` could not be
    /// asserted against any query at all.
    #[test]
    fn magit_narrow_guard_extends_the_query_and_decision_keys_fall_through() {
        let dir = tempfile::tempdir().unwrap();
        magit_narrow_repo(dir.path());
        let mut s = magit_narrow_store(dir.path());
        s.open_magit_status();
        assert_eq!(s.top_view(), ViewId::MagitStatus);

        for c in "oa".chars() {
            s.key_event(Key::char(c));
        }
        assert_eq!(
            s.magit_narrow_query(),
            "oa",
            "the typed chars must reach the magit narrow query (the prompt guard)"
        );
        // `g` is the view's refresh key, not a query char: it re-derives
        // (its echo lands) and the query stays put.
        s.key_event(key("g"));
        assert_eq!(s.magit_narrow_query(), "oa", "`g` must fall through, not extend the query");
        assert_eq!(s.message, "status refreshed");
        // `n` moves the cursor (within the narrowed set), not the query.
        s.key_event(key("n"));
        assert_eq!(s.magit_narrow_query(), "oa", "`n` must fall through, not extend the query");
        // `s`/`u` are the stage/unstage keys (see the staging pin below) —
        // neither may extend the query either.
        s.key_event(key("s"));
        s.key_event(key("u"));
        assert_eq!(s.magit_narrow_query(), "oa", "`s`/`u` must fall through, not extend the query");
        // `q` closes the view; the query state persists store-side (the
        // re-open shows the query still active).
        s.key_event(key("q"));
        assert_ne!(s.top_view(), ViewId::MagitStatus);
        assert_eq!(s.magit_narrow_query(), "oa");
        // Backspace pops the last character.
        s.open_magit_status();
        s.key_event(Key::new(KeyCode::Backspace));
        assert_eq!(s.magit_narrow_query(), "o");
    }

    /// U-E10: live narrowing keeps the SECTION STRUCTURE — a heading
    /// survives iff it or a surviving descendant matches the shared
    /// core's scoring. The group heading `Unstaged changes` never
    /// matches `"oa"` itself; it survives by its child alone. The
    /// canonical tree is untouched (a projection at view time).
    #[test]
    fn magit_narrow_live_section_projection_keeps_the_structure() {
        let dir = tempfile::tempdir().unwrap();
        magit_narrow_repo(dir.path());
        let mut s = magit_narrow_store(dir.path());
        s.open_magit_status();
        // The un-narrowed list: header + 3 groups + 2 file rows (the
        // file sections start folded, so no hunk rows).
        assert_eq!(s.magit_view_info().2, 6, "the un-narrowed list");

        for c in "oa".chars() {
            s.key_event(Key::char(c));
        }
        let (rows, _top, total) = s.magit_view_info();
        assert_eq!(total, 2, "the narrowed set is the match's structural path: {rows:?}");
        assert_eq!(
            rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
            vec!["Unstaged changes", "  M omega.txt (+1 -0)"],
            "the group survives iff its child does; the staged side and the clean file are filtered out"
        );
        // The canonical tree is never narrowed away (view-time projection).
        assert_eq!(s.magit_rows().len(), 6, "the canonical rows are untouched");
        // The selection clamps into the narrowed set: the default cursor
        // (staged:alpha.txt) is filtered out, so it moves onto the
        // surviving file — the cursor's identity is the section it is on,
        // never a lost row index.
        assert_eq!(
            s.status_tree.as_ref().unwrap().cursor_section().unwrap().id,
            "unstaged:omega.txt",
            "the cursor clamps onto the surviving section, not lost"
        );
        // Shared-core cross-check over the tree's OWN texts (every
        // section's heading and every hunk body line — never a copy of
        // the projection's strings): the core scores exactly the omega
        // file's heading (no other tree text carries `o…a`), and the
        // projection is that core-matched section plus its surviving
        // ancestor (the group heading, which never matched itself).
        fn all_texts(sec: &redline_model::sections::Section, out: &mut Vec<(String, String)>) {
            out.push((sec.id.clone(), sec.heading.clone()));
            for l in &sec.body {
                out.push((sec.id.clone(), l.content.clone()));
            }
            for c in &sec.children {
                all_texts(c, out);
            }
        }
        let tree = s.status_tree.as_ref().unwrap();
        let mut texts = Vec::new();
        for top in tree.sections() {
            all_texts(top, &mut texts);
        }
        let core_matched: std::collections::BTreeSet<String> = texts
            .iter()
            .filter(|(_id, text)| {
                !crate::app::store::narrowing::narrow("oa", &[text.as_str()], &mut s.matcher).is_empty()
            })
            .map(|(id, _)| id.clone())
            .collect();
        assert_eq!(
            core_matched,
            std::collections::BTreeSet::from(["unstaged:omega.txt".to_string()]),
            "the fixture must discriminate: the shared core scores exactly the omega heading (`oa` is a subsequence, not a substring — a contains-filter would score nothing)"
        );
        let projected = tree
            .narrowed_surviving_ids(&mut |t| !crate::app::store::narrowing::narrow("oa", &[t], &mut s.matcher).is_empty());
        assert_eq!(
            projected,
            vec!["unstaged".to_string(), "unstaged:omega.txt".to_string()],
            "the section projection = the core's matched section + its surviving ancestor — structure, not rank"
        );
    }

    /// U-E10: a FOLDED section whose descendant matches still reveals the
    /// match — the projection renders the surviving child (and the
    /// hunk's FULL body: the diff payload is one document) even under a
    /// canonical fold, and the fold state itself is untouched (clearing
    /// the query restores the previous folds byte-for-byte).
    #[test]
    fn magit_narrow_folded_file_reveals_the_matching_hunk() {
        let dir = tempfile::tempdir().unwrap();
        git_repo_init(dir.path(), "Test", "test@example.com", true);
        std::fs::write(dir.path().join("zeta.txt"), "one\ntwo\nthree\n").unwrap();
        git_cli(dir.path(), &["add", "zeta.txt"], "Test", "test@example.com");
        git_cli(dir.path(), &["commit", "-q", "-m", "init"], "Test", "test@example.com");
        std::fs::write(dir.path().join("zeta.txt"), "one\ntwo\nthree\nwemo\n").unwrap();
        let mut s = magit_narrow_store(dir.path());
        s.open_magit_status();
        // The zeta file section starts folded (the magit default) and the
        // cursor is on it.
        assert!(
            s.status_tree.as_ref().unwrap().cursor_section().unwrap().folded,
            "the file section starts folded"
        );
        // "weld" matches ONLY the hunk's added body line (no heading,
        // no other line carries that text).
        for c in "wemo".chars() {
            s.key_event(Key::char(c));
        }
        let (rows, _top, total) = s.magit_view_info();
        assert_eq!(total, 7, "the revealed match's full structure: {rows:?}");
        assert_eq!(
            rows.iter().map(|r| format!("{:?} {}", r.role, r.text)).collect::<Vec<_>>(),
            vec![
                "Group Unstaged changes",
                "File   M zeta.txt (+1 -0)",
                "HunkHeader     @@ -1,3 +1,4 @@",
                "DiffContext        one",
                "DiffContext        two",
                "DiffContext        three",
                "DiffAdd       +wemo",
            ],
            "the surviving hunk renders its FULL body (context/add/delete lines are one document): {rows:?}"
        );
        assert!(rows[1].selected, "the cursor row keeps its selection bar: {rows:?}");
        // The canonical fold state is untouched by the projection.
        assert!(
            s.status_tree.as_ref().unwrap().cursor_section().unwrap().folded,
            "the projection must not mutate the canonical fold state"
        );
        assert_eq!(s.magit_rows().len(), 5, "the un-narrowed render still hides the hunk");
    }

    /// U-E10: C-g CLEARS the query (the full list re-derives) and the
    /// selection is clamped, not lost — the cursor's identity is the
    /// section it is on (a stable id), and nothing-survives leaves the
    /// cursor alone (there is no row to select).
    #[test]
    fn magit_narrow_c_g_clears_the_query_and_the_cursor_survives() {
        let dir = tempfile::tempdir().unwrap();
        magit_narrow_repo(dir.path());
        let mut s = magit_narrow_store(dir.path());
        s.open_magit_status();
        for c in "oa".chars() {
            s.key_event(Key::char(c));
        }
        // The cursor clamped from the filtered-out staged:alpha.txt onto
        // the surviving omega section.
        assert_eq!(
            s.status_tree.as_ref().unwrap().cursor_section().unwrap().id,
            "unstaged:omega.txt"
        );
        // C-g clears (NOT closes): the full list re-derives, the cursor's
        // section still exists, so it simply stays on it — clamped, not
        // lost.
        s.key_event(key("C-g"));
        assert_eq!(s.magit_narrow_query(), "");
        assert_eq!(s.message, "filter cleared");
        assert_eq!(s.magit_view_info().2, 6, "the full list re-derived");
        assert_eq!(
            s.status_tree.as_ref().unwrap().cursor_section().unwrap().id,
            "unstaged:omega.txt",
            "the selection survived the clear"
        );
        assert_eq!(s.top_view(), ViewId::MagitStatus, "C-g must not close the view");

        // The nothing-survives case: the discriminating query `j v f` —
        // a subsequence of NO tree text, MEASURED against this exact
        // fixture (nucleo `Pattern::parse(.., Ignore, Smart)` over the
        // tree's headings + diff lines): `j`, `v`, and `f` each match
        // ZERO sections on their own, and no heading/diff line holds a
        // `j` that later trails a `v` that later trails an `f` (the
        // full text set is `## main`, the `Staged`/`Unstaged`/`Untracked`
        // groups, the `M <file>` + `@@`/`hello`/`world`/`staged line`/
        // `work line` lines). (The query carries no bound MagitStatus key:
        // j, v, and f are unbound.)
        s.key_event(key("C-g")); // empty-query C-g: still the clear no-op path
        assert_eq!(s.message, "filter cleared", "C-g on an empty query is the clear path, not a view close");
        for c in "j v f".chars() {
            s.key_event(Key::char(c));
        }
        assert_eq!(s.magit_narrow_query(), "j v f");
        assert_eq!(s.magit_view_info().2, 0, "nothing survives `j v f`");
        assert_eq!(
            s.status_tree.as_ref().unwrap().cursor_section().unwrap().id,
            "unstaged:omega.txt",
            "the empty survivor set leaves the cursor where it was (its section still exists canonically)"
        );
        s.key_event(key("C-g"));
        assert_eq!(s.magit_view_info().2, 6);
        assert_eq!(
            s.status_tree.as_ref().unwrap().cursor_section().unwrap().id,
            "unstaged:omega.txt",
            "the full list re-derived with the cursor intact"
        );
    }

    /// U-E10: `n`/`p` move WITHIN the narrowed set (a filtered-out section
    /// is never a landing spot) and keep magit's no-wrap boundary echoes.
    #[test]
    fn magit_narrow_n_p_move_within_the_narrowed_set() {
        let dir = tempfile::tempdir().unwrap();
        magit_narrow_repo(dir.path());
        let mut s = magit_narrow_store(dir.path());
        s.open_magit_status();
        for c in "oa".chars() {
            s.key_event(Key::char(c));
        }
        // Surviving sections: the unstaged group + its omega file.
        let cursor_id = |s: &AppStore| s.status_tree.as_ref().unwrap().cursor_section().unwrap().id.clone();
        assert_eq!(cursor_id(&s), "unstaged:omega.txt");
        // `n` at the last surviving section: stays put, `No next section`
        // (it must NOT wrap to the filtered-out alpha or any full-list
        // section).
        s.key_event(key("n"));
        assert_eq!(cursor_id(&s), "unstaged:omega.txt", "no wrap past the narrowed set");
        assert_eq!(s.message, "No next section");
        // `p` steps to the surviving group heading…
        s.key_event(key("p"));
        assert_eq!(cursor_id(&s), "unstaged");
        // …and `p` at its edge: the filtered-out staged/header sections
        // are not allowed predecessors.
        s.key_event(key("p"));
        assert_eq!(cursor_id(&s), "unstaged", "no step onto a filtered-out section");
        assert_eq!(s.message, "No previous section");
    }

    /// U-E10 class-bug pin (the pending-sequence discipline): a chord that
    /// arms a prefix must NOT be stranded by the narrow-prompt guard. With
    /// the status view on top, `C-x` reaches the engine (arms the `C-x`
    /// prefix — it carries no char value, so the guard cannot swallow it),
    /// and the follow-up `2` — which COMPLETES the bound `C-x 2` sequence
    /// — must reach the engine too, echoing the unbound-key dead end. On
    /// the pre-fix guard the `2` was consumed into the narrow query (the
    /// guard tested the key in isolation, so `resolve([2])` was `None`) and
    /// the `C-x 2` sequence was stranded. The guard now composes the armed
    /// prefix with the key: a non-empty `self.pending` means the key MUST
    /// reach the engine (the `notes_edit_key_event` / `dispatch_key`
    /// discipline), so the query stays empty and the echo lands.
    #[test]
    fn magit_narrow_guard_reaches_the_engine_for_a_pending_chord_c_x_2() {
        let dir = tempfile::tempdir().unwrap();
        magit_narrow_repo(dir.path());
        let mut s = magit_narrow_store(dir.path());
        s.open_magit_status();
        s.key_event(key("C-x"));
        assert!(
            s.magit_narrow_query().is_empty(),
            "a bare C-x must not feed the narrow query"
        );
        s.key_event(key("2"));
        assert_eq!(s.magit_narrow_query(), "", "the `2` must NOT be swallowed into the narrow query");
        assert!(
            s.message.contains("unbound key: 2"),
            "the `2` reaches the engine and dead-ends to the unbound echo: {}",
            s.message
        );
        assert_eq!(s.top_view(), ViewId::MagitStatus, "no view change on the echo");
    }

    /// U-E10: the "old outcome holds" leg — staging while narrowed hits
    /// the RIGHT file. The cursor is on the surviving omega section;
    /// `s` (falling through to `magit-stage`) stages omega.txt — not
    /// alpha — and the refresh re-derives the projection under the same
    /// query onto the now-staged omega section.
    #[test]
    fn magit_narrow_staging_hits_the_right_file() {
        let dir = tempfile::tempdir().unwrap();
        magit_narrow_repo(dir.path());
        let mut s = magit_narrow_store(dir.path());
        s.open_magit_status();
        for c in "oa".chars() {
            s.key_event(Key::char(c));
        }
        assert_eq!(
            s.status_tree.as_ref().unwrap().cursor_section().unwrap().id,
            "unstaged:omega.txt"
        );
        s.key_event(key("s"));
        let d = s.dirty_counts().unwrap();
        assert_eq!(
            (d.staged, d.unstaged),
            (2, 0),
            "omega must be staged (alpha was already) and unstaged must be empty: {d:?}"
        );
        // The refresh re-derived the projection under the SAME query: the
        // surviving structure moved to the staged side, alpha (no match)
        // stays filtered out.
        let (rows, _top, total) = s.magit_view_info();
        assert_eq!(total, 2, "the re-derived narrowed set: {rows:?}");
        assert_eq!(
            rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
            vec!["Staged changes", "  M omega.txt (+1 -0)"],
            "the surviving structure follows the staged omega"
        );
        // The cursor re-clamped onto the surviving (now staged) omega
        // section: its old id (unstaged:omega.txt) no longer exists.
        assert_eq!(
            s.status_tree.as_ref().unwrap().cursor_section().unwrap().id,
            "staged:omega.txt",
            "the cursor must land on the re-derived section under point"
        );
    }

    /// U-E10: RET lands on the selected row's REAL target while narrowed
    /// (the visit-file convention: open the cursor's file, close the
    /// status view; the query state persists store-side).
    #[test]
    fn magit_narrow_ret_lands_on_the_selected_real_target() {
        let dir = tempfile::tempdir().unwrap();
        magit_narrow_repo(dir.path());
        let mut s = magit_narrow_store(dir.path());
        s.open_magit_status();
        for c in "oa".chars() {
            s.key_event(Key::char(c));
        }
        assert_eq!(
            s.status_tree.as_ref().unwrap().cursor_section().unwrap().id,
            "unstaged:omega.txt",
            "RET must land on the NARROWED row's target, not the full list's cursor"
        );
        s.key_event(key("RET"));
        assert_eq!(s.top_view(), ViewId::Buffer, "RET must open the file buffer");
        assert_eq!(s.view_name_display(), "omega.txt", "the visited file must be the narrowed row's file");
        assert_eq!(s.magit_narrow_query(), "oa", "the query state persists across the visit");
    }

    /// U-E10: `g`/refresh re-derives under a still-active query — the
    /// fresh tree is re-projected (a new unstaged file with no match
    /// stays filtered out; the surviving one stays in), the query is
    /// neither cleared nor re-typed, and the cursor's section id
    /// survives the re-derivation.
    #[test]
    fn magit_narrow_g_refresh_rederives_under_the_same_query() {
        let dir = tempfile::tempdir().unwrap();
        magit_narrow_repo(dir.path());
        let mut s = magit_narrow_store(dir.path());
        s.open_magit_status();
        for c in "oa".chars() {
            s.key_event(Key::char(c));
        }
        assert_eq!(s.magit_view_info().2, 2, "narrowed before the refresh");
        // A fresh, non-matching unstaged change (beta).
        std::fs::write(dir.path().join("beta.txt"), "hello\nbeta work\n").unwrap();
        s.key_event(key("g"));
        assert_eq!(s.message, "status refreshed");
        assert_eq!(s.magit_narrow_query(), "oa", "`g` must re-derive, not clear the query");
        let (rows, _top, total) = s.magit_view_info();
        assert_eq!(total, 2, "beta (no match) stays filtered out; omega still survives: {rows:?}");
        assert_eq!(
            rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
            vec!["Unstaged changes", "  M omega.txt (+1 -0)"],
        );
        assert_eq!(
            s.status_tree.as_ref().unwrap().cursor_section().unwrap().id,
            "unstaged:omega.txt",
            "the cursor's section id survives the re-derivation"
        );
    }

    /// U-E10 (the `7f0090a` keys-first rule): the prompt row is ONE NoWrap
    /// row directly under the title, and EVERY decision group survives the
    /// 80-col clip AND leads the query — a right-edge clip eats the query
    /// tail, never the keys.
    #[test]
    fn magit_narrow_prompt_row_keys_lead_at_80_with_long_query() {
        use crate::ui::root::render_at_width;

        let dir = tempfile::tempdir().unwrap();
        magit_narrow_repo(dir.path());
        let mut s = magit_narrow_store(dir.path());
        s.open_magit_status();
        // "oa" + 200 x's (`x` is not a decision key, so the whole run
        // reaches the query).
        for c in "oa".chars() {
            s.key_event(Key::char(c));
        }
        for _ in 0..200 {
            s.key_event(Key::char('x'));
        }
        let frame = render_at_width(s, 80);
        let lines: Vec<&str> = frame.lines().collect();
        let title_idx = lines
            .iter()
            .position(|l| l.contains("*magit-status*"))
            .unwrap_or_else(|| panic!("title row missing\n{frame}"));
        let prompt_idx = lines
            .iter()
            .position(|l| l.contains("s/u stage"))
            .unwrap_or_else(|| panic!("prompt row missing\n{frame}"));
        assert_eq!(
            prompt_idx,
            title_idx + 1,
            "the prompt row sits directly under the title\n{frame}"
        );
        let prompt = lines[prompt_idx];
        // One row, NoWrap: at a 202-char query the row is filled to the
        // edge with the query tail (a wrap would leave the edge empty).
        assert!(prompt.trim_end().ends_with('x'), "the prompt row is one NoWrap row filled by the query: {prompt:?}");
        let query_at = prompt.find('x').unwrap();
        for group in ["s/u stage", "n/p", "g refresh", "RET open", "C-g clear", "q close"] {
            assert!(
                prompt.contains(group),
                "decision group {group:?} must survive the 80-col clip: {prompt:?}"
            );
            assert!(
                prompt.find(group).unwrap() < query_at,
                "decision group {group:?} must LEAD the query: {prompt:?}"
            );
        }
        // The query tail is the clipped part.
        assert!(
            !prompt.contains(&"x".repeat(25)),
            "the 202-char query cannot fit: its tail is the clipped part: {prompt:?}"
        );
    }

    /// U-E10: at rest (no query) the prompt row shows the decision keys +
    /// the `type to narrow` placeholder — the shape pin of the at-rest
    /// frame (the live-frame assertions live in the drive).
    #[test]
    fn magit_narrow_prompt_row_at_rest_shows_keys_and_placeholder() {
        use crate::ui::root::render_at_width;

        let dir = tempfile::tempdir().unwrap();
        magit_narrow_repo(dir.path());
        let mut s = magit_narrow_store(dir.path());
        s.open_magit_status();
        let frame = render_at_width(s, 80);
        let lines: Vec<&str> = frame.lines().collect();
        let title_idx = lines
            .iter()
            .position(|l| l.contains("*magit-status*"))
            .unwrap_or_else(|| panic!("title row missing\n{frame}"));
        let prompt = lines.get(title_idx + 1).copied().unwrap_or("");
        assert!(prompt.contains("type to narrow"), "the empty-query placeholder: {prompt:?}");
        for group in ["s/u stage", "n/p", "g refresh", "RET open", "C-g clear", "q close"] {
            assert!(prompt.contains(group), "decision group {group:?} missing at rest: {prompt:?}");
        }
        // The un-narrowed list renders below the prompt row (both file
        // rows of the fixture).
        assert!(
            lines.iter().any(|l| l.contains("  M alpha.txt")),
            "the alpha file row renders: {frame}"
        );
        assert!(
            lines.iter().any(|l| l.contains("  M omega.txt")),
            "the omega file row renders: {frame}"
        );
    }

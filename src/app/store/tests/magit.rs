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

        // Terminal row 0 is the tree title: a no-op (selection + point).
        s.tree_click_row(0);
        assert_eq!(s.tree_selected(), 0);
        assert_eq!((s.point_line(), s.point_col()), (3, 2), "title-row click must not move the code point");

        // Terminal row 2 → visible row 1 (window top is `selected-5`, 0
        // here): the selection moves, the point does not.
        s.tree_click_row(2);
        assert_eq!(s.tree_selected(), 1, "terminal row 2 → tree row 1");
        assert_eq!((s.point_line(), s.point_col()), (3, 2), "tree-row click must not move the code point");

        // The help row (row TREE_VISIBLE_ROWS+1 = 9) is a no-op.
        s.tree_click_row(9);
        assert_eq!(s.tree_selected(), 1);
        // A far row (past the window) is a no-op.
        s.tree_click_row(99);
        assert_eq!(s.tree_selected(), 1);

        // With the tree hidden every tree click is a no-op.
        s.toggle_tree();
        assert!(!s.tree_visible());
        s.tree_click_row(2);
        assert_eq!(s.tree_selected(), 1, "hidden tree: click is a no-op");
    }

    #[test]
    fn tree_click_row_selects_within_a_scrolled_window() {
        // Window top is `selected - 5`: after moving the selection to row 7
        // (of 8 rows) the visible window starts at row 2, so terminal row
        // 1 (the first visible row) selects tree row 2.
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
        s.tree_click_row(1);
        assert_eq!(s.tree_selected(), 2, "terminal row 1 → window row 0 → tree row 2");
        assert_eq!((s.point_line(), s.point_col()), (0, 0), "selection only; point untouched");
    }

    #[test]
    fn apply_project_change_tracked_path_refreshes_magit_counts() {
        // F4 carried leg: the classifier (any_tracked) is tested separately;
        // this drives the magit-REFRESH leg inside apply_project_change —
        // a tracked-path change must update the dirty counts.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fn git_cli(d: &std::path::Path, args: &[&str]) {
            let out = std::process::Command::new("git")
                .arg("-C").arg(d)
                .args(args)
                .env("GIT_AUTHOR_NAME", "T")
                .env("GIT_AUTHOR_EMAIL", "t@e.com")
                .env("GIT_COMMITTER_NAME", "T")
                .env("GIT_COMMITTER_EMAIL", "t@e.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .expect("run git");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        git_cli(root, &["init", "-q", "-b", "main"]);
        git_cli(root, &["config", "user.name", "T"]);
        git_cli(root, &["config", "user.email", "t@e.com"]);
        std::fs::write(root.join("tracked.rs"), "fn a() {}\n").unwrap();
        git_cli(root, &["add", "tracked.rs"]);
        git_cli(root, &["commit", "-q", "-m", "init"]);

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
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
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
                .output()
                .expect("run git");
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
        git_cli(root, &["init", "-q", "-b", "main"]);
        git_cli(root, &["config", "user.name", "Test"]);
        git_cli(root, &["config", "user.email", "test@example.com"]);
        git_cli(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("tracked.txt"), "x\n").unwrap();
        git_cli(root, &["add", "tracked.txt"]);
        git_cli(root, &["commit", "-q", "-m", "init"]);
        // An untracked file.
        std::fs::write(root.join("untracked.txt"), "y\n").unwrap();
        // Use the git repo directly.
        let repo = crate::git::GitRepo::discover(root).unwrap();
        let tracked = vec![root.join("tracked.txt")];
        let untracked = vec![root.join("untracked.txt")];
        assert!(repo.any_tracked(&tracked, root), "tracked file must be detected");
        assert!(!repo.any_tracked(&untracked, root), "untracked file must NOT be detected");
    }

    #[test]
    fn unstage_hunk_fully_staged_add_removes_index_entry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
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
                .output()
                .expect("run git");
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
        git_cli(root, &["init", "-q", "-b", "main"]);
        git_cli(root, &["config", "user.name", "Test"]);
        git_cli(root, &["config", "user.email", "test@example.com"]);
        git_cli(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("initial.txt"), "init\n").unwrap();
        git_cli(root, &["add", "initial.txt"]);
        git_cli(root, &["commit", "-q", "-m", "init"]);
        // Create a new file and stage it (fully-staged addition).
        std::fs::write(root.join("newfile.txt"), "line1\nline2\nline3\n").unwrap();
        git_cli(root, &["add", "newfile.txt"]);
        // The hunk new_start for a new file is 1 (first line).
        let repo = crate::git::GitRepo::discover(root).unwrap();
        repo.unstage_hunk("newfile.txt", 1).unwrap();
        // After unstaging a fully-staged addition, the index entry is
        // removed (the file is back to untracked, not an empty blob).
        let status = repo.status().unwrap();
        let newfile_entry = status.files.iter().find(|f| f.path == "newfile.txt");
        assert!(newfile_entry.is_some(), "newfile.txt must appear in status after unstage");
        // It must be untracked (not staged).
        let entry = newfile_entry.unwrap();
        assert!(entry.untracked, "fully-staged addition after unstage must be untracked");
        assert_eq!(entry.staged, crate::git::status::StatusKind::None,
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
        fn git_cli(dir: &std::path::Path, args: &[&str]) {
            let out = std::process::Command::new("git")
                .arg("-C").arg(dir).args(args)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "t@e.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "t@e.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .expect("run git");
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
        git_cli(dir.path(), &["init", "-q", "-b", "main"]);
        git_cli(dir.path(), &["config", "user.name", "Test"]);
        git_cli(dir.path(), &["config", "user.email", "t@e.com"]);
        git_cli(dir.path(), &["config", "commit.gpgsign", "false"]);
        for i in 0..20 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), "x\n").unwrap();
        }
        git_cli(dir.path(), &["add", "-A"]);
        git_cli(dir.path(), &["commit", "-q", "-m", "init"]);
        for i in 0..20 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), format!("x\nchanged {i}\n")).unwrap();
        }
        git_cli(dir.path(), &["add", "-A"]);

        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.project = Some(crate::model::project::Project::new(dir.path().to_path_buf()));
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
        fn git_cli(dir: &std::path::Path, args: &[&str]) {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .expect("run git");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        git_cli(root, &["init", "-q", "-b", "main"]);
        git_cli(root, &["config", "user.name", "Test"]);
        git_cli(root, &["config", "user.email", "test@example.com"]);
        git_cli(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("a.txt"), "A\n").unwrap();
        std::fs::write(root.join("b.txt"), "B\n").unwrap();
        git_cli(root, &["add", "a.txt", "b.txt"]);
        git_cli(root, &["commit", "-q", "-m", "init"]);
        std::fs::write(root.join("a.txt"), "A2\n").unwrap();
        std::fs::write(root.join("b.txt"), "B2\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(root, base.path().to_path_buf());
        s.project = Some(crate::model::project::Project::new(root.to_path_buf()));
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

    // ── issue 05 (finding 1): editable-buffer key order ─────────────


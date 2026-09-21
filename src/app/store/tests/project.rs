use super::*;

    #[test]
    fn detect_project_from_start_dir_and_show_name() {
        let dir = tempfile::tempdir().unwrap();
        let name = dir.path().file_name().unwrap().to_string_lossy().into_owned();
        project_with_files(dir.path());
        let store = store(dir.path());
        assert_eq!(store.project_display(), name);
        // 06a: boot is the home view (no auto-created scratch).
        assert_eq!(store.view_name_display(), "home");
    }

    #[test]
    fn switch_project_lands_in_new_projects_file_picker() {
        let base = tempfile::tempdir().unwrap();
        // Two sibling projects sharing one persistence base.
        let p1 = base.path().join("alpha");
        let p2 = base.path().join("beta");
        std::fs::create_dir_all(p1.join("src")).unwrap();
        std::fs::create_dir_all(p2.join("src")).unwrap();
        std::fs::write(p1.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(p1.join("src/a.rs"), "a\n").unwrap();
        std::fs::write(p2.join("pyproject.toml"), "[project]\n").unwrap();
        std::fs::write(p2.join("src/b.py"), "b\n").unwrap();

        let mut store = AppStore::at(&p1, base.path().to_path_buf());
        assert_eq!(store.project_display(), "alpha");
        // Register beta in the known-project registry.
        store.project_store.registry.upsert(&p2);
        let _ = store.project_store.save_registry();

        store.key_event(key("C-c"));
        store.key_event(key("p"));
        assert_eq!(store.pending_display(), "C-c p");
        store.key_event(key("p"));
        assert!(store.picker_open());
        assert_eq!(store.picker_kind(), Some(PickerKind::Projects));
        // The current project is excluded from the candidates.
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.display.as_str())
            .collect();
        assert_eq!(names, vec!["beta"], "{names:?}");
        // picker-density: the project row is name-first — the project
        // name left, the root path right.
        let c = store.picker_filtered().first().unwrap().0.clone();
        assert_eq!(c.label, "beta", "the project name is the label: {c:?}");
        assert_eq!(
            c.detail,
            p2.to_string_lossy().into_owned(),
            "the root path is the detail: {c:?}"
        );

        store.key_event(key("RET"));
        assert_eq!(store.project_display(), "beta");
        // Default switch action: land in beta's file picker.
        assert_eq!(store.picker_kind(), Some(PickerKind::FindFile));
        let files: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.display.as_str())
            .collect();
        assert!(files.contains(&"src/b.py"), "{files:?}");
        // The registry now has both, beta most recent.
        let roots: Vec<_> = store
            .project_store
            .registry
            .list()
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(roots, vec!["beta", "alpha"]);
    }

    #[test]
    fn recent_files_persist_across_restart() {
        let base = tempfile::tempdir().unwrap();
        let p = base.path().join("gamma");
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(p.join("src/one.rs"), "one\n").unwrap();
        std::fs::write(p.join("src/two.rs"), "two\n").unwrap();

        {
            let mut store = AppStore::at(&p, base.path().to_path_buf());
            store.open_path("src/one.rs");
            store.open_path("src/two.rs");
            store.open_path("src/one.rs"); // MRU again
        }
        // "Restart": a fresh store on the same base.
        let mut store = AppStore::at(&p, base.path().to_path_buf());
        let root = store.project.as_ref().unwrap().root.to_string_lossy().into_owned();
        assert_eq!(
            store.project_store.recents.list(&root),
            vec!["src/one.rs", "src/two.rs"],
            "recents must survive a restart, MRU first"
        );
        assert_eq!(
            store
                .project_store
                .registry
                .list()
                .iter()
                .map(|pr| pr.name.as_str())
                .collect::<Vec<_>>(),
            vec!["gamma"],
            "registry must survive a restart"
        );

        store.key_event(key("C-c"));
        store.key_event(key("p"));
        store.key_event(key("e"));
        assert_eq!(store.picker_kind(), Some(PickerKind::RecentFiles));
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.display.as_str())
            .collect();
        assert_eq!(names, vec!["src/one.rs", "src/two.rs"]);
    }

    #[test]
    fn re_walk_picks_up_new_files() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_find_file();
        assert_eq!(store.picker_count(), (4, 4));

        // A new file lands on disk; the cache still hides it…
        std::fs::write(dir.path().join("src/new.rs"), "new\n").unwrap();
        store.cancel();
        store.open_find_file();
        assert_eq!(store.picker_count(), (4, 4), "cache until re-walk");

        // …and the re-walk command invalidates it.
        store.dispatch("re-walk", None).unwrap();
        assert!(store.message.contains("5 files"));
        store.open_find_file();
        assert_eq!(store.picker_count(), (5, 5));
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.display.as_str())
            .collect();
        assert!(names.contains(&"src/new.rs"), "{names:?}");
    }

    #[test]
    fn open_path_missing_file_reports_error() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_path("src/nope.rs");
        assert!(store.message.contains("cannot open src/nope.rs"));
        assert_eq!(store.buffers.len(), 0); // 06a: the failed open creates no buffer
    }

    #[test]
    fn reload_anchor_preserves_line_when_it_exists() {
        // Gains lines below: the anchor line still exists → keep it.
        assert_eq!(reload_anchor(50, 100), 50);
        // Gains lines above: the anchor line still exists (by number) → keep.
        assert_eq!(reload_anchor(5, 105), 5);
        // Anchor exactly on the last line.
        assert_eq!(reload_anchor(19, 20), 19);
    }

    #[test]
    fn reload_anchor_clamps_when_anchor_vanished() {
        // File shrank past the anchor: clamp to the last line.
        assert_eq!(reload_anchor(50, 20), 19);
        assert_eq!(reload_anchor(100, 1), 0);
    }

    #[test]
    fn reload_anchor_empty_file() {
        assert_eq!(reload_anchor(50, 0), 0);
        assert_eq!(reload_anchor(0, 0), 0);
    }

    #[test]
    fn auto_reload_defaults_on_and_watcher_inactive_until_started() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        assert!(s.auto_reload, "auto_reload must default to on");
        assert!(!s.watcher_suspended());
        assert_eq!(s.watcher_count(), 0, "no watcher until started");
    }

    #[test]
    fn toggle_watcher_flips_suspend_state() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut s = store(dir.path());
        s.toggle_watcher();
        assert!(s.watcher_suspended(), "first toggle suspends");
        s.toggle_watcher();
        assert!(!s.watcher_suspended(), "second toggle resumes");
    }

    #[test]
    fn non_edited_buffer_auto_reloads_on_change() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let path = dir.path().join("src/t.rs");
        std::fs::write(&path, "fn old() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_path("src/t.rs");
        let key = s.buffers.current().unwrap().to_string();
        // A plain read-only file buffer: not locally owned.
        assert!(!s.buffers.get(&key).unwrap().is_locally_owned());

        std::fs::write(&path, "fn fresh() {}\n").unwrap();
        s.apply_project_change(&change(vec![path]));
        assert!(
            s.buffer_text().contains("fresh"),
            "auto-reload must re-read content"
        );
        assert!(
            !s.buffers.get(&key).unwrap().changed_on_disk,
            "no conflict marker for a clean buffer"
        );
    }

    #[test]
    fn conflict_flag_transitions_on_locally_modified_buffer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let path = dir.path().join("src/t.rs");
        std::fs::write(&path, "fn old() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_path("src/t.rs");
        let key = s.buffers.current().unwrap().to_string();
        // Simulate a local edit (the light-editing flag path).
        s.mark_locally_modified(&key);
        assert!(s.buffers.get(&key).unwrap().locally_modified);

        // A disk change arrives while locally owned → NO auto-reload; marker set.
        s.apply_project_change(&change(vec![path.clone()]));
        assert!(
            s.buffers.get(&key).unwrap().changed_on_disk,
            "conflict marker must be set"
        );
        assert!(s.current_buffer_changed_on_disk());
        assert!(
            s.buffer_text().contains("old"),
            "no auto-reload while conflicted"
        );

        // Change the disk, then `g` (force reload) supersedes the conflict.
        std::fs::write(&path, "fn new() {}\n").unwrap();
        s.reload_current_buffer();
        assert!(
            !s.buffers.get(&key).unwrap().changed_on_disk,
            "marker cleared after g"
        );
        assert!(
            !s.buffers.get(&key).unwrap().locally_modified,
            "local flag cleared after g"
        );
        assert!(
            s.buffer_text().contains("new"),
            "g re-read the new content"
        );
    }

    #[test]
    fn reload_preserves_scroll_anchor_when_lines_added() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let path = dir.path().join("src/big.txt");
        let base: String = (0..100)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&path, &base).unwrap();
        let base_dir = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base_dir.path().to_path_buf());
        s.set_viewport_lines(10);
        s.open_path("src/big.txt");
        s.scroll_to_bottom();
        let n_before = s.buffers.current_buffer().unwrap().line_count();
        let top_before = s.scroll_top();
        assert_eq!(top_before, n_before - 10, "scrolled to last visible page");

        // The file grows well past the anchor; the anchor line still exists.
        let grown: String = (0..200)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&path, &grown).unwrap();
        s.apply_project_change(&change(vec![path]));
        assert_eq!(s.scroll_top(), top_before, "anchor line preserved");
        assert!(
            s.buffers.current_buffer().unwrap().line_count() > top_before,
            "file grew past the anchor"
        );
    }

    #[test]
    fn reload_clamps_scroll_anchor_when_file_shrinks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let path = dir.path().join("src/big.txt");
        let base: String = (0..100)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&path, &base).unwrap();
        let base_dir = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base_dir.path().to_path_buf());
        s.set_viewport_lines(10);
        s.open_path("src/big.txt");
        s.scroll_to_bottom();
        let n_before = s.buffers.current_buffer().unwrap().line_count();
        let top_before = s.scroll_top();
        assert_eq!(top_before, n_before - 10, "scrolled to last visible page");

        // The file shrinks to 20 content lines (well below the anchor).
        let shrunken: String = (0..20)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&path, &shrunken).unwrap();
        s.apply_project_change(&change(vec![path]));
        let n_after = s.buffers.current_buffer().unwrap().line_count();
        assert!(
            n_after <= top_before,
            "anchor line must vanish: n_after={n_after}, top_before={top_before}"
        );
        assert_eq!(s.scroll_top(), n_after - 1, "clamped to last line");
    }

    // ── issue 05: jump stack tests (finding #3) ────────────────────────

    #[test]
    fn access_only_batch_does_not_set_changed_on_disk() {
        use crate::app::watcher::summarize;
        use notify_debouncer_full::DebouncedEvent;

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        let bkey = s.buffers.current().unwrap().to_string();
        let notes_path = dir.path().join(".redline-notes.md");

        // Type to make the buffer locally owned (the bug's precondition).
        s.key_event(key("a"));
        assert!(s.buffers.get(&bkey).unwrap().locally_modified);

        // Simulate the old bug path: access events for the notes file go
        // through summarize. After the fix, summarize drops them, producing
        // an empty change.
        let access_events = vec![DebouncedEvent {
            event: notify::Event {
                kind: notify::EventKind::Access(notify::event::AccessKind::Any),
                paths: vec![notes_path.clone()],
                attrs: Default::default(),
            },
            time: std::time::Instant::now(),
        }];
        let change = summarize(dir.path(), access_events);
        assert!(change.is_empty(), "summarize must drop access-only batches");

        // The empty change must not set the marker.
        s.apply_project_change(&change);
        assert!(
            !s.buffers.get(&bkey).unwrap().changed_on_disk,
            "access-only batch must not set changed_on_disk"
        );
        assert!(!s.current_buffer_changed_on_disk());
    }

    // ── plan 004 issue 03: mark / region / kill ring / yank tests ──────


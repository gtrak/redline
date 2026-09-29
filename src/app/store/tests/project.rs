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
        assert!(s.buffers.get(&key).unwrap().locally_modified());

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
            !s.buffers.get(&key).unwrap().locally_modified(),
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
        assert!(s.buffers.get(&bkey).unwrap().locally_modified());

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

    // ── issue-external-change-reload: the reopen re-stat path ───────────

    /// Force the buffer's recorded mtime to differ from the on-disk mtime so
    /// the reopen's re-stat branch ("mtime changed") fires on any filesystem
    /// granularity. A real external write has already happened (the on-disk
    /// content is genuinely new); this only pins the comparison's
    /// precondition (second-granularity filesystems can stamp two writes
    /// with the same mtime).
    fn force_mtime_divergence(s: &mut AppStore, bufk: &str, abs: &std::path::Path) {
        let disk = std::fs::metadata(abs).unwrap().modified().unwrap();
        if s.buffers.get(bufk).unwrap().mtime == disk {
            s.buffers.get_mut(bufk).unwrap().mtime = std::time::SystemTime::UNIX_EPOCH;
        }
    }

    /// Open a one-file project and a store on it (the reopen tests' setup).
    fn reopen_fixture(dir: &std::path::Path) -> std::path::PathBuf {
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
        let path = dir.join("src/t.rs");
        std::fs::write(&path, "fn old() {}\n").unwrap();
        path
    }

    /// Requirement 1: edit → external change (content + mtime) → reopen.
    /// The unsaved edit must NOT be silently discarded: the confirm arms,
    /// the text and the mode survive, and a cancel keeps the edits with a
    /// visible conflict marker.
    #[test]
    fn dirty_reopen_arms_reload_confirm_and_keeps_unsaved_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = reopen_fixture(dir.path());
        let mut s = store(dir.path());
        s.open_path("src/t.rs");
        let bufk = s.buffers.current().unwrap().to_string();
        // Into Accurate mode with unsaved edits (the data-loss setup: the
        // in-memory text differs from disk).
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        assert!(
            s.buffers.get(&bufk).unwrap().editable,
            "into edit mode (msg: {})",
            s.message
        );
        s.insert_text("zz");
        assert!(s.buffers.get(&bufk).unwrap().locally_modified());
        // The external change (a build / formatter / background agent):
        // the file's content AND mtime move on under the buffer.
        std::fs::write(&path, "fn externally() {}\n").unwrap();
        force_mtime_divergence(&mut s, &bufk, &path);
        // The reopen fires the re-stat branch.
        s.open_path("src/t.rs");
        assert!(
            s.reload_confirm_active(),
            "dirty reopen must arm the confirm, not reload silently (msg: {})",
            s.message
        );
        assert!(
            s.message.contains("discard unsaved edits"),
            "the confirm must be explicit: {:?}",
            s.message
        );
        assert!(
            s.buffer_text().contains("zz"),
            "the unsaved edit must survive the reopen: {:?}",
            s.buffer_text()
        );
        assert!(
            s.buffers.get(&bufk).unwrap().locally_modified(),
            "no decision made yet: the local flag stays set"
        );
        assert_eq!(
            s.buffer_mode_display(),
            "Accurate",
            "the mode survives the armed reopen"
        );
        // Cancel: the edits stay and the conflict is surfaced.
        s.key_event(key("C-g"));
        assert!(!s.reload_confirm_active());
        assert!(
            s.buffer_text().contains("zz"),
            "cancel keeps the unsaved edit"
        );
        assert!(
            s.current_buffer_changed_on_disk(),
            "cancel must leave the visible conflict marker"
        );
        assert_eq!(
            s.buffer_mode_display(),
            "Accurate",
            "the mode survives the cancel"
        );
        // A second reopen re-arms; `n` cancels the same way.
        s.open_path("src/t.rs");
        assert!(s.reload_confirm_active(), "second reopen re-arms the confirm");
        s.key_event(key("n"));
        assert!(!s.reload_confirm_active());
        assert!(s.buffer_text().contains("zz"), "n keeps the unsaved edit");
    }

    /// Requirement 1 (the explicit-yes half) + requirement 3: `y` on the
    /// confirm discards the unsaved edits — but ONLY because the user said
    /// so — reloads from disk in place, clears the flags, and KEEPS the
    /// buffer's mode (a reload changes content, not identity).
    #[test]
    fn reload_confirm_accept_reloads_from_disk_and_keeps_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = reopen_fixture(dir.path());
        let mut s = store(dir.path());
        s.open_path("src/t.rs");
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        s.insert_text("zz");
        std::fs::write(&path, "fn externally() {}\n").unwrap();
        force_mtime_divergence(&mut s, &bufk, &path);
        s.open_path("src/t.rs");
        assert!(s.reload_confirm_active());
        // `y`: the explicit discard + reload.
        s.key_event(key("y"));
        assert!(!s.reload_confirm_active());
        assert_eq!(
            s.buffer_text(),
            "fn externally() {}\n",
            "y re-reads the disk content"
        );
        let buf = s.buffers.get(&bufk).unwrap();
        assert!(!buf.locally_modified(), "y clears the local flag");
        assert!(!buf.changed_on_disk, "y clears the conflict marker");
        assert!(buf.editable, "the edit-mode state survives the reload");
        assert_eq!(
            s.buffer_mode_display(),
            "Accurate",
            "the mode survives the reload (a reload is content, not identity)"
        );
        // The confirm never writes the disk.
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "fn externally() {}\n"
        );
    }

    /// Requirement 2: a CLEAN buffer (no unsaved edits) still picks up the
    /// new on-disk content on reopen — the behaviour that must keep
    /// working — with no confirm and no marker.
    #[test]
    fn clean_reopen_picks_up_new_disk_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = reopen_fixture(dir.path());
        let mut s = store(dir.path());
        s.open_path("src/t.rs");
        let bufk = s.buffers.current().unwrap().to_string();
        assert!(
            !s.buffers.get(&bufk).unwrap().is_locally_owned(),
            "a plain read-only file buffer is not locally owned"
        );
        std::fs::write(&path, "fn fresh() {}\n").unwrap();
        force_mtime_divergence(&mut s, &bufk, &path);
        s.open_path("src/t.rs");
        assert!(
            !s.reload_confirm_active(),
            "a clean reopen must not arm a confirm"
        );
        assert!(
            s.buffer_text().contains("fn fresh()"),
            "the clean buffer must pick up the new content: {:?}",
            s.buffer_text()
        );
        let disk = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(
            s.buffers.get(&bufk).unwrap().mtime,
            disk,
            "the recorded mtime advances to the disk value"
        );
        assert!(!s.current_buffer_changed_on_disk());
    }

    /// Requirement 3: the mode survives the reopen on the CLEAN path too —
    /// a buffer in `Accurate` with no unsaved edits re-reads the new
    /// content in place and is still `Accurate` afterwards (the old
    /// `insert_rope` replace reset it to `Annotation`).
    #[test]
    fn clean_reopen_in_edit_mode_keeps_mode_and_updates_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = reopen_fixture(dir.path());
        let mut s = store(dir.path());
        s.open_path("src/t.rs");
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        assert_eq!(
            s.buffer_mode_display(),
            "Accurate",
            "precondition (msg: {})",
            s.message
        );
        assert!(!s.buffers.get(&bufk).unwrap().locally_modified());
        std::fs::write(&path, "fn fresh() {}\n").unwrap();
        force_mtime_divergence(&mut s, &bufk, &path);
        s.open_path("src/t.rs");
        assert!(
            !s.reload_confirm_active(),
            "no unsaved edits: no confirm"
        );
        assert!(
            s.buffer_text().contains("fn fresh()"),
            "content updated in place: {:?}",
            s.buffer_text()
        );
        assert_eq!(
            s.buffer_mode_display(),
            "Accurate",
            "the mode survives the clean reopen"
        );
        assert!(
            s.buffers.get(&bufk).unwrap().editable,
            "editability survives the clean reopen"
        );
    }

    // ── plan 016 issue 03: the reload contract, composed with the landed
    // ask policy (issue-external-change-reload) ────────────────────────────

    /// plan 016 issue 03 (the `y` half of the reload contract): after an
    /// external-change confirm answered `y`, the history MUST be cleared
    /// and the marker reset — the recorded offsets referred to content that
    /// no longer exists, so no step may survive (and undo is a no-op with a
    /// message). Composed with the landed ask policy: the confirm arms only
    /// because the buffer was dirty; `y` re-reads from disk and the buffer
    /// proves clean again.
    #[test]
    fn reload_confirm_y_clears_history_and_resets_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = reopen_fixture(dir.path());
        let mut s = store(dir.path());
        s.open_path("src/t.rs");
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        // Two edits so a live history exists at the confirm.
        s.insert_text("z");
        s.insert_text("z");
        assert_eq!(
            s.buffers.get(&bufk).unwrap().undo.len(),
            2,
            "precondition: a live undo history"
        );
        std::fs::write(&path, "fn externally() {}\n").unwrap();
        force_mtime_divergence(&mut s, &bufk, &path);
        s.open_path("src/t.rs");
        assert!(
            s.reload_confirm_active(),
            "dirty reopen must arm the confirm (msg: {})",
            s.message
        );
        // `y`: the explicit discard + reload.
        s.key_event(key("y"));
        let buf = s.buffers.get(&bufk).unwrap();
        assert_eq!(
            buf.undo.len(),
            0,
            "y reload: the recorded offsets referred to content that no longer exists — the history is cleared"
        );
        assert_eq!(
            buf.saved_marker,
            Some(0),
            "y reload: the marker resets to the fresh sentinel (content re-read from disk)"
        );
        assert!(
            !buf.locally_modified(),
            "y reload: the buffer proves clean (disk truth)"
        );
        assert_eq!(s.buffer_text(), "fn externally() {}\n");
        // Undo after the clear is a no-op with a message, not a stale apply.
        s.dispatch("undo", None).unwrap();
        assert!(
            s.message.contains("nothing to undo"),
            "a cleared history has nothing to undo: {:?}",
            s.message
        );
        assert_eq!(
            s.buffers.get(&bufk).unwrap().text(),
            "fn externally() {}\n",
            "the no-op undo must not touch the text"
        );
    }

    /// plan 016 issue 03 (the `n` half of the reload contract): after the
    /// external-change confirm is cancelled with `n`, the history stays
    /// INTACT (the user kept their edits — the recorded offsets are still
    /// valid) and the buffer stays modified. Undo must still work: stepping
    /// back through the surviving history to the saved position proves the
    /// buffer clean again (the marker survived the cancel untouched).
    #[test]
    fn reload_confirm_n_preserves_history_and_stays_modified() {
        let dir = tempfile::tempdir().unwrap();
        let path = reopen_fixture(dir.path());
        let mut s = store(dir.path());
        s.open_path("src/t.rs");
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        // Two edits, a SAVE (the marker lands on the post-save position),
        // then one more edit: the buffer is dirty at the confirm.
        s.insert_text("a");
        s.insert_text("a");
        assert!(s.save_buffer_key(&bufk), "the save must land");
        assert!(
            !s.buffers.get(&bufk).unwrap().locally_modified(),
            "saved: clean before the last edit"
        );
        s.insert_text("b");
        assert!(s.buffers.get(&bufk).unwrap().locally_modified());
        std::fs::write(&path, "fn externally() {}\n").unwrap();
        force_mtime_divergence(&mut s, &bufk, &path);
        s.open_path("src/t.rs");
        assert!(
            s.reload_confirm_active(),
            "dirty reopen must arm the confirm (msg: {})",
            s.message
        );
        // `n`: cancel — the edits and the history are kept.
        s.key_event(key("n"));
        assert!(!s.reload_confirm_active());
        let buf = s.buffers.get(&bufk).unwrap();
        assert_eq!(
            buf.undo.len(),
            3,
            "n cancel: the history stays INTACT (the user kept their edits)"
        );
        assert!(
            buf.locally_modified(),
            "n cancel: the buffer stays modified"
        );
        assert_eq!(
            s.buffer_text(),
            "fn old() {}\naab",
            "n cancel: the unsaved text survives: {:?}",
            s.buffer_text()
        );
        // The intact history still undoes: the recorded offsets are valid
        // (the content was never replaced). One undo removes the "b".
        s.dispatch("undo", None).unwrap();
        assert!(
            !s.buffers.get(&bufk).unwrap().locally_modified(),
            "undo back to the saved position proves the buffer clean (the marker survived the cancel)"
        );
        assert_eq!(
            s.buffer_text(),
            "fn old() {}\naa",
            "the intact history restores the pre-last-edit (saved) text"
        );
    }


    // ── plan 004 issue 03: mark / region / kill ring / yank tests ──────


    // ── U-E12: tree sidebar narrowing (018-fu-tree-narrow) ──────────

    /// The U-E12 pin fixture: five files in sorted walk order —
    /// `Cargo.toml` (0; ALSO the project marker `detect_root` needs, the
    /// rest of the suite's `store()` fixtures carry it the same way),
    /// `alpha.txt` (1), `beta.txt` (2), `src/delta.rs` (3),
    /// `src/gamma.rs` (4). The discriminating queries, measured against
    /// this exact set (nucleo `Pattern::parse(.., Ignore, Smart)` over the
    /// FULL relative paths): `gam` scores exactly `src/gamma.rs`; `src`
    /// scores exactly the two files under the `src/` directory (the parent
    /// component match keeps its children); `d`/`da` exactly
    /// `src/delta.rs`; `gamz` and `zzz` score NOTHING (zero-match legs).
    fn tree_narrow_fixture(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.join("alpha.txt"), "a\n").unwrap();
        std::fs::write(dir.join("beta.txt"), "b\n").unwrap();
        std::fs::write(dir.join("src/delta.rs"), "d\n").unwrap();
        std::fs::write(dir.join("src/gamma.rs"), "g\n").unwrap();
    }

    fn type_tree_query(s: &mut AppStore, q: &str) {
        for c in q.chars() {
            s.key_event(Key::char(c));
        }
    }

    /// U-E12: the tree narrow-prompt guard — printable chars extend the
    /// tree query live, Backspace pops, and the sidebar's OWN keys fall
    /// through to the keymap engine instead of the query: `?` (the globally
    /// bound transient-menu key) never extends the query, the `t`
    /// COMPLETING the bound `C-c p t` toggle reaches the engine and
    /// toggles the tree (the pending-chord discipline: a non-empty pending
    /// means the key MUST reach the engine), and the `2` of `C-x 2` reaches
    /// the engine (split-window-vertical's echo lands — the buffer view
    /// binds it), with the query untouched; and an unbound COMPLETION of an
    /// armed prefix (C-c p z) dead-ends to the unbound-key echo, never the
    /// query (the arm itself proven load-bearing by mutation) — the fifth
    /// surface of the `issue-narrow-guard-pending-prefix` discipline.
    #[test]
    fn tree_narrow_guard_extends_the_query_and_bound_keys_fall_through() {
        let dir = tempfile::tempdir().unwrap();
        tree_narrow_fixture(dir.path());
        let mut s = store(dir.path());
        s.ensure_files();
        s.toggle_tree();
        assert!(s.tree_visible());
        assert_eq!(s.top_view(), ViewId::Home, "boot is the home view (tree guard is active over it)");

        // Typing extends the query live (the top view is Home — the
        // sidebar's key surface, no buffer open).
        type_tree_query(&mut s, "gam");
        assert_eq!(s.tree_narrow_query(), "gam", "the typed chars must reach the tree narrow query (the prompt guard)");

        // Open a buffer (the tree stays visible): `?` — the globally bound
        // transient-menu key (a single-char binding the engine resolves) —
        // must fall through to the keymap, never extend the query.
        s.open_path("alpha.txt");
        assert_eq!(s.top_view(), ViewId::Buffer);
        s.key_event(key("?"));
        assert!(s.menu_open(), "`?` must open the transient menu through the engine");
        assert_eq!(s.tree_narrow_query(), "gam", "`?` must fall through, not extend the query");
        // Close the menu again (its own C-g — the menu owns the key while
        // open, before the tree guard can see it).
        s.key_event(key("C-g"));
        assert!(!s.menu_open(), "the menu closed");
        assert_eq!(s.tree_narrow_query(), "gam", "the menu's C-g must not touch the tree query");

        // The `t` COMPLETING the bound `C-c p t` (toggle-tree) must reach
        // the engine: the armed prefix (C-c, p) is exactly the "mid-
        // sequence" case — a non-empty pending means the key MUST reach
        // the engine (a swallowed `t` would strand the toggle sequence).
        s.key_event(key("C-c"));
        s.key_event(key("p"));
        assert_eq!(s.pending_display(), "C-c p");
        s.key_event(key("t"));
        assert!(!s.tree_visible(), "C-c p t must toggle the tree off through the engine");
        assert_eq!(s.tree_narrow_query(), "gam", "the `t` completing C-c p t must NOT be swallowed into the query");

        // Re-open the tree: the still-active query re-derives (the magit/
        // log query-persists precedent) and the selection re-clamps into
        // the surviving set (gamma, the only `gam` survivor).
        s.key_event(key("C-c"));
        s.key_event(key("p"));
        s.key_event(key("t"));
        assert!(s.tree_visible());
        assert_eq!(s.tree_selected(), 4, "re-clamped onto the surviving gamma row (full-list index)");

        // The C-x 2 twin: a chord that arms a prefix (C-x carries no char
        // value, so the guard cannot swallow it); the follow-up `2`
        // COMPLETES the bound `C-x 2` (split-window-vertical — the buffer
        // view binds it) and must reach the engine: the command's own echo
        // lands and the query stays put (the pre-fix guard fed the `2` into
        // the query, stranding the chord).
        s.key_event(key("C-x"));
        s.key_event(key("2"));
        assert_eq!(
            s.message,
            "single pane by design: redline runs as a herdr popup, one pane",
            "the `2` must reach the engine (C-x 2 = split-window-vertical)"
        );
        assert_eq!(s.tree_narrow_query(), "gam", "the `2` must NOT be swallowed into the narrow query");

        // The pending PREFIX itself is load-bearing too (proven by mutation:
        // disabling the `!self.pending.is_empty()` arm makes the guard
        // swallow the follow-up key instead of letting the engine dead-end
        // it): with the `C-c p` prefix armed, the unbound completion `z`
        // (no `C-c p z` binding) must reach the engine (unbound-key echo,
        // pending cleared) — never the query.
        s.key_event(key("C-c"));
        s.key_event(key("p"));
        assert_eq!(s.pending_display(), "C-c p");
        s.key_event(key("z"));
        assert_eq!(s.message, "unbound key: z", "the armed prefix's unbound completion must reach the engine");
        assert_eq!(s.tree_narrow_query(), "gam", "an unbound completion of an armed prefix must NOT be swallowed into the query");
        assert_eq!(s.pending_display(), "", "the engine cleared the dead prefix");

        // Backspace pops the last character (live re-derive).
        s.key_event(Key::new(KeyCode::Backspace));
        assert_eq!(s.tree_narrow_query(), "ga");
    }

    /// U-E12 survival rule: a node matching the query survives WITH its
    /// ancestor chain rendered (the row's depth indentation — a child hit
    /// is never hidden behind a filtered-out sibling), non-matching leaves
    /// collapse, a parent component match keeps every file under it
    /// (filter-children-keep-parents), the rows are never re-ranked
    /// (source order), and the tree's UNDERLYING DATA is never mutated by
    /// the query (view-time projection — the full rows stay intact through
    /// the narrow, and the clear re-derives them byte-for-byte).
    #[test]
    fn tree_narrow_survival_rule_keeps_the_ancestor_chain_and_mutates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        tree_narrow_fixture(dir.path());
        let mut s = store(dir.path());
        s.ensure_files();
        s.toggle_tree();
        let full: Vec<String> = s.tree_rows().iter().map(|r| r.rel_path.clone()).collect();
        assert_eq!(
            full,
            vec!["Cargo.toml", "alpha.txt", "beta.txt", "src/delta.rs", "src/gamma.rs"],
            "the fixture must walk in sorted order (the pins below index into it)"
        );

        // `gam` matches exactly ONE file: the rendered row set is exactly
        // that row — and it carries its depth-1 indentation (the ancestor
        // `src` chain is visible, never hidden behind the filtered-out
        // siblings).
        type_tree_query(&mut s, "gam");
        let (rows, sel) = s.tree_view_info();
        assert_eq!(
            rows.iter().map(|r| r.rel_path.clone()).collect::<Vec<_>>(),
            vec!["src/gamma.rs"],
            "a node matching the query survives WITH its ancestor chain; the non-matching leaves collapse: {rows:?}"
        );
        assert_eq!(rows[0].depth, 1, "the survivor keeps its depth (the ancestor chain renders)");
        assert_eq!(sel, 0, "the (clamped) selection is the in-window index of the survivor");

        // The shared-core cross-check over the tree's OWN paths (never a
        // copy of the projection's strings): the core scores exactly
        // gamma, and the projection is that core-matched row alone.
        let core_matched: Vec<String> = full
            .iter()
            .filter(|p| !crate::app::store::narrowing::narrow("gam", &[p.as_str()], &mut s.matcher).is_empty())
            .cloned()
            .collect();
        assert_eq!(
            core_matched,
            vec!["src/gamma.rs"],
            "the fixture must discriminate: the shared core scores exactly gamma"
        );

        // Parent component match keeps its children (filter-children-keep-
        // parents): `src` survives on the TWO files under it, in SOURCE
        // order (never re-ranked).
        s.key_event(key("C-g"));
        assert_eq!(s.tree_narrow_query(), "");
        type_tree_query(&mut s, "src");
        let (rows, _) = s.tree_view_info();
        assert_eq!(
            rows.iter().map(|r| r.rel_path.clone()).collect::<Vec<_>>(),
            vec!["src/delta.rs", "src/gamma.rs"],
            "the parent component match keeps every file under it: {rows:?}"
        );

        // The underlying data is never mutated by the query (view-time
        // projection): the full rows are untouched through the whole
        // narrow…
        let after: Vec<String> = s.tree_rows().iter().map(|r| r.rel_path.clone()).collect();
        assert_eq!(after, full, "the canonical rows must be untouched by the query");
        // …and the clear re-derives the full tree byte-for-byte.
        s.key_event(key("C-g"));
        let (rows, _) = s.tree_view_info();
        assert_eq!(
            rows.iter().map(|r| r.rel_path.clone()).collect::<Vec<_>>(),
            full,
            "C-g: the full tree re-derives byte-for-byte"
        );
    }

    /// U-E12 selection fate: if the selection SURVIVES it stays (identity
    /// is the file, not a row index); if it does not, it clamps onto the
    /// FIRST surviving row in source order; a zero-match query leaves it
    /// alone (no surviving row to rest on) and the index stays valid
    /// against the FULL list — the stale-index class U-E10/U-E13 caught:
    /// `selected` must never index into the narrowed set.
    #[test]
    fn tree_narrow_selection_clamps_into_the_surviving_set_and_never_loses_identity() {
        let dir = tempfile::tempdir().unwrap();
        tree_narrow_fixture(dir.path());
        let mut s = store(dir.path());
        s.ensure_files();
        s.toggle_tree();

        // The surviving-selection-stays leg: rest on `src/delta.rs` (row 3)
        // and narrow to `d` — delta survives, so the selection stays on it.
        s.tree_move_down();
        s.tree_move_down();
        s.tree_move_down();
        assert_eq!(s.tree_selected(), 3);
        type_tree_query(&mut s, "d");
        assert_eq!(s.tree_selected(), 3, "a surviving selection stays on its file");
        assert_eq!(s.tree.rows[3].rel_path, "src/delta.rs");

        // The clamp leg: `gam` filters delta out — the selection clamps
        // onto the first surviving row in source order (gamma, row 4),
        // keeping its FULL-LIST index (the identity is the file).
        for _ in 0..2 {
            s.key_event(Key::new(KeyCode::Backspace));
        }
        assert_eq!(s.tree_narrow_query(), "");
        type_tree_query(&mut s, "gam");
        assert_eq!(s.tree_selected(), 4, "clamped onto the first surviving row");
        assert_eq!(s.tree.rows[4].rel_path, "src/gamma.rs");
        let (rows, sel) = s.tree_view_info();
        assert_eq!(rows.len(), 1, "the narrowed window holds exactly the survivor");
        assert_eq!(rows[sel].rel_path, "src/gamma.rs", "the in-window selection IS the surviving file");

        // The zero-match leg: extend `gam` to `gamz` (no path holds a
        // g…a…m…z chain — gamma's path is g…a…m…m…a: a z never trails the
        // match) — nothing survives: the empty state is sane (empty
        // window, no panic, no highlight row) and the selection is LEFT
        // ALONE (no surviving row to rest on; it must not index into the
        // narrowed set, because `tree_open_selected` reads it against the
        // full rows).
        s.key_event(Key::char('z'));
        assert_eq!(s.tree_narrow_query(), "gamz");
        let (rows, sel) = s.tree_view_info();
        assert!(rows.is_empty(), "zero matches: the empty state renders no rows: {rows:?}");
        assert_eq!(sel, 0, "zero matches: no row to highlight (in-window index is inert)");
        assert_eq!(s.tree_selected(), 4, "zero matches: the selection is left alone (still a FULL-list index)");
        // The stale-index class: the selection must still address the FULL
        // list — RET opens the file at the full index (gamma), never a
        // narrowed-set row.
        s.tree_open_selected();
        let want = s.project.as_ref().unwrap().root.join("src/gamma.rs").to_string_lossy().into_owned();
        assert_eq!(
            s.buffers.current(),
            Some(want.as_str()),
            "the selection must index the FULL row list (the stale-index class)"
        );

        // The clear re-derives the full list with the selection intact.
        s.key_event(key("C-g"));
        assert_eq!(s.tree_narrow_query(), "");
        let (rows, sel) = s.tree_view_info();
        assert_eq!(rows.len(), 5, "the full tree re-derives");
        assert_eq!(rows[sel].rel_path, "src/gamma.rs", "the selection survived the clear (clamped, not lost)");
    }

    /// U-E12 C-g: the clear is layered and honest — the full tree
    /// re-derives, the message is the shared `filter cleared` (v1: clear,
    /// not close — the sidebar has no close: `C-c p t` toggles it), and a
    /// second C-g on the already-empty query is still the clear path (not
    /// the global cancel — the prompt stays live while the sidebar is on
    /// the key surface).
    #[test]
    fn tree_narrow_c_g_clears_the_query_and_the_tree_stays() {
        let dir = tempfile::tempdir().unwrap();
        tree_narrow_fixture(dir.path());
        let mut s = store(dir.path());
        s.ensure_files();
        s.toggle_tree();
        type_tree_query(&mut s, "gam");
        assert_eq!(s.tree_selected(), 4, "clamped onto the survivor before the clear");

        s.key_event(key("C-g"));
        assert_eq!(s.tree_narrow_query(), "");
        assert_eq!(s.message, "filter cleared");
        assert_eq!(s.tree_view_info().0.len(), 5, "the full list re-derived");
        assert_eq!(s.tree_selected(), 4, "the selection survived the clear");
        assert!(s.tree_visible(), "C-g must not toggle/close the tree");

        // The nothing-survives-then-clear leg (the U-E10/U-E13 stale-
        // index class): `zzz` survives nothing, the selection is left
        // alone, and the clear restores the full list with it intact.
        type_tree_query(&mut s, "zzz");
        assert!(s.tree_view_info().0.is_empty(), "`zzz` survives nothing");
        assert_eq!(s.tree_selected(), 4, "the empty survivor set leaves the selection where it was");
        s.key_event(key("C-g"));
        assert_eq!(s.tree_view_info().0.len(), 5);
        assert_eq!(s.tree_selected(), 4, "the full list re-derived with the selection intact");
        // A C-g on an already-empty query is still the clear path (the
        // prompt is live: it must not reach the global cancel).
        s.key_event(key("C-g"));
        assert_eq!(s.message, "filter cleared", "empty-query C-g is the clear path, not a cancel");
        assert!(s.tree_visible());
    }

    /// U-E12: arrows step WITHIN the narrowed set — a filtered-out file is
    /// never a landing spot (down at the last survivor stays put; up at
    /// the first survivor never steps onto a filtered-out predecessor),
    /// and the full-list no-wrap boundary semantics are kept.
    #[test]
    fn tree_narrow_arrows_move_within_the_narrowed_set() {
        let dir = tempfile::tempdir().unwrap();
        tree_narrow_fixture(dir.path());
        let mut s = store(dir.path());
        s.ensure_files();
        s.toggle_tree();
        // `s` survives on the two `src/` files (rows 3 and 4); the
        // selection (row 0, filtered out) clamps onto the first survivor.
        type_tree_query(&mut s, "s");
        assert_eq!(s.tree_selected(), 3, "clamped onto the first surviving row");

        // Down walks the survivors only (delta → gamma), then stops (no
        // wrap past the narrowed set — rows 0/1/2 are filtered out and row
        // 4 is the last survivor).
        s.tree_move_down();
        assert_eq!(s.tree_selected(), 4, "down: onto the next survivor");
        s.tree_move_down();
        assert_eq!(s.tree_selected(), 4, "down: no wrap past the narrowed set");
        // Up steps back through the survivors only and never onto a
        // filtered-out predecessor (rows 0/1/2).
        s.tree_move_up();
        assert_eq!(s.tree_selected(), 3, "up: onto the previous survivor");
        s.tree_move_up();
        assert_eq!(s.tree_selected(), 3, "up: the filtered-out rows above are not allowed predecessors");

        // The full-list move (empty query) is byte-for-byte unchanged:
        // clear the query first, then back down to row 1 down still lands
        // on row 2 — a row the `s` query would have filtered out (motion
        // is only narrowed while a query is active).
        s.key_event(key("C-g"));
        s.tree_move_up();
        s.tree_move_up();
        assert_eq!(s.tree_selected(), 1, "back onto the root-level row (full-list motion)");
        s.tree_move_down();
        assert_eq!(s.tree_selected(), 2, "the full-list motion is untouched by the narrow");
    }

    /// U-E12 windowing re-home (PLAN §1 row 4 — the ONE surface whose
    /// windowing used to live in the RENDERER): the store's window tracks
    /// the selection (the pre-re-home `sel.saturating_sub(5)` +
    /// `TREE_VISIBLE_ROWS` math, verbatim), the invariant is that the
    /// cursor row is ALWAYS inside the window, and the window is over the
    /// NARROWED set while a query is active (with an empty query that IS
    /// the full list — byte-for-byte the old renderer window).
    #[test]
    fn tree_view_info_window_tracks_the_selection_and_always_contains_the_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // `Cargo.toml` (the project marker) + 12 `fileNN` rows: 13 rows,
        // sorted as Cargo.toml (0), file00.rs (1), …, file11.rs (12).
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        for i in 0..12 {
            std::fs::write(root.join(format!("file{i:02}.rs")), "x\n").unwrap();
        }
        let mut s = store(root);
        s.ensure_files();
        s.toggle_tree();
        assert_eq!(s.tree_rows().len(), 13, "13 rows: the window (8) must actually clip");

        // Selection at 0: the window is the first 8 rows, cursor in-window 0.
        let (rows, sel) = s.tree_view_info();
        assert_eq!(rows.len(), 8);
        assert_eq!(rows[sel].rel_path, "Cargo.toml");
        assert!(sel <= 5, "the cursor's in-window index is within 5 rows of the window top");

        // Selection at 12 (file11): the window slides (top = 12 - 5 = 7)
        // and the cursor row is STILL inside it (in-window 5 — the last
        // slot).
        for _ in 0..12 {
            s.tree_move_down();
        }
        let (rows, sel) = s.tree_view_info();
        assert_eq!(rows.len(), 6, "the window slides to the list end (6 rows): {rows:?}");
        assert_eq!(rows[sel].rel_path, "file11.rs", "the cursor row is always inside the window");
        assert_eq!(rows[0].rel_path, "file06.rs", "the window top is sel.saturating_sub(5)");

        // The window is over the NARROWED set while a query is active:
        // `file11` scores exactly the last row (a subsequence of no other
        // `fileNN` — `file10` has no trailing 1), so the window holds the
        // single survivor with the cursor inside it.
        for c in "file11".chars() {
            s.key_event(Key::char(c));
        }
        let (rows, sel) = s.tree_view_info();
        assert_eq!(
            rows.iter().map(|r| r.rel_path.clone()).collect::<Vec<_>>(),
            vec!["file11.rs"],
            "the store window is over the narrowed set: {rows:?}"
        );
        assert_eq!(rows[sel].rel_path, "file11.rs");
        assert_eq!(s.tree_selected(), 12, "the full-list index is unchanged by the window");
    }

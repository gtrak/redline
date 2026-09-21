use super::*;

    #[test]
    fn bare_q_in_home_view_is_unbound_not_quit() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        // 06a: bare `q` on the home view is UNBOUND (no buffer to close) —
        // an unbound-key echo, NOT a quit and NOT a view change.
        s.key_event(key("q"));
        assert!(!s.quit, "bare q must not quit the app");
        assert_eq!(s.view_stack.len(), 1, "the home view must remain");
        assert_eq!(s.top_view(), ViewId::Home);
        assert!(s.message.contains("unbound key: q"), "msg: {}", s.message);

        // `q` closes an overlay list view back to home (consistent with
        // the list views' `q`).
        s.key_event(key("C-x"));
        s.key_event(key("C-b")); // list-buffers
        assert_eq!(s.top_view(), ViewId::BufferList);
        s.key_event(key("q"));
        assert_eq!(s.top_view(), ViewId::Home, "q must close the list view");
        assert!(!s.quit);

        // `C-x C-c` quits IMMEDIATELY from home (no buffers ⇒ nothing to
        // prompt; the 004-04 semantics).
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit, "C-x C-c must still quit");
        assert!(!s.quit_prompt_active(), "no save prompt with zero buffers");
    }

    #[test]
    fn toggle_read_only_noop_on_scratch_and_non_buffer_views() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        // 06a: boot has NO current buffer — toggle is a no-op with a message
        // (and must not create one). Then explicit scratch (the 06a
        // affordance): scratch (no path) is a no-op with its own message.
        s.toggle_read_only();
        assert!(s.message.contains("not a buffer view"), "msg: {}", s.message);
        assert_eq!(s.buffers.len(), 0, "toggle must not create a buffer");
        s.open_scratch();
        s.toggle_read_only();
        assert!(s.message.contains("scratch"), "msg: {}", s.message);
        let scratch_key = SCRATCH_NAME.to_string();
        assert!(s.buffers.get(&scratch_key).unwrap().editable);

        // Non-buffer view: no-op even though a file buffer is current.
        let (_dir2, mut s2) = file_buffer_store();
        s2.push_view(ViewId::BufferList);
        let bufk = s2.buffers.current().unwrap().to_string();
        s2.toggle_read_only();
        assert!(s2.message.contains("not a buffer view"), "msg: {}", s2.message);
        assert!(!s2.buffers.get(&bufk).unwrap().editable,
            "the toggle must not fire outside the buffer view");
    }

    /// Boot pin: the table starts empty, `current` is `None`, and the main
    /// view is home with a DERIVED header/body (project + "redline",
    /// registry categories + live global bindings, no buffer-view keys).
    #[test]
    fn boot_starts_on_home_with_empty_buffer_table() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let name = dir.path().file_name().unwrap().to_string_lossy().into_owned();
        let s = store(dir.path());
        assert_eq!(s.top_view(), ViewId::Home);
        assert_eq!(s.render_view(), ViewId::Home);
        assert_eq!(s.view_name_display(), "home");
        assert_eq!(s.buffers.len(), 0, "06a: no auto-created buffer at boot");
        assert!(s.buffers.current().is_none());
        assert!(
            s.home_title().starts_with(&format!("redline · {name}")),
            "{}",
            s.home_title()
        );
        // The home body is derived: registry categories + live globals.
        let rows = s.home_rows();
        assert!(rows.iter().any(|r| r.is_header && r.label == "files"));
        assert!(rows.iter().any(|r| r.is_header && r.label == "git"));
        assert!(rows.iter().any(|r| !r.is_header && r.key_display == "C-x C-f"));
        assert!(rows.iter().any(|r| !r.is_header && r.key_display == "C-x C-c"));
        // View-local buffer keys are NOT on home (the view map is empty).
        assert!(!rows.iter().any(|r| !r.is_header && r.key_display == "q"));
    }

    /// Anti-drift pin (06a): home's body must be GENERATED from the live
    /// keymap × command registry — mutate the registry and the keymap in a
    /// test and home's derived content must change. A hand-maintained
    /// string list would not.
    #[test]
    fn home_rows_track_live_keymap_and_registry() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        let before = s.home_rows();
        // Rebind an existing global command to a new sequence: the new
        // key row appears on home.
        s.engine.global.bind(&[Key::alt_char('z')], "open-palette").unwrap();
        let after_rebind = s.home_rows();
        assert_ne!(before, after_rebind, "rebinding must change home's derived rows");
        assert!(after_rebind.iter().any(|r| !r.is_header && r.key_display == "M-z"));
        // Register a brand-new command + bind it: a new category appears.
        s.registry.register(crate::app::command::Command::new(
            "home-probe-command",
            "probe docs",
            "home-probe-category",
            |_, _| {},
        ));
        s.engine
            .global
            .bind(&[Key::alt_char('y')], "home-probe-command")
            .unwrap();
        let after_register = s.home_rows();
        assert_ne!(after_rebind, after_register, "registry mutation must change home");
        assert!(after_register.iter().any(|r| r.is_header && r.label == "home-probe-category"));
        assert!(after_register.iter().any(
            |r| !r.is_header && r.key_display == "M-y" && r.label == "probe docs"
        ));
        // The `?` menu (same machinery, at the top path) tracks it too.
        assert!(s
            .menu_rows()
            .iter()
            .any(|r| r.is_header && r.label == "home-probe-category"));
    }

    /// 06a key contract on home: `?` opens the descendable menu; every
    /// global entry point works FROM home and replaces home with the
    /// opened view; the explicit scratch affordance still creates scratch.
    #[test]
    fn home_entry_points_replace_home_and_scratch_stays_explicit() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        // `?` opens the menu on home (and C-g closes it).
        let mut s = store(dir.path());
        s.key_event(key("?"));
        assert!(s.menu_open());
        s.key_event(key("C-g"));
        assert!(!s.menu_open());

        // C-x C-f landing (open_path) replaces home with the buffer view.
        let mut s = store(dir.path());
        s.open_path("src/main.rs");
        assert_eq!(s.top_view(), ViewId::Buffer);
        assert_eq!(s.view_stack, vec![ViewId::Buffer]);
        assert_eq!(s.view_name_display(), "src/main.rs");

        // C-x g pushes magit on top of home; q returns to home.
        let mut s = store(dir.path());
        // A git repo so C-x g has something to show (open_magit_status
        // reports and stays on home in a non-git dir).
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["init", "-q", "-b", "main"])
            .output();
        s.key_event(key("C-x"));
        s.key_event(key("g"));
        assert_eq!(s.top_view(), ViewId::MagitStatus, "C-x g must open magit from home");
        assert_eq!(s.view_stack, vec![ViewId::Home, ViewId::MagitStatus]);
        s.key_event(key("q"));
        assert_eq!(s.top_view(), ViewId::Home, "q on magit returns to home");

        // The explicit affordance: open-scratch creates scratch on demand
        // and replaces home.
        let mut s = store(dir.path());
        s.open_scratch();
        assert_eq!(s.top_view(), ViewId::Buffer);
        assert_eq!(s.buffers.current(), Some(SCRATCH_NAME));
        assert_eq!(s.view_name_display(), "*scratch*");
    }

    /// Killing the LAST buffer returns to home: empty table, no current,
    /// and NO scratch is created by the kill (06a's "no accidental buffer
    /// creation" audit path).
    #[test]
    fn kill_last_buffer_returns_to_home_without_creating_scratch() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut s = store(dir.path());
        s.open_path("src/main.rs");
        assert_eq!(s.top_view(), ViewId::Buffer);
        let key = s.buffers.current().unwrap().to_string();
        s.kill_buffer(&key);
        assert_eq!(s.buffers.len(), 0);
        assert!(s.buffers.current().is_none());
        assert!(s.buffers.get(SCRATCH_NAME).is_none(), "no scratch from the kill");
        assert_eq!(s.top_view(), ViewId::Home);
        assert_eq!(s.render_view(), ViewId::Home);
        assert_eq!(s.view_name_display(), "home");
        assert!(s.message.contains("killed"), "{:?}", s.message);
    }

    /// 06a review P1 repro-mirror: a jump entry whose buffer no longer
    /// exists must NOT create `*scratch*` on `M-,`/`C-i` — the honest
    /// "no buffer" report leaves the view and the table unchanged
    /// (the contract's no-accidental-buffer-creation invariant, both
    /// directions of the stack).
    #[test]
    fn dead_jump_entry_reports_no_buffer_without_creating_scratch() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut s = store(dir.path());
        s.open_path("src/main.rs");
        let before = s.buffers.len();
        // A dead entry: the buffer was killed (or never open) — simulate
        // exactly what a failed search-RET / stale origin would record.
        let dead = JumpEntry {
            buffer_key: "/gone/gone.rs".to_string(),
            line: 3,
            col: 0,
            label: "search-RET".to_string(),
        };
        s.jump_stack.record_jump(
            &JumpEntry {
                buffer_key: SEARCH_JUMP_KEY.to_string(),
                line: 0,
                col: 0,
                label: "*search*".to_string(),
            },
            &dead,
        );
        // `M-,` returns to the ORIGIN (the search sentinel); the dead entry
        // is the DESTINATION, reached with `C-i` (jump-forward) — the
        // search-RET-then-C-i repro shape from the review.
        s.jump_back();
        assert_eq!(s.buffers.len(), before);
        assert!(s.buffers.get(SCRATCH_NAME).is_none());
        s.jump_forward();
        assert_eq!(s.buffers.len(), before, "no buffer created by a dead entry");
        assert!(s.buffers.get(SCRATCH_NAME).is_none());
        assert!(s.message.contains("no buffer"), "{:?}", s.message);
    }


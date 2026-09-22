use super::*;

    #[test]
    fn pending_prefix_shows_and_cancels() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.key_event(key("C-x"));
        assert_eq!(store.pending_display(), "C-x");
        assert!(store.message.is_empty());

        store.key_event(key("C-g"));
        assert!(store.pending.is_empty());
        assert_eq!(store.message, "cancel");
    }

    #[test]
    fn pending_then_full_match_dispatches() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.key_event(key("C-x"));
        assert_eq!(store.pending_display(), "C-x");
        store.key_event(key("C-c"));
        assert!(store.quit);
        assert!(store.pending.is_empty());
    }

    #[test]
    fn unknown_key_after_prefix_cancels_pending() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.key_event(key("C-x"));
        assert_eq!(store.pending_display(), "C-x");
        store.key_event(key("C-z"));
        assert!(store.pending.is_empty());
        assert_eq!(store.message, "unbound key: C-z");
    }

    #[test]
    fn c_x_c_q_and_c_x_c_s_are_bound() {
        let (_dir, s) = file_buffer_store();
        use crate::app::keymap::{Lookup, parse_sequence};
        assert_eq!(
            s.engine.resolve(&parse_sequence("C-x C-q").unwrap()),
            Some(Lookup::Command("toggle-read-only")),
            "C-x C-q must resolve to toggle-read-only"
        );
        assert_eq!(
            s.engine.resolve(&parse_sequence("C-x C-s").unwrap()),
            Some(Lookup::Command("save-buffer")),
            "C-x C-s must resolve to save-buffer"
        );
    }

    #[test]
    fn issue_02_bindings_resolve_including_c_c_p_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        use crate::app::keymap::{Lookup, parse_sequence};
        let expect = |seq: &str, cmd: &str| {
            assert_eq!(
                store.engine.resolve(&parse_sequence(seq).unwrap()),
                Some(Lookup::Command(cmd)),
                "`{seq}` must resolve to `{cmd}`"
            );
        };
        expect("C-x C-f", "find-file");
        expect("C-x b", "switch-buffer");
        expect("C-x C-b", "list-buffers");
        expect("C-x k", "kill-buffer");
        expect("C-c p f", "find-file");
        expect("C-c p p", "switch-project");
        expect("C-c p e", "recent-files");
        expect("C-c p i", "re-walk");
        // The C-c p prefix path stays pending while building.
        assert_eq!(
            store.engine.resolve(&parse_sequence("C-c p").unwrap()),
            Some(Lookup::Pending)
        );
    }

    #[test]
    fn unbound_key_echo_suppressed_while_picker_open() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_find_file();
        assert!(store.message.is_empty());
        // PageDown is unbound everywhere; with the picker open it must
        // NOT echo "unbound key".
        store.key_event(key("PGDN"));
        assert!(store.message.is_empty(), "echo leaked: {:?}", store.message);
        assert!(store.picker_open());
    }

    #[test]
    fn set_mark_keybinding_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.open_scratch(); // 06a: boot is home; set-mark lives in the buffer view
        use crate::app::keymap::{Lookup, parse_sequence};
        // C-SPC resolves to set-mark. The terminal delivers C-SPC as
        // Char(' ') + CONTROL (NUL byte decoded by crossterm/iocraft).
        let c_spc = vec![crate::app::keymap::Key::ctrl_char(' ')];
        assert_eq!(
            store.engine.resolve(&c_spc),
            Some(Lookup::Command("set-mark")),
            "C-SPC must resolve to set-mark"
        );
        // Also verify the parser path: "C-SPC" in config resolves to the same key.
        let parsed = parse_sequence("C-SPC").unwrap();
        assert_eq!(parsed, c_spc, "parser C-SPC must match the terminal representation");
        assert_eq!(
            store.engine.resolve(&parsed),
            Some(Lookup::Command("set-mark")),
            "parsed C-SPC must resolve to set-mark"
        );
        // C-w resolves to kill-region.
        assert_eq!(
            store.engine.resolve(&parse_sequence("C-w").unwrap()),
            Some(Lookup::Command("kill-region")),
        );
        // M-w resolves to copy-region.
        assert_eq!(
            store.engine.resolve(&parse_sequence("M-w").unwrap()),
            Some(Lookup::Command("copy-region")),
        );
        // C-y resolves to yank.
        assert_eq!(
            store.engine.resolve(&parse_sequence("C-y").unwrap()),
            Some(Lookup::Command("yank")),
        );
        // M-y resolves to yank-pop.
        assert_eq!(
            store.engine.resolve(&parse_sequence("M-y").unwrap()),
            Some(Lookup::Command("yank-pop")),
        );
        // C-x C-x resolves to exchange-point-and-mark.
        assert_eq!(
            store.engine.resolve(&parse_sequence("C-x C-x").unwrap()),
            Some(Lookup::Command("exchange-point-and-mark")),
        );
    }

    /// (015-03) The notes-edit guard routes the accurate-mode keys to the
    /// point-accurate commands. In particular `C-d` routes to delete-char
    /// (NOT the half-page scroll it used to drive), and `M-d` to kill-word
    /// forward (M-DEL is the backward kill). These go through `key_event` so
    /// the mode routing itself is what is pinned, not just the store
    /// methods.
    #[test]
    fn accurate_mode_keys_route_to_point_commands_via_guard() {
        // C-d: delete-char at the point (freed from half-page scroll).
        {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join("src")).unwrap();
            std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
            std::fs::write(dir.path().join("src/f.rs"), "ab cd\n").unwrap();
            let mut s = store(dir.path());
            s.open_path("src/f.rs");
            s.toggle_read_only(); // Accurate
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 1, 1); // on 'b'
            s.key_event(key("C-d"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "a cd\n",
                "C-d must delete the char at the point, not scroll"
            );
            assert!(!s.message.contains("unbound"), "C-d must be handled: {}", s.message);
        }
        // M-d: kill-word FORWARD (Alt+char 'd'); M-DEL is the backward kill.
        // Forward `kill-word` kills the word at the point only (the space
        // after it survives — emacs `forward-word` does not consume the
        // trailing non-word).
        {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join("src")).unwrap();
            std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
            std::fs::write(dir.path().join("src/f.rs"), "hello world\n").unwrap();
            let mut s = store(dir.path());
            s.open_path("src/f.rs");
            s.toggle_read_only();
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 0, 0); // on "hello"
            s.key_event(key("M-d"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                " world\n",
                "M-d must kill the word forward (the following space survives)"
            );
            // M-DEL: kill-word backward.
            let mut s = store(dir.path());
            s.open_path("src/f.rs");
            s.toggle_read_only();
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 11, 11); // end of line, after "world"
            s.key_event(key("M-DEL"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "hello \n",
                "M-DEL must kill the previous word"
            );
        }
    }

    /// (015-03) `C-d` was freed from half-page scroll. In `Annotation` mode
    /// the guard does not handle it, so it reaches the engine and now echoes
    /// unbound — the accepted loss of C-d half-page scrolling (page scrolling
    /// keeps `C-v`/`M-v`/PGDN/PGUP; `C-u` stays half-page scroll). Pinned so
    /// the loss is documented, not silent.
    #[test]
    fn annotation_mode_c_d_falls_through_unbound() {
        let (_dir, mut s) = notes_store(); // notes buffer: editable + Annotation
        s.set_point(0, 0, 0);
        s.key_event(key("C-d"));
        assert!(
            s.message.contains("unbound key"),
            "C-d is no longer half-page scroll in Annotation mode: {}",
            s.message
        );
    }


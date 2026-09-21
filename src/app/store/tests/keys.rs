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


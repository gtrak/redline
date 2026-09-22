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

    /// plan 016 issue 01: undo is bound to BOTH `C-x u` and `C-/` (SETTLED by
    /// the user). Both must resolve to `undo` in the buffer view; `C-x` stays
    /// a pending prefix (no collision with the `C-x` family). The app-level
    /// Key for `C-/` is pinned here (Char('/') + ctrl); the TERMINAL-side
    /// control-code subtlety (what byte a physical Ctrl+/ sends and how
    /// crossterm decodes it) is pinned in the input-layer test
    /// `crossterm_0x1f_decodes_to_c_7`.
    #[test]
    fn undo_bindings_resolve_and_pin_the_control_code() {
        let km = load_bindings(BUFFER_BINDINGS);
        assert_eq!(
            km.lookup(&parse_sequence("C-x u").unwrap()),
            Some(Lookup::Command("undo")),
            "C-x u must bind undo (fits the C-x family)"
        );
        assert_eq!(
            km.lookup(&parse_sequence("C-/").unwrap()),
            Some(Lookup::Command("undo")),
            "C-/ must bind undo (the settled emacs undo mnemonic)"
        );
        // C-x stays a pending prefix — no collision with the C-x family.
        assert_eq!(
            km.lookup(&parse_sequence("C-x").unwrap()),
            Some(Lookup::Pending),
            "C-x must stay a prefix (C-x u reachable, no C-x collision)"
        );
        // C-/ is a single-key leaf, not a prefix.
        assert!(
            !km.is_prefix(&parse_sequence("C-/").unwrap()),
            "C-/ must be a complete binding, not a prefix"
        );
        // Pin the app-level Key for the C-/ binding rather than assume it.
        let c_slash = parse_sequence("C-/").unwrap()[0];
        assert_eq!(
            c_slash,
            Key::ctrl_char('/'),
            "C-/ must parse to Char('/') + ctrl"
        );
        assert_eq!(c_slash.code, KeyCode::Char('/'));
        assert!(c_slash.ctrl);
    }

    /// plan 016 issue 02: bind `C-7` for undo so that a BYTE-BASED terminal's
    /// physical Ctrl+/ (raw control byte 0x1F, which crossterm decodes as
    /// Char('7') + CONTROL — pinned in `input.rs::crossterm_0x1f_decodes_to_c_7`
    ///) reaches the undo command at the APP level. The three undo bindings
    /// split terminal coverage (stated in the binding table): `C-x u`
    /// (everywhere), `C-/` (CSI-u / kitty terminals only), `C-7` (byte-based
    /// terminals only). This asserts the app-level mapping for the 0x1F byte
    /// reaches undo.
    #[test]
    fn c7_binding_reaches_undo_the_0x1f_app_key() {
        let km = load_bindings(BUFFER_BINDINGS);
        // The app key crossterm produces for raw 0x1F (byte-based Ctrl+/ /
        // Ctrl-_) is Char('7') + CONTROL.
        let seq = parse_sequence("C-7").unwrap();
        let c7 = &seq[0];
        assert_eq!(
            c7,
            &Key::ctrl_char('7'),
            "C-7 must parse to Char('7') + ctrl (the app key crossterm yields for 0x1F)"
        );
        assert_eq!(c7.code, KeyCode::Char('7'));
        assert!(c7.ctrl);
        // That app key must resolve to undo in the buffer view.
        assert_eq!(
            km.lookup(&seq),
            Some(Lookup::Command("undo")),
            "C-7 (0x1F on byte-based terminals) must reach the undo command"
        );
        // C-7 is a single-key leaf, not a prefix (no collision with the
        // C-x family or any longer sequence).
        assert!(!km.is_prefix(&seq), "C-7 must be a complete binding, not a prefix");
        // All three undo bindings now resolve (stated in the commit).
        assert_eq!(
            km.lookup(&parse_sequence("C-x u").unwrap()),
            Some(Lookup::Command("undo")),
            "C-x u (every terminal) must bind undo"
        );
        assert_eq!(
            km.lookup(&parse_sequence("C-/").unwrap()),
            Some(Lookup::Command("undo")),
            "C-/ (CSI-u / kitty terminals) must bind undo"
        );
    }

    /// annotations-render-fold: the `C-c a` tree and its genuine aliases are
    /// pinned so a later refactor cannot silently drop one: `C-c a n` reaches
    /// the SAME command as `A` (annotate — the new-annotation prompt) and
    /// `C-c a l` reaches the SAME command as the global `C-c n a`
    /// (annotations-picker); `C-c a h` / `C-c a s` are the fold pair. Bare
    /// `SHIFT` is pinned as DELIBERATELY UNBOUND (gate P1 on this lane): a
    /// `KeyCode::Modifier` event is unreachable under our stack — crossterm
    /// 0.29 requires BOTH `DISAMBIGUATE_ESCAPE_CODES` (1) and
    /// `REPORT_ALL_KEYS_AS_ESCAPE_CODES` (8) for it, and iocraft 0.9.1
    /// pushes only `REPORT_EVENT_TYPES` (2) — so a Shift binding could
    /// never fire on any terminal; the fold path is `C-c a h` / `C-c a s`.
    #[test]
    fn annotation_tree_bindings_pin_the_aliases_and_fold_pair() {
        let km = load_bindings(BUFFER_BINDINGS);
        let expect = |seq: &str, cmd: &str, why: &str| {
            assert_eq!(
                km.lookup(&parse_sequence(seq).unwrap()),
                Some(Lookup::Command(cmd)),
                "`{seq}` must bind `{cmd}` ({why})"
            );
        };
        // The tree leaves.
        expect("C-c a h", "annotate-hide", "the fold hide leaf");
        expect("C-c a s", "annotate-show", "the fold show leaf");
        // The genuine aliases: the command NAMES are the ones the
        // pre-existing bindings carry (aliasing the command, not a new one).
        let a_cmd = km
            .lookup(&parse_sequence("A").unwrap())
            .and_then(|l| match l { Lookup::Command(c) => Some(c), _ => None })
            .expect("`A` must still bind its command");
        assert_eq!(
            km.lookup(&parse_sequence("C-c a n").unwrap()),
            Some(Lookup::Command(a_cmd)),
            "C-c a n must reach the SAME command as A (new-annotation prompt)"
        );
        assert_eq!(a_cmd, "annotate", "A must bind annotate");
        let global = load_bindings(GLOBAL_BINDINGS);
        let picker_cmd = global
            .lookup(&parse_sequence("C-c n a").unwrap())
            .and_then(|l| match l { Lookup::Command(c) => Some(c), _ => None })
            .expect("the global C-c n a must bind its command");
        assert_eq!(
            km.lookup(&parse_sequence("C-c a l").unwrap()),
            Some(Lookup::Command(picker_cmd)),
            "C-c a l must reach the SAME command as C-c n a (annotations picker)"
        );
        assert_eq!(picker_cmd, "annotations-picker", "C-c n a must bind annotations-picker");
        // `C-c a` is now a PREFIX, not a command (the old toggle leaf is
        // gone — the engine forbids a command on a strict prefix of a longer
        // binding). `C-c` alone stays pending; the global `C-c n a` is
        // still reachable through the engine's dead-end fallthrough.
        assert_eq!(km.lookup(&parse_sequence("C-c a").unwrap()), Some(Lookup::Pending));
        assert!(!km.is_prefix(&parse_sequence("C-c a h").unwrap()), "C-c a h is a complete leaf");
        let engine = global.lookup(&parse_sequence("C-c n a").unwrap());
        assert_eq!(engine, Some(Lookup::Command("annotations-picker")));
        // Bare SHIFT: deliberately UNBOUND (gate P1). A binding was
        // added here once and removed because it could never fire:
        // crossterm 0.29 decodes a bare-Shift press (CSI u keycodes
        // 57441 → `LeftShift`, 57447 → `RightShift`; 57442 is
        // `LeftControl`) to a `KeyCode::Modifier` event only when BOTH
        // `DISAMBIGUATE_ESCAPE_CODES` (1) and `REPORT_ALL_KEYS_AS_ESCAPE_
        // CODES` (8) are enabled, and iocraft 0.9.1 pushes only
        // `REPORT_EVENT_TYPES` (2) — so the event never arrives on any
        // terminal, byte-based or kitty-protocol. A binding that never
        // fires is worse than none; `C-c a h` / `C-c a s` is the fold
        // path (pinned above). This assertion is the absence pin: it
        // fails if someone re-binds SHIFT without first proving the
        // event is reachable.
        let shift = parse_sequence("SHIFT").unwrap();
        assert_eq!(
            km.lookup(&shift),
            None,
            "bare SHIFT must be deliberately unbound (unreachable event — see doc)"
        );
        assert!(!km.is_prefix(&shift), "SHIFT must not prefix any binding");
        // The app-level Key shape stays pinned (the `to_app_key` handler
        // still maps a synthetic `Modifier(LeftShift)`/`Modifier(RightShift)`
        // event to it, as an inert guard — pinned in ui/root/input.rs's
        // `bare_shift_press_maps_to_the_shift_key`): code Shift, no modifier
        // flags (the code IS the modifier — the event's SHIFT flag is not
        // copied, the Char case rule).
        assert_eq!(shift, vec![Key::new(KeyCode::Shift)]);
        assert!(!shift[0].ctrl && !shift[0].alt && !shift[0].shift);
    }


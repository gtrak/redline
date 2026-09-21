use super::*;

    #[test]
    fn find_file_picker_opens_filters_and_opens_on_ret() {
        let dir = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap(); // long-lived: persistence assertions
        project_with_files(dir.path());
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());

        store.key_event(key("C-x"));
        store.key_event(key("C-f"));
        assert!(store.picker_open());
        assert_eq!(store.picker_kind(), Some(PickerKind::FindFile));
        assert_eq!(store.picker_prompt(), "Find file: ");
        assert_eq!(store.picker_count(), (4, 4));
        // Preview shows the first page of the selected (first) file:
        // Cargo.toml in the sorted list.
        assert!(
            store.picker_preview().contains("[package]"),
            "preview: {}",
            store.picker_preview()
        );

        // Type "main" → only src/main.rs survives; RET opens it.
        store.key_event(key("m"));
        store.key_event(key("a"));
        store.key_event(key("i"));
        store.key_event(key("n"));
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.display.as_str())
            .collect();
        assert_eq!(names, vec!["src/main.rs"], "{names:?}");
        store.key_event(key("RET"));
        assert!(!store.picker_open());
        assert_eq!(store.view_name_display(), "src/main.rs");
        assert!(store.buffer_text().contains("println!"));

        // The file made it into recents (persisted under the temp base).
        let root = store.project.as_ref().unwrap().root.to_string_lossy().into_owned();
        assert_eq!(store.project_store.recents.list(&root), vec!["src/main.rs"]);
        assert!(store.project_store.recents_path().is_file());
    }

    #[test]
    fn preview_follows_selection_movement() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_find_file();
        // Empty query: candidates are in sorted source order (no nucleo
        // scoring involved), so the preview expectations are exact.
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["Cargo.toml", "README.md", "src/lib.rs", "src/main.rs"],
            "{names:?}"
        );

        // Index 0: Cargo.toml.
        assert!(
            store.picker_preview().contains("[package]"),
            "preview: {}",
            store.picker_preview()
        );
        // Down moves to README.md and the preview must follow.
        store.key_event(key("DOWN"));
        assert_eq!(store.picker_selected(), 1);
        assert!(
            store.picker_preview().contains("# readme"),
            "preview after DOWN: {}",
            store.picker_preview()
        );
        // C-n moves the same way.
        store.key_event(key("C-n"));
        assert_eq!(store.picker_selected(), 2);
        assert!(
            store.picker_preview().contains("// lib"),
            "preview after C-n: {}",
            store.picker_preview()
        );
        // Up moves back to README.md.
        store.key_event(key("UP"));
        assert_eq!(store.picker_selected(), 1);
        assert!(
            store.picker_preview().contains("# readme"),
            "preview after UP: {}",
            store.picker_preview()
        );
        // Down to src/main.rs, then wrap at the end back to Cargo.toml.
        store.key_event(key("DOWN"));
        store.key_event(key("DOWN"));
        store.key_event(key("DOWN"));
        assert_eq!(store.picker_selected(), 0);
        assert!(store.picker_preview().contains("[package]"));
        // C-p at index 0 wrap-decrements to the last candidate.
        store.key_event(key("C-p"));
        assert_eq!(store.picker_selected(), 3);
        assert!(
            store.picker_preview().contains("println!"),
            "preview after wrap: {}",
            store.picker_preview()
        );
    }

    #[test]
    fn file_preview_stays_capped_for_huge_files() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        // Far beyond the 64KB preview cap: ~200 lines of ~1KB each
        // (~200KB total), with a marker past the cap.
        let line = "x".repeat(1000);
        let mut content = String::new();
        for i in 0..200 {
            if i == 100 {
                content.push_str("HUGE-FILE-MARKER\n");
            }
            content.push_str(&line);
            content.push('\n');
        }
        std::fs::write(dir.path().join("big.txt"), &content).unwrap();

        let mut store = store(dir.path());
        store.open_find_file();
        for c in "big".chars() {
            store.key_event(key(&c.to_string()));
        }
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["big.txt"], "{names:?}");

        let preview = store.picker_preview();
        // The first page: exactly 32 lines, all from the file head, and
        // nothing from past the byte cap.
        assert_eq!(preview.lines().count(), 32);
        assert!(preview.starts_with("xxxxxxxxxx"));
        assert!(!preview.contains("HUGE-FILE-MARKER"));
    }

    #[test]
    fn picker_query_backspace_edits_query() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_find_file();

        store.key_event(key("m"));
        store.key_event(key("a"));
        assert_eq!(store.picker_query(), "ma");
        store.key_event(key("Backspace"));
        assert_eq!(store.picker_query(), "m");
        // C-h (control-h) edits the query the same way.
        store.key_event(key("a"));
        store.key_event(key("C-h"));
        assert_eq!(store.picker_query(), "m");

        // Backspace empties the query (all candidates come back)…
        store.key_event(key("Backspace"));
        assert_eq!(store.picker_query(), "");
        // …and a Backspace on the empty query is a no-op (stays open).
        store.key_event(key("Backspace"));
        assert!(store.picker_open());
        assert_eq!(store.picker_query(), "");
    }

    #[test]
    fn find_file_without_project_explains_itself() {
        let dir = tempfile::tempdir().unwrap(); // no markers → no project
        let mut store = store(dir.path());
        store.key_event(key("C-x"));
        store.key_event(key("C-f"));
        assert!(!store.picker_open());
        assert!(store.message.contains("no project"));
    }

    #[test]
    fn palette_navigation_and_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.open_palette();
        assert_eq!(store.picker_count().0, 108);

        // Shipped UI path (M-x, Down, Up): Up must wrap-decrement, not
        // reflect — prev(1) is 0, not 8.
        store.key_event(key("DOWN"));
        assert_eq!(store.picker_selected(), 1);
        store.key_event(key("UP"));
        assert_eq!(store.picker_selected(), 0);

        // Mid-list: Up from an interior index decrements by exactly one.
        store.picker_select_next();
        store.picker_select_next(); // 0 -> 2
        store.picker_select_prev();
        assert_eq!(store.picker_selected(), 1);

        // Wrap at top: Up at index 0 lands on the last candidate.
        store.picker_select_prev(); // 1 -> 0
        store.picker_select_prev();
        assert_eq!(store.picker_selected(), 107);

        // C-p goes through the same wrap-decrement path as Up.
        store.key_event(key("C-p"));
        assert_eq!(store.picker_selected(), 106);

        // RET runs the candidate at the selected index (the last command —
        // a no-op on *scratch*, so just a message).
        store.key_event(key("RET"));
        assert!(!store.picker_open());
        assert!(!store.quit);
    }

    #[test]
    fn palette_ret_runs_selected_command() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.open_palette();
        // Seed order: quit is first.
        store.key_event(key("RET"));
        assert!(!store.picker_open());
        assert!(store.quit);
    }

    #[test]
    fn palette_c_g_closes() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.open_palette();
        store.key_event(key("C-g"));
        assert!(!store.picker_open());
        assert_eq!(store.message, "cancel");
    }

    #[test]
    fn palette_query_filters_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.open_palette();
        assert_eq!(store.picker_count().0, 108);

        store.key_event(key("q"));
        store.key_event(key("u"));
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.display.as_str())
            .collect();
        assert_eq!(names, vec!["quit"], "{names:?}");

        // Backspace now edits the query (carry-over from the issue 01
        // review): "qu" -> "q".
        store.key_event(key("Backspace"));
        assert!(store.picker_open());
        assert_eq!(store.picker_query(), "q");
    }

    #[test]
    fn menu_derives_exactly_the_view_bindings() {
        // Anti-drift: the menu's reachable leaf commands == the view+global
        // bindings' commands (derived, not hand-written; nothing dropped).
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path()); // Buffer view
        let bound: std::collections::HashSet<String> = s
            .menu_bindings()
            .into_iter()
            .map(|(_, c)| c)
            .collect();
        let menu_leaves = collect_menu_leaves(&s, s.menu_path());
        assert_eq!(
            menu_leaves, bound,
            "menu leaves must exactly equal the keymap × registry bindings"
        );
    }

    #[test]
    fn menu_root_lists_buffer_leaves_and_prefixes() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.open_scratch(); // 06a: boot is home; the test asserts BUFFER-view leaves
        s.open_menu();
        let entries = s.menu_entries();
        // A single-key leaf: j → scroll-line-down (Buffer view).
        assert!(
            entries
                .iter()
                .any(|e| e.key == key("j") && e.command.as_deref() == Some("scroll-line-down")),
            "j leaf missing: {entries:?}"
        );
        // A prefix: C-c (the projectile family) is a prefix, not a leaf.
        assert!(
            entries.iter().any(|e| e.key == key("C-c") && e.is_prefix),
            "C-c prefix missing: {entries:?}"
        );
        // The menu's own opener ? is a leaf (open-transient-menu).
        assert!(
            entries
                .iter()
                .any(|e| e.key == key("?") && e.command.as_deref() == Some("open-transient-menu")),
            "? leaf missing: {entries:?}"
        );
    }

    #[test]
    fn menu_prefix_descent_into_projectile() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.open_menu();
        // Descend into C-c p (projectile family).
        let path: crate::app::keymap::KeySeq = vec![key("C-c"), key("p")];
        let entries = s.menu_entries_for_path(&path);
        assert!(
            entries
                .iter()
                .any(|e| e.key == key("f") && e.command.as_deref() == Some("find-file")),
            "C-c p f missing: {entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|e| e.key == key("p") && e.command.as_deref() == Some("switch-project")),
            "C-c p p missing: {entries:?}"
        );
        // s is a prefix here (C-c p s s).
        assert!(
            entries.iter().any(|e| e.key == key("s") && e.is_prefix),
            "C-c p s prefix missing: {entries:?}"
        );
    }

    #[test]
    fn menu_leaf_executes_and_closes() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.open_menu();
        assert!(s.menu_open());
        // M-x is a leaf (open-palette) at the root.
        s.key_event(key("M-x"));
        assert!(!s.menu_open(), "leaf must close the menu");
        assert!(s.picker_open(), "M-x must open the palette");
    }

    #[test]
    fn menu_prefix_key_descends_not_executes() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.open_menu();
        s.key_event(key("C-c"));
        assert!(s.menu_open(), "prefix keeps the menu open");
        assert_eq!(s.menu_path().len(), 1, "path descends by one");
        assert_eq!(s.menu_path()[0], key("C-c"));
    }

    #[test]
    fn menu_c_g_closes() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.open_menu();
        s.key_event(key("C-g"));
        assert!(!s.menu_open());
        assert_eq!(s.message, "cancel");
    }

    #[test]
    fn menu_swallows_non_listed_keys() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.open_menu();
        // z is not bound in the Buffer view nor global → not in the menu.
        s.key_event(key("z"));
        assert!(s.menu_open(), "non-listed key must not close the menu");
        assert!(
            s.message.is_empty(),
            "no unbound-key echo: got `{}`",
            s.message
        );
    }

    #[test]
    fn menu_h_opens_in_magit_view() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.push_view(ViewId::MagitStatus);
        s.key_event(key("h"));
        assert!(s.menu_open(), "h must open the menu in magit views");
        let entries = s.menu_entries();
        assert!(
            entries
                .iter()
                .any(|e| e.key == key("k") && e.command.as_deref() == Some("magit-discard")),
            "k leaf missing in magit menu: {entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|e| e.key == key("s") && e.command.as_deref() == Some("magit-stage")),
            "s leaf missing in magit menu: {entries:?}"
        );
    }


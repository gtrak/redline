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

        // picker-density: the recents row is name-first — the file name
        // left, the path right; display (the match target) stays the path.
        store.key_event(key("C-c"));
        store.key_event(key("p"));
        store.key_event(key("e"));
        assert!(store.picker_open(), "C-c p e opens the recents picker");
        let c = store.picker_filtered().iter().find(|(c, _)| c.name == "src/main.rs").map(|(c, _)| c.clone()).unwrap();
        assert_eq!(c.display, "src/main.rs", "{c:?}");
        assert_eq!(c.label, "main.rs", "the file name is the label: {c:?}");
        assert_eq!(c.detail, "src/main.rs", "the path is the detail: {c:?}");
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
    fn find_file_candidates_are_name_first() {
        // picker-density: find-file rows split into name (the file name)
        // and detail (the path); `display` — the nucleo match target —
        // stays the full path.
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_find_file();
        let c = store
            .picker_filtered()
            .iter()
            .find(|(c, _)| c.name == "src/main.rs")
            .map(|(c, _)| c.clone())
            .unwrap();
        assert_eq!(c.display, "src/main.rs", "matching stays on the full path: {c:?}");
        assert_eq!(c.label, "main.rs", "the file name is the label: {c:?}");
        assert_eq!(c.detail, "src/main.rs", "the path is the detail: {c:?}");
        // A root-level file has no directory component, so the name IS
        // the path: the detail stays empty (the row would otherwise draw
        // `Cargo.toml …………… Cargo.toml`) and display (the match target)
        // stays the path. This pin fails if the name-first split ever
        // duplicates a root-level file's name into both cells.
        let root = store
            .picker_filtered()
            .iter()
            .find(|(c, _)| c.name == "Cargo.toml")
            .map(|(c, _)| c.clone())
            .unwrap();
        assert_eq!(root.display, "Cargo.toml", "matching stays on the full path: {root:?}");
        assert_eq!(root.label, "Cargo.toml", "{root:?}");
        assert!(
            root.detail.is_empty(),
            "root-level: no detail — the name must not repeat: {root:?}"
        );
    }

    #[test]
    fn root_level_recent_rows_carry_no_detail() {
        // Same requirement on the RECENTS path: a root-level file in the
        // recents picker must not draw the name twice. detail stays
        // empty (single left-anchored name); display (the match target)
        // stays the full path.
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        // Visit the root-level file through the find-file picker so it
        // lands in recents.
        store.open_find_file();
        for c in "Cargo.toml".chars() {
            store.key_event(key(&c.to_string()));
        }
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["Cargo.toml"], "{names:?}");
        store.key_event(key("RET"));
        assert!(!store.picker_open());
        store.key_event(key("C-c"));
        store.key_event(key("p"));
        store.key_event(key("e"));
        assert!(store.picker_open(), "C-c p e opens the recents picker");
        let c = store
            .picker_filtered()
            .iter()
            .find(|(c, _)| c.name == "Cargo.toml")
            .map(|(c, _)| c.clone())
            .unwrap();
        assert_eq!(c.display, "Cargo.toml", "matching stays on the full path: {c:?}");
        assert_eq!(c.label, "Cargo.toml", "{c:?}");
        assert!(c.detail.is_empty(), "root-level recent: the name must not repeat: {c:?}");
    }

    #[test]
    fn palette_rows_stay_display_only() {
        // picker-density judgement call: a palette row's whole content is
        // one command identifier (no kind or path context to right-align),
        // so the row keeps the single left-anchored `display` shape.
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.open_palette();
        let c = &store.picker_filtered()[0].0;
        assert!(
            c.label.is_empty() && c.detail.is_empty(),
            "the palette stays display-only: {c:?}"
        );
        assert_eq!(c.display, c.name, "{c:?}");
    }

    #[test]
    fn palette_navigation_and_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.open_palette();
        // annotations-fold-visual: the seed registry now carries annotate-fold
        // (118 → 119; annotate-hide / annotate-show were removed — the fold
        // is a single `C-c a h` toggle → annotate-toggle). plan 016 issue 04
        // added `redo` → 120.
        assert_eq!(store.picker_count().0, 120);

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
        let total = store.picker_count().0;
        assert_eq!(store.picker_selected(), total - 1);

        // C-p goes through the same wrap-decrement path as Up.
        store.key_event(key("C-p"));
        assert_eq!(store.picker_selected(), total - 2);

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
        // annotations-fold-visual: the seed registry now carries annotate-fold
        // (118 → 119; annotate-hide / annotate-show were removed — the fold
        // is a single `C-c a h` toggle → annotate-toggle). plan 016 issue 04
        // added `redo` → 120.
        assert_eq!(store.picker_count().0, 120);

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

    // ── 015-01: the annotations picker ────────────────────────────────

    /// Fixture: a notes document with two valid records and TWO Raw
    /// entries (a stray line INSIDE the structured section, before the
    /// first record, and a malformed record block) — the Raw content
    /// must never surface as a candidate. Records: `src/main.rs` line 2
    /// (1-based) and `README.md` line 1.
    fn project_with_annotations(dir: &std::path::Path) {
        project_with_files(dir);
        std::fs::write(
            dir.join(".redline-notes.md"),
            "# Notes\n\n<!-- redline-annotations:begin -->\nA stray line inside the section.\n[annotation]\npath: src/main.rs\nline: 1\nanchor:     println!(\"hi\");\nnote: fix the off-by-one\n[annotation]\npath: src/main.rs\nline: 9\n(not a record — the required fields are missing)\n[annotation]\npath: README.md\nline: 0\nanchor: # readme\nnote: top of the docs\norphaned: true\n<!-- redline-annotations:end -->\n",
        )
        .unwrap();
    }

    #[test]
    fn annotations_picker_lists_records_excluding_raw_in_document_order() {
        let dir = tempfile::tempdir().unwrap();
        project_with_annotations(dir.path());
        let mut store = store(dir.path());
        store.open_annotations_picker();
        assert!(store.picker_open());
        assert_eq!(store.picker_kind(), Some(PickerKind::Annotations));
        assert_eq!(store.picker_prompt(), "Annotations: ");
        // The count row: 2 candidates, 2 total — the Raw entries (the
        // stray line and the malformed block) contribute nothing.
        assert_eq!(store.picker_count(), (2, 2));
        // Row shape + document order: README.md:1, then src/main.rs:2
        // (file, then line — not recency, not the notes doc's append
        // order, which puts src/main.rs first).
        let rows: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| (c.name.as_str(), c.detail.as_str()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("top of the docs", "README.md:1"),
                ("fix the off-by-one", "src/main.rs:2"),
            ]
        );
        // Raw exclusion: neither the stray line nor the malformed block
        // may surface as a candidate — in name, label, or detail.
        for (c, _) in store.picker_filtered() {
            assert!(!c.name.contains("stray"), "raw stray line surfaced: {c:?}");
            assert!(
                !c.name.contains("not a record"),
                "malformed record surfaced: {c:?}"
            );
            assert!(!c.detail.contains("src/main.rs:10"), "the malformed record's line surfaced: {c:?}");
            assert!(!c.label.contains("stray"), "raw stray line surfaced in the label: {c:?}");
        }
        // Row shape: name/label = the annotation text, detail = path:line
        // (1-based, the location pickers' display convention), display
        // carries both (the nucleo match target).
        let fix = store
            .picker_filtered()
            .iter()
            .find(|(c, _)| c.name == "fix the off-by-one")
            .map(|(c, _)| c.clone())
            .unwrap();
        assert_eq!(fix.label, "fix the off-by-one");
        assert_eq!(fix.detail, "src/main.rs:2");
        assert_eq!(fix.display, "fix the off-by-one  src/main.rs:2");
        assert_eq!(fix.category, "annotation");
        // Inherited filtering: the query narrows the list, the total
        // stays the unfiltered count.
        store.key_event(key("o"));
        store.key_event(key("f"));
        store.key_event(key("f"));
        assert_eq!(store.picker_count(), (1, 2));
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["fix the off-by-one"], "{names:?}");
    }

    #[test]
    fn annotations_picker_ret_lands_on_the_annotation_line() {
        let dir = tempfile::tempdir().unwrap();
        project_with_annotations(dir.path());
        let mut store = store(dir.path());
        store.open_annotations_picker();
        // Select "fix the off-by-one" (src/main.rs line 2, 1-based).
        // (A bare `j` would extend the filter query; C-n is the nav key.)
        store.key_event(key("C-n"));
        assert_eq!(store.picker_selected(), 1);
        // The preview shows the file around the annotation line.
        assert!(
            store.picker_preview().contains("println!"),
            "preview: {}",
            store.picker_preview()
        );
        store.key_event(key("RET"));
        assert!(!store.picker_open());
        assert_eq!(store.view_name_display(), "src/main.rs");
        assert_eq!(
            store.point_line(),
            1,
            "the point must land on the annotation's 0-based line"
        );
        assert!(
            store.message.contains("jumped to src/main.rs:2"),
            "{}",
            store.message
        );
    }

    #[test]
    fn annotations_picker_d_deletes_the_selected_annotation() {
        let dir = tempfile::tempdir().unwrap();
        project_with_annotations(dir.path());
        let mut store = store(dir.path());
        store.open_annotations_picker();
        store.key_event(key("C-n")); // select "fix the off-by-one" (src/main.rs:2)
        store.key_event(key("d"));
        assert!(store.picker_open(), "d must not close the picker");
        // The list recomputes on the same (empty) query: 1 of 1.
        assert_eq!(store.picker_count(), (1, 1));
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["top of the docs"], "{names:?}");
        assert!(
            store.message.contains("deleted annotation"),
            "{}",
            store.message
        );
        // The record is gone; the Raw entries survive the round trip
        // (2 Raw + the remaining record = 3 entries).
        let entries = store.notes_doc.entries.clone();
        assert_eq!(entries.len(), 3, "one record removed, Raw intact: {entries:?}");
        let content = std::fs::read_to_string(dir.path().join(".redline-notes.md"))
            .unwrap();
        assert!(!content.contains("fix the off-by-one"));
        assert!(content.contains("A stray line inside the section."));
    }

    // ── annot-picker-repair: several records on ONE line ─────────────

    /// Fixture: TWO annotation records on the SAME (path, line) at two
    /// different columns (record 1 on `a` at char col 8, record 2 on `b` at
    /// char col 12 of `    let a = b;`). This is the shape the `A` key path
    /// now produces (per-symbol, not per-line), so the two rows share the
    /// detail `src/perc.rs:2` and are distinguishable ONLY by the record's
    /// own `col` — the identity the picker used to drop.
    fn project_with_two_annotations_on_one_line(dir: &std::path::Path) {
        project_with_files(dir);
        std::fs::write(dir.join("src/perc.rs"), "fn f() {\n    let a = b;\n}\n").unwrap();
        std::fs::write(
            dir.join(".redline-notes.md"),
            "# Notes\n\n<!-- redline-annotations:begin -->\n[annotation]\npath: src/perc.rs\nline: 1\ncol: 8\nanchor:     let a = b;\nnote: note a\n[annotation]\npath: src/perc.rs\nline: 1\ncol: 12\nanchor:     let a = b;\nnote: note b\n<!-- redline-annotations:end -->\n",
        )
        .unwrap();
    }

    #[test]
    fn annotations_picker_d_deletes_the_selected_record_not_the_lines_first() {
        let dir = tempfile::tempdir().unwrap();
        project_with_two_annotations_on_one_line(dir.path());
        let mut store = store(dir.path());
        store.open_annotations_picker();
        assert_eq!(store.picker_count(), (2, 2));
        // Both rows carry the SAME detail; only the record's own `col`
        // distinguishes them (this is what the picker used to lose).
        let rows: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| (c.name.as_str(), c.detail.as_str(), c.ann_col))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("note a", "src/perc.rs:2", Some(8)),
                ("note b", "src/perc.rs:2", Some(12)),
            ]
        );
        // Select the SECOND row (note b, col 12) and delete it.
        store.key_event(key("C-n"));
        assert_eq!(store.picker_selected(), 1);
        store.key_event(key("d"));
        assert!(store.picker_open(), "d must not close the picker");
        // The list recomputes: only the col-8 sibling survives. Pre-fix the
        // line-keyed delete resolved the FIRST record, so this deleted
        // "note a" and left "note b" behind (the gate's FAIL).
        assert_eq!(store.picker_count(), (1, 1));
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["note a"], "the sibling must survive: {names:?}");
        // The echo NAMES the second record, not the first.
        assert!(
            store.message.contains("deleted annotation: note b"),
            "the echo must name the deleted (second) record: {}",
            store.message
        );
        let content = std::fs::read_to_string(dir.path().join(".redline-notes.md")).unwrap();
        assert!(content.contains("note a"), "the sibling record must survive: {content}");
        assert!(!content.contains("note b"), "the second record must be gone: {content}");
    }

    #[test]
    fn annotations_picker_ret_lands_on_the_selected_records_own_col() {
        let dir = tempfile::tempdir().unwrap();
        project_with_two_annotations_on_one_line(dir.path());
        let mut store = store(dir.path());
        store.open_annotations_picker();
        // Select the SECOND row (note b, col 12); RET must land on THAT
        // record's column, not the line's first record (col 8) — the gate's
        // FAIL landed where `A` pre-filled the FIRST note.
        store.key_event(key("C-n"));
        assert_eq!(store.picker_selected(), 1);
        store.key_event(key("RET"));
        assert!(!store.picker_open());
        assert_eq!(store.view_name_display(), "src/perc.rs");
        assert_eq!(store.point_line(), 1, "the point must land on the annotation's 0-based line");
        assert_eq!(
            store.point_col(),
            12,
            "the point must land on the SECOND record's col, not the first's (8)"
        );
        assert!(
            store.message.contains("jumped to src/perc.rs:2"),
            "{}",
            store.message
        );
        // A following `A` pre-fills the SECOND note (the record at point),
        // not the first — the user's original report.
        store.key_event(key("A"));
        assert!(store.note_prompt_active());
        assert_eq!(
            store.note_prompt_input(),
            "note b",
            "A must pre-fill the second note, not the first"
        );
    }

    #[test]
    fn annotations_picker_without_project_explains_itself() {
        let dir = tempfile::tempdir().unwrap(); // no markers → no project
        let mut store = store(dir.path());
        store.key_event(key("C-c"));
        store.key_event(key("n"));
        store.key_event(key("a"));
        assert!(!store.picker_open());
        assert!(store.message.contains("no project"), "{}", store.message);
    }

    #[test]
    fn annotations_picker_empty_notes_document_opens_empty() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path()); // no .redline-notes.md
        let mut store = store(dir.path());
        store.open_annotations_picker();
        // An empty list is acceptable (0/0) — the picker must not
        // error, and RET on an empty list must stay safe.
        assert!(store.picker_open());
        assert_eq!(store.picker_count(), (0, 0));
        store.key_event(key("RET"));
        assert!(!store.picker_open());
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


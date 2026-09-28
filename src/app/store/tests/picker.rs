use super::*;
use nucleo_matcher::{Config, Matcher};
use crate::app::store::narrowing::narrow;
use crate::nav::index::build_index;

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
        // order, which puts src/main.rs first). name is the machine key
        // (path:line, the location pickers' convention — landing and
        // deletion parse it); detail is the display location (identical
        // here: both lines host a single record, so no same-line
        // disambiguator rides it).
        let rows: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| (c.name.as_str(), c.detail.as_str()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("README.md:1", "README.md:1"),
                ("src/main.rs:2", "src/main.rs:2"),
            ]
        );
        // Raw exclusion: neither the stray line nor the malformed block
        // may surface as a candidate — in display (the match target =
        // note text + location), label, or detail.
        for (c, _) in store.picker_filtered() {
            assert!(!c.display.contains("stray"), "raw stray line surfaced: {c:?}");
            assert!(
                !c.label.contains("not a record"),
                "malformed record surfaced: {c:?}"
            );
            assert!(!c.detail.contains("src/main.rs:10"), "the malformed record's line surfaced: {c:?}");
            assert!(!c.label.contains("stray"), "raw stray line surfaced in the label: {c:?}");
        }
        // Row shape: name = path:line (the machine key — NOT the note
        // text), label = the annotation text, detail = path:line (1-based,
        // the location pickers' display convention; these legacy records
        // carry no col key → col 0, and their lines host a single record,
        // so the detail stays the plain form), display carries both (the
        // nucleo match target), ann_col = the record's own col.
        let fix = store
            .picker_filtered()
            .iter()
            .find(|(c, _)| c.label == "fix the off-by-one")
            .map(|(c, _)| c.clone())
            .unwrap();
        assert_eq!(fix.name, "src/main.rs:2");
        assert_eq!(fix.label, "fix the off-by-one");
        assert_eq!(fix.detail, "src/main.rs:2");
        assert_eq!(fix.display, "fix the off-by-one  src/main.rs:2");
        assert_eq!(fix.ann_col, Some(0), "no col key in the record → col 0");
        assert_eq!(fix.category, "annotation");
        // Inherited filtering: the query narrows the list (the match
        // target is the display — note text + location), the total stays
        // the unfiltered count.
        store.key_event(key("o"));
        store.key_event(key("f"));
        store.key_event(key("f"));
        assert_eq!(store.picker_count(), (1, 2));
        let labels: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.label.as_str())
            .collect();
        assert_eq!(labels, vec!["fix the off-by-one"], "{labels:?}");
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
        // The preview shows the file around the annotation line (parsed
        // from the name machine key — path:line).
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
        let labels: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.label.as_str())
            .collect();
        assert_eq!(labels, vec!["top of the docs"], "{labels:?}");
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
    /// now produces (per-symbol, not per-line): the two rows share the
    /// name machine key `src/perc.rs:2`, and the record's own `col` is the
    /// identity the detail renders so the rows read apart on screen.
    fn project_with_two_annotations_on_one_line(dir: &std::path::Path) {
        project_with_files(dir);
        std::fs::write(dir.join("src/perc.rs"), "fn f() {\n    let a = b;\n}\n").unwrap();
        std::fs::write(
            dir.join(".redline-notes.md"),
            "# Notes\n\n<!-- redline-annotations:begin -->\n[annotation]\npath: src/perc.rs\nline: 1\ncol: 8\nanchor:     let a = b;\nnote: note a\n[annotation]\npath: src/perc.rs\nline: 1\ncol: 12\nanchor:     let a = b;\nnote: note b\n<!-- redline-annotations:end -->\n",
        )
        .unwrap();
    }

    /// Fixture: FIVE records across TWO same-line sibling groups, covering
    /// the row shapes the disambiguation must handle: (1) records WITH a
    /// syntax anchor (the symbol name rides the detail), (2) records with
    /// `syntax_name` ABSENT (text-rule anchors — only the col rides), (3)
    /// a (path, line) hosting SEVERAL records (the case that read apart
    /// pre-fix), and (4) a sibling record at col 0 (a pin that only covers
    /// column-0 would leave the indented siblings unexercised). Line 0
    /// (`fn f() {`): col 0 = `fn` (keyword — no identifier-ish answer →
    /// text rule), col 3 = the function name `f`. Line 1 (`    let a = b;`):
    /// col 4 = `let` (keyword → text rule), col 8 = `a`, col 12 = `b`.
    fn project_with_mixed_same_line_annotations(dir: &std::path::Path) {
        project_with_files(dir);
        std::fs::write(dir.join("src/perc.rs"), "fn f() {\n    let a = b;\n}\n").unwrap();
        std::fs::write(
            dir.join(".redline-notes.md"),
            "# Notes\n\n<!-- redline-annotations:begin -->\n[annotation]\npath: src/perc.rs\nline: 0\ncol: 0\nanchor: fn f() {\nnote: n0\n[annotation]\npath: src/perc.rs\nline: 0\ncol: 3\nanchor: fn f() {\nnote: n3\nsyntax_kind: identifier\nsyntax_name: f\n[annotation]\npath: src/perc.rs\nline: 1\ncol: 4\nanchor:     let a = b;\nnote: n4\n[annotation]\npath: src/perc.rs\nline: 1\ncol: 8\nanchor:     let a = b;\nnote: n8\nsyntax_kind: identifier\nsyntax_name: a\n[annotation]\npath: src/perc.rs\nline: 1\ncol: 12\nanchor:     let a = b;\nnote: n12\nsyntax_kind: identifier\nsyntax_name: b\n<!-- redline-annotations:end -->\n",
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
        // Both rows share the name machine key (path:line); the DETAIL
        // now carries the record's own cell so the rows read apart on
        // screen — pre-fix the details were both `src/perc.rs:2`.
        let rows: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| (c.name.as_str(), c.detail.as_str(), c.ann_col))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("src/perc.rs:2", "src/perc.rs:2:8", Some(8)),
                ("src/perc.rs:2", "src/perc.rs:2:12", Some(12)),
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
        let labels: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.label.as_str())
            .collect();
        assert_eq!(labels, vec!["note a"], "the sibling must survive: {labels:?}");
        // The survivor's detail drops back to the plain `path:line` form
        // (a lone record on its line needs no disambiguator).
        let survivor = store.picker_filtered().first().map(|(c, _)| c.clone()).unwrap();
        assert_eq!(survivor.detail, "src/perc.rs:2", "lone record: plain detail: {survivor:?}");
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

    /// The acceptance pin: TWO records on the same line must render
    /// DISTINCT location cells on screen. Fails on the pre-fix code, where
    /// both rows' detail was the shared `src/perc.rs:2` (the rows read
    /// apart only through the note text — exactly the residual the user
    /// reported). The name machine key stays the shared `path:line` — the
    /// landing address does not move.
    #[test]
    fn annotations_picker_same_line_rows_carry_the_record_cell() {
        let dir = tempfile::tempdir().unwrap();
        project_with_two_annotations_on_one_line(dir.path());
        let mut store = store(dir.path());
        store.open_annotations_picker();
        let rows: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.clone())
            .collect();
        let details: Vec<_> = rows.iter().map(|c| c.detail.as_str()).collect();
        assert_eq!(
            details,
            vec!["src/perc.rs:2:8", "src/perc.rs:2:12"],
            "the two rows must read apart: {rows:?}"
        );
        assert_ne!(details[0], details[1], "same-line rows share no location cell");
        // The disambiguator rides the DISPLAY too (the nucleo match
        // target): a query can filter by the record's cell.
        assert_eq!(rows[0].display, "note a  src/perc.rs:2:8");
        assert_eq!(rows[1].display, "note b  src/perc.rs:2:12");
        // The machine key is still the shared path:line (landing
        // unchanged — the address never learned the display's suffix).
        assert_eq!(rows[0].name, "src/perc.rs:2");
        assert_eq!(rows[1].name, "src/perc.rs:2");
    }

    /// The row shapes the same-line disambiguator must cover: the symbol
    /// name rides the detail when the record has a syntax anchor, the bare
    /// `:col` when it does not (text-rule anchor — the keyword cells), and
    /// col 0 renders like any other cell (a pin that only covers column-0
    /// leaves the indented siblings unexercised — this fixture spans both
    /// line 0 and line 1, col 0 and indented cols).
    #[test]
    fn annotations_picker_same_line_shapes_cover_symbol_text_rule_and_col_zero() {
        let dir = tempfile::tempdir().unwrap();
        project_with_mixed_same_line_annotations(dir.path());
        let mut store = store(dir.path());
        store.open_annotations_picker();
        assert_eq!(store.picker_count(), (5, 5));
        // (path, line, col) order: left-to-right on each line.
        let rows: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| (c.label.as_str(), c.detail.as_str(), c.ann_col))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("n0", "src/perc.rs:1:0", Some(0)), // text rule, col 0
                ("n3", "src/perc.rs:1:3 (f)", Some(3)), // symbol anchor
                ("n4", "src/perc.rs:2:4", Some(4)), // text rule
                ("n8", "src/perc.rs:2:8 (a)", Some(8)), // symbol anchor
                ("n12", "src/perc.rs:2:12 (b)", Some(12)), // symbol anchor
            ]
        );
        // Every same-line sibling group reads apart: the five details
        // are pairwise distinct.
        let details: Vec<&str> = rows.iter().map(|r| r.1).collect();
        for d in &details {
            assert_eq!(
                details.iter().filter(|o| *o == d).count(),
                1,
                "detail {d:?} must be unique among the rows: {rows:?}"
            );
        }
        // The col-0 sibling (n0) and its col-3 sibling (n3) read apart:
        // the col-0 cell renders `:0`, never dropped as "no column".
        assert!(rows[0].1.ends_with(":1:0"));
    }

    /// Landing stays EXACT on the new detail shapes: RET on the symbol row
    /// (`src/perc.rs:2:8 (a)`) lands on THAT record's cell (line 1, col 8 —
    // the detail's `:8 (a)` is display-only, never part of the address),
    /// and RET on the col-0 text-rule row lands at (line 0, col 0). A
    /// following `A` pre-fills the selected record's note, not the
    /// sibling's.
    #[test]
    fn annotations_picker_ret_lands_exact_on_each_same_line_row() {
        let dir = tempfile::tempdir().unwrap();
        project_with_mixed_same_line_annotations(dir.path());
        let mut store = store(dir.path());
        store.open_annotations_picker();
        // Row 3 = `n8` (`src/perc.rs:2:8 (a)`): land on (line 1, col 8).
        for _ in 0..3 {
            store.key_event(key("C-n"));
        }
        assert_eq!(store.picker_selected(), 3);
        store.key_event(key("RET"));
        assert!(!store.picker_open());
        assert_eq!(store.view_name_display(), "src/perc.rs");
        assert_eq!(store.point_line(), 1, "the point must land on line 1 (0-based)");
        assert_eq!(store.point_col(), 8, "the point must land on the SELECTED record's col");
        // `A` pre-fills the record at point (the col-8 record, `n8`).
        store.key_event(key("A"));
        assert!(store.note_prompt_active());
        assert_eq!(store.note_prompt_input(), "n8", "A must pre-fill the second record, not a sibling");
        store.key_event(key("ESC"));
        // Re-open; row 0 = `n0` (`src/perc.rs:1:0`): the col-0 landing.
        store.key_event(key("C-c"));
        store.key_event(key("n"));
        store.key_event(key("a"));
        assert!(store.picker_open());
        store.key_event(key("RET"));
        assert!(!store.picker_open());
        assert_eq!(store.point_line(), 0, "the col-0 row must land on line 0");
        assert_eq!(store.point_col(), 0, "the col-0 row must land at col 0, not the line's next record");
    }

    /// `d` on a SYMBOL row (the detail's ` (a)` suffix must not leak into
    /// the deletion's (path, line) parse) removes the SELECTED record and
    /// recomputes: the line's other siblings survive, and the survivor's
    /// detail keeps its own cell.
    #[test]
    fn annotations_picker_d_deletes_the_selected_symbol_row_not_its_siblings() {
        let dir = tempfile::tempdir().unwrap();
        project_with_mixed_same_line_annotations(dir.path());
        let mut store = store(dir.path());
        store.open_annotations_picker();
        // Row 3 = `n8` (`src/perc.rs:2:8 (a)`); delete it.
        for _ in 0..3 {
            store.key_event(key("C-n"));
        }
        store.key_event(key("d"));
        assert!(store.picker_open(), "d must not close the picker");
        assert_eq!(store.picker_count(), (4, 4));
        assert!(
            store.message.contains("deleted annotation: n8"),
            "the echo must name the deleted (symbol) record: {}",
            store.message
        );
        let rows: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| (c.label.as_str(), c.detail.as_str()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("n0", "src/perc.rs:1:0"),
                ("n3", "src/perc.rs:1:3 (f)"),
                ("n4", "src/perc.rs:2:4"),
                ("n12", "src/perc.rs:2:12 (b)"),
            ],
            "the sibling records must survive with their own cells: {rows:?}"
        );
        let content = std::fs::read_to_string(dir.path().join(".redline-notes.md")).unwrap();
        assert!(!content.contains("note: n8"), "the selected record must be gone: {content}");
        assert!(content.contains("note: n12"), "the col-12 sibling must survive: {content}");
    }

    /// Rendered-canvas quote of the same-line rows AS THEY APPEAR (the
    /// claim is about the screen, not the struct): the two rows are drawn
    /// by the picker's own row renderer at the real 80-col layout (52-col
    /// candidate column with preview) and read back cell by cell. The
    /// disambiguator rides the detail's tail, right-aligned at the column
    /// edge — pre-fix both rows quoted as `… src/perc.rs:2 …`.
    #[test]
    fn annotations_picker_same_line_rows_render_distinguishably() {
        use crate::theme;
        use crate::ui::picker::{draw_candidate_row, picker_column_layout};
        let dir = tempfile::tempdir().unwrap();
        project_with_two_annotations_on_one_line(dir.path());
        let mut store = store(dir.path());
        store.open_annotations_picker();
        let rows: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.clone())
            .collect();
        assert_eq!(rows.len(), 2);
        let w = 80usize;
        let (cand_w, _, _) = picker_column_layout(w, true); // preview active: 52
        let face = theme::current().list_item;
        let mut canvas = iocraft::Canvas::new(w, 2);
        {
            let mut sv = canvas.subview_mut(0, 0, 0, 0, w, 2);
            draw_candidate_row(&mut sv, 0, cand_w, &rows[0], face, false);
            draw_candidate_row(&mut sv, 1, cand_w, &rows[1], face, false);
        }
        let quote = |y: usize| {
            (0..w)
                .map(|x| canvas.cell(x, y).unwrap().text().unwrap_or(" "))
                .collect::<String>()
        };
        let row0 = quote(0);
        let row1 = quote(1);
        // The rows AS THEY APPEAR: label left, the record's own cell
        // right-aligned at the candidate column's right edge (col 52).
        let expected0 = format!(" note a{}src/perc.rs:2:8{}", " ".repeat(31), " ".repeat(27));
        let expected1 = format!(" note b{}src/perc.rs:2:12{}", " ".repeat(30), " ".repeat(27));
        assert_eq!(row0, expected0, "row 0 as rendered: {row0:?}");
        assert_eq!(row1, expected1, "row 1 as rendered: {row1:?}");
        assert_ne!(row0, row1, "the same-line rows must read apart on screen");
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


    /// issue-clipboard-and-selection part 3: the picker filter accepts a
    /// pasted non-ASCII query char (the printable() widening is monotonic
    /// — every text surface accepts MORE, the picker's nucleo query just
    /// narrows on real text now).
    #[test]
    fn find_file_picker_accepts_a_pasted_nonascii_query_char() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/café.rs", "fn c() {}\n");
        s.open_find_file();
        for c in "café".chars() {
            s.key_event(crate::app::keymap::Key::new(
                crate::app::keymap::KeyCode::Char(c),
            ));
        }
        let names: Vec<String> =
            s.picker_filtered().iter().map(|(c, _)| c.name.clone()).collect();
        assert_eq!(
            names,
            vec!["src/café.rs".to_string()],
            "the pasted é must reach the filter (before the widening it was dropped, 'caf' would match more)"
        );
    }

    // ── plan 018 issue 01: the shared-narrowing-mechanism discrimination pins ──

    /// A store with candidates for EVERY `PickerKind` — the fixture for the
    /// narrowing cross-check pin (plan 018 issue 01). Every kind's
    /// `candidates_for` is non-empty so the per-kind "picker output == shared
    /// core" comparison is exercised, not vacuous. The tempdirs are leaked
    /// (OS-cleaned on exit) so the paths the store holds stay valid.
    fn rich_picker_store() -> AppStore {
        let dir = tempfile::tempdir().unwrap();
        // A git project (committed files) — the walk + index root.
        git_repo_init(dir.path(), "Test", "test@example.com", true);
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "fn alpha() {}\nfn beta() {}\nimpl SomeTrait for T {\n    fn method(&self) {}\n}\nfn main() {\n    alpha();\n}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/other.rs"), "fn alpha() {}\nfn gamma() {}\n").unwrap();
        git_cli(dir.path(), &["add", "-A"], "Test", "test@example.com");
        git_cli(dir.path(), &["commit", "-q", "-m", "init"], "Test", "test@example.com");
        // A second branch (Branch kind) and a stash (Stash kind): dirty a
        // tracked file, then stash the change.
        git_cli(dir.path(), &["branch", "feature"], "Test", "test@example.com");
        std::fs::write(
            dir.path().join("src/other.rs"),
            "fn alpha() {}\nfn gamma() {}\n// dirty\n",
        )
        .unwrap();
        git_cli(dir.path(), &["stash", "push", "-m", "wip subject"], "Test", "test@example.com");

        let base = tempfile::tempdir().unwrap();
        let base_path = base.path().to_path_buf();
        std::mem::forget(base);
        let mut s = AppStore::at(dir.path(), base_path);
        s.project = Some(crate::model::project::Project::new(dir.path().to_path_buf()));

        // Prime the cached git repo handle (Branch, Stash kinds) and the
        // project file list (FindFile kind).
        s.ensure_git(dir.path().to_path_buf());
        s.ensure_files();

        // The symbol index (Symbols, Imenu, Xref, Impls kinds).
        let files_list = crate::model::files::FileList::build(dir.path()).unwrap();
        let index = build_index(dir.path(), &files_list.files, None);
        s.set_index(index);

        // A second registered project (Projects kind excludes the current).
        let p2 = tempfile::tempdir().unwrap();
        std::fs::write(p2.path().join("Cargo.toml"), "[package]\n").unwrap();
        s.project_store.registry.upsert(p2.path());
        std::mem::forget(p2);

        // Open buffers (Buffers, KillBuffer, Imenu) + a recent (RecentFiles).
        s.open_path("src/lib.rs");
        s.open_path("src/other.rs");
        s.record_recent("src/other.rs");

        // A note record (Annotations kind). `anchor` is a required field —
        // a record missing it is a Raw entry, never a candidate.
        std::fs::write(
            dir.path().join(".redline-notes.md"),
            "# Notes\n\n<!-- redline-annotations:begin -->\n[annotation]\npath: src/lib.rs\nline: 1\nanchor: fn alpha() {}\nnote: top of the lib\n<!-- redline-annotations:end -->\n",
        )
        .unwrap();

        // An ambiguous Xref lookup and an Impls lookup (two defs of `alpha`; one
        // impl of `SomeTrait`).
        s.xref_lookup_name = "alpha".to_string();
        s.impls_keys = vec!["SomeTrait".to_string()];

        std::mem::forget(dir);
        s
    }

    /// Pin C (PLAN 5.5, `0829ddd` cross-check): enumerate EVERY `PickerKind`
    /// from the production enumeration (`picker_kinds`, kept in lockstep with
    /// the enum + `candidates_for`) and assert each kind's query path re-derives
    /// its rows through the SHARED core — i.e. the picker's filtered rows equal
    /// `narrowing::narrow` on the same `candidates_for(kind)` and the same
    /// matcher (`picker_kind_uses_file_matcher`, the shared predicate). A kind
    /// whose query path diverges from the core (its own private filter) fails
    /// here, named; a new variant that is not routed through the seam would too.
    #[test]
    fn every_picker_kind_routes_its_query_through_the_shared_core() {
        let mut s = rich_picker_store();
        let mut empty_kinds = Vec::new();
        for kind in picker_kinds() {
            let candidates = s.candidates_for(kind);
            if candidates.is_empty() {
                empty_kinds.push(kind);
            }
            s.open_picker(kind, "Pin C: ", candidates.clone());
            // Extend the query ('a' is not the Stash `x` / Annotations `d` verb).
            s.picker_query_char('a');
            let query = s.picker_query().to_string();
            let uses_file = picker_kind_uses_file_matcher(kind);
            let displays: Vec<&str> = candidates.iter().map(|c| c.display.as_str()).collect();
            let expected = {
                let m = if uses_file { &mut s.file_matcher } else { &mut s.matcher };
                narrow(&query, &displays, m)
            };
            let expected_names: Vec<&str> =
                expected.iter().map(|(i, _)| candidates[*i].name.as_str()).collect();
            let actual_names: Vec<&str> =
                s.picker_filtered().iter().map(|(c, _)| c.name.as_str()).collect();
            assert_eq!(
                actual_names, expected_names,
                "kind {kind:?}: the picker's filtered rows must equal the shared core (narrowing::narrow) on the same candidates_for({kind:?}) and matcher — a divergence means this kind bypassed the shared narrowing seam"
            );
        }
        assert!(
            empty_kinds.is_empty(),
            "rich_picker_store gave no candidates for {empty_kinds:?} — the cross-check is vacuous for those; populate them"
        );
    }

    /// Pin A: the shared core parses with `CaseMatching::Ignore` — the query's
    /// case need not match the candidate's. A query typed UPPERCASE still finds
    /// the lowercase candidate. Mutating the core to `CaseMatching::Respect`
    /// reddens this.
    #[test]
    fn picker_query_matches_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_find_file();
        // Uppercase query against the lowercase "src/lib.rs" display.
        for c in "LIB".chars() {
            store.key_event(key(&c.to_string()));
        }
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["src/lib.rs"],
            "CaseMatching::Ignore must match case-insensitively: {names:?}"
        );
    }

    /// Pin B: when a query shrinks the filtered set below the current selection,
    /// the selection is clamped into the new set (the session's one rule: `selected
    /// = selected.min(filtered.len().saturating_sub(1))`). The selection survives
    /// the shrink on the last surviving row. Dropping the clamp reddens this.
    #[test]
    fn picker_selection_clamps_into_a_shrunk_filtered_set() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_find_file(); // 4 rows: Cargo.toml, README.md, src/lib.rs, src/main.rs
        // Move the selection to the last row (index 3).
        for _ in 0..3 {
            store.key_event(key("DOWN"));
        }
        assert_eq!(store.picker_selected(), 3);
        // "lib" leaves exactly one row (src/lib.rs); the selection clamps 3 -> 0.
        for c in "lib".chars() {
            store.key_event(key(&c.to_string()));
        }
        assert_eq!(store.picker_count().0, 1, "only src/lib.rs survives");
        assert_eq!(store.picker_selected(), 0, "selection clamped into the shrunken set");
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["src/lib.rs"], "the survivor is src/lib.rs: {names:?}");
    }

    /// Pin D: `narrow`'s sort is STABLE — rows with equal scores keep source
    /// order. Identical displays score identically; a stable sort keeps the lower
    /// source index first. The existing picker suite never asserted tie order, so
    /// this locks it as behaviour (plan 018 issue 01).
    ///
    /// What this pin actually discriminates: it locks the TIE ORDER (equal-score
    /// rows appear in source order). It reddens under any order-CHANGING mutation
    /// — e.g. a tie-break by descending index, `sort_by_key(|(Reverse(score),
    /// Reverse(index))|)`, observed as the index list coming out `[N-1, N-2, …]`.
    /// It does NOT redden under `sort_unstable_by_key`: std's sort detects that an
    /// all-equal input is already ordered and bails, so that specific mutation is
    /// non-discriminative here (measured at N=32 and N=4096). The pin therefore
    /// proves the tie ORDER is pinned as behaviour; it is not, by itself, a proof
    /// that the sort is specifically the stable `sort_by_key`.
    #[test]
    fn narrow_sort_is_stable_equal_scores_keep_source_order() {
        // A modest list of identical displays → all equal scores. A stable sort
        // keeps the source index order [0..N); an order-changing sort does not.
        // N is kept small: on failure we print only a prefix, never the whole list.
        const N: usize = 32;
        let displays = vec!["same candidate"; N];
        let mut m = Matcher::new(Config::DEFAULT);
        let out = narrow("candidate", &displays, &mut m);
        let indices: Vec<usize> = out.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices.len(), N, "every row must be retained for an all-match query");
        // A permutation of 0..N is the source order iff it is strictly ascending.
        // That check is complete here and, on failure, dumps only a short prefix.
        let is_source_order = indices.windows(2).all(|w| w[0] < w[1]);
        assert!(
            is_source_order,
            "equal-score rows must keep source order (stable sort); first 6: {:?}",
            &indices[..N.min(6)]
        );
    }

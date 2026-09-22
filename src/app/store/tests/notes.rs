use super::*;

    /// The notes edit path records its `InputEdit` on the retained parse
    /// tree and the rebuild stays byte-identical to a full parse.
    #[test]
    fn notes_edit_keeps_incremental_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        let mut store = store(root);
        store.open_notes();
        let key = store.buffers.current().unwrap().to_string();
        let mtime = store.buffers.get(&key).unwrap().mtime;
        let tree_key = TreeKey::new(&key, mtime);
        // First highlight: full parse + retained tree.
        store.ensure_highlight();
        assert!(
            store.highlight_cache.retain_contains(&tree_key),
            "markdown notes buffer must retain a parse tree after highlighting"
        );
        // Type a line: the edit is recorded on the retained tree and the
        // cache entry is invalidated (rebuilt on the next ensure).
        for c in "hi\n".chars() {
            store.notes_insert_char(c);
        }
        store.ensure_highlight();
        assert!(
            store.highlight_cache.retain_contains(&tree_key),
            "retained tree must survive the edit (same buffer, same mtime)"
        );
        // The rebuilt result must be byte-identical to a from-scratch
        // parse of the same content (the correctness bar).
        let (rope, path, lang) = {
            let buf = store.buffers.get(&key).unwrap();
            let path_str = buf.path.as_ref().unwrap().to_string_lossy().into_owned();
            (
                buf.rope.clone(),
                buf.path.clone().unwrap(),
                store.grammar_registry.language_for(&path_str),
            )
        };
        assert_eq!(lang, LanguageId::Markdown);
        let from_scratch = highlight::highlight_reusable(&rope, lang).unwrap().0;
        let cache_key = CacheKey::new(&path, mtime, store.theme.name());
        assert_eq!(
            store.highlight_cache.get(&cache_key),
            Some(&from_scratch),
            "incrementally rebuilt highlight must be byte-identical to a full parse"
        );
    }

    #[test]
    fn notes_open_is_editable_and_save_writes_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
        let mut store = store(root);
        store.open_notes();
        // The notes buffer must be current and editable.
        let key = store.buffers.current().unwrap().to_string();
        let buf = store.buffers.get(&key).unwrap();
        assert!(buf.editable, "notes buffer must be editable");
        assert!(buf.path.is_some(), "notes buffer must have a path");
        // Type some text (bounded editing: append at end).
        store.notes_insert_char('H');
        store.notes_insert_char('i');
        let buf = store.buffers.get(&key).unwrap();
        assert!(buf.locally_modified(), "editing must set locally_modified");
        assert!(buf.text().contains("Hi"));
        // Save: writes to disk.
        store.save_buffer();
        let notes_path = root.join(".redline-notes.md");
        assert!(notes_path.exists(), "notes file must exist after save");
        let content = std::fs::read_to_string(&notes_path).unwrap();
        assert!(content.contains("Hi"), "saved content must contain the edit");
        // After save, locally_modified is cleared.
        let buf = store.buffers.get(&key).unwrap();
        assert!(!buf.locally_modified());
    }

    #[test]
    fn notes_backspace_deletes_last_char() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
        let mut store = store(root);
        store.open_notes();
        let key = store.buffers.current().unwrap().to_string();
        store.notes_insert_char('a');
        store.notes_insert_char('b');
        store.notes_backspace();
        let buf = store.buffers.get(&key).unwrap();
        assert!(!buf.text().ends_with("ab"), "backspace must remove 'b'");
    }

    /// (f) The annotation-mode regression guard for the whole issue (015-03):
    /// in `Annotation` mode the coarse behaviour stays byte-for-byte —
    /// self-insert appends at the END of the buffer and Backspace deletes the
    /// LAST char, EVEN WHEN THE POINT IS AT THE START. A col-0 point that
    /// still lands the edit at the buffer end is exactly the coarse shape, so
    /// this discriminates from the accurate point-accurate commands (which
    /// would land at the point). Routed through `key_event` (the notes-edit
    /// guard) so the mode routing itself is what is pinned.
    #[test]
    fn annotation_mode_keeps_coarse_append_and_backspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes(); // notes buffer: editable + Annotation mode
        let bk = s.buffers.current().unwrap().to_string();
        let before = s.buffers.get(&bk).unwrap().text().len();
        // Park the point at the very start of the buffer.
        s.set_point(0, 0, 0);
        assert_eq!(s.point_col(), 0);
        // Type a char: Annotation mode appends it at the END (not at col 0).
        s.key_event(key("Z"));
        let text = s.buffers.get(&bk).unwrap().text();
        assert!(text.ends_with('Z'), "annotation mode must append at the end");
        assert!(
            text.len() == before + 1 && !text.starts_with('Z'),
            "the char must not land at the point (col 0) — it is appended at the end"
        );
        // Backspace: removes the LAST char (the Z), wherever the point is.
        s.key_event(key("C-h"));
        assert_eq!(s.buffers.get(&bk).unwrap().text().len(), before);
        assert!(!s.buffers.get(&bk).unwrap().text().ends_with('Z'));
    }

    /// Editable buffers (issue 003-02): typing in the notes buffer near the
    /// bottom keeps the insertion row inside the visible file-view window.
    #[test]
    fn notes_typing_near_bottom_keeps_insertion_row_in_view() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        s.set_viewport_lines(8); // file-view window == 8
        s.open_notes();
        // Type 40 lines so the buffer far exceeds the 8-line viewport.
        for _ in 0..40 {
            s.notes_insert_char('x');
            s.notes_insert_char('\n');
        }
        let key = s.buffers.current().unwrap().to_string();
        let total = s.buffers.get(&key).unwrap().line_count();
        assert!(total > 8, "notes buffer must overflow the viewport: {total} lines");
        // The insertion row (the last line) must be inside the visible window.
        let (top, total2, viewport) = s.file_view_scroll_info();
        assert_eq!(total2, total);
        let last = total - 1;
        assert!(
            last < top + viewport,
            "insertion row {last} must be in view [top={top}, {viewport} rows)"
        );
        // The rendered window ends exactly on the last line (no gap).
        // (No annotations here: every rendered row is a code row, so the
        // row count equals the buffer-line count.)
        let rows = s.file_view_rows();
        assert_eq!(
            top + rows.len(),
            total,
            "window must end at the last line: top={top} len={} total={total}",
            rows.len()
        );
    }

    #[test]
    fn notes_printable_inserts_and_c_c_p_f_dispatches_while_editing() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut s = store(dir.path());
        s.open_notes();
        let bkey = s.buffers.current().unwrap().to_string();

        // A plain printable (not bound) self-inserts while editing.
        s.key_event(key("l"));
        assert!(
            s.buffers.get(&bkey).unwrap().text().ends_with("l"),
            "`l` must self-insert in the notes buffer"
        );

        // C-c p f (find-file) must open the finder while a notes buffer is
        // current — a multi-key sequence ending in two printables.
        s.key_event(key("C-c"));
        s.key_event(key("p"));
        s.key_event(key("f"));
        assert!(s.picker_open(), "C-c p f must open the finder");
        assert_eq!(s.picker_kind(), Some(PickerKind::FindFile));
        assert!(
            !s.buffers.get(&bkey).unwrap().text().contains("p f"),
            "the prefix tail must not have been typed into notes"
        );
    }

    #[test]
    fn notes_c_x_g_dispatches_while_editing() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = git_store(dir.path());
        s.open_notes();
        let bkey = s.buffers.current().unwrap().to_string();

        // A plain printable (not bound) self-inserts while editing.
        s.key_event(key("l"));
        assert!(
            s.buffers.get(&bkey).unwrap().text().ends_with("l"),
            "`l` must self-insert in the notes buffer"
        );

        // C-x g (magit-status) must dispatch while a notes buffer is current:
        // the pending prefix must reach the engine, not be self-inserted.
        s.key_event(key("C-x"));
        assert_eq!(s.pending_display(), "C-x", "C-x must arm the prefix");
        s.key_event(key("g"));
        assert_eq!(
            s.top_view(),
            ViewId::MagitStatus,
            "C-x g must dispatch while editing notes"
        );
        assert!(
            !s.buffers.get(&bkey).unwrap().text().contains("x g"),
            "the prefix must not have been typed into notes"
        );
    }

    #[test]
    fn notes_c_g_clears_pending_only_not_buffer() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut s = store(dir.path());
        s.open_notes();
        s.key_event(key("C-x")); // arm a prefix
        assert_eq!(s.pending_display(), "C-x");
        s.key_event(key("C-g")); // global cancel
        assert!(s.pending.is_empty(), "C-g must clear the pending prefix");
        // The notes buffer stays current and open; the app is not quitting.
        assert_eq!(s.top_view(), ViewId::Buffer);
        assert!(!s.quit, "C-g in notes must not quit");
        // A printable now self-inserts (the prefix is gone).
        s.key_event(key("a"));
        assert!(s.buffers.current_buffer().unwrap().text().ends_with("a"));
    }

    // ── issue 05 (finding 2): notes self-creation is not a conflict ───

    #[test]
    fn notes_self_creation_event_is_ignored_then_real_edit_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        let bkey = s.buffers.current().unwrap().to_string();
        let notes_path = dir.path().join(".redline-notes.md");

        // Type first, so the buffer is locally owned when the (late) watcher
        // event for our own file creation lands.
        s.key_event(key("a"));
        assert!(s.buffers.get(&bkey).unwrap().locally_modified());

        // The self-inflicted creation event must NOT flag a conflict.
        s.apply_project_change(&change(vec![notes_path.clone()]));
        assert!(
            !s.buffers.get(&bkey).unwrap().changed_on_disk,
            "our own file creation must not set changed_on_disk"
        );
        assert!(!s.current_buffer_changed_on_disk());

        // A genuine external edit (a later, un-marked event) still conflicts.
        s.apply_project_change(&change(vec![notes_path.clone()]));
        assert!(
            s.buffers.get(&bkey).unwrap().changed_on_disk,
            "a genuine external edit must still set changed_on_disk"
        );
        assert!(s.current_buffer_changed_on_disk());
    }

    // ── issue 003-01: access-only batch must not set changed_on_disk ─────

    /// Parse → serialize → parse round-trip: records, raw blocks, and free
    /// text on BOTH sides of the section survive verbatim.
    #[test]
    fn notes_parse_serialize_round_trip() {
        let doc = NotesDoc {
            before: vec!["# Notes".to_string(), "free line one".to_string()],
            entries: vec![
                NotesEntry::Raw("# a comment".to_string()),
                NotesEntry::Record(Annotation {
                    syntax: None,
                    path: "src/main.rs".to_string(),
                    line: 4,
                    col: 2,
                    anchor: "fn main() {".to_string(),
                    text: "fix the off-by-one: it's: nasty".to_string(),
                    orphaned: false,
                }),
                NotesEntry::Raw("[annotation]\npath: src/main.rs\nline: 9\n(no required fields)".to_string()),
                NotesEntry::Record(Annotation {
                    syntax: None,
                    path: "README.md".to_string(),
                    line: 0,
                    col: 0,
                    anchor: "Redline".to_string(),
                    text: "top".to_string(),
                    orphaned: true,
                }),
            ],
            after: vec!["trailing free text".to_string()],
        };
        let text = serialize_notes(&doc);
        let reparsed = parse_notes(&text);
        assert_eq!(reparsed, doc, "round-trip must be exact");
        // The anchor with a colon round-trips byte-for-byte (the value is
        // the text after the FIRST colon of its field line).
        let rec = &reparsed.entries[1];
        let a = rec.as_record().unwrap();
        assert_eq!(a.anchor, "fn main() {");
        assert_eq!(a.text, "fix the off-by-one: it's: nasty");
        // A file with NO section at all parses into pure `before` and
        // re-serializes with the section appended, keeping the text.
        let plain = "alpha\nbeta\n";
        let d = parse_notes(plain);
        assert_eq!(d.before, vec!["alpha".to_string(), "beta".to_string()]);
        assert!(d.entries.is_empty());
        let s = serialize_notes(&d);
        assert!(s.starts_with("alpha\nbeta\n"));
        assert!(s.contains(NOTES_BEGIN) && s.contains(NOTES_END));
        assert_eq!(parse_notes(&s).before, d.before);
    }

    /// Tolerant parse: a malformed record (missing required fields) is
    /// kept VERBATIM as a raw block, never dropped, and valid records
    /// around it still parse.
    #[test]
    fn notes_tolerant_parse_keeps_malformed() {
        let text = "before\n\
                    <!-- redline-annotations:begin -->\n\
                    [annotation]\n\
                    path: a.rs\n\
                    line: 1\n\
                    note: missing anchor\n\
                    stray line without colon\n\
                    [annotation]\n\
                    path: b.rs\n\
                    line: 0\n\
                    anchor: hello\n\
                    note: ok\n\
                    <!-- redline-annotations:end -->\n\
                    after\n";
        let doc = parse_notes(text);
        assert_eq!(doc.before, vec!["before".to_string()]);
        assert_eq!(doc.after, vec!["after".to_string()]);
        // Two entries: the malformed block (raw, verbatim) + the good one.
        assert_eq!(doc.entries.len(), 2);
        assert!(matches!(doc.entries[0], NotesEntry::Raw(_)), "malformed kept verbatim: {:?}", doc.entries[0]);
        let raw = match &doc.entries[0] {
            NotesEntry::Raw(s) => s,
            _ => unreachable!(),
        };
        assert!(raw.contains("missing anchor"), "{raw}");
        assert!(raw.contains("stray line without colon"), "{raw}");
        let a = doc.entries[1].as_record().unwrap();
        assert_eq!(a.path, "b.rs");
        assert_eq!(a.anchor, "hello");
        // Re-serializing keeps the malformed block byte-for-byte.
        let out = serialize_notes(&doc);
        assert!(out.contains("note: missing anchor"), "{out}");
        assert_eq!(parse_notes(&out).entries.len(), 2);
    }

    /// A record whose stored line no longer holds the anchor is re-anchored
    /// by content within ±25 lines (the window bound is exclusive beyond
    /// ±25: a match at exactly ±25 re-anchors, at ±26 it does not).
    #[test]
    fn notes_reanchor_on_drift_and_bounds() {
        // 30 distinct filler lines + the anchor at line 29 (0-based).
        let mut lines: Vec<String> = (0..30).map(|i| format!("filler {i}")).collect();
        lines.push("THE ANCHOR LINE".to_string()); // line 30
        let content = lines.join("\n") + "\n";

        // Case 1: the stored line is 5 ABOVE the anchor (line 25 vs 30):
        // drift → unique match within ±25 → re-anchor to 30.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann1.rs", &content);
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/ann1.rs".to_string(),
            line: 25,
            col: 0,
            anchor: "THE ANCHOR LINE".to_string(),
            text: "n".to_string(),
            orphaned: false,
        }));
        let key = s.buffers.current().unwrap().to_string();
        s.reanchor_for_key(&key);
        let a = ann_records(&s)[0];
        assert_eq!(a.line, 30, "unique match re-anchors");
        assert!(!a.orphaned);
        drop(s);

        // Case 2 (boundary): a match exactly +25 away re-anchors.
        let mut s = store_with_project();
        let far: Vec<String> = (0..26).map(|i| format!("pad {i}")).collect();
        let content = far.join("\n") + "\nEDGE\n"; // anchor at line 26 = 0 + 26? no: line 26
        open_ann_file(&mut s, "src/ann2.rs", &content);
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/ann2.rs".to_string(),
            line: 1,
            col: 0,
            anchor: "EDGE".to_string(),
            text: "n".to_string(),
            orphaned: false,
        }));
        let key = s.buffers.current().unwrap().to_string();
        s.reanchor_for_key(&key);
        assert_eq!(ann_records(&s)[0].line, 26, "match at exactly +25 re-anchors");

        // Case 3 (boundary): a match at +26 does NOT re-anchor → orphaned,
        // line unchanged.
        let mut s = store_with_project();
        let far: Vec<String> = (0..27).map(|i| format!("pad {i}")).collect();
        let content = far.join("\n") + "\nFAR\n"; // anchor at line 27
        open_ann_file(&mut s, "src/ann3.rs", &content);
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/ann3.rs".to_string(),
            line: 1,
            col: 0,
            anchor: "FAR".to_string(),
            text: "n".to_string(),
            orphaned: false,
        }));
        let key = s.buffers.current().unwrap().to_string();
        s.reanchor_for_key(&key);
        let a = ann_records(&s)[0];
        assert_eq!(a.line, 1, "no move at +26 (never a guessed line)");
        assert!(a.orphaned, "orphaned flag set");
    }

    /// Ambiguous drift (two identical anchor lines inside ±25) → orphaned,
    /// no move; and an anchor that returns to its stored line clears the
    /// orphan flag (idempotent/stable: a second pass changes nothing).
    #[test]
    fn notes_reanchor_ambiguous_and_idempotent() {
        let content = "same line\nother\nsame line\n".to_string();
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann4.rs", &content);
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/ann4.rs".to_string(),
            line: 5, // out of range: drift for sure
            col: 0,
            anchor: "same line".to_string(),
            text: "n".to_string(),
            orphaned: false,
        }));
        let key = s.buffers.current().unwrap().to_string();
        s.reanchor_for_key(&key);
        let a = ann_records(&s)[0];
        assert!(a.orphaned, "ambiguous (2 matches) must not guess");
        assert_eq!(a.line, 5);
        // Idempotent: a second pass leaves the record exactly as-is.
        let (a_line, a_orphaned) = (a.line, a.orphaned);
        s.reanchor_for_key(&key);
        let a2 = ann_records(&s)[0];
        assert_eq!(a2.line, a_line);
        assert_eq!(a2.orphaned, a_orphaned);
        // Stable: when the anchor text returns to the STORED line (line 5,
        // uniquely), the orphan flag clears.
        let key = s.buffers.current().unwrap().to_string();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.rope = Rope::from_str("a\nb\nc\nd\ne\nsame line\n");
        }
        let key = s.buffers.current().unwrap().to_string();
        s.reanchor_for_key(&key);
        assert!(!ann_records(&s)[0].orphaned, "anchor back → orphan cleared");
    }

    /// `A` on a line prompts with an empty prefill; on an annotated line it
    /// pre-fills the existing note for edit; RET commits the record to the
    /// notes file (and the in-memory doc); `d` deletes with an echo; a `d`
    /// on an unannotated line is a no-op with a message.
    #[test]
    fn notes_a_flow_prefill_commit_delete() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann5.rs", "one\ntwo\nthree\n");
        // A on line 1 (fresh): prompt opens, empty input.
        s.set_point_line(1);
        s.annotate();
        assert!(s.note_prompt_active());
        assert_eq!(s.note_prompt_input(), "");
        s.note_prompt_char('f');
        s.note_prompt_char('i');
        s.note_prompt_confirm();
        assert!(!s.note_prompt_active());
        assert_eq!(ann_records(&s).len(), 1);
        let a = ann_records(&s)[0];
        assert_eq!(a.path, "src/ann5.rs");
        assert_eq!(a.line, 1);
        assert_eq!(a.anchor, "two", "anchor = the exact anchored-line text");
        assert_eq!(a.text, "fi");
        // The record is on disk in the notes file, in the structured
        // section.
        let disk = std::fs::read_to_string(
            s.project.as_ref().unwrap().root.join(".redline-notes.md"),
        )
        .unwrap();
        assert!(disk.contains(NOTES_BEGIN), "{disk}");
        assert!(disk.contains("path: src/ann5.rs"), "{disk}");
        assert!(disk.contains("note: fi"), "{disk}");
        // A on the annotated line pre-fills for edit.
        s.set_point_line(1);
        s.annotate();
        assert_eq!(s.note_prompt_input(), "fi", "prefill for edit");
        s.note_prompt_cancel();
        // d on the annotated line deletes with an echo.
        s.set_point_line(1);
        s.annotate_delete();
        assert!(s.message.contains("deleted annotation: fi"), "{}", s.message);
        assert_eq!(ann_records(&s).len(), 0);
        // d on an unannotated line: message, no record, no unbound-key echo.
        s.set_point_line(0);
        s.annotate_delete();
        assert_eq!(s.message, "no annotation on this line");
        assert!(ann_records(&s).is_empty());
    }

    /// Empty RET while editing an existing record deletes it (the
    /// pre-filled-for-edit cancel path); empty RET on a fresh prompt just
    /// cancels.
    #[test]
    fn notes_empty_ret_edits_delete_cancels() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann6.rs", "a\nb\n");
        s.set_point_line(0);
        s.annotate();
        s.note_prompt_char('x');
        s.note_prompt_confirm();
        assert_eq!(ann_records(&s).len(), 1);
        // Pre-fill for edit, clear it, RET → delete.
        s.set_point_line(0);
        s.annotate();
        assert_eq!(s.note_prompt_input(), "x");
        s.note_prompt_backspace();
        s.note_prompt_confirm();
        assert_eq!(ann_records(&s).len(), 0, "empty RET on existing record deletes");
        // Fresh prompt, RET with nothing → cancel, no record.
        s.set_point_line(1);
        s.annotate();
        s.note_prompt_confirm();
        assert_eq!(ann_records(&s).len(), 0);
        assert_eq!(s.message, "note cancelled");
    }

    // ── plan 007 issue 02: syntax-anchored annotations ───────────────────

    /// The HEADLINE test (plan 007 issue 02): annotate a Rust fn (the
    /// point on the function name → `SyntaxAnchor { identifier,
    /// target_one }`), then simulate a 100-line insertion AND a
    /// rustfmt-style reformat (the anchored line's exact text is GONE
    /// from the file). The syntax-anchored record follows the function
    /// (re-anchored to its new line, orphan flag cleared); the SAME
    /// record WITHOUT the syntax anchor orphans at its last known line —
    /// proof the ±25-line text path alone cannot do this.
    #[test]
    fn notes_syntax_anchor_survives_insertion_and_reformat() {
        let original = "fn target_one() {\n    let x = 1;\n    x\n}\n";
        // The reformat rewrites the signature line; 100 filler lines push
        // the function 100 lines down (far outside ±25).
        let filler: Vec<String> = (0..100).map(|i| format!("// filler {i}")).collect();
        let rewritten =
            filler.join("\n") + "\nfn target_one()\n{\n    let x = 1;\n    x\n}\n";
        // The discriminating precondition: the anchored line's text no
        // longer exists ANYWHERE — the text path (exact-line and ±25)
        // has nothing to match, inside or outside the window.
        assert!(
            !rewritten.contains("fn target_one() {"),
            "the text path must have nothing to match"
        );

        // Part 1: the syntax-anchored record follows the function.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/syn1.rs", original);
        s.set_point(0, 3, 3); // col 3: on the `target_one` name
        s.annotate();
        s.note_prompt_char('s');
        s.note_prompt_char('y');
        s.note_prompt_confirm();
        let a = ann_records(&s)[0];
        assert_eq!(
            a.syntax,
            Some(SyntaxAnchor {
                kind: "identifier".to_string(),
                name: "target_one".to_string(),
            }),
            "the fn name captures a syntax anchor"
        );
        assert_eq!(a.anchor, "fn target_one() {");
        // Simulate the 100-line insertion + the reformat.
        let key = s.buffers.current().unwrap().to_string();
        {
            let buf = s.buffers.get_mut(&key).unwrap();
            buf.rope = Rope::from_str(&rewritten);
        }
        s.reanchor_for_key(&key);
        let a = ann_records(&s)[0];
        assert_eq!(
            a.line, 100,
            "the note follows the function across 100 lines + a reformat"
        );
        assert!(!a.orphaned, "unique syntax match → not orphaned");
        // Stable: a second pass changes nothing (idempotent).
        s.reanchor_for_key(&key);
        assert_eq!(ann_records(&s)[0].line, 100);
        assert!(!ann_records(&s)[0].orphaned);
        drop(s);

        // Part 2 (discriminating half): the SAME drive with a record that
        // has NO syntax anchor (the legacy shape) — the ±25-line text
        // path finds nothing (the anchor text is gone and the move is
        // 100 lines out) → orphaned, line unchanged. Without the syntax
        // anchor, Part 1's outcome is impossible.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/syn2.rs", original);
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            path: "src/syn2.rs".to_string(),
            line: 0,
            col: 3,
            anchor: "fn target_one() {".to_string(),
            text: "sy".to_string(),
            orphaned: false,
            syntax: None,
        }));
        let key = s.buffers.current().unwrap().to_string();
        {
            let buf = s.buffers.get_mut(&key).unwrap();
            buf.rope = Rope::from_str(&rewritten);
        }
        s.reanchor_for_key(&key);
        let a = ann_records(&s)[0];
        assert!(
            a.orphaned,
            "text path alone: 0 matches → orphaned (never a guess)"
        );
        assert_eq!(a.line, 0, "the orphan stays at its last known line");
    }

    /// A legacy record (no syntax_* keys) parses with `syntax: None`,
    /// still re-anchors by text alone, and serializes back WITHOUT the
    /// syntax keys — byte-identical to the pre-007-02 shape (no
    /// migration of legacy records that are never touched).
    #[test]
    fn notes_legacy_record_stays_byte_identical_and_text_anchors() {
        let record = "[annotation]\n\
                      path: src/old.rs\n\
                      line: 1\n\
                      col: 0\n\
                      anchor: two\n\
                      note: old note\n\
                      orphaned: false\n";
        let doc_text = format!("{NOTES_BEGIN}\n{record}{NOTES_END}\n");
        let doc = parse_notes(&doc_text);
        let rec = doc.entries[0].as_record().expect("the legacy record parses");
        assert!(rec.syntax.is_none(), "legacy record → syntax: None");
        assert_eq!(
            serialize_notes(&doc),
            doc_text,
            "a never-moved legacy record round-trips byte-identically (no syntax keys appear)"
        );
        // Re-anchoring still works by text alone for it.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/old.rs", "one\ntwo\nthree\n");
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            path: "src/old.rs".to_string(),
            line: 5, // out of range: drift for sure
            col: 0,
            anchor: "two".to_string(),
            text: "old note".to_string(),
            orphaned: false,
            syntax: None,
        }));
        let key = s.buffers.current().unwrap().to_string();
        s.reanchor_for_key(&key);
        assert_eq!(ann_records(&s)[0].line, 1, "legacy record re-anchors by text");
        let out = serialize_notes(&s.notes_doc);
        assert!(
            !out.contains("syntax_kind"),
            "no syntax keys ever appear for legacy records: {out}"
        );
        // Unknown keys inside a record block: the record stays VALID
        // (forward compatibility) and the unknown key is not re-emitted —
        // the pre-007-02 rule, unchanged.
        let unknown = format!(
            "{NOTES_BEGIN}\n\
             [annotation]\n\
             path: src/old.rs\n\
             line: 1\n\
             col: 0\n\
             anchor: two\n\
             note: old note\n\
             orphaned: false\n\
             future_key: later\n\
             {NOTES_END}\n"
        );
        let udoc = parse_notes(&unknown);
        assert!(
            udoc.entries[0].as_record().is_some(),
            "a record with an unknown key stays a valid record"
        );
        assert!(
            udoc.entries[0].as_record().unwrap().syntax.is_none()
        );
        let uout = serialize_notes(&udoc);
        assert!(!uout.contains("future_key"), "unknown keys are not re-emitted (005-02 rule)");
        assert!(uout.contains("line: 1"), "the record's own fields survive");
    }

    /// Ambiguous syntax match (multiple identifier nodes with the same
    /// kind + name anywhere in the file) → no guess → the text rules run;
    /// when they also fail, the record orphans at its last known line.
    #[test]
    fn notes_syntax_ambiguous_match_orphans() {
        let original = "fn helper() {\n    let helper = 1;\n    helper\n}\n";
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/syn3.rs", original);
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            path: "src/syn3.rs".to_string(),
            line: 0,
            col: 3,
            anchor: "fn helper() {".to_string(),
            text: "amb".to_string(),
            orphaned: false,
            syntax: Some(SyntaxAnchor {
                kind: "identifier".to_string(),
                name: "helper".to_string(),
            }),
        }));
        // The signature line is reformatted (the anchor text is gone);
        // `helper` still appears as the fn name AND the local → 3
        // identifier nodes named `helper` → ambiguous.
        let key = s.buffers.current().unwrap().to_string();
        {
            let buf = s.buffers.get_mut(&key).unwrap();
            buf.rope = Rope::from_str("fn helper() -> i32\n{\n    let helper = 1;\n    helper\n}\n");
        }
        s.reanchor_for_key(&key);
        let a = ann_records(&s)[0];
        assert!(a.orphaned, "2+ syntax matches → no guess → orphan");
        assert_eq!(a.line, 0, "the orphan keeps its last known line");
    }

    /// A record WITH a syntax anchor round-trips exactly: serialize →
    /// parse → an equal record (the keys are re-emitted in full).
    #[test]
    fn notes_syntax_anchor_round_trip() {
        let rec = Annotation {
            path: "src/x.rs".to_string(),
            line: 7,
            col: 3,
            anchor: "fn x() {".to_string(),
            text: "rt".to_string(),
            orphaned: false,
            syntax: Some(SyntaxAnchor {
                kind: "identifier".to_string(),
                name: "x".to_string(),
            }),
        };
        let doc = NotesDoc {
            before: Vec::new(),
            entries: vec![NotesEntry::Record(rec.clone())],
            after: Vec::new(),
        };
        let out = serialize_notes(&doc);
        assert!(out.contains("syntax_kind: identifier\n"), "{out}");
        assert!(out.contains("syntax_name: x\n"), "{out}");
        let back = parse_notes(&out);
        assert_eq!(back.entries, vec![NotesEntry::Record(rec)]);
    }

    /// A half-written anchor (`syntax_kind` without `syntax_name`) degrades
    /// to `syntax: None` — the record stays VALID (tolerant parse, never
    /// half-fires); a record with a malformed required field stays a Raw
    /// block, verbatim (never dropped), even when it carries syntax keys
    /// (005-02).
    #[test]
    fn notes_syntax_keys_half_and_malformed() {
        let doc_text = format!(
            "{NOTES_BEGIN}\n\
             [annotation]\n\
             path: a.rs\n\
             line: 0\n\
             col: 0\n\
             anchor: one\n\
             note: half anchor\n\
             orphaned: false\n\
             syntax_kind: identifier\n\
             [annotation]\n\
             path: b.rs\n\
             line: 0\n\
             col: 0\n\
             note: missing its anchor\n\
             syntax_kind: identifier\n\
             syntax_name: ghost\n\
             {NOTES_END}\n"
        );
        let doc = parse_notes(&doc_text);
        assert_eq!(doc.entries.len(), 2);
        // The half anchor: a VALID record, `syntax: None`.
        let a = doc
            .entries[0]
            .as_record()
            .expect("a record with a half syntax pair stays valid");
        assert!(
            a.syntax.is_none(),
            "syntax_kind without syntax_name → None (never half-fires)"
        );
        assert_eq!(a.text, "half anchor");
        // The record missing `anchor` stays a Raw block, verbatim (the
        // syntax keys included — malformed records are never dropped).
        match &doc.entries[1] {
            NotesEntry::Raw(raw) => {
                assert!(raw.contains("missing its anchor"), "{raw}");
                assert!(raw.contains("syntax_name: ghost"), "{raw}");
            }
            other => panic!("malformed record must stay Raw: {other:?}"),
        }
    }

    /// A non-Rust buffer captures no syntax anchor (007-01 returns None
    /// for every non-Rust language — and no parse happens at all) and
    /// rides the text rules as before; a Rust point on a KEYWORD (col 0
    /// of the `fn` header) is the same: `None`, not a guess.
    #[test]
    fn notes_non_rust_and_keyword_points_have_no_syntax_anchor() {
        // Non-Rust: the record has `syntax: None` (old behavior intact).
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann_py.py", "def f():\n    return 1\n");
        s.set_point(0, 4, 4); // on the `f` in `def f():`
        s.annotate();
        s.note_prompt_char('p');
        s.note_prompt_confirm();
        let a = ann_records(&s)[0];
        assert!(a.syntax.is_none(), "non-Rust → no syntax anchor");
        assert_eq!(a.anchor, "def f():");
        drop(s);

        // Rust, point on the `fn` keyword (col 0): `node_at` has no
        // identifier-ish node there → honest None.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann_kw.rs", "fn kw_target() {\n    let y = 2;\n}\n");
        s.set_point(0, 0, 0); // col 0: the `f` of `fn`
        s.annotate();
        s.note_prompt_char('k');
        s.note_prompt_confirm();
        let a = ann_records(&s)[0];
        assert!(a.syntax.is_none(), "a keyword point captures no anchor");
        assert_eq!(a.anchor, "fn kw_target() {");
    }

    // ── plan 008 issue 01: annotation keying on external buffers ─────────

    /// The key-derivation point: project files key by project-relative path
    /// (byte-identical to pre-008); an external buffer keys by its ABSOLUTE
    /// path string; scratch (pathless) still yields `None`.
    #[test]
    fn notes_annotation_path_under_root_and_absolute() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/annext.rs", "a\nb\n");
        let proj_key = s.buffers.current().unwrap().to_string();
        assert_eq!(
            s.buffer_annotation_path(&proj_key).as_deref(),
            Some("src/annext.rs"),
            "under-root key unchanged (byte-identical)"
        );

        let ext = tempfile::tempdir().unwrap();
        let abs = ext.path().join("lib.rs");
        std::fs::write(&abs, "x\n").unwrap();
        let abs_str = abs.to_string_lossy().into_owned();
        let ext_key = s.open_external_path(&abs).unwrap();
        assert_eq!(
            s.buffer_annotation_path(&ext_key).as_deref(),
            Some(abs_str.as_str()),
            "out-of-root key = the absolute path"
        );
        // The current-buffer variant follows the current buffer.
        assert_eq!(
            s.current_annotation_path().as_deref(),
            Some(abs_str.as_str()),
            "current_annotation_path tracks the current buffer"
        );
        // Scratch: pathless buffers still have no key.
        let scratch_key = s.buffers
            .insert_rope(None, Rope::from_str("s"), std::time::SystemTime::now(), true);
        assert!(s.buffer_annotation_path(&scratch_key).is_none());
    }

    /// `A` → type → RET commits on an external buffer (record keyed by the
    /// absolute path, written to the project's notes file), `d` deletes it;
    /// the prefill sees the external record back.
    #[test]
    fn notes_external_buffer_annotate_delete_round_trip() {
        let (mut s, abs_str) = store_with_external_file();
        s.set_point_line(1);
        s.annotate();
        assert!(s.note_prompt_active());
        assert_eq!(s.note_prompt_input(), "", "fresh prompt, no prefill");
        s.note_prompt_char('l');
        s.note_prompt_char('i');
        s.note_prompt_confirm();
        let a = ann_records(&s)[0];
        assert_eq!(a.path, abs_str, "record keyed by the absolute path");
        assert_eq!(a.line, 1);
        assert_eq!(a.anchor, "ext line two");
        assert_eq!(a.text, "li");
        // On disk, in the project's notes file (placement unchanged).
        let disk = std::fs::read_to_string(
            s.project.as_ref().unwrap().root.join(".redline-notes.md"),
        )
        .unwrap();
        assert!(disk.contains(&format!("path: {abs_str}")), "{disk}");
        // The status count sees the external record.
        assert_eq!(s.annotation_count_display(), "1 note");
        // Pre-fill for edit on the external record.
        s.set_point_line(1);
        s.annotate();
        assert_eq!(s.note_prompt_input(), "li", "external record pre-fills");
        s.note_prompt_cancel();
        // `d` removes it.
        s.set_point_line(1);
        s.annotate_delete();
        assert!(s.message.contains("deleted annotation: li"), "{}", s.message);
        assert!(ann_records(&s).is_empty());
    }

    /// `file_view_rows` marker + note row render on an external buffer,
    /// keyed by the absolute path (pre-008 these were dead there).
    #[test]
    fn notes_external_buffer_marker_and_note_row() {
        let (mut s, _) = store_with_external_file();
        s.set_point_line(0);
        s.annotate();
        s.note_prompt_char('m');
        s.note_prompt_confirm();
        s.show_note_rows = true;
        let rows = s.file_view_rows();
        let code_rows: Vec<_> = rows.iter().filter(|r| !r.is_note).collect();
        let marker: Vec<_> = code_rows
            .iter()
            .filter(|r| r.annotated && r.line == 0)
            .collect();
        assert_eq!(marker.len(), 1, "margin marker on the annotated line");
        let note_rows: Vec<_> = rows.iter().filter(|r| r.is_note).collect();
        assert_eq!(note_rows.len(), 1, "the inline note row renders");
        assert!(note_rows[0].text.contains("m"), "{}", note_rows[0].text);
        // A record carrying the external file's IN-PROJECT relative name
        // must NOT match the external buffer (the external key is the
        // absolute path, so the keys differ).
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/lib_source.rs".to_string(),
            line: 0,
            col: 0,
            anchor: "ext line one".to_string(),
            text: "rel-path impostor".to_string(),
            orphaned: false,
        }));
        let rows = s.file_view_rows();
        assert_eq!(
            rows.iter().filter(|r| r.is_note).count(),
            1,
            "a rel-path record for the external file's in-project relative name never matches"
        );
    }

    /// The quit-dump carries the external record's path VERBATIM (absolute)
    /// with the open buffer's live code line; project records keep their
    /// relative path in the same dump.
    #[test]
    fn notes_external_buffer_dump_verbatim_path() {
        let (mut s, abs_str) = store_with_external_file();
        s.set_point_line(2);
        s.annotate();
        s.note_prompt_char('d');
        s.note_prompt_confirm();
        // A project record in the same doc keeps its relative path.
        open_ann_file(&mut s, "src/proj.rs", "p0\np1\n");
        s.set_point_line(0);
        s.annotate();
        s.note_prompt_char('p');
        s.note_prompt_confirm();
        let items = s.annotations_for_dump();
        let ext = items.iter().find(|i| i.path == abs_str).expect("external item");
        assert_eq!(ext.line, 3, "0-based line 2 → 1-based 3");
        assert_eq!(ext.code, "ext line three", "live code from the open external buffer");
        let proj = items.iter().find(|i| i.path == "src/proj.rs").expect("project item");
        assert_eq!(proj.code, "p0");
        // The formatted dump prints both paths verbatim.
        let block = format_notes_dump(&items, s.project.as_ref().unwrap().root.to_str().unwrap(), false);
        assert!(block.contains(&format!("{abs_str}:3\n")), "block: {}", block);
        let plain = format_notes_dump(&items, "", true);
        assert!(plain.contains(&format!("{abs_str}:3: d\n")), "plain: {plain}");
        assert!(plain.contains("src/proj.rs:1: p\n"), "plain: {plain}");
        // 008-01 P3 (folded into 006-02b item 8): pin the mixed ordering —
        // the absolute (`/`-prefixed) record sorts BEFORE the
        // project-relative record byte-wise (`/` < `s`), in BOTH modes.
        let (ext_pos, proj_pos) = (block.find(abs_str.as_str()), block.find("src/proj.rs"));
        assert!(ext_pos.is_some_and(|e| proj_pos.is_some_and(|p| e < p)),
            "block: the absolute record precedes the relative record:\n{block}");
        let (ext_pos, proj_pos) = (plain.find(abs_str.as_str()), plain.find("src/proj.rs"));
        assert!(ext_pos.is_some_and(|e| proj_pos.is_some_and(|p| e < p)),
            "plain: the absolute record precedes the relative record:\n{plain}");
    }

    /// Adding an external record must not touch the existing project-relative
    /// record's bytes in the notes file (no migration, no re-keying).
    #[test]
    fn notes_external_annotation_leaves_rel_records_untouched() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/keep.rs", "k0\nk1\n");
        s.set_point_line(1);
        s.annotate();
        s.note_prompt_char('k');
        s.note_prompt_confirm();
        let before = std::fs::read_to_string(
            s.project.as_ref().unwrap().root.join(".redline-notes.md"),
        )
        .unwrap();
        assert!(before.contains("path: src/keep.rs"), "{before}");
        // The record block for the rel record: its `path:` line through the
        // line before the next record's `path:`.
        let block = |text: &str, path: &str| -> String {
            let marker = format!("path: {path}\n");
            let start = text.find(&marker).expect("record line");
            // Through the record's final `orphaned:` line (schema-ordered).
            let tail = &text[start + marker.len()..];
            let end = tail.find("orphaned:").unwrap() + "orphaned:".len();
            text[start..start + marker.len() + end].to_string()
        };

        let (mut s, abs_str) = {
            let ext = tempfile::tempdir().unwrap();
            let abs = ext.path().join("lib.rs");
            std::fs::write(&abs, "e0\ne1\n").unwrap();
            s.open_external_path(&abs).unwrap();
            (s, abs.to_string_lossy().into_owned())
        };
        s.set_point_line(1);
        s.annotate();
        s.note_prompt_char('e');
        s.note_prompt_confirm();
        let after = std::fs::read_to_string(
            s.project.as_ref().unwrap().root.join(".redline-notes.md"),
        )
        .unwrap();
        assert_eq!(
            block(&before, "src/keep.rs"),
            block(&after, "src/keep.rs"),
            "the rel record's bytes are untouched: {after}"
        );
        assert!(after.contains(&format!("path: {abs_str}")), "external record written: {after}");
    }

    // ── plan 005 issue 03: quit-dump formatter + accessor ───────────────

    /// The default block mode is the agent brief: header + blank line, then
    /// path:line, the anchored line indented +4, and `NOTE:` with the text.
    #[test]
    fn notes_dump_block_format_exact_bytes() {
        let items = vec![dump_item(
            "src/app/store.rs",
            1420,
            "fn save_buffer(&mut self) {",
            "this silently overwrites the mtime; check the conflict marker first",
            false,
        )];
        let out = format_notes_dump(&items, "/home/dev/red", false);
        let expected = "# redline annotations \u{2014} /home/dev/red\n\n".to_owned()
            + "src/app/store.rs:1420\n"
            + "    fn save_buffer(&mut self) {\n"
            + "  NOTE: this silently overwrites the mtime; check the conflict marker first\n";
        assert_eq!(out, expected, "exact block bytes");
    }

    /// An orphaned record carries the explicit marker line (the agent must
    /// not trust a stale line number silently) and its code line is the
    /// STORED anchor (last known anchored text).
    #[test]
    fn notes_dump_orphaned_marker() {
        let items = vec![dump_item("src/old.rs", 7, "fn gone() {", "stale note", true)];
        let out = format_notes_dump(&items, "/p", false);
        let expected = "# redline annotations \u{2014} /p\n\n".to_owned()
            + "src/old.rs:7\n"
            + "  ORPHANED (anchor text not found)\n"
            + "    fn gone() {\n"
            + "  NOTE: stale note\n";
        assert_eq!(out, expected, "exact orphaned bytes");
    }

    /// Multi-line notes: subsequent lines align under the first note line's
    /// text (block: 8 spaces = 2 + `NOTE: `; plain: the `path:line: `
    /// prefix repeats).
    #[test]
    fn notes_dump_multiline_note_alignment() {
        let items = vec![dump_item("a.rs", 3, "code", "first\nsecond\nthird", false)];
        let block = format_notes_dump(&items, "/p", false);
        assert!(
            block.contains("  NOTE: first\n        second\n        third\n"),
            "block continuation alignment: {block:?}"
        );
        let plain = format_notes_dump(&items, "/p", true);
        assert_eq!(
            plain,
            "a.rs:3: first\n".to_owned() + "a.rs:3: second\n" + "a.rs:3: third\n",
            "plain continuation alignment"
        );
    }

    /// Ordering is part of the contract: path asc, then line asc, in BOTH
    /// modes (diff stability).
    #[test]
    fn notes_dump_ordering_path_then_line() {
        let items = vec![
            dump_item("b.rs", 5, "x", "n5", false),
            dump_item("a.rs", 9, "y", "n9", false),
            dump_item("a.rs", 2, "z", "n2", false),
        ];
        for plain in [false, true] {
            let out = format_notes_dump(&items, "/p", plain);
            let positions: Vec<usize> = out
                .match_indices("a.rs:2")
                .map(|(i, _)| i)
                .chain(out.match_indices("a.rs:9").map(|(i, _)| i))
                .chain(out.match_indices("b.rs:5").map(|(i, _)| i))
                .collect();
            assert!(
                positions.windows(2).all(|w| w[0] < w[1]),
                "order a:2 < a:9 < b:5 (plain={plain}): {out:?}"
            );
        }
    }

    /// `--notes=plain` parity: same records, the grep/pipe shape with NO
    /// header and no code/orphan lines; the block keeps them.
    #[test]
    fn notes_dump_plain_flag_parity() {
        let items = vec![
            dump_item("src/main.rs", 4, "fn main() {", "fix the off-by-one", false),
            dump_item("README.md", 1, "Redline", "top", true),
        ];
        let plain = format_notes_dump(&items, "/p", true);
        assert_eq!(
            plain,
            "README.md:1: top\n".to_owned() + "src/main.rs:4: fix the off-by-one\n",
            "plain: path:line: text, no header, ordered"
        );
        assert!(!plain.contains("ORPHANED") && !plain.contains("# redline"));
        let block = format_notes_dump(&items, "/p", false);
        assert!(block.starts_with("# redline annotations \u{2014} /p\n\n"));
        assert!(block.contains("ORPHANED (anchor text not found)"));
        assert!(block.contains("    fn main() {") && block.contains("  NOTE: top"));
    }

    /// No annotations ⇒ zero bytes in BOTH modes (pipes stay clean).
    #[test]
    fn notes_dump_empty_is_zero_bytes() {
        assert_eq!(format_notes_dump(&[], "/p", false), "");
        assert_eq!(format_notes_dump(&[], "/p", true), "");
    }

    /// The accessor resolves the dump shape from the store: 1-based lines,
    /// `code` = the open buffer's current content when the anchor holds, the
    /// STORED anchor for orphaned records (never the line's current text),
    /// and the stored anchor when the buffer is not open.
    #[test]
    fn notes_dump_accessor_resolves_code_and_lines() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/dump1.rs", "alpha line\nbeta line\ngamma line\n");
        for a in [
            Annotation {
                syntax: None,
                path: "src/dump1.rs".to_string(),
                line: 0,
                col: 0,
                anchor: "alpha line".to_string(),
                text: "note one".to_string(),
                orphaned: false,
            },
            Annotation {
                syntax: None,
                path: "src/dump1.rs".to_string(),
                line: 1,
                col: 0,
                anchor: "GHOST ANCHOR".to_string(),
                text: "ghost note".to_string(),
                orphaned: true,
            },
        ] {
            s.notes_doc.entries.push(NotesEntry::Record(a));
        }
        // Persist so the lazy-load path does not clobber the in-memory doc
        // (the real A/prompt flow does this via sync on commit).
        s.sync_notes_from_doc();
        let items = s.annotations_for_dump();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].path, "src/dump1.rs");
        assert_eq!(items[0].line, 1, "record line 0 → dump line 1 (1-based)");
        assert_eq!(items[0].code, "alpha line", "open buffer's current line");
        assert_eq!(items[0].text, "note one");
        assert!(!items[0].orphaned);
        assert_eq!(items[1].line, 2, "record line 1 → dump line 2 (1-based)");
        assert_eq!(
            items[1].code, "GHOST ANCHOR",
            "orphaned: the stored anchor, NOT the line's current text"
        );
        assert!(items[1].orphaned);

        // Closed buffer: `code` falls back to the stored anchor.
        let mut s2 = store_with_project();
        s2.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/closed.rs".to_string(),
            line: 12,
            col: 0,
            anchor: "fn closed() {".to_string(),
            text: "closed note".to_string(),
            orphaned: false,
        }));
        s2.sync_notes_from_doc();
        let items2 = s2.annotations_for_dump();
        assert_eq!(items2.len(), 1);
        assert_eq!(items2[0].line, 13, "1-based");
        assert_eq!(items2[0].code, "fn closed() {", "closed buffer → stored anchor");
    }

    /// The rendered-row map round-trips buffer_line ↔ rendered_row through
    /// the STORE's file_view_rows (the real viewport, interleaved note
    /// rows), and mouse_click_position maps a code row under an annotation
    /// to the right buffer line.
    #[test]
    fn notes_row_map_store_and_click_mapping() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann7.rs", "c0\nc1\nc2\nc3\nc4\n");
        // Two annotations: line 1 and line 3 (both note rows visible).
        for (line, note) in [(1, "n1"), (3, "n3")] {
            s.notes_doc.entries.push(NotesEntry::Record(Annotation {
                syntax: None,
                path: "src/ann7.rs".to_string(),
                line,
                col: 0,
                anchor: format!("c{line}"),
                text: note.to_string(),
                orphaned: false,
            }));
        }
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);
        let rows = s.file_view_rows();
        // Row list (annotations-render-fold: each note row sits directly
        // ABOVE its own code row): c0, note1, c1, c2, note3, c3, c4, (empty
        // last line from the trailing newline — ropey's len_lines counts
        // it).
        let shape: Vec<(usize, bool)> = rows
            .iter()
            .map(|r| (r.line, r.is_note))
            .collect();
        assert_eq!(
            shape,
            vec![
                (0, false),
                (1, true),
                (1, false),
                (2, false),
                (3, true),
                (3, false),
                (4, false),
                (5, false)
            ],
            "row shape (note above its code row): {shape:?}"
        );
        // The note row sits directly ABOVE its own code row (the ordering
        // discriminator: the old note-below shape would fail this).
        for (i, r) in rows.iter().enumerate() {
            if r.is_note {
                let next_code = rows[i + 1..].iter().find(|r| !r.is_note).expect("a code row after a note");
                assert_eq!(next_code.line, r.line, "note row {i} must sit directly above its own code row: {shape:?}");
            }
        }
        // Marker flags: code rows 1 and 3 annotated, others not.
        assert!(rows[2].annotated && rows[5].annotated);
        assert!(!rows[0].annotated && !rows[3].annotated);
        // Both directions of the map.
        for line in 0..6 {
            let r = FileViewRow::row_for_line(&rows, line).unwrap();
            assert_eq!(FileViewRow::line_for_row(&rows, r), Some(line));
        }
        assert_eq!(FileViewRow::line_for_row(&rows, 1), Some(1), "note row → anchored line");
        assert_eq!(FileViewRow::line_for_row(&rows, 4), Some(3));
        // Total rendered rows = 6 code + 2 note.
        assert_eq!(s.file_view_total_rows(), 8);
        // Click mapping: rendered row 5 (c3, the code row DIRECTLY UNDER its
        // note row at row 4) and rendered row 1 (the note row above c1) both
        // map to their line; the old dense math (scroll_top + row) would
        // have sent row 5 to line 5.
        let mut s2 = s;
        s2.mouse_click_position(5, 0); // c3
        assert_eq!(s2.point_line(), 3, "click on c3's rendered row → line 3");
        s2.mouse_click_position(1, 0); // note row above c1
        assert_eq!(s2.point_line(), 1, "click on a note row → anchored line");
        // C-c a h (the toggle) hides the note rows: the map collapses back
        // to 1:1, the marker flags stay (and note_rows_folded flips — the
        // UI's margin-arrow state).
        s2.annotate_toggle();
        let rows2 = s2.file_view_rows();
        assert_eq!(rows2.len(), 6, "note rows hidden");
        assert!(rows2[1].annotated && rows2[3].annotated, "markers stay");
        assert!(s2.note_rows_folded(), "fold state reads folded");
        assert_eq!(s2.file_view_total_rows(), 6);
        // toggle again: back to shown.
        s2.annotate_toggle();
        assert!(!s2.note_rows_folded(), "fold state reads shown");
        assert_eq!(s2.file_view_rows().len(), 8);
    }

    /// C-n/C-p across a virtual note row: the point walks BUFFER lines
    /// (never a note row), so from the annotated line C-n lands on the
    /// NEXT CODE line and C-p returns — the status position display is the
    /// buffer line (map correctness).
    #[test]
    fn notes_point_motion_crosses_note_rows_on_code_lines() {
        let mut s = store_with_project();
        let content: String = (0..30).map(|i| format!("k{i}\n")).collect();
        open_ann_file(&mut s, "src/ann8.rs", &content);
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/ann8.rs".to_string(),
            line: 1,
            col: 0,
            anchor: "k1".to_string(),
            text: "note".to_string(),
            orphaned: false,
        }));
        s.sync_notes_from_doc();
        // Point on the annotated line (1). C-n → line 2 (the CODE line,
        // never "stuck" on the note row); the position display is the
        // BUFFER line (map correctness).
        s.set_point_line(1);
        s.point_down();
        assert_eq!(s.point_line(), 2, "C-n crosses the note row → next code line");
        assert_eq!(
            s.file_view_position_display(),
            "L3,6%",
            "status shows the code line: {}",
            s.file_view_position_display()
        );
        s.point_up();
        assert_eq!(s.point_line(), 1, "C-p back onto the annotated line");
        assert_eq!(s.file_view_position_display(), "L2,3%");
        // The rendered slice has 4 rows in the annotated region (3 code +
        // 1 note) and the cursor row for point line 1 is 2 (the note row
        // now comes BEFORE it — annotations-render-fold; it was 1 when the
        // note rendered after).
        let rows = s.file_view_rows();
        assert_eq!(FileViewRow::row_for_line(&rows, 1), Some(2));
        assert!(rows[1].is_note, "the note row sits above the code row");
        assert_eq!(rows[2].line, 1, "the code row is directly below its note");
    }

    /// plan 005 issue 02b (round 2) + 02c (span fill): the budget counts
    /// are FINAL. The all-lines-annotated window still emits the point's
    /// code row, never exceeds `viewport_lines` rendered rows, and the
    /// emitted-range note recount keeps `code + note <= viewport` even
    /// when many records share one line (where the full-window note
    /// count underestimates the advanced range's note count). 02c: the
    /// span is the LARGEST that fits, so the all-annotated repro fills
    /// the canvas (10 code + 10 note rows of 21) instead of collapsing
    /// to a 1-row span.
    #[test]
    fn notes_all_lines_annotated_budget_is_final() {
        // Leg 1 (the repro): 25-line file, ALL 25 lines annotated,
        // viewport 21, scroll_top 0 → the largest fitting span is 10
        // (10 + 10 notes = 20 <= 21; 11 + 11 = 22 > 21): 10 code rows +
        // 10 note rows — the canvas is FILLED, not just non-blank, and
        // the point's line is drawn.
        let mut s = store_with_project();
        let content: String = (0..25).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
        open_ann_file(&mut s, "src/ann10.rs", &content);
        for line in 0..25 {
            s.notes_doc.entries.push(NotesEntry::Record(Annotation {
                syntax: None,
                path: "src/ann10.rs".to_string(),
                line,
                col: 0,
                anchor: format!("line{line}"),
                text: "note".to_string(),
                orphaned: false,
            }));
        }
        s.sync_notes_from_doc();
        s.set_viewport_lines(21);
        s.set_scroll_top(0);
        s.set_point_line(0);
        let rows = s.file_view_rows();
        assert!(!rows.is_empty(), "view must not blank: {rows:?}");
        assert!(
            rows.len() <= 21,
            "budget exceeded: {} rows",
            rows.len()
        );
        // Canvas FILL (02c): the emitted count is close to viewport_lines
        // (>= 18 of 21), specifically the 10 code + 10 note shape.
        assert!(
            rows.len() >= 18,
            "view under-fills the canvas: {} rows",
            rows.len()
        );
        assert_eq!(rows.len(), 20, "10 code + 10 note rows of 21");
        assert_eq!(rows.iter().filter(|r| !r.is_note).count(), 10);
        assert_eq!(rows.iter().filter(|r| r.is_note).count(), 10);
        // The point's line IS drawn (as a code row, with its text intact;
        // annotations-render-fold: its note row sits directly ABOVE it, so
        // the code row is at index 1, not 0).
        assert_eq!(FileViewRow::row_for_line(&rows, 0), Some(1));
        assert!(!rows[1].is_note);
        assert_eq!(rows[1].text, "line0");
        assert!(rows[1].annotated);
        // Every note row sits directly ABOVE its own code row (the nearest
        // subsequent code row is its own line — the ordering discriminator:
        // this fails under the old note-below ordering).
        for (i, r) in rows.iter().enumerate() {
            if r.is_note {
                let next_code = rows[i + 1..].iter().find(|r| !r.is_note).expect("a code row after a note");
                assert_eq!(next_code.line, r.line);
            }
        }

        // Leg 2: the point is NOT at the window top: the span is 10
        // (largest that fits), so start advances to keep the point's
        // line drawn — the point is the LAST code row (line 10 at index
        // 18), the window still fills (10 code + 10 note rows), and the
        // view is not blank.
        let mut s2 = store_with_project();
        let content: String = (0..25).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
        open_ann_file(&mut s2, "src/ann11.rs", &content);
        for line in 0..25 {
            s2.notes_doc.entries.push(NotesEntry::Record(Annotation {
                syntax: None,
                path: "src/ann11.rs".to_string(),
                line,
                col: 0,
                anchor: format!("line{line}"),
                text: "note".to_string(),
                orphaned: false,
            }));
        }
        s2.sync_notes_from_doc();
        s2.set_viewport_lines(21);
        s2.set_scroll_top(0);
        s2.set_point_line(10);
        let rows = s2.file_view_rows();
        assert!(!rows.is_empty(), "view must not blank: {rows:?}");
        assert_eq!(
            FileViewRow::row_for_line(&rows, 10),
            Some(19),
            "point's line is the last code row (note rows above shift it to index 19): {:?}",
            rows.iter().map(|r| (r.line, r.is_note)).collect::<Vec<_>>()
        );
        assert_eq!(rows[19].text, "line10");
        assert!(!rows[19].is_note);
        assert_eq!(rows.len(), 20, "advanced window still fills: 10 + 10");
        // The emitted window's first buffer line (1) is above scroll_top
        // (0) — the \u{2191} indicator keys off rows[0].line.
        assert_eq!(rows[0].line, 1, "window advanced above scroll_top");

        // Leg 3 (the re-overflow): 22 records on ONE line (more than the
        // viewport) — no span >= 6 fits (6 + 22 > 21), so the span is 5
        // (lines 0..5 hold no notes); the advanced range [1, 6) still
        // holds all 22 notes. The emitted-range recount caps the note
        // rows so code + note <= 21 always (5 code + 16 note = 21).
        let mut s3 = store_with_project();
        let content: String = (0..30).map(|i| format!("k{i}")).collect::<Vec<_>>().join("\n");
        open_ann_file(&mut s3, "src/ann12.rs", &content);
        for i in 0..22 {
            s3.notes_doc.entries.push(NotesEntry::Record(Annotation {
                syntax: None,
                path: "src/ann12.rs".to_string(),
                line: 5,
                col: 0,
                anchor: "k5".to_string(),
                text: format!("note {i}"),
                orphaned: false,
            }));
        }
        s3.sync_notes_from_doc();
        s3.set_viewport_lines(21);
        s3.set_scroll_top(0);
        s3.set_point_line(5);
        let rows = s3.file_view_rows();
        assert_eq!(rows.len(), 21, "5 code rows + 16 capped note rows (<= 21)");
        // annotations-render-fold: the 16 capped note rows (16 of the 22 on
        // line 5 — the budget is final) sit ABOVE their code row, so the
        // code row is at index 4 + 16 = 20 (4 code rows for lines 1–4, then
        // the notes, then the code row last).
        assert_eq!(FileViewRow::row_for_line(&rows, 5), Some(20), "point's line drawn");
        assert_eq!(rows.iter().filter(|r| r.is_note).count(), 16);

        // Leg 4 (non-degenerate regression): a single note in a 31-line
        // file, viewport 21 → the span stays 20 code rows + the 1 note
        // (the pre-round-2 behavior, unchanged): the note row is drawn.
        let mut s4 = store_with_project();
        let content: String = (0..30).map(|i| format!("k{i}")).collect::<Vec<_>>().join("\n");
        open_ann_file(&mut s4, "src/ann13.rs", &content);
        s4.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/ann13.rs".to_string(),
            line: 2,
            col: 0,
            anchor: "k2".to_string(),
            text: "note".to_string(),
            orphaned: false,
        }));
        s4.sync_notes_from_doc();
        s4.set_viewport_lines(21);
        s4.set_scroll_top(0);
        s4.set_point_line(2);
        let rows = s4.file_view_rows();
        let shape: Vec<(usize, bool)> = rows.iter().map(|r| (r.line, r.is_note)).collect();
        assert_eq!(
            shape,
            vec![
                (0, false),
                (1, false),
                (2, true),
                (2, false),
                (3, false),
                (4, false),
                (5, false),
                (6, false),
                (7, false),
                (8, false),
                (9, false),
                (10, false),
                (11, false),
                (12, false),
                (13, false),
                (14, false),
                (15, false),
                (16, false),
                (17, false),
                (18, false),
                (19, false)
            ],
            "20 code rows + 1 note = 21: {shape:?}"
        );
    }

    /// The status line shows the current file's annotation count (and an
    /// edit-mode save of the anchored file re-anchors in the same pass).
    #[test]
    fn notes_status_count_and_save_reanchors() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann9.rs", "p0\np1\n");
        assert_eq!(s.annotation_count_display(), "");
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/ann9.rs".to_string(),
            line: 1,
            col: 0,
            anchor: "p1".to_string(),
            text: "a".to_string(),
            orphaned: false,
        }));
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/ann9.rs".to_string(),
            line: 0,
            col: 0,
            anchor: "p0".to_string(),
            text: "b".to_string(),
            orphaned: false,
        }));
        assert_eq!(s.annotation_count_display(), "2 notes");
        // Edit-mode save re-anchors: flip the file to edit mode, change
        // p1's line content elsewhere… simplest: the buffer already holds
        // the anchor at line 1, so save is a no-op re-anchor (count stable).
        let key = s.buffers.current().unwrap().to_string();
        s.buffers.get_mut(&key).unwrap().editable = true;
        s.save_buffer();
        assert_eq!(s.annotation_count_display(), "2 notes", "save keeps the records");
        // Now delete the anchor line out-of-band + save: p0 gone → orphan
        // flag, record NOT lost.
        let key = s.buffers.current().unwrap().to_string();
        s.buffers.get_mut(&key).unwrap().rope = Rope::from_str("x\np1\n");
        // plan 016 issue 03: the flag is DERIVED from the saved-state
        // marker — out-of-band text changed without a save is the
        // no-evidence state (`None` proves nothing → modified).
        s.buffers.get_mut(&key).unwrap().saved_marker = None;
        s.save_buffer();
        let recs = ann_records(&s);
        assert_eq!(recs.len(), 2, "orphaned records are not lost");
        let o = recs.iter().find(|a| a.anchor == "p0").unwrap();
        assert!(o.orphaned, "p0's anchor text is gone → orphaned");
        let k = recs.iter().find(|a| a.anchor == "p1").unwrap();
        assert!(!k.orphaned);
        assert_eq!(s.annotation_count_display(), "2 notes");
    }

    // ── 011-02: per-language import walks (bare-symbol hints) ───────


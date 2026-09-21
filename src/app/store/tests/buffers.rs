use super::*;

    #[test]
    fn quit_prompt_unmodified_fast_path() {
        let (_dir, mut s) = notes_store();
        // No typed edits: notes is open but UNMODIFIED → immediate quit,
        // no prompt (the existing q-quit behavior is preserved).
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit, "unmodified buffers must quit immediately");
        assert!(!s.quit_prompt_active());
        assert!(
            !s.message.contains("Save this buffer"),
            "no prompt must be rendered: {:?}",
            s.message
        );
    }

    #[test]
    fn quit_prompt_y_saves_and_quits() {
        let (dir, mut s) = notes_store();
        let notes_path = dir.path().join(".redline-notes.md");
        s.key_event(key("H"));
        s.key_event(key("i"));
        assert!(s.buffers.current_buffer().unwrap().locally_modified);

        // Interception: the prompt names the modified buffer's path.
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(!s.quit, "the prompt must hold the quit");
        assert!(s.quit_prompt_active());
        let expected = format!("Save this buffer: {}? (y, n, !, C-g)", notes_path.display());
        assert_eq!(s.message, expected, "prompt text mismatch");
        assert_eq!(s.quit_prompt_buffer().unwrap(), notes_path.display().to_string());

        // `y`: saved to disk, then the last answer quits.
        s.key_event(key("y"));
        assert!(s.quit, "y on the last modified buffer must quit");
        assert!(!s.quit_prompt_active());
        let on_disk = std::fs::read_to_string(&notes_path).unwrap();
        assert!(on_disk.contains("Hi"), "y must write the edit to disk");
        assert!(
            !s.buffers.current_buffer().unwrap().locally_modified,
            "save must clear locally_modified"
        );
    }

    #[test]
    fn quit_prompt_n_skips_and_quits() {
        let (dir, mut s) = notes_store();
        let notes_path = dir.path().join(".redline-notes.md");
        s.key_event(key("x"));
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit_prompt_active());

        // `n`: knowingly discard → quit, the file is left unwritten
        // (open_notes created it with the seed line; the edit must not land).
        s.key_event(key("n"));
        assert!(s.quit, "n on the last modified buffer must quit");
        let on_disk = std::fs::read_to_string(&notes_path).unwrap();
        assert!(!on_disk.contains("x"), "n must NOT write the edit to disk");
        assert!(
            s.buffers.current_buffer().unwrap().locally_modified,
            "the skipped buffer keeps its local text (still modified in memory)"
        );
    }

    #[test]
    fn quit_prompt_bang_saves_all_remaining_then_quits() {
        let (dir, mut s) = notes_store();
        let (extra_key, extra) = two_modified_buffers(dir.path(), &mut s);
        let notes_path = dir.path().join(".redline-notes.md");

        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit_prompt_active());

        // `!`: save this and ALL remaining, then quit (no per-buffer answers).
        s.key_event(key("!"));
        assert!(s.quit, "! must quit after saving everything");
        assert!(!s.quit_prompt_active());
        assert!(extra.exists());
        let extra_disk = std::fs::read_to_string(&extra).unwrap();
        assert!(extra_disk.contains("old"), "! must save the extra buffer");
        let notes_disk = std::fs::read_to_string(&notes_path).unwrap();
        assert!(notes_disk.contains("z"), "! must save the notes buffer");
        assert!(!s.buffers.get(&extra_key).unwrap().locally_modified);
        assert!(!s.buffers.current_buffer().unwrap().locally_modified);
    }

    #[test]
    fn quit_prompt_asks_oldest_first_one_at_a_time() {
        let (_dir, mut s) = notes_store();
        let (_extra_key, extra) = two_modified_buffers(_dir.path(), &mut s);
        // notes was opened BEFORE extra.md → it is the OLDER of the two
        // modified buffers and must be asked first (extra.md, inserted
        // after, sits at the MRU front → last in the oldest-first walk).
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit_prompt_active());
        assert!(
            s.quit_prompt_buffer().unwrap().ends_with(".redline-notes.md"),
            "the oldest modified buffer must be asked first"
        );

        // `n` skips it; the extra.md prompt appears next (oldest-first walk).
        s.key_event(key("n"));
        assert!(!s.quit, "only one buffer answered so far");
        assert!(s.quit_prompt_active());
        assert_eq!(s.quit_prompt_buffer().unwrap(), extra.display().to_string());
        s.key_event(key("n"));
        assert!(s.quit, "the last answer must quit");
        assert!(!s.quit_prompt_active());
    }

    #[test]
    fn quit_prompt_c_g_cancels_the_whole_quit() {
        let (_dir, mut s) = notes_store();
        s.key_event(key("x"));
        let text_before = s.buffers.current_buffer().unwrap().text();

        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit_prompt_active());
        // C-g: cancel the quit entirely — prompt gone, app stays alive.
        s.key_event(key("C-g"));
        assert!(!s.quit, "C-g must cancel the quit");
        assert!(!s.quit_prompt_active());
        assert_eq!(s.message, "cancel");
        assert_eq!(
            s.buffers.current_buffer().unwrap().text(),
            text_before,
            "buffer content must be intact after C-g"
        );
        assert!(s.buffers.current_buffer().unwrap().locally_modified);

        // Quitting again re-enters the prompt (the buffer is still modified);
        // a bare answer finishes it.
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit_prompt_active());
        s.key_event(key("n"));
        assert!(s.quit);
    }

    #[test]
    fn quit_prompt_save_failure_reports_and_reprompts_same_buffer() {
        let (dir, mut s) = notes_store();
        let notes_path = dir.path().join(".redline-notes.md");
        // Make the write fail: the notes file becomes read-only.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&notes_path, std::fs::Permissions::from_mode(0o444)).unwrap();
        }
        s.key_event(key("x"));
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));

        // `y` fails: the error is reported, the SAME buffer is re-offered,
        // and the prompt decision line is redisplayed alongside the error.
        let prompt = format!(
            "Save this buffer: {}? (y, n, !, C-g)",
            notes_path.display()
        );
        s.key_event(key("y"));
        assert!(!s.quit, "a failed save must not quit");
        assert!(s.quit_prompt_active(), "the prompt must stay up after a failed save");
        assert!(
            s.message.contains("save failed"),
            "the error must be reported: {:?}",
            s.message
        );
        assert!(
            s.message.contains(&prompt),
            "the prompt decision line must be redisplayed alongside the error: {:?}",
            s.message
        );
        assert_eq!(
            s.quit_prompt_buffer(),
            Some(notes_path.display().to_string()),
            "the failed buffer must remain the offered one"
        );
        // The `!` failure branch redisplay the prompt the same way: `!`
        // re-saves the (still read-only) head buffer, fails, and re-prompts
        // from it with the decision line visible.
        s.key_event(key("!"));
        assert!(!s.quit, "a failed `!` save must not quit");
        assert!(s.quit_prompt_active(), "the prompt must stay up after a failed `!`");
        assert!(
            s.message.contains("save failed") && s.message.contains(&prompt),
            "error + prompt must both be visible after a failed `!`: {:?}",
            s.message
        );
        // Re-prompted buffer still answered by the state machine: `n` quits.
        s.key_event(key("n"));
        assert!(s.quit);
    }

    #[test]
    fn quit_prompt_snapshot_is_frozen_at_interception() {
        let (_dir, mut s) = notes_store();
        let extra = _dir.path().join("after.md");
        std::fs::write(&extra, "a\n").unwrap();
        let mtime = std::fs::metadata(&extra)
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let extra_key = s
            .buffers
            .insert_rope(Some(extra.clone()), Rope::from_str("a\n"), mtime, true);

        // Modify notes only; a SECOND buffer becomes modified AFTER the
        // interception (it must never enter the prompt).
        s.key_event(key("m"));
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit_prompt_active());
        s.mark_locally_modified(&extra_key);

        // A buffer saved mid-prompt must not re-appear either: save notes
        // through the API, then answer `n` — exactly ONE prompt total.
        let notes_key = s.buffers.current().unwrap().to_string();
        assert!(s.save_buffer_key(&notes_key));
        assert!(!s.buffers.get(&notes_key).unwrap().locally_modified);
        s.key_event(key("n"));
        assert!(s.quit, "one snapshot entry → the next answer quits");
        assert!(!s.quit_prompt_active());
        // The post-interception buffer was never asked about.
        assert!(s.buffers.get(&extra_key).unwrap().locally_modified);
    }

    #[test]
    fn quit_prompt_unknown_keys_are_swallowed() {
        let (_dir, mut s) = notes_store();
        s.key_event(key("x"));
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        let prompt = s.message.clone();
        // Stray keys during the prompt: no "unbound key" echo, no state
        // change (typing must not leak into the buffer either).
        s.key_event(key("z"));
        s.key_event(key("C-SPC"));
        s.key_event(key("RET"));
        assert!(s.quit_prompt_active(), "stray keys must not exit the prompt");
        assert!(!s.quit);
        assert_eq!(s.message, prompt, "the prompt must stay on screen");
        assert!(s.buffers.current_buffer().unwrap().text().ends_with("x"));
    }

    // ── plan 005 issue 01: file edit mode (C-x C-q / C-x C-s) ─────────

    #[test]
    fn toggle_read_only_flips_file_buffer_on_and_off() {
        let (_dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        assert!(!s.buffers.get(&bufk).unwrap().editable,
            "file buffers must start read-only");
        assert_eq!(s.buffers.get(&bufk).unwrap().mode, BufferMode::Annotation,
            "file buffers must start in Annotation mode");
        assert_eq!(s.buffer_mode_display(), "Read-only");

        // 015-02 re-pin (was: `buffer_mode_display() == "Edit"`): C-x C-q
        // is the MODE toggle — entering Accurate makes the buffer
        // editable, and the status line shows the mode word.
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        let buf = s.buffers.get(&bufk).unwrap();
        assert_eq!(buf.mode, BufferMode::Accurate,
            "C-x C-q must enter the Accurate mode");
        assert!(buf.editable, "Accurate ⟹ editable");
        assert_eq!(s.buffer_mode_display(), "Accurate");
        assert!(s.message.contains("editable"), "msg: {}", s.message);

        // 015-02 re-pin (was: flip back to `editable == false` read-only):
        // leaving Accurate returns the buffer to its baseline — for a
        // plain file buffer that is read-only in Annotation mode.
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        let buf = s.buffers.get(&bufk).unwrap();
        assert_eq!(buf.mode, BufferMode::Annotation,
            "second C-x C-q must return to Annotation mode");
        assert!(!buf.editable,
            "leaving Accurate must return a file buffer to its read-only baseline");
        assert_eq!(s.buffer_mode_display(), "Read-only");
    }

    #[test]
    fn toggle_read_only_also_flips_a_real_file_backed_notes_buffer() {
        // DECISION (2026-09-18, orchestrator): a notes buffer is a real
        // file-backed buffer, so `C-x C-q` toggles it like any other file
        // (emacs `toggle-read-only` is buffer-agnostic). The earlier spec
        // wording said notes "no-op"; that was the ambiguous half and is
        // superseded. The behavior is recoverable (toggle back) and now
        // pinned by this test.
        // 015-02 re-pin (was: toggle → read-only → editable): the notes
        // buffer is INHERENTLY editable (that is how annotations are
        // typed), so C-x C-q toggles its MODE, not its editability:
        // Annotation ⇄ Accurate, `editable` stays true throughout.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        let notes_key = s.buffers.current().unwrap().to_string();
        assert!(s.buffers.get(&notes_key).unwrap().editable,
            "notes start editable");
        assert_eq!(s.buffers.get(&notes_key).unwrap().mode, BufferMode::Annotation,
            "notes start in Annotation mode");

        s.toggle_read_only();
        let buf = s.buffers.get(&notes_key).unwrap();
        assert_eq!(buf.mode, BufferMode::Accurate,
            "C-x C-q enters the Accurate mode on the notes buffer");
        assert!(buf.editable, "Accurate ⟹ editable");

        s.toggle_read_only();
        let buf = s.buffers.get(&notes_key).unwrap();
        assert_eq!(buf.mode, BufferMode::Annotation,
            "and back into Annotation mode");
        assert!(buf.editable,
            "the notes buffer's baseline stays editable (annotations are typed there)");
    }

    #[test]
    fn toggle_read_only_refuses_external_buffer() {
        // 006-02b item 1: the C-x C-q override must NEVER turn an external
        // (registry / tooling) buffer editable — it is a cache shared by
        // every project on the machine.
        let (_dir, _ext, mut s) = store_with_external_buffer();
        let key = s.buffers.current().unwrap().to_string();
        assert!(!s.buffers.get(&key).unwrap().editable,
            "external buffers start read-only");
        s.toggle_read_only();
        assert!(!s.buffers.get(&key).unwrap().editable,
            "C-x C-q must NOT turn an external buffer editable");
        assert!(s.message.contains("external buffer is read-only"), "msg: {}", s.message);
        // Plan-005 semantics intact: a PROJECT file in the same session
        // still toggles into edit mode.
        std::fs::create_dir_all(_dir.path().join("src")).unwrap();
        std::fs::write(_dir.path().join("src/p.rs"), "fn p() {}\n").unwrap();
        s.open_path("src/p.rs");
        s.toggle_read_only();
        let pkey = s.buffers.current().unwrap().to_string();
        assert!(s.buffers.get(&pkey).unwrap().editable,
            "a project file toggles into edit mode as before (msg: {})", s.message);
    }

    #[test]
    fn buffer_is_project_owned_classifies_external_scratch_and_project() {
        // 006-02b item 1: the ownership notion itself — external paths
        // refuse, scratch and project files (incl. the notes file) pass.
        let (dir, _ext, mut s) = store_with_external_buffer();
        let ext_key = s.buffers.current().unwrap().to_string();
        assert!(!s.buffer_is_project_owned(&ext_key), "registry source: not owned");
        // 06a: scratch no longer exists at boot — create it (the explicit
        // affordance) before checking its ownership.
        s.open_scratch();
        assert!(s.buffer_is_project_owned(SCRATCH_NAME), "scratch (no path): owned as today");
        // The notes file lives under the root: owned (the 005 decision —
        // notes keep their edit-mode semantics).
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        s.open_notes();
        let notes_key = s.notes_key().unwrap();
        assert!(s.buffer_is_project_owned(&notes_key), "notes file (under root): owned");
    }

    #[test]
    fn save_buffer_key_refuses_external_even_when_editable() {
        // 006-02b item 1: even if an external buffer somehow reached edit
        // mode (a future bug), the save must be refused — the write would
        // corrupt a cache shared by every project on the machine.
        let (_dir, ext, mut s) = store_with_external_buffer();
        let key = s.buffers.current().unwrap().to_string();
        s.buffers.get_mut(&key).unwrap().editable = true; // the hypothetical future bug
        assert!(!s.save_buffer_key(&key), "the save must be refused");
        assert!(s.message.contains("external buffer is read-only"), "msg: {}", s.message);
        let abs = ext.path().join("registry_src.rs");
        assert_eq!(std::fs::read_to_string(&abs).unwrap(), "pub fn spawn<F>(f: F) {}\n",
            "the external file is untouched");
        // Plan-005 path still works: a project file passes the gate and
        // writes to disk.
        std::fs::create_dir_all(_dir.path().join("src")).unwrap();
        std::fs::write(_dir.path().join("src/p.rs"), "old\n").unwrap();
        s.open_path("src/p.rs");
        s.toggle_read_only(); // into edit mode (project file: allowed)
        let pkey = s.buffers.current().unwrap().to_string();
        assert!(s.save_buffer_key(&pkey), "a project file saves (msg: {})", s.message);
    }

    #[test]
    fn save_buffer_key_still_writes_previous_project_buffer_after_switch() {
        // 006-02b item 1: the guard refuses only EXTERNAL (registry / tooling)
        // buffers. A previous project's buffer (no longer under the new root
        // after a switch) is the user's own file — C-x C-s still writes it.
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        for d in [&dir1, &dir2] {
            std::fs::write(d.path().join("Cargo.toml"), "[package]\n").unwrap();
            std::fs::create_dir_all(d.path().join("src")).unwrap();
        }
        std::fs::write(dir1.path().join("src/a.rs"), "fn a() {}\n").unwrap();
        let mut s = store(dir1.path());
        s.open_path("src/a.rs");
        s.toggle_read_only(); // into edit mode (under root1: owned)
        let key1 = s.buffers.current().unwrap().to_string();
        assert!(s.buffers.get(&key1).unwrap().editable);
        // Switch to a second project: dir1's buffer is no longer under the
        // current root, but it is not an external landing either.
        s.switch_project_root(dir2.path().to_str().unwrap());
        let root2 = s.project.as_ref().unwrap().root.clone();
        assert!(!s.buffers.get(&key1).unwrap().path.as_ref().unwrap().starts_with(&root2),
            "precondition: the buffer is outside the new root");
        assert!(s.buffer_is_project_owned(&key1), "previous-project file is still owned");
        assert!(s.save_buffer_key(&key1), "previous-project file still saves (msg: {})", s.message);
    }

    #[test]
    fn edit_mode_file_buffer_accepts_typing_and_set_locally_modified() {
        let (_dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        // The file buffer in edit mode takes the same self-insert path as
        // notes (a printable that binds nothing appends at the end).
        assert!(s.insert_text("X"), "edit-mode file buffer must accept typing");
        assert!(
            s.buffers.get(&bufk).unwrap().locally_modified,
            "an edit must set locally_modified"
        );
        assert_eq!(s.buffers.get(&bufk).unwrap().text(), "fn old() {}\nX");
        // Backspace goes through the shared bounded-edit path too.
        s.notes_backspace();
        assert_eq!(s.buffers.get(&bufk).unwrap().text(), "fn old() {}\n");
    }

    #[test]
    fn save_buffer_clears_locally_modified_and_writes_disk() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        s.insert_text("X");
        assert!(s.buffers.get(&bufk).unwrap().locally_modified);

        s.key_event(key("C-x"));
        s.key_event(key("C-s"));
        let on_disk = std::fs::read_to_string(dir.path().join("src/f.rs")).unwrap();
        assert_eq!(on_disk, "fn old() {}\nX", "C-x C-s must write the edit to disk");
        let buf = s.buffers.get(&bufk).unwrap();
        assert!(!buf.locally_modified, "save must clear locally_modified");
        assert!(!buf.changed_on_disk, "save must clear changed_on_disk");
        assert!(s.message.contains("wrote"), "msg: {}", s.message);
    }

    #[test]
    fn save_read_only_buffer_refuses() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        // Still read-only: C-x C-s must refuse and leave the disk untouched.
        s.key_event(key("C-x"));
        s.key_event(key("C-s"));
        assert!(s.message.contains("read-only"), "msg: {}", s.message);
        assert!(!s.buffers.get(&bufk).unwrap().locally_modified);
        let on_disk = std::fs::read_to_string(dir.path().join("src/f.rs")).unwrap();
        assert_eq!(on_disk, "fn old() {}\n");
    }

    #[test]
    fn toggle_back_to_read_only_with_unsaved_edits_arms_confirm() {
        let (_dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        s.insert_text("X");

        // C-x C-q while the buffer is editable+modified: no flip, confirm armed.
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        assert!(s.toggle_ro_active(), "the discard confirm must be armed");
        assert!(s.message.contains("Discard unsaved edits"), "msg: {}", s.message);
        assert!(s.buffers.get(&bufk).unwrap().editable,
            "the flip must not happen before the answer");
        // Stray keys are swallowed and the text is untouched.
        s.key_event(key("z"));
        assert!(s.toggle_ro_active());
        assert_eq!(s.buffers.get(&bufk).unwrap().text(), "fn old() {}\nX");
    }

    #[test]
    fn toggle_ro_confirm_cancel_keeps_edit_mode_and_text() {
        for cancel_key in ["n", "C-g", "ESC"] {
            let (_dir, mut s) = file_buffer_store();
            let bufk = s.buffers.current().unwrap().to_string();
            s.key_event(key("C-x"));
            s.key_event(key("C-q"));
            s.insert_text("X");
            s.key_event(key("C-x"));
            s.key_event(key("C-q"));
            assert!(s.toggle_ro_active());
            s.key_event(key(cancel_key));
            assert!(!s.toggle_ro_active(), "{} must close the confirm", cancel_key);
            assert!(s.buffers.get(&bufk).unwrap().editable,
                "cancel must keep edit mode");
            assert!(!s.message.contains("read-only"), "msg: {}", s.message);
            assert_eq!(s.buffers.get(&bufk).unwrap().text(), "fn old() {}\nX",
                "cancel must keep the unsaved text");
        }
    }

    #[test]
    fn toggle_ro_confirm_accept_discards_and_makes_read_only() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        s.insert_text("X");
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        assert!(s.toggle_ro_active());

        s.key_event(key("y"));
        assert!(!s.toggle_ro_active());
        let buf = s.buffers.get(&bufk).unwrap();
        assert!(!buf.editable, "accept must make the buffer read-only");
        assert!(!buf.locally_modified, "accept must clear locally_modified");
        assert!(!buf.changed_on_disk);
        assert_eq!(buf.text(), "fn old() {}\n",
            "accept must re-read the on-disk content (the edit is discarded)");
        let on_disk = std::fs::read_to_string(dir.path().join("src/f.rs")).unwrap();
        assert_eq!(on_disk, "fn old() {}\n", "accept must not write to disk");
    }

    #[test]
    fn saved_path_suppresses_own_watcher_event() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        s.insert_text("X");
        s.key_event(key("C-x"));
        s.key_event(key("C-s"));
        assert!(!s.buffers.get(&bufk).unwrap().locally_modified);

        // The watcher's event for our own save must not flag the buffer.
        let path = dir.path().join("src/f.rs");
        s.apply_project_change(&change(vec![path.clone()]));
        let buf = s.buffers.get(&bufk).unwrap();
        assert!(!buf.changed_on_disk,
            "our own save must not flag changed_on_disk");
        assert_eq!(buf.text(), "fn old() {}\nX", "the buffer must be untouched");

        // The marker was consumed: a repeat event now behaves like a
        // normal external change (the buffer is in edit mode → locally
        // owned → the conflict marker lands instead of a silent clobber).
        s.apply_project_change(&change(vec![path]));
        assert!(
            s.buffers.get(&bufk).unwrap().changed_on_disk,
            "a second event after the suppression must conflict"
        );
    }

    #[test]
    fn genuine_external_write_after_save_still_conflicts() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        let path = dir.path().join("src/f.rs");
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        s.insert_text("X");
        s.key_event(key("C-x"));
        s.key_event(key("C-s"));

        // A genuinely later external write (new mtime) must NOT be
        // suppressed: the mtime recorded at save no longer matches.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&path, "fn old() {}\n\nexternal\n").unwrap();
        s.apply_project_change(&change(vec![path]));
        let buf = s.buffers.get(&bufk).unwrap();
        assert!(
            buf.changed_on_disk,
            "a later external write after our save must flag changed_on_disk"
        );
        assert_eq!(buf.text(), "fn old() {}\nX",
            "the buffer must not be auto-clobbered while in edit mode");
    }

    #[test]
    fn read_only_file_buffer_still_auto_reloads_on_external_change() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        let path = dir.path().join("src/f.rs");
        // Untouched (read-only) file buffer: the existing auto-reload
        // behavior is unchanged by edit mode.
        std::fs::write(&path, "fn fresh() {}\n").unwrap();
        s.apply_project_change(&change(vec![path]));
        assert!(s.buffer_text().contains("fresh"), "auto-reload must land");
        assert!(!s.buffers.get(&bufk).unwrap().changed_on_disk);
        assert!(!s.buffers.get(&bufk).unwrap().editable);
    }

    #[test]
    fn edit_mode_without_edits_is_locally_owned() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        // Edit mode ON, zero edits: the reload guard now keys off the mode,
        // not just locally_modified.
        assert!(s.buffers.get(&bufk).unwrap().is_locally_owned());
        let path = dir.path().join("src/f.rs");
        std::fs::write(&path, "fn fresh() {}\n").unwrap();
        s.apply_project_change(&change(vec![path]));
        let buf = s.buffers.get(&bufk).unwrap();
        assert!(buf.changed_on_disk,
            "an external change while in edit mode must conflict, not auto-reload");
        assert_eq!(buf.text(), "fn old() {}\n",
            "the buffer content must not be reloaded under the cursor");
    }

    #[test]
    fn switch_buffer_and_kill_buffer() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_path("src/main.rs");
        store.open_path("src/lib.rs");
        assert_eq!(store.buffers.len(), 2); // 06a: no scratch

        // C-x b: switch to src/lib.rs.
        store.key_event(key("C-x"));
        store.key_event(key("b"));
        assert_eq!(store.picker_kind(), Some(PickerKind::Buffers));
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.display.as_str())
            .collect();
        assert!(names.contains(&"src/lib.rs"), "{names:?}");
        // picker-density: the buffer row is name-first — label = the
        // (marked) display name, detail = the absolute path (the buffer
        // key); display (the match target) is pinned above.
        let c = store
            .picker_filtered()
            .iter()
            .find(|(c, _)| c.display == "src/lib.rs")
            .map(|(c, _)| c.clone())
            .unwrap();
        assert_eq!(c.label, "src/lib.rs", "the buffer name is the label: {c:?}");
        assert_eq!(c.detail, c.name, "the absolute path is the detail: {c:?}");
        // Select the lib.rs row (find its index, drive selection there).
        let idx = names.iter().position(|n| *n == "src/lib.rs").unwrap();
        for _ in 0..idx {
            store.picker_select_next();
        }
        store.key_event(key("RET"));
        assert!(!store.picker_open());
        assert_eq!(store.view_name_display(), "src/lib.rs");

        // C-x k: kill it; the killed buffer disappears. 06a: no scratch
        // fallback — with main.rs still current the view stays on it.
        store.key_event(key("C-x"));
        store.key_event(key("k"));
        assert_eq!(store.picker_kind(), Some(PickerKind::KillBuffer));
        let idx = store
            .picker_filtered()
            .iter()
            .position(|(c, _)| c.display == "src/lib.rs")
            .unwrap();
        for _ in 0..idx {
            store.picker_select_next();
        }
        store.key_event(key("RET"));
        assert_eq!(store.buffers.len(), 1); // main.rs only (06a: no scratch)
        assert_eq!(store.view_name_display(), "src/main.rs");
        assert!(store.message.contains("killed src/lib.rs"));
    }

    #[test]
    fn list_buffers_view_and_buffer_list_keys() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_path("src/main.rs");

        store.dispatch("list-buffers", None).unwrap();
        assert_eq!(store.top_view(), ViewId::BufferList);
        let rows = store.buffer_rows();
        assert_eq!(rows.len(), 1, "06a: only the opened buffer, no scratch");
        assert!(rows.iter().any(|r| r.name == "src/main.rs" && r.current));
        assert!(!rows.iter().any(|r| r.name == "*scratch*"));

        // q closes the view back to the buffer view (main.rs replaced home
        // when it was opened; the table is non-empty, so it renders as the
        // buffer view).
        store.key_event(key("q"));
        assert_eq!(store.top_view(), ViewId::Buffer);
        store.dispatch("list-buffers", None).unwrap();
        store.key_event(key("RET")); // row 0 is the MRU buffer (main.rs)
        assert_eq!(store.top_view(), ViewId::Buffer);
        assert_eq!(store.view_name_display(), "src/main.rs");
    }

    // ── plan 004 issue 06a: empty buffer table + home view ─────────────

    /// Issue 05h: `n`/`p` move the selection exactly as `C-n`/`C-p` (no
    /// "unbound key" echo), and `d` kills the selected buffer via the
    /// existing `kill_buffer` path — the list stays open and the selection
    /// clamps to a valid row.
    #[test]
    fn buffer_list_n_p_d_keys() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_path("src/main.rs");
        store.open_path("src/lib.rs");
        store.open_path("src/main.rs"); // main.rs current; MRU: main, lib (06a)

        store.dispatch("list-buffers", None).unwrap();
        assert_eq!(store.top_view(), ViewId::BufferList);
        let rows = store.buffer_rows();
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["src/main.rs", "src/lib.rs"], "{names:?}");
        assert!(rows[0].current);

        // n/p move identically to C-n/C-p, with no unbound-key echo.
        store.key_event(key("C-n"));
        assert_eq!(store.buffer_list_selected(), 1);
        store.key_event(key("C-p"));
        assert_eq!(store.buffer_list_selected(), 0);
        store.key_event(key("n"));
        assert_eq!(store.buffer_list_selected(), 1);
        assert!(!store.message.contains("unbound key"), "`n` must not echo: {:?}", store.message);
        store.key_event(key("p"));
        assert_eq!(store.buffer_list_selected(), 0);

        // d kills the selected (non-current) buffer: the list stays open
        // and the selection clamps to the row that shifted into place.
        store.key_event(key("n")); // row 1: src/lib.rs
        store.key_event(key("d"));
        assert_eq!(store.top_view(), ViewId::BufferList, "d must not close the list");
        assert!(!store.quit);
        let rows = store.buffer_rows();
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["src/main.rs"], "{names:?}");
        assert!(store.message.contains("killed"), "{:?}", store.message);
        assert_eq!(store.buffer_list_selected(), 0, "selection clamps to a valid row");

        // d on the last (current) buffer kills it: the table is empty, the
        // selection clamps to row 0, and the list stays open (06a: no
        // scratch is created by the kill).
        store.key_event(key("d"));
        assert_eq!(store.buffer_rows().len(), 0);
        assert_eq!(store.buffer_list_selected(), 0);
        assert_eq!(store.top_view(), ViewId::BufferList);
        assert!(store.buffers.current().is_none());
        assert!(!store.buffers.list().iter().any(|&(k, _)| k == SCRATCH_NAME));

        // q closes the list: the empty table normalizes the top back to
        // home (close_view's re-normalization, 06a).
        store.key_event(key("q"));
        assert_eq!(store.top_view(), ViewId::Home);
        assert_eq!(store.render_view(), ViewId::Home, "empty table renders home");
    }

    #[test]
    fn insert_text_goes_to_current_buffer() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        // The scratch buffer is editable; file buffers are read-only (issue 03).
        store.open_scratch();
        assert!(store.insert_text("demo text\n"));
        assert!(store.buffer_text().contains("demo text"));
    }

    // ── issue 03 store-level tests (finding 9) ─────────────────────────

    #[test]
    fn reopen_invalidates_stale_buffer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "fn old() {}\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        assert!(s.buffer_text().contains("old"));
        // Modify the file on disk; ensure mtime differs.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(dir.path().join("src/t.rs"), "fn new() {}\n").unwrap();
        s.open_path("src/t.rs");
        assert!(
            s.buffer_text().contains("new"),
            "reopen must reload changed file"
        );
    }

    // ── issue 04 store-level tests (no notify needed) ─────────────────

    #[test]
    fn set_mark_sets_buffer_mark_and_echoes() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let key = s.buffers.current().unwrap().to_string();
        assert!(s.buffers.get(&key).unwrap().mark.is_none(), "no mark initially");
        s.set_mark();
        assert!(s.buffers.get(&key).unwrap().mark.is_some(), "mark must be set");
        assert_eq!(s.message, "Mark set");
    }

    #[test]
    fn region_byte_range_returns_normalized_range() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let key = s.buffers.current().unwrap().to_string();
        // Set mark at line 2 (byte offset of line 2's start).
        let line2_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(line2_byte);
        }
        // Point at line 5: the region's end is line 5's start.
        s.set_point_line(5);
        let range = s.region_byte_range().unwrap();
        let line5_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(5).unwrap();
        assert_eq!(range.0, line2_byte, "start = min(mark, point)");
        assert_eq!(range.1, line5_byte, "end = max(mark, point)");
    }

    #[test]
    fn region_byte_range_none_when_mark_not_set() {
        let (s, _dir) = notes_store_with_lines(10);
        assert!(s.region_byte_range().is_none());
    }

    #[test]
    fn region_size_bytes_returns_correct_size() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let key = s.buffers.current().unwrap().to_string();
        let line2_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(line2_byte);
        }
        s.set_point_line(5);
        let size = s.region_size_bytes().unwrap();
        assert!(size > 0, "region must have a positive size");
    }

    #[test]
    fn region_line_range_returns_correct_lines() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let key = s.buffers.current().unwrap().to_string();
        let line2_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(line2_byte);
        }
        s.set_point_line(5);
        let (start, end) = s.region_line_range().unwrap();
        assert_eq!(start, 2, "region starts at line 2");
        assert_eq!(end, 4, "region ends at line 4 (end byte is exclusive: line 5's start)");
    }

    #[test]
    fn c_g_clears_mark() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let bkey = s.buffers.current().unwrap().to_string();
        s.set_mark();
        assert!(s.buffers.get(&bkey).unwrap().mark.is_some());
        s.key_event(key("C-g"));
        assert!(s.buffers.get(&bkey).unwrap().mark.is_none(), "C-g must clear the mark");
    }

    #[test]
    fn exchange_point_and_mark_swaps_positions() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let key = s.buffers.current().unwrap().to_string();
        // Set mark at line 2.
        let line2_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(line2_byte);
        }
        // Point is at line 0.
        assert_eq!(s.point_line(), 0);
        s.exchange_point_and_mark();
        // After exchange: point should be at line 2, mark should be at line 0's byte.
        assert_eq!(s.point_line(), 2, "point must move to where mark was");
        let line0_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(0).unwrap();
        assert_eq!(s.buffers.get(&key).unwrap().mark, Some(line0_byte), "mark must be at old point");
    }

    #[test]
    fn exchange_point_and_mark_lands_mark_column() {
        // Regression (column-landings): `C-x C-x` took only
        // `try_byte_to_line(mark)` and landed col 0, dropping the
        // column the mark's byte offset encodes. The mark here is
        // MID-LINE (byte 3 of line 2); the existing fixture's mark was
        // a line-start byte (col 0) and cannot discriminate.
        let (mut s, _dir) = store_with_lines(5);
        let key = s.buffers.current().unwrap().to_string();
        let line2_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(line2_byte + 3); // mid-line: "lin|e2"
        }
        // Point is at line 0, col 0.
        s.exchange_point_and_mark();
        assert_eq!(s.point_line(), 2, "point must move to where the mark was");
        assert_eq!(s.point_col(), 3, "the mark's column must land — not the line start");
        assert_eq!(
            s.file_point().goal_col,
            3,
            "the landing column becomes the goal column (C-n/C-p hold it)"
        );
        // Existing semantics kept: the mark moves to the old point
        // (a line-start byte — region semantics, plan 004 issue 05b).
        let line0_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(0).unwrap();
        assert_eq!(s.buffers.get(&key).unwrap().mark, Some(line0_byte));
    }

    #[test]
    fn exchange_point_and_mark_lands_multibyte_mark_column() {
        // Pins the byte→char conversion: in "café omega" the 'o' of
        // "omega" sits at BYTE 6 of the line (é is 2 bytes) but CHAR 5.
        // A byte-based landing would put the cursor at col 6 (the 'm');
        // the old line-only landing at col 0.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "café omega\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        let key = s.buffers.current().unwrap().to_string();
        // Mark on the 'o' of "omega": byte 6 (c=0 a=1 f=2 é=3..4 ' '=5 o=6).
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(6);
        }
        s.exchange_point_and_mark();
        assert_eq!(s.point_line(), 0);
        assert_eq!(s.point_col(), 5, "char index 5 — not byte index 6, not col 0");
    }

    #[test]
    fn exchange_point_and_mark_noop_when_no_mark() {
        let (mut s, _dir) = notes_store_with_lines(10);
        assert_eq!(s.scroll_top(), 0);
        s.exchange_point_and_mark();
        assert_eq!(s.scroll_top(), 0, "no-op when mark not set");
        assert!(s.message.contains("Mark not set"));
    }

    #[test]
    fn copy_region_pushes_to_kill_ring() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let bkey = s.buffers.current().unwrap().to_string();
        let line2_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&bkey) {
            buf.mark = Some(line2_byte);
        }
        s.set_point_line(5);
        let region_text = s.buffers.get(&bkey).unwrap()
            .rope.slice(line2_byte..s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(5).unwrap())
            .to_string();
        s.copy_region();
        assert!(s.message.contains("copied to kill ring"));
        // The kill ring should now hold the region text.
        assert_eq!(s.kill_ring.top(), Some(region_text.as_str()));
    }

    #[test]
    fn kill_region_removes_text_in_editable_buffer() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let key = s.buffers.current().unwrap().to_string();
        let len_before = s.buffers.get(&key).unwrap().rope.len_bytes();
        let line2_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(line2_byte);
        }
        s.set_point_line(5);
        let line5_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(5).unwrap();
        s.kill_region();
        let len_after = s.buffers.get(&key).unwrap().rope.len_bytes();
        assert!(len_after < len_before, "kill must remove text");
        assert_eq!(len_before - len_after, line5_byte - line2_byte, "exactly the region bytes removed");
        // Mark is cleared after kill.
        assert!(s.buffers.get(&key).unwrap().mark.is_none());
        // Kill ring holds the removed text.
        assert!(!s.kill_ring.is_empty());
    }

    #[test]
    fn kill_region_read_only_copies_without_removing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut content = String::new();
        for i in 0..10 {
            content.push_str(&format!("line{}\n", i));
        }
        std::fs::write(dir.path().join("src/t.rs"), &content).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        let bkey = s.buffers.current().unwrap().to_string();
        let len_before = s.buffers.get(&bkey).unwrap().rope.len_bytes();
        // Set mark at line 2.
        let line2_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&bkey) {
            buf.mark = Some(line2_byte);
        }
        s.set_point_line(5);
        s.kill_region();
        // Buffer is unchanged (read-only).
        assert_eq!(s.buffers.get(&bkey).unwrap().rope.len_bytes(), len_before);
        // Kill ring has the text.
        assert!(!s.kill_ring.is_empty());
        // Mark is cleared.
        assert!(s.buffers.get(&bkey).unwrap().mark.is_none());
    }

    #[test]
    fn yank_inserts_at_point_in_editable_buffer() {
        let (mut s, _dir) = notes_store_with_lines(5);
        let bkey = s.buffers.current().unwrap().to_string();
        // First, copy some text to the kill ring.
        let line1_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(1).unwrap();
        let _line3_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(3).unwrap();
        if let Some(buf) = s.buffers.get_mut(&bkey) {
            buf.mark = Some(line1_byte);
        }
        s.set_point_line(3);
        s.copy_region();
        let yank_text = s.kill_ring.top().unwrap().to_string();
        let len_before = s.buffers.get(&bkey).unwrap().rope.len_bytes();
        // Now yank at line 0.
        s.set_point_line(0);
        s.yank();
        let len_after = s.buffers.get(&bkey).unwrap().rope.len_bytes();
        assert_eq!(len_after - len_before, yank_text.len(), "yank must insert the ring text");
        // The inserted text is at the start (line 0's byte offset).
        let buf_text = s.buffers.get(&bkey).unwrap().rope.to_string();
        assert!(buf_text.starts_with(&yank_text), "yanked text must be at the start");
    }

    #[test]
    fn yank_read_only_buffer_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "hello\nworld\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        // Open the read-only file and copy a region to the kill ring.
        s.open_path("src/t.rs");
        let file_key = s.buffers.current().unwrap().to_string();
        assert!(!s.buffers.get(&file_key).unwrap().editable, "file buffer must be read-only");
        // Set mark at line 0, scroll to line 1, copy the region.
        if let Some(buf) = s.buffers.get_mut(&file_key) {
            buf.mark = Some(0);
        }
        s.set_point_line(1);
        s.copy_region();
        assert!(!s.kill_ring.is_empty(), "kill ring must have an entry");
        // Switch to the read-only file (it's already current, but set it
        // explicitly to be sure).
        s.buffers.set_current(&file_key);
        assert_eq!(s.buffers.current().unwrap(), &file_key, "current must be the file buffer");
        let len_before = s.buffers.get(&file_key).unwrap().rope.len_bytes();
        s.yank();
        assert_eq!(s.buffers.get(&file_key).unwrap().rope.len_bytes(), len_before);
        assert!(s.message.contains("read-only"), "got: {}", s.message);
    }

    #[test]
    fn yank_pop_cycles_kill_ring() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        // The notes buffer starts with "# Notes\n" (1 line pre-filled).
        // Type unique lines so the kill ring entries are distinguishable.
        for c in "AAAA\n".chars() { s.notes_insert_char(c); }
        for c in "BBBB\n".chars() { s.notes_insert_char(c); }
        for c in "CCCC\n".chars() { s.notes_insert_char(c); }
        let bkey = s.buffers.current().unwrap().to_string();
        // Buffer: line 0="# Notes", line 1="AAAA", line 2="BBBB", line 3="CCCC"
        // Copy lines 1-2 ("AAAA\n") to the ring.
        let line1_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(1).unwrap();
        let _line2_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&bkey) {
            buf.mark = Some(line1_byte);
        }
        s.set_point_line(2);
        s.copy_region();
        let first_entry = s.kill_ring.top().unwrap().to_string();
        assert!(first_entry.contains("AAAA"), "first entry: {first_entry}");
        // Copy lines 3-4 ("CCCC\n") to the ring.
        let line3_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(3).unwrap();
        let _line4_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(4).unwrap();
        if let Some(buf) = s.buffers.get_mut(&bkey) {
            buf.mark = Some(line3_byte);
        }
        s.set_point_line(4);
        s.copy_region();
        let second_entry = s.kill_ring.top().unwrap().to_string();
        assert!(second_entry.contains("CCCC"), "second entry: {second_entry}");
        assert_ne!(first_entry, second_entry);
        // Now yank (inserts second_entry at line 0).
        s.set_point_line(0);
        s.yank();
        let text_after_yank = s.buffers.get(&bkey).unwrap().rope.to_string();
        // The yanked text (CCCC) is now at the start.
        assert!(text_after_yank.starts_with("CCCC"), "C-y must insert at start: {text_after_yank}");
        // M-y: replace with first_entry (AAAA...).
        s.yank_pop();
        let text_after_pop = s.buffers.get(&bkey).unwrap().rope.to_string();
        // After yank-pop, the first_entry replaces the second_entry at the start.
        assert!(text_after_pop.starts_with("AAAA"), "M-y must replace with first entry: {text_after_pop}");
    }

    #[test]
    fn yank_pop_noop_without_prior_yank() {
        let (mut s, _dir) = notes_store_with_lines(5);
        s.yank_pop();
        assert!(s.message.contains("no previous yank"));
    }

    #[test]
    fn kill_ring_shared_across_buffers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "hello world\nfoo bar\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        // Open the read-only file and copy a region to the kill ring.
        s.open_path("src/t.rs");
        let file_key = s.buffers.current().unwrap().to_string();
        let _line1_byte = s.buffers.get(&file_key).unwrap().rope.try_line_to_byte(1).unwrap();
        if let Some(buf) = s.buffers.get_mut(&file_key) {
            buf.mark = Some(0);
        }
        s.set_point_line(1);
        s.copy_region();
        let copied = s.kill_ring.top().unwrap().to_string();
        assert!(!copied.is_empty());
        // Switch to notes and yank: the kill ring is shared.
        s.open_notes();
        let len_before = s.buffers.current_buffer().unwrap().rope.len_bytes();
        s.yank();
        let len_after = s.buffers.current_buffer().unwrap().rope.len_bytes();
        assert_eq!(len_after - len_before, copied.len(), "cross-buffer yank must work");
    }

    #[test]
    fn kill_ring_bounded_at_60() {
        let (mut s, _dir) = notes_store_with_lines(5);
        // Push 65 different strings.
        for i in 0..65 {
            s.kill_ring.push(format!("entry_{}", i));
        }
        assert_eq!(s.kill_ring.len(), 60, "ring must be bounded at 60");
        // The most recent entry is entry_64.
        assert_eq!(s.kill_ring.top(), Some("entry_64"));
        // The oldest is entry_5 (entry_0 through entry_4 were evicted).
        assert_eq!(s.kill_ring.at(59), Some("entry_5"));
    }

    #[test]
    fn kill_ring_suppresses_consecutive_duplicates() {
        let (mut s, _dir) = notes_store_with_lines(5);
        s.kill_ring.push("hello".into());
        s.kill_ring.push("hello".into());
        s.kill_ring.push("world".into());
        assert_eq!(s.kill_ring.len(), 2, "consecutive duplicates must be suppressed");
        assert_eq!(s.kill_ring.top(), Some("world"));
        assert_eq!(s.kill_ring.at(1), Some("hello"));
    }

    #[test]
    fn region_display_in_status_line() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let key = s.buffers.current().unwrap().to_string();
        let line2_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(line2_byte);
        }
        s.set_point_line(5);
        let size = s.region_size_bytes().unwrap();
        assert!(size > 0);
        // The status line should show the region size.
        // (We verify via the store method, not the full render.)
        assert_eq!(s.region_size_bytes(), Some(size));
    }

    #[test]
    fn kill_region_non_ascii_content_correct() {
        // Proves the byte-to-char conversion at the edit boundary: a region
        // containing multi-byte UTF-8 characters must be killed correctly
        // (not the wrong text, not a panic from out-of-bounds char index).
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        // Type content with multi-byte characters: "café\nnaïve\nend\n"
        // Buffer: line 0="# Notes", line 1="café", line 2="naïve", line 3="end"
        for c in "café\nnaïve\nend\n".chars() {
            s.notes_insert_char(c);
        }
        let key = s.buffers.current().unwrap().to_string();
        // Set mark at byte 0 (start of "# Notes"), scroll to line 2 (start of "naïve").
        if let Some(b) = s.buffers.get_mut(&key) {
            b.mark = Some(0); // byte 0 = char 0
        }
        s.set_point_line(2);
        // Region is bytes [0, byte_offset_of_line2) = "# Notes\ncafé\n"
        s.kill_region();
        // After kill, the buffer should contain "naïve\nend\n".
        let remaining = s.buffers.get(&key).unwrap().rope.to_string();
        assert_eq!(remaining, "naïve\nend\n", "kill must remove exactly the region: {remaining:?}");
        // The kill ring holds the killed text.
        assert_eq!(s.kill_ring.top(), Some("# Notes\ncafé\n"));
    }

    #[test]
    fn yank_non_ascii_content_correct() {
        // Proves that yank inserts at the correct char offset when the buffer
        // contains multi-byte characters before the insertion point.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        // Buffer: "# Notes\n" + "héllo\n" (héllo has a multi-byte é)
        for c in "héllo\n".chars() {
            s.notes_insert_char(c);
        }
        let key = s.buffers.current().unwrap().to_string();
        // Copy "héllo\n" to the kill ring (lines 1-2).
        let line1_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(1).unwrap();
        let _line2_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(line1_byte);
        }
        s.set_point_line(2);
        s.copy_region();
        let yank_text = s.kill_ring.top().unwrap().to_string();
        assert_eq!(yank_text, "héllo\n");
        // Now yank at line 0 (start of buffer, before the multi-byte content).
        s.set_point_line(0);
        s.yank();
        let buf_text = s.buffers.get(&key).unwrap().rope.to_string();
        // The yanked text is inserted at the start.
        assert!(buf_text.starts_with("héllo\n"), "yank must insert at correct char offset: {buf_text:?}");
    }

    #[test]
    fn yank_pop_non_ascii_content_correct() {
        // Proves that yank-pop removes/inserts at the correct char offsets
        // when the buffer contains multi-byte characters.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        // Type unique lines with multi-byte content.
        for c in "café\n".chars() { s.notes_insert_char(c); }
        for c in "naïve\n".chars() { s.notes_insert_char(c); }
        for c in "end\n".chars() { s.notes_insert_char(c); }
        let key = s.buffers.current().unwrap().to_string();
        // Copy "café\n" (lines 1-2) to the ring.
        let l1 = s.buffers.get(&key).unwrap().rope.try_line_to_byte(1).unwrap();
        let _l2 = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) { buf.mark = Some(l1); }
        s.set_point_line(2);
        s.copy_region();
        // Copy "naïve\n" (lines 2-3) to the ring (now on top).
        let l2b = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        let _l3 = s.buffers.get(&key).unwrap().rope.try_line_to_byte(3).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) { buf.mark = Some(l2b); }
        s.set_point_line(3);
        s.copy_region();
        // Yank at line 0 (inserts "naïve\n" at the start).
        s.set_point_line(0);
        s.yank();
        let after_yank = s.buffers.get(&key).unwrap().rope.to_string();
        assert!(after_yank.starts_with("naïve\n"), "yank: {after_yank:?}");
        // M-y: replace with "café\n".
        s.yank_pop();
        let after_pop = s.buffers.get(&key).unwrap().rope.to_string();
        assert!(after_pop.starts_with("café\n"), "yank-pop must replace with prev entry: {after_pop:?}");
        // The rest of the buffer is intact.
        assert!(after_pop.contains("naïve\n"), "original content must be preserved: {after_pop:?}");
        assert!(after_pop.contains("end\n"), "original content must be preserved: {after_pop:?}");
    }

    // ── plan 015 issue 02: the honest point + the per-buffer edit mode ──

    #[test]
    fn set_mark_records_the_honest_point_byte_not_the_line_start() {
        // Discriminator (spec a): the point is MID-LINE, on a line where
        // byte ≠ char (é occupies 2 bytes, so char 5 = byte 6). The old
        // `current_point_byte` returned the LINE START (byte 0) regardless
        // of column, so a col-0 fixture cannot catch it.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "café omega\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        let key = s.buffers.current().unwrap().to_string();
        s.set_point(0, 5, 5); // the 'o' of "omega": CHAR 5, BYTE 6
        s.set_mark();
        assert_eq!(
            s.buffers.get(&key).unwrap().mark,
            Some(6),
            "the mark must hold the point's byte (6) — not the line start (0) or the char index (5)"
        );
    }

    #[test]
    fn region_same_line_mark_and_point_is_non_empty_and_exact() {
        // Discriminator (spec b): mark and point on the SAME line. The old
        // point (the line start) made mark == point there, so the region
        // was `None` — this pin fails on the old code. The region is exact
        // in both edit modes (plan 015 decision: a fine mark makes sense
        // in annotation mode too).
        let (_dir, mut s) = notes_store();
        for c in "hello world\n".chars() {
            s.notes_insert_char(c);
        }
        let key = s.buffers.current().unwrap().to_string();
        s.set_point(1, 2, 2); // "he|llo world"
        s.set_mark();
        s.set_point(1, 5, 5); // "hello| world"
        let line1 = s.buffers.get(&key).unwrap().rope.try_line_to_byte(1).unwrap();
        assert_eq!(
            s.region_byte_range(),
            Some((line1 + 2, line1 + 5)),
            "a same-line mark+point must yield the exact char range"
        );
        assert_eq!(s.region_size_bytes(), Some(3), "the region is 'llo' — 3 bytes");
    }

    #[test]
    fn copy_region_copies_exact_mid_line_text() {
        // Discriminator (spec c): the copied text is the EXACT mid-line
        // span (the old line-granular region would have copied the whole
        // line, and with a same-line mark+point it would have copied
        // nothing — "Mark not set").
        let (_dir, mut s) = notes_store();
        for c in "hello world\n".chars() {
            s.notes_insert_char(c);
        }
        let key = s.buffers.current().unwrap().to_string();
        s.set_point(1, 6, 6); // after "hello "
        s.set_mark();
        s.set_point(1, 11, 11); // EOL
        s.copy_region();
        assert_eq!(s.kill_ring.top(), Some("world"),
            "the copy must be the exact mid-line span");
        assert!(s.message.contains("copied to kill ring"));
        // Copy keeps the mark and the buffer.
        assert!(s.buffers.get(&key).unwrap().mark.is_some());
        assert_eq!(s.buffers.get(&key).unwrap().text(), "# Notes\nhello world\n");
    }

    #[test]
    fn kill_region_removes_exact_mid_line_text_in_editable_buffer() {
        // Discriminator (spec c): the kill removes EXACTLY the mid-line
        // span — the old line-granular region with a same-line mark+point
        // was empty ("Mark not set"), and a line-granular mark at line
        // start would have removed the whole line.
        let (_dir, mut s) = notes_store();
        for c in "hello world\n".chars() {
            s.notes_insert_char(c);
        }
        let key = s.buffers.current().unwrap().to_string();
        s.set_point(1, 5, 5); // after "hello" (the space sits at col 5)
        s.set_mark();
        s.set_point(1, 11, 11); // EOL
        s.kill_region();
        assert_eq!(s.buffers.get(&key).unwrap().text(), "# Notes\nhello\n",
            "kill must remove exactly ' world', not the whole line");
        assert_eq!(s.kill_ring.top(), Some(" world"));
        assert!(s.buffers.get(&key).unwrap().mark.is_none(), "mark cleared after kill");
    }

    #[test]
    fn yank_inserts_at_the_honest_point_not_the_line_start() {
        // The honest point's consumer: `C-y` inserts at the point's
        // COLUMN — the old line-start point would have inserted at col 0
        // (wrong under both modes, plan 015).
        let (_dir, mut s) = notes_store();
        for c in "hello world\n".chars() {
            s.notes_insert_char(c);
        }
        let key = s.buffers.current().unwrap().to_string();
        // Copy "world" (line 1, cols 6..11) to the ring.
        s.set_point(1, 6, 6);
        s.set_mark();
        s.set_point(1, 11, 11);
        s.copy_region();
        // Yank with the point MID-LINE (after "hello", col 5).
        s.set_point(1, 5, 5);
        s.yank();
        assert_eq!(s.buffers.get(&key).unwrap().text(), "# Notes\nhelloworld world\n",
            "yank must insert at the point's column, not the line start");
    }

    #[test]
    fn c_x_c_q_enters_and_leaves_accurate_and_notes_stay_typable() {
        // The invariant Accurate ⟹ editable, both directions of the
        // toggle, on both buffer kinds, with the status-line mode word
        // visible (spec d + e).
        // File buffer: Read-only (Annotation) ⇄ Accurate (editable).
        let (_dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        let b = s.buffers.get(&bufk).unwrap();
        assert_eq!(b.mode, BufferMode::Accurate, "C-x C-q enters Accurate");
        assert!(b.editable, "Accurate ⟹ editable");
        assert_eq!(s.buffer_mode_display(), "Accurate",
            "the status line must show the mode");
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        let b = s.buffers.get(&bufk).unwrap();
        assert_eq!(b.mode, BufferMode::Annotation, "leaving returns to Annotation");
        assert!(!b.editable, "a file buffer's baseline is read-only");
        assert_eq!(s.buffer_mode_display(), "Read-only");

        // Notes buffer: Accurate ⇄ Annotation, editable in BOTH — and a
        // typed annotation still lands in Annotation mode.
        let (_dir, mut s) = notes_store();
        let nkey = s.buffers.current().unwrap().to_string();        s.toggle_read_only();
        assert_eq!(s.buffers.get(&nkey).unwrap().mode, BufferMode::Accurate);
        assert!(s.buffers.get(&nkey).unwrap().editable, "Accurate ⟹ editable");
        assert_eq!(s.buffer_mode_display(), "Accurate", "the status line must show the mode");
        s.toggle_read_only();
        let b = s.buffers.get(&nkey).unwrap();
        assert_eq!(b.mode, BufferMode::Annotation, "leaving returns to Annotation");
        assert!(b.editable, "the notes buffer stays editable in Annotation mode");
        s.key_event(key("z"));
        assert!(
            s.buffer_text().ends_with('z'),
            "typed annotations must land in Annotation mode"
        );
    }

    // ── plan 005 issue 02: inline annotations ─────────────────────────


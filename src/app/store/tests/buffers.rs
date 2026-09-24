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
        assert!(s.buffers.current_buffer().unwrap().locally_modified());

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
            !s.buffers.current_buffer().unwrap().locally_modified(),
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
            s.buffers.current_buffer().unwrap().locally_modified(),
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
        assert!(!s.buffers.get(&extra_key).unwrap().locally_modified());
        assert!(!s.buffers.current_buffer().unwrap().locally_modified());
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
        assert!(s.buffers.current_buffer().unwrap().locally_modified());

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
        assert!(!s.buffers.get(&notes_key).unwrap().locally_modified());
        s.key_event(key("n"));
        assert!(s.quit, "one snapshot entry → the next answer quits");
        assert!(!s.quit_prompt_active());
        // The post-interception buffer was never asked about.
        assert!(s.buffers.get(&extra_key).unwrap().locally_modified());
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
            s.buffers.get(&bufk).unwrap().locally_modified(),
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
        assert!(s.buffers.get(&bufk).unwrap().locally_modified());

        s.key_event(key("C-x"));
        s.key_event(key("C-s"));
        let on_disk = std::fs::read_to_string(dir.path().join("src/f.rs")).unwrap();
        assert_eq!(on_disk, "fn old() {}\nX", "C-x C-s must write the edit to disk");
        let buf = s.buffers.get(&bufk).unwrap();
        assert!(!buf.locally_modified(), "save must clear locally_modified");
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
        assert!(!s.buffers.get(&bufk).unwrap().locally_modified());
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
        assert!(!buf.locally_modified(), "accept must clear locally_modified");
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
        assert!(!s.buffers.get(&bufk).unwrap().locally_modified());

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

    // ── issue-clipboard-and-selection: M-w → OSC 52 + kill ring ──────────

    /// A store with a file buffer whose lines carry multi-byte UTF-8
    /// (é, 中) on two of the three lines — the payload shape where a
    /// char/byte confusion produces DIFFERENT base64.
    fn multibyte_file_store() -> (AppStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/mb.rs"),
            "alpha\ncafé beta\n中 gamma\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/mb.rs");
        (s, dir)
    }

    /// The M-w copy publishes the byte-exact OSC 52 escape for the region's
    /// UTF-8 (a multi-byte payload, cross-checked against an independent
    /// encoder) AND keeps the kill ring (the primary sink — C-y parity).
    #[test]
    fn copy_region_publishes_byte_exact_osc52_and_keeps_kill_ring() {
        let (mut s, _dir) = multibyte_file_store();
        let mut rx = s.take_clipboard_rx().expect("clipboard rx");
        let bkey = s.buffers.current().unwrap().to_string();
        // Region: line 1 start (byte 6 — é is 2 bytes) to line 2's EOL
        // (byte 26 — the EOL is BEFORE the line's own trailing newline):
        // "café beta\n中 gamma".
        let start = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(1).unwrap();
        assert_eq!(start, 6, "byte 6 = line 1 start (é is 2 bytes)");
        let end = s.buffers.get(&bkey).unwrap().rope.len_bytes() - 1;
        assert_eq!(end, 26, "line 2's EOL, before its trailing newline");
        if let Some(buf) = s.buffers.get_mut(&bkey) {
            buf.mark = Some(start);
        }
        s.set_point(2, 7, 7); // "中 gamma": 7 chars, EOL
        s.key_event(key("M-w"));
        // The kill ring is intact (C-y must still yank this).
        assert_eq!(s.kill_ring.top(), Some("café beta\n中 gamma"));
        // The second sink: the byte-exact escape for exactly those bytes.
        assert_eq!(
            rx.try_recv().ok(),
            Some("\u{1b}]52;c;Y2Fmw6kgYmV0YQrkuK0gZ2FtbWE=\u{7}".to_string()),
        );
        assert!(s.message.contains("copied to kill ring"));
        assert!(!s.message.contains("cap"), "within the cap: no cap suffix");
        assert!(rx.try_recv().is_err(), "exactly one escape per copy");
    }

    /// Past the 32 KiB OSC 52 cap: the escape is SKIPPED (never truncated —
    /// a truncated clipboard copies silently wrong text), the kill ring
    /// keeps the full text, and the message says where the text went.
    #[test]
    fn copy_region_past_osc52_cap_keeps_kill_ring_and_says_so() {
        let (_dir, mut s) = notes_store();
        let big = "a".repeat(crate::app::clipboard::OSC52_MAX_BYTES + 1);
        // One long line in the notes buffer (the editable one).
        for _ in 0..big.len() {
            s.notes_insert_char('a');
        }
        let bkey = s.buffers.current().unwrap().to_string();
        let mut rx = s.take_clipboard_rx().expect("clipboard rx");
        // The seeded header is line 0 ("# Notes"); the a's are line 1:
        // mark at its start, point at its EOL.
        let line1_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(1).unwrap();
        if let Some(buf) = s.buffers.get_mut(&bkey) {
            buf.mark = Some(line1_byte);
        }
        s.set_point(1, big.len(), big.len()); // EOL of the a-line
        s.key_event(key("M-w"));
        assert_eq!(
            s.kill_ring.top(),
            Some(big.as_str()),
            "the kill ring gets the FULL text even past the cap ({} bytes)",
            big.len()
        );
        assert!(
            rx.try_recv().is_err(),
            "past the cap: no escape is published"
        );
        assert!(
            s.message.contains("kill ring only"),
            "the message reports the cap skip: `{}`",
            s.message
        );
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
        // Accurate mode: C-y inserts at the point (plan 015-04). Annotation
        // would append at the end, so this test pins the Accurate half.
        s.buffers.get_mut(&bkey).unwrap().mode = BufferMode::Accurate;
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
        // Accurate mode (plan 015-04): C-y inserts at the point, M-y replaces
        // at that same point — the start-of-buffer assertions below assume
        // point-insert, so the notes buffer must be Accurate, not Annotation.
        s.buffers.get_mut(&bkey).unwrap().mode = BufferMode::Accurate;
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
        // Accurate mode (plan 015-04): C-y inserts at the point (line 0 start),
        // so the multi-byte char-offset arithmetic is what this asserts. In
        // Annotation the same yank would append at the end instead.
        s.buffers.get_mut(&key).unwrap().mode = BufferMode::Accurate;
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
        // Accurate mode (plan 015-04): C-y / M-y operate at the point (line 0
        // start), so the multi-byte replace arithmetic is what this asserts.
        s.buffers.get_mut(&key).unwrap().mode = BufferMode::Accurate;
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
        // Accurate mode (plan 015-04): the honest point's consumer. This is the
        // discriminator — in Annotation the same mid-line yank would APPEND at
        // the end, not insert at col 5, so an Accurate-mode buffer is required
        // to keep pinning the point-insert behaviour.
        s.buffers.get_mut(&key).unwrap().mode = BufferMode::Accurate;
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
    fn yank_in_annotation_mode_appends_at_the_end() {
        // plan 015-04 (spec b): in Annotation mode C-y APPENDS at the end of
        // the buffer (matching self-insert's "try append"), NOT at the point.
        // The fixture is DISCRIMINATING: the point sits mid-line and the
        // buffer does NOT end at that line, so the old point-insert behaviour
        // would have landed the yanked text mid-line ("helloworld world")
        // while the Annotation append lands it after the final newline.
        let (_dir, mut s) = notes_store();
        for c in "hello world\n".chars() {
            s.notes_insert_char(c);
        }
        let nk = s.buffers.current().unwrap().to_string();
        assert_eq!(
            s.buffers.get(&nk).unwrap().mode,
            BufferMode::Annotation,
            "the notes buffer starts in Annotation mode"
        );
        // Copy "world" (line 1, cols 6..11) to the ring.
        s.set_point(1, 6, 6);
        s.set_mark();
        s.set_point(1, 11, 11);
        s.copy_region();
        assert_eq!(s.kill_ring.top(), Some("world"));
        // Point MID-LINE (after "hello", col 5) — deliberately NOT the end.
        s.set_point(1, 5, 5);
        s.yank();
        // Annotation append: "world" lands at the very end, not at col 5.
        assert_eq!(
            s.buffers.get(&nk).unwrap().text(),
            "# Notes\nhello world\nworld",
            "Annotation C-y must append at the end, not at the point"
        );
    }

    #[test]
    fn yank_pop_replaces_at_the_original_yank_position_in_both_modes() {
        // plan 015-04 (spec c): M-y re-replaces the SAME range the C-y used.
        // M-y must replace the range that was INSERTED (`yank_pos`), not the
        // current point: in Annotation mode the insertion is at the buffer end
        // while the point is elsewhere, and since issue-yank-followups the
        // Accurate-mode point advances past the inserted text too, so a pop
        // that used the point would corrupt it either way.

        // ── Accurate: yank at a mid-buffer point, pop at the inserted start ──
        let (mut s, bk, _dir) = accurate_file_store("top\nmid\nbot\n");
        s.set_point(0, 0, 0);
        s.kill_line(); // kill "top" → ring ["top"], "\nmid\nbot\n"
        s.set_point(1, 0, 0);
        s.kill_line(); // kill "mid" → ring ["mid","top"], "\n\nbot\n"
        assert_eq!(s.kill_ring.top(), Some("mid"));
        // Yank "mid" at (1,0) = char 1 (Accurate: at the point). The point
        // advances past the inserted text (C-y parity, issue-yank-followups),
        // but M-y replaces at the INSERTED range (`yank_pos`), not the moved
        // point.
        s.set_point(1, 0, 0);
        s.yank();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "\nmid\nbot\n",
            "Accurate C-y inserts at the point (char 1)"
        );
        // M-y replaces at the inserted range's start (char 1).
        s.yank_pop();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "\ntop\nbot\n",
            "Accurate M-y must replace at the inserted range's start"
        );

        // ── Annotation: yank appends at the end, pop replaces the tail ──
        let (_dir2, mut s2) = notes_store();
        for c in "aa\nbb\ncc\n".chars() {
            s2.notes_insert_char(c);
        }
        let nk2 = s2.buffers.current().unwrap().to_string();
        assert_eq!(
            s2.buffers.get(&nk2).unwrap().mode,
            BufferMode::Annotation,
            "the notes buffer starts in Annotation mode"
        );
        // Copy "aa\n" then "bb\n" → ring ["bb\n","aa\n"] (bb on top).
        s2.set_point(1, 0, 0);
        s2.set_mark();
        s2.set_point(2, 0, 0);
        s2.copy_region(); // "aa\n"
        s2.set_point(2, 0, 0);
        s2.set_mark();
        s2.set_point(3, 0, 0);
        s2.copy_region(); // "bb\n"
        assert_eq!(s2.kill_ring.top(), Some("bb\n"));
        // Annotation C-y appends "bb\n" at the end.
        s2.yank();
        assert_eq!(
            s2.buffers.get(&nk2).unwrap().text(),
            "# Notes\naa\nbb\ncc\nbb\n",
            "Annotation C-y appends at the end"
        );
        // M-y replaces the tail with "aa\n".
        s2.yank_pop();
        assert_eq!(
            s2.buffers.get(&nk2).unwrap().text(),
            "# Notes\naa\nbb\ncc\naa\n",
            "Annotation M-y must replace the appended tail"
        );
    }

    #[test]
    fn yank_annotation_append_is_char_accurate_with_multibyte() {
        // plan 015-04 (spec e): the Annotation append position is the buffer's
        // CHAR length (`rope.len_chars()`), never its byte length. A byte-based
        // position would over-index on a multi-byte line (é is 2 bytes) and
        // corrupt or panic the insert. Both the ring text and the existing
        // content carry multi-byte chars.
        let (_dir, mut s) = notes_store();
        for c in "héllo\n".chars() {
            s.notes_insert_char(c);
        }
        let nk = s.buffers.current().unwrap().to_string();
        assert_eq!(s.buffers.get(&nk).unwrap().mode, BufferMode::Annotation);
        // Copy "héllo\n" (line 1..2) to the ring.
        let l1 = s.buffers.get(&nk).unwrap().rope.try_line_to_byte(1).unwrap();
        let _l2 = s.buffers.get(&nk).unwrap().rope.try_line_to_byte(2).unwrap();
        s.buffers.get_mut(&nk).unwrap().mark = Some(l1);
        s.set_point_line(2);
        s.copy_region();
        assert_eq!(s.kill_ring.top(), Some("héllo\n"));
        // Annotation append: char-accurate position = len_chars, so the
        // appended "héllo\n" lands cleanly after the existing content.
        s.yank();
        assert_eq!(
            s.buffers.get(&nk).unwrap().text(),
            "# Notes\nhéllo\nhéllo\n",
            "Annotation append must be char-accurate (len_chars, not len_bytes)"
        );
    }

    #[test]
    fn yank_pop_coalesces_with_annotation_yank_into_one_undo_step() {
        // plan 015-04 (spec f) + 016-02: the Annotation-mode C-y is a PURE
        // insertion at the buffer end (the yank position), so the M-y
        // coalescing rule still fires: the yank-and-rotate sequence collapses
        // into ONE undo step even though the position is the end, not a
        // mid-line point. A doubled `retain_rope_edit` (a second undo step on
        // the C-y) would leave 2 steps here and break the merge.
        let (_dir, mut s) = notes_store();
        for c in "aa\nbb\n".chars() {
            s.notes_insert_char(c);
        }
        let nk = s.buffers.current().unwrap().to_string();
        assert_eq!(s.buffers.get(&nk).unwrap().mode, BufferMode::Annotation);
        // Two copies for the ring (copies record no undo step): "aa\n" then
        // "bb\n" → ring ["bb\n","aa\n"] (bb on top).
        let l1 = s.buffers.get(&nk).unwrap().rope.try_line_to_byte(1).unwrap();
        let l2 = s.buffers.get(&nk).unwrap().rope.try_line_to_byte(2).unwrap();
        s.buffers.get_mut(&nk).unwrap().mark = Some(l1);
        s.set_point_line(2);
        s.copy_region(); // "aa\n"
        s.buffers.get_mut(&nk).unwrap().mark = Some(l2);
        s.set_point_line(3);
        s.copy_region(); // "bb\n"
        assert_eq!(s.kill_ring.top(), Some("bb\n"));
        // Copies record no undo step — the stack holds only the six typed
        // chars. Track the base so the coalescing delta is what's asserted.
        let base = s.buffers.get(&nk).unwrap().undo.len();
        // Annotation C-y appends "bb\n" at the end → ONE undo step (stack+1).
        s.yank();
        assert_eq!(
            s.buffers.get(&nk).unwrap().text(),
            "# Notes\naa\nbb\nbb\n",
            "Annotation C-y appends the top ring entry at the end"
        );
        assert_eq!(
            s.buffers.get(&nk).unwrap().undo.len(),
            base + 1,
            "C-y must be EXACTLY one undo step (a doubled record would make this base+2)"
        );
        // M-y replaces the tail with "aa\n" and COALESCES with the C-y step:
        // the stack stays at base+1 (no coalescing would leave base+2).
        s.yank_pop();
        assert_eq!(
            s.buffers.get(&nk).unwrap().text(),
            "# Notes\naa\nbb\naa\n",
            "Annotation M-y replaces the appended tail"
        );
        assert_eq!(
            s.buffers.get(&nk).unwrap().undo.len(),
            base + 1,
            "C-y and M-y must coalesce into ONE undo step; a doubled C-y record would leave base+2 here"
        );
        // ONE undo removes the whole yank-and-rotate → pre-C-y.
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(
            s.buffers.get(&nk).unwrap().text(),
            "# Notes\naa\nbb\n",
            "a single undo must restore the pre-C-y buffer (the whole yank-and-rotate sequence)"
        );
    }

    // ── issue-yank-followups: C-y point advance, visibility, notes-dirty ──

    #[test]
    fn yank_advances_the_point_to_the_end_of_the_inserted_text_in_both_modes() {
        // issue-yank-followups P3-1 (emacs parity, oracle-verified on vanilla
        // emacs 30.2: C-y's doc says "Put point at the end" of the reinserted
        // text, and `emacs --batch` measured point 2 -> 5 after a 3-char
        // yank): the point lands at the END of the yanked text in both
        // modes. Measured pre-fix: the point did not move at all (the gate's
        // fixture, `set_point(0,1,1)` + yank "XYZ", left `point_col()` at 1).

        // ── Accurate: mid-line yank; the point lands past the inserted text ──
        let (mut s, bk, _dir) = accurate_file_store("ab\nxy\nzz\n");
        s.set_point(2, 0, 0);
        s.key_event(key("C-k")); // kill "zz" → ring ["zz"], "ab\nxy\n\n"
        s.set_point(1, 0, 0);
        s.key_event(key("C-k")); // kill "xy" → ring ["xy","zz"], "ab\n\n\n"
        assert_eq!(s.kill_ring.top(), Some("xy"));
        s.set_point(0, 1, 1); // "a|b" — the gate's fixture position
        s.yank();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "axyb\n\n\n",
            "Accurate C-y inserts at the point"
        );
        assert_eq!(
            (s.point_line(), s.point_col()),
            (0, 3),
            "Accurate C-y must leave the point at the END of the inserted text (past 'xy'), not where it started"
        );
        // M-y replaces the range that was INSERTED (chars 1..3), NOT the
        // moved point (char 3): a point-based pop would duplicate the text.
        s.yank_pop();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "azzb\n\n\n",
            "Accurate M-y must replace the inserted range, not the moved point"
        );

        // ── Annotation: append at the end; the point ends at the buffer end ──
        let (_dir2, mut s2) = notes_store();
        for c in "aa\nbb\ncc\n".chars() {
            s2.notes_insert_char(c);
        }
        let nk2 = s2.buffers.current().unwrap().to_string();
        assert_eq!(
            s2.buffers.get(&nk2).unwrap().mode,
            BufferMode::Annotation,
            "the notes buffer starts in Annotation mode"
        );
        // Ring ["bb\n","aa\n"] (bb on top) for a two-step yank/pop.
        s2.set_point(1, 0, 0);
        s2.set_mark();
        s2.set_point(2, 0, 0);
        s2.copy_region(); // "aa\n"
        s2.set_point(2, 0, 0);
        s2.set_mark();
        s2.set_point(3, 0, 0);
        s2.copy_region(); // "bb\n" (top)
        assert_eq!(s2.kill_ring.top(), Some("bb\n"));
        // Point MID-LINE — deliberately far from the append position.
        s2.set_point(1, 1, 1);
        s2.yank();
        assert_eq!(
            s2.buffers.get(&nk2).unwrap().text(),
            "# Notes\naa\nbb\ncc\nbb\n",
            "Annotation C-y appends at the end"
        );
        // The appended entry ends with a newline, so "end of the appended
        // text" is col 0 of the buffer's final (empty) line: char 17 (the
        // append position) + "bb\n" (3 chars) = char 20 = line 5, col 0.
        assert_eq!(
            (s2.point_line(), s2.point_col()),
            (5, 0),
            "Annotation C-y must leave the point at the end of the appended text (the buffer end), not where it started"
        );
        // M-y replaces the tail that was INSERTED (chars 17..20), NOT the
        // moved point (char 20, the buffer end): a point-based pop would
        // append the second entry and leave the first in place.
        s2.yank_pop();
        assert_eq!(
            s2.buffers.get(&nk2).unwrap().text(),
            "# Notes\naa\nbb\ncc\naa\n",
            "Annotation M-y must replace the appended tail, not the moved point"
        );
    }

    #[test]
    fn yank_annotation_append_keeps_the_insertion_visible() {
        // issue-yank-followups P3-2: in Annotation mode the insertion is the
        // buffer END, so a C-y with the window parked at the top can land
        // off-screen with no feedback (the landed "No scroll adjustment" note
        // was false for that mode; self-insert keeps the insertion visible).
        // The P3-1 point advance fixes it: the point lands on the appended
        // line and `set_point`'s follow-scroll puts it in the viewport.
        let (_dir, mut s) = notes_store();
        s.set_viewport_lines(10);
        // 20 typed lines: line 0 = "# Notes" (8 chars), lines 1..=20 =
        // "x00".."x19" (4 chars each: content 88, 22 lines — line 21 is the
        // empty line after the final newline). The window (10 rows) cannot
        // cover the buffer end from the top.
        for i in 0..20 {
            for c in format!("x{i:02}\n").chars() {
                s.notes_insert_char(c);
            }
        }
        let nk = s.buffers.current().unwrap().to_string();
        assert_eq!(s.buffers.get(&nk).unwrap().mode, BufferMode::Annotation);
        // Park the window at the top: the buffer end (line 21) is 12 lines
        // below the 10-row window, i.e. off-screen.
        s.set_scroll_top(0);
        assert_eq!(s.scroll_top(), 0);
        // Copy "x19" (line 20, cols 0..3 — WITHOUT the newline, so the
        // appended text has no trailing newline of its own) so the ring
        // holds it.
        let l20 = s.buffers.get(&nk).unwrap().rope.try_line_to_byte(20).unwrap();
        s.buffers.get_mut(&nk).unwrap().mark = Some(l20);
        s.set_point(20, 3, 3);
        s.copy_region();
        assert_eq!(s.kill_ring.top(), Some("x19"));
        s.set_point(0, 0, 0); // the point starts at the top, far from the end
        s.yank(); // Annotation: appends "x19" at the buffer end (line 21)
        let expected = String::from("# Notes\n")
            + &(0..20).map(|i| format!("x{i:02}\n")).collect::<String>()
            + "x19";
        assert_eq!(
            s.buffers.get(&nk).unwrap().text(),
            expected,
            "Annotation C-y appends at the end"
        );
        assert_eq!(s.point_line(), 21, "the point lands on the appended line");
        assert_eq!(s.point_col(), 3, "at the end of the appended text");
        // The window must have followed: the appended line is the LAST
        // visible row (keep_cursor_visible lands it on the bottom row).
        assert_eq!(
            s.scroll_top(),
            12,
            "the appended text must be inside the 10-row window; pre-fix the scroll stayed at 0 (off-screen)"
        );
    }

    #[test]
    fn yank_marks_the_notes_doc_dirty_and_the_reparse_contains_the_yank() {
        // issue-yank-followups P3-3: a C-y is a notes edit like
        // `notes_insert_char` / `notes_backspace` / every Accurate edit, so
        // it must set `notes_buffer_dirty` and let the doc reparse. Measured
        // pre-fix: the flag stayed `false` before AND after the yank in both
        // modes, so the yanked text was absent from the parsed doc until
        // some other edit triggered a reparse. The assertion discriminates
        // by OCCURRENCE COUNT — the yanked line was already typed into the
        // buffer, so a stale doc parses it exactly once; the reparsed doc
        // must parse it twice.
        // ── Accurate ──
        let (_dir, mut s) = notes_store();
        for c in "aa\nbb\ncc\n".chars() {
            s.notes_insert_char(c);
        }
        let nk = s.buffers.current().unwrap().to_string();
        assert_eq!(s.buffers.get(&nk).unwrap().mode, BufferMode::Annotation);
        // Consume the type-run's dirty state: the doc now holds the typed
        // content and is clean.
        s.ensure_notes_doc();
        assert!(!s.notes_buffer_dirty, "baseline: typed content already reparsed");
        assert_eq!(
            s.notes_doc.before.iter().filter(|l| *l == "bb").count(),
            1,
            "baseline: one typed 'bb' line in the parsed doc"
        );
        // Copy "bb\n" (line 2..3) to the ring.
        s.set_point(2, 0, 0);
        s.set_mark();
        s.set_point(3, 0, 0);
        s.copy_region();
        assert_eq!(s.kill_ring.top(), Some("bb\n"));
        s.buffers.get_mut(&nk).unwrap().mode = BufferMode::Accurate;
        s.set_point(0, 0, 0);
        s.yank(); // inserts "bb\n" at the start
        assert_eq!(
            s.buffers.get(&nk).unwrap().text(),
            "bb\n# Notes\naa\nbb\ncc\n",
            "Accurate C-y inserts at the point"
        );
        assert!(
            s.notes_buffer_dirty,
            "Accurate C-y must mark the notes doc dirty (pre-fix it stayed false)"
        );
        s.ensure_notes_doc();
        assert!(!s.notes_buffer_dirty, "the reparse must clear the flag");
        assert_eq!(
            s.notes_doc.before.iter().filter(|l| *l == "bb").count(),
            2,
            "the reparsed doc must contain the yanked line (a stale doc parses it only once; before={:?}",
            s.notes_doc.before
        );

        // ── Annotation ──
        let (_dir2, mut s2) = notes_store();
        for c in "aa\nbb\ncc\n".chars() {
            s2.notes_insert_char(c);
        }
        let nk2 = s2.buffers.current().unwrap().to_string();
        assert_eq!(s2.buffers.get(&nk2).unwrap().mode, BufferMode::Annotation);
        s2.ensure_notes_doc();
        assert!(!s2.notes_buffer_dirty, "baseline: typed content already reparsed");
        s2.set_point(2, 0, 0);
        s2.set_mark();
        s2.set_point(3, 0, 0);
        s2.copy_region();
        assert_eq!(s2.kill_ring.top(), Some("bb\n"));
        s2.set_point(1, 1, 1); // mid-line point; the append ignores it
        s2.yank(); // appends "bb\n" at the end
        assert_eq!(
            s2.buffers.get(&nk2).unwrap().text(),
            "# Notes\naa\nbb\ncc\nbb\n",
            "Annotation C-y appends at the end"
        );
        assert!(
            s2.notes_buffer_dirty,
            "Annotation C-y must mark the notes doc dirty (pre-fix it stayed false)"
        );
        s2.ensure_notes_doc();
        assert_eq!(
            s2.notes_doc.before.iter().filter(|l| *l == "bb").count(),
            2,
            "the reparsed doc must contain the yanked line (a stale doc parses it only once; before={:?}",
            s2.notes_doc.before
        );
    }

    #[test]
    fn yank_pop_lands_the_point_at_the_end_of_the_replacement() {
        // gate P2-1: `yank-pop` deletes [yank_pos, end) and then inserts the
        // new text, so in emacs point ends AFTER the replacement — and it
        // therefore MOVES when the replacement's length differs. The lane that
        // fixed `yank` asserted the opposite ("M-y must NOT advance the
        // point") from `simple.el`; the gate contradicted it from the same
        // source, because `yank-pop` does `delete-region` then
        // `insert-for-yank`. Oracle (emacs 30.2, measured): C-y "Q" at bob ->
        // point 2; then M-y "XYZW" -> point 5, i.e. the END of the new text.
        //
        // The fixture must be LENGTH-CHANGING **and end mid-line**. With
        // equal-length entries the point's end position coincides; and a
        // fixture like "XYZW\n" -> "Q\n" ends at (line 1, col 0) either way,
        // so a line/col assertion could not tell the rules apart. Here the
        // pop's replacement is 4 chars where the yank's was 1, mid-line.
        let (mut s, bk, _dir) = accurate_file_store("XYZWQ\n");
        // Ring: ["Q", "XYZW"] — the SHORT entry on top so the M-y
        // replacement is LONGER.
        s.set_point(0, 0, 0);
        s.set_mark();
        s.set_point(0, 4, 4);
        s.copy_region(); // "XYZW"
        s.set_point(0, 4, 4);
        s.set_mark();
        s.set_point(0, 5, 5);
        s.copy_region(); // "Q" -> top
        assert_eq!(s.kill_ring.top(), Some("Q"));
        s.set_point(0, 0, 0);
        s.yank();
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "QXYZWQ\n");
        assert_eq!(
            (s.point_line(), s.point_col()),
            (0, 1),
            "C-y leaves the point at the end of the inserted text"
        );
        s.yank_pop(); // replaces "Q" (1 char) with "XYZW" (4 chars)
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "XYZWXYZWQ\n");
        assert_eq!(
            (s.point_line(), s.point_col()),
            (0, 4),
            "M-y must land the point at the END OF THE REPLACEMENT — char 4, \
             not char 1 where the C-y left it (pre-fix it stayed at 1)"
        );
    }

    #[test]
    fn yank_pop_marks_the_notes_doc_dirty_too() {
        // gate P2-2: fix 3 was incomplete for M-y. `yank` marks the notes doc
        // dirty, but `file_view_rows()` -> `ensure_notes_doc()` consumes that
        // flag BETWEEN keystrokes, so by the time M-y lands the C-y's flag is
        // already gone and the pop never reaches the doc. Measured pre-fix:
        // `notes_buffer_dirty` false before AND after the pop, and not even an
        // explicit reparse picked it up (the flag was false and the doc was
        // loaded) — the annotated pop "did nothing" until an unrelated edit.
        let (_dir, mut s) = notes_store();
        for c in "aa\nbb\ncc\n".chars() {
            s.notes_insert_char(c);
        }
        let nk = s.buffers.current().unwrap().to_string();
        s.ensure_notes_doc();
        assert!(!s.notes_buffer_dirty, "baseline: typed content already reparsed");
        // Ring: ["bb\n", "aa\n"] — bb on top, aa beneath for the pop.
        s.set_point(1, 0, 0);
        s.set_mark();
        s.set_point(2, 0, 0);
        s.copy_region(); // "aa\n"
        s.set_point(2, 0, 0);
        s.set_mark();
        s.set_point(3, 0, 0);
        s.copy_region(); // "bb\n" -> top
        assert_eq!(s.kill_ring.top(), Some("bb\n"));
        s.buffers.get_mut(&nk).unwrap().mode = BufferMode::Accurate;
        s.set_point(0, 0, 0);
        s.yank();
        // Consume the C-y's dirty flag, exactly as the render loop does between
        // keystrokes — this is the step that made the pre-fix pop invisible.
        s.ensure_notes_doc();
        assert!(!s.notes_buffer_dirty, "the C-y's flag was consumed by the reparse");
        s.yank_pop(); // rotates "bb\n" -> "aa\n"
        assert!(
            s.notes_buffer_dirty,
            "M-y must mark the notes doc dirty too — the C-y's flag was already \
             consumed, so without this call the pop never reaches the doc"
        );
        s.ensure_notes_doc();
        assert_eq!(
            s.notes_doc.before.iter().filter(|l| *l == "aa").count(),
            2,
            "the reparsed doc must contain the popped line (typed once + popped \
             once); a stale doc parses it only once — before={:?}",
            s.notes_doc.before
        );
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

    // ── plan 015 issue 03: accurate-mode editing ─────────────────────
    // A file buffer opened read-only, then toggled into `Accurate` mode
    // (editable). The content is the test fixture.
    fn accurate_file_store_with(content: &str) -> (tempfile::TempDir, AppStore) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/f.rs"), content).unwrap();
        let mut s = store(dir.path());
        s.open_path("src/f.rs");
        s.toggle_read_only(); // C-x C-q → Accurate + editable
        (dir, s)
    }

    #[test]
    fn accurate_insert_lands_at_point_and_advances() {
        // (a): a mid-line self-insert lands AT the point (not the end of
        // the buffer) and the point advances past the inserted char. The
        // col-0/end-of-buffer fixture is deliberately avoided — the point is
        // mid-line, so this discriminates from the append-at-end coarse path.
        let (_dir, mut s) = accurate_file_store_with("hello world\n");
        let bk = s.buffers.current().unwrap().to_string();
        s.set_point(0, 5, 5); // between "hello" and " world"
        s.key_event(key("x"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hellox world\n",
            "the char must land at the point (after 'hello'), not the buffer end"
        );
        assert_eq!(s.point_col(), 6, "the point must advance past the insert");
        assert!(s.buffers.get(&bk).unwrap().locally_modified());
    }

    #[test]
    fn accurate_backspace_removes_preceding_char() {
        // (b): Backspace mid-line removes the char BEFORE the point and the
        // point moves back over it (not the last char of the buffer).
        let (_dir, mut s) = accurate_file_store_with("hello world\n");
        let bk = s.buffers.current().unwrap().to_string();
        s.set_point(0, 5, 5); // between "hello" and " world"
        s.key_event(key("C-h"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hell world\n",
            "the char before the point ('o' of hello) must be removed"
        );
        assert_eq!(s.point_col(), 4, "the point must move back one char");
    }

    #[test]
    fn accurate_backspace_at_buffer_start_is_noop() {
        // (c): at the buffer start there is no char before the point
        // (emacs behaviour): the buffer and the point are untouched.
        let (_dir, mut s) = accurate_file_store_with("hello world\n");
        let bk = s.buffers.current().unwrap().to_string();
        s.set_point(0, 0, 0);
        s.key_event(key("C-h"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "hello world\n");
        assert_eq!(s.point_col(), 0);
        assert!(!s.buffers.get(&bk).unwrap().locally_modified(), "a no-op must not mark the buffer");
    }

    #[test]
    fn accurate_ret_splits_line_at_point() {
        // (d): RET inserts a newline at the point, splitting the line; the
        // point lands on the new (lower) line. Today RET is unhandled in the
        // Buffer view, so this test fails on the pre-change code.
        let (_dir, mut s) = accurate_file_store_with("hello world\n");
        let bk = s.buffers.current().unwrap().to_string();
        s.set_point(0, 5, 5);
        s.key_event(key("RET"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hello\n world\n",
            "RET must split the line at the point"
        );
        assert_eq!(s.point_line(), 1, "the point must land on the new line");
        assert_eq!(s.point_col(), 0);
    }

    #[test]
    fn accurate_kill_line_to_eol_and_yank_back() {
        // (e): C-k kills from the point to EOL (keeping the newline) and
        // pushes it to the kill ring; C-y then re-inserts it. Also pins the
        // yank-pop reset (the kill is not a yank).
        let (_dir, mut s) = accurate_file_store_with("hello world\nsecond\n");
        let bk = s.buffers.current().unwrap().to_string();
        s.set_point(0, 5, 5);
        s.key_event(key("C-k"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hello\nsecond\n",
            "C-k must kill to EOL, keeping the newline"
        );
        assert_eq!(s.kill_ring.top(), Some(" world"), "the killed text must hit the kill ring");
        assert_eq!(s.yank_pos, None, "a kill must reset the yank-pop state");
        // Yank it back at the same point.
        s.key_event(key("C-y"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hello world\nsecond\n",
            "C-y must re-insert the killed text at the point"
        );
    }

    #[test]
    fn accurate_kill_line_at_eol_joins_lines() {
        // C-k at EOL kills the newline itself (join), per the stated
        // end-of-line decision. At the buffer end (no trailing newline) it
        // is a no-op.
        let (_dir, mut s) = accurate_file_store_with("hello world\nsecond\n");
        let bk = s.buffers.current().unwrap().to_string();
        // Point at the end of line 0 (after "hello world", before the \n).
        s.set_point(0, 11, 11);
        s.key_event(key("C-k"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hello worldsecond\n",
            "C-k at EOL must kill the newline and join the lines"
        );
        assert_eq!(s.kill_ring.top(), Some("\n"));
        // Now one line with a trailing \n; C-k at its EOL kills that final
        // newline (a kill, not a no-op — the join is what C-k does at EOL).
        s.set_point(0, 17, 17); // EOL of the joined line, before the final \n
        s.key_event(key("C-k"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hello worldsecond",
            "C-k at EOL kills the trailing newline"
        );
        // Now the point is at the buffer end (no trailing newline): a no-op.
        s.set_point(0, 17, 17);
        s.key_event(key("C-k"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hello worldsecond",
            "C-k at the buffer end (no trailing newline) must be a no-op"
        );
    }

    #[test]
    fn accurate_delete_char_forward() {
        // (item 5): C-d deletes the char AT the point (freed from half-page
        // scroll); the point stays put. At the buffer end it is a no-op.
        {
            let (_dir, mut s) = accurate_file_store_with("hello world\n");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 5, 5); // on the space
            s.key_event(key("C-d"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "helloworld\n",
                "C-d must delete the char at the point (the space)"
            );
            assert_eq!(s.point_col(), 5, "the point must not move");
            // On the trailing \n (the last char): C-d removes it.
            s.set_point(0, 10, 10);
            s.key_event(key("C-d"));
            assert_eq!(s.buffers.get(&bk).unwrap().text(), "helloworld");
        }
        // Past the end (EOL of a no-trailing-newline buffer) is a no-op.
        {
            let (_dir, mut s) = accurate_file_store_with("ab");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 2, 2); // past the last char
            s.key_event(key("C-d"));
            assert_eq!(s.buffers.get(&bk).unwrap().text(), "ab");
        }
    }

    #[test]
    fn accurate_kill_word_backward() {
        // (item 6): M-DEL kills from the point backward to the previous word
        // boundary (emacs `backward-kill-word`) and pushes it to the kill
        // ring. The point moves to the START of the killed text. A run of
        // whitespace IMMEDIATELY before the point is killed with the word;
        // interior whitespace (a gap between two words) is NOT crossed. At the
        // buffer start it is a no-op.
        {
            let (_dir, mut s) = accurate_file_store_with("hello world\n");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 11, 11); // end of line 0, after "world"
            s.key_event(key("M-DEL"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "hello \n",
                "M-DEL must kill the previous word (the preceding space is kept)"
            );
            assert_eq!(s.kill_ring.top(), Some("world"));
        }
        // Discriminating multibyte mid-line case (plan 015-03 P1): the point
        // lands at the KILL START, not at the stale pre-kill point char. On the
        // old code (landing at the pre-kill point_char) the point would sit at
        // col 4 of " omega" (on 'g'); the fix puts it at col 0.
        {
            let (_dir, mut s) = accurate_file_store_with("café omega\n");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 4, 4); // mid-line, on the space after "café"
            s.key_event(key("M-DEL"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                " omega\n",
                "M-DEL must kill the word 'café' (the space before the point is kept)"
            );
            assert_eq!(s.kill_ring.top(), Some("café"));
            assert_eq!(s.point_col(), 0, "point must land at the kill start, not the stale pre-kill point char");
        }
        // Whitespace immediately before the point is killed with the word.
        {
            let (_dir, mut s) = accurate_file_store_with("word  \n"); // "word" + 2 spaces
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 6, 6); // end, after the two spaces
            s.key_event(key("M-DEL"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "\n",
                "M-DEL must kill the word and the whitespace before it"
            );
            assert_eq!(s.kill_ring.top(), Some("word  "));
        }
        // Buffer start: no-op.
        {
            let (_dir, mut s) = accurate_file_store_with("word\n");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 0, 0);
            s.key_event(key("M-DEL"));
            assert_eq!(s.buffers.get(&bk).unwrap().text(), "word\n");
        }
    }

    #[test]
    fn accurate_kill_word_forward() {
        // (item 6, P2-c): M-d kills from the point FORWARD with the emacs
        // `kill-word 1` extent and pushes it to the kill ring; the point
        // stays at the kill start (the text after the killed region moves up
        // to it). Emacs `forward-word` extent: a word char at the point kills
        // that word ONLY (the trailing space is NOT consumed); a non-word at
        // the point kills the non-word run plus the next word. At the buffer
        // end it is a no-op.
        {
            let (_dir, mut s) = accurate_file_store_with("hello world\n");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 0, 0); // start of line 0, on "hello"
            s.key_event(key("M-d"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                " world\n",
                "M-d must kill the word at the point only, NOT the following space"
            );
            assert_eq!(s.kill_ring.top(), Some("hello"));
            assert_eq!(s.point_col(), 0, "point stays at the kill start");
        }
        // Mid-line: the word under the point is killed; the two spaces left
        // behind (the trailing gap is NOT consumed) — the following word
        // moves up to the point, which stays put.
        {
            let (_dir, mut s) = accurate_file_store_with("aa bb cc\n");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 3, 3); // mid-line, on the 'b' of "bb"
            s.key_event(key("M-d"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "aa  cc\n",
                "M-d must kill the word at the point only (two spaces survive)"
            );
            assert_eq!(s.kill_ring.top(), Some("bb"));
            assert_eq!(s.point_col(), 3, "point stays at the kill start (col 3)");
        }
        // Multibyte + three-word line (the gate's `café omega zeta` case):
        // char 5 is the 'o' of "omega"; the kill is "omega" only, leaving the
        // double space, and the point stays on the first of the two spaces.
        {
            let (_dir, mut s) = accurate_file_store_with("café omega zeta\n");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 5, 5); // the 'o' of "omega"
            s.key_event(key("M-d"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "café  zeta\n",
                "M-d must not eat the space after the word"
            );
            assert_eq!(s.kill_ring.top(), Some("omega"));
            assert_eq!(s.point_col(), 5, "point stays at the kill start");
        }
        // The newline is non-word but is NOT consumed when the kill is the
        // word at the point (the gate's `café omega\n` @5 case).
        {
            let (_dir, mut s) = accurate_file_store_with("café omega\n");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 5, 5); // the 'o' of "omega"
            s.key_event(key("M-d"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "café \n",
                "M-d must not join the lines (the newline survives)"
            );
            assert_eq!(s.kill_ring.top(), Some("omega"));
            assert_eq!(s.point_col(), 5);
        }
        // Point at line start on a word char: only the word is killed (the
        // gate's `café omega\n` @0 case).
        {
            let (_dir, mut s) = accurate_file_store_with("café omega\n");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 0, 0); // on the 'c' of "café"
            s.key_event(key("M-d"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                " omega\n",
                "M-d must kill the word only (the following space survives)"
            );
            assert_eq!(s.kill_ring.top(), Some("café"));
            assert_eq!(s.point_col(), 0);
        }
        // Point INSIDE whitespace: the non-word run and the next word are
        // killed (the gate's `café  zeta` @5 case) — the old code killed the
        // single space only.
        {
            let (_dir, mut s) = accurate_file_store_with("café  zeta\n");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 5, 5); // the second space
            s.key_event(key("M-d"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "café \n",
                "M-d on whitespace must kill the run plus the next word"
            );
            assert_eq!(s.kill_ring.top(), Some(" zeta"));
            assert_eq!(s.point_col(), 5);
        }
        // Buffer end: no-op.
        {
            let (_dir, mut s) = accurate_file_store_with("word");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 4, 4); // past the last char
            s.key_event(key("M-d"));
            assert_eq!(s.buffers.get(&bk).unwrap().text(), "word");
        }
    }

    #[test]
    fn accurate_open_line() {
        // (item 8): C-o opens a line before the point; the text from the
        // point on drops to the new line, the point stays at the end of the
        // upper line.
        let (_dir, mut s) = accurate_file_store_with("abcdef\n");
        let bk = s.buffers.current().unwrap().to_string();
        s.set_point(0, 3, 3); // between "abc" and "def"
        s.key_event(key("C-o"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "abc\ndef\n",
            "C-o must open a line before the point"
        );
        assert_eq!(s.point_line(), 0, "the point stays on the upper line");
        assert_eq!(s.point_col(), 3);
    }

    #[test]
    fn accurate_transpose_chars() {
        // (item 8): C-t is emacs `transpose-chars`: mid-line it swaps the
        // char before and at the point and moves the point forward one,
        // past both swapped chars. The line-edge cases
        // emacs folds in: at a line end (char at point is \n) the PREVIOUS
        // TWO chars are exchanged and the point does not move; at a line
        // start (char before is \n) the first char of the line moves to the
        // end of the previous and the point moves forward one.
        {
            let (_dir, mut s) = accurate_file_store_with("ab\n");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 1, 1); // between a and b
            s.key_event(key("C-t"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "ba\n",
                "C-t must swap the two chars"
            );
            assert_eq!(
                s.point_col(),
                2,
                "the point moves forward one, past both swapped chars"
            );
        }
        // At EOL (char at point is \n): emacs exchanges the PREVIOUS TWO
        // chars of the line (the old pin, "last char moves to the start of
        // the next line", is what emacs does NOT do); the point does not
        // move.
        {
            let (_dir, mut s) = accurate_file_store_with("ab\ncd\n");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 2, 2); // end of line 0, on the \n
            s.key_event(key("C-t"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "ba\ncd\n",
                "EOL transpose must exchange the previous two chars"
            );
            assert_eq!(s.point_col(), 2, "the point does not move at EOL");
        }
        // At a line start (char before is \n): the first char of the line
        // moves to the end of the previous; the point moves forward one.
        {
            let (_dir, mut s) = accurate_file_store_with("ab\ncd\n");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(1, 0, 0); // start of line 1, on the 'c'
            s.key_event(key("C-t"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "abc\nd\n",
                "line-start transpose must drag the newline past the char"
            );
            assert_eq!(
                s.point_col(),
                0,
                "the point moves one char forward and stays at the line start (now on 'd')"
            );
        }
        // At the buffer start (no char before): no-op.
        {
            let (_dir, mut s) = accurate_file_store_with("ab\n");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 0, 0);
            s.key_event(key("C-t"));
            assert_eq!(s.buffers.get(&bk).unwrap().text(), "ab\n");
        }
        // At the buffer END (point past the last char): emacs transposes the
        // LAST TWO chars and leaves the point at the end (one past the
        // between-position) — not a no-op (plan 015-03 P2-c). The buffer-end
        // case is what the old no-op guard got wrong.
        {
            let (_dir, mut s) = accurate_file_store_with("ab");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 2, 2); // past the last char
            s.key_event(key("C-t"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "ba",
                "buffer-end C-t must transpose the last two chars"
            );
            assert_eq!(s.point_col(), 2, "point leaves at the end of the buffer");
        }
        // EOL with fewer than two chars before the point in the buffer
        // (the guard is buffer-position based, so it is not a per-line
        // criterion): emacs signals an error; redline no-ops (as at the
        // buffer start) rather than erroring.
        {
            let (_dir, mut s) = accurate_file_store_with("x\ncd\n");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 1, 1); // end of the one-char line 0, on the \n
            s.key_event(key("C-t"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "x\ncd\n",
                "EOL with one char before the point in the buffer is a no-op"
            );
            assert_eq!(s.point_col(), 1);
        }
        // Buffer end on a single-char buffer (total_chars < 2): nothing to
        // transpose — emacs signals (beginning-of-buffer); redline no-ops
        // (as at the buffer start) rather than erroring.
        {
            let (_dir, mut s) = accurate_file_store_with("a");
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 1, 1); // past the single char
            s.key_event(key("C-t"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "a",
                "buffer-end C-t on a one-char buffer is a no-op"
            );
            assert_eq!(s.point_col(), 1);
        }
    }

    #[test]
    fn accurate_multibyte_point_arithmetic() {
        // (g): the multibyte byte/char trap. `point_col` is a CHAR index; a
        // byte-arithmetic implementation would land at the wrong spot. The
        // two-é fixture has char index 1 at byte index 2, so a byte-as-char
        // bug is visible.
        {
            let (_dir, mut s) = accurate_file_store_with("\u{e9}\u{e9}\n"); // "éé\n"
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 1, 1); // between the two é (char 1, byte 2)
            s.key_event(key("x"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "\u{e9}x\u{e9}\n",
                "the insert must land between the two é, not after both"
            );
        }
        // Backspace over a multibyte char at the boundary.
        {
            let (_dir, mut s) = accurate_file_store_with("h\u{e9}llo\n"); // "héllo"
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 2, 2); // after the é (char 2, byte 3)
            s.key_event(key("C-h"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().text(),
                "hllo\n",
                "backspace must remove the whole é, not a stray byte"
            );
        }
        // Transpose across a multibyte char keeps the bytes whole.
        {
            let (_dir, mut s) = accurate_file_store_with("a\u{e9}b\n"); // "aéb"
            let bk = s.buffers.current().unwrap().to_string();
            s.set_point(0, 2, 2); // between é and b
            s.key_event(key("C-t"));
            assert_eq!(s.buffers.get(&bk).unwrap().text(), "ab\u{e9}\n");
        }
    }

    #[test]
    fn baseline_editable_survives_project_root_switch() {
        // The 015-02 gate drift, pinned: the baseline predicate must be
        // BUFFER-relative (is_notes), not root-relative (notes_key()). Open
        // the notes buffer, enter Accurate, then switch_project_root: the old
        // notes buffer survives the table and its baseline stays editable.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        let notes_key = s.buffers.current().unwrap().to_string();
        assert!(s.buffer_baseline_editable(&notes_key), "the notes buffer is baseline-editable");
        s.toggle_read_only();
        assert_eq!(
            s.buffers.get(&notes_key).unwrap().mode,
            BufferMode::Accurate
        );
        // A second, distinct project root.
        let dir2 = tempfile::tempdir().unwrap();
        std::fs::write(dir2.path().join("Cargo.toml"), "[package]\n").unwrap();
        s.switch_project_root(dir2.path().to_str().unwrap());
        // The old notes buffer survives the root switch (the table is not
        // cleared)...
        assert!(
            s.buffers.get(&notes_key).is_some(),
            "the old notes buffer must survive switch_project_root"
        );
        // ...and the root-relative notes_key() now points at the NEW project,
        // so a root-relative predicate would read the OLD notes buffer as
        // non-notes (read-only baseline).
        let new_notes = dir2
            .path()
            .join(".redline-notes.md")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            s.notes_key().as_deref(),
            Some(new_notes.as_str()),
            "sanity: notes_key() is now root-relative to the new project"
        );
        // The FIX: the baseline is buffer-relative (is_notes) and does not
        // drift.
        assert!(
            s.buffer_baseline_editable(&notes_key),
            "the baseline must stay editable across a project-root switch"
        );
    }

    #[test]
    fn notes_opened_via_find_file_is_baseline_editable() {
        // (P2-a): the is_notes flag must be set on EVERY route that opens the
        // notes file, not only open_notes. Opening .redline-notes.md through
        // find-file (open_path) must leave the buffer with the same
        // buffer-relative baseline as open_notes; the old code left it
        // is_notes=false, so C-x C-q → C-x C-q on a find-file-opened notes
        // buffer ended read-only.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join(".redline-notes.md"), "# Notes\n").unwrap();
        let mut s = store(dir.path());
        // Find-file route (open_path), not open_notes.
        s.open_path(".redline-notes.md");
        let notes_key = s.buffers.current().unwrap().to_string();
        assert!(
            s.notes_key().as_deref() == Some(notes_key.as_str()),
            "sanity: the opened buffer is the notes file"
        );
        assert!(
            s.buffer_baseline_editable(&notes_key),
            "the notes buffer opened via find-file must be baseline-editable"
        );
        // The full drift repro: find-file open, enter Accurate, exit — the
        // baseline must not flip it read-only.
        s.toggle_read_only();
        assert_eq!(
            s.buffers.get(&notes_key).unwrap().mode,
            BufferMode::Accurate
        );
        s.toggle_read_only();
        assert!(
            s.buffers.get(&notes_key).unwrap().editable,
            "the find-file-opened notes buffer must stay editable after leaving Accurate"
        );
    }

    #[test]
    fn accurate_commands_are_no_op_in_annotation_mode_via_dispatch() {
        // (P2-b): the point-accurate commands are gated on Accurate mode, so a
        // direct M-x (dispatch) on an Annotation-mode buffer is a no-op —
        // Annotation stays coarse. Without the gate, M-x delete-char-forward
        // would delete at the point on the notes buffer (contradicting
        // PLAN §4). The contrast (Accurate) proves the gate does not block
        // the key-routed path.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        let bk = s.buffers.current().unwrap().to_string();
        // The notes buffer is Annotation by default; its seed content is
        // "# Notes\n".
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "# Notes\n");
        assert_eq!(s.buffers.get(&bk).unwrap().mode, BufferMode::Annotation);
        s.set_point(0, 0, 0); // on '#'
        // M-x delete-char-forward in Annotation mode: no-op (gate). Without
        // the gate this would turn "# Notes" into " Notes".
        s.dispatch("delete-char-forward", None).unwrap();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "# Notes\n",
            "M-x delete-char-forward must be a no-op in Annotation mode"
        );
        // Contrast: the same command in Accurate mode does the point edit.
        s.toggle_read_only();
        s.dispatch("delete-char-forward", None).unwrap();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            " Notes\n",
            "M-x delete-char-forward must act in Accurate mode"
        );
    }

    // ── plan 016 issue 01: the undo stack (self-insert + backspace) ──────

    /// A small file buffer in `Accurate` mode (the point-accurate editing
    /// shape), rooted in a throwaway project. Returns the store and the
    /// buffer key.
    fn accurate_file_store(content: &str) -> (AppStore, String, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/f.rs"), content).unwrap();
        let mut s = store(dir.path());
        s.open_path("src/f.rs");
        s.toggle_read_only(); // Accurate + editable
        let bk = s.buffers.current().unwrap().to_string();
        (s, bk, dir)
    }

    /// plan 016 issue 01 (UPDATED by issue 04): a self-insert edit sequence
    /// can be FULLY undone, with the buffer text byte-identical to the
    /// expected intermediate state after EVERY undo step (asserted at each
    /// step, not just the end). plan 016 issue 04: the consecutive
    /// self-inserts are ONE coalesced undo step (the self-insert-run rule),
    /// so ONE undo removes the whole run — a per-keystroke step is no longer
    /// the shape (the pre-04 expectation is replaced by the coalescing
    /// pin, and the run boundary itself is pinned in the issue 04 tests
    /// below).
    #[test]
    fn full_undo_roundtrip_self_insert_asserts_every_step() {
        let (mut s, bk, _dir) = accurate_file_store("hello\n");
        // Type "abc" at the start via the real key path (C-x u for undo ties
        // the binding to the behaviour).
        s.set_point(0, 0, 0);
        s.key_event(key("a"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "ahello\n");
        s.key_event(key("b"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "abhello\n");
        s.key_event(key("c"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "abchello\n");
        // issue 04: the consecutive self-inserts are ONE coalesced step.
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            1,
            "the self-insert run is ONE undo step (the coalescing rule)"
        );
        // One undo removes the WHOLE run, asserting the text.
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "hello\n", "undo 1: the whole run");
        // A second undo: the stack is empty → a no-op with a message.
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "hello\n", "past the start: no change");
        assert!(
            s.message.contains("nothing to undo"),
            "empty history must echo a message: {:?}",
            s.message
        );
    }

    /// plan 016 issue 01: backspace (delete-char-before-point) is undoable —
    /// the deleted char is restored exactly.
    #[test]
    fn backspace_undo_restores_deleted_char() {
        let (mut s, bk, _dir) = accurate_file_store("hello\n");
        s.set_point(0, 5, 5); // end of "hello" (char col 5)
        s.delete_char_before_point(); // backspace: remove 'o' (accurate mode)
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "hell\n");
        s.dispatch("undo", None).unwrap();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hello\n",
            "undo must restore the backspaced char"
        );
        // The point lands back on the restored char's position (char col 4).
        assert_eq!(s.point_col(), 4, "point must land char-accurate");
    }

    /// plan 016 issue 02: RET (newline-at-point) is undoable, asserted at
    /// every step with MULTIBYTE content before the edit point — the
    /// restored text is byte-identical and the point is char-accurate (é is
    /// two bytes, so a byte-based undo would land the point one char early,
    /// mid-é). This is one of the five remaining edit paths covered by the
    /// recording hook with no recording change (the RET site already funnels
    /// through `retain_rope_edit`).
    #[test]
    fn ret_undo_restores_multibyte_text_and_char_point() {
        // "café\n": c a f é (é = 2 bytes). 4 chars, 5 bytes.
        let (mut s, bk, _dir) = accurate_file_store("café\n");
        // Point at char 4 = AFTER é (byte 4 is the second byte of é), just
        // before the trailing newline. A byte-based record would put the
        // inverse at byte 4 (mid-é) → col 3 on undo.
        s.set_point(0, 4, 4);
        assert_eq!(s.point_col(), 4, "point is at char col 4 (past the multibyte char)");
        s.key_event(key("RET")); // split before the newline → "café\n\n", point on the new line
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "café\n\n",
            "RET must insert a newline at the point"
        );
        assert_eq!(s.point_line(), 1, "point lands on the new (lower) line");
        // Undo the RET: the exact text (including the multibyte char) is
        // restored and the point is char-accurate.
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "café\n",
            "undo RET must restore the exact text byte-identically"
        );
        assert_eq!(s.point_line(), 0, "point line restored");
        assert_eq!(
            s.point_col(),
            4,
            "point restored to CHAR col 4 (a byte-based undo lands at col 3, the byte mid-é)"
        );
    }

    /// plan 016 issue 02: C-k (kill-line) is undoable. The killed text is
    /// restored exactly and the point lands back at the kill start (char-
    /// accurate). Covered by the recording hook with no recording change.
    #[test]
    fn kill_line_undo_restores_killed_text_and_point() {
        let (mut s, bk, _dir) = accurate_file_store("hello world\nsecond\n");
        s.set_point(0, 5, 5); // between "hello" and " world"
        s.key_event(key("C-k")); // kill " world" (to EOL, keep the newline)
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hello\nsecond\n",
            "C-k must kill to EOL, keeping the newline"
        );
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hello world\nsecond\n",
            "undo must restore the killed text byte-identically"
        );
        assert_eq!(
            s.point_col(),
            5,
            "point must land back at the kill start (char col 5)"
        );
    }

    /// plan 016 issue 02: C-y (yank) is undoable. Undoing a yank removes the
    /// just-inserted text and restores the buffer to its pre-yank state, with
    /// the point back at the yank point (char-accurate). Covered by the
    /// recording hook with no recording change.
    #[test]
    fn yank_undo_restores_pre_yank_text_and_point() {
        let (mut s, bk, _dir) = accurate_file_store("hello world\n");
        // Seed the kill ring with " world" (C-k), then yank it at the start.
        s.set_point(0, 5, 5);
        s.key_event(key("C-k"));
        assert_eq!(s.kill_ring.top(), Some(" world"));
        s.set_point(0, 0, 0);
        s.key_event(key("C-y")); // yank " world" at char 0 → " worldhello\n"
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            " worldhello\n",
            "C-y must insert the yanked text at the point"
        );
        // Undo the C-y only: the yanked text is removed, pre-yank text stays.
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hello\n",
            "undo C-y must remove the yanked text and restore the pre-yank buffer"
        );
        assert_eq!(
            s.point_col(),
            0,
            "point must land back at the yank point (char col 0)"
        );
    }

    /// plan 016 issue 02: the M-y (yank-pop) coalescing DECISION, pinned over
    /// a known C-k … C-y … M-y sequence. M-y is a REPLACEMENT, and it
    /// coalesces with the preceding C-y into ONE undo step (matching emacs):
    /// a single undo removes the whole yank-and-rotate sequence, restoring
    /// the pre-C-y buffer. Scoped to the yank sequence only (the general
    /// self-insert-run rule is issue 04).
    #[test]
    fn yank_pop_coalesces_with_yank_into_one_undo_step() {
        // Seed the kill ring with two entries: kill "alpha" then "beta" so
        // ring = [beta (top), alpha]. C-y yanks the top (beta); M-y rotates
        // to alpha.
        let (mut s, bk, _dir) = accurate_file_store("alpha\nbeta\n");
        s.set_point(0, 0, 0);
        s.key_event(key("C-k")); // kill "alpha" → "\nbeta\n", ring=[alpha]
        assert_eq!(s.kill_ring.top(), Some("alpha"));
        s.set_point(1, 0, 0);
        s.key_event(key("C-k")); // kill "beta" → "\n\n", ring=[beta, alpha]
        assert_eq!(s.kill_ring.top(), Some("beta"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "\n\n");
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            2,
            "two kills recorded (one step each, no coalescing yet)"
        );
        // C-y yanks the top ("beta") at char 0 → "beta\n\n". One step.
        s.set_point(0, 0, 0);
        s.key_event(key("C-y"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "beta\n\n",
            "C-y yanks the top ring entry at the point"
        );
        assert_eq!(s.buffers.get(&bk).unwrap().undo.len(), 3);
        // M-y replaces "beta" with "alpha" (at(1)) → "alpha\n\n". This
        // coalesces with the C-y step: the stack stays at 3, not 4.
        s.key_event(key("M-y"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "alpha\n\n",
            "M-y rotates the yanked text to the previous kill"
        );
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            3,
            "C-y and M-y must coalesce into ONE undo step (2 kills + 1 yank-and-rotate); no coalescing would make this 4"
        );
        // ONE undo removes the whole yank-and-rotate sequence → pre-C-y ("\n\n").
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "\n\n",
            "a single undo must restore the pre-C-y buffer (the whole yank-and-rotate sequence)"
        );
        // The next undos walk the two kills, restoring the original buffer.
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "\nbeta\n",
            "undo 2 restores the second kill (beta)"
        );
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "alpha\nbeta\n",
            "undo 3 restores the first kill (alpha) — the original buffer"
        );
    }

    /// plan 016 issue 02: C-w (kill-region) is undoable. The killed region is
    /// restored byte-identically and the point lands back at the region start
    /// (char-accurate). Covered by the recording hook with no recording
    /// change.
    #[test]
    fn kill_region_undo_restores_region_and_point() {
        let (mut s, bk, _dir) = accurate_file_store("hello world\n");
        // Region [0, 5) = "hello": point at char 5, mark at byte 0.
        s.set_point(0, 5, 5);
        s.buffers.get_mut(&bk).unwrap().mark = Some(0);
        s.key_event(key("C-w")); // kill-region removes "hello" → " world\n"
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            " world\n",
            "C-w must remove the region and push it to the kill ring"
        );
        assert_eq!(s.kill_ring.top(), Some("hello"));
        // Undo restores the region.
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hello world\n",
            "undo C-w must restore the killed region byte-identically"
        );
        assert_eq!(
            s.point_col(),
            0,
            "point must land back at the region start (char col 0)"
        );
    }

    /// plan 016 issue 02 requirement 4: the kill-ring interaction. Undoing a
    /// C-k (or C-w) restores the killed text to the buffer but does NOT pop
    /// the kill ring (emacs's undo leaves the ring alone). A silent
    /// divergence between the buffer and the kill ring after undo is the bug
    /// this prevents — so the ring's contents are asserted after the undo.
    #[test]
    fn undo_of_kill_does_not_pop_the_kill_ring() {
        let (mut s, bk, _dir) = accurate_file_store("hello world\n");
        s.set_point(0, 5, 5);
        s.key_event(key("C-k")); // kill " world" → ring top " world" (len 1)
        assert_eq!(s.kill_ring.top(), Some(" world"));
        assert_eq!(s.kill_ring.len(), 1);
        s.dispatch("undo", None).unwrap();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hello world\n",
            "undo restores the killed text to the buffer"
        );
        assert_eq!(
            s.kill_ring.top(),
            Some(" world"),
            "undo must NOT pop the kill ring (the entry the undo just restored)"
        );
        assert_eq!(
            s.kill_ring.len(),
            1,
            "the ring depth is unchanged by the undo (no silent buffer/ring divergence)"
        );
    }

    /// plan 016 issue 02 requirement 5: the reparse invariant holds for a NEW
    /// path. A C-k undo must re-apply through the SAME `retain_rope_edit`
    /// hook as the forward edit, so the retained parse tree never drifts from
    /// the rope — a ROPE-ONLY undo (mutating the rope without the hook) would
    /// leave the tree at the post-kill length and fail this assert.
    #[test]
    fn kill_line_undo_reparses_through_the_retained_tree_hook() {
        let (mut s, bk, _dir) = accurate_file_store("fn main() {}\n");
        let mtime = s.buffers.get(&bk).unwrap().mtime;
        s.ensure_highlight_for_key(&bk);
        let tree_key = TreeKey::new(&bk, mtime);
        assert!(
            s.highlight_cache.retain_contains(&tree_key),
            "a small Rust buffer keeps a retained parse tree"
        );
        let base_bytes = s.buffers.get(&bk).unwrap().rope.len_bytes();
        // C-k from char 3 (after "fn ") kills "main() {}" (to EOL, keep \n).
        s.set_point(0, 3, 3);
        s.key_event(key("C-k"));
        let after_kill_bytes = s.buffers.get(&bk).unwrap().rope.len_bytes();
        assert_eq!(after_kill_bytes, base_bytes - 9, "C-k removed the 9-char body after 'fn '");
        assert_eq!(
            s.highlight_cache
                .retain_tree(&tree_key)
                .unwrap()
                .tree()
                .root_node()
                .end_byte(),
            after_kill_bytes,
            "the kill edits the retained tree (incremental reparse)"
        );
        // The undo re-applies the inverse through the SAME hook; the tree
        // must grow back with the rope.
        s.dispatch("undo", None).unwrap();
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "fn main() {}\n");
        let after_undo_bytes = s.buffers.get(&bk).unwrap().rope.len_bytes();
        assert_eq!(after_undo_bytes, base_bytes);
        assert_eq!(
            s.highlight_cache
                .retain_tree(&tree_key)
                .unwrap()
                .tree()
                .root_node()
                .end_byte(),
            after_undo_bytes,
            "a C-k undo must re-apply through retain_rope_edit: the retained tree's end must grow back with the rope (a rope-only undo leaves it at the post-kill length)"
        );
    }

    /// plan 016 issue 02 requirement 7: the staleness guard must catch a
    /// byte/char unit mix-up in a NEW path's recording rather than silently
    /// absorb a wrong-range inverse. Simulated by mutation on C-k: a buggy
    /// byte-based recording of killing "é" would record é's BYTE width
    /// (range [3,5), 2 bytes) with the same char text ("é"). The guard's
    /// char-slice check sees range [3,5) as the two chars "é\n" — not "é" —
    /// and REJECTS the step. The discriminating pair: the correct char-width
    /// inverse (range [3,4)) applies.
    #[test]
    fn staleness_guard_catches_byte_char_mixup_in_kill_recording() {
        use crate::model::buffer::UndoStep;
        let (mut s, bk, _dir) = accurate_file_store("café\n");
        // c a f é (é = 2 bytes: 0xC3 0xA9). 4 chars, 5 bytes, + newline.
        // A correct char-based kill of "é" records range [3,4), removed "é".
        // A buggy BYTE-based recording records é's byte range [3,5) with the
        // same char text "é" — the byte/char mix-up.
        s.buffers.get_mut(&bk).unwrap().undo.push(UndoStep {
            // plan 016 issue 03: manual seeds carry explicit ids (the
            // counter is bypassed); non-zero keeps them clear of the
            // fresh/empty-history sentinel.
            id: 1,
            range: 3..5, // byte width (é spans 2 bytes), NOT the char width
            removed: "é".into(),
            inserted: String::new(),
        });
        // The guard must REJECT it (the char-slice [3,5) is "é\n", not "é")
        // instead of silently applying a wrong-range remove.
        s.dispatch("undo", None).unwrap();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "café\n",
            "a wrong-unit inverse must be rejected; the buffer stays untouched"
        );
        assert!(
            s.message.contains("stale"),
            "the byte/char mix-up must surface via the guard's stale message, not vanish silently: {:?}",
            s.message
        );
        // Discriminating pair: the correct char-width inverse for the same
        // kill DOES apply.
        s.buffers.get_mut(&bk).unwrap().undo.push(UndoStep {
            id: 2,
            range: 3..4,
            removed: "é".into(),
            inserted: String::new(),
        });
        s.dispatch("undo", None).unwrap();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "caf\n",
            "the char-width inverse applies (the byte-width one did not)"
        );
    }

    /// plan 016 issue 02 requirement 6: the history cap still holds for a
    /// long sequence of the NEW paths (a path that recorded more than one
    /// step could otherwise bypass 01's cap). Repeated RET (newline-at-point,
    /// one step each — no coalescing for RET) exceeds the cap; the oldest are
    /// dropped and undo stops earlier.
    #[test]
    fn undo_cap_still_holds_for_the_new_paths() {
        use crate::model::buffer::UndoStack;
        let max = UndoStack::MAX_ENTRIES;
        let (mut s, bk, _dir) = accurate_file_store("x\n");
        s.set_point(0, 0, 0);
        // `max + 30` RETs at the moving point: each inserts a "\n" (one undo
        // step each). Well over the cap.
        for _ in 0..(max + 30) {
            s.newline_at_point();
        }
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            max,
            "the stack must be capped at MAX_ENTRIES even for the new paths (oldest dropped)"
        );
        // Undo `max` times: the first 30 newlines were dropped, so undo stops
        // with 30 "\n"s still in front of "x".
        for _ in 0..max {
            s.dispatch("undo", None).unwrap();
        }
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            format!("{}x\n", "\n".repeat(30)),
            "undo stops earlier once the oldest new-path steps are dropped"
        );
        s.dispatch("undo", None).unwrap();
        assert!(s.message.contains("nothing to undo"));
    }

    /// plan 016 issue 01: undo is per-buffer (emacs is buffer-local). Undo in
    /// buffer B must never touch buffer A's history, and a buffer with no
    /// edits echoes "nothing to undo" while leaving its text alone.
    #[test]
    fn undo_is_per_buffer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "alpha\n").unwrap();
        std::fs::write(dir.path().join("src/b.rs"), "beta\n").unwrap();
        let mut s = store(dir.path());
        s.open_path("src/a.rs");
        s.toggle_read_only(); // a: Accurate
        let akey = s.buffers.current().unwrap().to_string();
        s.set_point(0, 0, 0);
        s.insert_text_at_point("X"); // a: "Xalpha\n"
        assert_eq!(s.buffers.get(&akey).unwrap().text(), "Xalpha\n");
        assert_eq!(s.buffers.get(&akey).unwrap().undo.len(), 1, "a has one recorded edit");

        // Switch to b (Accurate) — b has no edits of its own.
        s.open_path("src/b.rs");
        s.toggle_read_only(); // b: Accurate
        let bkey = s.buffers.current().unwrap().to_string();
        s.set_point(0, 0, 0);
        s.dispatch("undo", None).unwrap();
        assert_eq!(
            s.buffers.get(&bkey).unwrap().text(),
            "beta\n",
            "undo on b must not touch b's text (it has no history)"
        );
        assert!(
            s.message.contains("nothing to undo"),
            "b has no history: {:?}",
            s.message
        );
        // a's history is intact (one step still).
        assert_eq!(s.buffers.get(&akey).unwrap().undo.len(), 1);

        // Undo on a (switch back) removes a's own 'X'.
        s.buffers.set_current(&akey);
        s.dispatch("undo", None).unwrap();
        assert_eq!(
            s.buffers.get(&akey).unwrap().text(),
            "alpha\n",
            "undo on a must restore a's own edit"
        );
    }

    /// plan 016 issue 01: killing a buffer drops its undo history — a
    /// reopened file must not inherit stale offsets from the killed buffer.
    #[test]
    fn killing_a_buffer_drops_its_undo_history() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/f.rs"), "original\n").unwrap();
        let mut s = store(dir.path());
        s.open_path("src/f.rs");
        s.toggle_read_only();
        let k = s.buffers.current().unwrap().to_string();
        s.set_point(0, 0, 0);
        s.insert_text_at_point("x"); // "xoriginal\n" (unsaved)
        assert_eq!(s.buffers.get(&k).unwrap().undo.len(), 1);

        // Kill the buffer: its history goes with it.
        s.kill_buffer(&k);
        assert!(s.buffers.get(&k).is_none(), "the killed buffer is gone");

        // Reopen the same file: a FRESH buffer with an empty undo history.
        s.open_path("src/f.rs");
        let k2 = s.buffers.current().unwrap().to_string();
        assert_eq!(
            s.buffers.get(&k2).unwrap().undo.len(),
            0,
            "a reopened file must start with an empty undo history (no stale offsets)"
        );
        // Make it editable to prove undo has nothing (not merely read-only)
        // to undo.
        s.toggle_read_only(); // Accurate
        s.dispatch("undo", None).unwrap();
        assert!(
            s.message.contains("nothing to undo"),
            "the reopened buffer has no history to undo: {:?}",
            s.message
        );
        assert_eq!(s.buffers.get(&k2).unwrap().text(), "original\n");
    }

    /// plan 016 issue 01: the multibyte case. An edit and its undo on a line
    /// with non-ASCII BEFORE the edit point must restore the EXACT text and
    /// the EXACT point (char indices, not bytes — é is two bytes, so a
    /// byte-based undo would land the point off-by-one).
    #[test]
    fn undo_multibyte_restores_exact_text_and_point() {
        // "café\n": c a f é (é = 2 bytes). 4 chars, 5 bytes.
        let (mut s, bk, _dir) = accurate_file_store("café\n");
        s.set_point(0, 4, 4); // char col 4 = after é (end of "café")
        s.key_event(key("x")); // self-insert 'x' → "caféx\n", point char 5
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "caféx\n",
            "insert past the multibyte char"
        );
        assert_eq!(s.point_col(), 5, "point advanced past the inserted char");

        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "café\n",
            "the exact text (including the multibyte char) is restored"
        );
        assert_eq!(
            s.point_line(),
            0,
            "point line restored"
        );
        assert_eq!(
            s.point_col(),
            4,
            "point restored to CHAR col 4 (a byte-based undo would land at 3, the byte mid-é)"
        );
    }

    /// plan 016 issue 01: undo re-applies through the SAME path as an edit —
    /// `retain_rope_edit` — so the retained parse tree never drifts from the
    /// rope. After an edit and an undo, the retained tree's covered byte
    /// length must match the rope's: a ROPE-ONLY undo (mutating the rope
    /// without the reparse hook) leaves the tree at the post-edit length and
    /// fails this assert.
    #[test]
    fn undo_reparses_through_the_retained_tree_hook() {
        let (mut s, bk, _dir) = accurate_file_store("fn main() {}\n");
        let mtime = s.buffers.get(&bk).unwrap().mtime;
        // Populate the retained tree (Rust is a reuse language → a full
        // parse is retained as the incremental baseline).
        s.ensure_highlight_for_key(&bk);
        let tree_key = TreeKey::new(&bk, mtime);
        assert!(
            s.highlight_cache.retain_contains(&tree_key),
            "a small Rust buffer keeps a retained parse tree"
        );
        let base_bytes = s.buffers.get(&bk).unwrap().rope.len_bytes();
        assert_eq!(
            s.highlight_cache.retain_tree(&tree_key).unwrap().tree().root_node().end_byte(),
            base_bytes,
            "baseline tree covers the whole rope"
        );

        // Self-insert 'x' at the start. The edit goes through
        // retain_rope_edit (records undo + edits the retained tree).
        s.set_point(0, 0, 0);
        s.insert_text_at_point("x");
        let after_edit_bytes = s.buffers.get(&bk).unwrap().rope.len_bytes();
        assert_eq!(after_edit_bytes, base_bytes + 1);
        assert_eq!(
            s.highlight_cache.retain_tree(&tree_key).unwrap().tree().root_node().end_byte(),
            after_edit_bytes,
            "the edit edits the retained tree (incremental reparse)"
        );

        // Undo re-applies the inverse through the SAME hook. The retained
        // tree must shrink with the rope.
        s.dispatch("undo", None).unwrap();
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "fn main() {}\n");
        let after_undo_bytes = s.buffers.get(&bk).unwrap().rope.len_bytes();
        assert_eq!(after_undo_bytes, base_bytes);
        assert_eq!(
            s.highlight_cache.retain_tree(&tree_key).unwrap().tree().root_node().end_byte(),
            after_undo_bytes,
            "undo must re-apply through retain_rope_edit: the retained tree's end must shrink with the rope (a rope-only undo leaves it at the post-edit length)"
        );
    }

    /// plan 016 issue 01: the history is capped. Exceeding the cap drops the
    /// OLDEST steps, so undo simply stops earlier — the newest edits always
    /// win.
    #[test]
    fn undo_history_is_capped() {
        use crate::model::buffer::UndoStack;
        let max = UndoStack::MAX_ENTRIES;
        let (mut s, bk, _dir) = accurate_file_store("end\n");
        s.set_point(0, 0, 0);
        // Type `max + 30` self-inserts at the start (well over the cap).
        for _ in 0..(max + 30) {
            s.insert_text_at_point("a");
        }
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            max,
            "the stack must be capped at MAX_ENTRIES (oldest dropped)"
        );
        // Undo `max` times: each removes one 'a'. The first 30 inserts were
        // dropped, so undo stops with 30 'a's still in front of "end".
        for _ in 0..max {
            s.dispatch("undo", None).unwrap();
        }
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            format!("{}end\n", "a".repeat(30)),
            "undo stops earlier once the oldest steps are dropped"
        );
        // One more undo: nothing left.
        s.dispatch("undo", None).unwrap();
        assert!(s.message.contains("nothing to undo"));
    }

    /// plan 016 issue 01: in a read-only buffer undo is a no-op with a
    /// message, and must NOT resurrect the mode or `editable` state (the
    /// `Accurate` ⟹ `editable` invariant stays untouched). The gate holds
    /// even when a history somehow exists (proven by seeding a step directly).
    #[test]
    fn undo_is_a_noop_in_read_only_buffers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/f.rs"), "hello\n").unwrap();
        let mut s = store(dir.path());
        s.open_path("src/f.rs"); // read-only (Annotation, editable = false)
        let bk = s.buffers.current().unwrap().to_string();
        assert!(!s.buffers.get(&bk).unwrap().editable);
        // Seed a step to prove the gate holds even with history present.
        s.buffers.get_mut(&bk).unwrap().undo.push(crate::model::buffer::UndoStep {
            id: 1,
            range: 0..1,
            removed: "h".into(),
            inserted: "H".into(),
        });
        s.dispatch("undo", None).unwrap();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hello\n",
            "read-only: undo must not change the text"
        );
        assert!(
            s.message.contains("read-only"),
            "read-only undo must echo a message: {:?}",
            s.message
        );
        // The history is untouched (not consumed) and mode/editable intact.
        assert_eq!(s.buffers.get(&bk).unwrap().undo.len(), 1, "read-only undo leaves the history");
        assert!(!s.buffers.get(&bk).unwrap().editable, "editable must stay false");
        assert_eq!(
            s.buffers.get(&bk).unwrap().mode,
            BufferMode::Annotation,
            "mode must stay Annotation"
        );
    }

    /// plan 016 issue 03 (WIRE SITE 3 OF 3, driven through the notes path
    /// — `sync_notes_from_doc`, which annotation save/delete/reanchor all
    /// reach): the notes sync replaces the buffer's rope BEHIND a live undo
    /// history. `replace_buffer_text` must drop the history AND reset the
    /// marker: the recorded ranges were stale (pre-03, undo here was a
    /// reachable panic in ropey's `remove`, caught only by the guard), and
    /// a cleared history proves nothing. This sync, however, WROTE the
    /// serialized doc to disk first and reflects that same text into the
    /// buffer, so it re-proves the buffer clean (the fresh sentinel) — the
    /// notes save path of the dirty-flag contract. This drives the real
    /// notes sync path, not a direct `replace_buffer_text` call.
    #[test]
    fn notes_sync_drops_history_and_reproves_clean() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        let key = s.buffers.current().unwrap().to_string();
        assert!(
            s.buffers.get(&key).unwrap().editable,
            "the notes buffer is editable — undo is enabled here"
        );
        // Type enough free text that the buffer clearly exceeds the short
        // empty-doc serialization (the begin/end markers, ~67 chars), so the
        // sync visibly replaces the rope. (The typed free text is not part
        // of the structured notes doc.)
        for _ in 0..80 {
            s.notes_insert_char('a');
        }
        let pre_len = s.buffers.get(&key).unwrap().rope.len_chars();
        assert!(!s.buffers.get(&key).unwrap().undo.is_empty(),
            "typing must build a live undo history");
        assert!(
            s.buffers.get(&key).unwrap().locally_modified(),
            "precondition: the typed notes buffer is modified"
        );
        // The notes sync replaces the buffer's rope with the serialized doc
        // (the free text is not in the doc) — the rope shrinks.
        s.sync_notes_from_doc();
        let post_len = s.buffers.get(&key).unwrap().rope.len_chars();
        assert!(
            post_len < pre_len,
            "the sync must shrink the rope (replaces the content); pre={pre_len} post={post_len}"
        );
        let buf = s.buffers.get(&key).unwrap();
        assert_eq!(
            buf.undo.len(),
            0,
            "the sync must drop the recorded history (site 3 of 3)"
        );
        assert_eq!(
            buf.saved_marker,
            Some(0),
            "the sync re-proves clean: the content IS the just-written disk truth"
        );
        assert!(
            !buf.locally_modified(),
            "the notes sync is a save/load path: the buffer is clean afterwards"
        );
        // Undo after the clear is a no-op with a message, not a panic and
        // not a stale apply — the history is simply gone.
        s.undo();
        assert!(
            s.message.contains("nothing to undo"),
            "a cleared history has nothing to undo: {:?}",
            s.message
        );
        assert_eq!(
            s.buffers.get(&key).unwrap().rope.len_chars(),
            post_len,
            "the no-op undo must not mutate the rope"
        );
    }

    /// plan 016 issue 01 + gate P2-1: the `removed`-text check is load-bearing,
    /// not just the bounds check. A step whose range still FITS the rope but
    /// whose `removed` text no longer matches the rope's text at that range is
    /// stale and must be rejected (dropped + reported) — this is the second
    /// half of the `undo()` validation that the out-of-bounds case
    /// short-circuits past. Seeded directly (the notes path produces the
    /// out-of-bounds half; a fit-but-wrong-text step is the cleanest way to
    /// isolate the text check).
    #[test]
    fn undo_rejects_stale_step_when_removed_text_no_longer_matches() {
        let (mut s, bk, _dir) = accurate_file_store("defgh\n");
        // A step whose range (0..3) fits "defgh\n" but whose `removed` ("abc")
        // does NOT match the rope's text there ("def") → stale on the text
        // check even though the range is in bounds.
        s.buffers.get_mut(&bk).unwrap().undo.push(crate::model::buffer::UndoStep {
            id: 1,
            range: 0..3,
            removed: "abc".into(),
            inserted: "x".into(),
        });
        s.undo();
        assert!(
            s.message.contains("stale"),
            "an in-bounds but text-mismatched step must be reported stale: {:?}",
            s.message
        );
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "defgh\n",
            "the text check must reject the step: the rope is untouched"
        );
        // The stale step was dropped (popped), leaving an empty history.
        assert_eq!(s.buffers.get(&bk).unwrap().undo.len(), 0, "the stale step is dropped");
    }

    /// The undo guard must be a **panic backstop**, not merely a staleness
    /// check: a step whose range is INVERTED (`start > end`) must be rejected
    /// like any other invalid step rather than reaching ropey's `slice`.
    ///
    /// Not reachable today — the recorder builds `char_start..char_start +
    /// len`, so `start <= end` by construction — which is precisely why the
    /// guard must not *depend* on that invariant holding.
    #[test]
    fn undo_rejects_an_inverted_range_instead_of_panicking() {
        let (mut s, bk, _dir) = accurate_file_store("defgh\n");
        // Bound through variables: a literal `3..1` trips
        // `clippy::reversed_empty_ranges` (deny-by-default), and the subject
        // here is the guard, not the lint.
        let (start, end) = (3usize, 1usize);
        s.buffers.get_mut(&bk).unwrap().undo.push(crate::model::buffer::UndoStep {
            id: 1,
            range: start..end, // start > end
            removed: "de".into(),
            inserted: "x".into(),
        });
        s.undo();
        assert!(
            s.message.contains("stale"),
            "an inverted range must be reported stale, not panic: {:?}",
            s.message
        );
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "defgh\n",
            "the rope is untouched"
        );
        assert_eq!(s.buffers.get(&bk).unwrap().undo.len(), 0, "the step is dropped");
    }


    // ── plan 016 issue 03: the saved-state marker and the dirty flag ────

    /// plan 016 issue 03 (the round-trip matrix, each step asserted on the
    /// TEXT and the FLAG): edit → save → clean; edit → save → edit →
    /// modified; edit → save → edit → undo → clean; then edit again →
    /// modified (the reverse direction: a new edit after reaching the
    /// saved state re-sets the flag).
    #[test]
    fn dirty_flag_round_trip_matrix_text_and_flag() {
        let (mut s, bk, dir) = accurate_file_store("hello\n");
        assert!(
            !s.buffers.get(&bk).unwrap().locally_modified(),
            "a freshly opened file proves clean (fresh marker, empty history)"
        );
        // edit → modified
        s.set_point(0, 0, 0);
        s.insert_text_at_point("1"); // "1hello\n"
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "1hello\n");
        assert!(
            s.buffers.get(&bk).unwrap().locally_modified(),
            "an edit moves the position off the marker → modified"
        );
        // save → clean
        assert!(s.save_buffer_key(&bk), "the save must land");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/f.rs")).unwrap(),
            "1hello\n",
            "the save wrote the edited text"
        );
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "1hello\n");
        assert!(
            !s.buffers.get(&bk).unwrap().locally_modified(),
            "edit → save → clean (the marker lands on the saved position)"
        );
        // edit again → modified
        s.insert_text_at_point("2"); // "12hello\n"
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "12hello\n");
        assert!(
            s.buffers.get(&bk).unwrap().locally_modified(),
            "a new edit after the save → modified"
        );
        // undo back to the saved state → clean
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "1hello\n", "undo restores the saved text");
        assert!(
            !s.buffers.get(&bk).unwrap().locally_modified(),
            "edit → save → edit → undo → clean (the position is the saved position again)"
        );
        // a further edit re-sets the flag (the reverse direction)
        s.insert_text_at_point("3"); // "13hello\n"
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "13hello\n");
        assert!(
            s.buffers.get(&bk).unwrap().locally_modified(),
            "a new edit after reaching the saved state re-sets modified"
        );
    }



    /// plan 016 issue 03 (save at an intermediate point — the case a
    /// depth-comparison gets wrong and the marker gets right): edit → edit
    /// → save → undo → undo → **modified**. After the second undo the buffer
    /// sits *BEFORE* the saved state (its text is even the original
    /// disk-at-open text, which no longer equals what the save wrote), so it
    /// differs from disk and must read modified. A `history.len() == saved
    /// depth`-style rule would read it clean — this test carries the
    /// data-loss direction: it FAILS on any implementation that reports
    /// clean while the buffer holds text that differs from disk.
    #[test]
    fn save_at_intermediate_point_undo_past_saved_state_reads_modified() {
        let (mut s, bk, dir) = accurate_file_store("end\n");
        s.set_point(0, 0, 0);
        s.insert_text_at_point("a"); // "aend\n"
        s.insert_text_at_point("b"); // "abend\n" (b lands at char 1)
        assert!(s.save_buffer_key(&bk));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/f.rs")).unwrap(),
            "abend\n",
            "the save wrote the intermediate state"
        );
        assert!(
            !s.buffers.get(&bk).unwrap().locally_modified(),
            "saved: clean"
        );
        // Undo past the saved state: the position is now BEFORE the marker.
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "aend\n", "undo 1");
        assert!(
            s.buffers.get(&bk).unwrap().locally_modified(),
            "one undo leaves the buffer BEFORE the saved state → modified"
        );
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "end\n", "undo 2");
        assert!(
            s.buffers.get(&bk).unwrap().locally_modified(),
            "fully undone PAST the saved state: the buffer differs from disk (\"end\\n\" vs \"abend\\n\") → still modified, never clean"
        );
        assert!(
            s.buffers.get(&bk).unwrap().undo.is_empty(),
            "the history is empty — the marker must still say modified with nothing left to undo"
        );
    }

    /// plan 016 issue 03 (cap eviction of the SAVED marker): the cap drops
    /// the OLDEST steps, so a saved marker can be evicted and the saved
    /// state becomes unreachable by undo. An unreachable marker must read
    /// MODIFIED, never clean-by-default: the position's id can never equal
    /// the evicted id again (ids are unique and monotonic), so the
    /// comparison is conservative by construction.
    #[test]
    fn dirty_flag_cap_eviction_of_saved_marker_reads_modified() {
        use crate::model::buffer::UndoStack;
        let max = UndoStack::MAX_ENTRIES;
        let (mut s, bk, dir) = accurate_file_store("end\n");
        s.set_point(0, 0, 0);
        // Five edits, then save: the marker lands on the 5th step's id.
        for _ in 0..5 {
            s.insert_text_at_point("a");
        }
        assert!(s.save_buffer_key(&bk));
        assert!(
            !s.buffers.get(&bk).unwrap().locally_modified(),
            "saved: clean (marker on the 5th step's id)"
        );
        let marker = s.buffers.get(&bk).unwrap().saved_marker;
        assert_eq!(marker, Some(5), "the marker is the top step's id at save time");
        // Far more edits than the cap: steps 1..15 (including the marker's)
        // get evicted, oldest-first.
        for _ in 0..(max + 10) {
            s.insert_text_at_point("a");
        }
        assert!(
            s.buffers.get(&bk).unwrap().locally_modified(),
            "new edits: modified"
        );
        // Undo everything: the stack empties 15 edits short of the original
        // "end\n" (the first 15 steps were evicted, marker among them).
        for _ in 0..max {
            s.key_event(key("C-x"));
            s.key_event(key("u"));
        }
        let buf = s.buffers.get(&bk).unwrap();
        assert_eq!(buf.undo.len(), 0, "fully undone to the evicted floor");
        assert_eq!(
            buf.text(),
            format!("{}end\n", "a".repeat(15)),
            "undo stopped at the cap's floor"
        );
        assert_ne!(
            buf.text(),
            std::fs::read_to_string(dir.path().join("src/f.rs")).unwrap(),
            "the floor text differs from disk (disk holds the 5-a save)"
        );
        assert!(
            buf.locally_modified(),
            "the saved marker was EVICTED: the saved state is unreachable, so the buffer reads modified, never clean"
        );
        assert_eq!(
            buf.saved_marker,
            marker,
            "the marker itself is untouched — only its target step is gone"
        );
    }

    /// plan 016 issue 03 (the data-loss-direction pin, explicit): a buffer
    /// that CANNOT PROVE it matches the disk state must read modified.
    /// Forced evidence-free marker (`None`) and an unresolvable marker (an
    /// id no step carries) both report modified while the buffer holds
    /// unsaved edits. A clean-by-default marker would fail this test with
    /// unsaved text in memory — the `C-x C-c` stops-asking data-loss path.
    #[test]
    fn dirty_flag_tie_break_unresolvable_marker_reads_modified() {
        let (mut s, bk, _dir) = accurate_file_store("hello\n");
        s.set_point(0, 0, 0);
        s.insert_text_at_point("X"); // "Xhello\n", step id 1
        assert!(s.save_buffer_key(&bk));
        assert!(!s.buffers.get(&bk).unwrap().locally_modified());
        s.insert_text_at_point("Y"); // "XYhello\n", step id 2 — UNSAVED
        assert!(s.buffers.get(&bk).unwrap().locally_modified());

        // Force the evidence-free state (what a history clear with no
        // re-load leaves): no proof exists → modified.
        s.buffers.get_mut(&bk).unwrap().saved_marker = None;
        assert!(
            s.buffers.get(&bk).unwrap().locally_modified(),
            "no evidence (marker None) → modified: a false clean here is the C-x C-c data-loss path"
        );

        // Force an unresolvable marker (an id no step carries — the state
        // after the cap evicted the saved step, made direct): still
        // modified.
        s.buffers.get_mut(&bk).unwrap().saved_marker = Some(99_999);
        assert!(
            s.buffers.get(&bk).unwrap().locally_modified(),
            "an unresolvable marker (evicted / pointing at nothing) → modified"
        );

        // The marker is still the only source of truth: pointing it at the
        // TRUE top step re-proves clean (the buffer IS at that position);
        // pointing it one step earlier reads modified (before the save).
        s.buffers.get_mut(&bk).unwrap().saved_marker = Some(2);
        assert!(!s.buffers.get(&bk).unwrap().locally_modified());
        s.buffers.get_mut(&bk).unwrap().saved_marker = Some(1);
        assert!(s.buffers.get(&bk).unwrap().locally_modified());
    }

    /// plan 016 issue 03 (WIRE SITE 1 OF 3 — the `reload_in_place`
    /// chokepoint, driven through the `g` force-reload route): a disk
    /// reload must clear the history AND reset the marker; the re-read
    /// content is disk truth, so the buffer proves clean and undo is a
    /// no-op with a message, not a stale apply.
    #[test]
    fn force_reload_clears_history_and_resets_marker() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q")); // Accurate + editable
        s.insert_text("X"); // unsaved edit, live history
        assert_eq!(s.buffers.get(&bufk).unwrap().undo.len(), 1);
        assert!(s.buffers.get(&bufk).unwrap().locally_modified());
        // New disk content under the buffer.
        std::fs::write(dir.path().join("src/f.rs"), "fn new() {}\n").unwrap();
        s.reload_current_buffer(); // `g`
        let buf = s.buffers.get(&bufk).unwrap();
        assert_eq!(buf.text(), "fn new() {}\n", "the reload re-read the disk content");
        assert_eq!(buf.undo.len(), 0, "the reload must clear the history (site 1 of 3)");
        assert_eq!(
            buf.saved_marker,
            Some(0),
            "the reload resets the marker to the fresh sentinel (disk truth)"
        );
        assert!(!buf.locally_modified(), "the reloaded buffer proves clean");
        s.dispatch("undo", None).unwrap();
        assert!(
            s.message.contains("nothing to undo"),
            "a cleared history undoes to a message, not a stale apply: {:?}",
            s.message
        );
        assert_eq!(
            s.buffers.get(&bufk).unwrap().text(),
            "fn new() {}\n",
            "the no-op undo leaves the reloaded text alone"
        );
    }

    /// plan 016 issue 03 (WIRE SITE 2 OF 3 — `toggle_ro_accept`, which
    /// assigns `buf.rope` directly): the read-only discard confirm's `y`
    /// must clear the history AND reset the marker (the re-read file is
    /// disk truth → fresh sentinel), making undo a no-op with a message.
    #[test]
    fn toggle_ro_accept_clears_history_and_resets_marker() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q")); // into Accurate
        s.insert_text("X"); // unsaved edit, live history
        assert_eq!(s.buffers.get(&bufk).unwrap().undo.len(), 1);
        s.key_event(key("C-x"));
        s.key_event(key("C-q")); // leave Accurate with unsaved edits → confirm
        assert!(s.toggle_ro_active(), "the discard confirm must arm");
        s.key_event(key("y")); // accept: discard + re-read
        assert!(!s.toggle_ro_active());
        let buf = s.buffers.get(&bufk).unwrap();
        assert_eq!(buf.text(), "fn old() {}\n", "the edit is discarded, disk content re-read");
        assert_eq!(buf.undo.len(), 0, "the accept must clear the history (site 2 of 3)");
        assert_eq!(
            buf.saved_marker,
            Some(0),
            "the accept resets the marker to the fresh sentinel (disk truth)"
        );
        assert!(!buf.locally_modified(), "the accepted buffer proves clean");
        assert!(!buf.editable, "the buffer is read-only again");
        s.dispatch("undo", None).unwrap();
        assert!(
            s.message.contains("read-only") || s.message.contains("nothing to undo"),
            "undo on the cleared, read-only buffer is a no-op with a message: {:?}",
            s.message
        );
        assert_eq!(
            s.buffers.get(&bufk).unwrap().text(),
            "fn old() {}\n",
            "the no-op undo leaves the re-read text alone"
        );
        let _ = &dir;
    }

    /// plan 016 issue 03 (the M-y merge trap): the coalesced step keeps the
    /// M-y step's id (the newer/top id), never the preceding yank's. The
    /// data-loss variant: a buffer SAVED in between the C-y and the M-y
    /// carries a marker on the C-y step's id; after the M-y's merge replaces
    /// that step, the marker must NOT match the merged step — the buffer
    /// holds rotated, unsaved text and must read modified. (If the merge had
    /// kept the older id, this buffer would read clean with unsaved edits.)
    #[test]
    fn m_y_merge_cannot_hijack_the_saved_marker() {
        let (mut s, bk, dir) = accurate_file_store("base\n");
        // Two kills for the ring: "base" (killed first, deeper in the
        // ring) and a marker word killed second.
        s.set_point(0, 0, 0);
        // C-k at the start kills the whole "base" line content... the kill
        // line at (0,0) kills to EOL ("base").
        s.kill_line(); // ring: ["base"]
        // Type a new line content and kill that too: "zzz"
        s.insert_text_at_point("z");
        s.insert_text_at_point("z");
        s.insert_text_at_point("z"); // "zzz\n"
        s.set_point(0, 0, 0);
        s.kill_line(); // ring: ["base", "zzz"] (top = "zzz")
        // C-y yanks "zzz" back at the (post-kill) point.
        s.yank(); // "zzz\n" again
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "zzz\n");
        // SAVE here: the marker lands on the C-y step's id.
        assert!(s.save_buffer_key(&bk));
        assert!(!s.buffers.get(&bk).unwrap().locally_modified());
        let marker_at_save = s.buffers.get(&bk).unwrap().saved_marker;
        // M-y rotates to "base": the merge pops the M-y step AND the C-y
        // step (the marker's target) and pushes ONE coalesced step.
        s.yank_pop();
        let buf = s.buffers.get(&bk).unwrap();
        assert_eq!(buf.text(), "base\n", "the rotation swapped in the previous kill");
        assert_ne!(
            buf.text(),
            std::fs::read_to_string(dir.path().join("src/f.rs")).unwrap(),
            "the rotated text differs from disk (disk holds \"zzz\\n\")"
        );
        assert_eq!(
            buf.saved_marker,
            marker_at_save,
            "the marker is untouched by the merge"
        );
        assert!(
            buf.locally_modified(),
            "the merged step must NOT carry the C-y step's id, or this buffer would read clean while holding unsaved rotated text — the data-loss direction"
        );
        // Undo the coalesced step: one undo removes the WHOLE
        // yank-and-rotate sequence, and the buffer is at the pre-yank
        // (empty-line) state — before the save → modified, not clean.
        s.dispatch("undo", None).unwrap();
        let buf = s.buffers.get(&bk).unwrap();
        assert_eq!(
            buf.text(),
            "\n",
            "one undo removes the whole yank-and-rotate sequence"
        );
        assert!(
            buf.locally_modified(),
            "pre-yank state ≠ saved state → modified"
        );
    }

    /// plan 016 issue 03 (buffer kill / reopen): the marker lives on the
    /// `Buffer`, so killing a dirty buffer and reopening the file must not
    /// leak a stale clean (or dirty) state across the kill — the reopened
    /// buffer starts with NO history and a FRESH marker.
    #[test]
    fn killed_buffer_marker_cannot_leak_into_the_reopen() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/f.rs"), "original\n").unwrap();
        let mut s = store(dir.path());
        s.open_path("src/f.rs");
        s.toggle_read_only(); // Accurate
        let k = s.buffers.current().unwrap().to_string();
        s.set_point(0, 0, 0);
        s.insert_text_at_point("x"); // "xoriginal\n", unsaved
        assert!(s.buffers.get(&k).unwrap().locally_modified());
        let killed_seq = s.buffers.get(&k).unwrap().undo_seq;
        let killed_marker = s.buffers.get(&k).unwrap().saved_marker;
        assert_eq!(killed_marker, Some(0), "precondition: marker still fresh (never saved)");

        s.kill_buffer(&k);
        // Reopen the same file.
        s.open_path("src/f.rs");
        let k2 = s.buffers.current().unwrap().to_string();
        let fresh = s.buffers.get(&k2).unwrap();
        assert_eq!(fresh.undo.len(), 0, "the reopened file has no history");
        assert_eq!(
            fresh.saved_marker,
            Some(0),
            "the reopened file carries a FRESH marker — no stale state leaked"
        );
        assert_eq!(
            fresh.undo_seq,
            0,
            "the id counter is per-buffer: it restarts (no id can collide with a dead marker)"
        );
        assert!(
            !fresh.locally_modified(),
            "the reopened file proves clean at its own disk content"
        );
        assert_eq!(fresh.text(), "original\n", "the reopen reads disk, not the killed buffer's edits");
        assert!(
            killed_seq >= 1,
            "the killed buffer really had recorded steps (seq {killed_seq}) — its state existed but died with it"
        );
    }

    // ── plan 016 issue 04: the redo stack + the self-insert-run rule ──

    /// Full round trip (acceptance): two coalesced typing runs, separated
    /// by a point motion, undone completely and then redone completely,
    /// with the buffer text asserted BYTE-IDENTICAL at EVERY step (not just
    /// the endpoints) — including the "undo after redo" flip-flop and the
    /// step-id stability across the round trip (the marker seam's
    /// precondition).
    #[test]
    fn redo_full_roundtrip_asserts_every_step() {
        let (mut s, bk, _dir) = accurate_file_store("hello\n");
        s.set_point(0, 0, 0);
        // Run 1: "abc" — the consecutive self-inserts coalesce into ONE
        // step (issue 04's stated rule).
        s.key_event(key("a"));
        s.key_event(key("b"));
        s.key_event(key("c"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "abchello\n");
        assert_eq!(s.buffers.get(&bk).unwrap().undo.len(), 1, "run 1 is ONE step");
        let id_abc = s.buffers.get(&bk).unwrap().undo.position_id();
        // A point motion ENDS the run — even one that ends back at the same
        // column (LEFT then RIGHT): the intervening command, not just the
        // contiguity, separates the two runs.
        s.key_event(key("LEFT"));
        s.key_event(key("RIGHT"));
        // Run 2: "def" at the same column the first run ended at.
        s.key_event(key("d"));
        s.key_event(key("e"));
        s.key_event(key("f"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "abcdefhello\n");
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            2,
            "the two runs are TWO steps (the motion between them ended run 1)"
        );
        let id_def = s.buffers.get(&bk).unwrap().undo.position_id();
        assert!(
            id_def > id_abc,
            "precondition: two distinct step ids ({id_abc} vs {id_def})"
        );

        // UNDO: the "def" run goes first, then the "abc" run.
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "abchello\n", "undo 1: the def run");
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "hello\n", "undo 2: the abc run");
        assert_eq!(s.buffers.get(&bk).unwrap().redo.len(), 2, "both inverses are on the redo stack");

        // REDO: the "abc" run comes back first, then the "def" run — the
        // universal binding (C-x U) for the first, the byte-based second
        // binding (C-M-7, the app shape of ESC 0x1F) for the second.
        s.key_event(key("C-x"));
        s.key_event(key("U"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "abchello\n", "redo 1: the abc run");
        s.key_event(parse_sequence("C-M-7").unwrap()[0]);
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "abcdefhello\n",
            "redo 2 (C-M-7): the def run"
        );
        // Redo past the end: the redo stack is empty → a no-op with a
        // message, and the text is untouched.
        s.key_event(key("C-x"));
        s.key_event(key("U"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "abcdefhello\n",
            "past the end: no change"
        );
        assert!(
            s.message.contains("nothing to redo"),
            "empty redo stack must echo a message: {:?}",
            s.message
        );

        // UNDO AFTER REDO: the redone steps re-entered the undo stack —
        // undo flips back, and the inverse re-pops onto redo.
        s.key_event(key("C-x"));
        s.key_event(key("u"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "abchello\n", "undo after redo: def off");
        s.key_event(key("C-x"));
        s.key_event(key("U"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "abcdefhello\n",
            "redo after that undo: def back"
        );
        // The redone steps kept their ORIGINAL ids (moved, not minted):
        // the position is exactly where the typing run left it.
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.position_id(),
            id_def,
            "the round trip is id-stable: the marker seam reads the same position"
        );
    }

    /// issue 04, pinned: a NEW EDIT clears the redo stack (the emacs rule
    /// — not optional: a stale redo branch after an edit would redo onto
    /// content that no longer exists). Edit → undo → edit again: redo is
    /// unavailable, and the redo no-op is a message, not a stale apply.
    #[test]
    fn new_edit_clears_redo_stack() {
        let (mut s, bk, _dir) = accurate_file_store("hello\n");
        s.set_point(0, 0, 0);
        s.key_event(key("a"));
        s.key_event(key("b")); // one coalesced step
        assert_eq!(s.buffers.get(&bk).unwrap().undo.len(), 1);
        s.dispatch("undo", None).unwrap();
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "hello\n");
        assert_eq!(s.buffers.get(&bk).unwrap().redo.len(), 1, "the undo's inverse is on the redo stack");
        // A new edit (a self-insert in a fresh run) clears it.
        s.key_event(key("z"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "zhello\n");
        assert_eq!(
            s.buffers.get(&bk).unwrap().redo.len(),
            0,
            "the new edit must clear the redo stack"
        );
        s.dispatch("redo", None).unwrap();
        assert!(
            s.message.contains("nothing to redo"),
            "the cleared redo stack is a no-op with a message: {:?}",
            s.message
        );
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "zhello\n",
            "the no-op redo must not touch the text (no stale apply)"
        );
    }

    /// issue 04, the spec's known-sequence pin: type `abc`, move point with
    /// an arrow, type `def` — undo removes `def` as its OWN step first, then
    /// the `abc` run. Two steps, not one; the order is def-then-abc. And the
    /// "any other command ends the run" half pinned through BOTH paths: the
    /// arrow key's non-printable path and the registry `dispatch` path
    /// (C-SPC between two runs).
    #[test]
    fn mouse_and_tree_commands_end_the_self_insert_run() {
        // gate P2 (016-04): the run-ends rule is "any path that RUNS a command
        // clears the marker", but the mouse/tree handlers never pass through
        // `key_event`, where the key-path clears live. So a click or a wheel
        // tick left the run armed, and a later self-insert merged ACROSS it —
        // measured pre-fix: seq `a`,`b`,click,`c` left ONE undo step, so a
        // single `C-x u` removed "abc" spanning a mouse command. Each probe
        // types "ab", runs the command, types "c": a broken run = 2 steps.
        for (label, which) in [
            ("mouse_click_position", 0u8),
            ("mouse_scroll_up", 1),
            ("mouse_scroll_down", 2),
        ] {
            let (mut s, bk, _dir) = accurate_file_store("hello\n");
            s.set_point(0, 0, 0);
            s.key_event(key("a"));
            s.key_event(key("b"));
            assert!(
                s.self_insert_run.is_some(),
                "precondition: the run is armed before {label}"
            );
            match which {
                // A click at the run's end (col 2) IS point motion.
                0 => s.mouse_click_position(0, 2),
                1 => s.mouse_scroll_up(),
                _ => s.mouse_scroll_down(),
            }
            // Assert the MECHANISM (the marker), which is the same source of
            // truth the merge rule reads — the undo count below depends on
            // where the next key lands, which the tree case in particular
            // changes by switching the shown file.
            assert!(
                s.self_insert_run.is_none(),
                "{label} must clear the self-insert run marker"
            );
            s.key_event(key("c"));
            assert_eq!(
                s.buffers.get(&bk).unwrap().undo.len(),
                2,
                "{label} must end the self-insert run (1 = the run leaked across it)"
            );
        }

        // The tree row click is the same class of non-`key_event` input path.
        let (mut s, _bk, _dir) = accurate_file_store("hello\n");
        s.tree.visible = true;
        s.set_point(0, 0, 0);
        s.key_event(key("a"));
        s.key_event(key("b"));
        assert!(
            s.self_insert_run.is_some(),
            "precondition: the run is armed before tree_click_row"
        );
        s.tree_click_row(1);
        assert!(
            s.self_insert_run.is_none(),
            "tree_click_row must clear the self-insert run marker (it runs a command: it switches the shown file)"
        );

        // The deliberate other half of the rule: a path that ran NO command
        // leaves the run ARMED — like a pending prefix press or an
        // unbound-key echo. This is why the clears sit AFTER each guard
        // rather than at the top of the handler, and it is the counter-case
        // that stops someone "simplifying" the fix into a blanket clear.
        let (mut s, bk, _dir) = accurate_file_store("hello\n");
        s.set_point(0, 0, 0);
        s.key_event(key("a"));
        s.push_view(ViewId::Home); // not the buffer view: the click no-ops
        s.mouse_click_position(0, 2);
        assert!(
            s.self_insert_run.is_some(),
            "a click that ran no command must leave the run armed"
        );
        s.key_event(key("b"));
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            1,
            "a click that ran no command must NOT break the run"
        );
    }

    #[test]
    fn typing_run_arrow_typing_run_is_two_steps_in_order() {
        let (mut s, bk, _dir) = accurate_file_store("hello\n");
        s.set_point(0, 0, 0);
        s.key_event(key("a"));
        s.key_event(key("b"));
        s.key_event(key("c"));
        s.key_event(key("LEFT")); // an arrow between the two runs
        s.key_event(key("d"));
        s.key_event(key("e"));
        s.key_event(key("f"));
        // "abc", point back one (col 2), "def" inserted at col 2:
        // "ab" + "def" + "chello".
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "abdefchello\n");
        assert_eq!(s.buffers.get(&bk).unwrap().undo.len(), 2, "two steps, not one");
        s.dispatch("undo", None).unwrap();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "abchello\n",
            "the first undo removes the NEWER (def) run"
        );
        s.dispatch("undo", None).unwrap();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hello\n",
            "the second undo removes the abc run"
        );

        // The registry-command variant of the same rule: ANY registry
        // command between two typing runs ends the run — pinned here with
        // C-SPC (set-mark), which runs through `dispatch` (the arrow keys
        // above end the run through the non-printable path instead). The
        // mark lands exactly at the run's end, so contiguity alone cannot
        // save the merge: only the marker's absence may.
        let (mut s, bk, _dir) = accurate_file_store("word\n");
        s.set_point(0, 0, 0);
        s.key_event(key("a")); // "aword\n", point 1 — the run's top step: 0..1
        s.key_event(key("C-SPC")); // set-mark at 1 — a registry command
        s.key_event(key("b")); // "abword\n" — contiguous at 1, but after a command
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            2,
            "the set-mark between the runs ends the run: 'a' and 'b' are TWO \
             steps, not one (contiguity at the same column must not revive it)"
        );
        s.dispatch("undo", None).unwrap();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "aword\n",
            "the first undo removes the post-mark 'b', not the whole run"
        );
    }

    /// issue 04, the multibyte acceptance pin: a coalesced typing run
    /// restores exactly, on CHAR indices. v1 key input is ASCII-only
    /// (`Key::printable` gates `0x20..=0x7E`, so the key path self-inserts
    /// only through `char_value`), so the pin is split: the key-path half
    /// drives the rule with ASCII typing AROUND multibyte text ("café\n":
    /// char col 4 is byte 5 — a byte-based run record would put the range
    /// start at byte 4, mid-é, and the undo would chew into the é), and the
    /// non-ASCII-in-the-run half drives the same recording layer with the
    /// typing-run marker set exactly as the key path sets it (store-level
    /// self-inserts — the rule lives in `retain_rope_edit`, which is where
    /// a multi-byte self-insert would flow once the key gate allows it).
    /// Byte-identical at every step, char-accurate point.
    #[test]
    fn multibyte_self_insert_run_restores_exactly_on_char_indices() {
        let (mut s, bk, _dir) = accurate_file_store("café\n"); // 5 chars, 6 bytes
        s.set_point(0, 4, 4); // char col 4 = after the é (byte 5), before the \n
        s.key_event(key("a"));
        s.key_event(key("b"));
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "caféab\n"); // 7 chars, 8 bytes
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            1,
            "the run coalesced across the multibyte boundary"
        );
        s.dispatch("undo", None).unwrap();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "café\n",
            "byte-identical restore (a byte-based range start would land mid-é)"
        );
        assert_eq!(s.point_col(), 4, "char-accurate point (byte 4 is the SECOND byte of the é)");
        s.dispatch("redo", None).unwrap();
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "caféab\n", "byte-identical redo");
        assert_eq!(s.point_col(), 4, "char-accurate point on the redo");

        // Non-ASCII in the run: store-level self-inserts with the run marker
        // set as the key path sets it (one successful self-insert).
        s.set_point(0, 0, 0);
        s.self_insert_run = Some(bk.clone());
        s.insert_text_at_point("a");
        s.self_insert_run = Some(bk.clone());
        s.insert_text_at_point("é"); // the multibyte char continues the run
        s.self_insert_run = Some(bk.clone());
        s.insert_text_at_point("é");
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "aéécaféab\n",
            "10 chars, 12 bytes: the run spans the multibyte boundary"
        );
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            2,
            "the a-é-é run is ONE step, on top of the (re)done ab run"
        );
        s.dispatch("undo", None).unwrap();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "caféab\n",
            "byte-identical: the run restored exactly on char indices (a byte-based
            range would stop mid-run and leave the first é or eat the c)"
        );
        assert_eq!(s.point_col(), 0, "char-accurate point on the char-indexed undo");
        s.dispatch("redo", None).unwrap();
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "aéécaféab\n",
            "byte-identical redo of the multibyte run"
        );
        assert_eq!(s.point_col(), 0, "char-accurate point on the redo");
    }

    /// issue 04, the MARKER SEAM (03 owns the mechanism; this pins the redo
    /// half): a redone step must re-enter the undo stack with its ORIGINAL
    /// id, and the clean/modified readings through the round trip must be
    /// right. Both directions:
    ///
    ///  - Case A (data-loss direction): edit → save → edit → undo → REDO
    ///    PAST the saved position: the buffer MUST read modified (it holds
    ///    the unsaved text again).
    ///  - Case B (the id discriminator): edit → save → undo PAST the saved
    ///    position → redo BACK to the saved position: the buffer MUST read
    ///    clean. That only happens if the re-pushed step carries its
    ///    ORIGINAL id — a minted id would break the marker match and read
    ///    modified, hiding a provably-saved state.
    #[test]
    fn redo_reenters_with_the_original_id_for_the_marker() {
        // Case A: redo past the saved position reads modified.
        let (mut s, bk, _dir) = accurate_file_store("hello\n");
        s.set_point(0, 0, 0);
        s.insert_text_at_point("X"); // "Xhello\n", step id 1
        assert!(s.save_buffer_key(&bk));
        assert!(!s.buffers.get(&bk).unwrap().locally_modified());
        s.insert_text_at_point("Y"); // "XYhello\n", step id 2 — UNSAVED
        assert!(s.buffers.get(&bk).unwrap().locally_modified());
        s.dispatch("undo", None).unwrap(); // "Xhello\n" — back on the saved id
        assert!(!s.buffers.get(&bk).unwrap().locally_modified(), "undo back to the save reads clean");
        s.dispatch("redo", None).unwrap(); // "XYhello\n" — PAST the saved position
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "XYhello\n");
        assert!(
            s.buffers.get(&bk).unwrap().locally_modified(),
            "redo past the saved position must read MODIFIED (unsaved text re-landed)"
        );

        // Case B: undo past, redo back to the saved position, reads clean.
        let (mut s, bk, _dir) = accurate_file_store("hello\n");
        s.set_point(0, 0, 0);
        s.insert_text_at_point("X"); // step id 1
        s.insert_text_at_point("Y"); // step id 2 — "XYhello\n"
        assert!(s.save_buffer_key(&bk));
        assert_eq!(s.buffers.get(&bk).unwrap().saved_marker, Some(2));
        assert!(!s.buffers.get(&bk).unwrap().locally_modified());
        s.dispatch("undo", None).unwrap(); // "Xhello\n" — position back to id 1
        assert!(s.buffers.get(&bk).unwrap().locally_modified(), "past the save: modified");
        s.dispatch("redo", None).unwrap(); // "XYhello\n" — back to the saved position
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "XYhello\n");
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.position_id(),
            2,
            "the redone step re-entered with its ORIGINAL id (2), not a minted one"
        );
        assert!(
            !s.buffers.get(&bk).unwrap().locally_modified(),
            "redo back to the saved position must read CLEAN — a fresh id would read modified"
        );
    }

    /// issue 04, the marker × run-merge seam (mirrors 02's
    /// `m_y_merge_cannot_hijack_the_saved_marker` for the self-insert rule):
    /// a typing run that COALESCES across a saved-state marker must not
    /// read clean against it — the merged step keeps the NEW step's id
    /// (the newest recorded one), so a marker captured mid-run can never
    /// match the merged step. The key path cannot reach this state (a save
    /// is a command, and a command ends the run — `dispatch` clears the
    /// marker), so the pin synthesizes the seam exactly as the store path
    /// carries it: the run marker set, a second self-insert continues the
    /// run, and the marker is set to the first keystroke's position — the
    /// mid-run save the decision exists to survive.
    #[test]
    fn self_insert_run_merge_cannot_hijack_the_saved_marker() {
        let (mut s, bk, _dir) = accurate_file_store("base\n");
        s.set_point(0, 0, 0);
        s.self_insert_run = Some(bk.clone());
        s.insert_text_at_point("a"); // "abase\n" — step id 1
        s.self_insert_run = Some(bk.clone());
        s.insert_text_at_point("b"); // "babase\n" — coalesces: ONE step
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            1,
            "the run coalesced into one step"
        );
        // A marker captured after the FIRST keystroke (a save mid-run):
        s.buffers.get_mut(&bk).unwrap().saved_marker = Some(1);
        let buf = s.buffers.get(&bk).unwrap();
        assert_ne!(
            buf.undo.position_id(),
            1,
            "precondition: the merged step's position is not the mid-run marker's"
        );
        assert!(
            buf.locally_modified(),
            "the merged step must NOT carry the first keystroke's id, or this \
             buffer would read clean while holding unsaved text — the data-loss \
             direction"
        );
    }

    /// issue 04, the two INNER conditions of the stated rule (not just
    /// the command side pinned by `typing_run_arrow_typing_run_is_two_steps_in_order`):
    /// with the run marker set, a self-insert is NOT merged when (a) it does
    /// not start where the run's top step ends (the contiguity arm), (b) the
    /// run's top step is not itself a pure insertion, (c) the marker's
    /// buffer is not THIS buffer (the same-buffer arm), or (d) this edit is
    /// not a pure insertion (the pure-insertion arm). Store-level: the
    /// marker is set exactly as the key path sets it, and (b)/(d) reach
    /// the replacement top and the deletion through `retain_rope_edit`
    /// itself (the key path cannot — a non-self-insert editing key clears
    /// the marker).
    #[test]
    fn each_stated_arm_of_the_run_rule_gates_the_merge() {
        // (a) contiguity: an insert that starts mid-buffer (not at the
        // run's end) is its own step even with the marker set.
        let (mut s, bk, _dir) = accurate_file_store("base\n");
        s.set_point(0, 0, 0);
        s.self_insert_run = Some(bk.clone());
        s.insert_text_at_point("a"); // "abase\n" — the run's top step: 0..1
        s.set_point(0, 3, 3); // mid-buffer: NOT where the run's top step ends
        s.self_insert_run = Some(bk.clone());
        s.insert_text_at_point("b"); // "abasbe\n"
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            2,
            "a non-contiguous self-insert must not join the run: {:?}",
            s.buffers.get(&bk).unwrap().text()
        );
        // (b) pure-insertion top: a replacement (deleted text) at the
        // run's end is NOT a run top a self-insert may join — even
        // contiguous.
        let (mut s, bk, _dir) = accurate_file_store("base\n");
        let old_rope = s.buffers.get(&bk).unwrap().rope.clone();
        s.self_insert_run = Some(bk.clone());
        s.retain_rope_edit(&bk, &old_rope, 0, 1, "z"); // "zase\n" — top step replaced "b"
        s.set_point(0, 1, 1);
        let old_rope = s.buffers.get(&bk).unwrap().rope.clone();
        s.self_insert_run = Some(bk.clone());
        s.retain_rope_edit(&bk, &old_rope, 1, 1, "q"); // "zqase\n" — contiguous
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            2,
            "a self-insert must not merge into a top step that deleted text: {:?}",
            s.buffers.get(&bk).unwrap().text()
        );
        // (c) same buffer: the marker names the buffer whose preceding
        // command was the self-insert — a run in a DIFFERENT buffer never
        // continues into this one, no matter how contiguous.
        let (mut s, bk, _dir) = accurate_file_store("base\n");
        std::fs::write(_dir.path().join("src/g.rs"), "fn g() {}\n").unwrap();
        s.open_path("src/g.rs");
        let other = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q")); // Accurate mode for the store-level insert
        s.set_point(0, 0, 0);
        // A run INSIDE `other` itself:
        s.self_insert_run = Some(other.clone());
        s.insert_text_at_point("x"); // `other`'s run: step 0..1
        // Now the marker names `bk` (a self-insert just happened in the
        // OTHER buffer); the edit lands in `other`, contiguous at its top.
        s.self_insert_run = Some(bk.clone());
        s.insert_text_at_point("y");
        assert_eq!(
            s.buffers.get(&other).unwrap().undo.len(),
            2,
            "the other buffer's run must not continue into this buffer: {:?}",
            s.buffers.get(&other).unwrap().text()
        );
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            0,
            "the named buffer's stack is untouched"
        );
        // (d) pure insertion: a DELETION at the run's end does not join the
        // run even with the marker set and contiguity exact — only pure
        // insertions coalesce (a merged deletion would make the undo step
        // restore text the typing run never removed).
        let (mut s, bk, _dir) = accurate_file_store("base\n");
        s.set_point(0, 0, 0);
        s.self_insert_run = Some(bk.clone());
        s.insert_text_at_point("a"); // "abase\n" — the run's top step: 0..1
        s.set_point(0, 1, 1);
        let old_rope = s.buffers.get(&bk).unwrap().rope.clone();
        s.self_insert_run = Some(bk.clone());
        s.retain_rope_edit(&bk, &old_rope, 1, 2, ""); // delete "a": "base\n"
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            2,
            "a deletion must not coalesce into the typing run: {:?}",
            s.buffers.get(&bk).unwrap().text()
        );
    }

    /// issue 04, the redo stack's discipline (mirrors the undo stack's):
    /// per buffer (buffer A's redo never fires on buffer B, and B's edit
    /// does not clear A's redo), cleared at a rope-replacing site, capped at
    /// `UndoStack::MAX_ENTRIES` (the undo stack's cap), and dropped with the
    /// buffer on kill.
    #[test]
    fn redo_stack_obey_the_undo_stack_discipline() {
        // Per buffer: two Accurate buffers, each with its own redo step.
        let (mut s, bk, _dir) = accurate_file_store("hello\n");
        std::fs::write(_dir.path().join("src/g.rs"), "fn g() {}\n").unwrap();
        s.set_point(0, 0, 0);
        s.key_event(key("a"));
        s.key_event(key("b")); // buffer A: one coalesced step
        s.dispatch("undo", None).unwrap();
        assert_eq!(s.buffers.get(&bk).unwrap().redo.len(), 1, "A's undo left a redo step");
        // Buffer B: open, enter Accurate, one edit, one undo.
        s.open_path("src/g.rs");
        let bk2 = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        s.insert_text("z");
        s.dispatch("undo", None).unwrap();
        assert_eq!(s.buffers.get(&bk2).unwrap().redo.len(), 1, "B's undo left a redo step");
        assert_eq!(
            s.buffers.get(&bk).unwrap().redo.len(),
            1,
            "B's edit must NOT clear A's redo stack (per buffer)"
        );
        // Redo on B (the current buffer) fires B's step only.
        s.dispatch("redo", None).unwrap();
        assert!(
            s.buffers.get(&bk2).unwrap().text().ends_with("z"),
            "B's redo re-lands B's edit (append-at-end)"
        );
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "hello\n",
            "B's redo must not touch A's text"
        );
        assert_eq!(s.buffers.get(&bk).unwrap().redo.len(), 1, "A's redo step survives B's redo");
        // Kill B: its redo history dies with the buffer.
        s.kill_buffer(&bk2);
        assert!(s.buffers.get(&bk2).is_none(), "the killed buffer (and its redo stack) is gone");

        // Cleared at a rope-replacing site: a disk reload (`g`) drops BOTH
        // stacks — a stale redo step is the same reachable panic as a stale
        // undo step (01's gate P1), with a different trigger.
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q")); // Accurate + editable
        s.insert_text("X"); // unsaved edit, live history
        s.dispatch("undo", None).unwrap();
        assert_eq!(s.buffers.get(&bufk).unwrap().redo.len(), 1, "precondition: a live redo step");
        std::fs::write(dir.path().join("src/f.rs"), "fn new() {}\n").unwrap();
        s.reload_current_buffer(); // `g`
        assert_eq!(
            s.buffers.get(&bufk).unwrap().redo.len(),
            0,
            "the reload must clear the REDO stack too (site 1 of 3)"
        );
        s.dispatch("redo", None).unwrap();
        assert!(
            s.message.contains("nothing to redo"),
            "the cleared redo stack is a no-op with a message: {:?}",
            s.message
        );
        assert_eq!(
            s.buffers.get(&bufk).unwrap().text(),
            "fn new() {}\n",
            "the no-op redo leaves the reloaded text alone"
        );

        // Capped at the undo stack's cap: 105 edits → 100 undo steps (the
        // cap evicts the oldest) → undoing all 100 pushes exactly the cap
        // onto the redo stack. The redo stack must not grow past it.
        let (mut s, bk, _dir) = accurate_file_store("hello\n");
        s.set_point(0, 0, 0);
        for _ in 0..105 {
            s.insert_text_at_point("x");
        }
        assert_eq!(
            s.buffers.get(&bk).unwrap().undo.len(),
            UndoStack::MAX_ENTRIES,
            "precondition: the undo cap evicted the oldest 5 steps"
        );
        for _ in 0..UndoStack::MAX_ENTRIES {
            s.dispatch("undo", None).unwrap();
        }
        assert_eq!(
            s.buffers.get(&bk).unwrap().redo.len(),
            UndoStack::MAX_ENTRIES,
            "the redo stack is bounded by the SAME cap as the undo stack"
        );
        s.dispatch("undo", None).unwrap();
        assert!(
            s.message.contains("nothing to undo"),
            "the evicted steps are unrecoverable in either direction"
        );
    }

    /// issue 04, the redo half of 01/02's stale-step backstop: a redo step
    /// whose recorded range no longer fits the CURRENT rope (a content
    /// replacement that did not clear the redo stack — the class
    /// `drop_undo_history` exists to prevent) must be dropped and reported,
    /// never applied (a stale range would make ropey's `remove` panic,
    /// `Char range out of bounds`). A DELETION step is used: its inverse
    /// (`inserted` = the deleted char) is the text the redo validates
    /// against, so a replaced rope mismatches it. Mirrors the mirrored
    /// tie-break: the saved-state evidence is dropped, so the buffer reads
    /// modified.
    #[test]
    fn stale_redo_guard_drops_and_reports_instead_of_applying() {
        let (mut s, bk, _dir) = accurate_file_store("hello\n");
        s.set_point(0, 0, 0);
        s.key_event(key("C-d")); // delete 'h' → "ello\n"; step: `inserted` = "h"
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "ello\n");
        s.dispatch("undo", None).unwrap(); // "hello\n"; the inverse is on the redo stack
        assert_eq!(s.buffers.get(&bk).unwrap().redo.len(), 1);
        // Sanity: the guard passes against the live rope (not a false
        // positive) — redo re-lands the deletion exactly.
        s.dispatch("redo", None).unwrap();
        assert_eq!(s.buffers.get(&bk).unwrap().text(), "ello\n", "a live redo step applies");
        s.dispatch("undo", None).unwrap(); // back to "hello\n"
        // Simulate a rope replacement that did NOT flow through the edit
        // path (the caller-miss the backstop exists for — the same
        // scenario 01's gate P1 found on the undo side).
        s.buffers.get_mut(&bk).unwrap().rope = ropey::Rope::from_str("replaced\n");
        s.dispatch("redo", None).unwrap();
        assert!(
            s.message.contains("stale"),
            "the stale redo step must be reported: {:?}",
            s.message
        );
        assert_eq!(
            s.buffers.get(&bk).unwrap().text(),
            "replaced\n",
            "the stale step must not be applied"
        );
        assert_eq!(
            s.buffers.get(&bk).unwrap().redo.len(),
            0,
            "the stale step is dropped (it is already popped)"
        );
        assert!(
            s.buffers.get(&bk).unwrap().locally_modified(),
            "the tie-break: the saved-state evidence is dropped — modified, not clean"
        );
    }

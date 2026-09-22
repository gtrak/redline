use super::*;

impl AppStore {
    /// Feed one keypress from the terminal, as a priority chain of named
    /// modals: the first active modal that consumes the key ends the chain,
    /// and a key no modal consumes reaches `dispatch_key` (the keymap
    /// engine). A modal that swallows every key uses a unit-returning
    /// handler and ends its guard with `return`; a modal that may fall
    /// through to the engine uses a handler that reports whether it
    /// consumed the key and gates the fall-through on that.
    pub fn key_event(&mut self, key: Key) {
        if self.quit {
            return;
        }
        // Quit save-prompt (plan 004 issue 04): swallows every key
        // (y / n / ! / C-g; anything else is a no-op, no "unbound key"
        // echo mid-prompt) and routes to the state machine.
        if self.quit_prompt_active() {
            self.quit_prompt_key(key);
            return;
        }
        // Transient menu (issue 002): topmost overlay. Swallows every key
        // except C-g (a listed leaf closes and runs, a listed prefix
        // descends, non-listed keys are ignored).
        if self.menu_open() {
            self.menu_key_event(key);
            return;
        }
        // Armed discard confirmation (issue 002): `y` executes,
        // `n`/C-g/ESC cancel; other keys are swallowed.
        if self.discard_armed() {
            self.discard_key_event(key);
            return;
        }
        // Toggle-read-only discard confirm (plan 005 issue 01): `y`
        // discards and makes the buffer read-only, `n`/C-g/ESC cancel;
        // every other key is swallowed (no "unbound key" echo mid-prompt).
        if self.toggle_ro_active() {
            self.toggle_ro_key(key);
            return;
        }
        // External-change reload confirm (issue-external-change-reload):
        // `y` reloads from disk (discarding the unsaved edits), `n`/C-g/ESC
        // cancel (the edits and the changed-on-disk marker stay); every
        // other key is swallowed (no "unbound key" echo mid-prompt).
        if self.reload_confirm_active() {
            self.reload_confirm_key(key);
            return;
        }
        // Picker: printable chars extend the query, Backspace/C-h edit it,
        // RET / arrows / C-n / C-p drive it; other keys fall through.
        if self.picker.is_some() && self.picker_key_event(key) {
            return;
        }
        // Commit editor: edits the message; every key is routed (to the
        // editor or the keymap engine).
        if self.top_view() == ViewId::CommitEditor {
            self.commit_editor_key_event(key);
            return;
        }
        // Branch-create name prompt: printable chars extend, RET creates,
        // C-g / ESC cancel; other keys are swallowed.
        if self.branch_create.is_some() {
            self.branch_create_key_event(key);
            return;
        }
        // Isearch: printable self-inserts, C-s/C-r navigate, RET/C-g;
        // other keys are swallowed.
        if self.isearch.active {
            self.isearch_key_event(key);
            return;
        }
        // Goto-line: digits build the number, RET confirms, C-g cancels;
        // other keys are swallowed.
        if self.goto_line_active {
            self.goto_line_key_event(key);
            return;
        }
        // Annotation prompt: printable chars build the note, RET commits,
        // C-g/ESC cancel; other keys are swallowed.
        if self.note_prompt_active {
            self.note_prompt_key_event(key);
            return;
        }
        // Search-query prompt: printable chars extend, RET starts,
        // C-g/ESC cancel; other keys are swallowed.
        if self.search_prompt.is_some() {
            self.search_prompt_key_event(key);
            return;
        }
        // Notes / editable buffer: printable chars append, Backspace
        // deletes; a bound sequence or chord reaches the engine first,
        // otherwise fall through.
        if self.top_view() == ViewId::Buffer
            && self.buffers.current().and_then(|k| self.buffers.get(k).map(|b| b.editable && b.path.is_some())).unwrap_or(false)
            && self.notes_edit_key_event(key)
        {
            return;
        }
        // Tree sidebar: arrows / page keys move the cursor, RET opens the
        // file; other keys fall through.
        if self.tree_visible()
            && matches!(self.top_view(), ViewId::Buffer | ViewId::Home)
            && self.tree_key_event(key)
        {
            return;
        }
        // C-g in the results view cancels the in-flight search (the view
        // stays open on the partial results) — intercepted before the
        // global C-g so the advertised `C-g cancel search` works. The
        // picker guard keeps C-g closing an open palette (the global
        // intercept) instead of cancelling the search underneath it.
        if key == Key::ctrl_char('g')
            && self.top_view() == ViewId::Search
            && self.picker.is_none()
        {
            self.search_cancel();
            return;
        }
        // C-g aborts from any state: it clears the pending sequence and
        // closes the picker before the keymap engine can start one.
        if key == Key::ctrl_char('g') {
            self.cancel();
            return;
        }
        self.dispatch_key(key);
    }

    /// Picker modal (guard 6). While the picker is open, printable
    /// characters extend the query, Backspace/C-h edit it, and a small set
    /// of keys drives the picker (RET runs, C-g cancels, arrows / C-n /
    /// C-p move); everything else falls through to the keymap engine.
    /// Returns `true` when the key is consumed, `false` to fall through.
    fn picker_key_event(&mut self, key: Key) -> bool {
        if let Some(c) = key.char_value() {
            // Stash list: `x` drops the selected entry (magit's drop
            // key) instead of extending the filter query.
            if self.picker_kind() == Some(PickerKind::Stash) && c == 'x' {
                let idx = self
                    .picker_filtered()
                    .get(self.picker_selected())
                    .map(|(cand, _)| cand.name.clone())
                    .and_then(|n| n.parse::<usize>().ok());
                if let Some(idx) = idx {
                    self.stash_drop(idx);
                }
                return true;
            }
            // Annotations list (015-01): `d` deletes the selected
            // annotation (the buffer view's `d` reached from the
            // picker) instead of extending the filter query.
            if self.picker_kind() == Some(PickerKind::Annotations) && c == 'd' {
                self.annotations_picker_delete();
                return true;
            }
            self.picker_query_char(c);
            return true;
        }
        // Query editing: Backspace (and C-h, the control-h byte some
        // terminals emit for it) removes the last query character.
        if key.code == KeyCode::Backspace || key == Key::ctrl_char('h') {
            self.picker_query_backspace();
            return true;
        }
        if key.code == KeyCode::Enter {
            self.run_selected();
            return true;
        }
        if key.code == KeyCode::Down || key == Key::ctrl_char('n') {
            self.picker_select_next();
            return true;
        }
        if key.code == KeyCode::Up || key == Key::ctrl_char('p') {
            self.picker_select_prev();
            return true;
        }
        // Other keys fall through to the keymap engine; the
        // unbound-key echo is suppressed while the picker is open
        // (see dispatch_key).
        false
    }

    /// Commit editor (issue 08, guard 7): printable / backspace / RET /
    /// arrow keys edit the message; ESC aborts, and that is intercepted
    /// here, before the keymap engine. C-g clears an armed prefix only
    /// (Emacs convention), not the whole buffer. Only the C-c C-c / C-c C-k
    /// bindings reach the engine, so the `C-c` prefix pending state is
    /// visible in the status line. A bare q types "q" (it is not a command
    /// here). Edits clear any armed prefix; a bare `C-c` arms the prefix
    /// via the engine. This modal always consumes the key.
    fn commit_editor_key_event(&mut self, key: Key) {
        if let Some(c) = key.char_value() {
            self.commit_editor_insert(c);
            self.pending.clear();
            return;
        }
        if key.code == KeyCode::Backspace || key == Key::ctrl_char('h') {
            self.commit_editor_backspace();
            self.pending.clear();
            return;
        }
        if key.code == KeyCode::Enter {
            self.commit_editor_newline();
            self.pending.clear();
            return;
        }
        match key.code {
            KeyCode::Left => self.commit_editor_move(EditorMove::Left),
            KeyCode::Right => self.commit_editor_move(EditorMove::Right),
            KeyCode::Up => self.commit_editor_move(EditorMove::Up),
            KeyCode::Down => self.commit_editor_move(EditorMove::Down),
            _ => {}
        }
        if key.code == KeyCode::Left || key.code == KeyCode::Right {
            self.pending.clear();
            return;
        }
        if key.code == KeyCode::Up || key.code == KeyCode::Down {
            self.pending.clear();
            return;
        }
        if key.code == KeyCode::Escape {
            self.commit_editor_abort();
            return;
        }
        if key == Key::ctrl_char('g') {
            self.pending.clear();
            return;
        }
        // C-c … (and any other unintercepted key) goes through the engine.
        self.dispatch_key(key);
    }

    /// Branch-create name prompt (issue 08, guard 8): printable chars
    /// extend the name, RET creates, C-g / ESC cancels.
    fn branch_create_key_event(&mut self, key: Key) {
        if key == Key::ctrl_char('g') || key.code == KeyCode::Escape {
            self.branch_create_cancel();
            return;
        }
        if key.code == KeyCode::Enter {
            self.branch_create_confirm();
            return;
        }
        if key.code == KeyCode::Backspace || key == Key::ctrl_char('h') {
            self.branch_create_backspace();
            return;
        }
        if let Some(c) = key.char_value() {
            self.branch_create_char(c);
        }
        // Other keys: swallow (no "unbound key" echo mid-prompt).
    }

    /// Isearch mode (guard 9): every printable self-inserts into the query
    /// (run before any keymap dispatch, the isearch analogue of the notes
    /// editable branch in plan-002 issue 05). Chords keep their isearch
    /// semantics: C-s next, C-r reverse, RET end, C-g cancel, DEL rubout.
    fn isearch_key_event(&mut self, key: Key) {
        if key == Key::ctrl_char('g') {
            self.isearch_cancel();
            return;
        }
        if key.code == KeyCode::Enter {
            self.isearch_confirm();
            return;
        }
        // PART A fix (item 5): C-s / C-r while isearch is active repeat
        // the search (next / previous match) instead of being swallowed.
        if key == Key::ctrl_char('s') {
            self.isearch_next();
            return;
        }
        if key == Key::ctrl_char('r') {
            self.isearch_prev();
            return;
        }
        if let Some(c) = key.char_value() {
            self.isearch_query_char(c);
            return;
        }
        if key.code == KeyCode::Backspace || key == Key::ctrl_char('h') {
            self.isearch_backspace();
        }
        // Other keys: swallow (don't echo "unbound key" mid-search).
    }

    /// Goto-line mode (guard 10): digits build the line number, RET
    /// confirms, C-g cancels.
    fn goto_line_key_event(&mut self, key: Key) {
        if key == Key::ctrl_char('g') {
            self.goto_line_cancel();
            return;
        }
        if key.code == KeyCode::Enter {
            self.goto_line_confirm();
            return;
        }
        if key.code == KeyCode::Backspace {
            self.goto_line_backspace();
            return;
        }
        if let Some(c) = key.char_value()
            && c.is_ascii_digit()
        {
            self.goto_line_digit(c);
        }
        // Other keys: swallow.
    }

    /// Annotation prompt (plan 005 issue 02, guard 11): printable chars
    /// build the note text, Backspace edits it, RET commits (record written
    /// to the notes file; the cue appears immediately), C-g/ESC cancel.
    fn note_prompt_key_event(&mut self, key: Key) {
        if key == Key::ctrl_char('g') || key.code == KeyCode::Escape {
            self.note_prompt_cancel();
            return;
        }
        if key.code == KeyCode::Enter {
            self.note_prompt_confirm();
            return;
        }
        if key.code == KeyCode::Backspace || key == Key::ctrl_char('h') {
            self.note_prompt_backspace();
            return;
        }
        if let Some(c) = key.char_value() {
            self.note_prompt_char(c);
        }
        // Other keys: swallow (no "unbound key" echo mid-prompt).
    }

    /// Search-query prompt mode (C-c p s s / M-s o, guard 12): printable
    /// chars extend the query, Backspace/C-h edit it, RET starts the
    /// search, C-g/ESC cancel.
    fn search_prompt_key_event(&mut self, key: Key) {
        if key == Key::ctrl_char('g') || key.code == KeyCode::Escape {
            self.search_prompt_cancel();
            return;
        }
        if key.code == KeyCode::Enter {
            self.search_prompt_confirm();
            return;
        }
        if key.code == KeyCode::Backspace || key == Key::ctrl_char('h') {
            self.search_prompt_backspace();
            return;
        }
        if let Some(c) = key.char_value() {
            self.search_prompt_char(c);
        }
        // Other keys: swallow (don't echo "unbound key" mid-prompt).
    }

    /// Notes / editable buffer editing (issue 09, guard 13): when the
    /// current buffer is editable (the notes buffer or any locally-owned
    /// file), printable chars append and Backspace deletes. Bounded editing
    /// (commit-editor precedent): no cursor movement in v1.
    ///
    /// Interception order (issue 05, finding 1): a key that completes or
    /// extends a bound sequence reaches the keymap engine FIRST (so
    /// `C-x g` / `C-c p f` dispatch while editing), mirroring the commit
    /// editor's careful order. Only a printable that binds nothing
    /// self-inserts; C-g keeps its global cancel (clears pending, does not
    /// close the notes buffer). Returns `true` when the key is consumed,
    /// `false` to fall through to the engine.
    fn notes_edit_key_event(&mut self, key: Key) -> bool {
        // C-g cancels pending / closes overlays from any state,
        // including while editing (the advertised C-g matrix).
        if key == Key::ctrl_char('g') {
            self.cancel();
            return true;
        }
        let mut seq = self.pending.clone();
        seq.push(key);
        let extends_sequence = matches!(
            self.engine.resolve(&seq),
            Some(Lookup::Command(_)) | Some(Lookup::Pending)
        );
        // A printable that is a DEPTH-1 leaf command (g/j/k/q/G/n/p...)
        // self-inserts while typing: firing view commands on plain
        // letters destroyed unsaved notes text (g -> reload-buffer).
        // Only chords (C-x, C-c, M-...) and prefix continuations reach
        // the engine.
        let printable_leaf_command = key.char_value().is_some()
            && self.pending.is_empty()
            && matches!(self.engine.resolve(&seq), Some(Lookup::Command(_)))
            && !self.engine.prefix_exists(&seq);
        if !self.pending.is_empty() || (extends_sequence && !printable_leaf_command) {
            // A pending prefix (or a key that starts/continues a bound
            // sequence) must reach the engine before any self-insert.
            self.dispatch_key(key);
            return true;
        }
        // The current buffer's edit mode selects the editing SHAPE (plan 015
        // issue 03): in `Accurate` mode the coarse-model keys (self-insert,
        // Backspace, RET, C-k, C-d, M-d/M-DEL, C-o, C-t) route to the
        // point-accurate commands; in `Annotation` mode every one of them
        // keeps today's behaviour EXACTLY (append / backspace-at-end, the
        // rest fall through to the engine). `Accurate` ⟹ `editable`, and the
        // guard only runs for editable buffers, so this is well-defined.
        // emacs split: `M-d` = kill-word FORWARD, `M-DEL` = backward-kill-word.
        let accurate = self
            .buffers
            .current()
            .and_then(|k| self.buffers.get(k).map(|b| b.mode == BufferMode::Accurate))
            .unwrap_or(false);
        if let Some(c) = key.char_value() {
            if accurate {
                self.insert_text_at_point(&c.to_string());
            } else {
                self.notes_insert_char(c);
            }
            return true;
        }
        if key.code == KeyCode::Backspace || key == Key::ctrl_char('h') {
            // M-DEL (Alt+Backspace) is kill-word-backward in `Accurate` mode;
            // a plain Backspace / C-h is the one-char delete.
            if accurate && key.code == KeyCode::Backspace && key.alt && !key.ctrl {
                self.kill_word_backward();
            } else if accurate {
                self.delete_char_before_point();
            } else {
                self.notes_backspace();
            }
            return true;
        }
        // Accurate-mode control keys (plan 015 issue 03). All are freed from
        // their previous Buffer-view roles: `C-d` was half-page scroll (now
        // delete-char-forward, the emacs binding); `RET`/`C-k`/`C-o`/`C-t`/
        // `M-d`/`M-DEL` were unbound. `C-u` STAYS half-page scroll (universal
        // argument is 015 item 9, out of scope). Annotation mode falls through
        // every one of these to the engine (unchanged behaviour).
        if accurate {
            if key.code == KeyCode::Enter {
                self.newline_at_point();
                return true;
            }
            if key == Key::ctrl_char('k') {
                self.kill_line();
                return true;
            }
            if key == Key::ctrl_char('d') {
                self.delete_char_forward();
                return true;
            }
            if key == Key::ctrl_char('o') {
                self.open_line();
                return true;
            }
            if key == Key::ctrl_char('t') {
                self.transpose_chars();
                return true;
            }
            // M-d (Alt+char 'd'): kill-word FORWARD (emacs `kill-word`). M-DEL
            // (Alt+Backspace) is `kill_word_backward`, handled in the Backspace
            // branch above (Alt+Backspace is a Backspace code, so it never
            // reaches here).
            if key.code == KeyCode::Char('d') && key.alt && !key.ctrl {
                self.kill_word_forward();
                return true;
            }
        }
        // Other keys fall through to the keymap engine (motion, view
        // commands, C-x C-s save, etc.).
        false
    }

    /// Tree sidebar (issue 09, guard 14): when the tree is visible and the
    /// main view is the buffer view, arrows / page keys move the tree
    /// cursor and RET opens the selected file. The emacs motion keys
    /// (C-n/C-p/j/k) still scroll the file, so arrows and file-motion are
    /// cleanly split. 06a: home renders in the buffer slot, so the tree
    /// stays fully usable on top of it — RET opens the file and replaces
    /// home. Returns `true` when the key is consumed, `false` to fall
    /// through.
    fn tree_key_event(&mut self, key: Key) -> bool {
        match key.code {
            KeyCode::Down | KeyCode::PageDown => {
                self.tree_move_down();
                true
            }
            KeyCode::Up | KeyCode::PageUp => {
                self.tree_move_up();
                true
            }
            KeyCode::Enter => {
                self.tree_open_selected();
                true
            }
            _ => false,
        }
    }

    fn dispatch_key(&mut self, key: Key) {
        let mut seq = self.pending.clone();
        seq.push(key);
        match self.engine.resolve(&seq) {
            Some(Lookup::Command(cmd)) => {
                let cmd = cmd.to_string();
                self.pending.clear();
                let _ = self.dispatch(&cmd, None);
            }
            Some(Lookup::Pending) => {
                self.pending = seq;
            }
            None => {
                self.pending.clear();
                if self.picker.is_none() {
                    self.minibuffer_message(&format!("unbound key: {key}"));
                }
            }
        }
    }

    /// Dispatch-by-name through the store's own registry: clone the
    /// command out (the registry is a field of this store, so the
    /// handler may not borrow it while running), then run it here.
    pub fn dispatch(&mut self, name: &str, arg: Option<String>) -> Result<(), RegistryError> {
        let command = self
            .registry
            .get(name)
            .cloned()
            .ok_or_else(|| RegistryError::UnknownCommand(name.to_string()))?;
        // jump-highlight lifetime: the landing highlight lives ONE command
        // — it is set during the jump and cleared at the start of the
        // next command dispatch (a jump command replaces it: the clear
        // happens here, the jump's set happens in its handler, after).
        self.jump_highlight = None;
        // emacs `recenter-top-bottom`: the position only advances when the
        // immediately-preceding command was also recenter; any other
        // command resets the cycle (emacs `recenter-last-op`), so a fresh
        // C-l starts at the first position (middle).
        if name != "recenter" {
            self.recenter_cycle = 0;
        }
        command.run(self, arg);
        Ok(())
    }

    /// Quit interception (plan 004 issue 04): `C-x C-c` (and the palette
    /// `quit`) no longer flips `quit` directly. With NO locally-modified
    /// buffer the quit proceeds immediately (existing behavior); with ≥1
    /// modified buffer the save-prompt state machine starts over the
    /// modified set snapshotted at interception time (oldest-first).
    pub fn begin_quit(&mut self) {
        self.clear_pending();
        let modified: Vec<String> = self
            .buffers
            .list()
            .into_iter()
            .rev() // MRU order reversed → oldest-first
            .filter(|(_, b)| b.locally_modified)
            .map(|(k, _)| k.to_string())
            .collect();
        if modified.is_empty() {
            self.quit = true;
            return;
        }
        self.quit_prompt = Some(QuitPrompt { pending: modified });
        self.quit_prompt_show();
    }

    /// Whether the quit save-prompt is active (a modified buffer is being
    /// offered to save/skip).
    pub fn quit_prompt_active(&self) -> bool {
        self.quit_prompt.is_some()
    }

    /// The display name of the buffer currently offered by the quit
    /// save-prompt (its path, or the `*scratch*` sentinel when pathless).
    pub fn quit_prompt_buffer(&self) -> Option<String> {
        let key = self
            .quit_prompt
            .as_ref()
            .and_then(|p| p.pending.first())?;
        match self.buffers.get(key).and_then(|b| b.path.as_ref()) {
            Some(p) => Some(p.display().to_string()),
            None => Some(key.clone()),
        }
    }

    /// Render the prompt for the head of the snapshot in the minibuffer row.
    fn quit_prompt_show(&mut self) {
        let Some(name) = self.quit_prompt_buffer() else {
            return;
        };
        self.minibuffer_message(&format!(
            "Save this buffer: {name}? (y, n, !, C-g)"
        ));
    }

    /// Re-render the prompt AFTER a failed save left an error message in
    /// the minibuffer: compose the prompt with the error so the decision
    /// line stays visible alongside it (plan 004 issue 05d, carried 004-04
    /// review P2). The prompt comes FIRST: the composed line wraps at the
    /// pane width, leaving the error (e.g. `save failed: ...`) on its own
    /// continuation row. A no-op when the prompt is not active.
    fn quit_prompt_show_with_error(&mut self) {
        let Some(name) = self.quit_prompt_buffer() else {
            return;
        };
        self.minibuffer_message(&format!(
            "Save this buffer: {name}? (y, n, !, C-g) — {}",
            self.message
        ));
    }

    /// Answer one key of the quit save-prompt. `y` saves the offered buffer
    /// (a failed save reports the error and re-prompts the SAME buffer),
    /// `n` skips it, `!` saves this and ALL remaining snapshotted buffers
    /// then quits, `C-g` cancels the whole quit. Every other key is
    /// swallowed (no "unbound key" echo mid-prompt).
    pub fn quit_prompt_key(&mut self, key: Key) {
        if key == Key::ctrl_char('g') {
            self.quit_prompt_cancel();
            return;
        }
        let Some(c) = key.char_value() else { return };
        match c {
            'y' => {
                let Some(p) = self.quit_prompt.as_mut() else {
                    return;
                };
                let Some(asked) = p.pending.first().cloned() else {
                    return;
                };
                if self.save_buffer_key(&asked) {
                    self.quit_prompt_advance();
                } else {
                    // A failed save already reported the error in the
                    // minibuffer; the snapshot head is untouched, so the
                    // same buffer is re-offered on the next y/n/!/C-g —
                    // redisplay the prompt alongside the error so the
                    // decision line stays visible.
                    self.quit_prompt_show_with_error();
                }
            }
            'n' => {
                self.quit_prompt_advance();
            }
            '!' => {
                let mut rest = self
                    .quit_prompt
                    .take()
                    .map(|p| p.pending)
                    .unwrap_or_default();
                for (i, k) in rest.iter().enumerate() {
                    if !self.save_buffer_key(k) {
                        // Report the failure and re-prompt from the FAILED
                        // buffer (the already-saved ones drop off), with the
                        // prompt redisplayed alongside the error.
                        self.quit_prompt = Some(QuitPrompt {
                            pending: rest.split_off(i),
                        });
                        self.quit_prompt_show_with_error();
                        return;
                    }
                }
                // Every snapshotted buffer saved: clear the prompt and quit.
                self.quit_prompt = None;
                self.quit = true;
            }
            _ => {}
        }
    }

    /// Drop the answered buffer from the snapshot; the last answer quits.
    fn quit_prompt_advance(&mut self) {
        let answered = self
            .quit_prompt
            .as_mut()
            .and_then(|p| p.pending.drain(0..1).next());
        if answered.is_none() {
            return;
        }
        if self
            .quit_prompt
            .as_ref()
            .is_some_and(|p| p.pending.is_empty())
        {
            self.quit_prompt = None;
            self.quit = true;
        } else {
            self.quit_prompt_show();
        }
    }

    /// C-g: cancel the whole quit. Buffers already answered `y` are kept
    /// saved; the rest are untouched. 004-03 cancel discipline applies to
    /// the rest of the state (pending / picker / mark), and the cancel is
    /// echoed like the other prompt cancels.
    fn quit_prompt_cancel(&mut self) {
        self.quit_prompt = None;
        self.cancel();
        self.minibuffer_message("cancel");
    }
}

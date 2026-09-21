use super::*;

impl AppStore {
    /// Selection cursor of the buffer-list view.
    pub fn buffer_list_selected(&self) -> usize {
        self.buffer_list_selected
    }

    /// Insert text at the end of the current buffer (the shared editing
    /// primitive behind notes / scratch typing); false when no buffer is
    /// current or the buffer is not editable.
    pub fn insert_text(&mut self, text: &str) -> bool {
        let Some(key) = self.buffers.current() else {
            return false;
        };
        let key = key.to_string();
        // Check editable before mutating.
        let editable = self.buffers.get(&key).map(|b| b.editable).unwrap_or(false);
        if !editable {
            return false;
        }
        let pos = self
            .buffers
            .get(&key)
            .map(|b| b.rope.len_chars())
            .unwrap_or(0);
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.insert(pos, text);
            // A local edit: the buffer now differs from disk (the
            // light-editing flag, plan decision #6).
            buf.locally_modified = true;
            // Plan 007 issue 04: record the edit on the retained parse
            // tree so the next ensure_highlight reparses incrementally.
            if let Some(old_rope) = old_rope {
                self.retain_rope_edit(&key, &old_rope, pos, pos, text);
            }
            // Invalidate the highlight cache for this buffer (text changed).
            self.invalidate_highlight_for_key(&key);
            // Keep the insertion row (the new end of the buffer) inside the
            // visible window (issue 003-02: typing near the bottom keeps the
            // active region in view). The file view's window is the full
            // `viewport_lines` (its title/indicator live in the same area).
            self.buffer_keep_insert_visible();
            true
        } else {
            false
        }
    }

    /// Keep the current buffer's last line (the insertion row) inside the
    /// visible file-view window after an edit; no-op when there is no current
    /// buffer or the row is already visible. Uses the shared window math with
    /// the file view's window size (`viewport_lines`).
    fn buffer_keep_insert_visible(&mut self) {
        let key = self.buffers.current().map(String::from);
        let Some(key) = key else {
            return;
        };
        let total = self.buffers.get(&key).map(|b| b.line_count()).unwrap_or(0);
        if total == 0 {
            return;
        }
        let insert_row = total - 1;
        let window = self.viewport_lines.max(1);
        let next = keep_cursor_visible(self.scroll_top(), insert_row, total, window);
        if next != self.scroll_top() {
            self.scroll.insert(key, next);
        }
    }

    /// Invalidate the highlight cache entry for a buffer key (called
    /// after an edit so the next render rebuilds the highlight).
    pub(super) fn invalidate_highlight_for_key(&mut self, key: &str) {
        let buf = match self.buffers.get(key) {
            Some(b) => b,
            None => return,
        };
        let path = match &buf.path {
            Some(p) => p.clone(),
            None => return,
        };
        let cache_key = CacheKey::new(&path, buf.mtime, self.theme.name());
        // Remove the cache entry so it's rebuilt on next ensure_highlight.
        // (We can't do this cleanly without a remove method on HighlightCache;
        // instead we clear the whole cache — simple and correct.)
        if self.highlight_cache.get(&cache_key).is_some() {
            self.highlight_cache.clear();
        }
    }

    /// Record a rope edit on the buffer's retained parse tree (plan 007
    /// issue 04) so the next `ensure_highlight` can reparse incrementally.
    /// `old_rope` is the PRE-edit rope (the char positions refer to it).
    /// No-op when the buffer has no retained tree (plain text, big file,
    /// non-reuse language, or never highlighted yet) — those keep the
    /// full-parse path.
    pub(super) fn retain_rope_edit(
        &mut self,
        key: &str,
        old_rope: &Rope,
        char_start: usize,
        char_end: usize,
        new_text: &str,
    ) {
        let Some(buf) = self.buffers.get(key) else { return };
        if buf.is_big() || buf.path.is_none() {
            return;
        }
        let edit =
            highlight::rope_edit_to_input_edit(old_rope, char_start, char_end, new_text);
        self.highlight_cache
            .retain_apply_edit(&TreeKey::new(key, buf.mtime), &edit);
    }

    /// Switch to (creating if needed) the `*scratch*` buffer.
    /// 06a: an EXPLICIT affordance (M-o / C-x o / M-x) — scratch is no
    /// longer auto-created at boot; opening it replaces the home view.
    pub fn open_scratch(&mut self) {
        let key = SCRATCH_NAME.to_string();
        if self.buffers.get(&key).is_none() {
            self.buffers.insert(None, String::new());
        }
        self.buffers.set_current(&key);
        self.normalize_top_view();
        self.minibuffer_message("switched to *scratch*");
    }

    /// Save the current buffer to its on-disk path (issue 09, notes +
    /// any editable buffer). No-op when there is no current buffer.
    pub fn save_buffer(&mut self) {
        let Some(key) = self.buffers.current() else {
            self.minibuffer_message("save-buffer: no current buffer");
            return;
        };
        let key = key.to_string();
        self.save_buffer_key(&key);
    }

    /// Save the buffer with `key` to its on-disk path (plan 004 issue 04:
    /// the quit save-prompt must save buffers that are not the current one).
    /// Updates the buffer's mtime and clears `locally_modified`. Returns
    /// `true` when the write landed; on any refusal/failure the minibuffer
    /// message reports the reason and `false` is returned.
    pub fn save_buffer_key(&mut self, key: &str) -> bool {
        let (path, text) = {
            let buf = match self.buffers.get(key) {
                Some(b) => b,
                None => {
                    self.minibuffer_message("save-buffer: no buffer");
                    return false;
                }
            };
            // 006-02b item 1: ownership guard — an external (registry /
            // tooling) source is a cache shared by every project on the
            // machine; refuse the write even if the buffer somehow reached
            // edit mode.
            if !self.buffer_is_project_owned(key) {
                self.minibuffer_message(
                    "save-buffer: external buffer is read-only (not project-owned)",
                );
                return false;
            }
            if !buf.editable {
                self.minibuffer_message("save-buffer: buffer is read-only");
                return false;
            }
            let path = match &buf.path {
                Some(p) => p.clone(),
                None => {
                    self.minibuffer_message("save-buffer: no file (scratch)");
                    return false;
                }
            };
            (path, buf.rope.to_string())
        };
        match std::fs::write(&path, &text) {
            Ok(()) => {
                let mtime = std::fs::metadata(&path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                if let Some(buf) = self.buffers.get_mut(key) {
                    buf.mtime = mtime;
                    buf.locally_modified = false;
                    buf.changed_on_disk = false;
                }
                // Watcher self-write suppression (plan 005 issue 01): the
                // next watcher event for this path is OUR own save. Record
                // the post-save mtime so `apply_project_change` can tell
                // our write (mtime still matches → suppress) from a
                // genuinely later external write (mtime differs → still
                // conflicts).
                self.saved_paths.insert(key.to_string(), mtime);
                self.invalidate_highlight_for_key(key);
                // plan 005 issue 02: the automatic anchoring runs in the
                // same pass as the edit-mode save — the saved content is
                // the anchor truth.
                self.reanchor_for_key(key);
                // Saving the notes file itself re-parses its structured
                // section from the saved text (the buffer is the source
                // of truth while open).
                if self.notes_key().as_deref() == Some(key) {
                    self.notes_doc_mtime = Some(mtime);
                    self.notes_buffer_dirty = true;
                    self.ensure_notes_doc();
                }
                self.minibuffer_message(&format!("wrote {}", path.display()));
                true
            }
            Err(e) => {
                self.minibuffer_message(&format!("save failed: {e}"));
                false
            }
        }
    }

    /// Replace a buffer's text (the notes-buffer sync path).
    pub(super) fn replace_buffer_text(&mut self, key: &str, text: &str) {
        let rope = Rope::from_str(text);
        if let Some(buf) = self.buffers.get_mut(key) {
            buf.rope = rope;
        }
        self.drop_retained_tree(key);
    }

    /// `C-x C-q` (plan 015 issue 02: the per-buffer edit-MODE toggle;
    /// emacs `toggle-read-only`): flip the current FILE buffer between the
    /// edit modes. Entering `Accurate` makes the buffer `Accurate` +
    /// `editable = true`; leaving it returns `Annotation` and the buffer's
    /// BASELINE editability (`buffer_baseline_editable`: the notes buffer
    /// and scratch stay editable in both modes — inherently editable —
    /// while plain file buffers are read-only in `Annotation`). The
    /// invariant is `Accurate` ⟹ `editable`; `editable` stays the gate for
    /// whether text may be modified at all. Leaving `Accurate` with
    /// unsaved edits on a read-only-baseline buffer arms the discard
    /// confirm (`y` discards + goes read-only, `n`/C-g/ESC cancel and keep
    /// accurate mode); the notes baseline loses nothing, so no confirm
    /// there. Non-file buffers (scratch) and non-buffer views are no-ops
    /// with a minibuffer message.
    pub fn toggle_read_only(&mut self) {
        if self.top_view() != ViewId::Buffer {
            self.minibuffer_message("toggle-read-only: not a buffer view");
            return;
        }
        let Some(key) = self.buffers.current().map(str::to_string) else {
            self.minibuffer_message("toggle-read-only: no current buffer");
            return;
        };
        let (mode, locally_modified, owned, baseline) = {
            let buf = match self.buffers.get(&key) {
                Some(b) => b,
                None => {
                    self.minibuffer_message("toggle-read-only: no current buffer");
                    return;
                }
            };
            if buf.path.is_none() {
                // Scratch (no on-disk path): nothing to toggle.
                self.minibuffer_message("toggle-read-only: scratch has no file");
                return;
            }
            (
                buf.mode,
                buf.locally_modified,
                self.buffer_is_project_owned(&key),
                self.buffer_baseline_editable(&key),
            )
        };
        if !owned {
            // 006-02b item 1: the C-x C-q override must NEVER turn an
            // external (registry / tooling) source editable — it is a cache
            // shared by every project on the machine.
            self.minibuffer_message(
                "toggle-read-only: external buffer is read-only (not project-owned)",
            );
            return;
        }
        if mode == BufferMode::Accurate {
            if locally_modified && !baseline {
                // Unsaved edits must not be lost silently: confirm first
                // (the display name keeps the prompt inside one minibuffer
                // row at 80 columns).
                self.toggle_ro_confirm = Some(key.clone());
                self.minibuffer_message(&format!(
                    "Discard unsaved edits in {} to make it read-only? (y or n)",
                    self.buffer_display(&key)
                ));
                return;
            }
            if let Some(buf) = self.buffers.get_mut(&key) {
                buf.mode = BufferMode::Annotation;
                buf.editable = baseline;
            }
            self.minibuffer_message(if baseline {
                "annotation mode (still editable)"
            } else {
                "read-only (C-x C-q to edit)"
            });
        } else {
            if let Some(buf) = self.buffers.get_mut(&key) {
                buf.mode = BufferMode::Accurate;
                buf.editable = true;
            }
            self.minibuffer_message("accurate mode (editable, C-x C-s to save)");
        }
    }

    /// The buffer at `key`'s BASELINE editability (plan 015 issue 02): what
    /// `editable` is when the buffer sits in `Annotation` mode — `true` for
    /// the inherently-editable buffers (the notes document, scratch),
    /// `false` for plain file buffers. A predicate, not a remembered value:
    /// the baseline is what the buffer IS, so it cannot drift with session
    /// state (a save or a reload never changes it), and it is the same
    /// kind/is-notes predicate plan 015 will keep using for the annotation
    /// vs accurate behaviour split.
    pub(super) fn buffer_baseline_editable(&self, key: &str) -> bool {
        let Some(buf) = self.buffers.get(key) else {
            return false;
        };
        buf.path.is_none() || self.notes_key().as_deref() == Some(key)
    }

    /// Whether a toggle-read-only discard confirm is armed.
    pub fn toggle_ro_active(&self) -> bool {
        self.toggle_ro_confirm.is_some()
    }

    /// The confirm's state machine (plan 005 issue 01): `y` discards the
    /// unsaved edits (re-reads the on-disk content) and makes the buffer
    /// read-only; `n`, C-g, and ESC cancel and keep edit mode (the text is
    /// untouched). Every other key is swallowed (no "unbound key" echo
    /// mid-prompt, matching the quit save-prompt discipline).
    pub fn toggle_ro_key(&mut self, key: Key) {
        if key == Key::ctrl_char('g') || key.code == KeyCode::Escape {
            self.toggle_ro_cancel();
            return;
        }
        let Some(c) = key.char_value() else { return };
        match c {
            'y' => {
                self.toggle_ro_accept();
            }
            'n' => {
                self.toggle_ro_cancel();
            }
            _ => {}
        }
    }

    /// The confirm's `y`: discard the local edits (re-read the file from
    /// disk, clearing `locally_modified`/`changed_on_disk`) and turn the
    /// buffer read-only. A failed re-read keeps the confirm armed (no
    /// silent state half-change).
    fn toggle_ro_accept(&mut self) {
        let Some(key) = self.toggle_ro_confirm.clone() else {
            return;
        };
        let Some(path) = self.buffers.get(&key).and_then(|b| b.path.clone()) else {
            self.toggle_ro_confirm = None;
            return;
        };
        let (rope, mtime) = match load_file(&path) {
            Ok(x) => x,
            Err(e) => {
                self.minibuffer_message(&format!("cannot discard: {e}"));
                return;
            }
        };
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope = rope;
            buf.mtime = mtime;
            buf.locally_modified = false;
            buf.changed_on_disk = false;
            buf.editable = false;
            buf.mode = BufferMode::Annotation;
        }
        self.drop_retained_tree(&key);
        self.toggle_ro_confirm = None;
        self.ensure_highlight_for_key(&key);
        self.minibuffer_message("read-only (C-x C-q to edit)");
    }

    /// The confirm's `n` / C-g / ESC: cancel; edit mode and the text stay.
    fn toggle_ro_cancel(&mut self) {
        self.toggle_ro_confirm = None;
        self.minibuffer_message("cancel (edit mode kept)");
    }

    /// The current buffer's "point" as a byte offset: the byte of the
    /// point's exact `(line, col)` position (plan 015 issue 02: the HONEST
    /// point — the old implementation returned the point's LINE START, which
    /// made the region line-granular, `C-y` land at the line start, and the
    /// C-x C-x column work invisible). `point_col()` is a CHAR index
    /// (`file_point()`'s clamped `col`), so the conversion is line→char +
    /// col, then char→byte, via the non-panicking `try_` steps (an
    /// out-of-range line degrades to `None`, as before). The mark is a byte
    /// offset, and the region is now EXACT (char-granular) in BOTH edit
    /// modes — a fine mark is meaningful in annotation mode too (plan 015
    /// decision). Returns `None` when there is no current buffer.
    fn current_point_byte(&self) -> Option<usize> {
        let key = self.buffers.current()?.to_string();
        let buf = self.buffers.get(&key)?;
        let p = self.file_point();
        point_byte_offset(&buf.rope, p.line, p.col)
    }

    /// The region's normalized byte range [start, end) for the current buffer,
    /// or `None` when no mark is set.
    pub fn region_byte_range(&self) -> Option<(usize, usize)> {
        let key = self.buffers.current()?.to_string();
        let buf = self.buffers.get(&key)?;
        let mark = buf.mark?;
        let point = self.current_point_byte()?;
        let start = mark.min(point);
        let end = mark.max(point);
        if start == end {
            None
        } else {
            Some((start, end))
        }
    }

    /// The region size in bytes (for the status line display). `None` when
    /// no mark is set or the region is empty.
    pub fn region_size_bytes(&self) -> Option<usize> {
        self.region_byte_range()
            .map(|(s, e)| e - s)
    }

    /// The region's line range (start_line, end_line inclusive) in buffer
    /// line indices, for the file view's region face rendering. `None` when
    /// no mark is set or the region is empty.
    pub fn region_line_range(&self) -> Option<(usize, usize)> {
        let (byte_start, byte_end) = self.region_byte_range()?;
        let key = self.buffers.current()?.to_string();
        let buf = self.buffers.get(&key)?;
        let start_line = buf.rope.try_byte_to_line(byte_start).ok()?;
        // end is exclusive; the last line in the region is the line
        // containing byte_end - 1 (the last byte of the region).
        let last_byte = byte_end.saturating_sub(1);
        let end_line = buf.rope.try_byte_to_line(last_byte).ok()?;
        Some((start_line, end_line))
    }

    /// C-SPC: set the mark at the current point. Echoes "Mark set".
    pub fn set_mark(&mut self) {
        let point = match self.current_point_byte() {
            Some(p) => p,
            None => {
                self.minibuffer_message("no buffer");
                return;
            }
        };
        let key = self.buffers.current().map(String::from);
        if let Some(key) = key
            && let Some(buf) = self.buffers.get_mut(&key)
        {
            buf.mark = Some(point);
        }
        self.minibuffer_message("Mark set");
    }

    /// C-x C-x: exchange point and mark. If no mark is set, no-op.
    /// After the exchange, the cursor is at where the mark was, and the mark
    /// is at where the point was.
    pub fn exchange_point_and_mark(&mut self) {
        let key = match self.buffers.current().map(String::from) {
            Some(k) => k,
            None => {
                self.minibuffer_message("no buffer");
                return;
            }
        };
        let (mark, point) = {
            let buf = match self.buffers.get(&key) {
                Some(b) => b,
                None => {
                    self.minibuffer_message("no buffer");
                    return;
                }
            };
            (buf.mark, self.current_point_byte())
        };
        let Some(mark) = mark else {
            self.minibuffer_message("Mark not set");
            return;
        };
        let Some(point) = point else { return };
        // Move the point to where the mark was (the new point); the
        // window follows. The mark is a BYTE offset (region semantics,
        // plan 004 issue 05b), so land it via the byte→(line, char
        // column) conversion — a line-only landing drops the mark's
        // column (a raw byte column is off-by-N on multibyte lines).
        let (mark_line, mark_col) = self
            .buffers
            .get(&key)
            .and_then(|b| b.try_byte_to_line_col(mark))
            .unwrap_or((0, 0));
        self.set_point(mark_line, mark_col, mark_col);
        // Set the mark to the old point.
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.mark = Some(point);
        }
    }

    /// C-w: kill the region. In editable buffers, removes the text and saves
    /// it to the kill ring. In read-only buffers, saves the text to the kill
    /// ring without modifying the buffer (emacs read-only kill-ring-save).
    /// Clears the mark after the operation.
    pub fn kill_region(&mut self) {
        let range = match self.region_byte_range() {
            Some(r) => r,
            None => {
                self.minibuffer_message("Mark not set");
                return;
            }
        };
        let key = self.buffers.current().map(String::from).unwrap_or_default();
        let (editable, text, char_start, char_end, old_rope) = {
            let Some(buf) = self.buffers.get(&key) else {
                self.minibuffer_message("no buffer");
                return;
            };
            // Convert byte offsets to char offsets (ropey edit APIs are char-index based).
            let char_start = buf.rope.byte_to_char(range.0);
            let char_end = buf.rope.byte_to_char(range.1);
            let text = buf.rope.slice(char_start..char_end).to_string();
            (buf.editable, text, char_start, char_end, buf.rope.clone())
        };
        // Save to the kill ring (always, regardless of editable).
        self.kill_ring.push(text.clone());
        // In editable buffers, remove the text from the rope.
        if editable {
            if let Some(buf) = self.buffers.get_mut(&key) {
                buf.rope.remove(char_start..char_end);
                buf.locally_modified = true;
                buf.mark = None;
            }
            self.retain_rope_edit(&key, &old_rope, char_start, char_end, "");
            self.invalidate_highlight_for_key(&key);
            // Adjust scroll to keep the view sane after the text removal.
            let total = self
                .buffers
                .get(&key)
                .map(|b| b.line_count())
                .unwrap_or(0);
            let top = self.scroll_top();
            if total > 0 && top >= total.saturating_sub(1) {
                self.set_scroll_top(total.saturating_sub(1));
            }
        } else {
            // Read-only: clear the mark (region is consumed).
            if let Some(buf) = self.buffers.get_mut(&key) {
                buf.mark = None;
            }
        }
        // Reset yank-pop state (a new kill is not a yank).
        self.yank_pos = None;
        self.yank_len = None;
        self.yank_ring_index = None;
        self.minibuffer_message(&format!("{} bytes killed", range.1 - range.0));
    }

    /// M-w: copy the region to the kill ring (no removal; works in both
    /// editable and read-only buffers). Keeps the mark active.
    pub fn copy_region(&mut self) {
        let range = match self.region_byte_range() {
            Some(r) => r,
            None => {
                self.minibuffer_message("Mark not set");
                return;
            }
        };
        let key = self.buffers.current().map(String::from).unwrap_or_default();
        let text = {
            let Some(buf) = self.buffers.get(&key) else {
                self.minibuffer_message("no buffer");
                return;
            };
            // Convert byte offsets to char offsets (ropey slice is char-index based).
            let char_start = buf.rope.byte_to_char(range.0);
            let char_end = buf.rope.byte_to_char(range.1);
            buf.rope.slice(char_start..char_end).to_string()
        };
        self.kill_ring.push(text);
        // Reset yank-pop state.
        self.yank_pos = None;
        self.yank_len = None;
        self.yank_ring_index = None;
        self.minibuffer_message(&format!("{} bytes copied to kill ring", range.1 - range.0));
    }

    /// C-y: yank the most recent kill ring entry at the current point.
    /// Only works in editable buffers. Sets the yank-pop state for M-y.
    pub fn yank(&mut self) {
        let text = match self.kill_ring.top() {
            Some(t) => t.to_string(),
            None => {
                self.minibuffer_message("Kill ring is empty");
                return;
            }
        };
        let key = match self.buffers.current().map(String::from) {
            Some(k) => k,
            None => {
                self.minibuffer_message("no buffer");
                return;
            }
        };
        let editable = self
            .buffers
            .get(&key)
            .map(|b| b.editable)
            .unwrap_or(false);
        if !editable {
            self.minibuffer_message("Buffer is read-only");
            return;
        }
        let point_byte = match self.current_point_byte() {
            Some(p) => p,
            None => {
                self.minibuffer_message("no buffer");
                return;
            }
        };
        let point_char = {
            let buf = self.buffers.get(&key).unwrap();
            buf.rope.byte_to_char(point_byte)
        };
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.insert(point_char, &text);
            buf.locally_modified = true;
            // Clear the mark (the text insertion shifts byte offsets).
            buf.mark = None;
        }
        if let Some(old_rope) = old_rope {
            self.retain_rope_edit(&key, &old_rope, point_char, point_char, &text);
        }
        self.invalidate_highlight_for_key(&key);
        // Set the yank-pop state (char offsets for ropey edit APIs).
        self.yank_pos = Some(point_char);
        self.yank_len = Some(text.chars().count());
        self.yank_ring_index = Some(0);
        // No scroll adjustment: the insertion is at the current top line,
        // so the view is already anchored correctly (finding 4 fix).
    }

    /// M-y: yank-pop — replace the last yanked text with the previous kill
    /// ring entry. Only valid immediately after C-y or another M-y.
    pub fn yank_pop(&mut self) {
        let Some(idx) = self.yank_ring_index else {
            self.minibuffer_message("Yank-pop: no previous yank");
            return;
        };
        let next = idx + 1;
        let text = match self.kill_ring.at(next) {
            Some(t) => t.to_string(),
            None => {
                self.minibuffer_message("Yank-pop: end of kill ring");
                return;
            }
        };
        let key = match self.buffers.current().map(String::from) {
            Some(k) => k,
            None => {
                self.minibuffer_message("no buffer");
                return;
            }
        };
        let (yank_pos, yank_len) = match (self.yank_pos, self.yank_len) {
            (Some(p), Some(l)) => (p, l),
            _ => {
                self.minibuffer_message("Yank-pop: no previous yank");
                return;
            }
        };
        let editable = self
            .buffers
            .get(&key)
            .map(|b| b.editable)
            .unwrap_or(false);
        if !editable {
            self.minibuffer_message("Buffer is read-only");
            return;
        }
        let end = (yank_pos + yank_len).min(
            self.buffers
                .get(&key)
                .map(|b| b.rope.len_chars())
                .unwrap_or(0),
        );
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.remove(yank_pos..end);
            buf.rope.insert(yank_pos, &text);
            buf.locally_modified = true;
            buf.mark = None;
        }
        if let Some(old_rope) = old_rope {
            self.retain_rope_edit(&key, &old_rope, yank_pos, end, &text);
        }
        self.invalidate_highlight_for_key(&key);
        self.yank_len = Some(text.chars().count());
        self.yank_ring_index = Some(next);
        // No scroll adjustment: the replacement is at the current top line.
    }

    /// Kill the buffer with key `key`. 06a: no accidental buffer creation —
    /// when the killed buffer was current, the MRU survivor becomes current
    /// (emacs `kill-buffer` fallback); with the LAST buffer killed the main
    /// view returns to home.
    pub fn kill_buffer(&mut self, key: &str) {
        let display = self.buffer_display(key);
        if !self.buffers.kill(key) {
            self.minibuffer_message(&format!("no buffer: {display}"));
            return;
        }
        if self.buffers.current().is_none() {
            let mru = self.buffers.list().first().map(|(k, _)| k.to_string());
            if let Some(k) = mru {
                self.buffers.set_current(&k);
            }
        }
        self.normalize_top_view();
        self.minibuffer_message(&format!("killed {display}"));
    }

    /// Buffer-list view: open the selected buffer and close the list.
    pub fn open_buffer_list_selected(&mut self) {
        let key = match self.buffers.list().get(self.buffer_list_selected) {
            Some((key, _)) => key.to_string(),
            None => return,
        };
        self.buffers.set_current(&key);
        // 006-03b item 1: a switched-to external buffer keeps its owning
        // crate MRU.
        self.bump_current_crate_recency();
        self.close_view();
        self.normalize_top_view();
    }

    pub fn buffer_list_next(&mut self) {
        let n = self.buffers.len();
        if n > 0 {
            self.buffer_list_selected = (self.buffer_list_selected + 1) % n;
        }
    }

    pub fn buffer_list_prev(&mut self) {
        let n = self.buffers.len();
        if n > 0 {
            self.buffer_list_selected = (self.buffer_list_selected + n - 1) % n;
        }
    }

    /// Buffer list: `d` kills the SELECTED buffer (issue 05h; the
    /// dired-convention kill verb backlogged in parity log row 31). Reuses
    /// the existing `kill_buffer` path the `C-x k` picker runs — no second
    /// kill verb. The list stays open; the selection clamps to a valid row.
    pub fn buffer_list_kill_selected(&mut self) {
        let key = match self.buffers.list().get(self.buffer_list_selected) {
            Some((key, _)) => key.to_string(),
            None => return,
        };
        self.kill_buffer(&key);
        let n = self.buffers.len();
        self.buffer_list_selected = if n > 0 {
            self.buffer_list_selected.min(n - 1)
        } else {
            0
        };
    }
}

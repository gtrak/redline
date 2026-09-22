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

    /// Land the current buffer's point at CHAR index `char_idx` (plan 015
    /// issue 03): the point-accurate commands move the point by char, not
    /// byte, so a multibyte edit never desyncs the `(line, col)` (a raw
    /// byte offset as a column is off-by-N on a line that starts with a
    /// multibyte char). The window follows via `set_point`.
    fn land_point_at_char(&mut self, key: &str, char_idx: usize) {
        let (line, col) = self
            .buffers
            .get(key)
            .and_then(|b| {
                let byte = b.rope.try_char_to_byte(char_idx).ok()?;
                b.try_byte_to_line_col(byte)
            })
            .unwrap_or((0, 0));
        self.set_point(line, col, col);
    }

    /// `A`-less edit hygiene shared by the point-accurate commands: mark the
    /// notes document stale when the edited buffer is the notes document
    /// (plan 005 issue 02's re-parse trigger — the coarse `notes_insert_char`
    /// / `notes_backspace` set it, and the Accurate commands must too, or an
    /// Accurate edit of the notes buffer leaves the doc stale).
    fn mark_notes_dirty_if_current(&mut self, key: &str) {
        if self.notes_key().as_deref() == Some(key) {
            self.notes_buffer_dirty = true;
        }
    }

    /// Whether the current buffer is in `Accurate` mode (plan 015 issue 03,
    /// P2-b): the point-accurate commands are accurate-MODE behaviour, so a
    /// direct `M-x` / registry invocation must not run them on an
    /// Annotation-mode buffer (which stays coarse). The notes-edit key guard
    /// already routes them to Accurate mode only, so this gate only bites for
    /// the registry path; in Annotation mode an `M-x` of one of these is a
    /// no-op rather than a point-accurate edit.
    fn current_buffer_accurate(&self) -> bool {
        self.buffers
            .current()
            .and_then(|k| self.buffers.get(k).map(|b| b.mode == BufferMode::Accurate))
            .unwrap_or(false)
    }

    /// Insert `text` at the HONEST point (plan 015 issue 03, accurate-mode
    /// self-insert): lands at the point's byte (char-converted) and advances
    /// the point past the inserted text. Returns `false` when there is no
    /// current buffer or it is not editable. Requires `editable` and sets the
    /// three edit-hygiene flags exactly as `insert_text` does
    /// (`locally_modified` / `retain_rope_edit` / `invalidate_highlight_for_key`).
    /// Mark decision: CLEARED (a text insertion shifts every byte offset after
    /// it, so a stale mark would be wrong — the same call `yank`/`kill_region`
    /// make). This is the accurate counterpart to the end-of-buffer
    /// `insert_text`; the mode guard (`notes_edit_key_event`) picks between
    /// them.
    pub fn insert_text_at_point(&mut self, text: &str) -> bool {
        let Some(key) = self.buffers.current() else {
            return false;
        };
        let key = key.to_string();
        let editable = self
            .buffers
            .get(&key)
            .map(|b| b.editable)
            .unwrap_or(false);
        if !editable {
            return false;
        }
        let point_byte = match self.current_point_byte() {
            Some(b) => b,
            None => return false,
        };
        let point_char = self
            .buffers
            .get(&key)
            .map(|b| b.rope.byte_to_char(point_byte))
            .unwrap_or(0);
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.insert(point_char, text);
            buf.locally_modified = true;
            buf.mark = None;
        }
        if let Some(old_rope) = old_rope {
            self.retain_rope_edit(&key, &old_rope, point_char, point_char, text);
        }
        self.invalidate_highlight_for_key(&key);
        self.mark_notes_dirty_if_current(&key);
        // The point moves past the inserted text (char-accurate).
        let inserted = text.chars().count();
        self.land_point_at_char(&key, point_char + inserted);
        true
    }

    /// Delete the char BEFORE the point (plan 015 issue 03, accurate-mode
    /// Backspace / C-h); a no-op at the buffer start (emacs behaviour).
    /// The point moves back over the deleted char. Requires `editable` and
    /// sets the three edit-hygiene flags; the mark is cleared (the removal
    /// shifts byte offsets after it).
    pub fn delete_char_before_point(&mut self) {
        let Some(key) = self.buffers.current().map(String::from) else {
            return;
        };
        let editable = self
            .buffers
            .get(&key)
            .map(|b| b.editable)
            .unwrap_or(false);
        if !editable {
            return;
        }
        let point_byte = match self.current_point_byte() {
            Some(b) => b,
            None => return,
        };
        if point_byte == 0 {
            // Buffer start: no char before the point (emacs no-op).
            return;
        }
        let point_char = self
            .buffers
            .get(&key)
            .map(|b| b.rope.byte_to_char(point_byte))
            .unwrap_or(0);
        if point_char == 0 {
            return;
        }
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.remove(point_char - 1..point_char);
            buf.locally_modified = true;
            buf.mark = None;
        }
        if let Some(old_rope) = old_rope {
            self.retain_rope_edit(&key, &old_rope, point_char - 1, point_char, "");
        }
        self.invalidate_highlight_for_key(&key);
        self.mark_notes_dirty_if_current(&key);
        self.land_point_at_char(&key, point_char - 1);
    }

    /// Delete the char AT the point (plan 015 issue 03, `C-d`
    /// delete-char-forward, freed from half-page scroll); a no-op at the
    /// buffer end (emacs behaviour). The point stays put. Requires `editable`
    /// and sets the three edit-hygiene flags; the mark is cleared.
    pub fn delete_char_forward(&mut self) {
        // P2-b: accurate-mode command; a no-op on an Annotation-mode buffer.
        if !self.current_buffer_accurate() {
            return;
        }
        let Some(key) = self.buffers.current().map(String::from) else {
            return;
        };
        let (editable, point_byte, total_bytes) = {
            let Some(buf) = self.buffers.get(&key) else {
                return;
            };
            (buf.editable, self.current_point_byte(), buf.rope.len_bytes())
        };
        if !editable {
            return;
        }
        let point_byte = match point_byte {
            Some(b) => b,
            None => return,
        };
        if point_byte >= total_bytes {
            // Buffer end: no char at the point (emacs no-op).
            return;
        }
        let point_char = self
            .buffers
            .get(&key)
            .map(|b| b.rope.byte_to_char(point_byte))
            .unwrap_or(0);
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.remove(point_char..point_char + 1);
            buf.locally_modified = true;
            buf.mark = None;
        }
        if let Some(old_rope) = old_rope {
            self.retain_rope_edit(&key, &old_rope, point_char, point_char + 1, "");
        }
        self.invalidate_highlight_for_key(&key);
        self.mark_notes_dirty_if_current(&key);
        // Point stays at its char position.
        self.land_point_at_char(&key, point_char);
    }

    /// Newline at the point (plan 015 issue 03, `RET`): insert `"\n"` at the
    /// honest point, splitting the line there; the point lands on the new line
    /// (just after the inserted newline). This IS `insert_text_at_point("\n")`
    /// (no separate implementation — the spec says so rather than duplicate).
    pub fn newline_at_point(&mut self) {
        // P2-b: accurate-mode command — a no-op on an Annotation-mode buffer.
        if !self.current_buffer_accurate() {
            return;
        }
        self.insert_text_at_point("\n");
    }

    /// Kill the line (plan 015 issue 03, `C-k`): kill from the point to the
    /// end of the line and push it to the kill ring (so `C-y` yanks it back).
    /// End-of-line decision (STATED, matching emacs `kill-line`): at a NON-EOL
    /// point it kills through to EOL (the newline is kept); at EOL it kills the
    /// newline itself, JOINING the line with the next one — and at the buffer
    /// end (no trailing newline) it is a no-op. Requires `editable`; sets the
    /// three edit-hygiene flags, clears the mark, and RESETS the yank-pop state
    /// (a new kill is not a yank — a stale `M-y` would cycle the old entry).
    pub fn kill_line(&mut self) {
        // P2-b: accurate-mode command; a no-op on an Annotation-mode buffer
        // (the coarse model has no point-accurate kill-line).
        if !self.current_buffer_accurate() {
            return;
        }
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("kill-line: no buffer");
            return;
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
        let (line_start_char, line_len_chars, point_byte, total_chars) = {
            let Some(buf) = self.buffers.get(&key) else {
                self.minibuffer_message("kill-line: no buffer");
                return;
            };
            let line = self.file_point().line;
            let line_start_char = buf.rope.line_to_char(line);
            let line_len_chars = buf
                .line_text(line)
                .map(|t| t.chars().count())
                .unwrap_or(0);
            let point_byte = self.current_point_byte().unwrap_or(0);
            (line_start_char, line_len_chars, point_byte, buf.rope.len_chars())
        };
        let eol_char = line_start_char + line_len_chars;
        let point_char = self
            .buffers
            .get(&key)
            .map(|b| b.rope.byte_to_char(point_byte))
            .unwrap_or(eol_char);
        // The kill region in char indices: [point_char, end_of_kill).
        let end_of_kill = if point_char < eol_char {
            eol_char // to EOL, keep the newline
        } else if eol_char < total_chars {
            eol_char + 1 // at EOL: kill the newline (join the lines)
        } else {
            // At the buffer end (EOL, no trailing newline): nothing to kill.
            self.minibuffer_message("kill-line: nothing to kill");
            return;
        };
        if point_char >= end_of_kill {
            return;
        }
        let killed = self
            .buffers
            .get(&key)
            .map(|b| b.rope.slice(point_char..end_of_kill).to_string())
            .unwrap_or_default();
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        self.kill_ring.push(killed.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.remove(point_char..end_of_kill);
            buf.locally_modified = true;
            buf.mark = None;
        }
        if let Some(old_rope) = old_rope {
            self.retain_rope_edit(&key, &old_rope, point_char, end_of_kill, "");
        }
        self.invalidate_highlight_for_key(&key);
        self.mark_notes_dirty_if_current(&key);
        // Point stays at the start of the killed region (now at EOL, or at
        // the join after the newline was killed).
        self.land_point_at_char(&key, point_char);
        // Reset yank-pop state (a new kill is not a yank).
        self.yank_pos = None;
        self.yank_len = None;
        self.yank_ring_index = None;
        self.minibuffer_message(&format!("{} chars killed", end_of_kill - point_char));
    }

    /// Kill the word backward (plan 015 issue 03, `M-DEL`): kill from the
    /// point backward to the previous word boundary and push it to the kill
    /// ring. The kill matches emacs `backward-kill-word`: skip the whitespace
    /// immediately before the point, then the word before it (a newline is
    /// non-word, so a run of blank lines before a word is killed too). The
    /// point moves to the START of the killed text (emacs `backward-kill-word`
    /// leaves point at the kill start — landing at the pre-kill point char
    /// would point into the text that followed the kill). A no-op (no kill, no
    /// ring push) when the point is at the buffer start or there is no
    /// word/whitespace before it. Requires `editable` and `Accurate` mode; sets
    /// the three edit-hygiene flags, clears the mark, and RESETS the yank-pop
    /// state (as `kill_line` does).
    pub fn kill_word_backward(&mut self) {
        // P2-b: accurate-mode command; a no-op on an Annotation-mode buffer.
        if !self.current_buffer_accurate() {
            return;
        }
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("kill-word-backward: no buffer");
            return;
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
            Some(b) => b,
            None => return,
        };
        let point_char = self
            .buffers
            .get(&key)
            .map(|b| b.rope.byte_to_char(point_byte))
            .unwrap_or(0);
        // Char-level walk (is_word_char is the crate-wide word rule). First
        // the whitespace/punctuation immediately before the point, then the
        // word before it.
        let chars: Vec<char> = {
            let buf = self.buffers.get(&key).unwrap();
            buf.rope.slice(0..point_char).chars().collect()
        };
        let mut i = point_char;
        // Skip non-word chars backward.
        while i > 0 && !is_word_char(chars[i - 1]) {
            i -= 1;
        }
        // Skip the word backward.
        while i > 0 && is_word_char(chars[i - 1]) {
            i -= 1;
        }
        if i == point_char {
            // Nothing before the point to kill (buffer start / all-word with
            // the point at its very start): a no-op.
            return;
        }
        let killed = self
            .buffers
            .get(&key)
            .map(|b| b.rope.slice(i..point_char).to_string())
            .unwrap_or_default();
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        self.kill_ring.push(killed.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.remove(i..point_char);
            buf.locally_modified = true;
            buf.mark = None;
        }
        if let Some(old_rope) = old_rope {
            self.retain_rope_edit(&key, &old_rope, i, point_char, "");
        }
        self.invalidate_highlight_for_key(&key);
        self.mark_notes_dirty_if_current(&key);
        // Point lands at the START of the killed text (the kill is backward,
        // so the pre-kill point char now sits inside the text that survived).
        self.land_point_at_char(&key, i);
        self.yank_pos = None;
        self.yank_len = None;
        self.yank_ring_index = None;
        self.minibuffer_message(&format!("{} chars killed", point_char - i));
    }

    /// Kill the word forward (plan 015 issue 03, `M-d`): kill from the point
    /// forward and push it to the kill ring. The kill extent matches emacs
    /// `kill-word` (`M-d` = `kill-word 1`), which kills the region the point
    /// moves over with `forward-word`: if the char AT the point is a word
    /// char, only that word is killed (the trailing non-word is NOT consumed);
    /// if the point is on a non-word char, the non-word run and then the next
    /// word are killed (a newline is non-word, so a run of blank lines between
    /// the point and the next word is killed too). The point stays at the kill
    /// start (the text after the killed region moves up to it). A no-op (no
    /// kill, no ring push) when the point is at the buffer end. Requires
    /// `editable` and `Accurate` mode; sets the three edit-hygiene flags,
    /// clears the mark, and RESETS the yank-pop state (as
    /// `kill_word_backward` does). Emacs `M-d` is the FORWARD kill; `M-DEL`
    /// stays `kill_word_backward`.
    pub fn kill_word_forward(&mut self) {
        // P2-b: accurate-mode command; a no-op on an Annotation-mode buffer.
        if !self.current_buffer_accurate() {
            return;
        }
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("kill-word-forward: no buffer");
            return;
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
            Some(b) => b,
            None => return,
        };
        let (point_char, total_chars) = {
            let buf = self.buffers.get(&key).unwrap();
            (buf.rope.byte_to_char(point_byte), buf.rope.len_chars())
        };
        if point_char >= total_chars {
            // Buffer end: nothing to kill forward.
            return;
        }
        // Char-level walk (is_word_char is the crate-wide word rule),
        // matching emacs `forward-word` as used by `kill-word 1`: word char
        // at the point → skip that word only (the trailing non-word is NOT
        // consumed); non-word at the point → skip the non-word run, then the
        // next word. `j` is always >= 1 here (point_char < total_chars, so
        // the remainder is non-empty and either opens a word run or a
        // non-word run). The buffer-end no-op is handled by the
        // `point_char >= total_chars` guard above.
        let chars: Vec<char> = {
            let buf = self.buffers.get(&key).unwrap();
            buf.rope.slice(point_char..total_chars).chars().collect()
        };
        let mut j = 0;
        if is_word_char(chars[0]) {
            while j < chars.len() && is_word_char(chars[j]) {
                j += 1;
            }
        } else {
            while j < chars.len() && !is_word_char(chars[j]) {
                j += 1;
            }
            while j < chars.len() && is_word_char(chars[j]) {
                j += 1;
            }
        }
        let end_char = point_char + j;
        let killed = self
            .buffers
            .get(&key)
            .map(|b| b.rope.slice(point_char..end_char).to_string())
            .unwrap_or_default();
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        self.kill_ring.push(killed.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.remove(point_char..end_char);
            buf.locally_modified = true;
            buf.mark = None;
        }
        if let Some(old_rope) = old_rope {
            self.retain_rope_edit(&key, &old_rope, point_char, end_char, "");
        }
        self.invalidate_highlight_for_key(&key);
        self.mark_notes_dirty_if_current(&key);
        // Point stays at the kill start (the kill is forward, so the text after
        // the killed word moves up to the point).
        self.land_point_at_char(&key, point_char);
        self.yank_pos = None;
        self.yank_len = None;
        self.yank_ring_index = None;
        self.minibuffer_message(&format!("{} chars killed", j));
    }

    /// Open a line (plan 015 issue 03, `C-o`): insert a newline JUST BEFORE
    /// the point, so the text from the point on drops to a new line; the
    /// point stays at the end of the (now upper) line. Requires `editable` and
    /// sets the three edit-hygiene flags; the mark is cleared.
    pub fn open_line(&mut self) {
        // P2-b: accurate-mode command; a no-op on an Annotation-mode buffer.
        if !self.current_buffer_accurate() {
            return;
        }
        let Some(key) = self.buffers.current().map(String::from) else {
            return;
        };
        let editable = self
            .buffers
            .get(&key)
            .map(|b| b.editable)
            .unwrap_or(false);
        if !editable {
            return;
        }
        let point_byte = match self.current_point_byte() {
            Some(b) => b,
            None => return,
        };
        let point_char = self
            .buffers
            .get(&key)
            .map(|b| b.rope.byte_to_char(point_byte))
            .unwrap_or(0);
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.insert(point_char, "\n");
            buf.locally_modified = true;
            buf.mark = None;
        }
        if let Some(old_rope) = old_rope {
            self.retain_rope_edit(&key, &old_rope, point_char, point_char, "\n");
        }
        self.invalidate_highlight_for_key(&key);
        self.mark_notes_dirty_if_current(&key);
        // The point stays at its char position — now the end of the upper
        // line (just before the inserted newline).
        self.land_point_at_char(&key, point_char);
    }

    /// Transpose the two chars around the point (plan 015 issue 03, `C-t`,
    /// emacs `transpose-chars`): mid-line, swap the char BEFORE the point
    /// with the char AT the point and move the point forward one, past both
    /// swapped chars. Swapping whole characters (not bytes) makes the
    /// multibyte case correct. The line-edge cases emacs folds in: at a line
    /// end (char at point is `\n`) the PREVIOUS TWO chars are exchanged and
    /// the point does NOT move; at a line start (char before is `\n`) the
    /// first char of the line moves to the end of the previous one and the
    /// point moves forward one. At the buffer END (point past the last char)
    /// emacs transposes the LAST TWO chars and leaves the point at the end
    /// (one past the between-position). A no-op at the buffer start (no char
    /// before), when the EOL case has fewer than two chars before the point
    /// in the buffer (emacs's guard is buffer-position based, so it crosses
    /// line boundaries), and at a buffer end on a single-char buffer
    /// (`total_chars < 2`): emacs signals `(beginning-of-buffer)` in all
    /// three cases; redline no-ops rather than erroring.
    /// Requires `editable` and `Accurate` mode; sets the three edit-hygiene
    /// flags, clears the mark.
    pub fn transpose_chars(&mut self) {
        // P2-b: accurate-mode command; a no-op on an Annotation-mode buffer.
        if !self.current_buffer_accurate() {
            return;
        }
        let Some(key) = self.buffers.current().map(String::from) else {
            return;
        };
        let (editable, point_byte, total_chars) = {
            let Some(buf) = self.buffers.get(&key) else {
                return;
            };
            (buf.editable, self.current_point_byte(), buf.rope.len_chars())
        };
        if !editable {
            return;
        }
        let point_byte = match point_byte {
            Some(b) => b,
            None => return,
        };
        let point_char = self
            .buffers
            .get(&key)
            .map(|b| b.rope.byte_to_char(point_byte))
            .unwrap_or(0);
        if point_char == 0 {
            // Buffer start: no char before — a no-op.
            return;
        }
        // Which two chars to swap, and where the point lands.
        // - Buffer END (point past the last char): emacs transposes the LAST
        //   TWO chars and leaves the point at the end (one past the
        //   between-position — "moves forward one").
        // - EOL (char at point is \n): emacs exchanges the PREVIOUS TWO
        //   chars; the point does not move. Needs two chars before the
        //   point in the buffer (buffer-position based, so the guard crosses
        //   line boundaries; point_char >= 2); with one, emacs signals an
        //   error — a no-op here.
        // - Mid-line (including a line start, where the char before is \n):
        //   swap the char before and at the point; the point moves forward
        //   one, past both swapped chars.
        let (first_idx, second_idx, land_char) = if point_char >= total_chars {
            if total_chars < 2 {
                return; // fewer than two chars: nothing to transpose
            }
            (total_chars - 2, total_chars - 1, total_chars)
        } else {
            // The char at the point (point_char < total_chars, so in range).
            let char_at_point: Option<char> = self
                .buffers
                .get(&key)
                .and_then(|b| b.rope.slice(point_char..point_char + 1).chars().next());
            if char_at_point == Some('\n') {
                if point_char < 2 {
                    return; // not two chars before in the buffer: nothing to swap
                }
                (point_char - 2, point_char - 1, point_char)
            } else {
                (point_char - 1, point_char, point_char + 1)
            }
        };
        let (c1, c2) = {
            let buf = self.buffers.get(&key).unwrap();
            (
                buf.rope.slice(first_idx..first_idx + 1).to_string(),
                buf.rope.slice(second_idx..second_idx + 1).to_string(),
            )
        };
        let swapped = format!("{c2}{c1}");
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.remove(first_idx..second_idx + 1);
            buf.rope.insert(first_idx, &swapped);
            buf.locally_modified = true;
            buf.mark = None;
        }
        if let Some(old_rope) = old_rope {
            self.retain_rope_edit(&key, &old_rope, first_idx, second_idx + 1, &swapped);
        }
        self.invalidate_highlight_for_key(&key);
        self.mark_notes_dirty_if_current(&key);
        // Mid-line / line start: past both swapped chars (one past the old
        // point). EOL / buffer end: where the point already was.
        self.land_point_at_char(&key, land_char);
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
    ///
    /// Plan 016 issue 01: this is also the ONE place the INVERSE of each
    /// text edit is recorded on the buffer's undo stack. Every text edit
    /// (self-insert, backspace, RET, C-k, C-y/M-y, C-w, …) already flows
    /// through here, so recording beside it covers them all with no
    /// per-command bookkeeping — and 02's "retrofit" of the wider edit
    /// commands needs no change to this recording logic, because the
    /// inverse is derived purely from `(old_rope, char_start, char_end,
    /// new_text)`. The inverse is NOT recorded while an undo itself is
    /// re-applying its inverse (`undo_in_progress`): that would make undo
    /// flip-flop. Issue 04 routes the undo's inverse to a redo stack; this
    /// guard is that seam.
    pub(super) fn retain_rope_edit(
        &mut self,
        key: &str,
        old_rope: &Rope,
        char_start: usize,
        char_end: usize,
        new_text: &str,
    ) {
        // Record the inverse on the buffer's undo stack — beside the
        // incremental-reparse record, in this single hook. Char indices
        // throughout (matching the ropey edit units; never bytes).
        if !self.undo_in_progress && (char_start != char_end || !new_text.is_empty()) {
            let len = old_rope.len_chars();
            // The original text the forward edit replaced (empty for a
            // pure insertion). Clamped so an out-of-range record can never
            // panic the slice (ropey slice panics on an over-long range).
            let old_text = old_rope.slice(char_start.min(len)..char_end.min(len)).to_string();
            let step = UndoStep {
                // The forward edit's inserted text now lives at
                // [char_start, char_start + len(new_text)) in the post-edit
                // rope: undo removes `new_text` there and reinserts
                // `old_text`.
                range: char_start..char_start + new_text.chars().count(),
                removed: new_text.to_string(),
                inserted: old_text,
            };
            if let Some(buf) = self.buffers.get_mut(key) {
                buf.undo.push(step);
            }
        }
        let Some(buf) = self.buffers.get(key) else { return };
        if buf.is_big() || buf.path.is_none() {
            return;
        }
        let edit =
            highlight::rope_edit_to_input_edit(old_rope, char_start, char_end, new_text);
        self.highlight_cache
            .retain_apply_edit(&TreeKey::new(key, buf.mtime), &edit);
    }

    /// Undo the current buffer's most recent text edit (plan 016 issue 01):
    /// pop the top inverse edit and re-apply it through the SAME path as an
    /// edit — `retain_rope_edit` + `invalidate_highlight_for_key` (+
    /// `locally_modified`) — so the retained parse tree never drifts from
    /// the rope (the exact class `retain_rope_edit` exists to prevent).
    ///
    /// Only in an EDITABLE buffer: a read-only buffer is a no-op with a
    /// message, and undo must NOT resurrect the mode or `editable` state
    /// (the `Accurate` ⟹ `editable` invariant stays untouched). The commit
    /// editor has its own text model and is out of scope. Without the
    /// 04 coalescing rule every keystroke is one undo step — expected at
    /// this stage, NOT a bug. Point lands on the undone edit.
    ///
    /// Stale-step guard (gate P1): before applying, the inverse is validated
    /// against the CURRENT rope. A step is stale when the buffer's rope was
    /// replaced behind the history by a content replacement that does not
    /// flow through the edit path (the notes sync `replace_buffer_text`, a
    /// disk reload `reload_in_place`, or a read-only accept
    /// `toggle_ro_accept`) — 03 clears the history at those three sites via
    /// `drop_undo_history`, but until it does, a stale range would make
    /// ropey's `remove` panic. A stale step is dropped (it is already popped)
    /// and reported, never applied.
    pub fn undo(&mut self) {
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("nothing to undo");
            return;
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
        let Some(step) = self.buffers.get_mut(&key).and_then(|b| b.undo.pop()) else {
            self.minibuffer_message("nothing to undo");
            return;
        };
        let (start, end) = (step.range.start, step.range.end);
        // Validate the inverse against the CURRENT rope before applying
        // (gate P1): the rope may have been REPLACED behind this history by a
        // content replacement that does NOT flow through the edit path — the
        // notes sync (`replace_buffer_text`), a disk reload
        // (`reload_in_place`), or a read-only accept (`toggle_ro_accept`). 03
        // clears the history at those three sites (via `drop_undo_history`);
        // until it does, a stale step's range can no longer fit the rope and
        // an unclamped `remove` would PANIC in ropey (`Char range out of
        // bounds`). The record is trustworthy only while the range still fits
        // AND the text it expects to remove still sits there — the
        // `removed`-text check also makes `UndoStep.removed` a live reader
        // rather than dead weight (gate P2-1). Otherwise the step is already
        // popped, so report it and leave the rope alone.
        let valid = self
            .buffers
            .get(&key)
            .map(|b| {
                let len = b.rope.len_chars();
                start <= end && end <= len && b.rope.slice(start..end).chars().eq(step.removed.chars())
            })
            .unwrap_or(false);
        if !valid {
            self.minibuffer_message("undo history is stale — ignored");
            return;
        }
        // Re-apply the inverse through the edit path. The guard suppresses
        // recording this re-application as a NEW undo step (the undo's
        // inverse belongs to a redo stack, issue 04).
        self.undo_in_progress = true;
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.remove(start..end);
            buf.rope.insert(start, &step.inserted);
            buf.locally_modified = true;
            buf.mark = None;
        }
        if let Some(old_rope) = old_rope {
            self.retain_rope_edit(&key, &old_rope, start, end, &step.inserted);
        }
        self.undo_in_progress = false;
        self.invalidate_highlight_for_key(&key);
        self.mark_notes_dirty_if_current(&key);
        // The point lands on the undone edit (char-accurate).
        self.land_point_at_char(&key, start);
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

    /// Replace a buffer's text (the notes-buffer sync path, from
    /// `sync_notes_from_doc`). ONE OF THREE rope-replacing sites: this
    /// assigns `buf.rope` directly, so it does NOT flow through the edit
    /// path and the recorded inverse ranges become invalid. 03 clears the
    /// undo history here (and at the other two) by calling
    /// `drop_undo_history`; until then `undo()` validates each step and
    /// rejects a stale one (so a pre-03 replacement degrades to a message,
    /// never a panic). See that helper for the full list of the three sites.
    pub(super) fn replace_buffer_text(&mut self, key: &str, text: &str) {
        let rope = Rope::from_str(text);
        if let Some(buf) = self.buffers.get_mut(key) {
            buf.rope = rope;
        }
        self.drop_retained_tree(key);
    }

    /// Clear a buffer's undo history (plan 016 issue 03 seam — 03 wires this
    /// in, not 01). Every place that REPLACES `buf.rope` outright — NOT
    /// through the edit path, so the recorded inverse char ranges become
    /// invalid — must call this so a later undo cannot re-apply a stale range
    /// (a stale range would otherwise make ropey's `remove` panic; see the
    /// stale-step guard in `undo`). 03 hooks this ONE function rather than
    /// discovering the three sites itself. The three rope-assigning sites are:
    ///
    ///   1. `reload_in_place` (`index_wiring.rs`) — a disk reload re-reads the
    ///      file into the rope (four in-place reload routes funnel here).
    ///   2. `toggle_ro_accept` (`buffers.rs`) — the read-only accept discards
    ///      local edits and re-reads the file.
    ///   3. `replace_buffer_text` (`buffers.rs`) — the notes-buffer sync
    ///      (`sync_notes_from_doc`) rewrites the notes buffer from the doc.
    ///
    /// Until 03 calls it from each site, `undo()`'s stale-step guard is the
    /// safety net: a stale step is dropped and reported, never applied.
    #[allow(dead_code)] // 03 calls this from the three sites above; 01 only adds the guard
    pub(super) fn drop_undo_history(&mut self, key: &str) {
        if let Some(buf) = self.buffers.get_mut(key) {
            buf.undo = UndoStack::default();
        }
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
    /// `false` for plain file buffers.
    ///
    /// The test is BUFFER-relative, not root-relative (plan 015 issue 03,
    /// folded in from the 02 gate): the notes document's baseline rides on
    /// the buffer's own `is_notes` flag (set on every route that opens the
    /// notes file — `open_notes` AND find-file's `open_path`), not on
    /// `notes_key()` (which is computed from the CURRENT project root). That
    /// is what the 02 gate drifted on by execution — open the notes buffer,
    /// enter Accurate, then `switch_project_root`: the old notes buffer
    /// survives in the table, but a root-relative `notes_key()` no longer
    /// matches its key, so leaving Accurate on it would flip it read-only.
    /// With `is_notes` the baseline is what the buffer IS, so it cannot
    /// drift with a project-root switch, a save, or a reload.
    pub(super) fn buffer_baseline_editable(&self, key: &str) -> bool {
        let Some(buf) = self.buffers.get(key) else {
            return false;
        };
        buf.path.is_none() || buf.is_notes
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
    ///
    /// Plan 016 (undo) seam — issue 03 owns "a disk reload clears the undo
    /// history" (the recorded char offsets become invalid). This is ONE OF
    /// THREE rope-replacing sites: it assigns `buf.rope` directly, so it does
    /// NOT flow through the `reload_in_place` chokepoint (`index_wiring.rs`).
    /// 03 clears the history at all three by calling `drop_undo_history`
    /// (this is site 2 of 3; the others are `reload_in_place` and
    /// `replace_buffer_text`). Until then `undo()`'s stale-step guard is the
    /// safety net (a stale step is dropped and reported, never a panic).
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
    ///
    /// plan 016 issue 02: a M-y is a REPLACEMENT, and it COALESCES with the
    /// preceding C-y (or an earlier M-y) into ONE undo step — one undo removes
    /// the whole yank-and-rotate sequence, restoring the pre-yank buffer
    /// (matching emacs). See the coalesce block below for the exact rule; the
    /// scoped, yank-sequence-only decision means an intervening edit breaks the
    /// run and M-y becomes its own step.
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
        // plan 016 issue 02 (STATED DECISION, scoped to the yank sequence
        // only — the general self-insert-run rule is issue 04): M-y is a
        // REPLACEMENT, and it coalesces with the preceding C-y / M-y into one
        // undo step (one undo removes the whole yank-and-rotate sequence,
        // matching emacs). `retain_rope_edit` just recorded the M-y step; it
        // recorded IFF `end != yank_pos || !text.is_empty()` (the hook's no-op
        // guard) and `undo_in_progress` is false here, so `recorded` mirrors
        // whether a step exists to coalesce. See the helper for the exact rule.
        let recorded = end != yank_pos || !text.is_empty();
        if recorded {
            self.coalesce_yank_pop_with_preceding_yank(&key, yank_pos);
        }
        self.invalidate_highlight_for_key(&key);
        self.yank_len = Some(text.chars().count());
        self.yank_ring_index = Some(next);
        // No scroll adjustment: the replacement is at the current top line.
    }

    /// plan 016 issue 02 (STATED DECISION, scoped to the yank sequence only —
    /// the general self-insert-run rule is issue 04): coalesce a M-y
    /// (yank-pop) inverse with the preceding yank (a C-y, or an earlier M-y
    /// that already coalesced) into ONE undo step. M-y is a REPLACEMENT of the
    /// just-yanked text, and emacs effectively removes the whole yank-and-
    /// rotate sequence with ONE undo, so the single combined step removes the
    /// current (rotated) text and restores the EMPTY pre-yank origin rather
    /// than leaving a step that reinserts the previous kill.
    ///
    /// `retain_rope_edit` has just recorded the M-y step {range: [yank_pos,
    /// yank_pos+|text|), removed: text, inserted: the just-replaced text}. It
    /// is the rotation's anchor IFF the step beneath it is the pure yank
    /// insertion this M-y replaces: empty `inserted` (a pure insertion), at
    /// `yank_pos`, and its recorded `removed` equals the text M-y just swapped
    /// in (`step.inserted`). Chained M-y's keep the coalesced anchor's
    /// `inserted` empty, so the rule holds across a whole rotate run. Any
    /// intervening edit lands its own step on top (with a non-empty `inserted`
    /// or a different `removed`) and prevents coalescing — exactly as emacs's
    /// sequence-bound undo would.
    fn coalesce_yank_pop_with_preceding_yank(&mut self, key: &str, yank_pos: usize) {
        let Some(buf) = self.buffers.get_mut(key) else {
            return;
        };
        // The M-y step `retain_rope_edit` just recorded (the caller guarantees
        // it recorded: `undo_in_progress` is false and the edit is non-empty
        // or non-zero-width).
        let Some(mut step) = buf.undo.pop() else {
            return;
        };
        // No step beneath: the C-y was not recorded — keep the M-y as its own.
        let Some(prev) = buf.undo.pop() else {
            buf.undo.push(step);
            return;
        };
        if step.inserted == prev.removed
            && prev.inserted.is_empty()
            && prev.range.start == yank_pos
        {
            // Coalesce: one undo removes the rotated text and restores the
            // pre-yank (empty) origin.
            step.inserted.clear();
            buf.undo.push(step);
        } else {
            // Not a yank rotation: restore both, M-y on top.
            buf.undo.push(prev);
            buf.undo.push(step);
        }
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

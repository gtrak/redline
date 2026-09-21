use super::*;

impl AppStore {
    /// The scroll position (top line) for the current buffer.
    pub fn scroll_top(&self) -> usize {
        self.buffers
            .current()
            .and_then(|key| self.scroll.get(key))
            .copied()
            .unwrap_or(0)
    }

    /// Set the scroll position for the current buffer.
    pub(super) fn set_scroll_top(&mut self, top: usize) {
        let key = self.buffers.current().map(String::from);
        if let Some(key) = key {
            let total = self
                .buffers
                .get(&key)
                .map(|b| b.line_count())
                .unwrap_or(0);
            self.scroll.insert(key, top.min(total.saturating_sub(1)));
        }
    }

    /// Scroll down by one line (window motion; the point's screen row is
    /// kept fixed — plan 004 issue 05b). Bound to `j` in the file view.
    pub fn scroll_line_down(&mut self) {
        self.scroll_window_point(1);
    }

    /// Scroll up by one line (window motion; the point's screen row is kept
    /// fixed). Bound to `k` in the file view.
    pub fn scroll_line_up(&mut self) {
        self.scroll_window_point(-1);
    }

    /// Scroll down by one page. PART A fix (item 5): keep a 2-line overlap
    /// (emacs `next-screen-context-lines`) so context carries over between
    /// pages.
    pub fn scroll_page_down(&mut self) {
        let step = self.viewport_lines.saturating_sub(2).max(1) as i64;
        self.scroll_window_point(step);
    }

    /// Scroll up by one page (2-line overlap, matching page-down).
    pub fn scroll_page_up(&mut self) {
        let step = self.viewport_lines.saturating_sub(2).max(1) as i64;
        self.scroll_window_point(-step);
    }

    /// Scroll down by half a page.
    pub fn scroll_half_page_down(&mut self) {
        self.scroll_window_point((self.viewport_lines / 2) as i64);
    }

    /// Scroll up by half a page.
    pub fn scroll_half_page_up(&mut self) {
        self.scroll_window_point(-((self.viewport_lines / 2) as i64));
    }

    /// Mouse wheel up (plan 004 issue 05c): in the file view this is a
    /// WINDOW scroll of 3 lines with the point's screen row pinned (emacs
    /// `mwheel-scroll` — the same primitive C-v/M-v use after issue 05b).
    /// The point's buffer line advances only because the window moves under
    /// it; the wheel never drags the point's line (the pre-05b
    /// window-scroll behavior). List views keep their cursor model: the
    /// selection moves by 3 rows.
    pub fn mouse_scroll_up(&mut self) {
        const STEP: usize = 3;
        match self.top_view() {
            ViewId::Buffer => {
                self.scroll_window_point(-(STEP as i64));
            }
            ViewId::BufferList => {
                for _ in 0..STEP { self.buffer_list_prev(); }
            }
            ViewId::MagitStatus => {
                for _ in 0..STEP { self.magit_cursor_up(); }
            }
            ViewId::Search => {
                for _ in 0..STEP { self.search_prev(); }
            }
            ViewId::Log => {
                for _ in 0..STEP { self.log_move_up(); }
            }
            _ => {}
        }
    }

    /// Mouse wheel down: the mirror of `mouse_scroll_up` — a 3-line window
    /// scroll with the point's screen row pinned in the file view; the
    /// selection moves by 3 rows in list views.
    pub fn mouse_scroll_down(&mut self) {
        const STEP: usize = 3;
        match self.top_view() {
            ViewId::Buffer => {
                self.scroll_window_point(STEP as i64);
            }
            ViewId::BufferList => {
                for _ in 0..STEP { self.buffer_list_next(); }
            }
            ViewId::MagitStatus => {
                for _ in 0..STEP { self.magit_cursor_down(); }
            }
            ViewId::Search => {
                for _ in 0..STEP { self.search_next(); }
            }
            ViewId::Log => {
                for _ in 0..STEP { self.log_move_down(); }
            }
            _ => {}
        }
    }

    /// Click-to-position in the file view (plan 004 issue 05c): set the
    /// point to the clicked `(line, col)` — both are clamped to the buffer's
    /// bounds, so a click past EOL lands at EOL and a click on an empty line
    /// lands at col 0. `row` is the click's row within the visible file area
    /// (the caller subtracts the file view's title-line offset); `col` is
    /// the clicked DISPLAY column RELATIVE TO THE FILE VIEW'S LEFT EDGE —
    /// with the tree sidebar visible the root event arm subtracts the tree
    /// width before calling (plan 004 issue 05e), so callers never mix the
    /// two. Wide (CJK) chars occupy 2 cells, so the display column is
    /// converted to a char index for the point (a click inside a wide char
    /// maps to that char; plan 004 issue 05d). The existing goal column is
    /// preserved (a mouse set-point does not touch it, emacs model); the
    /// window follows (05b behavior). No-op outside the file view.
    pub fn mouse_click_position(&mut self, row: usize, col: usize) {
        if self.top_view() != ViewId::Buffer {
            return;
        }
        // Map-aware (plan 005 issue 02): with note rows visible a rendered
        // row is NOT `scroll_top + row` buffer lines — translate through
        // the rendered-row list (a note row maps to its anchored code
        // row). A click past the last rendered row maps to the last CODE
        // row in the slice.
        let rows = self.file_view_rows();
        let Some(target_line) = FileViewRow::line_for_row(&rows, row)
            .or_else(|| rows.iter().rev().find(|r| !r.is_note).map(|r| r.line))
        else {
            return;
        };
        // plan 005 issue 02b: annotated lines render their code at cell 1
        // (the 1-cell gutter for the \u{258e} marker). A click in the gutter
        // (col 0) maps to char 0; a click at code cell k maps to char
        // display_col_to_char_index(text, k - 1).
        let annotated = rows
            .iter()
            .any(|r| !r.is_note && r.line == target_line && r.annotated);
        let code_col = if annotated { col.saturating_sub(1) } else { col };
        let line_len = self.line_char_len(target_line);
        // Display column -> char index (plan 004 issue 05d).
        let char_col = self
            .buffers
            .current_buffer()
            .and_then(|b| b.line_text(target_line))
            .map(|t| crate::model::text_width::display_col_to_char_index(&t, code_col))
            .unwrap_or(0)
            .min(line_len);
        let p = self.file_point();
        self.set_point(target_line, char_col, p.goal_col);
    }

    /// Scroll to the top (line 0).
    pub fn scroll_to_top(&mut self) {
        self.set_scroll_top(0);
    }

    /// Scroll to the bottom (last visible line).
    pub fn scroll_to_bottom(&mut self) {
        let total = self
            .buffers
            .current_buffer()
            .map(|b| b.line_count())
            .unwrap_or(0);
        let top = total.saturating_sub(self.viewport_lines);
        self.set_scroll_top(top);
    }

    /// `C-l` (plan 004 issue 05c): emacs `recenter-top-bottom`. The point
    /// does NOT move; the window repositions so the point's screen row
    /// cycles through the positions. `recenter-positions` defaults to
    /// `(middle top bottom)`, and `recenter-top-bottom` advances the
    /// position only when the immediately-preceding command was also
    /// recenter (tracked by `recenter_cycle`, reset on any other command
    /// in `dispatch` — emacs `recenter-last-op`). So a fresh C-l goes to
    /// MIDDLE; consecutive C-l cycles middle → top → bottom → middle.
    ///
    /// The chosen screen row is `viewport/2` (middle), `0` (top), or
    /// `viewport-1` (bottom); `scroll_top = point_line - desired_row`
    /// clamped to `[0, max_scroll]`. When the buffer barely scrolls the
    /// clamps pin the point where it is and the cycle collapses to the
    /// reachable positions (emacs recenter on a barely-scrolling buffer
    /// behaves the same way). No zone-derived guess: the position is
    /// purely the cycle index, so tiny viewports still cycle without
    /// dead-ends (middle/top/bottom naturally repeat when rows collide).
    pub fn recenter(&mut self) {
        let p = self.file_point();
        let total = self.current_line_count();
        let vp = self.viewport_lines.max(1);
        // Cycle order `(middle top bottom)` selected by the cycle index.
        let desired_row = match self.recenter_cycle % 3 {
            0 => vp / 2,
            1 => 0,
            _ => vp - 1,
        };
        if let Some(new_top) = recenter_top_for(p.line, desired_row, total, vp) {
            self.recenter_cycle += 1;
            self.set_scroll_top(new_top);
        }
    }

    /// Jump-landing recenter (plan 004 issue 07): vanilla emacs
    /// `xref-after-jump-hook` is `(recenter xref-pulse-momentarily)` —
    /// every M-. jump (and the picker/imenu selections that record the
    /// same jump) repositions the window so the landed line sits on the
    /// MIDDLE row, instead of the minimal `keep_cursor_visible` scroll
    /// that lands a below-window target on the BOTTOM row. Same
    /// arithmetic as `recenter` (`recenter_top_for`), but a jump is not a
    /// `C-l`: `recenter_cycle` is untouched, so a jump between two C-ls
    /// neither resets nor advances the middle→top→bottom cycle. No-op
    /// when the buffer does not scroll.
    pub fn recenter_landing(&mut self) {
        let line = self.point_line();
        let total = self.current_line_count();
        let vp = self.viewport_lines.max(1);
        if let Some(new_top) = recenter_top_for(line, vp / 2, total, vp) {
            self.set_scroll_top(new_top);
        }
    }

    /// Compact position display for the status line (plan 004 row 11):
    /// `Top` at the first line, `Bot` at the last, otherwise
    /// `L{n},{pct}%` where `n` is the 1-based line number and `pct` is
    /// the integer percentage through the buffer. Tracks the point's line
    /// (plan 004 issue 05b), not the window top.
    pub fn file_view_position_display(&self) -> String {
        let total = self
            .buffers
            .current_buffer()
            .map(|b| b.line_count())
            .unwrap_or(0);
        if total == 0 {
            return String::new();
        }
        let line = self.point_line(); // 0-based
        if line == 0 {
            return "Top".to_string();
        }
        if line + self.viewport_lines >= total {
            return "Bot".to_string();
        }
        let pct = (line * 100 + total / 2) / total.max(1);
        format!("L{},{}%", line + 1, pct)
    }

    /// Pre-compute the rendered rows for the file view (plan 005 issue 02):
    /// the code rows of the visible buffer lines
    /// `[top_line, top_line + viewport_lines)`, capped to the LARGEST span
    /// that fits so that `code_rows + note_rows <= viewport_lines` always
    /// holds (plan 005 issue 02c: at least one code row, the point's line
    /// always drawn, and a densely-annotated window still fills the
    /// canvas — a 25-line all-annotated file in a 21-row viewport emits
    /// 10 code + 10 note rows, not a 1-row span),
    /// with a virtual annotation note row directly under each annotated
    /// line as the note-row budget allows (`show_note_rows`; `C-c a`
    /// toggles — the `annotated` flag on the code rows is independent, so
    /// the margin marker stays). Every row
    /// carries its buffer-line index: the dense 1:1 "row i == line top+i"
    /// assumption is gone, and the renderer / `cursor_cell` /
    /// `mouse_click_position` translate `buffer_line` ↔ `rendered_row`
    /// through `FileViewRow::row_for_line` / `line_for_row`.
    pub fn file_view_rows(&mut self) -> Vec<FileViewRow> {
        let total = self.current_line_count();
        if total == 0 {
            return Vec::new();
        }
        self.ensure_notes_doc();
        let Some(buf) = self.buffers.current_buffer() else {
            return Vec::new();
        };
        let top = self.scroll_top();
        let mut start = top.min(total.saturating_sub(1));
        let mut end = (top + self.viewport_lines).min(total);
        if start >= end {
            return Vec::new();
        }

        // Get the highlight result from the cache (if any).
        let highlight: Option<&HighlightResult> = self.buffer_highlight_result();

        // This buffer's annotation records (matched by annotation key:
        // project-relative path, or the absolute path for external
        // buffers), in record order. The marker flag is independent of
        // note-row visibility.
        let Some(key) = self.buffers.current().map(String::from) else {
            return Vec::new();
        };
        let rel = self.buffer_annotation_path(&key);
        let records: Vec<&Annotation> = self
            .notes_doc
            .entries
            .iter()
            .filter_map(|e| e.as_record())
            .filter(|a| rel.as_deref() == Some(a.path.as_str()))
            .collect();

        // plan 005 issue 02c: choose the LARGEST code-row span that fits
        // the canvas — the max `s` in [1, window] with
        // `s + notes_in_window(s) <= viewport_lines` (the canvas has
        // exactly viewport_lines rows; note rows steal canvas rows, so
        // the code rows must be reduced). `notes_in_window` is monotonic
        // non-decreasing in `s`, so `s + notes_in_window(s)` is strictly
        // increasing in `s`: a scan from the window top down finds the
        // largest fitting span, and it is the answer (02b instead
        // floored `viewport_lines - n_notes` at 1, which under-filled —
        // a 25-line all-annotated file showed 3 rows of 21). The span is
        // at least 1 (round 2 P1): if no larger span fits (e.g. many
        // records on one line), the floor stays as the blank-view
        // guarantee. If the point's line is excluded by the span, advance
        // start so the point is always drawn.
        if self.show_note_rows && !records.is_empty() {
            let n_notes: usize = records
                .iter()
                .filter(|a| a.line >= start && a.line < end)
                .count();
            if n_notes > 0 {
                let window = end - start; // <= viewport_lines
                let mut code_span = 1; // the floor: blank-view guarantee
                for s in (1..=window).rev() {
                    let in_span = records
                        .iter()
                        .filter(|a| a.line >= start && a.line < start + s)
                        .count();
                    if s + in_span <= self.viewport_lines {
                        code_span = s;
                        break;
                    }
                }
                end = (start + code_span).min(total);
                // Ensure the point's line is in the emitted range: if the
                // cap excluded it, advance start so the point is the last
                // code row.
                let point_line = self.file_point().line.min(total.saturating_sub(1));
                if point_line >= end {
                    start = point_line.saturating_add(1).saturating_sub(code_span);
                    end = (start + code_span).min(total);
                }
            }
        }

        // Round 2 (budget counts are FINAL): the cap above is sized from
        // the FULL window's note count, but the emitted range — shrunk to
        // `code_span` and possibly advanced for the point — can hold a
        // different (larger) note count, e.g. many records on one line.
        // Recount in the emitted range and cap the emitted NOTE rows so
        // `code_rows + note_rows <= viewport_lines` always holds; the
        // code rows (>= 1, including the point's line) are emitted first
        // and never reduced further.
        let code_rows = end - start;
        let mut notes_left = self.viewport_lines.saturating_sub(code_rows);

        let mut out = Vec::with_capacity(end.saturating_sub(start) + notes_left);
        for line in start..end {
            let text = buf.line_text(line).unwrap_or_default().to_string();
            let spans = highlight
                .and_then(|h| h.lines.get(line))
                .map(|hl| hl.spans.clone())
                .unwrap_or_default();
            let annotated = records.iter().any(|a| a.line == line);
            // Issue match-highlight: this line's match ranges, clipped to
            // the line (VISIBLE lines only — the context is pre-computed,
            // so nothing here scans the whole buffer per frame).
            let matches = match_ranges_for_line(
                &self.match_context,
                &key,
                buf.try_line_to_byte(line),
                text.len(),
            );
            out.push(FileViewRow {
                line,
                is_note: false,
                annotated,
                matches,
                // jump-highlight: the snapshot attaches the landing row's
                // highlight (with the frame's fade intensity) after this.
                highlight: None,
                text,
                spans,
            });
            if self.show_note_rows {
                for a in records.iter().filter(|a| a.line == line) {
                    if notes_left == 0 {
                        break; // the note-row budget is final: stop here
                    }
                    notes_left -= 1;
                    let mut note = format!("  \u{25b8} {}", a.text);
                    if a.orphaned {
                        note.push_str(" (orphaned)");
                    }
                    out.push(FileViewRow {
                        line,
                        is_note: true,
                        annotated: false,
                        matches: Vec::new(),
                        highlight: None,
                        text: note,
                        spans: Vec::new(),
                    });
                }
            }
        }
        out
    }

    /// The total number of rendered rows for the current buffer
    /// (plan 005 issue 02): the buffer's line count plus the visible
    /// annotation note rows (zero when `C-c a` hid them). The renderer's
    /// bottom scroll indicator compares the slice length against this in
    /// rendered-row space.
    pub fn file_view_total_rows(&mut self) -> usize {        let total = self
            .buffers
            .current_buffer()
            .map(|b| b.line_count())
            .unwrap_or(0);
        if !self.show_note_rows {
            return total;
        }
        self.ensure_notes_doc();
        let key = self.buffers.current().map(String::from);
        let Some(rel) = key.as_deref().and_then(|k| self.buffer_annotation_path(k)) else {
            return total;
        };
        total + self
            .notes_doc
            .entries
            .iter()
            .filter(|e| matches!(e, NotesEntry::Record(a) if a.path == rel))
            .count()
    }

    /// Whether the buffer at `key` is PROJECT-OWNED (006-02b item 1, the
    /// edit-mode/save ownership guard): `true` for path-less (scratch)
    /// buffers, for paths under the current project root (project files +
    /// the notes file — this also covers a PREVIOUS project's buffers after
    /// a switch: they are the user's own files, not a shared cache), and
    /// for any other buffer that was not opened via the external-landing
    /// path. `false` for external (tooling / registry) sources — a cache
    /// shared by every project on the machine, never editable, never
    /// written.
    pub(super) fn buffer_is_project_owned(&self, key: &str) -> bool {
        let Some(buf) = self.buffers.get(key) else {
            return false;
        };
        let Some(path) = buf.path.as_ref() else {
            return true; // scratch (no on-disk path): as today
        };
        if let Some(root) = self.project.as_ref().map(|p| &p.root)
            && path.starts_with(root)
        {
            return true;
        }
        !self.external_buffers.contains(key)
    }

    /// The buffer at `key`'s annotation key (plan 008 issue 01, the ONE
    /// key-derivation point for annotation records): the project-relative
    /// path (lossy) when the buffer's path strips under the project root
    /// — byte-identical to the pre-008 key for existing project buffers
    /// (no migration, no notes-file change) — else the absolute path
    /// string (external buffers opened read-only via `open_external_path`,
    /// e.g. `M-.` into a registry source). Notes still live in the
    /// project's `.redline-notes.md`; the record's `path` simply carries
    /// the absolute string. `None` only for pathless buffers (scratch).
    pub(super) fn buffer_annotation_path(&self, key: &str) -> Option<String> {
        let buf = self.buffers.get(key)?;
        let abs = buf.path.as_ref()?;
        if let Some(root) = self.project.as_ref().map(|p| &p.root)
            && let Ok(rel) = abs.strip_prefix(root)
        {
            return Some(rel.to_string_lossy().into_owned());
        }
        Some(abs.to_string_lossy().into_owned())
    }

    /// (top_line, total_lines, viewport_lines) for the file view.
    pub fn file_view_scroll_info(&self) -> (usize, usize, usize) {
        let total = self
            .buffers
            .current_buffer()
            .map(|b| b.line_count())
            .unwrap_or(0);
        (self.scroll_top(), total, self.viewport_lines)
    }

    /// The current buffer's file-view point `(line, col)` for the UI cursor
    /// cell (plan 004 issue 05b). `(0, 0)` when there is no current buffer.
    pub fn file_view_point(&self) -> (usize, usize) {
        let p = self.file_point();
        (p.line, p.col)
    }

    /// The line count of the current buffer (0 when none).
    pub(super) fn current_line_count(&self) -> usize {
        self.buffers.current_buffer().map(|b| b.line_count()).unwrap_or(0)
    }

    /// The character length of buffer line `line` in the current buffer.
    fn line_char_len(&self, line: usize) -> usize {
        self.buffers
            .current_buffer()
            .and_then(|b| b.line_text(line))
            .map(|t| t.chars().count())
            .unwrap_or(0)
    }

    /// The current buffer's line `line` as chars (empty when out of range or
    /// no current buffer).
    fn line_chars(&self, line: usize) -> Vec<char> {
        self.buffers
            .current_buffer()
            .and_then(|b| b.line_text(line))
            .map_or_else(Vec::new, |t| t.chars().collect())
    }

    /// The current buffer's point, clamped to the buffer's bounds (a reload
    /// that shrinks the buffer self-heals here). `(0,0)` when there is no
    /// current buffer or it is empty. `goal_col` is not clamped to the
    /// current line — it may target a longer line that a later C-n/C-p
    /// moves onto.
    pub(super) fn file_point(&self) -> FilePoint {
        let Some(key) = self.buffers.current().map(String::from) else {
            return FilePoint::default();
        };
        let Some(buf) = self.buffers.get(&key) else {
            return FilePoint::default();
        };
        let total = buf.line_count();
        if total == 0 {
            return FilePoint::default();
        }
        let p = self.point.get(&key).copied().unwrap_or_default();
        let line = p.line.min(total - 1);
        let line_len = buf
            .line_text(line)
            .map(|t| t.chars().count())
            .unwrap_or(0);
        FilePoint {
            line,
            col: p.col.min(line_len),
            goal_col: p.goal_col,
        }
    }

    /// The point's line (0-based) for the current buffer.
    pub(super) fn point_line(&self) -> usize {
        self.file_point().line
    }

    /// The point's column (0-based char offset) for the current buffer.
    pub(super) fn point_col(&self) -> usize {
        self.file_point().col
    }

    /// Set the current buffer's point to `(line, col)` with goal column
    /// `goal_col` (all clamped to the buffer's bounds), then make the window
    /// follow so the point's line stays in the viewport (the shipped
    /// follow-scroll pattern). No-op when there is no current buffer.
    pub(super) fn set_point(&mut self, line: usize, col: usize, goal_col: usize) {
        let Some(key) = self.buffers.current().map(String::from) else {
            return;
        };
        let total = self
            .buffers
            .get(&key)
            .map(|b| b.line_count())
            .unwrap_or(0);
        if total == 0 {
            return;
        }
        let line = line.min(total - 1);
        let line_len = self
            .buffers
            .get(&key)
            .and_then(|b| b.line_text(line))
            .map(|t| t.chars().count())
            .unwrap_or(0);
        let col = col.min(line_len);
        // NOTE: `goal_col` is deliberately NOT clamped to the current line's
        // length — it is the emacs goal column, which a short line clamps
        // away from but a later C-n/C-p onto a longer line restores. It is
        // only ever applied with `.min(line_len)` at the moment of use.
        self.point
            .insert(key.clone(), FilePoint { line, col, goal_col });
        // Window follows: keep the point's line in the viewport.
        let window = self.viewport_lines.max(1);
        let top = self.scroll.get(&key).copied().unwrap_or(0);
        let next = keep_cursor_visible(top, line, total, window);
        if next != top {
            self.scroll.insert(key, next);
        }
    }

    /// A col-0 landing: `set_point` with the point's column (and goal
    /// column) reset to 0. For the callers whose target carries no
    /// column — goto-line (M-g g; emacs `goto-line` lands at the line
    /// start), imenu / the xref & symbol pickers (M-i / M-.; candidates
    /// are "symbol:line" / "file:line"), resolver landings
    /// (`ResolvedSource` has no column), and external xref jumps (line-only
    /// outcome). Callers that DO know a landing column must not use
    /// this — they land via `set_point(line, col, col)`: isearch lands
    /// on the match's column, isearch cancel (C-g) restores the recorded
    /// pre-search (line, col), C-x C-x lands the mark's byte offset via
    /// `try_byte_to_line_col`, project-search RET lands the hit's byte
    /// column (converted within the line; regex hits have none and land
    /// col 0), the unique-definition `M-.` jump lands
    /// `Symbol.start_byte` via `try_byte_to_line_col`, and `M-,` /
    /// picker-selected `M-.` restore the recorded column
    /// (`navigate_to_entry`).
    pub(super) fn set_point_line(&mut self, line: usize) {
        self.set_point(line, 0, 0);
    }

    /// Move the file-view window by `delta` lines (`delta > 0` = forward /
    /// down, `delta < 0` = backward / up), keeping the point's **screen row**
    /// fixed: the point's buffer line is recomputed from its on-screen row
    /// after the window moves (emacs scroll behavior). The point's column is
    /// re-clamped to the new line's length. Pure window motion — the cursor
    /// does not move on screen. The window top clamps exactly like
    /// `set_scroll_top` (so a buffer that fits the viewport still scrolls to
    /// its last line, matching the pre-05b behavior).
    fn scroll_window_point(&mut self, delta: i64) {
        let total = self.current_line_count();
        if total == 0 {
            return;
        }
        let p = self.file_point();
        let old_top = self.scroll_top();
        let screen_row = p.line.saturating_sub(old_top);
        let new_top = (old_top as i64 + delta).clamp(0, (total - 1) as i64) as usize;
        self.set_scroll_top(new_top);
        let new_line = (new_top + screen_row).min(total - 1);
        let new_len = self.line_char_len(new_line);
        let new_col = p.col.min(new_len);
        let key = self.buffers.current().map(String::from).unwrap();
        self.point.insert(
            key,
            FilePoint {
                line: new_line,
                col: new_col,
                goal_col: p.goal_col,
            },
        );
    }

    /// C-n / Down: point down one line, preserving the goal column (emacs
    /// `next-line`). No-op at the last line (buffer end).
    pub fn point_down(&mut self) {
        let p = self.file_point();
        let total = self.current_line_count();
        if total == 0 || p.line + 1 >= total {
            return;
        }
        let nl = p.line + 1;
        self.set_point(nl, p.goal_col.min(self.line_char_len(nl)), p.goal_col);
    }

    /// C-p / Up: point up one line, preserving the goal column (emacs
    /// `previous-line`). No-op at the first line.
    pub fn point_up(&mut self) {
        let p = self.file_point();
        if p.line == 0 {
            return;
        }
        let nl = p.line - 1;
        self.set_point(nl, p.goal_col.min(self.line_char_len(nl)), p.goal_col);
    }

    /// C-f / Right: point forward one character; wrap to the next line's
    /// start at end-of-line (emacs `forward-char`). No-op at the buffer end.
    pub fn point_forward(&mut self) {
        let p = self.file_point();
        let total = self.current_line_count();
        let line_len = self.line_char_len(p.line);
        if p.col < line_len {
            self.set_point(p.line, p.col + 1, p.col + 1);
        } else if p.line + 1 < total {
            self.set_point(p.line + 1, 0, 0);
        }
    }

    /// C-b / Left: point backward one character; wrap to the previous
    /// line's end at beginning-of-line (emacs `backward-char`). No-op at the
    /// buffer start.
    pub fn point_backward(&mut self) {
        let p = self.file_point();
        if p.col > 0 {
            self.set_point(p.line, p.col - 1, p.col - 1);
        } else if p.line > 0 {
            let prev_len = self.line_char_len(p.line - 1);
            self.set_point(p.line - 1, prev_len, prev_len);
        }
    }

    /// C-a: point to the beginning of the line (col 0).
    pub fn point_line_start(&mut self) {
        let p = self.file_point();
        self.set_point(p.line, 0, 0);
    }

    /// C-e: point to the end of the line (col = the line's char length).
    pub fn point_line_end(&mut self) {
        let p = self.file_point();
        let line_len = self.line_char_len(p.line);
        self.set_point(p.line, line_len, line_len);
    }

    /// M-f (plan 004 issue 05c; emacs `forward-word`): move to the END of
    /// the next word. If the char at point is a word char, advance to the
    /// end of that word run. Otherwise skip the non-word run
    /// (punctuation/whitespace) up to the next word, THEN walk word chars
    /// to that word's end — `forward-word` lands at the word's END, not its
    /// first char. Newlines are non-word, so the skip crosses line
    /// boundaries and keeps skipping (and then walks) on the next line; at
    /// the end of the buffer it is a no-op. A word motion sets `goal_col`
    /// to the landing column (emacs), and the window follows the point.
    pub fn point_word_forward(&mut self) {
        let p = self.file_point();
        let total = self.current_line_count();
        if total == 0 {
            return;
        }
        let mut line = p.line;
        let mut col = p.col;
        let mut moved = false;
        loop {
            let chars = self.line_chars(line);
            let len = chars.len();
            // Skip the non-word run (punctuation/whitespace) up to the
            // next word's first char.
            while col < len && !is_word_char(chars[col]) {
                col += 1;
                moved = true;
            }
            if col < len {
                // Landed on the next word's first char: advance to the
                // word's END (emacs forward-word lands at the end, not the
                // first char). A word run cannot span a line.
                while col < len && is_word_char(chars[col]) {
                    col += 1;
                    moved = true;
                }
                break;
            }
            // The non-word run reaches EOL: cross to the next line (the
            // newline is part of the run) or stop at the buffer end.
            if line + 1 < total {
                line += 1;
                col = 0;
                moved = true;
            } else {
                break;
            }
        }
        if moved {
            self.set_point(line, col, col);
        }
    }

    /// M-b (plan 004 issue 05c; emacs `backward-word`): move to the START
    /// of the previous word. If the char before point is a word char, that
    /// word's start; otherwise skip the non-word run backward (crossing
    /// lines — the newline is non-word), then keep retreating while word
    /// chars to the word's first char. `backward-word` lands at the word's
    /// START, not its end. A word motion sets `goal_col` to the landing
    /// column (emacs), and the window follows the point.
    pub fn point_word_backward(&mut self) {
        let p = self.file_point();
        let total = self.current_line_count();
        if total == 0 || (p.line == 0 && p.col == 0) {
            return;
        }
        if p.col > 0 && is_word_char(self.line_chars(p.line)[p.col - 1]) {
            // Preceded by a word: retreat to its first char.
            let chars = self.line_chars(p.line);
            let mut col = p.col;
            while col > 0 && is_word_char(chars[col - 1]) {
                col -= 1;
            }
            self.set_point(p.line, col, col);
            return;
        }
        // Non-word before point (or point at a line start, where the
        // preceding char is the newline): skip the non-word run backward,
        // crossing lines (the newline is part of the run).
        let mut line = p.line;
        let mut col = p.col;
        loop {
            if col > 0 {
                let chars = self.line_chars(line);
                while col > 0 && !is_word_char(chars[col - 1]) {
                    col -= 1;
                }
                if col > 0 {
                    // Landed just past a word's last char: retreat to the
                    // word's FIRST char (emacs backward-word lands at the
                    // start, not the end).
                    while col > 0 && is_word_char(chars[col - 1]) {
                        col -= 1;
                    }
                    self.set_point(line, col, col);
                    return;
                }
            }
            if line == 0 {
                return; // reached the start of the buffer
            }
            line -= 1;
            col = self.line_char_len(line);
        }
    }

    /// M-<: point to the buffer start (line 0, col 0); the window follows.
    pub fn point_buffer_start(&mut self) {
        self.set_point(0, 0, 0);
    }

    /// M-END: point to the buffer end (last line, last col); the window
    /// follows. (jump-ambiguity: `M->` moved to the force-definition-list
    /// hotkey — verified against the parity reference, vanilla emacs -Q
    /// 30.2: `M-<end>` is end-of-buffer-OTHER-WINDOW, a window-splitting
    /// command meaningless under the locked single-pane design, so the
    /// rebind costs no parity; `G` still binds this command.)
    pub fn point_buffer_end(&mut self) {
        let total = self.current_line_count();
        if total == 0 {
            return;
        }
        let line = total - 1;
        let line_len = self.line_char_len(line);
        self.set_point(line, line_len, line_len);
    }

    /// The highlight result for the current buffer, from the cache.
    /// Returns `None` when the buffer is plain text, big-file, or the
    /// cache has no entry for the current (path, mtime, theme).
    fn buffer_highlight_result(&self) -> Option<&HighlightResult> {
        let key = self.buffers.current()?.to_string();
        let buf = self.buffers.get(&key)?;
        // Big files skip highlighting entirely.
        if buf.is_big() {
            return None;
        }
        let path = buf.path.as_ref()?;
        let lang = self.grammar_registry.language_for(&path.to_string_lossy());
        if lang == redline_syntax::registry::LanguageId::Plain {
            return None;
        }
        let cache_key = CacheKey::new(path, buf.mtime, self.theme.name());
        self.highlight_cache.get(&cache_key)
    }

    /// Ensure the current buffer's highlight is cached; builds it on
    /// a cache miss. Called on buffer open and on theme change.
    pub fn ensure_highlight(&mut self) {
        if let Some(key) = self.buffers.current().map(str::to_string) {
            self.ensure_highlight_for_key(&key);
        }
    }

    /// Build (or reuse) the highlight for the buffer with `key`; a no-op
    /// when the buffer is plain text, big, or already cached for the
    /// current (path, mtime, theme). Used by reloads so a re-read buffer is
    /// re-highlighted immediately (cache invalidation by mtime).
    ///
    /// Plan 007 issue 04: for reuse-capable languages the reparse is
    /// incremental when a retained parse tree exists for this (buffer,
    /// mtime) — the edit paths recorded their `InputEdit`s on it and the
    /// parser gets the old tree as a hint. Fallbacks to the full parse:
    /// no retained tree, the retained tree carries a parse error (stale
    /// or mid-typing baseline), or the incremental parse itself errors.
    /// A disk reload changes the mtime, so its stale tree never matches
    /// the `TreeKey` and the buffer parses from scratch.
    pub(super) fn ensure_highlight_for_key(&mut self, key: &str) {
        let (path, mtime, big) = {
            let Some(buf) = self.buffers.get(key) else { return };
            (buf.path.clone(), buf.mtime, buf.is_big())
        };
        let Some(path) = path else { return };
        if big {
            return;
        }
        let path_str = path.to_string_lossy().into_owned();
        let lang = self.grammar_registry.language_for(&path_str);
        if lang == LanguageId::Plain {
            return;
        }
        let cache_key = CacheKey::new(&path, mtime, self.theme.name());
        if self.highlight_cache.get(&cache_key).is_some() {
            return; // cache hit
        }
        // Cache miss: build the highlight.
        let rope = {
            let buf = self.buffers.get(key).unwrap();
            // Rope clone is O(1) (data sharing).
            buf.rope.clone()
        };
        let config = match self.grammar_registry.config(lang) {
            Some(c) => c,
            None => return,
        };
        let result = if highlight::supports_reuse(lang) {
            let tree_key = TreeKey::new(key, mtime);
            let incremental = self
                .highlight_cache
                .retain_tree(&tree_key)
                .and_then(|retained| highlight::highlight_with_tree(&rope, lang, retained));
            match incremental {
                Some(r) => r,
                None => highlight::highlight_reusable(&rope, lang)
                    .map(|(result, tree)| {
                        // Retain the freshly parsed tree as the next
                        // incremental baseline.
                        self.highlight_cache
                            .retain_insert(tree_key, RetainedTree::new(tree));
                        result
                    }),
            }
        } else {
            // JS/TS/TSX: local-variable tracking lives in the
            // tree-sitter-highlight Highlighter, which does not expose
            // its parse tree — keep the full-parse path for them.
            highlight::highlight(&rope, config, lang)
        };
        match result {
            Ok(result) => {
                self.highlight_cache.insert(cache_key, result);
            }
            Err(()) => {
                tracing::warn!("highlight failed for {path_str}");
            }
        }
    }

    /// Start goto-line mode.
    pub fn goto_line_start(&mut self) {
        // PART A fix (item 5): never latch goto-line behind an open picker
        // (M-g g with a picker open is a no-op, not a goto-line prompt).
        if self.picker.is_some() {
            return;
        }
        self.goto_line_active = true;
        self.goto_line_input.clear();
        self.minibuffer_message("Go to line: ");
    }

    /// Whether goto-line mode is active.
    #[allow(dead_code)] // public API: used by tests and future UI layers
    pub fn goto_line_active(&self) -> bool {
        self.goto_line_active
    }

    /// The goto-line input buffer (for display).
    #[allow(dead_code)] // public API: used by tests and future UI layers
    pub fn goto_line_input(&self) -> &str {
        &self.goto_line_input
    }

    /// Append a digit to the goto-line input.
    pub(super) fn goto_line_digit(&mut self, c: char) {
        self.goto_line_input.push(c);
        self.minibuffer_message(&format!("Go to line: {}", self.goto_line_input));
    }

    /// Backspace in goto-line input.
    pub(super) fn goto_line_backspace(&mut self) {
        self.goto_line_input.pop();
        self.minibuffer_message(&format!("Go to line: {}", self.goto_line_input));
    }

    /// Confirm goto-line (RET): scroll to the target line. PART A fix
    /// (item 5): the input is 1-BASED (matching the error message and
    /// emacs), so `M-g g 50` lands on line 50 (0-based scroll top 49).
    pub fn goto_line_confirm(&mut self) {
        if !self.goto_line_active {
            return;
        }
        self.goto_line_active = false;
        let input = self.goto_line_input.clone();
        self.goto_line_input.clear();
        if let Ok(line) = input.parse::<usize>() {
            let total = self
                .buffers
                .current_buffer()
                .map(|b| b.line_count())
                .unwrap_or(0);
            if line >= 1 && line <= total {
                self.set_point_line(line - 1);
                self.minibuffer_message("");
            } else {
                self.minibuffer_message(&format!("line {line} out of range (1-{total})"));
            }
        } else {
            self.minibuffer_message("invalid line number");
        }
    }

    /// Cancel goto-line (C-g).
    pub fn goto_line_cancel(&mut self) {
        if !self.goto_line_active {
            return;
        }
        self.goto_line_active = false;
        self.goto_line_input.clear();
        self.minibuffer_message("cancel");
    }
}

/// Issue match-highlight: the active match context's ranges clipped to ONE
/// visible line: line-relative byte offsets (the same domain as the syntax
/// spans — the renderer converts to char offsets for the overlay), the end
/// clipped to the line's byte length (a stale or line-straddling range
/// clips rather than overhanging), `selected` flagged. Ranges are
/// consulted by binary search over the start-sorted `ctx.ranges`, so the
/// per-line cost is independent of the match count. Empty when the context
/// is for another buffer, has no ranges, or no range starts inside the
/// line (a line-straddling query's second half never highlights —
/// queries in practice cannot carry a newline: Enter confirms, it does
/// not self-insert).
pub(super) fn match_ranges_for_line(
    ctx: &MatchContext,
    key: &str,
    line_start: Option<usize>,
    line_len: usize,
) -> Vec<LineMatch> {
    if ctx.ranges.is_empty() || ctx.buffer_key != key {
        return Vec::new();
    }
    let Some(line_start) = line_start else {
        return Vec::new();
    };
    let line_end = line_start + line_len;
    let first = ctx.ranges.partition_point(|&(s, _)| s < line_start);
    ctx.ranges
        .iter()
        .enumerate()
        .skip(first)
        .take_while(|&(_, &(s, _))| s < line_end)
        .map(|(i, &(s, e))| LineMatch {
            start: s - line_start,
            end: e.saturating_sub(line_start).min(line_len),
            selected: i == ctx.selected,
        })
        .collect()
}

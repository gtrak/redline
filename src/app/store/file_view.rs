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
        // plan 016 issue 04 (gate P2): a mouse command ends the self-insert
        // run. The rule is "any path that RUNS a command clears the marker",
        // and these handlers never pass through `key_event`, where the
        // key-path clears live — so before this a click or a wheel tick left
        // the run armed and a later self-insert merged ACROSS it into one
        // undo step (one `C-x u` removed "abc" spanning a mouse click).
        self.self_insert_run = None;
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
        // plan 016 issue 04 (gate P2): see `mouse_scroll_up` — a mouse
        // command ends the self-insert run.
        self.self_insert_run = None;
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
        // plan 016 issue 04 (gate P2): a click that PLACES THE POINT ends the
        // self-insert run — it is point motion, and the run rule says any
        // command that runs ends the run. Placed AFTER the guard on purpose:
        // a click that ran no command (wrong view) must leave the run armed,
        // exactly like a pending prefix press or an unbound-key echo.
        self.self_insert_run = None;
        // Map-aware (plan 005 issue 02): with note rows visible a rendered
        // row is NOT `scroll_top + row` buffer lines — translate through
        // the rendered-row list (a note row maps to its anchored code
        // row; the note rows sit ABOVE their code row — the map is
        // order-agnostic, annotations-render-fold). A click past the last
        // rendered row maps to the last CODE row in the slice.
        let rows = self.file_view_rows();
        let Some(target_line) = FileViewRow::line_for_row(&rows, row)
            .or_else(|| rows.iter().rev().find(|r| !r.is_note).map(|r| r.line))
        else {
            return;
        };
        // issue-annotations-symbol-precise: the indicator sits before the
        // SYMBOL the annotation was made on, at that record's own anchor
        // (a SET of anchors on the code row — one per distinct record). A
        // click ON an indicator cell maps to THAT record's symbol char (its
        // stored `col`, the landed semantics — re-verified here, not
        // assumed: the indicator moved from the line's indent to the
        // symbol's own column, and a mid-line indicator sits inside the
        // code region, where the plain tail mapping would land on the
        // symbol itself or its preceding space instead). A click on a
        // blank cell in the ANCHORED ZONE (at or after the leftmost
        // indicator, left of the code) maps to the record with the largest
        // anchor at or before the click. A blank cell left of EVERY
        // indicator keeps the landed first-token semantics: it maps to the
        // line's first non-whitespace char (`indent_chars`) — e.g. a
        // record on an indented line's first token has its indicator at
        // the last indent cell, and the cells to its left are the line's
        // own indentation, not another record's territory. A click at or
        // right of `code_start` maps within the code: the code tail (the
        // full line minus its leading whitespace) starts at display column
        // `code_start`, so the char index is `indent_chars +
        // display_col_to_char_index(code_tail, col - code_start)`. A
        // non-annotated line maps the display column straight onto its full
        // line (the old, unchanged behavior).
        let target_row = rows.iter().find(|r| !r.is_note && r.line == target_line);
        let line_len = self.line_char_len(target_line);
        let key = self.buffers.current().map(String::from);
        // Display column -> char index (plan 004 issue 05d).
        let char_col = match target_row.filter(|r| r.annotated) {
            Some(r) => {
                let code_start = r.code_start;
                let indent = r.indent_chars;
                // The line's records (record order) and their (anchor,
                // char col) pairs — the same geometry the row's `anchors`
                // set dedups from.
                let Some(key) = key else {
                    return;
                };
                let Some(rel) = self.buffer_annotation_path(&key) else {
                    return;
                };
                let line_records: Vec<&Annotation> = self
                    .notes_doc
                    .entries
                    .iter()
                    .filter_map(|e| e.as_record())
                    .filter(|a| a.path == rel && a.line == target_line)
                    .collect();
                let mapped = self
                    .buffers
                    .current_buffer()
                    .and_then(|b| b.line_text(target_line))
                    .map(|t| {
                        let pairs = Self::line_anchors(&t, &line_records);
                        // 1) An indicator cell maps to its record's
                        // symbol char (the record's char column).
                        pairs
                            .iter()
                            .find(|&&(a, _)| a == col)
                            .map(|&(_, cc)| cc)
                            // 2) The anchored zone (at or after the
                            // leftmost indicator, left of the code): the
                            // record with the largest anchor at or before
                            // the click. A blank cell left of EVERY
                            // indicator keeps the landed first-token
                            // mapping (the line's own indentation belongs
                            // to the line's first token, not to any
                            // record's territory).
                            .or_else(|| {
                                if col < code_start {
                                    Some(
                                        pairs
                                            .iter()
                                            .filter(|&&(a, _)| a <= col)
                                            .max_by_key(|&&(a, _)| a)
                                            .map_or_else(
                                                || indent.min(line_len),
                                                |&(_, cc)| cc,
                                            ),
                                    )
                                } else {
                                    None
                                }
                            })
                            // 3) The code region: display col -> char in the
                            // code tail (`indent` is the leading run's byte
                            // length (spaces/tabs are single-byte), a
                            // valid char boundary; the code tail starts
                            // there).
                            .unwrap_or_else(|| {
                                let tail = &t[indent.min(t.len())..];
                                indent
                                    + crate::model::text_width::display_col_to_char_index(
                                        tail,
                                        col - code_start,
                                    )
                            })
                    })
                    .unwrap_or(indent);
                mapped.min(line_len)
            }
            None => self
                .buffers
                .current_buffer()
                .and_then(|b| b.line_text(target_line))
                .map(|t| crate::model::text_width::display_col_to_char_index(&t, col))
                .unwrap_or(0)
                .min(line_len),
        };
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
    /// with a virtual annotation note row directly ABOVE each annotated
    /// line as the note-row budget allows (annotations-fold-visual: the
    /// note reads as a header for the code it annotates; `show_note_rows`
    /// folds the blocks in and out via the `C-c a h` toggle — the
    /// `annotated` flag on the code rows is independent, so the margin
    /// arrow stays, in its folded ▸ state when the note rows are hidden).
    /// Every row
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
            let full_text = buf.line_text(line).unwrap_or_default();
            // issue-annotations-symbol-precise: the indicator sits before
            // the SYMBOL the annotation was made on — one anchor PER RECORD.
            // `line_anchors` returns each record's (final anchor, record
            // char col) in record order: the record's stored `col` is a
            // CHAR offset (notes.rs `point_col`), converted to a DISPLAY
            // column against the line's text (wide chars 2 cells, tabs to
            // the next 8-column stop) so the anchor agrees with the
            // renderer's own column arithmetic — a symbol after two CJK
            // chars sits 2 display columns further right than its char
            // offset, and after a tab up to 7. When the cell before the
            // symbol is whitespace the anchor is `display_col - 1` (the
            // indicator overwrites a blank cell — the code does not move);
            // otherwise it falls back to the line's indent anchor
            // (`indent_width - 1`, itself column 0 when the indent is 0 —
            // the exact-1 column-0 shift the line-based rule 78989f7 landed
            // as a special case: when the annotated symbol IS the line's
            // first token the two rules coincide).
            let line_records: Vec<&Annotation> = records
                .iter()
                .filter(|a| a.line == line)
                .copied()
                .collect();
            let anchors_pairs = Self::line_anchors(&full_text, &line_records);
            let annotated = !line_records.is_empty();
            // issue-annotations-anchor-at-symbol: the row's `text` is drawn
            // at `code_start` — the line's leading indentation run's
            // display width when the line has one (the code keeps its
            // source column; every indicator sits in a blank cell); a
            // column-0 line shifts right by exactly one cell only when a
            // column-0 indicator is present (a symbol at the line's start,
            // or a no-whitespace fallback on an unindented line) — a
            // mid-line-only anchor overwrites a blank cell and the line
            // stays put. This is a display-column fact about the line,
            // computed HERE (the store builds the row and knows the full
            // line) and carried on the row — the renderer / cursor / click
            // mapping read `code_start` rather than re-deriving it from the
            // (stripped) row text.
            let (indent_chars, indent_width) =
                if annotated { Self::leading_indent(&full_text) } else { (0, 0) };
            let code_start = if !annotated {
                0
            } else if indent_width > 0 {
                indent_width
            } else {
                // Column-0 line: shift exactly one cell iff a column-0
                // indicator is present.
                anchors_pairs.iter().any(|&(a, _)| a == 0) as usize
            };
            // One indicator per DISTINCT anchor (the row's anchor SET,
            // replacing the landed single `anchor_col`): two records
            // resolving to the same column — e.g. both falling back to the
            // same indent anchor — share that ONE indicator, so the set is
            // deduplicated; the renderer draws one ▴/▸ per entry and
            // folding leaves one ▸ per entry.
            let anchors: Vec<usize> = anchors_pairs
                .iter()
                .map(|&(a, _)| a)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            // annotations-render-fold: the note rows emit BEFORE the code
            // row — the note reads as a header for the code it annotates,
            // not a trailer under it. The budget bookkeeping is unchanged
            // (the code rows are never reduced; the note-row budget is
            // final). issue-annotations-symbol-precise: each note row
            // carries its OWN record's anchor (replacing the landed
            // "several records on one line share one anchor" rule) — two
            // annotations on one line get two note rows, each ╭ at its own
            // record's anchor. Their note rows stack above the code row in
            // record order, and folding hides them all while the code-row
            // indicators (▸) survive. `line_records[i]` and
            // `anchors_pairs[i]` are the same record (record order).
            if self.show_note_rows {
                for (i, a) in line_records.iter().enumerate() {
                    if notes_left == 0 {
                        break; // the note-row budget is final: stop here
                    }
                    notes_left -= 1;
                    // annotations-fold-visual: the note row's text is now the
                    // BARE note content — the old `  ▸ ` text prefix is gone
                    // (the canvas's curved corner ╭ at the anchor cell carries
                    // the "this is a note" meaning now).
                    let mut note = a.text.clone();
                    if a.orphaned {
                        note.push_str(" (orphaned)");
                    }
                    let anchor = anchors_pairs[i].0;
                    out.push(FileViewRow {
                        line,
                        is_note: true,
                        annotated: false,
                        anchors: vec![anchor],
                        code_start: 0,
                        indent_chars: 0,
                        matches: Vec::new(),
                        highlight: None,
                        text: note,
                        spans: Vec::new(),
                    });
                }
            }
            // Issue match-highlight: this line's match ranges, clipped to
            // the line (VISIBLE lines only — the context is pre-computed,
            // so nothing here scans the whole buffer per frame). Byte
            // offsets are relative to the FULL line start.
            let matches_full = match_ranges_for_line(
                &self.match_context,
                &key,
                buf.try_line_to_byte(line),
                full_text.len(),
            );
            let spans_full = highlight
                .and_then(|h| h.lines.get(line))
                .map(|hl| hl.spans.clone())
                .unwrap_or_default();
            // issue-annotations-anchor-at-symbol: for an annotated line the
            // leading indentation run is stripped from the row text (the
            // canvas would render a tab as ~1 cell, not the next 8-column
            // tab stop, so controlling placement means owning the leading
            // cells) and `spans`/`matches` are re-based by that run's byte
            // length (spaces/tabs are single-byte, so the char count IS the
            // byte length; spans never start inside the leading run, so the
            // re-base is exact). A column-0 line keeps its full text — the
            // column-0 indicators shift the whole line right by one when
            // present; mid-line indicators on a column-0 line overwrite
            // blank cells and the text stays at column 0.
            let (text, spans, matches) = if annotated {
                let stripped = full_text[indent_chars..].to_string();
                let rebase_spans = spans_full
                    .into_iter()
                    .map(|s| {
                        let start = s.start.saturating_sub(indent_chars);
                        let end = s.end.saturating_sub(indent_chars);
                        redline_syntax::highlight::LineSpan {
                            start,
                            end,
                            face: s.face,
                        }
                    })
                    .collect();
                let rebase_matches = matches_full
                    .into_iter()
                    .map(|m| LineMatch {
                        start: m.start.saturating_sub(indent_chars),
                        end: m.end.saturating_sub(indent_chars),
                        selected: m.selected,
                    })
                    .collect();
                (stripped, rebase_spans, rebase_matches)
            } else {
                (full_text.to_string(), spans_full, matches_full)
            };
            out.push(FileViewRow {
                line,
                is_note: false,
                annotated,
                anchors,
                code_start,
                indent_chars,
                matches,
                // jump-highlight: the snapshot attaches the landing row's
                // highlight (with the frame's fade intensity) after this.
                highlight: None,
                text,
                spans,
            });
        }
        out
    }

    /// issue-annotations-anchor-at-symbol: the line's LEADING INDENTATION
    /// run, as `(char_count, display_width)`. Spaces count one cell each;
    /// a tab advances to the next 8-column tab stop (tab width 8, from
    /// column 0 — the standard terminal rule the anchor's display-column
    /// position must honour: a `\t`-indented line's code sits at display
    /// column 8, so its anchor is column 7). The run stops at the first
    /// non-space, non-tab char (indentation is spaces/tabs; a line of pure
    /// whitespace yields the whole line as the run, which an annotated
    /// blank line renders as just its indicator). Returns `(0, 0)` when the
    /// line has no leading whitespace (the column-0 shift case).
    fn leading_indent(line: &str) -> (usize, usize) {
        let mut chars = 0usize;
        let mut col = 0usize;
        for c in line.chars() {
            match c {
                ' ' => {
                    chars += 1;
                    col += 1;
                }
                '\t' => {
                    chars += 1;
                    col += 8 - (col % 8);
                }
                _ => break,
            }
        }
        (chars, col)
    }

    /// issue-annotations-symbol-precise: the per-record anchor columns for
    /// the records on ONE line (in record order): each entry is
    /// `(final anchor column, record char col)` — the row's `anchors` set
    /// dedups the first halves, and the click mapping walks the pairs.
    ///
    /// The column-0 shift is a LINE property (a column-0 indicator present
    /// → the whole line moves right by exactly one), so it is decided up
    /// front: when the line has no leading indentation AND some record's
    /// anchor is column 0 (a symbol at the line's start, or a
    /// no-whitespace fallback on an unindented line), every RULE-1 anchor
    /// (a whitespace-preceded mid-line symbol) moves WITH the code — the
    /// cell before the symbol stays the (shifted) blank cell, and the
    /// symbol's indicator still sits one cell left of it. The column-0
    /// anchors themselves stay at 0 (the code now starts at 1, behind
    /// them). Records whose anchors are rule-1 but whose display column
    /// exceeds 0 can never resolve to 0 on an unindented line (the char
    /// before a rule-1 symbol is whitespace, and an unindented line's
    /// first char is not), so the two kinds never collide after the shift.
    fn line_anchors(text: &str, line_records: &[&Annotation]) -> Vec<(usize, usize)> {
        if line_records.is_empty() {
            return Vec::new();
        }
        let raw: Vec<(bool, usize, usize)> = line_records
            .iter()
            .map(|a| Self::record_anchor(text, a.col))
            .collect();
        let shift = Self::leading_indent(text).1 == 0
            && raw.iter().any(|&(_, a, _)| a == 0);
        raw.into_iter()
            .map(|(rule_one, a, cc)| (if rule_one && shift { a + 1 } else { a }, cc))
            .collect()
    }

    /// issue-annotations-symbol-precise: `(rule_one, anchor, char_col)`
    /// for ONE record on line `line`, where `record_col` is the record's
    /// stored column — a CHAR offset, not a display column (notes.rs sets
    /// it with `self.point_col()`, and the code comments say so): the char
    /// → display conversion below treats a wide (CJK/emoji) char as 2
    /// cells and a tab as advancing to the next 8-column stop (tab width
    /// 8 from column 0 — the same arithmetic `leading_indent` owns, so the
    /// anchor agrees with the renderer's own column arithmetic).
    ///
    /// The rule:
    /// 1. `display_col > 0` and the cell at `display_col - 1` is
    ///    whitespace → the anchor is `display_col - 1` (the indicator
    ///    overwrites a blank cell — the code does not move). `rule_one`
    ///    is true here (the caller may shift it with the code, `line_
    ///    anchors`).
    /// 2. The cell before the symbol is NOT whitespace (e.g. `x+y`
    ///    annotated at `y`), or the record is at display column 0 (there
    ///    is no cell before the symbol to point at) → fall back to the
    ///    line's indent anchor, `indent_width - 1`. `rule_one` is false —
    ///    a fallback anchor is an indent-cell fact, and it never moves
    ///    with a code shift (it is the cell the code starts behind, or
    ///    the last indent cell).
    ///
    ///    There is deliberately NO standalone `display_col == 0` rule. A
    ///    record at column 0 on an INDENTED line falls back to the indent
    ///    cell; shifting the whole line (code included) one cell right to
    ///    annotate a character that is whitespace rather than a symbol is
    ///    the very "the code moved" defect this rule exists to prevent.
    ///    When the line has NO leading indentation the fallback IS column
    ///    0, and `line_anchors` shifts the code right by exactly one cell
    ///    to make room — the one case where the code moves.
    fn record_anchor(line: &str, record_col: usize) -> (bool, usize, usize) {
        let char_len = line.chars().count();
        let char_col = record_col.min(char_len);
        // Char index → display column (wide = 2 cells, tab = next 8-column
        // stop — a `\t` char's own `char_display_width` is 1, so it is
        // special-cased here; `leading_indent` uses the same rule for the
        // leading run).
        let mut display = 0usize;
        for (i, c) in line.chars().enumerate() {
            if i == char_col {
                break;
            }
            match c {
                '\t' => display += 8 - display % 8,
                _ => display += crate::model::text_width::char_display_width(c),
            }
        }
        if display > 0 && char_col > 0 {
            // The cell at `display - 1` is whitespace iff the char just
            // before the symbol is a space or a tab (both single-cell, and
            // a tab's last cell is that one). A zero-width (combining)
            // predecessor shares its base's cell, so the cell before the
            // symbol is not a whitespace cell — the fallback applies.
            if let Some(prev) = line.chars().nth(char_col - 1)
                && (prev == ' ' || prev == '\t')
            {
                return (true, display - 1, char_col);
            }
        }
        let (_, indent_width) = Self::leading_indent(line);
        (false, indent_width.saturating_sub(1), char_col)
    }

    /// The `annotate-fold` command (annotations-render-fold / annotations-
    /// fold-visual): the read-only-mode fold toggle (hide ↔ show), reachable
    /// from the M-x palette only — deliberately UNBOUND (gate P1 on this
    /// lane): crossterm
    /// 0.29 only decodes a `KeyCode::Modifier(...)` event (CSI u keycodes
    /// 57441 → `LeftShift`, 57442 → `LeftControl`, 57447 → `RightShift`)
    /// when BOTH `DISAMBIGUATE_ESCAPE_CODES` (1) and
    /// `REPORT_ALL_KEYS_AS_ESCAPE_CODES` (8) are enabled, and iocraft 0.9.1
    /// pushes ONLY `REPORT_EVENT_TYPES` (2) — so a bare Shift press never
    /// reaches the app, on any terminal (byte-based terminals send no
    /// bare-Shift bytes at all). `C-c a h` (→ annotate-toggle) is the fold
    /// path. Read-only mode only: in Edit mode a bare Shift is part of
    /// normal text entry and must never fire the fold.
    pub fn annotate_fold(&mut self) {
        if self.top_view() != ViewId::Buffer {
            return;
        }
        if self.current_buffer_editable() {
            return;
        }
        self.annotate_toggle();
    }

    /// Whether the inline annotation note rows are folded away (the UI's
    /// margin arrow carries the folded state on annotated lines — ▸ instead
    /// of ▾, and the tree-line arms disappear). The inverse of the store's
    /// `show_note_rows` fold state.
    pub fn note_rows_folded(&self) -> bool {
        !self.show_note_rows
    }

    /// The total number of rendered rows for the current buffer
    /// (plan 005 issue 02): the buffer's line count plus the visible
    /// annotation note rows (zero when the fold hid them — `C-c a h`). The
    /// renderer's bottom scroll indicator compares the slice length against
    /// this in rendered-row space.
    pub fn file_view_total_rows(&mut self) -> usize {
        let total = self
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
    /// column) reset to 0. For the sole caller whose target carries no
    /// column — goto-line (M-g g; emacs `goto-line` lands at the line
    /// start). Callers that DO know a landing column must not use this —
    /// they land via `set_point(line, col, col)`: isearch lands on the
    /// match's column, isearch cancel (C-g) restores the recorded pre-search
    /// (line, col), C-x C-x lands the mark's byte offset via
    /// `try_byte_to_line_col`, project-search RET lands the hit's byte
    /// column (converted within the line; regex hits have none and land col
    /// 0), the unique-definition `M-.` jump lands `Symbol.start_byte` via
    /// `try_byte_to_line_col`, the xref / symbols / imenu pickers land the
    /// definition's `Symbol.start_byte` column (jump-column-landings),
    /// resolver (tooling) landings refine to the resolved item's first
    /// whole-word occurrence outside a comment or string (the app-side
    /// refinement the `ResolvedSource.line` doc describes), the Annotations
    /// picker lands the record's `Annotation.col` (the point's column at
    /// creation; the offer encodes only "path:line", so it is looked up,
    /// not re-derived), external xref jumps land the crate-index
    /// `start_byte` column, and `M-,` / picker-selected `M-.` restore the
    /// recorded column (`navigate_to_entry`).
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── issue-annotations-symbol-precise: the pure anchor rule ─────────

    /// One record wrapper for `line_anchors` tests: only `col` is read.
    fn rec(col: usize) -> Annotation {
        Annotation { col, ..Default::default() }
    }

    #[test]
    fn record_anchor_rule_one_mid_line_ascii() {
        // `let x = 1;`, record on `x` (char 4, display 4, preceded by a
        // space) → the anchor is display col 3 (rule 1, no fallback).
        assert_eq!(AppStore::record_anchor("let x = 1;", 4), (true, 3, 4));
        // A symbol that IS the line's first token (char 0): display 0 →
        // rule 2's fallback (column 0 when the line is unindented).
        assert_eq!(AppStore::record_anchor("let x = 1;", 0), (false, 0, 0));
    }

    #[test]
    fn record_anchor_rule_two_column_zero() {
        // `fn main() {` annotated at its first token: display 0 → the
        // anchor is 0 (the line shifts right by exactly one cell, decided
        // in `line_anchors`).
        assert_eq!(AppStore::record_anchor("fn main() {", 0), (false, 0, 0));
    }

    #[test]
    fn record_anchor_rule_three_no_whitespace_fallback() {
        // `x+y` annotated at `y`: the cell before the symbol is `+` (not
        // whitespace) → fall back to the line's indent anchor. No indent:
        // the fallback is itself column 0 (rule 2), the exact-1 shift.
        assert_eq!(AppStore::record_anchor("x+y", 2), (false, 0, 2));
        // Indented `x+y`: the fallback is the INDENT anchor (col 3), and
        // the code does not move (the indicator borrows the last indent
        // cell) — the no-whitespace case that makes "the code never moves"
        // absolute.
        assert_eq!(AppStore::record_anchor("    x+y", 6), (false, 3, 6));
    }

    #[test]
    fn record_anchor_wide_chars_are_two_cells() {
        // `中中 x = 1;`: `x` at char 3, but the two CJK chars before it
        // occupy 4 display cells → `x` sits at display col 5, anchor 4.
        // (The char-offset error anchors at 2 — inside the second CJK
        // glyph. Invisible on ASCII lines; the units trap.)
        assert_eq!(AppStore::record_anchor("中中 x = 1;", 3), (true, 4, 3));
        // Emoji (🦀 = 2 cells in the non-CJK table) behaves the same.
        assert_eq!(AppStore::record_anchor("🦀🦀 x;", 3), (true, 4, 3));
        // Annotating the CJK char itself (char 1): the cell before it is
        // `中` (not whitespace) → the fallback (column 0 here), NOT
        // display 1.
        assert_eq!(AppStore::record_anchor("中中 x;", 1), (false, 0, 1));
    }

    #[test]
    fn record_anchor_tabs_advance_to_eight_column_stops() {
        // Leading tab: `\tz;` — `z` at char 1, display col 8 (the tab
        // stop) → anchor 7 (the last tab cell), rule 1 (the tab IS the
        // whitespace before the symbol).
        assert_eq!(AppStore::record_anchor("\tz;", 1), (true, 7, 1));
        // Mid-line tab: `x\ty;` — `y` at char 2; x = 1 cell, the tab to col
        // 8 → anchor 7. A char-offset (or tab-as-1-cell) error anchors at
        // 1 or 2.
        assert_eq!(AppStore::record_anchor("x\ty;", 2), (true, 7, 2));
        // Mixed leading run: ` \tz;` — space (col 0→1) then tab (col 1 →
        // the next 8-column stop, i.e. col 8) → `z` sits at display col 8,
        // anchor 7 — the same cell the indent-anchor fallback would pick
        // (the run's display width is 8: a tab advances TO the stop, it
        // does not stack on top of it).
        assert_eq!(AppStore::record_anchor(" \tz;", 2), (true, 7, 2));
    }

    #[test]
    fn record_anchor_eol_and_clamping() {
        // A record past EOL clamps to EOL; the last char (`b`) is not
        // whitespace → the fallback (column 0 here).
        assert_eq!(AppStore::record_anchor("ab", 99), (false, 0, 2));
        // A trailing space: the cell before EOL is whitespace → rule 1 at
        // display 1.
        assert_eq!(AppStore::record_anchor("a ", 2), (true, 1, 2));
        // A zero-width (combining) predecessor shares its base's cell, so
        // the cell before the symbol is not a whitespace cell → the
        // fallback.
        assert_eq!(AppStore::record_anchor("a\u{301}x", 2), (false, 0, 2));
    }

    #[test]
    fn line_anchors_shift_moves_rule_one_anchors_with_the_code() {
        // `let x = 1;` with a record on `let` (char 0 → column-0 anchor,
        // the shift trigger) AND one on `x` (rule 1, raw anchor 3): the
        // line shifts by one, so the mid-line anchor moves WITH the code
        // (3 → 4 — the cell before the shifted symbol stays the blank
        // one), while the column-0 anchor stays at 0 (the code starts at
        // 1, behind it).
        let pairs = AppStore::line_anchors("let x = 1;", &[&rec(0), &rec(4)]);
        assert_eq!(pairs, vec![(0, 0), (4, 4)]);
        // No column-0 record: no shift, the rule-1 anchor stays put.
        let pairs = AppStore::line_anchors("let x = 1;", &[&rec(4)]);
        assert_eq!(pairs, vec![(3, 4)]);
    }

    #[test]
    fn line_anchors_no_shift_for_fallback_on_unindented_line() {
        // `x+y` annotated at `y`: the fallback lands at column 0 on the
        // unindented line — the shift is decided by `line_anchors` (the
        // pair itself stays (0, 2): the fallback anchor is an indent-cell
        // fact, never a rule-1 anchor, so it does not move).
        let pairs = AppStore::line_anchors("x+y", &[&rec(2)]);
        assert_eq!(pairs, vec![(0, 2)]);
    }

    #[test]
    fn line_anchors_fallbacks_share_one_column() {
        // `    a+b`: record on `a` (char 4 — first token, rule 1 at anchor
        // 3) and on `b` (char 5 — no whitespace before it, the fallback
        // indent anchor 3): BOTH resolve to column 3 — the deduplicated
        // anchor set (the caller's business) holds a single indicator, one
        // ▸ when folded.
        let pairs = AppStore::line_anchors("    a+b", &[&rec(4), &rec(5)]);
        assert_eq!(pairs, vec![(3, 4), (3, 5)]);
        let dedup: std::collections::BTreeSet<usize> =
            pairs.iter().map(|&(a, _)| a).collect();
        assert_eq!(dedup.len(), 1, "both records share the one anchor column");
    }

    #[test]
    fn line_anchors_indented_first_token_coincides_with_landed_rule() {
        // The landed line-based rule (78989f7) is a special case: a record
        // at char 0 on an indented line falls back to the indent anchor —
        // identical to the landed `indent_width - 1`.
        let pairs = AppStore::line_anchors("    if x > 0 {", &[&rec(0)]);
        assert_eq!(pairs, vec![(3, 0)]);
    }
}

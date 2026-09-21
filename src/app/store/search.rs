use super::*;

impl AppStore {
    /// Start an incremental search in the given direction.
    /// Records the pre-search position (line AND column) for clean exit
    /// (C-g restores it).
    pub fn isearch_start(&mut self, direction: IsearchDirection) {
        // PART A fix (item 5): never latch isearch behind an open picker
        // (C-s / C-r with a picker open is a no-op, not a search).
        if self.picker.is_some() {
            return;
        }
        if self.isearch.active {
            return; // already active; C-s during isearch is a no-op
        }
        self.isearch = IsearchState {
            active: true,
            query: String::new(),
            direction,
            matches: Vec::new(),
            current: 0,
            pre_search_line: self.point_line(),
            // A CHAR column (point_col's unit, consumed directly by
            // set_point) — NOT a byte: a byte stored here would land
            // off-by-N on a multibyte line.
            pre_search_col: self.point_col(),
        };
        // A fresh isearch session supersedes any leftover highlight
        // context (issue match-highlight's lifetime rule: a new search
        // with a different query clears; the new query starts empty).
        self.match_context = MatchContext::default();
        self.minibuffer_message("I-search: ");
    }

    /// Append a character to the isearch query and update matches.
    pub(super) fn isearch_query_char(&mut self, c: char) {
        self.isearch.query.push(c);
        self.isearch_recompute();
    }

    /// Remove the last character from the isearch query.
    pub(super) fn isearch_backspace(&mut self) {
        self.isearch.query.pop();
        self.isearch_recompute();
    }

    /// Recompute all matches for the current query and jump to the
    /// first match in the search direction.
    fn isearch_recompute(&mut self) {
        let query = self.isearch.query.clone();
        if query.is_empty() {
            self.isearch.matches.clear();
            self.isearch.current = 0;
            self.minibuffer_message("I-search: ");
            return;
        }
        // Get the full buffer text for searching.
        let text = self
            .buffers
            .current_buffer()
            .map(|b| b.text())
            .unwrap_or_default();
        self.isearch.matches = Self::find_all_matches(&text, &query, self.isearch.direction);
        if self.isearch.matches.is_empty() {            self.isearch.current = 0;
            self.minibuffer_message(&format!("I-search: {query} [no matches]"));
        } else {
            // Jump to the first match in the search direction.
            let start_line = self.point_line();
            let start_byte = self
                .buffers
                .current_buffer()
                .and_then(|b| b.try_line_to_byte(start_line))
                .unwrap_or(0);
            // Find the first match at or after start_byte (forward)
            // or at or before start_byte (backward).
            self.isearch.current = match self.isearch.direction {
                IsearchDirection::Forward => self
                    .isearch
                    .matches
                    .iter()
                    .position(|&m| m >= start_byte)
                    .unwrap_or(0),
                IsearchDirection::Backward => {
                    // Find the last match at or before start_byte.
                    self.isearch
                        .matches
                        .iter()
                        .rposition(|&m| m <= start_byte)
                        .unwrap_or(self.isearch.matches.len().saturating_sub(1))
                }
            };
            self.isearch_jump_to_current();
            let count = self.isearch.matches.len();
            let idx = self.isearch.current + 1;
            self.minibuffer_message(&format!("I-search: {query} [{idx}/{count}]"));
        }
        // Issue match-highlight: the buffer view highlights all matches
        // (the current one prominently) — sync the context with the live
        // state on every query change (cleared on no matches).
        self.isearch_sync_match_context();
    }

    /// Navigate to the next match (C-s) with wrap-around.
    pub fn isearch_next(&mut self) {
        if !self.isearch.active || self.isearch.matches.is_empty() {
            return;
        }
        let n = self.isearch.matches.len();
        self.isearch.current = (self.isearch.current + 1) % n;
        self.isearch_jump_to_current();
        self.isearch_sync_match_context();
        let count = n;
        let idx = self.isearch.current + 1;
        let query = self.isearch.query.clone();
        self.minibuffer_message(&format!("I-search: {query} [{idx}/{count}]"));
    }

    /// Navigate to the previous match (C-r) with wrap-around.
    pub fn isearch_prev(&mut self) {
        if !self.isearch.active || self.isearch.matches.is_empty() {
            return;
        }
        let n = self.isearch.matches.len();
        self.isearch.current = (self.isearch.current + n - 1) % n;
        self.isearch_jump_to_current();
        self.isearch_sync_match_context();
        let count = n;
        let idx = self.isearch.current + 1;
        let query = self.isearch.query.clone();
        self.minibuffer_message(&format!("I-search: {query} [{idx}/{count}]"));
    }

    /// Sync the match-highlight context with the LIVE isearch state
    /// (issue match-highlight): the query, every match's buffer-absolute
    /// byte range `(start, start + query's byte length)`, and the match
    /// under the cursor as selected. Ranges are normalized to start-
    /// ascending order (a backward isearch's `matches` run descending) so
    /// the per-line clipping can binary-search. Cleared on an empty query
    /// or when no match is current.
    fn isearch_sync_match_context(&mut self) {
        let query = self.isearch.query.clone();
        let matches = self.isearch.matches.clone();
        let current = self.isearch.current;
        if query.is_empty() || matches.is_empty() || current >= matches.len() {
            self.match_context = MatchContext::default();
            return;
        }
        let len = query.len();
        let mut ranges: Vec<(usize, usize)> =
            matches.iter().map(|&m| (m, m + len)).collect();
        ranges.sort_by_key(|&(s, _)| s);
        let sel_start = matches[current];
        let selected = ranges
            .iter()
            .position(|&(s, _)| s == sel_start)
            .unwrap_or(0);
        self.match_context = MatchContext {
            buffer_key: self.buffers.current().map(String::from).unwrap_or_default(),
            query,
            ranges,
            selected,
        };
    }

    /// Land the point ON the current match (line and column — the user
    /// reported the cursor stopping at the line's start, not the match)
    /// and scroll the view to show it.
    fn isearch_jump_to_current(&mut self) {
        let Some(&match_byte) = self.isearch.matches.get(self.isearch.current) else {
            return;
        };
        // The matches are BYTE offsets (find_all_matches), but the point's
        // `col` is a CHAR index (`set_point` clamps against
        // `chars().count()`): convert explicitly so a match that follows a
        // multibyte character lands ON the match, not off-by-N.
        let Some((line, col)) = self
            .buffers
            .current_buffer()
            .and_then(|b| b.try_byte_to_line_col(match_byte))
        else {
            return;
        };
        // The landing column becomes the goal column (emacs: a following
        // C-n/C-p holds it) — the same convention as `navigate_to_entry`.
        self.set_point(line, col, col);
    }

    /// Confirm isearch (RET): keep the current position, deactivate. The
    /// match highlight goes with the session — confirm CLEARS it, like
    /// cancel (emacs `isearch-exit` removes the lazy-highlight faces when
    /// the search ends). The user-visible "context after a search" case is
    /// the results-view jump (`set_match_context_from_jump`), which sets
    /// its own context and is unaffected by this. The isearch STATE
    /// (query / matches / current) is kept as-is: repeating `C-s` re-runs
    /// the same search (the highlight lifetime, not the search state,
    /// changes here).
    pub fn isearch_confirm(&mut self) {
        if !self.isearch.active {
            return;
        }
        let query = self.isearch.query.clone();
        self.isearch.active = false;
        // The highlight lifetime rule: isearch confirm CLEARS the match
        // context (matching `isearch_cancel` above and emacs
        // `isearch-exit`) — the faces vanish when the search ends.
        self.match_context = MatchContext::default();
        if query.is_empty() {
            self.minibuffer_message("");
        } else if self.isearch.matches.is_empty() {
            self.minibuffer_message(&format!("I-search: {query} [not found]"));
        } else {
            self.minibuffer_message("");
        }
    }

    /// Cancel isearch (C-g): restore the pre-search position (line AND
    /// column — the cursor goes back where it was, not to the line's
    /// start). The landing column becomes the goal column. The match
    /// highlight goes with the session (issue match-highlight's lifetime
    /// rule: cancel clears).
    pub fn isearch_cancel(&mut self) {
        if !self.isearch.active {
            return;
        }
        self.isearch.active = false;
        self.isearch.matches.clear();
        self.isearch.query.clear();
        self.match_context = MatchContext::default();
        self.set_point(
            self.isearch.pre_search_line,
            self.isearch.pre_search_col,
            self.isearch.pre_search_col,
        );
        self.minibuffer_message("cancel");
    }

    /// Whether isearch is currently active.
    #[allow(dead_code)] // public API: used by tests and future UI layers
    pub fn isearch_active(&self) -> bool {
        self.isearch.active
    }

    /// The isearch match count.
    #[allow(dead_code)] // public API: used by tests and future UI layers
    pub fn isearch_match_count(&self) -> usize {
        self.isearch.matches.len()
    }

    /// The current match index (1-based, for display).
    #[allow(dead_code)] // public API: used by tests and future UI layers
    pub fn isearch_match_index(&self) -> usize {
        self.isearch.current + 1
    }

    /// Find all occurrences of `query` in `text` (case-sensitive, plain
    /// text). Returns byte offsets in search order.
    fn find_all_matches(text: &str, query: &str, direction: IsearchDirection) -> Vec<usize> {
        if query.is_empty() {
            return Vec::new();
        }
        let mut matches = Vec::new();
        let mut search_from = 0;
        while let Some(pos) = text[search_from..].find(query) {
            let match_start = search_from + pos;
            matches.push(match_start);
            // Advance to the next char boundary after the match start
            // (avoids landing mid-UTF-8-char on multibyte matches).
            let mut next = match_start + 1;
            while next < text.len() && !text.is_char_boundary(next) {
                next += 1;
            }
            search_from = next;
        }
        if direction == IsearchDirection::Backward {
            matches.reverse();
        }
        matches
    }

    /// Hand the SearchBus receiver to the UI's drain task (issue 06: the
    /// `use_future` in `Root`) — exactly once per store; `None` when
    /// already taken.
    pub fn search_rx(&mut self) -> Option<mpsc::UnboundedReceiver<SearchEvent>> {
        self.search_rx.take()
    }

    /// Stop the in-flight search job (no-op when idle).
    pub(super) fn cancel_search_job(&mut self) {
        if let Some(flag) = self.search.cancel.take() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// `C-g` in the results view: cancel the in-flight search (the view
    /// stays open on the partial results). A cancel gesture also ends the
    /// highlight session (issue match-highlight's lifetime rule).
    pub fn search_cancel(&mut self) {
        self.match_context = MatchContext::default();
        if self.search.running {
            self.cancel_search_job();
            self.minibuffer_message("search cancelled");
        } else {
            self.minibuffer_message("nothing to cancel");
        }
    }

    /// `q` / `ESC` in the results view: cancel any in-flight search and
    /// close the view. Leaving the results ends the highlight session
    /// (issue match-highlight's lifetime rule: the context survives a
    /// jump back INTO a buffer, not a close of the session).
    pub fn search_close(&mut self) {
        if self.search.running {
            self.cancel_search_job();
        }
        self.match_context = MatchContext::default();
        if self.top_view() == ViewId::Search {
            self.close_view();
        }
    }

    /// `g` in the results view: re-run the current search with the same
    /// query.
    pub fn search_rerun(&mut self) {
        let query = self.search.query.clone();
        match self.search.kind {
            SearchKind::Project => self.start_project_search(query),
            SearchKind::References => self.start_references_search(query),
            SearchKind::Occur => self.start_occur(query),
        }
    }

    /// `n` in the results view: move to the next match (wraps).
    pub fn search_next(&mut self) {
        let n = self.search.hits.len();
        if n == 0 {
            self.minibuffer_message("no matches");
            return;
        }
        self.search.selected = (self.search.selected + 1) % n;
        self.search_keep_visible();
    }

    /// `p` in the results view: move to the previous match (wraps).
    pub fn search_prev(&mut self) {
        let n = self.search.hits.len();
        if n == 0 {
            self.minibuffer_message("no matches");
            return;
        }
        self.search.selected = (self.search.selected + n - 1) % n;
        self.search_keep_visible();
    }

    /// Keep the selected hit's row inside the visible window.
    fn search_keep_visible(&mut self) {
        let viewport = self.viewport_lines.max(1);
        let Some(&row) = self.search.hit_rows.get(self.search.selected) else {
            return;
        };
        let scroll = &mut self.search.scroll;
        if row < *scroll {
            *scroll = row;
        } else if row >= *scroll + viewport {
            *scroll = row - viewport + 1;
        }
    }

    /// `RET` in the results view: jump to the match under the cursor.
    /// Records the jump-stack origin as the results-view sentinel, so
    /// `M-,` returns to the results (with the selection restored) and
    /// the file position becomes the forward entry.
    pub fn search_jump(&mut self) {
        let Some(hit) = self.search.hits.get(self.search.selected).cloned() else {
            self.minibuffer_message("no match at point");
            return;
        };
        let sel = self.search.selected;
        let origin = JumpEntry {
            buffer_key: SEARCH_JUMP_KEY.to_string(),
            line: sel, // the hit index to restore on M-,.
            col: self.search.scroll,
            label: "*search*".to_string(),
        };
        // The pre-search position, captured BEFORE the open (the open
        // changes the current buffer): it records in front of the
        // sentinel so a second `M-,` from the results view lands here
        // (watchlist: `M-,` under the Search view pops through the
        // sentinel to the pre-search position in one step — emacs
        // `xref-pop-marker-stack`). The home state (no buffer) has none:
        // the sentinel stays the stack's first entry, unchanged.
        let pre_search = self.current_jump_entry();
        let file = hit.file.clone();
        let line_no = hit.line_no as usize;
        // The open-or-report seam (like `open_resolved_source`): only the
        // success path leaves the results view and records the jump.
        // Watchlist (the pre-011 artifact): on an OPEN FAILURE the
        // results view stays open and the failure is reported (the jump
        // did not happen) — no jump entry is recorded, no view closed.
        let opened = match self.project.as_ref() {
            Some(project) => self
                .open_project_path(&project.root.join(&file), &file)
                .is_ok(),
            None => false,
        };
        if !opened {
            // The hit's file could not be opened (e.g. it was deleted
            // since the walk, or an occur on a buffer with no file).
            self.minibuffer_message(&format!("cannot open {file}: the jump did not happen"));
            return;
        }
        // The hit's column is a BYTE offset within the line (rg's
        // pipeline unit); the point's `col` is a CHAR index — convert
        // within the hit's line (a raw byte column is off-by-N when a
        // multibyte character precedes the match). `None` (regex hits
        // carry no column) lands col 0.
        let hit_col = hit
            .col
            .map_or(0, |byte_col| Self::byte_col_to_char_col(&hit.line, byte_col));
        self.set_point(line_no.saturating_sub(1), hit_col, hit_col);
        self.ensure_highlight();
        // Leave the results view so the jumped file is what's on screen;
        // `M-,` (the sentinel entry) pushes the results back on top.
        if self.top_view() == ViewId::Search {
            self.close_view();
        }
        // Issue match-highlight: the jump is the reported use case —
        // highlight this buffer's matches of the query (all one face, the
        // jumped hit's range the prominent one, under the cursor).
        self.set_match_context_from_jump(sel);
        // 06a review P1-2: only record the destination jump when the open
        // actually opened a buffer (the `opened` gate above — a failed
        // open leaves the home state, and a `""`-keyed entry would later
        // create `*scratch*` via a dead jump entry (P1-1's class)).
        if let Some(key) = self.buffers.current().map(String::from) {
            if let Some(pre) = pre_search {
                self.jump_stack.record_jump(&pre, &origin);
            }
            let dest = JumpEntry {
                buffer_key: key,
                line: line_no.saturating_sub(1),
                // JumpEntry.col is a CHAR index (the landing above);
                // recording the raw byte column would send `M-,`
                // off-by-N on multibyte lines.
                col: hit_col,
                label: "search-RET".to_string(),
            };
            self.jump_stack.record_jump(&origin, &dest);
            // jump-highlight: the search-RET landing gets the same
            // landing highlight as every other jump (this path records
            // the jump stack directly, not through `record_jump`, so it
            // calls the shared helper itself — one rule, all jumps).
            self.record_landing_highlight(&dest);
            self.minibuffer_message(&format!("jumped to {file}:{line_no}"));
        }
    }

    /// The CHAR index of the byte offset `byte_col` within `line` (the
    /// point's `col` unit). A byte offset past the line's end lands at
    /// the line's end (the caller's `set_point` clamps to the CURRENT
    /// line's length anyway).
    fn byte_col_to_char_col(line: &str, byte_col: u64) -> usize {
        let mut rest = byte_col as usize;
        let mut col = 0;
        for c in line.chars() {
            if rest < c.len_utf8() {
                break;
            }
            rest -= c.len_utf8();
            col += 1;
        }
        col
    }

    /// The match-highlight context after a search-results jump (issue
    /// match-highlight): ONLY the current buffer's hits become ranges — a
    /// hit in another file must not highlight here. Each range is
    /// `(line start byte + hit's byte column, + the query's byte length)`
    /// in buffer-absolute bytes (fixed-string hits carry a column; regex
    /// hits carry none and get no range — no guessing). The jumped hit is
    /// the selected one; when its own column was undeterminable nothing is
    /// selected (an out-of-range index), so no range wears the prominent
    /// face by mistake.
    fn set_match_context_from_jump(&mut self, sel: usize) {
        let Some(key) = self.buffers.current().map(String::from) else {
            return;
        };
        let Some(rel) = self.buffer_annotation_path(&key) else {
            return;
        };
        let query_len = self.search.query.len();
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        // Out of range = none selected. This must NOT be derived from
        // `ranges`, which is empty here — that would always yield 0, and
        // when the jumped hit is skipped below (`col: None` from a
        // case-insensitive literal match, or an unreadable line) the cursor
        // is on no range at all, so the FIRST sibling range would wrongly
        // wear the prominent face (gate P2).
        let mut selected = usize::MAX;
        for (i, h) in self.search.hits.iter().enumerate() {
            if h.file != rel {
                continue; // hits in OTHER buffers never highlight here
            }
            let Some(col) = h.col else {
                continue; // undeterminable column (non-literal regex): no range
            };
            let line_idx = h.line_no.saturating_sub(1) as usize;
            let Some(line_start) = self
                .buffers
                .get(&key)
                .and_then(|b| b.try_line_to_byte(line_idx))
            else {
                continue;
            };
            let start = line_start + col as usize;
            if i == sel {
                selected = ranges.len();
            }
            ranges.push((start, start + query_len));
        }
        // `hits` is sorted (path, line, col) after `search_sort_hits`, so
        // this buffer's ranges are already start-ascending.
        self.match_context = MatchContext {
            buffer_key: key,
            query: self.search.query.clone(),
            ranges,
            selected,
        };
    }

    /// The visible window of results-view rows, pre-computed for the UI:
    /// (rows, scroll top, total rows, selected hit's row relative to the
    /// window). The window keeps the selected hit visible.
    pub fn search_view_info(&self) -> (Vec<ResultRow>, usize, usize, Option<usize>) {
        let s = &self.search;
        let total = s.rows.len();
        let viewport = self.viewport_lines.max(1);
        let mut scroll = s.scroll.min(total.saturating_sub(1));
        if let Some(&row) = s.hit_rows.get(s.selected) {
            if row < scroll {
                scroll = row;
            } else if row >= scroll + viewport {
                scroll = row - viewport + 1;
            }
            scroll = scroll.min(total.saturating_sub(1));
        }
        if total == 0 {
            return (Vec::new(), 0, 0, None);
        }
        let end = (scroll + viewport).min(total);
        let rows = s.rows[scroll..end].to_vec();
        let selected_row = s
            .hit_rows
            .get(s.selected)
            .copied()
            .filter(|r| *r >= scroll && *r < end)
            .map(|r| r - scroll);
        (rows, scroll, total, selected_row)
    }

    /// The results-view title: kind, query, running counts.
    pub fn search_title(&self) -> String {
        let s = &self.search;
        let files = s.file_row.len();
        let running = if s.running { " (searching…)" } else { "" };
        if s.cancelled {
            format!(
                "{}: '{}' — {} matches in {} files (cancelled){}",
                s.kind.label(),
                s.query,
                s.hits.len(),
                files,
                running
            )
        } else {
            format!(
                "{}: '{}' — {} matches in {} files{}",
                s.kind.label(),
                s.query,
                s.hits.len(),
                files,
                running
            )
        }
    }

    /// The status-line search indicator (empty when no search is
    /// running) — the async-activity slot from issue 01.
    pub fn search_display(&self) -> String {
        if self.search.running {
            "searching".to_string()
        } else {
            String::new()
        }
    }

    /// True while a search job is in flight.
    pub fn search_running(&self) -> bool {
        self.search.running
    }

    /// The last search error (when the most recent job failed to start).
    pub fn search_error(&self) -> Option<String> {
        self.search.error.clone()
    }

    /// `C-c p s s` (project search) / `M-s o` (occur): enter the query
    /// prompt mode (the minibuffer echoes the growing query).
    pub fn search_prompt_start(&mut self, kind: SearchPromptKind) {
        self.search_prompt = Some(SearchPrompt {
            kind,
            query: String::new(),
        });
        self.minibuffer_message(kind.label());
    }

    pub(super) fn search_prompt_char(&mut self, c: char) {
        let msg = match self.search_prompt.as_mut() {
            Some(p) => {
                p.query.push(c);
                format!("{}{}", p.kind.label(), p.query)
            }
            None => return,
        };
        self.minibuffer_message(&msg);
    }

    pub(super) fn search_prompt_backspace(&mut self) {
        let msg = match self.search_prompt.as_mut() {
            Some(p) => {
                p.query.pop();
                format!("{}{}", p.kind.label(), p.query)
            }
            None => return,
        };
        self.minibuffer_message(&msg);
    }

    pub(super) fn search_prompt_cancel(&mut self) {
        self.search_prompt.take();
        // A cancel gesture: the highlight goes with it (issue
        // match-highlight's lifetime rule).
        self.match_context = MatchContext::default();
        self.minibuffer_message("cancel");
    }

    pub(super) fn search_prompt_confirm(&mut self) {
        let Some(p) = self.search_prompt.take() else {
            return;
        };
        if p.query.is_empty() {
            self.search_prompt = Some(p);
            self.minibuffer_message("empty search");
            return;
        }
        match p.kind {
            SearchPromptKind::Project => self.start_project_search(p.query),
            SearchPromptKind::Occur => self.start_occur(p.query),
        }
    }

    /// Begin a new search job: cancel the previous job, bump the
    /// generation, reset the results state, spawn, and land in the
    /// results view (unless it is already on top).
    fn begin_search(&mut self, kind: SearchKind, query: String, spawn: impl FnOnce(&SearchBus, usize, Arc<AtomicBool>)) {
        self.cancel_search_job();
        // Issue match-highlight's lifetime rule: a DIFFERENT search clears
        // the old highlight context; re-running the SAME query keeps it
        // (the ranges are the query's, and a re-run re-derives the
        // selection on the next jump).
        if !self.match_context.ranges.is_empty() && self.match_context.query != query {
            self.match_context = MatchContext::default();
        }
        self.search_generation += 1;
        let generation = self.search_generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.search = SearchState {
            kind,
            query,
            running: true,
            cancelled: false,
            error: None,
            generation,
            hits: Vec::new(),
            rows: Vec::new(),
            hit_rows: Vec::new(),
            file_row: HashMap::new(),
            selected: 0,
            scroll: 0,
            cancel: Some(cancel.clone()),
        };
        let bus = self.search_bus.clone();
        spawn(&bus, generation, cancel);
        if self.top_view() != ViewId::Search {
            self.push_view(ViewId::Search);
        }
    }

    /// `C-c p s s`: project-wide search. The query is a LITERAL with
    /// smart case (redline's keymap has no prefix-arg mechanism, so
    /// projectile's "prefix = regexp" is not available; the pipeline's
    /// regex mode is used by `M-s o` and `M-?`).
    pub fn start_project_search(&mut self, query: String) {
        let Some(project) = self.project.clone() else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        let root = project.root.clone();
        let msg = format!("search: {query}");
        self.begin_search(SearchKind::Project, query.clone(), move |bus, generation, cancel| {
            let cfg = SearchConfig {
                root,
                pattern: query,
                word: false,
                fixed: true,
                case_smart: true,
                case_insensitive: false,
                glob: None,
                file_type: None,
                filter: None,
                cancel,
            };
            rg::spawn(cfg, bus, generation);
        });
        self.minibuffer_message(&msg);
    }

    /// `M-?`: references to the symbol under point. The buffer model has
    /// no column cursor yet, so point's column is 0; `symbol_under_point`
    /// takes the column and prefers the identifier at/preceding it.
    pub fn references_at_point(&mut self) {
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let Some(buf) = self.buffers.get(&key) else {
            self.minibuffer_message("no buffer");
            return;
        };
        if buf.path.is_none() {
            self.minibuffer_message("no file (scratch buffer)");
            return;
        }
        if self.project.is_none() {
            self.minibuffer_message("no project");
            return;
        }
        let line = self.point_line();
        let col = self.point_col(); // the point's column (plan 004 issue 05b)
        let line_text = buf.line_text(line).unwrap_or_default().to_string();
        let index = &self.index;
        let known = move |id: &str| !index.definitions_of(id).is_empty();
        let Some(symbol) = references::symbol_under_point(&line_text, col, known) else {
            self.minibuffer_message("no symbol under point");
            return;
        };
        self.start_references_search(symbol.to_string());
    }

    /// Start a references search for `symbol` (word-boundary, fixed,
    /// case-sensitive, token-class filtered via `references_filter`).
    pub fn start_references_search(&mut self, symbol: String) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        let msg = format!("references: {symbol}");
        self.begin_search(SearchKind::References, symbol.clone(), move |bus, generation, cancel| {
            let mut cfg = references::references_config(root, symbol);
            cfg.cancel = cancel;
            references::spawn_references(cfg, bus, generation);
        });
        self.minibuffer_message(&msg);
    }

    /// `M-s o`: occurrences of `query` (a regex, smart case) in the
    /// current buffer — the pipeline scoped to the buffer's text.
    pub fn start_occur(&mut self, query: String) {
        // 06a review P2: no scratch fallback — the home state has no buffer;
        // the honest "no buffer" echo is the whole behavior.
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let Some(buf) = self.buffers.get(&key) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let display = self.buffer_display(&key);
        let text = buf.text();
        let msg = format!("occur: {query}");
        self.begin_search(SearchKind::Occur, query.clone(), move |bus, generation, cancel| {
            occur::spawn_occur(display, text, query, cancel, bus.sender(), generation);
        });
        self.minibuffer_message(&msg);
    }

    /// Install a search event into the store (called by the UI's
    /// SearchBus drain). Events from a stale generation (a superseded
    /// job, or a project switch) are discarded.
    pub fn apply_search_event(&mut self, event: &SearchEvent) {
        if event.generation() != self.search.generation {
            return; // stale job
        }
        match event {
            SearchEvent::Hit { file, line_no, col, line, .. } => {
                let s = &mut self.search;
                if !s.file_row.contains_key(file) {
                    let row_idx = s.rows.len();
                    s.rows.push(ResultRow::Header {
                        file: file.clone(),
                        count: 0,
                        final_count: false,
                    });
                    s.file_row.insert(file.clone(), row_idx);
                }
                let hit = Hit {
                    file: file.clone(),
                    line_no: *line_no,
                    col: *col,
                    line: line.clone(),
                };
                let hit_index = s.hits.len();
                let row_idx = s.rows.len();
                s.rows.push(ResultRow::Hit { hit: hit.clone(), hit_index });
                s.hit_rows.push(row_idx);
                s.hits.push(hit);
                if let Some(&hr) = s.file_row.get(file)
                    && let ResultRow::Header { count, .. } = &mut s.rows[hr]
                {
                    *count += 1;
                }
            }
            SearchEvent::FileDone { file, hits, .. } => {
                let s = &mut self.search;
                if let Some(&hr) = s.file_row.get(file)
                    && let ResultRow::Header { count, final_count, .. } = &mut s.rows[hr]
                {
                    *count = *hits; // the authoritative final count
                    *final_count = true;
                }
            }
            SearchEvent::Finished { cancelled, .. } => {
                self.search.running = false;
                self.search.cancelled = *cancelled;
                self.search.cancel = None;
                // The parallel walk delivers hits in completion order;
                // normalize to a deterministic (path, line, col) order so
                // the results view is stable between runs. Streaming order
                // before Finished stays as-arrived (live feedback), the
                // final view is sorted.
                if !*cancelled {
                    self.search_sort_hits();
                }
                let n = self.search.hits.len();
                let m = self.search.file_row.len();
                if *cancelled {
                    self.minibuffer_message(&format!("search cancelled ({} matches so far)", n));
                } else {
                    self.minibuffer_message(&format!("{} matches in {} files", n, m));
                }
            }
            SearchEvent::Error { message, .. } => {
                self.search.running = false;
                self.search.error = Some(message.clone());
                self.search.cancel = None;
                self.minibuffer_message(&format!("search error: {message}"));
            }
        }
    }

    /// Reorder hits + display rows into deterministic (path, line, col)
    /// order. Preserves per-file counts and header positions; the selection
    /// resets to the first hit in display order (a fresh result set presents
    /// its first result, not the arrival-order artifact).
    fn search_sort_hits(&mut self) {
        let s = &mut self.search;
        if s.hits.len() < 2 {
            return;
        }
        s.hits.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then(a.line_no.cmp(&b.line_no))
                .then(a.col.cmp(&b.col))
        });

        // Rebuild rows: group by file in the new order, reusing the header
        // metadata (counts, final_count) from the old file_row table.
        let old_file_row = s.file_row.clone();
        let mut header_of: HashMap<String, (u64, bool)> = HashMap::new();
        for (file, &row) in &old_file_row {
            if let ResultRow::Header { count, final_count, .. } = &s.rows[row] {
                header_of.insert(file.clone(), (*count, *final_count));
            }
        }
        s.rows.clear();
        s.file_row.clear();
        s.hit_rows.clear();
        let mut last_file: Option<String> = None;
        for (hit_idx, hit) in s.hits.iter().enumerate() {
            if last_file.as_deref() != Some(hit.file.as_str()) {
                let (count, final_count) =
                    header_of.get(&hit.file).cloned().unwrap_or((0, false));
                s.file_row.insert(hit.file.clone(), s.rows.len());
                s.rows.push(ResultRow::Header {
                    file: hit.file.clone(),
                    count,
                    final_count,
                });
                last_file = Some(hit.file.clone());
            }
            s.hit_rows.push(s.rows.len());
            s.rows.push(ResultRow::Hit { hit: hit.clone(), hit_index: hit_idx });
        }
        // A fresh result set selects the first hit in display order.
        s.selected = 0;
        // Clamp the scroll anchor into the new range.
        if s.scroll >= s.rows.len() {
            s.scroll = s.rows.len().saturating_sub(1);
        }
    }
}

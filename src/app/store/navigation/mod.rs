mod definitions;
mod imports;
mod xref;

use super::*;
// (jump-ambiguity) the shared xref row/landing helpers (definitions.rs):
// re-exported for the picker (its re-derivation + the tooling row) and
// for the navigation mod itself — they are `pub(in crate::app::store)`
// items of the (private) definitions module, so the picker cannot name
// the module path itself.
pub(in crate::app::store) use definitions::{
    tooling_candidate_name, tooling_landing_line, xref_location_candidate,
};

impl AppStore {
    /// Open an ABSOLUTE path as a READ-ONLY buffer (plan 006 issue 02):
    /// the tooling-resolver lands external sources (registry source dirs)
    /// that are NOT project files. Unlike `open_path` it never records the
    /// file in the project's recents, never touches the tree/file-walk
    /// state, and inserts with `editable = false` so an external source
    /// can never enter edit mode (per-session, like other external jumps).
    /// Returns the buffer key, or `None` when the file cannot be read.
    pub(super) fn open_external_path(&mut self, abs: &Path) -> Option<String> {
        let (rope, mtime) = load_file(abs).ok()?;
        let key = self.buffers.insert_rope(Some(abs.to_path_buf()), rope, mtime, false);
        // 006-02b item 1: remember that this key is an external (registry /
        // tooling) source — the ownership guard refuses edit mode + save
        // for exactly these buffers.
        self.external_buffers.insert(key.clone());
        self.buffers.set_current(&key);
        // 006-03b item 1: the landed crate is now the crate we are IN —
        // keep it MRU so it can never be the LRU eviction victim.
        self.bump_current_crate_recency();
        // Build (or update) the highlight for the new current buffer.
        self.ensure_highlight();
        self.normalize_top_view();
        Some(key)
    }

    /// Land a tooling-resolver result (plan 006 issue 02, jump-
    /// ambiguity) — the RET on the tooling row of the Xref picker: the
    /// source inside the workspace opens through the project-relative
    /// path (recents, tree follow); an external source opens READ-ONLY
    /// via `open_external_path` (never in the project recents / file
    /// walk). On a load failure the failure is reported and no jump is
    /// recorded. The origin is the jump entry captured when `M-.` was
    /// PRESSED (`xref_tooling_origin`, set in `start_symbol_resolution`)
    /// — the tooling path is asynchronous, so capturing it at RET time
    /// could record a moved point and `M-,` would return to the wrong
    /// place (the synchronous index picker captures at open/selection
    /// time, which is the same instant there).
    pub(in crate::app::store) fn land_tooling_resolved_source(&mut self, source: &ResolvedSource, symbol: &str) {
        let origin = self.xref_tooling_origin.clone();
        let (display, opened) = if let Some(project) = self.project.as_ref()
            && let Ok(rel) = source.file.strip_prefix(&project.root)
        {
            let rel = rel.to_string_lossy().into_owned();
            (rel.clone(), self.open_project_path(&source.file, &rel).is_ok())
        } else {
            (
                source.file.display().to_string(),
                self.open_external_path(&source.file).is_some(),
            )
        };
        if !opened {
            self.minibuffer_message(&format!(
                "no provider resolution for `{symbol}`: cannot open {display}"
            ));
            return;
        }
        // Land on the resolved line (1-based; when the provider could not
        // pin one, the top of the file — a provider emitting 0 is treated
        // as "no line", 006-02b item 6, so no underflow).
        let line = tooling_landing_line(source);
        self.set_point_line(line);
        self.recenter_landing();
        self.ensure_highlight();
        self.record_jump(origin, "M-.");
        self.minibuffer_message(&format!("jumped to {display}:{}", line + 1));
    }

    /// (jump-ambiguity) A tooling-resolver HIT joins the Xref picker
    /// instead of jumping silently: a tooling resolve is the weakest kind
    /// of answer — a "top of file" landing must read as a weak resolve
    /// (the marked `tooling` row, preselected), not a mystery. Keeps the
    /// 006-03 external-buffer registration (`start_crate_indexing` for
    /// `source.external` — off the input path, the LRU cap governs; the
    /// landing itself never blocks on the index) and the read-only
    /// external open (the RET's `land_tooling_resolved_source` branch).
    /// The index candidates for the same symbol (project or origin-crate
    /// index) join the list; `open_picker` starts at index 0, so RET
    /// accepts the tooling row in one keystroke.
    fn xref_tooling_resolve_to_picker(&mut self, source: &ResolvedSource, symbol: &str) {
        // 006-03: an external (registry / tooling) landing registers its
        // crate's source tree for background indexing.
        if source.external {
            self.start_crate_indexing(&source.source_root, &source.file);
        }
        self.xref_tooling_pending = Some((source.clone(), symbol.to_string()));
        self.xref_lookup_name = symbol.to_string();
        // The picker's index keying follows the request's origin: the
        // project index for the project path, the origin crate's index
        // for M-. inside an external buffer (006-03).
        self.xref_crate_root = self.xref_tooling_crate_root.clone();
        let candidates = self.xref_candidates();
        self.open_picker(PickerKind::Xref, "Definition: ", candidates);
    }

    /// Capture the current position as a `JumpEntry` (for use as the
    /// origin or destination in `record_jump`). None when no buffer is
    /// current (06a: no scratch fallback — the home state has no buffer).
    pub(super) fn current_jump_entry(&self) -> Option<JumpEntry> {
        let key = self.buffers.current().map(String::from)?;
        let line = self.point_line();
        Some(JumpEntry {
            buffer_key: key,
            line,
            col: self.point_col(),
            label: String::new(),
        })
    }

    /// Record a jump from the current position (captured as `origin` before
    /// navigation) to the new position (captured as `destination` after
    /// navigation). Truncates forward history.
    pub(super) fn record_jump(&mut self, origin: Option<JumpEntry>, label: &str) {
        // 06a review P1-3: with no current buffer (home state) there is no
        // origin or destination to record — the old SCRATCH_NAME fallback
        // created `*scratch*` origins on async resolver landings after the
        // last buffer was killed (then `M-,` created the buffer).
        let (Some(origin), Some(mut dest)) = (origin, self.current_jump_entry()) else {
            return;
        };
        dest.label = label.to_string();
        self.jump_stack.record_jump(&origin, &dest);
        // jump-highlight hook (choke point 1): every FORWARD jump funnels
        // through here (M-., the xref/annotations/imenu pickers) — the
        // callers navigate first and then record, so the landing is
        // complete and `dest` describes the destination. One rule, all
        // forward jumps, no per-command special cases.
        self.record_landing_highlight(&dest);
    }

    /// `M-,`: pop back to the prior position (line + column).
    pub fn jump_back(&mut self) {
        let entry = self.jump_stack.back().cloned();
        match entry {
            Some(entry) => self.navigate_to_entry(&entry),
            None => self.minibuffer_message("no jump-back"),
        }
    }

    /// `C-i`: walk forward in the jump stack.
    pub fn jump_forward(&mut self) {
        let entry = self.jump_stack.forward().cloned();
        match entry {
            Some(entry) => self.navigate_to_entry(&entry),
            None => self.minibuffer_message("no jump-forward"),
        }
    }

    /// Navigate to a jump entry: open the buffer (if needed) and scroll to
    /// the entry's line. The results-view sentinel (recorded by `RET` in
    /// the search view) returns to the results view instead: its `line`
    /// is the hit index to restore, `col` the scroll top.
    fn navigate_to_entry(&mut self, entry: &JumpEntry) {
        if entry.buffer_key == SEARCH_JUMP_KEY {
            let hits = self.search.hits.len();
            self.search.selected = entry.line.min(hits.saturating_sub(1));
            self.search.scroll = entry.col;
            if self.top_view() != ViewId::Search {
                self.push_view(ViewId::Search);
            }
            return;
        }
        // If the buffer is not open, report it — no accidental buffer
        // creation (06a review P1-1: the pre-06a fallback created `*scratch*`,
        // reachable from home state via a dead jump entry).
        if self.buffers.get(&entry.buffer_key).is_none() {
            self.minibuffer_message(&format!(
                "no buffer: {}",
                self.buffer_display(&entry.buffer_key)
            ));
            return;
        }
        self.buffers.set_current(&entry.buffer_key);
        // 006-03b item 1: a jump-back into an external buffer keeps its
        // owning crate MRU.
        self.bump_current_crate_recency();
        self.set_point(entry.line, entry.col, entry.col);
        self.recenter_landing();
        self.ensure_highlight();
        // Watchlist: `M-,` through the sentinel lands the pre-search
        // position — the results view must close so the landing is
        // visible (before, the buffer/point moved underneath the results
        // view: a no-op until the view was closed by hand).
        if self.top_view() == ViewId::Search {
            self.close_view();
        }
        // jump-highlight hook (choke point 2): the landing path for
        // `jump_back` (M-,) / `jump_forward` (C-i) — the buffer path only
        // (the SEARCH_JUMP_KEY sentinel returns to the results view, where
        // there is no buffer symbol, and the no-buffer early return above
        // never reaches here: neither sets a highlight).
        self.record_landing_highlight(entry);
    }

    /// jump-highlight: after a landing, compute the symbol at the landing
    /// point and store its BYTE range as the transient landing highlight
    /// (the one rule for every jump — called from BOTH choke points:
    /// `record_jump` for forward jumps, `navigate_to_entry` for M-,
    /// /C-i). The extent is the word-boundary run at the point (the
    /// `is_word_char` walk the word motions use), gated by the
    /// syntax-aware `symbol_at_point`: neither yielding a non-empty extent
    /// (a whitespace/punctuation landing) sets NO highlight — never an
    /// empty or whole-line range. Byte/char: `JumpEntry.col` is a 0-based
    /// CHAR index; the span layer is BYTE offsets, and the conversion
    /// happens inside `symbol_extent_at` (the recurring bug class here —
    /// a byte column would land off-by-N on multibyte lines). A missing
    /// buffer or line sets nothing.
    pub(super) fn record_landing_highlight(&mut self, entry: &JumpEntry) {
        let Some(buf) = self.buffers.get(&entry.buffer_key) else {
            return;
        };
        let Some(text) = buf.line_text(entry.line) else {
            return;
        };
        let text = &*text;
        let lang = self
            .grammar_registry
            .language_for(&buf.path.as_ref().map(|p| p.to_string_lossy()).unwrap_or_default());
        // A symbol must exist at the landing point (no-symbol landings set
        // no highlight).
        if AppStore::symbol_at_point(lang, text, entry.col).is_none() {
            return;
        }
        let Some((start, end)) = symbol_extent_at(text, entry.col) else {
            return;
        };
        self.jump_highlight = Some(LandingHighlight {
            buffer_key: entry.buffer_key.clone(),
            line: entry.line,
            start,
            end,
            set_at: std::time::Instant::now(),
        });
        // Wake the animation driver (the Root hook). `try_send` (NOT the
        // async `send` — the store's landing paths are SYNC, and the
        // async `send`'s future would drop without waking the driver).
        // The capacity-1 channel coalesces rapid jump bursts; a full
        // channel (a wake already queued) needs no second wake — the
        // driver re-reads the latest `set_at` when it runs.
        // Latent hazard (P2-4, recorded not fixed): if `Root`'s
        // `use_future` is ever dropped or aborted, `try_send` fails
        // silently forever — the same silent-drop class this lane fixed
        // with `try_send`. Self-healing: the band still ends at the next
        // render (intensity 0 ⇒ no highlight) or the next command clear.
        // A `watch` channel or receiver re-install is only worth it if
        // `Root` re-mounting becomes possible.
        let _ = self.jump_wake_tx.try_send(());
    }
}

/// jump-highlight: the BYTE range (line-relative) of the word-boundary run
/// at char column `col` — the same `is_word_char` walk the word motions
/// use: a word char AT the point owns the run, else the run ending
/// immediately BEFORE it (a cursor parked just after the name — the usual
/// call-site spot — still belongs to it). `None` for whitespace / 
/// punctuation / no-run points. The char indices are converted to BYTE
/// offsets (the span layer's domain) explicitly — the char→byte
/// translation the whole recurring bug class demands.
pub(in crate::app::store) fn symbol_extent_at(text: &str, col: usize) -> Option<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    if col > chars.len() {
        return None;
    }
    let start = if col < chars.len() && is_word_char(chars[col]) {
        let mut i = col;
        while i > 0 && is_word_char(chars[i - 1]) {
            i -= 1;
        }
        i
    } else if col > 0 && is_word_char(chars[col - 1]) {
        let mut i = col - 1;
        while i > 0 && is_word_char(chars[i - 1]) {
            i -= 1;
        }
        i
    } else {
        return None;
    };
    let mut end = start;
    while end < chars.len() && is_word_char(chars[end]) {
        end += 1;
    }
    if end <= start {
        return None;
    }
    // Char indices → byte offsets (the span layer's domain).
    let start_byte = chars[..start].iter().map(|c| c.len_utf8()).sum::<usize>();
    let end_byte = start_byte + chars[start..end].iter().map(|c| c.len_utf8()).sum::<usize>();
    Some((start_byte, end_byte))
}

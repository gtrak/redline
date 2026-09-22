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
        // App-side refinement the `ResolvedSource.line` doc promises
        // (jump-column-landings): a provider pins a LINE but never a
        // column — locate the resolved item's name on that line (its first
        // whole-word occurrence outside a comment, string, or raw string,
        // P2-3) and land
        // on its first char column (the item is the path's last segment,
        // `tokio::spawn` → `spawn`), so an M-. on a function lands on the
        // name, not the line start. A whole-word match only (never
        // `respawn`/`spawned`); the scan is char-based, so a multibyte
        // prefix yields a CHAR column, not a byte offset. The honest col-0
        // fallback when the name is not a whole word on the line (or only
        // appears inside a comment/string) — never an invented column.
        let line_text = self
            .buffers
            .current_buffer()
            .and_then(|b| b.line_text(line).map(|t| t.into_owned()))
            .unwrap_or_default();
        let item = symbol
            .rsplit([':', '.'])
            .find(|seg| !seg.is_empty())
            .unwrap_or(symbol);
        let col = AppStore::first_word_column(&line_text, item).unwrap_or(0);
        self.set_point(line, col, col);
        self.recenter_landing();
        self.ensure_highlight();
        self.record_jump(origin, "M-.");
        self.minibuffer_message(&format!("jumped to {display}:{}", line + 1));
    }

    /// (jump-column-landings) The 0-based CHAR column of the first whole-word
    /// occurrence of `item` on `line_text` OUTSIDE a masked comment/string —
    /// the app-side refinement for a tooling landing whose provider pinned a
    /// line but no column. A word char immediately before or after `item`
    /// disqualifies that occurrence (`spawn` never matches `respawn`/
    /// `spawned`), so the landing sits on the definition's own name, not a
    /// substring inside a longer identifier. A leading inline block comment,
    /// string, or raw string that mentions the name (`/* spawn */ pub fn
    /// spawn()`, `let s = "spawn"; fn spawn()`, `let s = r#"spawn"#; fn
    /// spawn()`) is skipped so the landing sits on the live definition's name,
    /// not the mention. `None` when `item` is empty or no whole-word
    /// occurrence sits outside a masked region. The scan is char-based, so a
    /// multibyte prefix yields a CHAR column, not a byte offset.
    ///
    /// HONEST SCOPE (single-line, best-effort): the mask covers constructs
    /// that BEGIN on this line (a `/* … */` block comment — Rust's NESTED
    /// form, where a `/*` inside deepens it and the matching `*/` (depth 0)
    /// closes the run; a `"…"`/backtick string with escapes; a `r…"` raw
    /// string; a `'…'` char literal; and a block-comment CONTINUATION, via a
    /// leading `*/` whose prefix is plain comment text). It does NOT cover a
    /// `//` or `#` line comment (deliberately, matching base) nor a
    /// string/comment that BEGAN on a prior line (a multi-line string
    /// continuation, for example), and a CONTINUATION whose prefix holds a
    /// `'` or `"` (prose punctuation, e.g. `it's spawn */ …`) — the proxy
    /// declines that, so the leading mention can be returned. So a mention the
    /// scan UNDER-masks can still be returned — a WRONG column, not col 0; an
    /// OVER-mask that covers the definition (the name occurrence the landing
    /// would use) yields `None` → col 0, whether it ends mid-line or runs to
    /// the line's end (an unterminated construct) — PROVIDED no earlier
    /// occurrence sits outside the mask, in which case that earlier one is
    /// returned instead.
    pub(in crate::app::store) fn first_word_column(line_text: &str, item: &str) -> Option<usize> {
        if item.is_empty() {
            return None;
        }
        let chars: Vec<char> = line_text.chars().collect();
        let target: Vec<char> = item.chars().collect();
        let n = target.len();
        if n > chars.len() {
            return None;
        }
        let mask = Self::comment_or_string_mask(&chars);
        for i in 0..=chars.len() - n {
            if chars[i..i + n] != target[..] {
                continue;
            }
            let before_ok = i == 0 || !is_word_char(chars[i - 1]);
            let j = i + n;
            let after_ok = j >= chars.len() || !is_word_char(chars[j]);
            // Every char of the name must sit outside a comment/string region
            // (the name is a contiguous word, so its start decides, but a
            // full-range check is the belt-and-braces form).
            let clean = !(i..j).any(|k| mask[k]);
            if before_ok && after_ok && clean {
                return Some(i);
            }
        }
        None
    }

    /// (jump-column-landings) For each char index on `chars`, `true` when that
    /// char lies inside a construct this single-line, best-effort mask models:
    ///  - an inline block comment `/* … */` — Rust's NESTED form (a `/*`
    ///    inside deepens it; the matching `*/` at depth 0 closes the run); an
    ///    unterminated one extends to end of line;
    ///  - a `"…"` or backtick… string (a backslash escapes the next char);
    ///  - a raw string `r"…"`, `r#"…"#`, `r##"…"##` (closer = the quote
    ///    followed by the same number of `#`; raw strings take no escapes);
    ///  - a `'…'` char literal — a lifetime tick (`'static`, `'a`) is NOT a
    ///    string (it passes through);
    ///  - the leading run of a block-comment CONTINUATION — a `*/` whose
    ///    prefix on this line has no `/*`, `"`, `'`, backtick, or `//` (so it
    ///    is plain comment text closing a prior-line comment); a `*/` inside
    ///    a string or `//` comment is NOT a continuation and is left alone.
    ///
    /// Deliberately NOT modeled (a reader who trusts this doc should know): `//`
    /// and `#` line comments, and a string or comment that BEGAN ON A PRIOR
    /// line (a multi-line string's continuation line, for example) — those can
    /// under-mask a leading mention, which `first_word_column` then returns as
    /// a wrong column (its honest-scope note is the source of truth).
    fn comment_or_string_mask(chars: &[char]) -> Vec<bool> {
        let n = chars.len();
        let mut mask = vec![false; n];
        let mut i = 0;
        // A line that CONTINUES a block comment opened on a prior line shows a
        // `*/` whose prefix is plain comment text (no `/*`, `"`, `'`, backtick,
        // or `//` before it): mask the leading run through the close, then scan
        // normally from after it. A `*/` inside a string or `//` comment is NOT
        // a continuation and is left to the main scan. (A leading string
        // continuation has no such marker and is not detectable here — see the
        // doc above.)
        {
            let mut k = 0;
            let mut lead_close: Option<usize> = None;
            while k + 1 < n {
                if chars[k] == '*' && chars[k + 1] == '/' {
                    lead_close = Some(k);
                    break;
                }
                // A `/*` opener or a `//` line comment before the `*/` means it
                // is not a plain-continuation `*/` → do not mask the prefix.
                if chars[k] == '/' && (chars[k + 1] == '*' || chars[k + 1] == '/') {
                    break;
                }
                // A string opener before the `*/` means the `*/` sits inside
                // that string (e.g. `let s = "*/";`) → not a continuation.
                if chars[k] == '"' || chars[k] == '\'' || chars[k] == '`' {
                    break;
                }
                k += 1;
            }
            if let Some(c) = lead_close {
                mask[0..c + 2].fill(true);
                i = c + 2;
            }
        }
        while i < n {
            // Inline block comment: `/* … */`. Rust's block comments NEST — a
            // `/*` inside deepens the run and only the matching `*/` (depth 0)
            // closes it; an unterminated one extends to end of line.
            if i + 1 < n && chars[i] == '/' && chars[i + 1] == '*' {
                let start = i;
                i += 2;
                let mut depth = 1;
                let closed = loop {
                    if i + 1 < n {
                        if chars[i] == '/' && chars[i + 1] == '*' {
                            depth += 1; // a nested `/*` — the comment continues
                            i += 2;
                            continue;
                        }
                        if chars[i] == '*' && chars[i + 1] == '/' {
                            depth -= 1;
                            i += 2;
                            if depth == 0 {
                                break true; // the matching `*/` closed the run
                            }
                            continue;
                        }
                    }
                    if i >= n {
                        break false;
                    }
                    i += 1;
                };
                mask[start..i.min(n)].fill(true);
                if !closed {
                    mask[start..n].fill(true);
                    break;
                }
                continue;
            }
            // String literal: `"…"`, backtick…, or a RAW string (the `#`s
            // directly before the quote, with an `r` before them, mark the raw
            // form; its closer is the quote + the same `#`s, and it takes no
            // escapes). Plain `"…"`/backtick strings take a backslash escape.
            if chars[i] == '"' || chars[i] == '`' {
                let quote = chars[i];
                let hashes = Self::leading_hashes(chars, i);
                let is_raw = quote == '"' && i > hashes && chars[i - 1 - hashes] == 'r';
                if is_raw {
                    let start = i - 1 - hashes;
                    let mut j = i + 1;
                    while j + 1 + hashes <= n
                        && !(chars[j] == '"'
                            && (0..hashes).all(|d| chars[j + 1 + d] == '#'))
                    {
                        j += 1;
                    }
                    i = if j + 1 + hashes <= n { j + 1 + hashes } else { n };
                    mask[start..i.min(n)].fill(true);
                    continue;
                }
                let start = i;
                i += 1; // past the opening quote
                loop {
                    if i >= n {
                        // Unterminated: the rest of the line is string.
                        mask[start..n].fill(true);
                        break;
                    }
                    if chars[i] == '\\' {
                        i += 2; // backslash escapes the next char (`"` and backtick)
                        continue;
                    }
                    if chars[i] == quote {
                        i += 1; // past the closing quote
                        break;
                    }
                    i += 1;
                }
                mask[start..i].fill(true);
                continue;
            }
            // Char literal `'…'` — but a LIFETIME tick (`'static`, `'a`) is NOT
            // a string: only mask when the `'` clearly opens a char literal
            // (`'X'` or `\'…'`), else pass it through (the `'static`/`'a`
            // regression: a bare `'` used to open a string that ran to the
            // next `'`, over-masking the live definition to col 0).
            if chars[i] == '\'' {
                let is_char_lit = if i + 2 < n {
                    if chars[i + 1] == '\\' {
                        i + 3 < n && chars[i + 3] == '\'' // '\x' / '\''
                    } else {
                        chars[i + 1] != '\'' && chars[i + 2] == '\'' // 'X'
                    }
                } else {
                    false
                };
                if is_char_lit {
                    let start = i;
                    i += if chars[i + 1] == '\\' { 4 } else { 3 };
                    mask[start..i].fill(true);
                    continue;
                }
                // lifetime tick / stray ' — not a string; fall through to i += 1
            }
            i += 1;
        }
        mask
    }

    /// (jump-column-landings) The number of `#` chars immediately BEFORE
    /// position `i` (scanning backwards). A raw-string opener (`r#*"`) has one
    /// or more `#` directly before the quote.
    fn leading_hashes(chars: &[char], i: usize) -> usize {
        let mut k = 0;
        let mut p = i;
        while p > 0 && chars[p - 1] == '#' {
            p -= 1;
            k += 1;
        }
        k
    }

    /// (jump-column-landings) The index-recorded `start_byte` (an absolute
    /// file byte offset) of the symbol `(symbol_name, line)` in the picker's
    /// current index: the project index when `crate_root` is `None`, the
    /// crate's index otherwise. `None` when no such symbol is in the index
    /// (a stale index after an external edit) — the caller degrades to
    /// column 0. Matching the NAME as well as the line (P2-2) is what keeps
    /// a line that hosts two symbols (`fn a() {} fn b() {}`) from always
    /// `Symbol.start_byte` via `try_byte_to_line_col` (see the picker arms);
    /// the imenu path already matches name AND line.
    /// Inherent limit: a line that hosts the SAME name twice (`fn b() {} fn
    /// b() {}`) still lands on the FIRST — the picker row carries only
    /// `file:line` + the name, so two identical names on one line cannot be
    /// disambiguated here (a re-read of the same index cannot break the tie).
    /// Re-reading the index uses the SAME byte the candidate row was built
    /// from (no re-derivation of a position, no file re-parse) — the picker
    /// candidate carries only the line ("file:line"), so the byte is
    /// fetched, not re-derived.
    pub(in crate::app::store) fn definition_start_byte(
        &mut self,
        crate_root: Option<&Path>,
        file: &str,
        line: usize,
        symbol_name: &str,
    ) -> Option<usize> {
        match crate_root {
            Some(root) => self
                .crate_index_arc(root)
                .and_then(|arc| {
                    arc.lock()
                        .unwrap()
                        .outline(file)
                        .iter()
                        .find(|s| s.line == line && s.name == symbol_name)
                        .map(|s| s.start_byte)
                }),
            None => self
                .index
                .outline(file)
                .iter()
                .find(|s| s.line == line && s.name == symbol_name)
                .map(|s| s.start_byte),
        }
    }

    /// (jump-column-landings) Translate an index-recorded `start_byte` into
    /// the 0-based CHAR column of the name on its own line via the current
    /// buffer's byte→(line,char) conversion (`try_byte_to_line_col` — the
    /// byte→char step keeps it a CHAR column, not a raw byte offset). `0`
    /// when the byte is out of range of the current buffer (a stale index)
    /// or no buffer is current — the honest col-0 degradation, never an
    /// invented column.
    pub(in crate::app::store) fn landing_column_from_start_byte(
        &self,
        start_byte: Option<usize>,
    ) -> usize {
        start_byte
            .and_then(|byte| {
                self.buffers
                    .current_buffer()
                    .and_then(|b| b.try_byte_to_line_col(byte))
            })
            .map(|(_, col)| col)
            .unwrap_or(0)
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

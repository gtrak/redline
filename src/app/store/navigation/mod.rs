mod definitions;
mod imports;
mod xref;

use super::*;

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

    /// Land a tooling-resolver result (plan 006 issue 02): open the resolved
    /// source and record a jump like any M-. landing. A source inside the
    /// workspace opens through the project-relative path (recents, tree
    /// follow); an external source opens READ-ONLY via
    /// `open_external_path` (never in the project recents / file walk). On a
    /// load failure the failure is reported and no jump is recorded.
    fn open_resolved_source(&mut self, source: &ResolvedSource, symbol: &str) {
        let origin = self.current_jump_entry();
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
        // pin one, the top of the file) and record the jump. A provider
        // emitting 0 is treated as "no line" (006-02b item 6) so the
        // `(l - 1) as usize` below can never underflow.
        let line = source
            .line
            .filter(|l| *l > 0)
            .map(|l| (l - 1) as usize)
            .unwrap_or(0);
        // 006-03: an external (registry / tooling) landing registers its
        // crate's source tree for background indexing (off the input
        // path; the LRU cap governs) so M-. / imenu work INSIDE it. The
        // landing itself is never blocked on the index.
        if source.external {
            self.start_crate_indexing(&source.source_root, &source.file);
        }
        self.set_point_line(line);
        self.recenter_landing();
        self.ensure_highlight();
        self.record_jump(origin, "M-.");
        self.minibuffer_message(&format!("jumped to {display}:{}", line + 1));
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
    }
}

use super::*;

impl AppStore {
    /// `l` in the magit-status context: open (or re-focus) the log for the
    /// current branch (HEAD when detached/unborn).
    pub fn open_log(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        if !self.ensure_git(root) {
            return;
        }
        let branch = self
            .with_git(|g| g.branch())
            .ok()
            .and_then(|b| if !b.detached && !b.unborn { Some(b.name) } else { None });
        let total = self
            .with_git(|g| g.log_total(branch.as_deref()))
            .unwrap_or(0);
        let entries = self
            .with_git(|g| g.log(branch.as_deref(), 0, LOG_PAGE))
            .unwrap_or_default();
        self.log = Some(LogState {
            branch,
            offset: 0,
            limit: LOG_PAGE,
            total,
            entries,
            selected: 0,
        });
        self.log_scroll = 0;
        if self.top_view() != ViewId::Log {
            self.push_view(ViewId::Log);
        }
        self.minibuffer_message(&format!("log: {} ({} commits)", self.log.as_ref().unwrap().display(), total));
    }

    /// `n` in the log: move to the next page.
    pub fn log_next_page(&mut self) {
        let Some(log) = self.log.as_ref() else { return };
        if log.offset + log.entries.len() >= log.total {
            self.minibuffer_message("end of log");
            return;
        }
        self.log_offset_to(log.offset + log.limit);
    }

    /// `p` in the log: move to the previous page.
    pub fn log_prev_page(&mut self) {
        let Some(log) = self.log.as_ref() else { return };
        if log.offset == 0 {
            self.minibuffer_message("start of log");
            return;
        }
        self.log_offset_to(log.offset.saturating_sub(log.limit));
    }

    fn log_offset_to(&mut self, offset: usize) {
        let Some(log) = self.log.as_ref() else { return };
        let branch = log.branch.clone();
        let limit = log.limit;
        let total = self.with_git(|g| g.log_total(branch.as_deref())).unwrap_or(0);
        let entries = self
            .with_git(|g| g.log(branch.as_deref(), offset, limit))
            .unwrap_or_default();
        if let Some(l) = self.log.as_mut() {
            l.offset = offset;
            l.total = total;
            l.entries = entries;
            l.selected = 0;
        }
        // A new page starts its in-page window at the top (issue 003-02).
        self.log_scroll = 0;
    }

    /// Move the in-page log selection down (arrows / j / C-n).
    pub fn log_move_down(&mut self) {
        if let Some(l) = self.log.as_mut() {
            l.selected = (l.selected + 1).min(l.entries.len().saturating_sub(1));
        }
        self.log_keep_visible();
    }

    /// Move the in-page log selection up (arrows / k / C-p).
    pub fn log_move_up(&mut self) {
        if let Some(l) = self.log.as_mut() {
            l.selected = l.selected.saturating_sub(1);
        }
        self.log_keep_visible();
    }

    /// `RET` in the log: open the selected commit's full tree diff read-only.
    pub fn log_open_commit(&mut self) {
        let Some(oid) = self
            .log
            .as_ref()
            .and_then(|l| l.entries.get(l.selected))
            .map(|e| e.short_id.clone())
        else {
            self.minibuffer_message("no commit at point");
            return;
        };
        match self.with_git(|g| g.commit_diff(&oid)) {
            Ok(diff) => {
                self.commit_diff = Some(CommitDiffState { diff });
                self.commit_diff_scroll = 0;
                if self.top_view() != ViewId::CommitDiff {
                    self.push_view(ViewId::CommitDiff);
                }
                self.minibuffer_message("commit diff");
            }
            Err(e) => self.minibuffer_message(&format!("commit diff failed: {e}")),
        }
    }

    /// Rebuild the open log's current page (after a commit or branch switch).
    fn refresh_log_page(&mut self) {
        let Some(log) = self.log.as_ref() else { return };
        let branch = log.branch.clone();
        let offset = log.offset;
        let limit = log.limit;
        let total = self.with_git(|g| g.log_total(branch.as_deref())).unwrap_or(0);
        let entries = self
            .with_git(|g| g.log(branch.as_deref(), offset, limit))
            .unwrap_or_default();
        if let Some(l) = self.log.as_mut() {
            l.total = total;
            l.entries = entries;
            l.selected = l.selected.min(l.entries.len().saturating_sub(1));
        }
        self.log_keep_visible();
    }

    /// `b` in the magit-status context: blame the current buffer's file
    /// (project-relative), one line per commit/author/age prefix.
    pub fn open_blame(&mut self) {
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let Some(path) = self.buffers.get(&key).and_then(|b| b.path.clone()) else {
            self.minibuffer_message("no file (scratch buffer)");
            return;
        };
        let Some(project) = self.project.as_ref() else {
            self.minibuffer_message("no project");
            return;
        };
        let Ok(rel) = path.strip_prefix(&project.root) else {
            // 006-03: registry / tooling sources are not git checkouts —
            // blame stays refused, but the message says WHY.
            if self.external_buffers.contains(&key) {
                self.minibuffer_message("no git history for external sources");
            } else {
                self.minibuffer_message("buffer not in project");
            }
            return;
        };
        let rel = rel.to_string_lossy().into_owned();
        let root = project.root.clone();
        if !self.ensure_git(root) {
            return;
        }
        match self.with_git(|g| g.blame(&rel)) {
            Ok(lines) => {
                self.blame = Some(BlameState {
                    path: rel.clone(),
                    lines,
                    selected: 0,
                });
                self.blame_scroll = 0;
                if self.top_view() != ViewId::Blame {
                    self.push_view(ViewId::Blame);
                }
                self.minibuffer_message(&format!("blame: {rel}"));
            }
            Err(e) => self.minibuffer_message(&format!("blame failed: {e}")),
        }
    }

    /// `c` in the magit-status context: open the inline commit editor, pre-
    /// filled with a comment block listing the staged files.
    pub fn open_commit_editor(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        if !self.ensure_git(root) {
            return;
        }
        let staged: Vec<(String, char)> = self
            .git_status()
            .unwrap_or_default()
            .files
            .into_iter()
            .filter(|f| f.is_staged())
            .map(|f| (f.path, f.staged.letter()))
            .collect();
        let (text, cursor) = prefill_commit_message(&staged);
        let staged_paths: Vec<String> = staged.iter().map(|(p, _)| p.clone()).collect();
        self.commit_editor = Some(CommitEditorState {
            rope: Rope::from(text.as_str()),
            cursor,
            staged: staged_paths,
        });
        if self.top_view() != ViewId::CommitEditor {
            self.push_view(ViewId::CommitEditor);
        }
        self.minibuffer_message("commit: C-c C-c commit · C-c C-k abort");
    }

    /// `C-c C-c` in the commit editor: extract the message (comment lines
    /// stripped) and commit the staged changes.
    pub fn commit_editor_commit(&mut self) {
        let Some(ed) = self.commit_editor.as_ref() else { return };
        let msg = extract_commit_message(&ed.rope);
        if msg.trim().is_empty() {
            self.minibuffer_message("empty commit message: add a line not starting with '#'");
            return;
        }
        // Refuse to commit when nothing is staged (magit convention: a
        // commit with no staged changes is an error, not a no-op).
        let staged = self
            .git_status()
            .unwrap_or_default()
            .files
            .iter()
            .filter(|f| f.is_staged())
            .count();
        if staged == 0 {
            self.minibuffer_message("nothing staged to commit");
            return;
        }
        let msg = msg.to_string();
        match self.with_git(move |g| g.commit(&msg)) {
            Ok(oid) => {
                let short: String = oid.chars().take(7).collect();
                self.commit_editor = None;
                if self.top_view() == ViewId::CommitEditor {
                    self.close_view();
                }
                self.refresh_magit();
                if self.log.is_some() {
                    self.refresh_log_page();
                }
                self.minibuffer_message(&format!("committed {short}"));
            }
            Err(e) => self.minibuffer_message(&format!("commit failed: {e}")),
        }
    }

    /// `C-c C-k` (and ESC) in the commit editor: discard the
    /// buffer and touch nothing in the repository.
    pub fn commit_editor_abort(&mut self) {
        if self.commit_editor.take().is_some() {
            if self.top_view() == ViewId::CommitEditor {
                self.close_view();
            }
            self.pending.clear();
            self.minibuffer_message("commit aborted (no changes made)");
        } else {
            self.minibuffer_message("no commit in progress");
        }
    }

    /// Insert a printable character at the commit-editor cursor.
    pub fn commit_editor_insert(&mut self, c: char) {
        if let Some(ed) = self.commit_editor.as_mut() {
            ed.rope.insert_char(ed.cursor, c);
            ed.cursor += 1;
        }
    }

    /// Backspace in the commit editor.
    pub fn commit_editor_backspace(&mut self) {
        if let Some(ed) = self.commit_editor.as_mut()
            && ed.cursor > 0
        {
            ed.rope.remove(ed.cursor - 1..ed.cursor);
            ed.cursor -= 1;
        }
    }

    /// Insert a newline at the commit-editor cursor (RET in the editor).
    pub fn commit_editor_newline(&mut self) {
        if let Some(ed) = self.commit_editor.as_mut() {
            ed.rope.insert_char(ed.cursor, '\n');
            ed.cursor += 1;
        }
    }

    /// Move the commit-editor cursor left / right / up / down.
    pub(super) fn commit_editor_move(&mut self, dir: EditorMove) {
        let Some(ed) = self.commit_editor.as_mut() else { return };
        match dir {
            EditorMove::Left => ed.cursor = ed.cursor.saturating_sub(1),
            EditorMove::Right => ed.cursor = (ed.cursor + 1).min(ed.rope.len_chars()),
            EditorMove::Up => editor_cursor_line(ed, -1),
            EditorMove::Down => editor_cursor_line(ed, 1),
        }
    }

    /// The commit editor's rows (message + comment lines; the cursor line is
    /// marked `selected` and carries a `←` marker).
    pub fn commit_editor_rows(&self) -> Vec<MagitRow> {
        let Some(ed) = self.commit_editor.as_ref() else { return Vec::new() };
        let text = ed.rope.to_string();
        let lines: Vec<&str> = text.lines().collect();
        let n_lines = ed.rope.len_lines();
        let cursor_line = ed.rope.char_to_line(ed.cursor.min(ed.rope.len_chars()));
        (0..n_lines)
            .map(|i| {
                let line = lines.get(i).copied().unwrap_or("");
                let role = if line.trim_start().starts_with('#') {
                    RowRole::Comment
                } else {
                    RowRole::Text
                };
                let selected = i == cursor_line;
                let text = if selected {
                    format!("{line} ←")
                } else {
                    line.to_string()
                };
                MagitRow { text, role, selected }
            })
            .collect()
    }

    /// issue-current-line-highlight (follow-up): the commit editor's cursor
    /// line index (within the row list; same index since rows are 1:1 with
    /// lines). `None` when the editor is closed.
    pub fn commit_editor_cursor_line(&self) -> Option<usize> {
        self.commit_editor.as_ref().map(|ed| {
            ed.rope.char_to_line(ed.cursor.min(ed.rope.len_chars()))
        })
    }

    /// `y` in the magit-status context: the local-branch picker (RET checks
    /// out the selected branch).
    pub fn open_branch_picker(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        if !self.ensure_git(root) {
            return;
        }
        let candidates = self.branch_candidates();
        if candidates.is_empty() {
            self.minibuffer_message("no local branches");
            return;
        }
        self.open_picker(PickerKind::Branch, "Branch: ", candidates);
    }

    /// Check out the local branch `name`. On success, refreshes the magit
    /// status (branch + dirty state), schedules a full symbol-index rebuild
    /// (the wholesale change), and refreshes the open log. A dirty tree is
    /// refused (`GitError::DirtyTree`) per magit's default.
    pub fn checkout_branch(&mut self, name: &str) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            self.minibuffer_message("no project");
            return;
        };
        if !self.ensure_git(root) {
            return;
        }
        match self.with_git(|g| g.checkout_branch(name)) {
            Ok(()) => {
                self.refresh_magit();
                // Wholesale change: full-rebuild the symbol index (05's
                // start_indexing path). Bump the generation and clear pending
                // changes (mirroring switch_project_root) so the full rebuild
                // isn't silently skipped by start_indexing's single-flight
                // guard when a current-generation job is already in flight.
                self.index = SymbolIndex::new();
                self.index_generation += 1;
                self.pending_index_changes.clear();
                // A resolve job in flight belonged to the pre-checkout tree:
                // its event is discarded (mirrors the index bump).
                self.resolve_generation += 1;
                self.resolve_in_flight = false; // P3-4: that request is dead
                self.start_indexing();
                if self.log.is_some() {
                    self.refresh_log_page();
                }
                self.minibuffer_message(&format!("checked out {name}"));
            }
            Err(e) => self.minibuffer_message(&format!("checkout failed: {e}")),
        }
    }

    /// Create a new local branch at HEAD. `branch-create` (M-x) opens a name
    /// prompt; this performs the creation.
    pub fn create_branch_from_head(&mut self, name: &str) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            self.minibuffer_message("no project");
            return;
        };
        if !self.ensure_git(root) {
            return;
        }
        if name.trim().is_empty() {
            self.minibuffer_message("empty branch name");
            return;
        }
        match self.with_git(|g| g.create_branch_from_head(name)) {
            Ok(()) => self.minibuffer_message(&format!("created branch {name} at HEAD")),
            Err(e) => self.minibuffer_message(&format!("create branch failed: {e}")),
        }
    }

    /// `z` in the magit-status context: the stash list (RET pops, `x` drops).
    pub fn open_stash_picker(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        if !self.ensure_git(root) {
            return;
        }
        let candidates = self.stash_candidates();
        if candidates.is_empty() {
            self.minibuffer_message("no stashes");
            return;
        }
        self.open_picker(PickerKind::Stash, "Stash: ", candidates);
    }

    /// Pop (apply + drop) the stash at `index`, then refresh.
    pub fn stash_pop(&mut self, index: usize) {
        match self.with_git_mut(|g| g.stash_pop(index)) {
            Ok(()) => {
                self.refresh_magit();
                self.minibuffer_message(&format!("popped stash@{{{index}}}"));
            }
            Err(e) => self.minibuffer_message(&format!("stash pop failed: {e}")),
        }
    }

    /// Drop the stash at `index` (without applying it), then refresh.
    pub fn stash_drop(&mut self, index: usize) {
        match self.with_git_mut(|g| g.stash_drop(index)) {
            Ok(()) => {
                self.refresh_magit();
                self.minibuffer_message(&format!("dropped stash@{{{index}}}"));
            }
            Err(e) => self.minibuffer_message(&format!("stash drop failed: {e}")),
        }
    }

    /// Branch-create name prompt: start / append / backspace / confirm /
    /// cancel (mirrors the search-prompt pattern).
    pub fn branch_create_start(&mut self) {
        self.branch_create = Some(String::new());
        self.minibuffer_message("New branch name: ");
    }

    pub(super) fn branch_create_char(&mut self, c: char) {
        if let Some(name) = self.branch_create.as_mut() {
            name.push(c);
        }
        if let Some(name) = self.branch_create.as_ref() {
            self.minibuffer_message(&format!("New branch name: {name}"));
        }
    }

    pub(super) fn branch_create_backspace(&mut self) {
        if let Some(name) = self.branch_create.as_mut() {
            name.pop();
        }
        if let Some(name) = self.branch_create.as_ref() {
            self.minibuffer_message(&format!("New branch name: {name}"));
        }
    }

    pub(super) fn branch_create_cancel(&mut self) {
        self.branch_create.take();
        self.minibuffer_message("cancel");
    }

    pub(super) fn branch_create_confirm(&mut self) {
        let Some(name) = self.branch_create.take() else { return };
        if name.trim().is_empty() {
            self.branch_create = Some(name);
            self.minibuffer_message("empty branch name");
            return;
        }
        self.create_branch_from_head(&name);
    }

    /// The log view's rows (header + one row per commit + paging indicator).
    pub fn log_rows(&self) -> Vec<MagitRow> {
        let Some(log) = self.log.as_ref() else { return Vec::new() };
        let mut rows = vec![MagitRow {
            text: format!("## log ({})", log.display()),
            role: RowRole::Branch,
            selected: false,
        }];
        for (i, e) in log.entries.iter().enumerate() {
            rows.push(MagitRow {
                text: log_entry_display(e),
                role: RowRole::Commit,
                selected: i == log.selected,
            });
        }
        let start = log.offset.saturating_add(1);
        let end = log.offset.saturating_add(log.entries.len());
        rows.push(MagitRow {
            text: format!("  ({}–{}/{}  ·  n next · p prev · RET diff · q back)", start, end, log.total),
            role: RowRole::Comment,
            selected: false,
        });
        rows
    }

    /// The blame view's rows (header + one aligned row per line).
    pub fn blame_rows(&self) -> Vec<MagitRow> {
        let Some(b) = self.blame.as_ref() else { return Vec::new() };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let author_w = b
            .lines
            .iter()
            .map(|l| l.author.chars().count())
            .max()
            .unwrap_or(0)
            .max(4);
        let mut rows = vec![MagitRow {
            text: format!("## blame: {}", b.path),
            role: RowRole::Branch,
            selected: false,
        }];
        for (i, l) in b.lines.iter().enumerate() {
            rows.push(MagitRow {
                text: blame_line_display(l, now, author_w),
                role: RowRole::Blame,
                selected: i == b.selected,
            });
        }
        rows
    }

    /// The read-only commit-diff view's rows (header + diffstat + files +
    /// hunks), reusing the diff `RowRole`s so `row_face` colors them.
    pub fn commit_diff_rows(&self) -> Vec<MagitRow> {
        let Some(cd) = self.commit_diff.as_ref() else { return Vec::new() };
        let d = &cd.diff;
        let mut rows = vec![
            MagitRow {
                text: format!("## {} {}", d.short_id, d.subject),
                role: RowRole::Branch,
                selected: false,
            },
            MagitRow {
                text: format!(
                    "  {} files changed, {} insertions(+), {} deletions(-)",
                    d.files.len(),
                    d.insertions,
                    d.deletions
                ),
                role: RowRole::Comment,
                selected: false,
            },
        ];
        for f in &d.files {
            rows.push(MagitRow {
                text: format!("  {} {} (+{} -{})", f.path, if f.binary { "binary" } else { "" }, f.insertions, f.deletions),
                role: RowRole::File,
                selected: false,
            });
            for h in &f.hunks {
                rows.push(MagitRow {
                    text: format!("    {}", h.header),
                    role: RowRole::HunkHeader,
                    selected: false,
                });
                for l in &h.lines {
                    let role = match l.origin {
                        crate::git::diff::DiffOrigin::Addition => RowRole::DiffAdd,
                        crate::git::diff::DiffOrigin::Deletion => RowRole::DiffDelete,
                        _ => RowRole::DiffContext,
                    };
                    rows.push(MagitRow {
                        text: format!("    {}{}", l.origin.marker(), l.content),
                        role,
                        selected: false,
                    });
                }
            }
        }
        rows
    }

    /// The total row count of the commit-diff pane (header + diffstat +
    /// files + hunks), computed without building the row Vecs (the scroll
    /// handlers only need the bound, not the rows).
    fn commit_diff_row_count(&self) -> usize {
        let Some(cd) = self.commit_diff.as_ref() else {
            return 0;
        };
        let d = &cd.diff;
        let mut n = 2; // header + diffstat
        for f in &d.files {
            n += 1; // file row
            for h in &f.hunks {
                n += 1 + h.lines.len(); // hunk header + body lines
            }
        }
        n
    }

    /// The commit-diff window (visible rows + top + total) for the shared
    /// windowing (issue 003-02). This pane has no cursor: the emacs-motion
    /// keys move `commit_diff_scroll` (the window), so `window_slice` alone
    /// bounds it.
    pub fn commit_diff_view_info(&self) -> (Vec<MagitRow>, usize, usize) {
        let rows = self.commit_diff_rows();
        let total = rows.len();
        if total == 0 {
            return (Vec::new(), 0, 0);
        }
        let (start, end) = window_slice(self.commit_diff_scroll, total, pane_window(self.viewport_lines));
        (rows[start..end].to_vec(), start, total)
    }

    fn set_commit_diff_scroll(&mut self, top: usize) {
        self.commit_diff_scroll = top.min(self.commit_diff_row_count().saturating_sub(1));
    }

    /// C-n in the commit-diff view: scroll the window down one row.
    pub fn commit_diff_scroll_down(&mut self) {
        self.set_commit_diff_scroll(self.commit_diff_scroll + 1);
    }

    /// C-p in the commit-diff view: scroll the window up one row.
    pub fn commit_diff_scroll_up(&mut self) {
        self.set_commit_diff_scroll(self.commit_diff_scroll.saturating_sub(1));
    }

    /// C-v in the commit-diff view: scroll the window down one page.
    pub fn commit_diff_page_down(&mut self) {
        let step = pane_window(self.viewport_lines).saturating_sub(2).max(1);
        self.set_commit_diff_scroll(self.commit_diff_scroll + step);
    }

    /// M-v in the commit-diff view: scroll the window up one page.
    pub fn commit_diff_page_up(&mut self) {
        let step = pane_window(self.viewport_lines).saturating_sub(2).max(1);
        self.set_commit_diff_scroll(self.commit_diff_scroll.saturating_sub(step));
    }

    /// M-> in the commit-diff view: scroll the window to the last row.
    pub fn commit_diff_scroll_bottom(&mut self) {
        let total = self.commit_diff_row_count();
        let w = pane_window(self.viewport_lines);
        self.commit_diff_scroll = total.saturating_sub(w);
    }

    /// M-< in the commit-diff view: scroll the window to the top.
    pub fn commit_diff_scroll_top(&mut self) {
        self.commit_diff_scroll = 0;
    }

    /// The blame cursor row count (header + one row per blamed line).
    fn blame_row_count(&self) -> usize {
        self.blame.as_ref().map(|b| b.lines.len() + 1).unwrap_or(0)
    }

    /// Keep the blame cursor (`b.selected`) inside the visible window; persists
    /// `blame_scroll` so the window follows the cursor in both directions.
    fn blame_keep_visible(&mut self) {
        let Some(b) = self.blame.as_mut() else {
            return;
        };
        // The cursor row sits after the header, so its index in the row list
        // is `b.selected + 1`.
        let cursor = b.selected + 1;
        let total = self.blame_row_count();
        self.blame_scroll = keep_cursor_visible(self.blame_scroll, cursor, total, pane_window(self.viewport_lines));
    }

    /// The blame window (visible rows + top + total) for the shared windowing
    /// (issue 003-02). The cursor row is always inside it (kept by
    /// `blame_keep_visible`).
    pub fn blame_view_info(&self) -> (Vec<MagitRow>, usize, usize) {
        let rows = self.blame_rows();
        let total = rows.len();
        if total == 0 {
            return (Vec::new(), 0, 0);
        }
        let (start, end) = window_slice(self.blame_scroll, total, pane_window(self.viewport_lines));
        (rows[start..end].to_vec(), start, total)
    }

    /// C-n in the blame view: move the cursor down one row (the window keeps
    /// it in view).
    pub fn blame_cursor_down(&mut self) {
        if let Some(b) = self.blame.as_mut() {
            b.selected = (b.selected + 1).min(b.lines.len().saturating_sub(1));
        }
        self.blame_keep_visible();
    }

    /// C-p in the blame view: move the cursor up one row.
    pub fn blame_cursor_up(&mut self) {
        if let Some(b) = self.blame.as_mut() {
            b.selected = b.selected.saturating_sub(1);
        }
        self.blame_keep_visible();
    }

    /// C-v in the blame view: move the cursor down one page.
    pub fn blame_page_down(&mut self) {
        if let Some(b) = self.blame.as_mut() {
            let step = pane_window(self.viewport_lines).saturating_sub(2).max(1);
            b.selected = (b.selected + step).min(b.lines.len().saturating_sub(1));
        }
        self.blame_keep_visible();
    }

    /// M-v in the blame view: move the cursor up one page.
    pub fn blame_page_up(&mut self) {
        if let Some(b) = self.blame.as_mut() {
            let step = pane_window(self.viewport_lines).saturating_sub(2).max(1);
            b.selected = b.selected.saturating_sub(step);
        }
        self.blame_keep_visible();
    }

    /// M-> in the blame view: move the cursor to the last row.
    pub fn blame_cursor_bottom(&mut self) {
        if let Some(b) = self.blame.as_mut() {
            b.selected = b.lines.len().saturating_sub(1);
        }
        self.blame_keep_visible();
    }

    /// M-< in the blame view: move the cursor to the first row.
    pub fn blame_cursor_top(&mut self) {
        if let Some(b) = self.blame.as_mut() {
            b.selected = 0;
        }
        self.blame_keep_visible();
    }

    /// The log row count (header + one row per entry + paging footer).
    fn log_row_count(&self) -> usize {
        self.log.as_ref().map(|l| l.entries.len() + 2).unwrap_or(0)
    }

    /// Keep the log's in-page selection inside the visible window; persists
    /// `log_scroll` so the window follows the selection (paging via `n`/`p`
    /// resets the window to the top, separately).
    fn log_keep_visible(&mut self) {
        let Some(log) = self.log.as_ref() else {
            return;
        };
        // The selection row sits after the header, so its index is `log.selected + 1`.
        let cursor = log.selected + 1;
        let total = self.log_row_count();
        self.log_scroll = keep_cursor_visible(self.log_scroll, cursor, total, pane_window(self.viewport_lines));
    }

    /// The log window (visible rows + top + total) for the shared windowing
    /// (issue 003-02). The in-page selection is always inside it (kept by
    /// `log_keep_visible`).
    pub fn log_view_info(&self) -> (Vec<MagitRow>, usize, usize) {
        let rows = self.log_rows();
        let total = rows.len();
        if total == 0 {
            return (Vec::new(), 0, 0);
        }
        let (start, end) = window_slice(self.log_scroll, total, pane_window(self.viewport_lines));
        (rows[start..end].to_vec(), start, total)
    }

    /// The log view's title (the branch name or "(detached HEAD)").
    pub fn log_title(&self) -> String {
        self.log.as_ref().map(|l| format!("log — {}", l.display())).unwrap_or_default()
    }

    /// The blame view's title (the file being blamed).
    pub fn blame_title(&self) -> String {
        self.blame.as_ref().map(|b| format!("blame: {}", b.path)).unwrap_or_default()
    }

    /// The commit-diff view's title (the commit's short id + subject).
    pub fn commit_diff_title(&self) -> String {
        self.commit_diff
            .as_ref()
            .map(|cd| format!("commit {} — {}", cd.diff.short_id, cd.diff.subject))
            .unwrap_or_default()
    }

    /// The commit editor's title.
    pub fn commit_editor_title(&self) -> String {
        self.commit_editor
            .as_ref()
            .map(|ed| format!("commit ({} staged)", ed.staged.len()))
            .unwrap_or_default()
    }
}

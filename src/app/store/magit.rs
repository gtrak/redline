use super::*;

impl AppStore {
    /// Open (or re-focus) the magit status buffer: ensure the repo is
    /// open, refresh the section tree, and push the view if needed.
    pub fn open_magit_status(&mut self) {
        let Some(project) = self.project.clone() else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        if !self.ensure_git(project.root) {
            return;
        }
        if !self.refresh_magit() {
            return;
        }
        if self.top_view() != ViewId::MagitStatus {
            self.push_view(ViewId::MagitStatus);
        }
    }

    /// `g`: manually refresh the status (watcher-driven auto-refresh is
    /// issue 04; the watcher bus will subscribe here).
    pub fn magit_refresh(&mut self) {
        if self.refresh_magit() {
            self.minibuffer_message("status refreshed");
        }
    }

    /// `TAB`: fold/unfold the section under the cursor.
    pub fn magit_toggle_fold(&mut self) {
        if let Some(t) = self.status_tree.as_mut() {
            t.toggle_fold();
        }
        self.magit_keep_visible();
    }

    /// `n` / `C-n`: move the cursor to the next visible section. Magit
    /// 4.7.1 does not wrap: at the last visible section the cursor stays
    /// put and the echo area reports `No next section`.
    pub fn magit_cursor_down(&mut self) {
        if let Some(t) = self.status_tree.as_mut()
            && !t.move_down()
        {
            self.minibuffer_message("No next section");
        }
        self.magit_keep_visible();
    }

    /// `p` / `C-p`: move the cursor to the previous visible section. Magit
    /// 4.7.1 does not wrap: at the first visible section the cursor stays
    /// put and the echo area reports `No previous section`.
    pub fn magit_cursor_up(&mut self) {
        if let Some(t) = self.status_tree.as_mut()
            && !t.move_up()
        {
            self.minibuffer_message("No previous section");
        }
        self.magit_keep_visible();
    }

    /// `RET`: visit the file under the cursor through the buffer model
    /// (issue 02) at the file level, then return to the buffer view.
    /// TODO(issue 03): jump to the hunk offset once FileView lands.
    pub fn magit_visit_file(&mut self) {
        let path = match self
            .status_tree
            .as_ref()
            .and_then(|t| t.cursor_target())
            .and_then(|t| t.path)
        {
            Some(p) => p,
            None => {
                self.minibuffer_message("no file under point");
                return;
            }
        };
        self.open_path(&path);
        if self.top_view() == ViewId::MagitStatus {
            self.close_view();
        }
    }

    /// `s`: stage the change at point (file or hunk on the unstaged side,
    /// or an untracked file), then refresh.
    pub fn magit_stage(&mut self) {
        let Some(target) = self.status_tree.as_ref().and_then(|t| t.cursor_target()) else {
            self.minibuffer_message("no section under point");
            return;
        };
        let Some(path) = target.path else {
            self.minibuffer_message("no file under point");
            return;
        };
        let result = match (target.kind, target.side) {
            (SectionKind::File, Some(Side::Unstaged) | Some(Side::Untracked)) => {
                self.with_git(move |g| g.stage_file(&path))
            }
            (SectionKind::Hunk, Some(Side::Unstaged)) => {
                let start = target.hunk_new_start.unwrap_or(0);
                self.with_git(move |g| g.stage_hunk(&path, start))
            }
            _ => {
                self.minibuffer_message("nothing to stage at point");
                return;
            }
        };
        match result {
            Ok(()) => {
                self.refresh_magit();
            }
            Err(e) => self.minibuffer_message(&format!("stage failed: {e}")),
        }
    }

    /// `u`: unstage the change at point (file or hunk on the staged side),
    /// then refresh.
    pub fn magit_unstage(&mut self) {
        let Some(target) = self.status_tree.as_ref().and_then(|t| t.cursor_target()) else {
            self.minibuffer_message("no section under point");
            return;
        };
        let Some(path) = target.path else {
            self.minibuffer_message("no file under point");
            return;
        };
        let result = match (target.kind, target.side) {
            (SectionKind::File, Some(Side::Staged)) => {
                let orig = target.orig.clone();
                self.with_git(move |g| g.unstage_file(&path, orig.as_deref()))
            }
            (SectionKind::Hunk, Some(Side::Staged)) => {
                let start = target.hunk_new_start.unwrap_or(0);
                self.with_git(move |g| g.unstage_hunk(&path, start))
            }
            _ => {
                self.minibuffer_message("nothing to unstage at point");
                return;
            }
        };
        match result {
            Ok(()) => {
                self.refresh_magit();
            }
            Err(e) => self.minibuffer_message(&format!("unstage failed: {e}")),
        }
    }

    /// The magit status rows for the current fold state (empty when the
    /// tree has not been built).
    pub fn magit_rows(&self) -> Vec<MagitRow> {
        self.status_tree
            .as_ref()
            .map(|t| t.visible_rows())
            .unwrap_or_default()
    }

    /// The number of magit status rows that fit in the content area:
    /// `viewport_lines - 2` (room for the pinned title, a scroll indicator,
    /// and the help line), at least one. `viewport_lines` is the file-view
    /// content height set on resize.
    fn magit_window(&self) -> usize {
        pane_window(self.viewport_lines)
    }

    /// Keep the magit cursor row inside the visible window (issue 002-02
    /// windowing for long status buffers). Called on every cursor move, fold,
    /// and refresh; persists `magit_scroll` so the window tracks the cursor in
    /// both directions. Clamped to the row count.
    fn magit_keep_visible(&mut self) {
        let rows = self
            .status_tree
            .as_ref()
            .map(|t| t.visible_rows())
            .unwrap_or_default();
        let total = rows.len();
        if total == 0 {
            self.magit_scroll = 0;
            return;
        }
        let Some(cursor) = rows.iter().position(|r| r.selected) else {
            // No cursor row (shouldn't happen once the tree is built); just
            // clamp the offset.
            self.magit_scroll = self.magit_scroll.min(total.saturating_sub(1));
            return;
        };
        self.magit_scroll = keep_cursor_visible(
            self.magit_scroll,
            cursor,
            total,
            self.magit_window(),
        );
    }

    /// The magit status window (visible rows + top row index + total row
    /// count) for long status buffers (issue 002-02). The cursor row is always
    /// inside the window (kept by `magit_keep_visible`); the view renders the
    /// window plus a scroll indicator and a pinned help line so neither is
    /// clipped.
    pub fn magit_view_info(&self) -> (Vec<MagitRow>, usize, usize) {
        let rows = self.magit_rows();
        let total = rows.len();
        if total == 0 {
            return (Vec::new(), 0, 0);
        }
        let (start, end) = window_slice(self.magit_scroll, total, self.magit_window());
        (rows[start..end].to_vec(), start, total)
    }

    /// Live dirty counts for the status line (`None` until the first
    /// refresh).
    pub fn dirty_counts(&self) -> Option<DirtyCounts> {
        self.dirty
    }

    /// Run `f` against the cached repo, opening a `NotARepository` error
    /// when none is open.
    pub(super) fn with_git<R>(&self, f: impl FnOnce(&GitRepo) -> Result<R, GitError>) -> Result<R, GitError> {
        match self.git.as_ref() {
            Some(g) => f(g),
            None => Err(GitError::NotARepository {
                path: self
                    .project
                    .as_ref()
                    .map(|p| p.root.clone())
                    .unwrap_or_else(|| PathBuf::from(".")),
            }),
        }
    }

    /// Mutable variant of [`with_git`] (the git2 `stash_*` APIs take
    /// `&mut Repository`).
    pub(super) fn with_git_mut<R>(
        &mut self,
        f: impl FnOnce(&mut GitRepo) -> Result<R, GitError>,
    ) -> Result<R, GitError> {
        match self.git.as_mut() {
            Some(g) => f(g),
            None => Err(GitError::NotARepository {
                path: self
                    .project
                    .as_ref()
                    .map(|p| p.root.clone())
                    .unwrap_or_else(|| PathBuf::from(".")),
            }),
        }
    }

    /// Open (or keep) the cached git repo at `root`.
    pub(super) fn ensure_git(&mut self, root: PathBuf) -> bool {
        if self.git.is_none() {
            match GitRepo::discover(&root) {
                Ok(g) => self.git = Some(g),
                Err(e) => {
                    self.minibuffer_message(&e.to_string());
                    return false;
                }
            }
        }
        true
    }

    pub(super) fn git_status(&self) -> Result<RepoStatus, GitError> {
        match self.git.as_ref() {
            Some(g) => g.status(),
            None => Err(GitError::NotARepository {
                path: self
                    .project
                    .as_ref()
                    .map(|p| p.root.clone())
                    .unwrap_or_else(|| PathBuf::from(".")),
            }),
        }
    }

    /// Fetch per-file diffs for every changed file (staged and unstaged
    /// sides). Untracked files have no diff.
    fn collect_diffs(&self, status: &RepoStatus) -> (HashMap<String, FileDiff>, HashMap<String, FileDiff>) {
        let git = match self.git.as_ref() {
            Some(g) => g,
            None => return (HashMap::new(), HashMap::new()),
        };
        let mut staged = HashMap::new();
        let mut unstaged = HashMap::new();
        for f in &status.files {
            if f.is_staged()
                && let Ok(d) = git.diff(DiffSide::Staged, &f.path)
            {
                staged.insert(f.path.clone(), d);
            }
            if f.is_unstaged()
                && let Ok(d) = git.diff(DiffSide::Unstaged, &f.path)
            {
                unstaged.insert(f.path.clone(), d);
            }
        }
        (staged, unstaged)
    }

    /// Rebuild the status section tree (preserving fold + cursor) and the
    /// dirty counts from a fresh status. Returns false (and sets the
    /// minibuffer) on a git error.
    pub(super) fn refresh_magit(&mut self) -> bool {
        let status = match self.git_status() {
            Ok(s) => s,
            Err(e) => {
                self.minibuffer_message(&format!("git status failed: {e}"));
                return false;
            }
        };
        let (staged, unstaged) = self.collect_diffs(&status);
        let prev = self.status_tree.clone();
        let tree = StatusTree::build(&status, &staged, &unstaged, prev.as_ref());
        self.dirty = Some(DirtyCounts {
            staged: status.staged_count(),
            unstaged: status.unstaged_count(),
            untracked: status.untracked_count(),
        });
        self.status_tree = Some(tree);
        self.magit_keep_visible();
        true
    }

    /// `k`: arm a destructive-discard confirmation for the file/hunk under the
    /// cursor. The actual discard runs only on `y` (magit gates discards with
    /// a confirmation); `n`/C-g/ESC cancel.
    pub fn magit_discard(&mut self) {
        let Some(target) = self
            .status_tree
            .as_ref()
            .and_then(|t| t.cursor_target())
        else {
            self.minibuffer_message("no section under point");
            return;
        };
        let Some(path) = target.path.clone() else {
            self.minibuffer_message("no file under point");
            return;
        };
        if !matches!(target.kind, SectionKind::File | SectionKind::Hunk) {
            self.minibuffer_message("nothing to discard at point");
            return;
        }
        let what = match target.kind {
            SectionKind::Hunk => format!("hunk of {path}"),
            _ => path.clone(),
        };
        self.discard_confirm = Some(DiscardTarget {
            path: path.clone(),
            kind: target.kind,
            side: target.side,
            orig: target.orig.clone(),
            hunk_new_start: target.hunk_new_start,
        });
        self.minibuffer_message(&format!("discard {what}? y/n"));
    }

    /// Handle a key while a discard confirmation is armed: `y` executes the
    /// discard, `n`/C-g/ESC cancel; other keys are swallowed.
    pub(super) fn discard_key_event(&mut self, key: Key) {
        if key == Key::char('y') {
            self.confirm_discard();
            return;
        }
        if key == Key::char('n')
            || key == Key::ctrl_char('g')
            || key.code == crate::app::keymap::KeyCode::Escape
        {
            self.discard_confirm = None;
            self.minibuffer_message("discard cancelled");
        }
        // else: swallowed (ignored).
    }

    /// Whether a destructive-discard confirmation is currently armed.
    pub fn discard_armed(&self) -> bool {
        self.discard_confirm.is_some()
    }

    /// `y`: run the armed discard, then refresh.
    fn confirm_discard(&mut self) {
        let Some(target) = self.discard_confirm.take() else {
            return;
        };
        match self.execute_discard(&target) {
            Ok(()) => {
                self.refresh_magit();
                self.minibuffer_message(&format!("discarded {}", target.path));
            }
            Err(e) => self.minibuffer_message(&format!("discard failed: {e}")),
        }
    }

    /// Run the git operation for a discard target (no confirmation here —
    /// the caller has already confirmed).
    fn execute_discard(&self, target: &DiscardTarget) -> Result<(), GitError> {
        match target.kind {
            SectionKind::Hunk => {
                let start = target.hunk_new_start.unwrap_or(0);
                self.with_git(|g| g.discard_hunk(&target.path, target.side, start))
            }
            SectionKind::File => match target.side {
                Some(Side::Untracked) => {
                    self.with_git(|g| g.discard_untracked_file(&target.path))
                }
                Some(Side::Staged) => self
                    .with_git(|g| g.discard_staged_file(&target.path, target.orig.as_deref())),
                _ => self.with_git(|g| g.discard_unstaged_file(&target.path)),
            },
            _ => Err(GitError::NotARepository {
                path: self
                    .project
                    .as_ref()
                    .map(|p| p.root.clone())
                    .unwrap_or_default(),
            }),
        }
    }
}

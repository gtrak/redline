use super::*;

impl AppStore {
    /// Open the project-relative file in a buffer and make it current;
    /// records it in the project's recents (persisted).
    pub fn open_path(&mut self, rel: &str) {
        let Some(project) = self.project.clone() else {
            self.minibuffer_message("no project");
            return;
        };
        let abs = project.root.join(rel);
        if let Err(e) = self.open_project_path(&abs, rel) {
            self.minibuffer_message(&e);
        }
    }

    /// The load + install core of `open_path`: `abs` = `project.root.join(rel)`
    /// (already resolved by the caller). Returns the error report on failure
    /// so a caller (the tooling-resolver landing, plan 006 issue 02) can
    /// decide whether a jump entry is recorded.
    pub(super) fn open_project_path(&mut self, abs: &Path, rel: &str) -> Result<(), String> {
        let key = abs.to_string_lossy().into_owned();
        if self.buffers.get(&key).is_none() {
            match load_file(abs) {
                Ok((rope, mtime)) => {
                    self.buffers.insert_rope(Some(abs.to_path_buf()), rope, mtime, false);
                }
                Err(e) => {
                    return Err(format!("cannot open {rel}: {e}"));
                }
            }
        } else {
            // Re-stat on reopen: reload when mtime changed (spec: "highlight
            // cache invalidates when a file changes on disk (pre-watcher: on
            // reopen)"). Issue-03 buffers are read-only, so this is safe.
            if let Ok(new_mtime) = std::fs::metadata(abs).and_then(|m| m.modified()) {
                let old_mtime = self
                    .buffers
                    .get(&key)
                    .map(|b| b.mtime)
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                if new_mtime != old_mtime {
                    match load_file(abs) {
                        Ok((rope, mtime)) => {
                            self.buffers
                                .insert_rope(Some(abs.to_path_buf()), rope, mtime, false);
                        }
                        // Reload failure is non-fatal (the buffer stays current
                        // on its stale content); keep `open_path`'s report.
                        Err(e) => {
                            self.minibuffer_message(&format!("cannot reload {rel}: {e}"));
                        }
                    }
                }
            }
        }
        self.buffers.set_current(&key);
        self.record_recent(rel);
        // Buffer-follow (issue 09, off by default): sync the tree cursor.
        self.tree_follow_opened(rel);
        // plan 005 issue 02: on load, the annotation anchors for this file
        // maintain themselves against the (possibly re-read) content.
        self.reanchor_for_key(&key);
        // Build (or update) the highlight for the new current buffer.
        self.ensure_highlight();
        self.normalize_top_view();
        Ok(())
    }

    /// Record `rel` in the current project's recents and persist.
    pub(super) fn record_recent(&mut self, rel: &str) {
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let root = project.root.to_string_lossy().into_owned();
        self.project_store.recents.add(&root, rel);
        let _ = self.project_store.save_recents();
    }

    /// Ensure the current project's file list is cached; builds it on
    /// first use (one walk per project — the hot key is the nucleo
    /// filter over the cached list, not the walk).
    pub(super) fn ensure_files(&mut self) -> Option<()> {
        let project = self.project.clone()?;
        if !self.files.contains_key(&project.root) {
            match FileList::build(&project.root) {
                Ok(list) => {
                    self.files.insert(project.root.clone(), list);
                }
                Err(e) => {
                    self.minibuffer_message(&format!(
                        "walk failed for {}: {e}",
                        project.name
                    ));
                    return None;
                }
            }
        }
        Some(())
    }

    /// `C-c p i`: invalidate the cached file list and re-walk the
    /// project root.
    pub fn re_walk(&mut self) {
        let Some(project) = self.project.clone() else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        match FileList::build(&project.root) {
            Ok(list) => {
                let n = list.len();
                self.files.insert(project.root.clone(), list);
                // Rebuild the tree sidebar rows (blocking fix #2): the old
                // rows reflect the pre-walk file set.
                if self.tree.visible {
                    self.tree.rows = self.build_tree_rows();
                    self.tree.selected = 0;
                }
                self.minibuffer_message(&format!(
                    "re-walked {}: {} files",
                    project.name, n
                ));
            }
            Err(e) => self.minibuffer_message(&format!(
                "walk failed for {}: {e}",
                project.name
            )),
        }
    }

    /// Switch the current project to `root` (a known-project registry
    /// entry), persist the registry, and land in the new project's file
    /// picker (projectile's default switch action is find-file).
    pub fn switch_project_root(&mut self, root: &str) {
        let root_path = PathBuf::from(root);
        let project = Project::new(root_path.clone());
        let name = project.name.clone();
        self.project = Some(project);
        // Invalidate the cached git repo and magit state: they belong to
        // the previous project root and would otherwise be used against
        // the wrong repository after the switch.
        self.git = None;
        self.status_tree = None;
        self.dirty = None;
        // Swap the file watcher: stop the old project's watcher and start
        // one for the new root (exactly one at a time). No-op in plain unit
        // tests (no runtime).
        self.start_watcher();
        // Rebuild the symbol index for the new project (full rebuild, not
        // incremental: the old index belongs to the previous project).
        // Bump the generation so that any in-flight job from the previous
        // project is tagged stale and its event discarded by
        // `apply_index_event`. `start_indexing` will start a new job even
        // if the old project's job is still in flight (generation-aware
        // single-flight).
        self.index = SymbolIndex::new();
        self.index_generation += 1;
        self.pending_index_changes.clear();
        // A resolve job in flight belonged to the previous project: bump the
        // generation so its event is discarded (mirrors the index bump above).
        self.resolve_generation += 1;
        // Invalidate any in-flight search job (its root was the previous
        // project): bump the generation so its events are discarded, and
        // stop it. Results belong to the old project.
        self.cancel_search_job();
        self.search_generation += 1;
        self.search = SearchState::default();
        self.search.generation = self.search_generation;
        // Reset the tree sidebar (issue 09, blocking fix #2): rows belong to
        // the previous project and would open wrong-project files via
        // tree_open_selected. Clear them; the sidebar rebuilds on toggle.
        self.tree.rows.clear();
        self.tree.selected = 0;
        self.start_indexing();
        self.project_store.registry.upsert(&root_path);
        let _ = self.project_store.save_registry();
        self.ensure_files();
        self.minibuffer_message(&format!("project: {name}"));
        self.open_find_file();
    }

    /// `C-c p t`: toggle the file-tree sidebar. Builds the (ignore-aware)
    /// rows from the cached walk on first show.
    pub fn toggle_tree(&mut self) {
        self.tree.visible = !self.tree.visible;
        if self.tree.visible && self.tree.rows.is_empty() {
            // The file walk is lazy (populated by find-file); populate it
            // here so first-open shows the project instead of an empty tree
            // with a misleading "no files" message.
            self.ensure_files();
            self.tree.rows = self.build_tree_rows();
        }
        if self.tree.visible && self.tree.rows.is_empty() {
            self.minibuffer_message("tree: no files in project");
        }
    }

    /// Build the indented file rows from the project's cached file list.
    /// Each file is a row indented by its directory depth (treemacs-lite:
    /// directories are implied by the indentation, files are the leaves).
    fn build_tree_rows(&self) -> Vec<TreeRow> {
        let Some(project) = self.project.as_ref() else {
            return Vec::new();
        };
        let files = match self.files.get(&project.root) {
            Some(f) => f,
            None => return Vec::new(),
        };
        files
            .files
            .iter()
            .map(|rel| {
                let parts: Vec<&str> = rel.split('/').collect();
                let depth = parts.len().saturating_sub(1);
                let name = parts.last().copied().unwrap_or(rel).to_string();
                TreeRow {
                    depth,
                    name,
                    is_dir: false,
                    rel_path: rel.clone(),
                }
            })
            .collect()
    }

    /// `true` when the sidebar is showing (drives the root's Row layout).
    pub fn tree_visible(&self) -> bool {
        self.tree.visible
    }

    /// The sidebar's rows (empty when hidden or not yet built).
    pub fn tree_rows(&self) -> Vec<TreeRow> {
        if self.tree.visible {
            self.tree.rows.clone()
        } else {
            Vec::new()
        }
    }

    pub fn tree_selected(&self) -> usize {
        self.tree.selected
    }

    /// `↓` / `PageDown`: move the tree cursor down (clamped).
    pub fn tree_move_down(&mut self) {
        if !self.tree.visible || self.tree.rows.is_empty() {
            return;
        }
        self.tree.selected = (self.tree.selected + 1).min(self.tree.rows.len() - 1);
    }

    /// `↑` / `PageUp`: move the tree cursor up (clamped).
    pub fn tree_move_up(&mut self) {
        if !self.tree.visible || self.tree.rows.is_empty() {
            return;
        }
        self.tree.selected = self.tree.selected.saturating_sub(1);
    }

    /// Click-to-select in the tree sidebar (plan 004 issue 05e): the
    /// sidebar layout is a title row (terminal row 0), up to
    /// `TREE_VISIBLE_ROWS` file rows (starting at the visible window top,
    /// `selected.saturating_sub(5)`), then a help row. Map a 0-based
    /// terminal row onto the tree row under it; the title row, the help
    /// row, rows past the window, a hidden tree, or a non-buffer top view
    /// are no-ops. A tree click moves ONLY the tree cursor — it never
    /// touches the code point (the code point is set by code-pane clicks,
    /// and `RET` opens the selected file).
    pub fn tree_click_row(&mut self, terminal_row: usize) {
        // 06a: the tree shares the main pane with home (home renders in the
        // buffer slot), so clicks land while either is on top.
        if !self.tree.visible
            || !matches!(self.top_view(), ViewId::Buffer | ViewId::Home)
        {
            return;
        }
        let Some(rel) = terminal_row.checked_sub(1) else {
            return; // terminal row 0 is the tree title
        };
        if rel >= crate::model::tree_layout::TREE_VISIBLE_ROWS {
            return; // the help row (or below it)
        }
        let start = self.tree.selected.saturating_sub(5);
        let idx = start.saturating_add(rel);
        if idx < self.tree.rows.len() {
            self.tree.selected = idx;
        }
    }

    /// `RET` (with the tree focused): open the selected file, returning to
    /// the buffer view. No-op on an empty tree.
    pub fn tree_open_selected(&mut self) {
        let (is_dir, rel_path) = match self.tree.rows.get(self.tree.selected) {
            Some(row) => (row.is_dir, row.rel_path.clone()),
            None => {
                self.minibuffer_message("tree: nothing selected");
                return;
            }
        };
        if !is_dir {
            self.open_path(&rel_path);
        }
    }

    /// Toggle optional buffer-follow (off by default): when on, opening a
    /// buffer also moves the tree cursor to that file.
    pub fn toggle_tree_follow(&mut self) {
        self.tree.follow = !self.tree.follow;
        self.minibuffer_message(&format!("tree buffer-follow: {}", if self.tree.follow { "on" } else { "off" }));
    }

    /// When buffer-follow is on and the tree is visible, sync the tree cursor
    /// to the just-opened file (called from `open_path`).
    fn tree_follow_opened(&mut self, rel: &str) {
        if self.tree.follow && self.tree.visible
            && let Some(idx) = self.tree.rows.iter().position(|r| r.rel_path == rel)
        {
            self.tree.selected = idx;
        }
    }
}

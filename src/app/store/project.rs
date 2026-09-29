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
            // reopen)"). The update is always IN PLACE: the buffer's identity
            // (mode, editable, is_notes, mark) belongs to the session, not
            // the file, so a reopen changes the CONTENT, never the identity.
            // This branch used to `insert_rope` the buffer wholesale,
            // justified by "Issue-03 buffers are read-only, so this is
            // safe" — that justification was false the moment Accurate-mode
            // editing landed: `insert_rope` rebuilds the buffer via
            // `Buffer::new` (mode resets to `Annotation`) and replaces the
            // rope, so an externally-changed file dropped BOTH the edit mode
            // and any unsaved edits with no prompt and no record — a
            // data-loss path. The dirty case now asks instead.
            if let Ok(new_mtime) = std::fs::metadata(abs).and_then(|m| m.modified())
                && let Some(buf) = self.buffers.get(&key)
                && new_mtime != buf.mtime
            {
                if buf.locally_modified() {
                    // Unsaved edits are never discarded silently (the
                    // emacs `revert-buffer` policy: a modified buffer is
                    // not reverted without asking). Keep the text and
                    // the mode, arm the discard/reload confirm (`y`
                    // reloads from disk, `n`/C-g/ESC keep the edits),
                    // and surface the conflict so the buffer's marker
                    // shows either way.
                    if let Some(b) = self.buffers.get_mut(&key) {
                        b.changed_on_disk = true;
                    }
                    self.reload_confirm = Some(key.clone());
                    self.minibuffer_message(&format!(
                        "File changed on disk; discard unsaved edits in {} and reload? (y or n)",
                        self.buffer_display(&key)
                    ));
                } else {
                    // Clean buffer: the disk content wins, applied in
                    // place (mode and every other identity field
                    // survive the reopen).
                    if let Err(e) = self.reload_in_place(&key, abs) {
                        // Reload failure is non-fatal (the buffer stays
                        // current on its stale content); keep
                        // `open_path`'s report.
                        self.minibuffer_message(&format!("cannot reload {rel}: {e}"));
                    }
                }
            }
        }
        self.buffers.set_current(&key);
        // plan 015 issue 03 (P2-a): a notes document opened via find-file
        // (open_path) must carry the same buffer-relative baseline as
        // open_notes — the is_notes flag rides on the buffer, so it is set on
        // EVERY route that opens the notes file (compare against notes_key()
        // at open time), not only open_notes. Without this, C-x C-f
        // .redline-notes.md → C-x C-q → C-x C-q ends read-only (the old notes
        // buffer survives the table but the baseline is not marked).
        if self.notes_key().as_deref() == Some(key.as_str())
            && let Some(buf) = self.buffers.get_mut(&key)
        {
            buf.is_notes = true;
        }
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
                // U-E12: a re-walk re-derives under a still-active narrow
                // query — the projection recomputes over the fresh rows and
                // the selection re-clamps into the surviving set.
                self.tree_narrow_recompute();
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
        self.resolve_in_flight = false; // P3-4: that request is dead
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
        // U-E12: re-showing re-derives under a still-active narrow query
        // (the magit/log query-persists precedent) — the selection
        // re-clamps into the surviving set.
        self.tree_narrow_recompute();
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

    /// The sidebar's FULL rows (the canonical walk output; empty when
    /// hidden or not yet built). The narrow query never mutates these —
    /// it is a view-time projection over them (`tree_view_info`).
    #[allow(dead_code)] // test-only accessor (the view renders tree_view_info's windowed projection)
    pub fn tree_rows(&self) -> Vec<TreeRow> {
        if self.tree.visible {
            self.tree.rows.clone()
        } else {
            Vec::new()
        }
    }

    #[allow(dead_code)] // test-only accessor (the view renders the in-window selection from tree_view_info)
    pub fn tree_selected(&self) -> usize {
        self.tree.selected
    }

    /// The tree narrow query (U-E12): the text typed on the sidebar's own
    /// prompt row (keys leading, one NoWrap row under the `*tree*` title);
    /// empty = no narrowing (the full tree).
    pub fn tree_narrow_query(&self) -> &str {
        &self.tree.narrow_query
    }

    /// Re-derive the narrowed state after a query change (U-E12): the
    /// projection itself is view-time (`tree_narrowing_rows` recomputes
    /// it), so this keeps the selection invariant — while a query is
    /// active `selected` (an index into the FULL `rows` list) points at a
    /// SURVIVING row: if the currently selected file survives it stays,
    /// otherwise it clamps onto the FIRST surviving row in source order
    /// (the U-E10 cursor-clamp shape: the selection's identity is the
    /// file, never a row index of the narrowed set). A zero-match query
    /// leaves the selection alone: there is no surviving row to rest on,
    /// and the clear re-derives the full list where the index is valid
    /// again (the stale-index class U-E10/U-E13 caught — `selected` must
    /// never index into the narrowed set, because `tree_open_selected`
    /// and the follow sync read it against the full `rows`).
    pub(super) fn tree_narrow_recompute(&mut self) {
        if self.tree.narrow_query.is_empty() {
            return;
        }
        let q = self.tree.narrow_query.clone();
        let surviving =
            tree_surviving_indexes(&self.tree.rows, &q, &mut self.matcher);
        if surviving.is_empty() {
            return;
        }
        if !surviving.contains(&self.tree.selected) {
            self.tree.selected = surviving[0];
        }
    }

    /// `C-g` in the tree sidebar with a narrow query active (U-E12): clear
    /// the query — the full tree re-derives, and the selection is intact
    /// (clamped, not lost: while the query was active it rested on a
    /// surviving row, which exists in the full list again).
    pub(super) fn tree_narrow_clear(&mut self) {
        self.tree.narrow_query.clear();
        self.tree_narrow_recompute();
        self.minibuffer_message("filter cleared");
    }

    /// The store-owned tree window (U-E12 windowing re-home, PLAN §1 row 4
    /// — the ONE surface whose windowing used to live in the RENDERER):
    /// the narrowed projection windowed around the selection, the way every
    /// other list surface gets its window from the store. The window is the
    /// first `TREE_VISIBLE_ROWS` rows of the narrowed set starting at
    /// `sel.saturating_sub(5)` (the pre-re-home renderer math, verbatim —
    /// with an empty query the narrowed set IS the full list, so this is
    /// byte-for-byte the old renderer window), and `selected` is the
    /// IN-WINDOW index of the selected row. Load-bearing invariant: when
    /// the window is non-empty the cursor row is ALWAYS inside it (the
    /// selection's in-window index is `<= 5 < TREE_VISIBLE_ROWS`), and the
    /// renderer renders exactly what this returns — it no longer skips or
    /// takes anything.
    pub fn tree_view_info(&mut self) -> (Vec<TreeRow>, usize) {
        let q = self.tree.narrow_query.clone();
        let surviving =
            tree_surviving_indexes(&self.tree.rows, &q, &mut self.matcher);
        if surviving.is_empty() {
            return (Vec::new(), 0);
        }
        let sel_pos = surviving
            .iter()
            .position(|&i| i == self.tree.selected)
            .unwrap_or(0);
        let start = sel_pos.saturating_sub(5);
        let end = (start + redline_model::tree_layout::TREE_VISIBLE_ROWS).min(surviving.len());
        let rows: Vec<TreeRow> = surviving[start..end]
            .iter()
            .map(|&i| self.tree.rows[i].clone())
            .collect();
        (rows, sel_pos - start)
    }

    /// `↓` / `PageDown`: move the tree cursor down (clamped). U-E12: while
    /// the narrow query is active the move steps over the NARROWED set — a
    /// filtered-out file is never a landing spot (the clamped, no-wrap
    /// boundary semantics of the full-list move are kept: at the last
    /// surviving row the cursor stays put).
    pub fn tree_move_down(&mut self) {
        if !self.tree.visible || self.tree.rows.is_empty() {
            return;
        }
        if self.tree.narrow_query.is_empty() {
            self.tree.selected = (self.tree.selected + 1).min(self.tree.rows.len() - 1);
            return;
        }
        let q = self.tree.narrow_query.clone();
        let surviving =
            tree_surviving_indexes(&self.tree.rows, &q, &mut self.matcher);
        if let Some(pos) = surviving.iter().position(|&i| i == self.tree.selected)
            && pos + 1 < surviving.len()
        {
            self.tree.selected = surviving[pos + 1];
        }
    }

    /// `↑` / `PageUp`: move the tree cursor up (clamped). U-E12: while
    /// narrowed, over the narrowed set (see `tree_move_down`).
    pub fn tree_move_up(&mut self) {
        if !self.tree.visible || self.tree.rows.is_empty() {
            return;
        }
        if self.tree.narrow_query.is_empty() {
            self.tree.selected = self.tree.selected.saturating_sub(1);
            return;
        }
        let q = self.tree.narrow_query.clone();
        let surviving =
            tree_surviving_indexes(&self.tree.rows, &q, &mut self.matcher);
        if let Some(pos) = surviving.iter().position(|&i| i == self.tree.selected)
            && pos > 0
        {
            self.tree.selected = surviving[pos - 1];
        }
    }

    /// Click-to-select in the tree sidebar (plan 004 issue 05e): the
    /// sidebar layout is a title row (terminal row 0), the U-E12 narrow
    /// prompt row (terminal row 1), up to `TREE_VISIBLE_ROWS` file rows
    /// (the store-owned window, U-E12 re-home: starting at the window top
    /// `sel.saturating_sub(5)` over the NARROWED set), then a help row.
    /// Map a 0-based terminal row onto the tree row under it; the title
    /// row, the prompt row, the help row, rows past the window, a hidden
    /// tree, or a non-buffer top view are no-ops. A tree click moves ONLY
    /// the tree cursor — it never touches the code point (the code point
    /// is set by code-pane clicks, and `RET` opens the selected file).
    pub fn tree_click_row(&mut self, terminal_row: usize) {
        // 06a: the tree shares the main pane with home (home renders in the
        // buffer slot), so clicks land while either is on top.
        if !self.tree.visible
            || !matches!(self.top_view(), ViewId::Buffer | ViewId::Home)
        {
            return;
        }
        // plan 016 issue 04 (gate P2): a tree row click RUNS A COMMAND (it
        // moves the tree cursor; the method docs note it never touches the
        // code point), so it ends the self-insert run — the rule is about
        // the command, not about whether the point moved. This handler is a
        // non-`key_event` input path, so it must clear the marker itself.
        // AFTER the guard, like the buffer click: a click that ran nothing
        // leaves the run armed.
        self.self_insert_run = None;
        // U-E12: terminal row 0 is the tree title and row 1 is the narrow
        // prompt — neither is a file row (a prompt-row click is a no-op,
        // not a selection).
        let Some(rel) = terminal_row.checked_sub(2) else {
            return;
        };
        if rel >= redline_model::tree_layout::TREE_VISIBLE_ROWS {
            return; // the help row (or below it)
        }
        // The window is over the NARROWED set (U-E12): with an empty query
        // that IS the full list, so the mapping is the pre-re-home one.
        let q = self.tree.narrow_query.clone();
        let surviving =
            tree_surviving_indexes(&self.tree.rows, &q, &mut self.matcher);
        if surviving.is_empty() {
            return; // the zero-match empty state: no rows under the cursor
        }
        let sel_pos = surviving
            .iter()
            .position(|&i| i == self.tree.selected)
            .unwrap_or(0);
        let start = sel_pos.saturating_sub(5);
        // `rel` is the window SLOT (0-based from the row under the prompt):
        // the set index under it is `start + rel` (a slot past the end of a
        // short tail window is a no-op, as before the re-home).
        if let Some(&full) = surviving.get(start + rel) {
            self.tree.selected = full;
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
            // U-E12: the follow move re-derives under a still-active narrow
            // query — the opened file rests on its row when it survives,
            // otherwise the cursor clamps back onto the first survivor.
            self.tree_narrow_recompute();
        }
    }
}

/// U-E12: the full-list indexes of the tree rows that survive the narrow
/// `query`, in SOURCE order (never re-ranked — the
/// filter-children-keep-parents projection keeps the tree; re-ranking is
/// the score-reorder mechanism this surface is declared DIFFER against in
/// PLAN 018 §4). A row survives iff the shared core scores its FULL
/// relative path — its own name or an ancestor directory component — so a
/// parent component match keeps every file under it, and every surviving
/// file keeps its full indentation (the ancestor chain is never hidden).
/// An empty query survives everything (the full list in its own order).
fn tree_surviving_indexes(
    rows: &[TreeRow],
    query: &str,
    matcher: &mut Matcher,
) -> Vec<usize> {
    if query.is_empty() {
        return (0..rows.len()).collect();
    }
    rows.iter()
        .enumerate()
        .filter_map(|(i, row)| {
            if !super::narrowing::narrow(query, &[row.rel_path.as_str()], matcher).is_empty() {
                Some(i)
            } else {
                None
            }
        })
        .collect()
}

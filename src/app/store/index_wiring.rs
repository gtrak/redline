use super::*;

impl AppStore {
    /// The project-change bus (subscribers: FileView auto-reload, git status).
    pub fn watch_bus(&self) -> &ChangeBus {
        &self.watch_bus
    }

    /// Whether a file watcher is currently active (0 or 1 — exactly one at a
    /// time). Exposed for test diagnostics (the spec's "handle count" check).
    #[allow(dead_code)] // test-only diagnostic accessor; no production caller
    pub fn watcher_count(&self) -> usize {
        usize::from(self.watcher.is_some())
    }

    /// The root the active watcher is watching (exactly one at a time).
    /// Exposed for test diagnostics.
    #[allow(dead_code)] // test-only diagnostic accessor; no production caller
    pub fn watcher_active_root(&self) -> Option<&PathBuf> {
        self.watcher.as_ref().map(|w| &w.root)
    }

    /// Whether the watcher is runtime-suspended (`M-x toggle-watcher`).
    /// Exposed for test diagnostics.
    #[allow(dead_code)] // test-only diagnostic accessor; no production caller
    pub fn watcher_suspended(&self) -> bool {
        self.watch_suspended
    }

    /// Start (or replace) the debounced watcher for the current project with
    /// the default debounce window. No-op when there's no project, watching is
    /// disabled (`auto_reload`), or it's suspended.
    pub fn start_watcher(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            return;
        };
        self.start_watcher_at(&root, DEFAULT_DEBOUNCE);
    }

    /// Start a watcher for `root` with `debounce`. Stops any current watcher
    /// first (exactly one at a time) and skips the spawn when no runtime is
    /// available (plain unit tests have none; production + tokio tests do).
    pub fn start_watcher_at(&mut self, root: &Path, debounce: Duration) {
        if !self.auto_reload || self.watch_suspended {
            self.stop_watcher();
            return;
        }
        if self.watcher.as_ref().is_some_and(|w| w.root.as_path() == root) {
            return; // already watching this root
        }
        self.stop_watcher();
        if tokio::runtime::Handle::try_current().is_err() {
            return; // no runtime (plain unit test): nothing to spawn into
        }
        match crate::app::watcher::start_watch(root, &self.watch_bus, debounce) {
            Some(w) => {
                self.watcher = Some(w);
                tracing::info!(root = %root.display(), "watcher started");
            }
            None => {
                self.minibuffer_message("file watching unavailable (inotify limit?)");
            }
        }
    }

    /// Stop the current watcher (signal the consumer to tear down the
    /// debouncer + notify + pump threads). Idempotent.
    pub fn stop_watcher(&mut self) {
        if let Some(mut w) = self.watcher.take() {
            w.stop();
        }
    }

    /// `M-x toggle-watcher`: suspend/resume live watching. Suspend stops the
    /// watcher (events are dropped, not buffered); resume starts a fresh one.
    pub fn toggle_watcher(&mut self) {
        self.watch_suspended = !self.watch_suspended;
        if self.watch_suspended {
            self.stop_watcher();
            self.minibuffer_message("file watching suspended (M-x toggle-watcher)");
        } else {
            self.start_watcher();
            self.minibuffer_message("file watching resumed (M-x toggle-watcher)");
        }
    }

    /// Take the active watcher out of the store (brief mutable borrow) so the
    /// caller can tear it down WITHOUT holding the store lock across an
    /// await. The caller owns the returned `ActiveWatcher`. `None` when no
    /// watcher is active.
    pub fn take_watcher(&mut self) -> Option<ActiveWatcher> {
        self.watcher.take()
    }

    /// Apply a project change: auto-reload non-locally-owned buffers whose
    /// path changed (preserving the scroll anchor), set the conflict marker
    /// on locally-owned ones, and refresh git status (07's seam).
    ///
    /// This is the FileView / git-status subscription entry point.
    pub fn apply_project_change(&mut self, change: &ProjectChange) {
        if change.is_empty() {
            return;
        }
        let mut reloaded = 0usize;
        let mut conflicts = 0usize;
        for path in &change.paths {
            let key = path.to_string_lossy().into_owned();
            // The first watcher event for a file WE created (e.g. the notes
            // file on `C-x n`) is our own creation, not an external change:
            // consume the marker and ignore this event so it can't flag the
            // buffer "changed on disk" (issue 05, finding 2). A later, genuine
            // external edit is not marked and still conflicts.
            if self.created_paths.remove(&key) {
                continue;
            }
            // The watcher event for a file WE saved in place (plan 005
            // issue 01): suppress only when the on-disk mtime still matches
            // the one recorded right after our write — that is our own
            // write. A genuinely later external write changes the mtime; the
            // marker is consumed either way and the event falls through to
            // the normal conflict / reload handling below.
            if let Some(expected) = self.saved_paths.get(&key) {
                let now = std::fs::metadata(path)
                    .ok()
                    .and_then(|m| m.modified().ok());
                if now == Some(*expected) {
                    self.saved_paths.remove(&key);
                    continue;
                }
                self.saved_paths.remove(&key);
            }
            // Read the buffer's state without holding a borrow across the
            // mutable reload / conflict update below.
            let (matches, locally_owned) = match self.buffers.get(&key) {
                Some(buf) if buf.path.as_ref() == Some(path) => {
                    (true, buf.is_locally_owned())
                }
                _ => (false, false),
            };
            if !matches {
                continue;
            }
            if locally_owned {
                // Conflict: never auto-clobber a locally-owned buffer.
                if let Some(b) = self.buffers.get_mut(&key) {
                    b.changed_on_disk = true;
                }
                conflicts += 1;
            } else if self.reload_buffer(path) {
                reloaded += 1;
            }
        }
        // git status refresh (07's seam): only when a repo is open, so a
        // non-git project never spams a failure message on every change.
        // PART A fix (item 3): skip the expensive refresh when the batch
        // touches no TRACKED file (untracked/ignored files don't appear in
        // the status's tracked sections) — a cheap index pre-filter before
        // the git2 status + diff work.
        if let Some(git) = self.git.as_ref()
            && let Some(project) = self.project.as_ref()
            && git.any_tracked(&change.paths, &project.root)
        {
            self.refresh_magit();
        }
        if conflicts > 0 && reloaded == 0 {
            // The user should know their edit conflicts with a disk change.
            self.minibuffer_message("changed on disk — press g to reload");
        }
        // Incremental symbol-index refresh (issue 05): reparse only the
        // changed files (O(changed files), not a full rebuild).
        self.refresh_index(&change.paths);
    }

    /// Re-read the file buffer at `path` from disk, preserving the scroll
    /// anchor (same line number if it still exists, else clamp). Returns
    /// true when the buffer existed and was reloaded.
    fn reload_buffer(&mut self, path: &Path) -> bool {
        let key = path.to_string_lossy().into_owned();
        self.reload_in_place(&key, path).is_ok()
    }

    /// Re-read `path` into the buffer at `key` IN PLACE: the rope and the
    /// mtime are replaced, but the buffer's IDENTITY survives the reload —
    /// `mode`, `editable`, `is_notes` and `mark` are NOT reset (a disk
    /// reload changes the buffer's CONTENT, not its identity; a replace via
    /// `insert_rope` would rebuild the buffer via `Buffer::new` and
    /// silently drop both the mode and any unsaved edits — the data-loss
    /// path this exists to close). The undo history IS dropped and the
    /// saved-state marker re-established at the fresh sentinel (plan 016
    /// issue 03): the recorded inverse char ranges referred to the rope
    /// that no longer exists, and the new content IS the disk truth, so
    /// the buffer proves clean again. Scroll anchor is preserved (same
    /// line if it still exists, else clamp), the retained tree is dropped,
    /// and the highlight + annotation-anchor caches are refreshed.
    ///
    /// Every **in-place** disk reload flows through this one chokepoint: the
    /// watcher's auto-reload (`reload_buffer`), the reopen re-stat
    /// (`open_project_path`), the external-change confirm's `y`
    /// (`reload_confirm_accept`), and the `g` force-reload
    /// (`reload_current_buffer`).
    ///
    /// NOT exhaustive — TWO other routes replace the rope directly and
    /// therefore do NOT come through here: `toggle_ro_accept` (`buffers.rs`,
    /// the toggle-read-only discard/reload confirm, which also flips `mode`
    /// and `editable` by design) and `replace_buffer_text` (`buffers.rs`, the
    /// notes-buffer sync). Both assign `buf.rope` themselves. Together with
    /// this chokepoint they are the THREE rope-assigning sites in the store
    /// (all of which invalidate rope-relative state, so none silently loses
    /// undo).
    ///
    /// Plan 016 (undo) issue 03 — WIRE SITE 1 OF 3 (the chokepoint): every
    /// in-place disk reload re-reads the file into the rope, so the
    /// recorded inverse char ranges are invalid. This calls the single
    /// `drop_undo_history` helper (which clears the history AND resets the
    /// marker to no evidence), then `mark_fresh` — this site replaces the
    /// content with DISK TRUTH, so the fresh sentinel re-proves the buffer
    /// clean. The other two rope-assigning sites are `toggle_ro_accept`
    /// (`buffers.rs`) and `replace_buffer_text` (`buffers.rs`); the
    /// `undo()` stale-step guard stays as a backstop.
    pub(super) fn reload_in_place(
        &mut self,
        key: &str,
        path: &Path,
    ) -> std::result::Result<(), String> {
        let (rope, mtime) = match load_file(path) {
            Ok(x) => x,
            // File vanished / unreadable: keep the old content; report the
            // failure so the caller can decide (no error spam on every
            // change event is the watcher's policy, stated there).
            Err(e) => return Err(e.to_string()),
        };
        let new_total = rope.len_lines();
        let old_top = self.scroll.get(key).copied().unwrap_or(0);
        let new_top = reload_anchor(old_top, new_total);
        // The rope is about to be replaced from disk: the recorded inverse
        // ranges are dead. Drop the history + marker first (no evidence →
        // modified), then re-prove clean from the disk read.
        self.drop_undo_history(key);
        if let Some(buf) = self.buffers.get_mut(key) {
            buf.rope = rope;
            buf.mtime = mtime;
            buf.mark_fresh();
            buf.changed_on_disk = false;
        }
        self.drop_retained_tree(key);
        self.scroll.insert(key.to_string(), new_top);
        self.ensure_highlight_for_key(key);
        // plan 005 issue 02: a disk reload is a content change — the
        // annotation anchors for this file maintain themselves.
        self.reanchor_for_key(key);
        if self.notes_key().as_deref() == Some(key) {
            self.notes_buffer_dirty = true;
        }
        Ok(())
    }

    /// `g`: force-reload the current file buffer from disk, superseding any
    /// conflict (clears the changed-on-disk marker and the local-modified
    /// flag). The scratch buffer (no path) is handled gracefully.
    pub fn reload_current_buffer(&mut self) {
        let Some(key) = self.buffers.current().map(str::to_string) else {
            self.minibuffer_message("no buffer to reload");
            return;
        };
        let Some(path) = self.buffers.get(&key).and_then(|b| b.path.clone()) else {
            // Scratch (no path): nothing on disk to reload.
            self.minibuffer_message("no file to reload (scratch buffer)");
            return;
        };
        if let Err(e) = self.reload_in_place(&key, &path) {
            self.minibuffer_message(&format!("reload failed: {e}"));
            return;
        }
        // plan 016 issue 03: the chokepoint already dropped the history and
        // re-proved cleanliness from the disk read (the force reload
        // supersedes local edits — the marker is the fresh sentinel).
        self.minibuffer_message("reloaded");
    }

    /// Whether an external-change reload confirm is armed.
    pub fn reload_confirm_active(&self) -> bool {
        self.reload_confirm.is_some()
    }

    /// The external-change confirm's state machine (issue-
    /// external-change-reload; same shape as the quit / toggle-read-only
    /// confirms): `y` reloads the file from disk — discarding the unsaved
    /// edits but KEEPING the buffer's identity (the mode is not reset by a
    /// reload) — `n`, C-g and ESC cancel (the edits and the
    /// `changed_on_disk` marker stay). Every other key is swallowed (no
    /// "unbound key" echo mid-prompt).
    pub fn reload_confirm_key(&mut self, key: Key) {
        if key == Key::ctrl_char('g') || key.code == KeyCode::Escape {
            self.reload_confirm_cancel();
            return;
        }
        let Some(c) = key.char_value() else { return };
        match c {
            'y' => {
                self.reload_confirm_accept();
            }
            'n' => {
                self.reload_confirm_cancel();
            }
            _ => {}
        }
    }

    /// The confirm's `y`: reload from disk in place and re-prove the
    /// buffer clean (the disk content supersedes the edits). A failed
    /// re-read keeps the confirm armed (no silent state half-change, the
    /// `toggle_ro_accept` discipline).
    ///
    /// plan 016 issue 03 (the `y` half of the reload contract): the
    /// chokepoint dropped the history and reset the marker; because this
    /// reload replaced the content with DISK TRUTH, the buffer now proves
    /// clean — the recorded offsets referred to content that no longer
    /// exists, so no step survives to be undone. The `n` half (cancel) is
    /// a no-op on the marker: the history stays intact and a dirty buffer
    /// stays dirty.
    fn reload_confirm_accept(&mut self) {
        let Some(key) = self.reload_confirm.clone() else {
            return;
        };
        let Some(path) = self.buffers.get(&key).and_then(|b| b.path.clone()) else {
            self.reload_confirm = None;
            return;
        };
        if let Err(e) = self.reload_in_place(&key, &path) {
            self.minibuffer_message(&format!("cannot reload: {e}"));
            return;
        }
        self.reload_confirm = None;
        self.minibuffer_message("reloaded");
    }

    /// The confirm's `n` / C-g / ESC: cancel; the unsaved edits and the
    /// changed-on-disk marker stay.
    fn reload_confirm_cancel(&mut self) {
        self.reload_confirm = None;
        self.minibuffer_message("cancel (unsaved edits kept)");
    }

    /// Mark a buffer as locally modified (the light-editing hook; also used
    /// by tests to exercise the conflict logic before the editing UI lands).
    ///
    /// plan 016 issue 03: `locally_modified` is now DERIVED from the
    /// saved-state marker, so "mark modified" means DROP THE EVIDENCE — a
    /// marker of `None` cannot prove the buffer matches the disk state, and
    /// the conservative tie-break reports modified. This is strictly more
    /// conservative than the old unconditional `true`: even a buffer
    /// sitting exactly on its saved position now reads modified until the
    /// next save/load re-establishes the marker.
    #[allow(dead_code)] // test-only seam: exercises the conflict logic before the editing UI lands
    pub fn mark_locally_modified(&mut self, key: &str) {
        if let Some(buf) = self.buffers.get_mut(key) {
            buf.saved_marker = None;
        }
    }

    /// Whether the current buffer has an un-reconciled disk change (the
    /// "changed on disk" conflict marker shown in the file view).
    pub fn current_buffer_changed_on_disk(&self) -> bool {
        self.buffers
            .current_buffer()
            .map(|b| b.changed_on_disk)
            .unwrap_or(false)
    }

    /// Whether the current buffer is editable (drives the "changed on disk"
    /// banner hint: editable buffers reload via `M-x reload-buffer`, plain
    /// file buffers via `g`; on an editable buffer plain `g` self-inserts).
    pub fn current_buffer_editable(&self) -> bool {
        self.buffers
            .current_buffer()
            .map(|b| b.editable)
            .unwrap_or(false)
    }

    /// The status-line mode word for the buffer view (plan 005 issue 01 +
    /// plan 015 issue 02): `Accurate` in the accurate edit mode (the
    /// per-buffer `C-x C-q` opt-in — the one short indicator that makes
    /// the mode toggle visible), `Edit` in annotation mode on an editable
    /// buffer (the notes document, scratch), `Read-only` otherwise. Empty
    /// outside the buffer view (the other views have no buffer-mode
    /// notion; a constant word there would just be noise).
    pub fn buffer_mode_display(&self) -> String {
        if self.top_view() != ViewId::Buffer {
            return String::new();
        }
        match self.buffers.current_buffer() {
            Some(b) => match b.mode {
                BufferMode::Accurate => "Accurate".to_string(),
                BufferMode::Annotation => {
                    if b.editable {
                        "Edit".to_string()
                    } else {
                        "Read-only".to_string()
                    }
                }
            },
            None => String::new(),
        }
    }

    /// Start the initial background index build (issue 05). Builds the full
    /// symbol index over the project's file list using a rayon-parallel
    /// tree-sitter parse on a background thread. No-op when there's no
    /// project, no tokio runtime (plain unit tests), or a job for the
    /// *current* generation is already in flight. If a job from a stale
    /// generation (previous project) is in flight, a new job is started
    /// (the stale job's event will be discarded by `apply_index_event`).
    pub fn start_indexing(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            return;
        };
        if self.ensure_files().is_none() {
            return;
        }
        let files = self.files.get(&root).map(|f| f.files.clone()).unwrap_or_default();
        if files.is_empty() {
            return;
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        // Single-flight per generation: don't start a new job if a job for
        // the current generation is already in flight. A stale-generation
        // job (previous project) is superseded.
        if let Some((_, _, inflight_gen)) = self.indexing
            && inflight_gen == self.index_generation
        {
            return;
        }
        self.indexing = Some((0, files.len(), self.index_generation));
        self.indexing_incremental = false;
        let bus = self.index_bus.clone();
        let root_clone = root.clone();
        let generation = self.index_generation;
        tokio::task::spawn_blocking(move || {
            // Attach a publisher so `build_index` emits a coarse progress
            // event every PROGRESS_STEP files (PART A fix, item 2a). The final
            // event below is unchanged (same generation contract).
            let progress = IndexProgress::new(files.len()).with_publisher(bus.clone(), generation);
            let index = build_index(&root_clone, &files, Some(&progress));
            bus.send(IndexEvent {
                index,
                indexing: false,
                done: files.len(),
                total: files.len(),
                generation,
            });
        });
    }

    /// Spawn a background incremental reindex for the changed paths
    /// (issue 05). Reparses only the changed files (O(changed files)).
    /// No-op when no project or no tokio runtime. If a job for the
    /// *current* generation is in flight, the changed paths are accumulated
    /// in the pending set and coalesced into one incremental job when the
    /// flight clears (no dropped events). If a job from a stale generation
    /// (previous project) is in flight, a new job is started to supersede it.
    pub(super) fn refresh_index(&mut self, changed: &[PathBuf]) {
        if changed.is_empty() {
            return;
        }
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            return;
        };
        // The incremental path must agree with the file walk (the R3
        // invariant, third walker): the watcher reports ignored files
        // too, so filter the changed paths through the SAME gitignore +
        // `graft/` semantics the full build's `FileList` used —
        // otherwise a changed ignored file (a build artifact, a log,
        // anything under `target/`) gets parsed INTO the index the full
        // build correctly excluded it from. Purely subtractive: every
        // non-ignored change (including a deletion of an indexed file)
        // still reaches `refresh_in_place`.
        let changed = Self::indexable_changes(changed, &root);
        if changed.is_empty() {
            return;
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        // Single-flight per generation: if a job for the current generation
        // is in flight, accumulate the changed paths in the pending set so
        // they are coalesced into one job when the flight clears. A job from
        // a stale generation (previous project) is superseded.
        if let Some((_, _, inflight_gen)) = self.indexing
            && inflight_gen == self.index_generation
        {
            for p in changed {
                self.pending_index_changes.insert(p.clone());
            }
            return;
        }
        self.indexing = Some((0, changed.len(), self.index_generation));
        // This is an incremental job: the status line shows `indexing…`
        // (no misleading N/M total) rather than a full-build counter.
        self.indexing_incremental = true;
        let bus = self.index_bus.clone();
        let index = self.index.clone();
        let root_clone = root.clone();
        let changed = changed.to_vec();
        let generation = self.index_generation;
        tokio::task::spawn_blocking(move || {
            let mut idx = index;
            let touched = refresh_in_place(&root_clone, &changed, &mut idx);
            bus.send(IndexEvent {
                index: idx,
                indexing: false,
                done: touched.len(),
                total: touched.len(),
                generation,
            });
        });
    }

    /// The changed paths the incremental index job must process
    /// (this issue): everything EXCEPT the paths the file walk would
    /// never index — a hidden component (the walk's `hidden(true)`),
    /// the `.gitignore` chain from the project root down to the path,
    /// and, in git repos, `.git/info/exclude` + the global git excludes
    /// (the walk's native sources — `model::files::is_gitignored`, the
    /// same per-path predicate the walk and the search pipeline make)
    /// — or under the `graft/` agent cache (issue 05, finding 3).
    /// Purely subtractive: a non-ignored change still reaches
    /// `refresh_in_place`, and a path that no longer exists (a deletion)
    /// is kept whenever it is not ignored, so a deleted INDEXED file
    /// still hits the `remove_file` arm. A directory event is dropped
    /// only when the directory ITSELF is ignored (then its contents
    /// were never indexed either).
    ///
    /// The batch shares ONE `GitignoreMemo` (F2/F3): each directory's
    /// ignore files (`.gitignore` + `.ignore`) and the repo-level
    /// sources are parsed at most once per batch, not once per changed
    /// path, so a 10,000-path burst
    /// stays cheap on the UI thread.
    pub(super) fn indexable_changes(changed: &[PathBuf], root: &Path) -> Vec<PathBuf> {
        let memo: crate::model::files::GitignoreMemo =
            std::sync::Mutex::new(std::collections::HashMap::new());
        changed
            .iter()
            .filter(|p| {
                let is_dir = p.is_dir();
                !crate::model::files::under_graft(root, p, is_dir)
                    && !crate::model::files::is_gitignored(root, p, is_dir, &memo)
            })
            .cloned()
            .collect()
    }

    /// Start the background index build for an EXTERNAL source tree
    /// (plan 006 issue 03; the per-language generalization is plan 011
    /// issue 04): walk the OWNING language's source extensions under
    /// `root` — the language of the LANDED `landed_file` (the registry's
    /// extension map is the authority: Rust `rs`, JS/TS `js/jsx/ts/tsx/
    /// mjs/cjs`, Python `py/pyi`, Go `go`, C `c/h`, C++ `cc/cpp/cxx/hh/hpp/
    /// hxx`, Markdown `md/markdown/mdx` (011-07); any other language:
    /// nothing, the pre-011-04 end state) — and run the project indexer's
    /// machinery (`nav::index::build_index` is root-agnostic) on
    /// `spawn_blocking`; the finished index publishes on the
    /// `CrateIndexBus`. NEVER blocks the landing. Single-flight per
    /// source_root (already cached or already in flight: no-op).
    /// Refused with a clear message above `EXT_INDEX_FILE_CAP` files (no
    /// known registry crate approaches it); silent no-op without a tokio
    /// runtime (plain unit tests).
    pub fn start_crate_indexing(&mut self, root: &Path, landed_file: &Path) {
        if !root.is_dir() {
            return;
        }
        if self.external_indexes.iter().any(|(r, _)| r == root) {
            return; // already indexed
        }
        if self.crate_indexing.iter().any(|(r, _, _)| r == root) {
            return; // a build is already in flight for this root
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        // 011-04: the file-selection predicate is the OWNING language's
        // extension set (006-03's `**/*.rs` is the Rust case of this) —
        // NOT a global "index everything" walk.
        let lang = self
            .grammar_registry
            .language_for(&landed_file.to_string_lossy());
        let files = Self::crate_source_files(root, Self::source_extensions_for(lang));
        if files.is_empty() {
            return;
        }
        if files.len() > EXT_INDEX_FILE_CAP {
            self.minibuffer_message(&format!(
                "crate too large to index ({} source files; cap {})",
                files.len(),
                EXT_INDEX_FILE_CAP
            ));
            return;
        }
        let label = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        // 006-03b item 5: the N/M file counter — the same publisher-less
        // `IndexProgress` the project indexer uses (no `nav/index.rs`
        // change: `new` / `done` / `total` are already public).
        let progress = Arc::new(IndexProgress::new(files.len()));
        self.crate_indexing
            .push((root.to_path_buf(), label.clone(), progress.clone()));
        let bus = self.crate_index_bus.clone();
        let root_clone = root.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let index = build_index(&root_clone, &files, Some(progress.as_ref()));
            bus.send(CrateIndexEvent {
                source_root: root_clone,
                index,
            });
        });
    }

    /// The file-extension set the 011-04 index walk collects for the
    /// OWNING language (`registry.rs`'s extension map is the authority —
    /// each set here is the union of the extensions that map to the
    /// language's grammar family): Rust `rs`; JS/TS (JavaScript /
    /// TypeScript / Tsx) `js/jsx/ts/tsx/mjs/cjs`; Python `py/pyi`; Go
    /// `go`; C `c/h` (011-07 — headers ARE definition sources, the
    /// registry maps `h` to C); C++ `cc/cpp/cxx/hh/hpp/hxx` (011-07, the
    /// full registry map); Markdown `md/markdown/mdx` (011-07 — its
    /// definition query captures headings only); Java `java` /
    /// C# `cs` / Ruby `rb` / Scheme `scm/ss/sls/sld` (new-languages
    /// lane — the registry map per language); Clojure `clj/cljs/cljc`
    /// (runtime-bump lane). Every other language:    /// nothing — the walk finds no files, so no index builds (the same
    /// end state 006-03's `.rs`-only walk had for every non-Rust
    /// landing). The JSON/TOML/YAML/Bash outline queries are not M-.
    /// definition sources here (011-04 judgment, carried by 011-07):
    /// data/config/shell files stay out of the walk.
    pub(super) fn source_extensions_for(lang: LanguageId) -> &'static [&'static str] {
        match lang {
            LanguageId::Rust => &["rs"],
            LanguageId::JavaScript | LanguageId::TypeScript | LanguageId::Tsx =>
                &["js", "jsx", "ts", "tsx", "mjs", "cjs"],
            LanguageId::Python => &["py", "pyi"],
            LanguageId::Go => &["go"],
            LanguageId::C => &["c", "h"],
            LanguageId::Cpp => &["cc", "cpp", "cxx", "hh", "hpp", "hxx"],
            LanguageId::Markdown => &["md", "markdown", "mdx"],
            LanguageId::Java => &["java"],
            LanguageId::CSharp => &["cs"],
            LanguageId::Ruby => &["rb"],
            LanguageId::Scheme => &["scm", "ss", "sls", "sld"],
            LanguageId::Clojure => &["clj", "cljs", "cljc"],
            _ => &[],
        }
    }

    /// The source walk of an external tree for `exts` (the owning
    /// language's extensions, 011-04; crate-relative, forward-slash
    /// paths — the same key shape the project index uses). Nested
    /// `node_modules` trees are skipped (011-04 item 4): they hold OTHER
    /// packages (transitive deps), never the landed dependency's own
    /// source — the js provider's definition walk skips them the same
    /// way; the refusal cap stays the backstop for every other tree.
    ///
    /// Deliberate exception to the project's ignore rules (F5): a landed
    /// dependency lives exactly where the PROJECT index excludes it —
    /// `hidden(true)` skips a `.venv/…` root, and in a git repo the
    /// defaults `parents(true) + git_ignore(true) + git_exclude(true) +
    /// git_global(true) + require_git(true)` make `ignore::Walk::new`
    /// apply the repo exclude file, the global excludes, and any
    /// `.gitignore` pattern that matches a directory AT OR BELOW this
    /// walk root to THIS walk. A pattern matching an ancestor ABOVE the
    /// root does not apply (a project `node_modules/` rule does not
    /// prune a walk rooted inside `node_modules/<pkg>` — executed
    /// pre-F5, such a walk returned its files), so the real pre-F5
    /// failure modes here are hidden pruning, the exclude/global
    /// sources, and below-root `.gitignore` patterns (the dependency is
    /// excluded from the project index so it doesn't pollute project
    /// symbols; once the user lands in it, navigation inside it is the
    /// whole point). So this walk disables `hidden` and every gitignore
    /// source (the `parents` / `git_ignore` / `git_global` /
    /// `git_exclude` / `require_git` flags). The dependency's OWN
    /// `.gitignore` is deliberately not honoured either (it would only
    /// shrink the set of files we can jump to). The explicit rules stay:
    /// the owning language's extension set, the nested `node_modules`
    /// boundary, and — defensively, since `hidden(false)` now walks
    /// dotdirs — the dependency's own `.git`.
    pub(super) fn crate_source_files(root: &Path, exts: &[&str]) -> Vec<String> {
        let mut builder = ignore::WalkBuilder::new(root);
        builder
            .hidden(false)
            .parents(false)
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false)
            .require_git(false);
        builder
            .filter_entry(|e| {
                // A `.git` directory (the dependency may itself be a git
                // checkout; `hidden(false)` now walks dotdirs): its
                // objects/hooks are never source.
                !(e.file_type().is_some_and(|ft| ft.is_dir())
                    && e.file_name() == ".git")
            })
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_type().is_some_and(|t| t.is_file())
                    && e.path()
                        .extension()
                        .is_some_and(|x| {
                            let ext = x.to_string_lossy();
                            exts.iter().any(|wanted| ext.eq_ignore_ascii_case(wanted))
                        })
                    && !Self::under_node_modules(root, e.path())
            })
            .filter_map(|e| {
                e.path()
                    .strip_prefix(root)
                    .ok()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
            })
            .collect()
    }

    /// True when `path` (under `root`) is inside a `node_modules`
    /// DIRECTORY component of its crate-relative path — never for the
    /// file name itself. The root itself may BE a `node_modules` dir
    /// (a landed `node_modules/<pkg>`), so only components AFTER `root`
    /// count.
    fn under_node_modules(root: &Path, path: &Path) -> bool {
        let Ok(rel) = path.strip_prefix(root) else {
            return false;
        };
        let mut comps = rel.components();
        // Drop the file name: only a directory component can be the
        // `node_modules` boundary.
        let _ = comps.next_back();
        comps.any(|c| {
            matches!(c, std::path::Component::Normal(n) if n == "node_modules")
        })
    }

    /// Install a crate-index event into the store (called by the UI's
    /// CrateIndexBus drain). LRU: the root's entry becomes the newest;
    /// the oldest are evicted past `EXT_INDEX_CAP` — and then the current
    /// buffer's owning crate is bumped back to the newest slot, so the
    /// crate the user is IN is never the eviction victim (006-03b item
    /// 1: the arrival is where the eviction happens). The root's
    /// in-flight indicator clears (it can never stick past its own final
    /// event).
    pub fn apply_crate_index_event(&mut self, event: &CrateIndexEvent) {
        let index = Arc::new(std::sync::Mutex::new(event.index.clone()));
        // A duplicate entry for the same root (a stale re-build; never
        // expected — single-flight) would leak: replace, not append.
        self.external_indexes.retain(|(r, _)| r != &event.source_root);
        self.external_indexes
            .push((event.source_root.clone(), index));
        while self.external_indexes.len() > EXT_INDEX_CAP {
            self.external_indexes.remove(0); // evict the oldest
        }
        self.bump_current_crate_recency();
        self.crate_indexing
            .retain(|(r, _, _)| r != &event.source_root);
    }

    /// The `indexing crate <dir> (i/N)…` indicator for the activity
    /// display (empty when idle; the N/M counter is the build's file
    /// progress, 006-03b item 5): mirrors the project indexing indicator
    /// on the status line.
    pub fn crate_indexing_display(&self) -> String {
        self.crate_indexing
            .first()
            .map(|(_, label, p)| {
                format!("indexing crate {label} ({}/{})…", p.done(), p.total())
            })
            .unwrap_or_default()
    }

    /// The cached index handle for an exact `source_root` (the LRU
    /// recency bump — the entry moves to the newest slot): the caller
    /// locks it within its own scope.
    pub(super) fn crate_index_arc(
        &mut self,
        root: &Path,
    ) -> Option<Arc<std::sync::Mutex<SymbolIndex>>> {
        let pos = self.external_indexes.iter().position(|(r, _)| r == root);
        let entry = self.external_indexes.remove(pos?);
        self.external_indexes.push(entry.clone());
        Some(entry.1)
    }

    /// The cached index handle of the crate owning `path` (the
    /// `source_root` it is nested under — there is exactly one: the
    /// cache holds sibling registry crate roots, never a nested pair),
    /// with the LRU recency bump.
    pub(super) fn crate_index_arc_for_path(
        &mut self,
        path: &Path,
    ) -> Option<(PathBuf, Arc<std::sync::Mutex<SymbolIndex>>)> {
        let root = self
            .external_indexes
            .iter()
            .find(|(r, _)| path.starts_with(r))?
            .0
            .clone();
        let arc = self.crate_index_arc(&root)?;
        Some((root, arc))
    }

    /// The crate-relative index key for `path` under `root` —
    /// forward-slash normalized (006-03b item 3): the SAME key shape
    /// `crate_source_files` builds with, so a native-`\`-separated `rel`
    /// never breaks the same-file-first ordering or the `outline` lookup
    /// (the Windows separator case). `None` when `path` is not under
    /// `root` (callers treat that as the empty result, never a panic —
    /// 006-03b item 4).
    pub(super) fn crate_rel(path: &Path, root: &Path) -> Option<String> {
        path.strip_prefix(root)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
    }

    /// 006-03b item 1: keep the current buffer's OWNING crate MRU — the
    /// crate the user is IN is never the LRU eviction victim. Called on
    /// every transition that can put an external buffer current (the
    /// `open_external_path` landing, the buffer-list / picker / jump-back
    /// switches) and at every index arrival (the eviction point, in
    /// `apply_crate_index_event`).
    pub(super) fn bump_current_crate_recency(&mut self) {
        let Some(key) = self.buffers.current().map(String::from) else {
            return;
        };
        let Some(path) = self.buffers.get(&key).and_then(|b| b.path.clone()) else {
            return;
        };
        let Some((root, _)) = self
            .external_indexes
            .iter()
            .find(|(r, _)| path.starts_with(r))
            .cloned()
        else {
            return;
        };
        let _ = self.crate_index_arc(&root);
    }

    /// Install an index event into the store (called by the UI's IndexBus
    /// drain). Discards events from a stale generation (previous project's
    /// job). Replaces the index snapshot and updates the indexing state.
    /// When the flight clears, any accumulated pending paths are coalesced
    /// into one incremental job so no watcher event is dropped.
    pub fn apply_index_event(&mut self, event: &IndexEvent) {
        // Discard events from a stale generation (previous project).
        if event.generation != self.index_generation {
            return;
        }
        if event.indexing {
            // Coarse progress (PART A fix, item 2): update only the status-line
            // counter — do NOT install the (partial/empty) index, so the
            // reader's installed snapshot stays intact until the final event.
            self.indexing = Some((event.done, event.total, event.generation));
        } else {
            self.index = event.index.clone();
            self.indexing = None;
            self.indexing_incremental = false;
            // Coalesce pending paths into one incremental job now that the
            // flight has cleared (Finding 1: no dropped events during flight).
            if !self.pending_index_changes.is_empty() {
                let pending: Vec<PathBuf> = self.pending_index_changes.drain().collect();
                self.refresh_index(&pending);
            }
        }
    }

    /// The indexing indicator for the activity display (empty when idle).
    /// A full build shows an honest, advancing `indexing N/M`; an incremental
    /// reindex shows `indexing…` (PART A fix, item 2b — no misleading total).
    pub fn indexing_display(&self) -> String {
        match &self.indexing {
            // Full builds are long enough to warrant the counter. Incremental
            // refreshes finish in milliseconds; flashing "indexing…" per
            // file-change batch on a busy repo is pure noise, so they render
            // nothing (the single-flight state in `self.indexing` is
            // unaffected -- this is display-only).
            Some((done, total, _)) if *total > 0 && !self.indexing_incremental => {
                format!("indexing {done}/{total}")
            }
            _ => String::new(),
        }
    }

    /// Set the index directly (for tests; bypasses the background thread).
    #[allow(dead_code)] // test-only seam: bypasses the background indexing thread
    pub fn set_index(&mut self, index: SymbolIndex) {
        self.index = index;
    }

    /// The current symbol index (test-only accessor; no production caller).
    #[allow(dead_code)] // test-only accessor; the UI reads index data via views
    pub fn index(&self) -> &SymbolIndex {
        &self.index
    }

    /// Capture the pre-subscribed index receiver (see `index_rx`).
    pub fn set_index_rx(&mut self, rx: tokio::sync::watch::Receiver<crate::nav::index::IndexEvent>) {
        self.index_rx = Some(rx);
    }

    /// Take the pre-subscribed index receiver for the Root drain. Falls back
    /// to subscribing now (tests; the race only exists at startup).
    pub fn take_index_rx(&mut self) -> tokio::sync::watch::Receiver<crate::nav::index::IndexEvent> {
        self.index_rx
            .take()
            .unwrap_or_else(|| self.index_bus.subscribe())
    }
}

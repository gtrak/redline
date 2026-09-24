use super::*;

impl AppStore {
    /// Open (or create) the per-project notes file (`.redline-notes.md`) as
    /// an editable buffer. PART B item 8: the file is locally-owned
    /// (conflict rules identical to issue 04); editing is bounded
    /// (append + backspace, like the commit editor) with explicit save
    /// via `C-x C-s` (`save-buffer`).
    pub fn open_notes(&mut self) {
        const NOTES_REL: &str = ".redline-notes.md";
        let Some(project) = self.project.clone() else {
            self.minibuffer_message("open-notes: no project open");
            return;
        };
        let abs = project.root.join(NOTES_REL);
        // Create the file if it doesn't exist yet. Track whether WE created
        // it: the watcher's first event for a just-created file is our own
        // creation (not an external change), and it must not flag the buffer
        // "changed on disk" (issue 05, finding 2).
        let created_by_us = if abs.exists() {
            false
        } else {
            if std::fs::write(&abs, "# Notes\n").is_err() {
                self.minibuffer_message("open-notes: could not create notes file");
                return;
            }
            true
        };
        let key = abs.to_string_lossy().into_owned();
        if created_by_us {
            self.created_paths.insert(key.clone());
        }
        if self.buffers.get(&key).is_none() {
            match load_file(&abs) {
                Ok((rope, mtime)) => {
                    // Editable + locally-owned: the user types notes here;
                    // disk changes are flagged (conflict marker) rather
                    // than silently overwriting local edits.
                    self.buffers
                        .insert_rope(Some(abs.clone()), rope, mtime, true);
                }
                Err(e) => {
                    self.minibuffer_message(&format!("cannot open notes: {e}"));
                    return;
                }
            }
        }
        // Buffer-relative baseline (plan 015 issue 03, folded in from the
        // 02 gate): this buffer IS the notes document, so its baseline
        // editability rides on `is_notes` and cannot drift when
        // `switch_project_root` changes the root-relative `notes_key()`.
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.is_notes = true;
        }
        self.buffers.set_current(&key);
        // plan 005 issue 02: on (re)open the notes buffer's text is the
        // source of truth — re-parse the notes document from it.
        self.notes_buffer_dirty = true;
        self.record_recent(NOTES_REL);
        self.ensure_highlight();
        self.normalize_top_view();
        self.minibuffer_message("notes: C-x C-s to save");
    }

    /// The absolute buffer key of the per-project notes file
    /// (`.redline-notes.md`), when a project is open.
    pub(super) fn notes_key(&self) -> Option<String> {
        let root = self.project.as_ref()?.root.clone();
        Some(root.join(".redline-notes.md").to_string_lossy().into_owned())
    }

    /// Load / re-parse the notes document (plan 005 issue 02). While the
    /// notes buffer is open, ITS text is the source of truth (re-parsed
    /// when `notes_buffer_dirty` marks a local edit/reload); otherwise the
    /// disk file is (re-read on mtime change). The lazy disk-load path runs
    /// the re-anchor pass over every open buffer ("on load").
    pub(super) fn ensure_notes_doc(&mut self) {
        let Some(key) = self.notes_key() else {
            return;
        };
        if self.buffers.get(&key).is_some() {
            if !self.notes_doc_loaded || self.notes_buffer_dirty {
                let text = self
                    .buffers
                    .get(&key)
                    .map(|b| b.text())
                    .unwrap_or_default();
                self.notes_doc = parse_notes(&text);
                self.notes_doc_loaded = true;
                self.notes_buffer_dirty = false;
            }
        } else {
            let path = PathBuf::from(key.clone());
            let mtime = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok());
            if !self.notes_doc_loaded || mtime != self.notes_doc_mtime {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                self.notes_doc = parse_notes(&text);
                self.notes_doc_loaded = true;
                self.notes_doc_mtime = mtime;
                // On load: the anchors maintain themselves against every
                // open buffer's current content.
                self.reanchor_all_buffers();
            }
        }
    }

    /// The current buffer's annotation key (plan 008 issue 01): the
    /// project-relative path when the buffer's path strips under the
    /// project root, else the absolute path string (external buffers).
    /// `None` only for pathless buffers (scratch).
    pub(super) fn current_annotation_path(&self) -> Option<String> {
        let key = self.buffers.current()?.to_string();
        self.buffer_annotation_path(&key)
    }

    /// The index of the record addressed by the point in the current
    /// buffer (issue-annotation-per-symbol-creation): the record at the
    /// point's CELL `(path, line, col)` — `line` is the point's line and
    /// `col` the point's column AFTER the symbol-start snap (the
    /// symbol's start column when the point captured a symbol, the raw
    /// point column otherwise), so pointing anywhere inside a symbol
    /// addresses that symbol's record. Replaces the old line-keyed
    /// lookup ("the first record on the line"), which is what made `A`
    /// edit an annotated line's first record regardless of where the
    /// cursor was.
    fn record_index_at_point(&self) -> Option<usize> {
        let key = self.buffers.current()?.to_string();
        let line = self.point_line();
        let captured = self.capture_syntax_anchor(&key, line);
        let col = captured
            .as_ref()
            .map(|(_, c)| *c)
            .unwrap_or_else(|| self.point_col());
        self.record_index_at(line, col, captured.is_some())
    }

    /// The index of the record at the cell `(path, line, col)` of the
    /// current buffer — the per-symbol creation/deletion key
    /// (issue-annotation-per-symbol-creation). `symbol_captured`
    /// records whether the point landed on a symbol and gates the
    /// line-tied fallback below:
    /// - a SYMBOL point addresses EXACTLY the record at its cell — other
    ///   records on the line never match, which is what lets `A` create
    ///   a second annotation on an already-annotated line;
    /// - a NO-SYMBOL point (EOL, whitespace, a comment) that misses its
    ///   raw cell falls back to the line's record when EXACTLY ONE record
    ///   on the line is itself line-tied (no syntax anchor) — a line-tied
    ///   record has no symbol to key on, so it keeps the line as its
    ///   effective key (the one case where "at point" cannot be exact).
    ///   Several line-tied records on the line are ambiguous → no match
    ///   (creation), never a guess. Two records can never collide on
    ///   `(path, line, col)`: creation dedupes on the cell.
    fn record_index_at(&self, line: usize, col: usize, symbol_captured: bool) -> Option<usize> {
        let rel = self.current_annotation_path()?;
        let exact = self.notes_doc.entries.iter().position(|e| {
            matches!(e, NotesEntry::Record(a) if a.path == rel && a.line == line && a.col == col)
        });
        if exact.is_some() || symbol_captured {
            return exact;
        }
        let mut line_tied = self
            .notes_doc
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                matches!(e, NotesEntry::Record(a)
                    if a.path == rel && a.line == line && a.syntax.is_none())
            });
        let (idx, _) = line_tied.next()?;
        (line_tied.next().is_none()).then_some(idx)
    }

    /// Re-anchor the annotations of the buffer at `key` against the
    /// buffer's current content (plan 005 issue 02 + plan 007 issue 02):
    /// for each record anchored there, the re-anchor order is
    ///
    /// 1. **syntax anchor** (records carrying a `SyntaxAnchor`): a node of
    ///    the recorded `kind` + `name` found ANYWHERE in the file — exactly
    ///    one match re-anchors to its line (any distance, orphan flag
    ///    clears). Zero or multiple matches fall through (never guess,
    ///    exactly like the text rules' ambiguity rule). The file is
    ///    parsed at most once per pass, and only when at least one record
    ///    for this file HAS a syntax anchor (the common legacy case pays
    ///    no parse).
    /// 2. **exact-line text**: the content at the stored line still matches
    ///    `anchor` exactly (the record stays; an orphan flag clears).
    /// 3. **±25-line text search**: a UNIQUE match re-anchors (updates
    ///    `line`, clears `orphaned`).
    /// 4. **orphan**: zero or multiple text matches set `orphaned = true`
    ///    and leave `line` unchanged (NEVER move an anchor to a guessed
    ///    line).
    ///
    /// Stable and idempotent: a record whose line holds the anchor (or
    /// whose syntax node is unique) is untouched on a second pass.
    pub(super) fn reanchor_for_key(&mut self, key: &str) {
        let Some(rel) = self.buffer_annotation_path(key) else {
            return;
        };
        let Some(buf) = self.buffers.get(key) else {
            return;
        };
        let total = buf.line_count();
        if total == 0 {
            return;
        }
        // The syntax re-anchor index is built BEFORE the mutable pass (it
        // reads `self`); `None` for the legacy / non-Rust / no-anchor cases
        // and the text rules below run exactly as before.
        let syntax_index = self.build_syntax_index_for_key(key, &rel);
        let mut changed = false;
        for entry in self.notes_doc.entries.iter_mut() {
            let Some(a) = entry.as_record_mut() else {
                continue;
            };
            if a.path != rel {
                continue;
            }
            // Order matters (007-02): syntax first — the record's symbol
            // identity resolves to exactly one node in the file and the note
            // re-anchors there regardless of distance (this is what survives
            // a 100-line insertion), even when the stored line still holds
            // the anchor text (the note follows the symbol, not a
            // coincidental text match). When the identity cannot be resolved
            // to exactly one node it falls through to the text rules (never
            // guess — a wrong tie is worse than no tie).
            //
            // Two rules, by the record's stored identity (issue-annotations-
            // symbol-identity):
            // - stage 2 (the record carries an enclosing scope): the name is
            //   UNIQUE within that scope → this is the occurrence. This is
            //   what makes a repeated name follow its own scope — a `foo` in
            //   `bar` never collides with a `foo` in `baz`, or with `bar`
            //   repeated in a sibling scope (the full scope chain disambiguates
            //   same-named definitions in different modules too).
            // - the scope-blind `(kind, name)` uniqueness rule (exactly ONE
            //   occurrence file-wide): the answer for a UNIQUE symbol — it
            //   follows across a 100-line insertion AND across a scope move
            //   (the recorded scope is a disambiguator, not a pin, so an
            //   indented/moved unique symbol still follows) — and for LEGACY
            //   and top-level records (absent scope key → `None`), exactly as
            //   before.
            //
            // Same-scope name repeats are deliberately NOT resolved here: the
            // name is ambiguous within its scope, so it falls through to the
            // text rules (orphan, never a guess). An ordinal tie would be
            // worse — an ordinal shifts when a sibling is added or deleted,
            // migrating the note to a sibling (a wrong tie, which is worse
            // than no tie).
            let resolved = syntax_index.as_ref().and_then(|idx| {
                let sa = a.syntax.as_ref()?;
                if let Some(scope) = &sa.scope {
                    let scope_key = (sa.kind.clone(), sa.name.clone(), scope.clone());
                    if let Some(occ) = idx
                        .by_scope
                        .get(&scope_key)
                        .filter(|occ| occ.len() == 1)
                        .map(|occ| occ[0])
                    {
                        return Some(occ);
                    }
                }
                let key = (sa.kind.clone(), sa.name.clone());
                idx.by_name.get(&key).filter(|occ| occ.len() == 1).map(|occ| occ[0])
            });
            if let Some(occ) = resolved {
                if a.line != occ.line {
                    a.line = occ.line;
                    changed = true;
                }
                // The marker rides the symbol: refresh the record's col to
                // the symbol's START column in its (possibly new) line. A
                // plain insertion above leaves the column unchanged (a no-op
                // here); a re-indent / wrap / moved block moves the symbol's
                // cell and the marker follows it (issue-annotations-
                // symbol-identity, the col-on-symbol requirement).
                if a.col != occ.col {
                    a.col = occ.col;
                    changed = true;
                }
                if a.orphaned {
                    a.orphaned = false;
                    changed = true;
                }
                continue;
            }
            let held = a.line < total
                && buf
                    .line_text(a.line)
                    .map(|t| t.as_ref() == a.anchor.as_str())
                    .unwrap_or(false);
            if held {
                if a.orphaned {
                    a.orphaned = false;
                    changed = true;
                }
                continue;
            }
            // Drift: content search within ±ANNOTATION_REANCHOR_WINDOW.
            // `saturating_add` (gate P3, 017): `a.line` comes from the notes
            // file and `notes_doc` accepts ANY `usize`, so a hand-edited or
            // corrupt record with a huge line made this `+` panic with
            // "attempt to add with overflow". Pre-existing, but the fix is a
            // word and a panic is a panic.
            let lo = a.line.saturating_sub(ANNOTATION_REANCHOR_WINDOW);
            let hi = a
                .line
                .saturating_add(ANNOTATION_REANCHOR_WINDOW)
                .min(total - 1);
            let matches: Vec<usize> = (lo..=hi)
                .filter(|l| {
                    buf.line_text(*l)
                        .map(|t| t.as_ref() == a.anchor.as_str())
                        .unwrap_or(false)
                })
                .collect();
            if matches.len() == 1 {
                a.line = matches[0];
                a.orphaned = false;
                changed = true;
            } else if !a.orphaned {
                // 0 or ambiguous matches: flag, never guess.
                a.orphaned = true;
                changed = true;
            }
        }
        if changed {
            // The re-anchored records are the truth: refresh the notes
            // file (and the open notes buffer, when one is open).
            self.sync_notes_from_doc();
        }
    }

    /// The scope-aware syntax re-anchor index for the buffer at `key`
    /// (plan 007 issue 02, generalised + scope-aware by issue-annotations-
    /// symbol-identity): every identifier-ish node's `(kind, name)` and
    /// `(kind, name, enclosing-scope)` occurrences (line + start column,
    /// document order), built by the crate from ONE parse of the buffer's
    /// current text. `None` unless at least one record with annotation key
    /// `rel` carries a `SyntaxAnchor` AND the buffer's language has at least
    /// one identifier-ish kind (the per-language set from the descriptor
    /// table, derived from each pinned grammar's `node-types.json` — the
    /// Rust-only gate is gone). The common legacy / no-symbol-kind file
    /// (Yaml, Markdown, Plain) pays no parse at all on a re-anchor pass.
    fn build_syntax_index_for_key(
        &self,
        key: &str,
        rel: &str,
    ) -> Option<redline_syntax::node::AnnotationSymbolIndex> {
        let any_syntax = self.notes_doc.entries.iter().any(|e| {
            matches!(e, NotesEntry::Record(a) if a.path == rel && a.syntax.is_some())
        });
        if !any_syntax {
            return None;
        }
        let (path_str, source) = {
            let buf = self.buffers.get(key)?;
            let path = buf.path.as_ref()?;
            (path.to_string_lossy().into_owned(), buf.text())
        };
        let lang = self.grammar_registry.language_for(&path_str);
        // The crate builds the whole-buffer index (one fresh parse — the
        // 007-02 contract: tree reuse / incremental reparse is 007-04's job,
        // not this one's). A language with no identifier-ish kind returns
        // `None` (nothing to anchor; the text rules run, no parse).
        redline_syntax::node::build_annotation_symbol_index(lang, &source)
    }

    /// Re-anchor every open file buffer's annotations (the "on load"
    /// pass: runs after the notes document is (re)loaded from disk).
    fn reanchor_all_buffers(&mut self) {
        let keys: Vec<String> = self
            .buffers
            .list()
            .into_iter()
            .filter(|(_, b)| b.path.is_some())
            .map(|(k, _)| k.to_string())
            .collect();
        for key in keys {
            self.reanchor_for_key(&key);
        }
    }

    /// Collect the annotations for the quit-dump (plan 005 issue 03): one
    /// `DumpAnnotation` per structured record in the notes document (raw
    /// blocks are skipped — they carry no usable record). `line` is
    /// rendered 1-based; `code` is the open buffer's current content at
    /// the anchored line when the anchor holds, otherwise the stored
    /// `anchor` (an orphaned record's line must not be trusted, so its
    /// stored anchor text is the last known line).
    pub fn annotations_for_dump(&mut self) -> Vec<DumpAnnotation> {
        self.ensure_notes_doc();
        let items: Vec<DumpAnnotation> = self
            .notes_doc
            .entries
            .iter()
            .filter_map(|e| {
                let a = e.as_record()?;
                let code = if a.orphaned {
                    a.anchor.clone()
                } else {
                    self.buffer_line_text_for_rel(&a.path, a.line)
                        .unwrap_or_else(|| a.anchor.clone())
                };
                Some(DumpAnnotation {
                    path: a.path.clone(),
                    line: a.line + 1, // 0-based record line -> 1-based dump line
                    code,
                    text: a.text.clone(),
                    orphaned: a.orphaned,
                })
            })
            .collect();
        items
    }

    /// The open buffer's content at 0-based `line` for the annotation key
    /// `rel` (project-relative, or absolute for external buffers), when
    /// such a buffer is open.
    fn buffer_line_text_for_rel(&self, rel: &str, line: usize) -> Option<String> {
        for (key, _) in self.buffers.list() {
            if self.buffer_annotation_path(key).as_deref() == Some(rel) {
                return self
                    .buffers
                    .get(key)
                    .and_then(|b| b.line_text(line))
                    .map(|t| t.into_owned());
            }
        }
        None
    }

    /// Write the notes document back to disk (and into the open notes
    /// buffer when one is open — the real-file surface stays in sync with
    /// the in-memory records). Returns the post-write mtime on success;
    /// the watcher self-write suppression (plan 005 issue 01) records the
    /// write so our own event is not flagged "changed on disk".
    pub(super) fn sync_notes_from_doc(&mut self) -> Option<std::time::SystemTime> {
        let key = self.notes_key()?;
        let path = PathBuf::from(key.clone());
        let text = serialize_notes(&self.notes_doc);
        if std::fs::write(&path, &text).is_err() {
            return None;
        }
        let mtime = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok());
        if let Some(m) = mtime {
            self.saved_paths.insert(key.clone(), m);
            self.notes_doc_mtime = Some(m);
        }
        // The in-memory doc now equals the disk: loaded.
        self.notes_doc_loaded = true;
        if self.buffers.get(&key).is_some() {
            // `replace_buffer_text` dropped the buffer's undo history AND
            // reset its saved-state marker to no evidence (plan 016 issue
            // 03, the third rope-assigning site). This sync just WROTE the
            // serialized doc to disk and is now reflecting that SAME text
            // into the buffer — the content IS the disk truth, so the
            // fresh sentinel re-proves it clean (this is the notes
            // save/load path of the dirty-flag contract; a bare
            // replacement without a preceding write would leave the
            // buffer modified instead).
            self.replace_buffer_text(&key, &text);
            if let Some(buf) = self.buffers.get_mut(&key) {
                if let Some(m) = mtime {
                    buf.mtime = m;
                }
                buf.mark_fresh();
                buf.changed_on_disk = false;
                buf.mark = None;
            }
            self.invalidate_highlight_for_key(&key);
        }
        mtime
    }

    /// 007-04 review P2-1: drop a buffer's retained tree after a content
    /// replacement that is NOT an edit event (`replace_buffer_text`,
    /// read-only accept, disk reload). The `(key, mtime)` key alone
    /// under-protects on coarse-granularity or mtime-preserving
    /// filesystems; removing the tree forces a full parse on the next
    /// highlight — the conservative, always-correct outcome.
    pub(super) fn drop_retained_tree(&mut self, key: &str) {
        if let Some(buf) = self.buffers.get(key) {
            self.highlight_cache
                .retain_remove(&TreeKey::new(key, buf.mtime));
        }
    }

    /// `A` (plan 005 issue 02, per-symbol by issue-annotation-per-symbol-
    /// creation): prompt for an annotation AT POINT in the minibuffer. A
    /// record at the point's cell (path + line + the point's col after
    /// the symbol-start snap, with the line-tied fallback — see
    /// `record_index_at`) pre-fills the prompt (edit); a point with no
    /// record of its own commits a NEW record on RET — even when the
    /// line already carries annotations (a symbol, not the line, is the
    /// unit of annotation). RET commits (record written to
    /// `.redline-notes.md`, cue appears immediately), C-g/ESC cancels.
    pub fn annotate(&mut self) {
        if self.top_view() != ViewId::Buffer {
            self.minibuffer_message("annotate: not in the file view");
            return;
        }
        let Some(key) = self.buffers.current().map(String::from) else {
            return;
        };
        if self.buffers.get(&key).map(|b| b.path.is_none()).unwrap_or(true) {
            self.minibuffer_message("annotate: no file to annotate (scratch)");
            return;
        }
        self.ensure_notes_doc();
        let line = self.point_line();
        // issue-annotation-per-symbol-creation: prefill from the record
        // AT POINT, never from "the line's first record" — the old
        // line-keyed prefill is the user-reported bug (`A` on a second
        // symbol of an annotated line just edited the first one).
        let prefill = self
            .record_index_at_point()
            .and_then(|i| self.notes_doc.entries[i].as_record().map(|a| a.text.clone()));
        self.note_prompt_line = line;
        self.note_prompt_input = prefill.unwrap_or_default();
        self.note_prompt_active = true;
        self.minibuffer_message(&format!("Note: {}", self.note_prompt_input));
    }

    /// The `A` prompt state (for tests).
    #[allow(dead_code)] // public API: used by tests
    pub fn note_prompt_active(&self) -> bool {
        self.note_prompt_active
    }

    /// The `A` prompt's current input (for tests + prefill verification).
    #[allow(dead_code)] // public API: used by tests
    pub fn note_prompt_input(&self) -> &str {
        &self.note_prompt_input
    }

    pub(super) fn note_prompt_char(&mut self, c: char) {
        self.note_prompt_input.push(c);
        self.minibuffer_message(&format!("Note: {}", self.note_prompt_input));
    }

    pub(super) fn note_prompt_backspace(&mut self) {
        self.note_prompt_input.pop();
        self.minibuffer_message(&format!("Note: {}", self.note_prompt_input));
    }

    pub(super) fn note_prompt_cancel(&mut self) {
        if !self.note_prompt_active {
            return;
        }
        self.note_prompt_active = false;
        self.note_prompt_input.clear();
        self.minibuffer_message("note cancelled");
    }

    /// RET in the `A` prompt (plan 005 issue 02, per-symbol by
    /// issue-annotation-per-symbol-creation): commit the record. The
    /// prompt addresses the record at POINT (its cell — see
    /// `record_index_at`): empty input deletes THAT record when there is
    /// one, and cancels when the point addressed none; non-empty input
    /// edits it in place, or creates a new record at the point's cell
    /// when the point addressed none (even on a line that already
    /// carries annotations).
    pub fn note_prompt_confirm(&mut self) {
        if !self.note_prompt_active {
            return;
        }
        let line = self.note_prompt_line;
        let text = self.note_prompt_input.trim().to_string();
        self.note_prompt_active = false;
        self.note_prompt_input.clear();
        let Some(key) = self.buffers.current().map(String::from) else {
            if text.is_empty() {
                self.minibuffer_message("note cancelled");
            }
            return;
        };
        let Some(rel) = self.buffer_annotation_path(&key) else {
            self.minibuffer_message("annotate: no file to annotate (scratch)");
            return;
        };
        let Some(buf) = self.buffers.get(&key) else {
            return;
        };
        let total = buf.line_count();
        if total == 0 {
            self.minibuffer_message("annotate: empty buffer");
            return;
        }
        let line = line.min(total - 1);
        let anchor = buf.line_text(line).map(|t| t.into_owned()).unwrap_or_default();
        // 007-02 + issue-annotations-symbol-identity: capture the syntax
        // anchor at the point (None for non-symbol offsets — the text rules
        // alone keep working). When a symbol IS captured, the record's `col`
        // becomes the symbol's START column (the marker lands on the symbol,
        // not the raw cursor cell); a non-symbol point keeps the raw column.
        let captured = self.capture_syntax_anchor(&key, line);
        let syntax = captured.as_ref().map(|(sa, _)| sa.clone());
        let col = captured
            .as_ref()
            .map(|(_, c)| *c)
            .unwrap_or_else(|| self.point_col());
        // issue-annotation-per-symbol-creation: the record addressed by
        // the prompt is the one AT POINT (its cell + the line-tied
        // fallback) — not "the first record on the line".
        let at_point = self.record_index_at(line, col, captured.is_some());
        if text.is_empty() {
            match at_point {
                Some(idx) => self.delete_annotation_at_index(idx),
                None => self.minibuffer_message("note cancelled"),
            }
            return;
        }
        match at_point {
            Some(i) => {
                // Edit: only the note text changes (the record's stored
                // position stays — the re-anchor pass maintains it). A
                // re-capture at commit refreshes the syntax anchor when the
                // point lands on an identifier-ish node (and the record's
                // col to the symbol's start column); a `None` capture
                // (the cursor slid onto a keyword) keeps the record's
                // existing anchor rather than discarding a good one.
                if let Some(a) = self.notes_doc.entries[i].as_record_mut() {
                    a.text = text.clone();
                    if let Some((sa, c)) = &captured {
                        a.syntax = Some(sa.clone());
                        a.col = *c;
                    }
                }
            }
            None => {
                self.notes_doc
                    .entries
                    .push(NotesEntry::Record(Annotation {
                        path: rel.clone(),
                        line,
                        col,
                        anchor,
                        text: text.clone(),
                        orphaned: false,
                        syntax,
                    }));
            }
        }
        match self.sync_notes_from_doc() {
            Some(_) => {
                let n = self.current_buffer_annotation_count();
                self.minibuffer_message(&format!(
                    "note saved ({} note{} in {})",
                    n,
                    if n == 1 { "" } else { "s" },
                    rel
                ));
            }
            None => self.minibuffer_message("note not saved: could not write the notes file"),
        }
    }

    /// 007-02 + issue-annotations-symbol-identity: capture the syntax anchor
    /// for a record committed at buffer `key`'s line `line`, taken at the
    /// CURRENT POINT's byte offset via 007-01's `node_at`.
    ///
    /// Capture rule (explicit):
    /// - **Offset: the point, not the line's first non-whitespace byte.**
    ///   The point is where the user's attention is (the same target `M-.`
    ///   acts on), and in a language a line's first non-whitespace byte is
    ///   usually a KEYWORD (`fn`, `let`, `if`, `struct`, `def`, `func`, …)
    ///   where `node_at` has no identifier-ish node — the line-head rule
    ///   would silently strip the anchor from exactly the item-header lines
    ///   most worth anchoring. What is stored is the (kind, text) pair, so
    ///   the criterion is landing on a meaningful node, which the point does
    ///   best.
    /// - **Node: the identifier-ish node at the point verbatim** — `kind`
    ///   is `node_at`'s kind and `name` its text. `node_at` returns only
    ///   identifier-ish nodes (007-01's frozen surface: it has no item-kind
    ///   or statement surface), so there is no "statement → enclosing item"
    ///   fallback to take: a local/call identifier anchors on itself, and
    ///   re-anchoring's uniqueness filter keeps it honest (a name shadowed or
    ///   repeated anywhere in the file → ambiguous → text rules, never a
    ///   guess).
    /// - **col-on-symbol**: the returned column is the symbol's START column
    ///   (a char offset), not the raw cursor cell — so `A` pressed mid-token
    ///   lands the marker on the symbol's first cell, not where the cursor
    ///   happened to be.
    /// - **`None`** for languages with no identifier-ish kind, EOL points, and
    ///   offsets with no identifier-ish node — the record then rides the text
    ///   rules alone (today's behavior).
    fn capture_syntax_anchor(&self, key: &str, line: usize) -> Option<(SyntaxAnchor, usize)> {
        let point_col = self.point_col();
        let (path_str, source, byte, line_start) = {
            let buf = self.buffers.get(key)?;
            let path = buf.path.as_ref()?;
            let line_start = buf.try_line_to_byte(line)?;
            let line_text = buf.line_text(line)?;
            // The point's column is a CHAR offset; convert it to a byte
            // offset within the line. `col == line length` (EOL) has no
            // char under it: the byte lands past the last char and
            // `node_at` finds nothing (the honest answer for EOL).
            let byte_in_line = line_text
                .char_indices()
                .nth(point_col)
                .map(|(b, _)| b)
                .unwrap_or(line_text.len());
            (
                path.to_string_lossy().into_owned(),
                buf.text(),
                line_start + byte_in_line,
                line_start,
            )
        };
        let lang = self.grammar_registry.language_for(&path_str);
        // Generalised: capture for every grammar-bearing language that has an
        // identifier-ish kind (the per-language set from the descriptor
        // table); a language with no identifier kind (Yaml, Markdown, Plain)
        // degrades to the text rules exactly as the old Rust gate did.
        if redline_syntax::language::spec(lang).identifier_kinds.is_empty() {
            return None;
        }
        let info = redline_syntax::node::symbol_identity_at(lang, &source, byte)?;
        // col-on-symbol (issue-annotations-symbol-identity): the marker sits
        // on the symbol's START, not the raw cursor cell. The captured node
        // is a single-line identifier-ish node on `line`, so its start column
        // is the char count of source[line_start..info.start_byte]. A `None`
        // answer (EOL / keyword / no identifier node) keeps `point_col()` —
        // a wrong column is worse than the honest cursor cell, so never snap
        // to a neighbouring symbol.
        let col = match source.as_bytes().get(line_start..info.start_byte) {
            Some(prefix) => match std::str::from_utf8(prefix) {
                Ok(s) => s.chars().count(),
                Err(_) => point_col,
            },
            None => point_col,
        };
        Some((
            SyntaxAnchor {
                kind: info.kind,
                name: info.name,
                // A top-level symbol has no enclosing definition to key on
                // (an empty scope): it stores the scope-blind identity,
                // exactly the legacy shape. A scoped symbol stores the chain.
                scope: if info.scope.is_empty() { None } else { Some(info.scope) },
            },
            col,
        ))
    }

    /// `d` in the buffer view (plan 005 issue 02, per-symbol by
    /// issue-annotation-per-symbol-creation): delete the annotation AT
    /// POINT — the record at the point's cell (see `record_index_at`),
    /// not the line's first record — echoing what was removed. A point
    /// that addresses no annotation gets a message (never a
    /// self-insert, never an unbound-key echo).
    pub fn annotate_delete(&mut self) {
        if self.top_view() != ViewId::Buffer {
            self.minibuffer_message("annotate-delete: not in the file view");
            return;
        }
        let Some(key) = self.buffers.current().map(String::from) else {
            return;
        };
        if self.buffers.get(&key).map(|b| b.path.is_none()).unwrap_or(true) {
            self.minibuffer_message("annotate-delete: no file (scratch)");
            return;
        }
        self.ensure_notes_doc();
        match self.record_index_at_point() {
            Some(idx) => self.delete_annotation_at_index(idx),
            None => self.minibuffer_message("no annotation at point"),
        }
    }

    /// Delete the record at `(path, line, col)` (015-01's annotations
    /// picker `d`): the picker holds the location itself, so no current
    /// buffer line is involved. A record no longer present (deleted
    /// elsewhere while the list was open) gets a message, not a panic.
    /// The record is keyed on its own CELL — `col` disambiguates the
    /// several records a line can host (the line-only key resolved the
    /// FIRST record, so deleting the second row dropped the first).
    pub(super) fn delete_annotation_at_path_line_col(&mut self, path: &str, line: usize, col: usize) {
        // Like `annotate_delete`: the disk/buffer notes state is the
        // source of truth, so load/re-parse before deleting (an external
        // edit while the picker was open must not be clobbered).
        self.ensure_notes_doc();
        let idx = self
            .notes_doc
            .entries
            .iter()
            .position(|e| matches!(e, NotesEntry::Record(a) if a.path == path && a.line == line && a.col == col));
        match idx {
            Some(idx) => self.delete_annotation_at_index(idx),
            None => self.minibuffer_message("annotation already deleted"),
        }
    }

    fn delete_annotation_at_index(&mut self, idx: usize) {
        let removed = match self.notes_doc.entries.get(idx) {
            Some(NotesEntry::Record(a)) => a.text.clone(),
            _ => return,
        };
        self.notes_doc.entries.remove(idx);
        match self.sync_notes_from_doc() {
            Some(_) => {
                let chars: Vec<char> = removed.chars().collect();
                let echo: String = chars.iter().take(40).collect();
                let echo = if chars.len() > 40 { format!("{echo}…") } else { echo };
                self.minibuffer_message(&format!("deleted annotation: {echo}"));
            }
            None => self.minibuffer_message(
                "annotation not deleted: could not write the notes file",
            ),
        }
    }

    /// `C-c a` in the buffer view (plan 005 issue 02): toggle the inline
    /// annotation note rows; the margin markers stay either way.
    pub fn annotate_toggle(&mut self) {
        if self.top_view() != ViewId::Buffer {
            return;
        }
        self.show_note_rows = !self.show_note_rows;
        self.minibuffer_message(if self.show_note_rows {
            "note rows: shown"
        } else {
            "note rows: hidden"
        });
    }

    /// The annotation count for the current buffer's file (0 for
    /// pathless buffers or when no record matches its path).
    fn current_buffer_annotation_count(&self) -> usize {
        let Some(rel) = self.current_annotation_path() else {
            return 0;
        };
        self.notes_doc
            .entries
            .iter()
            .filter(|e| matches!(e, NotesEntry::Record(a) if a.path == rel))
            .count()
    }

    /// The status-line annotation count for the current file (`"1 note"` /
    /// `"3 notes"`; empty outside the buffer view or with no annotations).
    pub fn annotation_count_display(&self) -> String {
        if self.top_view() != ViewId::Buffer {
            return String::new();
        }
        let n = self.current_buffer_annotation_count();
        if n == 0 {
            return String::new();
        }
        format!("{n} note{}", if n == 1 { "" } else { "s" })
    }

    /// Append a character to the current buffer at its end (bounded
    /// editing for notes, mirroring the commit-editor's insert path).
    /// Returns `true` when the character was inserted.
    pub fn notes_insert_char(&mut self, c: char) -> bool {
        let key = self.buffers.current().map(String::from);
        let inserted = self.insert_text(&c.to_string());
        // plan 005 issue 02: local edits to the OPEN notes buffer mark the
        // notes document stale (it re-parses on the next ensure).
        if inserted && key.as_deref() == self.notes_key().as_deref() {
            self.notes_buffer_dirty = true;
        }
        inserted
    }

    /// Delete the last character of the current buffer (bounded editing
    /// for notes, mirroring the commit-editor's backspace path).
    pub fn notes_backspace(&mut self) {
        let Some(key) = self.buffers.current() else { return };
        let key = key.to_string();
        let len = self
            .buffers
            .get(&key)
            .map(|b| b.rope.len_chars())
            .unwrap_or(0);
        if len == 0 {
            return;
        }
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.remove((len - 1)..len);
            // plan 016 issue 03: no flag to set — the recorded undo step
            // moved the history position; the derived `locally_modified()`
            // reads modified from the marker comparison.
            if let Some(old_rope) = old_rope {
                self.retain_rope_edit(&key, &old_rope, len - 1, len, "");
            }
            self.invalidate_highlight_for_key(&key);
        }
        // plan 005 issue 02: a notes-buffer edit marks the notes document
        // stale (re-parsed on the next ensure).
        if key == self.notes_key().unwrap_or_default() {
            self.notes_buffer_dirty = true;
        }
    }
}

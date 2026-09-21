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

    /// The index of the FIRST structured-section record anchored at the
    /// current buffer's line `line`, or `None` when the line carries no
    /// annotation.
    fn record_index_for_line(&self, line: usize) -> Option<usize> {
        let rel = self.current_annotation_path()?;
        self.notes_doc
            .entries
            .iter()
            .position(|e| matches!(e, NotesEntry::Record(a) if a.path == rel && a.line == line))
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
            // Order matters (007-02): syntax first — a unique (kind, name)
            // node ANYWHERE in the file re-anchors to its line regardless
            // of distance (this is what survives a 100-line insertion),
            // even when the stored line still holds the anchor text (the
            // note follows the symbol, not a coincidental text match).
            // Zero or multiple matches fall through to the text rules.
            if let Some(idx) = &syntax_index
                && let Some(sa) = a.syntax.as_ref()
                && let Some(lines) = idx.get(&(sa.kind.clone(), sa.name.clone()))
                && lines.len() == 1
            {
                if a.line != lines[0] {
                    a.line = lines[0];
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
            let lo = a.line.saturating_sub(ANNOTATION_REANCHOR_WINDOW);
            let hi = (a.line + ANNOTATION_REANCHOR_WINDOW).min(total - 1);
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

    /// The syntax re-anchor index for the buffer at `key` (plan 007
    /// issue 02): `(kind, name) → the 0-based lines of every node of that
    /// kind + text`, built from ONE parse of the buffer's current text.
    /// `None` unless at least one record with annotation key `rel` carries
    /// a `SyntaxAnchor` AND the buffer's language parses (Rust today — the
    /// same single grammar pin 007-01 uses, via `queries::language_for`):
    /// the common legacy file pays no parse at all on a re-anchor pass.
    fn build_syntax_index_for_key(
        &self,
        key: &str,
        rel: &str,
    ) -> Option<HashMap<(String, String), Vec<usize>>> {
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
        if lang != redline_syntax::registry::LanguageId::Rust {
            return None;
        }
        // One fresh parse of the buffer's rope (the 007-02 contract: tree
        // reuse / incremental reparse is 007-04's job, not this one's).
        let language = redline_syntax::queries::language_for(lang)?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).ok()?;
        let tree = parser.parse(source.as_bytes(), None)?;
        let mut index: HashMap<(String, String), Vec<usize>> = HashMap::new();
        Self::collect_syntax_anchor_nodes(tree.root_node(), source.as_bytes(), &mut index);
        Some(index)
    }

/// Walk the parse tree collecting every NAMED node of one of 007-01's
/// identifier-ish kinds as `(kind, text) → lines` (plan 007 issue 02).
/// Restricting to those kinds keeps the walk cheap and correct: a
/// captured `SyntaxAnchor.kind` always belongs to the closed set, so no
/// other node kind can ever contribute a false match. `::` paths stay
/// whole (a `scoped_identifier` is one node — exactly as `node_at`
/// returns it). Multiple nodes may share a line (`x = x`); each is
/// counted, so uniqueness is over NODES, never lines.
fn collect_syntax_anchor_nodes(
    node: tree_sitter::Node,
    source: &[u8],
    index: &mut HashMap<(String, String), Vec<usize>>,
) {
    if node.is_named()
        && Self::is_syntax_anchor_kind(node.kind())
        && let Ok(text) = node.utf8_text(source)
    {
        index
            .entry((node.kind().to_string(), text.to_string()))
            .or_default()
            .push(node.start_position().row);
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            Self::collect_syntax_anchor_nodes(child, source, index);
        }
    }
}

/// The identifier-ish node kinds `node_at` (007-01) can return — the
/// closed set a captured `SyntaxAnchor.kind` belongs to. Kept in
/// lockstep with `src/syntax/node.rs`'s `is_rust_identifier_kind`
/// (007-01 is frozen — mirrored here, not shared).
fn is_syntax_anchor_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "field_identifier"
            | "type_identifier"
            | "scoped_identifier"
            | "scoped_type_identifier"
            | "primitive_type"
    )
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
            self.replace_buffer_text(&key, &text);
            if let Some(buf) = self.buffers.get_mut(&key) {
                if let Some(m) = mtime {
                    buf.mtime = m;
                }
                buf.locally_modified = false;
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

    /// `A` (plan 005 issue 02): prompt for an annotation on the line at
    /// point in the minibuffer. An existing record on the line pre-fills
    /// the prompt (edit); RET commits (record written to
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
        let prefill = self.notes_doc.entries.iter().find_map(|e| {
            e.as_record()
                .filter(|a| a.line == line && a.path == self.current_annotation_path().as_deref().unwrap_or(""))
                .map(|a| a.text.clone())
        });
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

    /// RET in the `A` prompt (plan 005 issue 02): commit the record.
    /// Empty input on an existing record deletes it; empty input on a
    /// fresh prompt cancels.
    pub fn note_prompt_confirm(&mut self) {
        if !self.note_prompt_active {
            return;
        }
        let line = self.note_prompt_line;
        let text = self.note_prompt_input.trim().to_string();
        self.note_prompt_active = false;
        self.note_prompt_input.clear();
        if text.is_empty() {
            if self.record_index_for_line(line).is_some() {
                self.delete_annotation_at(line);
            } else {
                self.minibuffer_message("note cancelled");
            }
            return;
        }
        let Some(key) = self.buffers.current().map(String::from) else {
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
        let col = self.point_col();
        // 007-02: capture the syntax anchor at the point (None for
        // non-Rust / keyword offsets — the text rules alone keep working).
        let syntax = self.capture_syntax_anchor(&key, line);
        let existing = self.notes_doc.entries.iter().position(|e| {
            matches!(e, NotesEntry::Record(a) if a.path == rel && a.line == line)
        });
        match existing {
            Some(i) => {
                // Edit: only the note text changes (the record's stored
                // position stays — the re-anchor pass maintains it). A
                // re-capture at commit refreshes the syntax anchor when the
                // point lands on an identifier-ish node; a `None` capture
                // (the cursor slid onto a keyword) keeps the record's
                // existing anchor rather than discarding a good one.
                if let Some(a) = self.notes_doc.entries[i].as_record_mut() {
                    a.text = text.clone();
                    if let Some(sa) = syntax.clone() {
                        a.syntax = Some(sa);
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

    /// 007-02: capture the syntax anchor for a record committed at buffer
    /// `key`'s line `line`, taken at the CURRENT POINT's byte offset via
    /// 007-01's `node_at`.
    ///
    /// Capture rule (explicit):
    /// - **Offset: the point, not the line's first non-whitespace byte.**
    ///   The point is where the user's attention is (the same target `M-.`
    ///   acts on), and in Rust a line's first non-whitespace byte is
    ///   usually a KEYWORD (`fn`, `let`, `if`, `struct`) where `node_at`
    ///   has no identifier-ish node — the line-head rule would silently
    ///   strip the anchor from exactly the item-header lines most worth
    ///   anchoring. What is stored is the (kind, text) pair, so the
    ///   criterion is landing on a meaningful node, which the point does
    ///   best.
    /// - **Node: the identifier-ish node at the point verbatim** — `kind`
    ///   is `node_at`'s kind and `name` its text. `node_at` returns only
    ///   identifier-ish nodes (007-01's frozen surface: it has no
    ///   item-kind or statement surface), so there is no "statement →
    ///   enclosing item" fallback to take: a local/call identifier
    ///   anchors on itself, and re-anchoring's uniqueness filter keeps it
    ///   honest (a name shadowed or repeated anywhere in the file →
    ///   ambiguous → text rules, never a guess).
    /// - **`None`** for non-Rust buffers, EOL points, and offsets with no
    ///   identifier-ish node — the record then rides the text rules alone
    ///   (today's behavior).
    fn capture_syntax_anchor(&self, key: &str, line: usize) -> Option<SyntaxAnchor> {
        let col = self.point_col();
        let (path_str, source, byte) = {
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
                .nth(col)
                .map(|(b, _)| b)
                .unwrap_or(line_text.len());
            (
                path.to_string_lossy().into_owned(),
                buf.text(),
                line_start + byte_in_line,
            )
        };
        let lang = self.grammar_registry.language_for(&path_str);
        if lang != redline_syntax::registry::LanguageId::Rust {
            return None;
        }
        let info = redline_syntax::node::node_at(lang, &source, byte)?;
        Some(SyntaxAnchor {
            kind: info.kind,
            name: info.text,
        })
    }

    /// `d` in the buffer view (plan 005 issue 02): delete the annotation
    /// on the line at point, echoing what was removed. A line with no
    /// annotation gets a message (never a self-insert, never an unbound-key
    /// echo).
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
        let line = self.point_line();
        match self.record_index_for_line(line) {
            Some(idx) => self.delete_annotation_at_index(idx),
            None => self.minibuffer_message("no annotation on this line"),
        }
    }

    /// Delete the record anchored at the current buffer's line `line` (the
    /// empty-RET edit-path delete).
    fn delete_annotation_at(&mut self, line: usize) {
        match self.record_index_for_line(line) {
            Some(idx) => self.delete_annotation_at_index(idx),
            None => self.minibuffer_message("note cancelled"),
        }
    }

    /// Delete the record at `(path, line)` (015-01's annotations picker
    /// `d`): the picker holds the location itself, so no current buffer
    /// line is involved. A record no longer present (deleted elsewhere
    /// while the list was open) gets a message, not a panic.
    pub(super) fn delete_annotation_at_path_line(&mut self, path: &str, line: usize) {
        // Like `annotate_delete`: the disk/buffer notes state is the
        // source of truth, so load/re-parse before deleting (an external
        // edit while the picker was open must not be clobbered).
        self.ensure_notes_doc();
        let idx = self
            .notes_doc
            .entries
            .iter()
            .position(|e| matches!(e, NotesEntry::Record(a) if a.path == path && a.line == line));
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
            buf.locally_modified = true;
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

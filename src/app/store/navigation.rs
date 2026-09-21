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

    /// `M-.`: jump to the definition of the symbol UNDER THE POINT
    /// (plan 006 issue 02 selection rule).
    ///
    /// Selection rule (the user's report: "doesn't use my cursor position"): the
    /// lookup keys off the point's COLUMN, not "every identifier on the line".
    /// 1. The identifier run at the point's column on the point line (a cursor
    ///    parked right after the name counts, the usual call-site spot); when
    ///    it sits inside a `::`-path (`tokio::spawn`), the full path token is
    ///    kept for the tooling-resolver fall-through and the workspace index —
    ///    which is name-keyed — is tried with BOTH the last segment and the
    ///    full path. A Rust `self.<member>` carries the receiver (`self.
    ///    <member>`, 010-01): before the name-keyed index is tried, the
    ///    member resolves via the LEXICALLY ENCLOSING impl's type (field →
    ///    the struct's field line; method → the impl method's line), and an
    ///    empty self-resolution degrades to the bare `<member>` lookup below
    ///    (byte-for-byte; generics / no-impl / unknown member never guess).
    /// 2. Same-file definitions are first-class (the old cross-file filter made
    ///    the normal struct+impl-in-one-file case unjumpable): candidates are
    ///    ordered same-file-first, then (file, line, name). Exactly one → jump
    ///    directly (same-file OR cross-file); several → the Xref picker so the
    ///    user chooses.
    /// 3. No symbol-at-point with a definition → the enclosing-symbol fallback
    ///    (unchanged): the enclosing symbol's definitions take over, and a
    ///    workspace hit on THAT never triggers the resolver (no resolver spam).
    /// 4. Still nothing (and the point sits on a symbol) → tooling-resolver
    ///    fall-through (plan 006): `SymbolContext { workspace_root,
    ///    symbol: <path-shaped token>, from_file }` runs OFF the input path
    ///    (`spawn_blocking` + `ResolveBus`, like the symbol indexer) because
    ///    `cargo fetch` is a network shell-out that must never block a keypress.
    pub fn xref_find_definitions(&mut self) {
        // Get the current file's project-relative path.
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let Some(buf) = self.buffers.get(&key) else {
            self.minibuffer_message("no buffer");
            return;
        };
        // An OWNED path: the `buf` borrow must not span the `&mut self`
        // calls below (the external-buffer navigation, 006-03).
        let path = match &buf.path {
            Some(p) => p.clone(),
            None => {
                self.minibuffer_message("no file (scratch buffer)");
                return;
            }
        };
        let Some(project) = self.project.as_ref() else {
            self.minibuffer_message("no project");
            return;
        };
        let Ok(rel) = path.strip_prefix(&project.root) else {
            // 006-03: an EXTERNAL (registry / tooling) buffer navigates
            // within its OWN crate's index (keyed against the crate's
            // source_root); a crate miss keeps the resolver fall-through
            // (the origin project's metadata — unchanged semantics).
            // A non-external buffer outside the root keeps the pre-006-03
            // refusal.
            if self.external_buffers.contains(&key) {
                self.xref_in_external_buffer(&key, &path);
            } else {
                self.minibuffer_message("buffer not in project");
            }
            return;
        };
        let rel = rel.to_string_lossy().into_owned();

        // 006-02b item 2: supersede any in-flight tooling resolve — a
        // successful workspace hit must not be clobbered by a stale event
        // from a superseded request that lands later. The fall-through
        // path re-bumps in `start_symbol_resolution`, which is fine (a
        // generation only needs to differ; the event carries its own).
        self.resolve_generation += 1;

        let line = self.point_line();
        let line_text = buf.line_text(line).unwrap_or_default();
        // (1) The symbol under the point: identifier run around the point's
        // column, plus the path token it belongs to (raw, for the
        // resolver). 011-06: language-aware — in a non-Rust buffer the
        // token is the whole dotted path when the point sits in the
        // language's path container, else the bare extraction.
        let lang = self.grammar_registry.language_for(&path.to_string_lossy());
        let at = Self::symbol_at_point(lang, &line_text, self.point_col());

        let defs: Option<Vec<crate::nav::index::Location>> = at
            .as_ref()
            .and_then(|(ident, path_token)| {
                // 010-01 (plan 010 Shape A, rung 1): the Rust self-receiver
                // pre-step — `self.<member>` resolves via the LEXICALLY
                // ENCLOSING impl's type (field → the struct's field line,
                // method → the impl method's line), same-file first. An
                // empty result degrades to today's bare-`<member>` index
                // lookup below (byte-for-byte; never a guess).
                if lang == LanguageId::Rust
                    && let Some(member) = path_token.strip_prefix("self.")
                    && !member.is_empty()
                {
                    let source = buf.rope.to_string();
                    if let Some(byte) = point_byte_offset(&buf.rope, line, self.point_col()) {
                        let cands =
                            Self::self_receiver_candidates(&self.index, &rel, &source, byte, member);
                        if !cands.is_empty() {
                            return Some(cands);
                        }
                    }
                }
                // 010-03 (plan 010 Shape A, rung 3): the local-binding
                // pre-step — `x.<member>` / `x.<member>()` resolves via
                // the binding's WRITTEN-DOWN type (a `let x: Type`
                // annotation or a `let x = Type { … }` literal) recorded
                // in an enclosing scope, through the same field / method
                // tables as the self pre-step above. An empty result
                // keeps today's bare-`<member>` behavior byte-for-byte
                // (the extraction's path token never changed; never a
                // guess — unannotated bindings are never inferred).
                if lang == LanguageId::Rust
                    && let Some((receiver, member_col)) =
                        Self::rust_dotted_receiver(&line_text, self.point_col())
                {
                    let source = buf.rope.to_string();
                    if let Some(byte) = point_byte_offset(&buf.rope, line, member_col) {
                        let cands = Self::local_binding_candidates(
                            &self.index,
                            &rel,
                            &source,
                            byte,
                            &receiver,
                            ident,
                        );
                        if !cands.is_empty() {
                            return Some(cands);
                        }
                    }
                }
                Self::xref_definition_candidates(&self.index, ident, path_token, &rel)
            });
        let defs = defs.unwrap_or_default();

        let lookup_name: String;
        let defs: Vec<crate::nav::index::Location> = if !defs.is_empty() {
            // (2) Symbol-at-point with definitions: first candidate after the
            // same-file-first order (the picker's lookup label).
            lookup_name = defs[0].symbol.name.clone();
            defs
        } else {
            // (3) Enclosing-symbol fallback (unchanged: by line, not by the
            // point's column).
            let outline = self.index.outline(&rel);
            let Some(sym) = crate::nav::index::enclosing_symbol(outline, line) else {
                // (4) Nothing the workspace knows about under/near the point:
                // fall through to the tooling resolver when the point sits on
                // a symbol (otherwise behave as before: no symbol under point).
                match &at {
                    Some((_, path_token)) => self.start_symbol_resolution(path_token, &rel),
                    None => self.minibuffer_message("no symbol under point"),
                }
                return;
            };
            lookup_name = sym.name.clone();
            self.index.definitions_of(&lookup_name)
        };

        if defs.is_empty() {
            // The enclosing symbol has no indexed definition: same (4) seam,
            // but the point's own token (path-shaped) is what the resolver
            // gets — not the enclosing name.
            match &at {
                Some((_, path_token)) => self.start_symbol_resolution(path_token, &rel),
                None => self.minibuffer_message(&format!("no definition for `{lookup_name}`")),
            }
            return;
        }

        if defs.len() == 1 {
            // Unique (same-file or cross-file): capture origin, navigate,
            // record jump.
            let origin = self.current_jump_entry();
            let def = &defs[0];
            self.open_path(&def.file);
            // Move the point to the definition's line; the jump-landing
            // recenter positions the window (plan 004 issue 07).
            self.set_point_line(def.symbol.line);
            self.recenter_landing();
            self.ensure_highlight();
            self.record_jump(origin, "M-.");
            self.minibuffer_message(&format!("jumped to {}: {}", def.file, def.symbol.line + 1));
        } else {
            // Ambiguous: open the Xref picker (same-file candidates first).
            // The jump entry is recorded when the user selects a candidate
            // (run_selected for Xref).
            self.xref_crate_root = None;
            self.xref_lookup_name = lookup_name;
            let candidates: Vec<PickerCandidate> = defs
                .iter()
                .map(|d| PickerCandidate {
                    name: format!("{}:{}", d.file, d.symbol.line + 1),
                    display: format!("{}:{}  [{}] {}", d.file, d.symbol.line + 1, d.symbol.kind.tag(), d.symbol.name),
                    label: d.symbol.name.clone(),
                    detail: format!("[{}] {}:{}", d.symbol.kind.tag(), d.file, d.symbol.line + 1),
                    docs: String::new(),
                    category: "xref".to_string(),
                })
                .collect();
            self.open_picker(PickerKind::Xref, "Definition: ", candidates);
        }
    }

    /// (010-04, plan 010 Shape A rung 4) find-implementations — the
    /// read-only view of the Rung 1 impl tables: a PICKER of the
    /// `impl <Trait> for <Type>` blocks implementing the trait at point
    /// (file + impl line, the self type), from the name-keyed trait map
    /// built in the SAME index pass as the Rust tables (zero extra parse;
    /// the 010-01 same-content-refresh discipline applies to the map —
    /// pinned in `nav/index.rs`). The trait at point reuses the M-.
    /// extraction (`symbol_at_point`, byte-for-byte); the map is tried
    /// with both the path token and the bare identifier (the index is
    /// name-keyed — `impl Display` vs `impl std::fmt::Display`, each
    /// spelling is its own key, exactly like the M-. candidate lookup).
    ///
    /// Honest degradation, byte-for-byte — the EXISTING bare-symbol M-.
    /// lookup runs (index / enclosing symbol / tooling fall-through) when:
    /// no symbol at point; no table entry for the trait (no Rust file
    /// impls it, or a non-Rust buffer — the tables only exist for Rust);
    /// or the impl's captured trait text is generic (`Display<T>` — a
    /// bare `Display` at point never matches the generic key; never a
    /// guess). The picker's RET reuses the Xref jump path (the same
    /// `xref_crate_root` seam, so external buffers land read-only).
    pub fn find_implementations(&mut self) {
        // Get the current file's project-relative path (the M-. guards).
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let Some(buf) = self.buffers.get(&key) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let path = match &buf.path {
            Some(p) => p.clone(),
            None => {
                self.minibuffer_message("no file (scratch buffer)");
                return;
            }
        };
        let Some(project) = self.project.as_ref() else {
            self.minibuffer_message("no project");
            return;
        };
        let lang = self.grammar_registry.language_for(&path.to_string_lossy());
        // The point's line text must be read (owned) BEFORE the crate-root
        // match below (its `crate_index_arc_for_path` takes `&mut self`;
        // the `buf` borrow must not span it — the 006-03 borrow rule).
        let line = self.point_line();
        let line_text: String = buf
            .line_text(line)
            .map(|c| c.into_owned())
            .unwrap_or_default();
        // The index source: the project index (project files) or the
        // owning crate's index (external buffers, 006-03); a non-external
        // buffer outside the root keeps the pre-006-03 refusal.
        let crate_root: Option<PathBuf> = match path.strip_prefix(&project.root) {
            Ok(_) => None,
            Err(_) if self.external_buffers.contains(&key) => {
                self.crate_index_arc_for_path(&path).map(|(root, _)| root)
            }
            Err(_) => {
                self.minibuffer_message("buffer not in project");
                return;
            }
        };
        // The trait at point: the SAME extraction as M-. (byte-for-byte).
        let Some((ident, path_token)) =
            Self::symbol_at_point(lang, &line_text, self.point_col())
        else {
            self.minibuffer_message("no symbol under point");
            return;
        };
        // The trait-keyed map is name-keyed: try the path token and the
        // bare identifier (deduped — they are equal for a bare trait).
        let mut keys: Vec<String> = vec![path_token.clone()];
        if path_token != ident {
            keys.push(ident.clone());
        }
        let mut locations: Vec<crate::nav::index::TraitImplLocation> = Vec::new();
        for k in &keys {
            let locs: Vec<crate::nav::index::TraitImplLocation> = match &crate_root {
                Some(root) => self
                    .crate_index_arc(root)
                    .map(|arc| arc.lock().unwrap().trait_impl_locations(k))
                    .unwrap_or_default(),
                None => self.index.trait_impl_locations(k),
            };
            for l in locs {
                if !locations.iter().any(|e| e.file == l.file && e.impl_line == l.impl_line) {
                    locations.push(l);
                }
            }
        }
        if locations.is_empty() {
            // Honest degradation: the EXISTING bare-symbol lookup,
            // byte-for-byte (the M-. path — index, enclosing symbol,
            // tooling fall-through, exactly as M-. does it).
            if crate_root.is_some() {
                self.xref_in_external_buffer(&key, &path);
            } else {
                self.xref_find_definitions();
            }
            return;
        }
        self.xref_crate_root = crate_root;
        self.impls_keys = keys;
        // The picker's candidate list comes from the SAME re-computation
        // the query editing uses (`impls_candidates`) — the initial list
        // and the filtered re-derivation cannot drift apart.
        let candidates = self.impls_candidates();
        self.open_picker(PickerKind::Impls, "Impls: ", candidates);
    }

    /// (M-., selection rule 2) The definition candidates for the symbol at the
    /// point in `index`: the index tried with BOTH the last segment and the
    /// full `::`-path (the index is name-keyed), deduplicated, ordered
    /// SAME-FILE-FIRST (`rel`) then (file, line, name) — a same-file match
    /// (struct + impl in one file is the normal case) wins the direct jump,
    /// and when several remain the picker lists them in that order. Shared
    /// by the project index and the external crate indexes (006-03).
    fn xref_definition_candidates(
        index: &SymbolIndex,
        ident: &str,
        path_token: &str,
        rel: &str,
    ) -> Option<Vec<crate::nav::index::Location>> {
        if ident.is_empty() {
            return None;
        }
        let mut all: Vec<crate::nav::index::Location> = index.definitions_of(ident);
        if path_token != ident {
            all.extend(index.definitions_of(path_token));
        }
        all.sort_by(|a, b| {
            // Same-file candidates first (false < true), then deterministic
            // (file, line, name).
            (a.file != rel).cmp(&(b.file != rel)).then_with(|| {
                (a.file.as_str(), a.symbol.line, &a.symbol.name).cmp(&(b.file.as_str(), b.symbol.line, &b.symbol.name))
            })
        });
        all.dedup_by(|a, b| a.file == b.file && a.symbol.line == b.symbol.line && a.symbol.name == b.symbol.name);
        (!all.is_empty()).then_some(all)
    }

    /// (010-01, plan 010 Shape A rung 1) The M-. self-receiver candidates:
    /// `self.<member>` resolves via the LEXICALLY ENCLOSING impl's self
    /// type ("which impl am I lexically inside" — no expression typing):
    /// - a FIELD → the struct's `field_declaration` line, from the index's
    ///   cross-file field locations (the same file comes out first after
    ///   the ordering below);
    /// - a METHOD → the impl method's line from the SAME FILE's impl table
    ///   (an impl block is lexically one file — its methods are never
    ///   cross-file, so only `rel`'s tables are consulted);
    /// - both are gathered when a name is both a field and a method (the
    ///   picker lets the user choose — never guessed away).
    ///
    /// `Vec::new()` (the caller degrades to today's bare-`<member>`
    /// behavior) whenever: the enclosing impl can't be found, its self
    /// type isn't a PLAIN identifier (generics — `impl<T> Foo<T>` —
    /// degrade; never a guess), or no field/method named `<member>` is
    /// recorded for that type. Pure over the index (testable in isolation,
    /// shared by the project and the external crate paths).
    fn self_receiver_candidates(
        index: &SymbolIndex,
        rel: &str,
        source: &str,
        byte: usize,
        member: &str,
    ) -> Vec<crate::nav::index::Location> {
        let Some(type_name) = crate::syntax::queries::rust_self_type_at(source, byte) else {
            return Vec::new();
        };
        Self::type_member_candidates(index, rel, &type_name, member)
    }

    /// (010-03, plan 010 Shape A rung 3) The M-. local-binding
    /// candidates: `x.<member>` / `x.<member>()` resolves when the
    /// binding `x` has a WRITTEN-DOWN type in the enclosing scope — a
    /// `let x: Type` annotation or a `let x = Type { … }` struct
    /// literal (innermost scope wins; within a scope the last `let`
    /// before the use wins — the shadow rule) — and then exactly like
    /// the self pre-step: field → the struct's `field_declaration`
    /// line (cross-file via the index), method → the impl method's line
    /// in the same file.
    ///
    /// `Vec::new()` (the caller degrades to today's bare-`<member>`
    /// behavior byte-for-byte) when the file has no binding table, the
    /// binding's type isn't written down anywhere in the scope chain
    /// (never inferred), or no field/method named `<member>` is recorded
    /// for that type (non-struct types, `&T { … }`, generics, … all
    /// degrade here or earlier).
    fn local_binding_candidates(
        index: &SymbolIndex,
        rel: &str,
        source: &str,
        byte: usize,
        receiver: &str,
        member: &str,
    ) -> Vec<crate::nav::index::Location> {
        let Some(tables) = index.tables(rel) else {
            return Vec::new();
        };
        let Some(type_name) =
            crate::syntax::queries::rust_binding_type_at(tables, source, byte, receiver)
        else {
            return Vec::new();
        };
        Self::type_member_candidates(index, rel, &type_name, member)
    }

    /// The shared member gathering of the 010-01 / 010-03 pre-steps: for
    /// type `type_name`, the member `member` — a FIELD → the struct's
    /// `field_declaration` lines from the index's cross-file field
    /// locations (the same file orders first below), a METHOD → the
    /// same file's impl tables (an impl's methods are lexically one
    /// file); both are gathered when a name is both (the picker lets the
    /// user choose — never guessed away), deduped, same-file-first.
    fn type_member_candidates(
        index: &SymbolIndex,
        rel: &str,
        type_name: &str,
        member: &str,
    ) -> Vec<crate::nav::index::Location> {
        let mk = |file: String, kind: crate::syntax::queries::SymbolKind, line: usize| {
            crate::nav::index::Location {
                file,
                symbol: crate::nav::index::Symbol {
                    name: member.to_string(),
                    kind,
                    line,
                    end_line: line,
                    start_byte: 0,
                    end_byte: 0,
                },
            }
        };
        let mut out: Vec<crate::nav::index::Location> = Vec::new();
        // Fields: the struct's `field_declaration` lines, all files (the
        // same file orders first below).
        for (file, line) in index.field_locations(type_name, member) {
            out.push(mk(
                file,
                crate::syntax::queries::SymbolKind::Constant,
                line,
            ));
        }
        // Methods: the same file's impl tables only (lexical — an impl's
        // methods all live in its own file).
        if let Some(tables) = index.tables(rel)
            && let Some(methods) = tables.impls.get(type_name)
        {
            for m in methods.iter().filter(|m| m.method == member) {
                out.push(mk(
                    rel.to_string(),
                    crate::syntax::queries::SymbolKind::Function,
                    m.line,
                ));
            }
        }
        if out.is_empty() {
            return out;
        }
        out.sort_by(|a, b| {
            (a.file != rel).cmp(&(b.file != rel)).then_with(|| {
                (a.file.as_str(), a.symbol.line, &a.symbol.name).cmp(&(b.file.as_str(), b.symbol.line, &b.symbol.name))
            })
        });
        out.dedup_by(|a, b| a.file == b.file && a.symbol.line == b.symbol.line && a.symbol.name == b.symbol.name);
        out
    }

    /// (010-03, plan 010 Shape A rung 3) The bare receiver identifier of
    /// a Rust `x.<member>` access when the point sits on (or
    /// immediately after) the MEMBER identifier run, plus the member
    /// run's char column (for the byte translation). The run derivation
    /// mirrors `symbol_at_point`'s (the char at the point when it is an
    /// identifier char, else the run ending immediately before it — the
    /// usual call-site spot). `None` for: the `self.` receiver (the
    /// 010-01 pre-step owns it — `Self` too, a type position), a
    /// receiver that isn't a bare identifier (`(expr).m`, `a[0].m`,
    /// `call().m`), a `::`-path receiver (`a::b.m`), a dot-chained
    /// receiver (`a.b.m` — the middle segment `b` is a field access,
    /// never a local binding), and a point not on a member run (the
    /// dot, whitespace).
    /// The extraction's path token stays BARE for these accesses — this
    /// scan is the pre-step's own, so a miss is byte-for-byte today's
    /// behavior (the caller gates on the language).
    pub(super) fn rust_dotted_receiver(line_text: &str, col: usize) -> Option<(String, usize)> {
        let chars: Vec<char> = line_text.chars().collect();
        if col > chars.len() {
            return None;
        }
        let is_ident = |c: char| c.is_alphanumeric() || c == '_';
        let start = if col < chars.len() && is_ident(chars[col]) {
            let mut i = col;
            while i > 0 && is_ident(chars[i - 1]) {
                i -= 1;
            }
            i
        } else if col > 0 && is_ident(chars[col - 1]) {
            let mut i = col - 1;
            while i > 0 && is_ident(chars[i - 1]) {
                i -= 1;
            }
            i
        } else {
            return None;
        };
        // The member run must be preceded by the `.`.
        if start < 2 || chars[start - 1] != '.' {
            return None;
        }
        // The receiver run ends at the char BEFORE the dot.
        let mut i = start - 2;
        if !is_ident(chars[i]) {
            return None; // `(expr).m`, `a[0].m`, `call().m` …
        }
        while i > 0 && is_ident(chars[i - 1]) {
            i -= 1;
        }
        // A bare identifier: nothing glued on the left (`myself.` is
        // fine — the run is the whole word — but `a::b.m`'s `b` has a
        // `:` before it: a path receiver; and `a.b.m`'s `b` has a `.`
        // before it: the middle segment of a dot chain — neither is a
        // local binding).
        if i > 0 && (is_ident(chars[i - 1]) || chars[i - 1] == ':' || chars[i - 1] == '.') {
            return None;
        }
        let receiver: String = chars[i..start - 1].iter().collect();
        match receiver.as_str() {
            "self" | "Self" => None,
            _ => Some((receiver, start)),
        }
    }

    /// (M., selection rule 4) Start the tooling-resolver fall-through OFF the
    /// input path (plan 006 issue 02): `spawn_blocking` + `ResolveBus`,
    /// mirroring the symbol-indexer pattern (`start_indexing`). `cargo
    /// metadata` / `cargo fetch` shell out (network, seconds) and must never
    /// block a keypress. A new request supersedes any in-flight one (the
    /// generation bump makes the stale event discard itself in
    /// `apply_resolve_event`). No-op-ish without a tokio runtime (plain unit
    /// tests): the indicator is cleared and a miss message reported, so the
    /// status line can never hang.
    ///
    /// 007-03: the `SymbolContext.scope` hint is populated from the current
    /// buffer's tree-sitter layer (`resolver_scope`), so a BARE symbol
    /// imported via `use` resolves instead of hitting the providers'
    /// "needs scope info" bail; without a hint the context stays empty and
    /// the providers behave exactly as before (byte-for-byte).
    pub fn start_symbol_resolution(&mut self, symbol: &str, from_file: &str) {
        let Some(project) = self.project.as_ref() else {
            self.minibuffer_message("no project");
            return;
        };
        let root = project.root.clone();
        let symbol_owned = symbol.to_string();
        self.resolve_generation += 1;
        let generation = self.resolve_generation;
        self.resolving = Some((format!("resolving `{symbol}`…"), generation));
        if tokio::runtime::Handle::try_current().is_err() {
            self.resolving = None;
            self.minibuffer_message(&format!("no provider resolution for `{symbol}` (no background runtime)"));
            return;
        }
        let bus = self.resolve_bus.clone();
        let from = std::path::PathBuf::from(from_file);
        // 007-03: the scope hint (use-declaration path for a bare symbol,
        // the enclosing item chain for a path-shaped one, empty otherwise).
        let scope = self.resolver_scope(symbol);
        // 011-01: the buffer's language (the dispatch key — only providers
        // whose `languages()` contain it are attempted; `None` for an
        // unknown extension keeps the pre-dispatch in-order walk).
        let language = self.resolution_language(from_file);
        tokio::task::spawn_blocking(move || {
            // The provider chain. 011-01 registers the non-Rust providers
            // and dispatches on the context language: a Python buffer can
            // never reach the cargo provider (and vice versa), so the
            // registration order among languages is only a tie-breaker.
            // `None` language (unknown extension) still walks the whole
            // chain, in this order.
            let mut chain = Resolver::new();
            chain.add(CargoProvider::new());
            chain.add(JsProvider::new());
            chain.add(PythonProvider::new());
            chain.add(GoProvider::new());
            let ctx = SymbolContext {
                workspace_root: root,
                symbol: symbol_owned.clone(),
                from_file: from,
                scope,
                language,
            };
            let (source, error) = match chain.resolve_traced(&ctx) {
                Ok(outcome) => (Some(outcome.source), None),
                Err(e) => (None, Some(e.to_string())),
            };
            bus.send(ResolveEvent { generation, symbol: symbol_owned, source, error });
        });
    }

    /// Install a resolve event into the store (called by the UI's ResolveBus
    /// drain in `Root`). Discards events from a stale generation (a
    /// superseded M-. request or a previous project — mirroring
    /// `apply_index_event`); otherwise clears the status activity and lands
    /// the result (jump) or reports the miss.
    pub fn apply_resolve_event(&mut self, event: &ResolveEvent) {
        if event.generation != self.resolve_generation {
            // A stale event (a superseded request or a previous project):
            // its result is discarded. The drain is latest-wins — this
            // stale send may have OVERWRITTEN the current generation's
            // event in the watch channel (006-02b item 3); if so the
            // current job's event never reaches us and its `resolving`
            // indicator would stick until the next action. Clearing it here
            // is always safe: a still-in-flight current-generation event
            // lands its jump when it arrives (its generation still matches)
            // — at worst the indicator hides a few moments early.
            self.resolving = None;
            return;
        }
        self.resolving = None;
        match (&event.source, &event.error) {
            (Some(source), _) => self.open_resolved_source(source, &event.symbol),
            (None, Some(e)) => {
                self.minibuffer_message(&format!(
                    "no provider resolution for `{}`: {}",
                    event.symbol, e
                ));
            }
            _ => {}
        }
    }

    /// The resolving indicator for the activity display (empty when idle;
    /// hidden automatically when the generation no longer matches — a
    /// superseded request or project switch).
    pub fn resolving_display(&self) -> String {
        match &self.resolving {
            Some((text, g)) if *g == self.resolve_generation => text.clone(),
            _ => String::new(),
        }
    }

    /// (011-01) The `SymbolContext.language` dispatch key for `from_file`:
    /// the grammar registry's lowercase language name ("rust", "python",
    /// "javascript", "go", … — matching the providers' `languages()`
    /// strings). `None` when the extension is unknown (`Plain`): the chain
    /// then keeps the pre-dispatch behavior and walks every provider in
    /// order (byte-for-byte today's behavior for unknown files).
    pub(super) fn resolution_language(&self, from_file: &str) -> Option<String> {
        let lang = self.grammar_registry.language_for(from_file);
        (lang != crate::syntax::registry::LanguageId::Plain).then(|| lang.name().to_string())
    }

    /// (007-03 / 011-02) The `SymbolContext.scope` hint for `symbol`, from
    /// the CURRENT buffer's tree-sitter layer (007-01's `scope_path_at` +
    /// the per-language import walks):
    /// - a BARE symbol with an import declaration that brings the name
    ///   into scope → the import's FULL original path, item included
    ///   (Rust `use a::B as C` → `["a","B"]` for bare `C`; JS/TS
    ///   `import { B as C } from "a"` → `["a","B"]`; Python
    ///   `from a import B as C` → `["a","B"]`; Go dot-import
    ///   `import . "a/b"` → `["b","<bare item>"]`);
    /// - a BARE symbol with no such import → EMPTY (the providers keep
    ///   their exact no-hint behavior — std/prelude names are never
    ///   guessed, byte-for-byte degradation);
    /// - a path-shaped symbol → the enclosing item chain (Rust, 007-01)
    ///   or a JS/TS namespace-aliased member rewrite (`ns.member` →
    ///   `["pkg","member"]` when `ns` comes from `import * as ns from
    ///   "pkg"`); Python/Go dotted symbols carry their own module /
    ///   package path and get no hint.
    ///
    /// Empty for a missing buffer/path, a failed parse, or an unimplemented
    /// language (including `Plain`).
    pub(super) fn resolver_scope(&self, symbol: &str) -> Vec<String> {
        let Some(key) = self.buffers.current().map(String::from) else {
            return Vec::new();
        };
        let Some(buf) = self.buffers.get(&key) else {
            return Vec::new();
        };
        let Some(path) = buf.path.as_ref() else {
            return Vec::new();
        };
        let p = self.file_point();
        Self::resolver_scope_for(path, &buf.rope, p.line, p.col, symbol)
    }

    /// The scope hint for the buffer at `(line, col)` — the testable seam
    /// behind [`resolver_scope`](Self::resolver_scope).
    fn resolver_scope_for(
        path: &Path,
        rope: &Rope,
        line: usize,
        col: usize,
        symbol: &str,
    ) -> Vec<String> {
        let lang = crate::syntax::registry::resolve_language(&path.display().to_string());
        let source = rope.to_string();
        let Some(byte) = point_byte_offset(rope, line, col) else {
            return Vec::new();
        };
        match lang {
            // 007-03 (Rust): bare → the `use` declaration's path; path-
            // shaped → the enclosing item chain (carried, not consumed).
            crate::syntax::registry::LanguageId::Rust => {
                if !symbol.contains("::") {
                    return Self::use_path_for_symbol(&source, byte, symbol).unwrap_or_default();
                }
                crate::syntax::node::scope_path_at(lang, &source, byte)
            }
            // 011-02: the per-language import walks (bare symbols), plus
            // the JS/TS namespace-member rewrite for path-shaped symbols.
            crate::syntax::registry::LanguageId::JavaScript
            | crate::syntax::registry::LanguageId::TypeScript
            | crate::syntax::registry::LanguageId::Tsx => {
                Self::js_ts_scope_for(lang, &source, byte, symbol)
            }
            crate::syntax::registry::LanguageId::Python => {
                Self::python_scope_for(&source, byte, symbol)
            }
            crate::syntax::registry::LanguageId::Go => Self::go_scope_for(&source, byte, symbol),
            // Every other language (and Plain): no hint — the providers
            // keep their exact no-hint behavior.
            _ => Vec::new(),
        }
    }

    /// (007-03) The FULL original path of the `use` declaration that brings
    /// `symbol` into scope at `byte` (e.g. `use serde::Deserialize;` →
    /// `["serde", "Deserialize"]`); an aliased import
    /// (`use a::B as C`) yields the ORIGINAL path for the alias `C`.
    ///
    /// Bounded by design: Rust imports are module-scoped, so only the
    /// `use_declaration` items of the source root (top level — the Rust
    /// grammar's root node is `source_file`) or of `byte`'s
    /// ANCESTOR `mod_item` chain are considered — a sibling or nested
    /// module's imports never name `byte`'s scope. Innermost module first
    /// (an inner import shadows an outer one); within a module, the LAST
    /// matching declaration wins. Globs (`use a::*`), single-segment
    /// imports (`use foo;` — same-crate modules), and `self`/`super`/
    /// `crate`-prefixed paths never name an external item → `None`
    /// (never guess).
    fn use_path_for_symbol(source: &str, byte: usize, symbol: &str) -> Option<Vec<String>> {
        let language = crate::syntax::queries::language_for(
            crate::syntax::registry::LanguageId::Rust,
        )?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).ok()?;
        let tree = parser.parse(source.as_bytes(), None)?;
        let root = tree.root_node();
        if !(root.start_byte() <= byte && byte < root.end_byte()) {
            return None;
        }
        // Innermost node containing `byte` (the same containment rule as
        // 007-01's `innermost_at`).
        let mut leaf = root;
        loop {
            let mut child = None;
            for i in 0..leaf.child_count() {
                if let Some(c) = leaf.child(i)
                    && c.start_byte() <= byte
                    && byte < c.end_byte()
                {
                    child = Some(c);
                    break;
                }
            }
            match child {
                Some(c) => leaf = c,
                None => break,
            }
        }
        // Candidate modules: the source root (top level) + the enclosing
        // `mod_item` ancestors, innermost first (nearest scope shadows).
        let mut modules = Vec::new();
        let mut anc = leaf.parent();
        while let Some(a) = anc {
            if a.kind() == "mod_item" || a.kind() == "source_file" {
                modules.push(a);
            }
            anc = a.parent();
        }
        // The ancestor walk already yields innermost-first order.
        for module in &modules {
            // `mod_item` items live in its `body` block; the source root's
            // are direct children.
            let items = if module.kind() == "mod_item" {
                module.child_by_field_name("body")?
            } else {
                *module
            };
            let mut hit: Option<Vec<String>> = None;
            for i in 0..items.child_count() {
                let child = items.child(i)?;
                if child.kind() != "use_declaration" {
                    continue;
                }
                if let Ok(text) = child.utf8_text(source.as_bytes())
                    && let Some(path) = Self::use_decl_path(text, symbol)
                {
                    hit = Some(path); // last matching declaration wins
                }
            }
            if hit.is_some() {
                return hit;
            }
        }
        None
    }

    /// The original import path (segments, item included) of a
    /// `use_declaration` TEXT that brings `symbol` into scope — or `None`
    /// when no entry of the declaration names `symbol` (globs, module-only
    /// imports, `self`/`super`/`crate` prefixes are never guessed).
    fn use_decl_path(text: &str, symbol: &str) -> Option<Vec<String>> {
        // `pub (vis) use <spec>;` — find the `use` KEYWORD (token-wise; a
        // `pub(crate)` prefix never contains the token `use`).
        let body = text.split(';').next()?.trim();
        let mut pos = 0usize;
        let mut found = false;
        for tok in body.split(char::is_whitespace) {
            if tok == "use" {
                pos += 3;
                found = true;
                break;
            }
            pos += tok.len() + 1;
        }
        if !found || pos > body.len() || !body.is_char_boundary(pos) {
            return None;
        }
        let rest = body[pos..].trim();
        if rest.is_empty() {
            return None;
        }
        // `prefix::{ ... }` / `prefix::Name [as Alias]`
        match rest.find('{') {
            Some(open) => {
                let close = rest.rfind('}')?;
                if close < open {
                    return None;
                }
                let prefix_raw = rest[..open].trim();
                let prefix = prefix_raw.strip_suffix("::").unwrap_or(prefix_raw).to_string();
                Self::use_group_entries(&rest[open + 1..close], &prefix, symbol)
            }
            None => {
                let (path_part, alias) = match rest.split_once(" as ") {
                    Some((p, a)) => (p, Some(a.trim())),
                    None => (rest, None),
                };
                let segments = Self::import_segments(path_part)?;
                // Single-segment imports are same-crate modules (no
                // external crate is named) — never guessed.
                if segments.len() < 2 {
                    return None;
                }
                let local = alias.unwrap_or(segments.last().unwrap());
                (local == symbol).then_some(segments)
            }
        }
    }

    /// The import entries of a `use` group body (comma-separated, nested
    /// `sub::{…}` groups recurse), matched against `symbol`.
    fn use_group_entries(group: &str, prefix: &str, symbol: &str) -> Option<Vec<String>> {
        let mut depth = 0i32;
        let mut start = 0usize;
        let mut entries: Vec<&str> = Vec::new();
        for (i, c) in group.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                ',' if depth == 0 => {
                    entries.push(&group[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        entries.push(&group[start..]);
        for entry in entries
            .into_iter()
            .map(|e| e.trim())
            .filter(|e| !e.is_empty())
        {
            if let Some(nested) = entry.find('{') {
                // `sub::{…}` — the group's own prefix joins in.
                let sub = entry[..nested].trim();
                let full = if prefix.is_empty() {
                    sub.to_string()
                } else {
                    format!("{prefix}::{sub}")
                };
                if let Some(close) = entry.rfind('}')
                    && let Some(p) =
                        Self::use_group_entries(&entry[nested + 1..close], &full, symbol)
                {
                    return Some(p);
                }
                continue;
            }
            let (name_part, alias) = match entry.split_once(" as ") {
                Some((p, a)) => (p.trim(), Some(a.trim())),
                None => (entry, None),
            };
            // `*` (glob) cannot name a specific symbol.
            if name_part == "*" {
                continue;
            }
            // A non-resolvable entry (`self`/`super`/`crate`-prefixed, or a
            // single segment) must SKIP, not abort the whole group: in
            // `use a::b::{self, c};` a bare `c` still has a valid hint.
            // `?` here would discard the remaining entries (007-03 review P2).
            let Some(segs) = Self::import_segments(name_part) else {
                continue;
            };
            let full = if prefix.is_empty() {
                segs
            } else {
                let Some(mut v) = Self::import_segments(prefix) else {
                    continue;
                };
                v.extend_from_slice(&segs);
                v
            };
            // Same rule as the plain form: without an external prefix a
            // single segment is a same-crate item (never guessed).
            if full.len() < 2 {
                continue;
            }
            let local = alias.unwrap_or(full.last().unwrap());
            if local == symbol {
                return Some(full);
            }
        }
        None
    }

    /// Split a `::`-path on `::` into clean identifier segments; `None`
    /// when a segment is empty or non-identifier (never guess).
    fn import_segments(s: &str) -> Option<Vec<String>> {
        let segs: Vec<&str> = s.split("::").collect();
        if segs.is_empty() || segs.iter().any(|g| g.is_empty()) {
            return None;
        }
        let mut out = Vec::with_capacity(segs.len());
        for g in segs {
            if !g.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return None;
            }
            out.push(g.to_string());
        }
        // `self`/`super`/`crate` prefixes never name an external crate.
        if matches!(out.first().map(String::as_str), Some("self" | "super" | "crate")) {
            return None;
        }
        Some(out)
    }

    /// The text of a syntax node (`None` on non-UTF8).
    fn node_text(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
        node.utf8_text(source).ok().map(String::from)
    }

    /// A non-empty ASCII identifier (`a0_Z`) — the segment shape an import
    /// path may carry (mirrors the Rust `import_segments` rule; anything
    /// else is an unsupported shape → no hint, never a guess).
    fn is_ascii_identifier(s: &str) -> bool {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    /// (011-02, relative extension in the 011-08 fix-jsrel P2-7
    /// follow-up) The JS/TS scope hint: a bare symbol → the package path
    /// (or the relative specifier) its import binds it to; a path-shaped
    /// `ns.member` → the namespace rewrite; a plain dotted path
    /// (`lodash.map`) → EMPTY (it carries its own package — the provider's
    /// path wins, never treated as bare).
    fn js_ts_scope_for(
        lang: crate::syntax::registry::LanguageId,
        source: &str,
        byte: usize,
        symbol: &str,
    ) -> Vec<String> {
        // One parse per miss (the 007-03 discipline): a parse failure or an
        // out-of-range offset degrades to the empty hint.
        let Some(language) = crate::syntax::queries::language_for(lang) else {
            return Vec::new();
        };
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&language).is_err() {
            return Vec::new();
        }
        let Some(tree) = parser.parse(source.as_bytes(), None) else {
            return Vec::new();
        };
        let root = tree.root_node();
        if !(root.start_byte() <= byte && byte < root.end_byte()) {
            return Vec::new();
        }
        let bytes = source.as_bytes();
        if !symbol.contains('.') {
            return Self::js_ts_bare_import_path(root, bytes, symbol)
                .unwrap_or_default();
        }
        // Path-shaped: exactly two dot segments (`ns.member`); deeper
        // chains are an unsupported shape (no hint).
        let Some((ns, member)) = symbol.split_once('.') else {
            return Vec::new();
        };
        if member.contains('.') {
            return Vec::new();
        }
        Self::js_ts_namespace_member_path(root, bytes, ns, member)
            .unwrap_or_default()
    }

    /// The import path for a BARE JS/TS symbol, from the module's import
    /// declarations. Bounded by design: only TOP-LEVEL declarations are
    /// considered (ESM imports are module-scoped; CJS `require` bindings
    /// are tracked at the top level only). First hit wins — a duplicate
    /// binding of one name is a syntax error, so at most one declaration
    /// can bind `symbol`:
    /// - `import { X } from "pkg"` / `import type { X }` → `["pkg", "X"]`;
    /// - `import { X as Y }` → `["pkg", "X"]` for bare `Y` (alias →
    ///   original);
    /// - `import X from "pkg"` → `["pkg", "X"]` (the default export);
    /// - `import * as ns from "pkg"` → `["pkg"]` for bare `ns` (the
    ///   package entry itself);
    /// - `const { X } = require("pkg")` / `const { X as Y } = require` →
    ///   the same rule (CJS destructuring);
    /// - `const m = require("pkg")` → `["pkg"]` for bare `m` (the module
    ///   object names the entry).
    ///
    /// 011-08 follow-up (fix-jsrel P2-7): a relative specifier (`./…`,
    /// `../…`) now CARRIES its hint (the JS provider resolves it against
    /// the importing buffer's directory and lands in the sibling file,
    /// workspace-local / `external = false`). Absolute paths and bare
    /// side-effect imports (`import "pkg"`) still bind nothing the
    /// provider can resolve → `None` (never guessed).
    fn js_ts_bare_import_path(root: tree_sitter::Node, source: &[u8], symbol: &str) -> Option<Vec<String>> {
        for i in 0..root.child_count() {
            let child = root.child(i)?;
            let hit = match child.kind() {
                "import_statement" => {
                    Self::js_ts_import_stmt_path(child, source, symbol)
                }
                "lexical_declaration" | "variable_declaration" => {
                    Self::js_ts_require_path(child, source, symbol)
                }
                _ => None,
            };
            if hit.is_some() {
                return hit;
            }
        }
        None
    }

    /// One `import_statement`: the local binding's original path
    /// (`None` when no binding names `symbol`).
    fn js_ts_import_stmt_path(stmt: tree_sitter::Node, source: &[u8], symbol: &str) -> Option<Vec<String>> {
        let source_node = stmt.child_by_field_name("source")?;
        let spec = Self::js_ts_specifier(source_node, source)?;
        // `import_clause` is NOT a grammar field (verified against the
        // pinned tree-sitter-javascript 0.25.0 sexp, re-pinned by the
        // grammar-bumps suite from the probed 0.23.1) — find it by kind.
        // Its absence is the side-effect form (`import "pkg"`) — binds
        // nothing, never a hint.
        let clause = (0..stmt.child_count())
            .filter_map(|k| stmt.child(k))
            .find(|n| n.kind() == "import_clause")?;
        for i in 0..clause.child_count() {
            let c = clause.child(i)?;
            match c.kind() {
                // `import X from "pkg"` — the local binding for the
                // package's default export.
                "identifier" => {
                    if let Some(name) = Self::node_text(c, source)
                        && name == symbol
                    {
                        return Some(Self::js_ts_import_hint(&spec, &name));
                    }
                }
                "named_imports" => {
                    for j in 0..c.child_count() {
                        let entry = c.child(j)?;
                        if entry.kind() != "import_specifier" {
                            continue;
                        }
                        let Some(name_node) = entry.child_by_field_name("name") else {
                            continue;
                        };
                        let name = Self::node_text(name_node, source)?;
                        let alias = entry
                            .child_by_field_name("alias")
                            .and_then(|a| Self::node_text(a, source));
                        if alias.as_deref().unwrap_or(&name) == symbol {
                            return Some(Self::js_ts_import_hint(&spec, &name));
                        }
                    }
                }
                // `import * as ns from "pkg"` — the bare namespace names
                // the package entry itself (its single named child is the
                // alias; `*`/`as` are anonymous tokens).
                "namespace_import" => {
                    let alias = (0..c.child_count())
                        .filter_map(|k| c.child(k))
                        .find(|n| n.kind() == "identifier")
                        .and_then(|n| Self::node_text(n, source))?;
                    if alias == symbol {
                        return Some(vec![spec]);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// `const/let/var` declarations whose initializer is a plain
    /// `require("pkg")` call (the CJS import shape).
    fn js_ts_require_path(decl: tree_sitter::Node, source: &[u8], symbol: &str) -> Option<Vec<String>> {
        for i in 0..decl.child_count() {
            let d = decl.child(i)?;
            if d.kind() != "variable_declarator" {
                continue;
            }
            let Some(name) = d.child_by_field_name("name") else {
                continue;
            };
            let Some(value) = d.child_by_field_name("value") else {
                continue;
            };
            if value.kind() != "call_expression" {
                continue;
            }
            // Only the bare identifier `require` (not a member/alias call).
            let Some(callee) = value.child_by_field_name("function") else {
                continue;
            };
            if callee.kind() != "identifier"
                || Self::node_text(callee, source).as_deref() != Some("require")
            {
                continue;
            }
            let Some(args) = value.child_by_field_name("arguments") else {
                continue;
            };
            // The arguments node: `(` at index 0, first arg at 1.
            let Some(first) = args.child(1) else {
                continue;
            };
            if first.kind() != "string" {
                continue;
            }
            let Some(spec) = Self::js_ts_specifier(first, source) else {
                continue;
            };
            match name.kind() {
                // `const m = require("pkg")` — the whole module object:
                // the binding names the package entry itself.
                "identifier" => {
                    if let Some(n) = Self::node_text(name, source)
                        && n == symbol
                    {
                        return Some(vec![spec]);
                    }
                }
                // `const { x, y: z } = require("pkg")` — destructured
                // exports (shorthand and `original: local` pairs).
                "object_pattern" => {
                    for j in 0..name.child_count() {
                        let p = name.child(j)?;
                        let (local, original) = match p.kind() {
                            "shorthand_property_identifier_pattern" => {
                                let t = Self::node_text(p, source)?;
                                (t.clone(), t)
                            }
                            "pair_pattern" => {
                                let Some(key) = p.child_by_field_name("key") else {
                                    continue;
                                };
                                let Some(val) = p.child_by_field_name("value") else {
                                    continue;
                                };
                                if val.kind() != "identifier" {
                                    continue; // nested patterns: unsupported shape.
                                }
                                (
                                    Self::node_text(val, source)?,
                                    Self::node_text(key, source)?,
                                )
                            }
                            _ => continue,
                        };
                        if local == symbol {
                            return Some(Self::js_ts_import_hint(&spec, &original));
                        }
                    }
                }
                _ => {} // array/nested patterns: unsupported shape → no hint.
            }
        }
        None
    }

    /// The hint path for an imported item: `import { default as D }` /
    /// `const { default: D } = require` name the ENTRY itself (no item
    /// segment); any other original name carries it.
    fn js_ts_import_hint(spec: &str, original: &str) -> Vec<String> {
        if original == "default" {
            vec![spec.to_string()]
        } else {
            vec![spec.to_string(), original.to_string()]
        }
    }

    /// A JS/TS string-literal module specifier the hint may carry —
    /// quoted with `'`/`"` (a template literal or other shape is
    /// unsupported). Two shapes pass:
    /// - a package path — scoped (`@scope/name`) and subpath (`name/sub`)
    ///   specs keep their `/` (the 011-02 external-package hint, which
    ///   resolves only through node_modules);
    /// - a relative specifier (`./…` / `../…`) — the 011-08 fix-jsrel
    ///   provider semantics: it resolves against the importing buffer's
    ///   directory (exact file → JS-extension walk → directory entry)
    ///   and lands workspace-locally (`external = false`), so the hint
    ///   carries it, item included.
    ///
    /// Absolute paths and any other `.`-leading shape (bare `.`/`..`) stay
    /// out: the provider bails dedicated on absolute, and a bare `.`/`..`
    /// never reaches its relative branch (it is not a `./`-prefixed spec)
    /// → never a hint.
    fn js_ts_specifier(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
        let text = Self::node_text(node, source)?;
        let bytes = text.as_bytes();
        if bytes.len() < 2 {
            return None;
        }
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if !(first == b'\'' && last == b'\'' || first == b'"' && last == b'"') {
            return None;
        }
        let spec = std::str::from_utf8(&bytes[1..bytes.len() - 1]).ok()?;
        let is_relative = spec.starts_with("./") || spec.starts_with("../");
        if spec.is_empty()
            || spec.starts_with('/')
            || (spec.starts_with('.') && !is_relative)
        {
            return None;
        }
        Some(spec.to_string())
    }

    /// The namespace-member rewrite: `ns.member` where `ns` comes from
    /// `import * as ns from "pkg"` → `["pkg", "member"]` (the provider's
    /// alias-rewrite rule turns it back into the package's real path;
    /// identity cases — `import * as pkg from "pkg"` — are a no-op there
    /// because the joined hint equals the symbol's own path).
    fn js_ts_namespace_member_path(
        root: tree_sitter::Node,
        source: &[u8],
        ns: &str,
        member: &str,
    ) -> Option<Vec<String>> {
        for i in 0..root.child_count() {
            let child = root.child(i)?;
            if child.kind() != "import_statement" {
                continue;
            }
            let Some(source_node) = child.child_by_field_name("source") else {
                continue;
            };
            let Some(spec) = Self::js_ts_specifier(source_node, source) else {
                continue;
            };
            let Some(clause) = (0..child.child_count())
                .filter_map(|k| child.child(k))
                .find(|n| n.kind() == "import_clause")
            else {
                // `import "pkg"` — side-effect only, binds nothing.
                continue;
            };
            for j in 0..clause.child_count() {
                let c = clause.child(j)?;
                if c.kind() != "namespace_import" {
                    continue;
                }
                let alias = (0..c.child_count())
                    .filter_map(|k| c.child(k))
                    .find(|n| n.kind() == "identifier")
                    .and_then(|n| Self::node_text(n, source))?;
                if alias == ns {
                    return Some(vec![spec, member.to_string()]);
                }
            }
        }
        None
    }

    /// (011-02) The Python scope hint: a BARE symbol → the module path +
    /// item its import binds it to. A dotted symbol (`os.path.join`) carries
    /// its own module path → EMPTY (never treated as bare).
    pub(super) fn python_scope_for(source: &str, byte: usize, symbol: &str) -> Vec<String> {
        if symbol.contains('.') {
            return Vec::new();
        }
        Self::python_import_path_for_symbol(source, byte, symbol).unwrap_or_default()
    }

    /// The module path (segments, item included) of the import that binds a
    /// BARE Python `symbol` at `byte`:
    /// - `from a import X` → `["a", "X"]`; `from a.b import X` →
    ///   `["a", "b", "X"]`; `from a import X as Y` → the original `X` for
    ///   bare `Y`;
    /// - `import a.b as c` → `["a", "b"]` for bare `c` (the module alias);
    /// - a plain `import a.b` binds ONLY the top-level `a` → never a hint
    ///   for bare `b` (not guessed);
    /// - relative imports (`from . import X`), wildcards (`import *`), and
    ///   `from a import b.c` (binds `b`, an attribute walk) → `None`
    ///   (the sys.path root is unknown from the buffer path alone — never
    ///   guessed).
    ///
    /// Bounded: the module level + the enclosing `function`/`class` blocks
    /// only (innermost first — a local import shadows the module-level
    /// one); imports nested deeper (under an `if`, etc.) are not counted.
    /// Within a block the LAST matching statement at/before `byte` wins
    /// (a re-import shadows the earlier one).
    fn python_import_path_for_symbol(source: &str, byte: usize, symbol: &str) -> Option<Vec<String>> {
        let language = crate::syntax::queries::language_for(
            crate::syntax::registry::LanguageId::Python,
        )?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).ok()?;
        let tree = parser.parse(source.as_bytes(), None)?;
        let root = tree.root_node();
        if !(root.start_byte() <= byte && byte < root.end_byte()) {
            return None;
        }
        // The innermost node containing `byte` (the same containment rule
        // as the Rust walk).
        let mut leaf = root;
        loop {
            let mut child = None;
            for i in 0..leaf.child_count() {
                if let Some(c) = leaf.child(i)
                    && c.start_byte() <= byte
                    && byte < c.end_byte()
                {
                    child = Some(c);
                    break;
                }
            }
            match child {
                Some(c) => leaf = c,
                None => break,
            }
        }
        // Candidate blocks, innermost first (nearest scope shadows), the
        // module level last.
        let mut scopes: Vec<tree_sitter::Node> = Vec::new();
        let mut anc = leaf.parent();
        while let Some(a) = anc {
            if a.kind() == "function_definition" || a.kind() == "class_definition" {
                scopes.push(a);
            }
            anc = a.parent();
        }
        scopes.push(root);
        let bytes = source.as_bytes();
        for scope in scopes {
            let block = if scope.kind() == "module" {
                scope
            } else {
                scope.child_by_field_name("body")?
            };
            let mut hit: Option<Vec<String>> = None;
            for i in 0..block.child_count() {
                let child = block.child(i)?;
                match child.kind() {
                    "import_statement" | "import_from_statement" => {
                        if !(child.start_byte() <= byte) {
                            continue;
                        }
                        if let Some(path) =
                            Self::python_import_stmt_path(child, bytes, symbol)
                        {
                            hit = Some(path); // last matching statement wins
                        }
                    }
                    _ => {}
                }
            }
            if hit.is_some() {
                return hit;
            }
        }
        None
    }

    /// One Python import statement: the original path the entry binds to
    /// `symbol` (`None` when no entry names it — plain `import a.b`,
    /// relative modules, wildcards, and attribute-walk entries are never
    /// guessed).
    fn python_import_stmt_path(
        stmt: tree_sitter::Node,
        source: &[u8],
        symbol: &str,
    ) -> Option<Vec<String>> {
        match stmt.kind() {
            "import_statement" => {
                // Each entry is field `name`: a `dotted_name` (binds only
                // its TOP-LEVEL segment — never a hint) or an
                // `aliased_import` (binds the alias to the full module).
                let mut hit: Option<Vec<String>> = None;
                for i in 0..stmt.child_count() {
                    let child = stmt.child(i)?;
                    if child.kind() != "aliased_import" {
                        continue;
                    }
                    let Some(alias_node) = child.child_by_field_name("alias") else {
                        continue;
                    };
                    let alias = Self::node_text(alias_node, source)?;
                    if alias != symbol {
                        continue;
                    }
                    let Some(name) = child.child_by_field_name("name") else {
                        continue;
                    };
                    hit = Some(Self::python_dotted_segments(name, source)?);
                }
                hit
            }
            "import_from_statement" => {
                let module = stmt.child_by_field_name("module_name")?;
                let base = if module.kind() == "relative_import" {
                    return None; // `from . import X`: the enclosing package is
                    // ambiguous without the sys.path root — not guessed.
                } else {
                    Self::python_dotted_segments(module, source)?
                };
                let mut hit: Option<Vec<String>> = None;
                for i in 0..stmt.child_count() {
                    let child = stmt.child(i)?;
                    match child.kind() {
                        "aliased_import" => {
                            let Some(alias_node) = child.child_by_field_name("alias") else {
                                continue;
                            };
                            let alias = Self::node_text(alias_node, source)?;
                            if alias != symbol {
                                continue;
                            }
                            let Some(name) = child.child_by_field_name("name") else {
                                continue;
                            };
                            let item = Self::python_dotted_segments(name, source)?;
                            if item.len() != 1 {
                                continue; // `from a import b.c` binds `b`, not `b.c`.
                            }
                            let mut full = base.clone();
                            full.extend(item);
                            hit = Some(full);
                        }
                        "dotted_name" => {
                            // `from a import b` — binds the (single) name.
                            let item = Self::python_dotted_segments(child, source)?;
                            if item.len() == 1 && item[0] == symbol {
                                let mut full = base.clone();
                                full.push(item[0].clone());
                                hit = Some(full);
                            }
                        }
                        _ => {} // `wildcard_import`: names no specific symbol.
                    }
                }
                hit
            }
            _ => None,
        }
    }

    /// The identifier segments of a Python `dotted_name` node (its `.`
    /// tokens are anonymous — only `identifier` children count); `None`
    /// when a segment is not a plain ASCII identifier.
    fn python_dotted_segments(node: tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
        let mut out = Vec::new();
        for i in 0..node.child_count() {
            let c = node.child(i)?;
            if c.kind() != "identifier" {
                continue;
            }
            let t = Self::node_text(c, source)?;
            if !Self::is_ascii_identifier(&t) {
                return None;
            }
            out.push(t);
        }
        if out.is_empty() {
            return None;
        }
        Some(out)
    }

    /// (011-02) The Go scope hint: a BARE symbol → the dot-imported package
    /// name + the symbol (Go binds bare item names only through DOT
    /// imports — plain and aliased imports bind a package NAME, which is
    /// always used qualified and needs no hint). A dotted symbol
    /// (`y.Fn`) carries its own package name → EMPTY.
    pub(super) fn go_scope_for(source: &str, byte: usize, symbol: &str) -> Vec<String> {
        if symbol.contains('.') {
            return Vec::new();
        }
        Self::go_import_path_for_symbol(source, byte, symbol).unwrap_or_default()
    }

    /// The dot-import hint for a bare Go `symbol`: exactly ONE dot-imported
    /// package in the file (file-scoped) → `["<local pkg name>",
    /// "<symbol>"]` (the local name is the import path's last segment);
    /// zero dot imports → `None`; SEVERAL → `None` (the origin of a bare
    /// item is ambiguous — never guessed).
    fn go_import_path_for_symbol(source: &str, byte: usize, symbol: &str) -> Option<Vec<String>> {
        let language = crate::syntax::queries::language_for(
            crate::syntax::registry::LanguageId::Go,
        )?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).ok()?;
        let tree = parser.parse(source.as_bytes(), None)?;
        let root = tree.root_node();
        if !(root.start_byte() <= byte && byte < root.end_byte()) {
            return None;
        }
        let bytes = source.as_bytes();
        let mut dot_pkgs: Vec<String> = Vec::new();
        for i in 0..root.child_count() {
            let child = root.child(i)?;
            if child.kind() != "import_declaration" {
                continue;
            }
            // Specs are direct children (single import) or children of the
            // `import_spec_list` (grouped `import ( … )`).
            let mut stack = vec![child];
            while let Some(node) = stack.pop() {
                for j in 0..node.child_count() {
                    let c = node.child(j)?;
                    match c.kind() {
                        "import_spec" => {
                            // Only `import . "pkg"` (the named `dot` node)
                            // binds bare item names.
                            let Some(name) = c.child_by_field_name("name") else {
                                continue;
                            };
                            if name.kind() != "dot" {
                                continue;
                            }
                            let Some(path_node) = c.child_by_field_name("path") else {
                                continue;
                            };
                            let path = Self::go_string_content(path_node, bytes)?;
                            let pkg = path.rsplit('/').next()?;
                            if !Self::is_ascii_identifier(pkg) {
                                continue;
                            }
                            dot_pkgs.push(pkg.to_string());
                        }
                        "import_spec_list" => stack.push(c),
                        _ => {}
                    }
                }
            }
        }
        let [pkg] = dot_pkgs.as_slice() else {
            return None;
        };
        Some(vec![pkg.to_string(), symbol.to_string()])
    }

    /// The content of a Go `interpreted_string_literal` (import paths): the
    /// quotes stripped, for `"…"` and backtick-quoted strings.
    fn go_string_content(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
        let text = Self::node_text(node, source)?;
        let bytes = text.as_bytes();
        if bytes.len() < 2 {
            return None;
        }
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if !(first == b'"' && last == b'"' || first == b'`' && last == b'`') {
            return None;
        }
        std::str::from_utf8(&bytes[1..bytes.len() - 1]).ok().map(String::from)
    }

    /// 006-03b item 2: the resolver fall-through's `from_file` for an
    /// EXTERNAL buffer. `SymbolContext.from_file` is documented
    /// root-relative (the project path passes the project-relative
    /// `rel`), so the absolute path never goes: the crate-relative key
    /// shape (the index's own key) when the owning root is known —
    /// cached, or still in flight — else the bare file name (relative,
    /// never absolute; the root is genuinely unknown when a build was
    /// refused or no runtime is present).
    pub(super) fn resolver_from_file(&self, path: &Path) -> String {
        let root = self
            .external_indexes
            .iter()
            .find(|(r, _)| path.starts_with(r))
            .map(|(r, _)| r.clone())
            .or_else(|| {
                self.crate_indexing
                    .iter()
                    .find(|(r, _, _)| path.starts_with(r))
                    .map(|(r, _, _)| r.clone())
            });
        root.and_then(|root| Self::crate_rel(path, &root))
            .unwrap_or_else(|| {
                // Never absolute: `SymbolContext.from_file` is root-relative.
                // file_name() is Some for every real buffer path; the
                // empty-string fallback (rather than display()) keeps the
                // contract even for the pathological `/` or `..` shapes.
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
    }

    /// M-. inside an EXTERNAL (registry / tooling) buffer (plan 006
    /// issue 03): the SAME selection rule as the project path (symbol at
    /// point + `::`-path token, same-file-first candidate ordering,
    /// dedup, enclosing-symbol fallback) run against the OWNING crate's
    /// index (keyed against its source_root); a crate miss keeps the
    /// resolver fall-through (unchanged semantics — `SymbolContext` still
    /// carries the ORIGIN project's workspace_root, so following a type
    /// into ANOTHER dependency resolves through the origin project's
    /// metadata; the landing in that crate registers its index per
    /// `open_resolved_source`, and the LRU cap governs).
    fn xref_in_external_buffer(&mut self, key: &str, path: &Path) {
        // Supersede any in-flight tooling resolve (006-02b item 2: a hit
        // must not be clobbered by a stale event of a superseded
        // request); the fall-through path re-bumps in
        // `start_symbol_resolution`.
        self.resolve_generation += 1;
        let line = self.point_line();
        let line_text = self
            .buffers
            .get(key)
            .map(|b| b.line_text(line).unwrap_or_default())
            .unwrap_or_default();
        // 011-06: language-aware path token (the same seam as the project
        // path — external buffers are non-Rust in practice, but the
        // behavior is uniform by construction).
        let lang = self.grammar_registry.language_for(&path.to_string_lossy());
        let at = Self::symbol_at_point(lang, &line_text, self.point_col());
        let Some((root, outcome)) =
            self.crate_xref_outcome(key, path, line, lang, at.as_ref())
        else {
            // No crate index for this root yet (build in flight, refused,
            // or no runtime): the miss behaves as the project's — the
            // resolver fall-through (or "no symbol under point").
            match &at {
                // 006-03b item 2: a root-relative `from_file` (never the
                // absolute path).
                Some((_, path_token)) => {
                    self.start_symbol_resolution(path_token, &self.resolver_from_file(path))
                }
                None => self.minibuffer_message("no symbol under point"),
            }
            return;
        };
        match outcome {
            ExternalXrefOutcome::Jump { file, line } => {
                let origin = self.current_jump_entry();
                let abs = root.join(&file);
                if self.open_external_path(&abs).is_some() {
                    self.set_point_line(line);
                    self.recenter_landing();
                    self.ensure_highlight();
                    self.record_jump(origin, "M-.");
                    self.minibuffer_message(&format!(
                        "jumped to {file}:{}",
                        line + 1
                    ));
                } else {
                    self.minibuffer_message(&format!("cannot open {file}"));
                }
            }
            ExternalXrefOutcome::Picker { lookup, defs } => {
                self.xref_crate_root = Some(root);
                self.xref_lookup_name = lookup;
                let candidates: Vec<PickerCandidate> = defs
                    .iter()
                    .map(|d| PickerCandidate {
                        name: format!("{}:{}", d.file, d.symbol.line + 1),
                        display: format!(
                            "{}:{}  [{}] {}",
                            d.file,
                            d.symbol.line + 1,
                            d.symbol.kind.tag(),
                            d.symbol.name
                        ),
                        label: d.symbol.name.clone(),
                        detail: format!("[{}] {}:{}", d.symbol.kind.tag(), d.file, d.symbol.line + 1),
                        docs: String::new(),
                        category: "xref".to_string(),
                    })
                    .collect();
                self.open_picker(PickerKind::Xref, "Definition: ", candidates);
            }
            ExternalXrefOutcome::Resolver(token) => {
                // 006-03b item 2: a root-relative `from_file` (never the
                // absolute path).
                self.start_symbol_resolution(&token, &self.resolver_from_file(path))
            }
            ExternalXrefOutcome::NoDefinition(name) => {
                self.minibuffer_message(&format!("no definition for `{name}`"))
            }
            ExternalXrefOutcome::NoSymbol => {
                self.minibuffer_message("no symbol under point")
            }
        }
    }

    /// The M-. selection outcome for an EXTERNAL buffer, run against its
    /// owning crate's index (crate-relative paths, the project path's
    /// same-file-first ordering). `None` when the root has no cached
    /// index yet.
    fn crate_xref_outcome(
        &mut self,
        key: &str,
        path: &Path,
        line: usize,
        lang: LanguageId,
        at: Option<&(String, String)>,
    ) -> Option<(PathBuf, ExternalXrefOutcome)> {
        let (root, arc) = self.crate_index_arc_for_path(path)?;
        let idx = arc.lock().unwrap();
        // 006-03b item 3: the index key shape (forward-slash normalized).
        let rel = Self::crate_rel(path, &root)?;
        // (2) Symbol-at-point definitions (the same selection rule as the
        // project path).
        let defs = at
            .and_then(|(ident, token)| {
                // 010-01: the Rust self-receiver pre-step (the SAME seam as
                // the project path — external crates are Rust in practice,
                // and the behavior is uniform by construction).
                if lang == LanguageId::Rust
                    && let Some(member) = token.strip_prefix("self.")
                    && !member.is_empty()
                    && let Some(buf) = self.buffers.get(key)
                {
                    let source = buf.rope.to_string();
                    if let Some(byte) = point_byte_offset(&buf.rope, line, self.point_col()) {
                        let cands =
                            Self::self_receiver_candidates(&idx, &rel, &source, byte, member);
                        if !cands.is_empty() {
                            return Some(cands);
                        }
                    }
                }
                // 010-03: the local-binding pre-step (the SAME seam as the
                // project path — external crates are Rust in practice, and
                // the behavior is uniform by construction).
                if lang == LanguageId::Rust
                    && let Some(buf) = self.buffers.get(key)
                    && let Some((receiver, member_col)) = Self::rust_dotted_receiver(
                        &buf.line_text(line).unwrap_or_default(),
                        self.point_col(),
                    )
                {
                    let source = buf.rope.to_string();
                    if let Some(byte) = point_byte_offset(&buf.rope, line, member_col) {
                        let cands = Self::local_binding_candidates(
                            &idx,
                            &rel,
                            &source,
                            byte,
                            &receiver,
                            ident,
                        );
                        if !cands.is_empty() {
                            return Some(cands);
                        }
                    }
                }
                Self::xref_definition_candidates(&idx, ident, token, &rel)
            })
            .unwrap_or_default();
        let outcome = if !defs.is_empty() {
            let lookup = defs[0].symbol.name.clone();
            if defs.len() == 1 {
                ExternalXrefOutcome::Jump {
                    file: defs[0].file.clone(),
                    line: defs[0].symbol.line,
                }
            } else {
                ExternalXrefOutcome::Picker { lookup, defs }
            }
        } else {
            // (3) Enclosing-symbol fallback (unchanged: by line, not by the
            // point's column).
            let outline = idx.outline(&rel).to_vec();
            match crate::nav::index::enclosing_symbol(&outline, line) {
                Some(sym) => {
                    let lookup = sym.name.clone();
                    let defs = idx.definitions_of(&lookup);
                    if defs.is_empty() {
                        // (4) The point's own (path-shaped) token is what
                        // the resolver gets — not the enclosing name.
                        match at {
                            Some((_, token)) => {
                                ExternalXrefOutcome::Resolver(token.clone())
                            }
                            None => ExternalXrefOutcome::NoDefinition(lookup),
                        }
                    } else if defs.len() == 1 {
                        ExternalXrefOutcome::Jump {
                            file: defs[0].file.clone(),
                            line: defs[0].symbol.line,
                        }
                    } else {
                        ExternalXrefOutcome::Picker { lookup, defs }
                    }
                }
                None => match at {
                    Some((_, token)) => ExternalXrefOutcome::Resolver(token.clone()),
                    None => ExternalXrefOutcome::NoSymbol,
                },
            }
        };
        Some((root, outcome))
    }

    /// The identifier run at the point's column (char offset) on `text`, plus
    /// the path token it belongs to (plan 006 issue 02, selection rule 1).
    /// Returns `(identifier, path_token)` — e.g. the cursor inside
    /// `tokio::spawn` → `("spawn", "tokio::spawn")`; on a Rust `obj.name`
    /// → `("name", "name")`; on a Rust `self.name` →
    /// `("name", "self.name")` (010-01 — the `self.` receiver is carried so
    /// the M-. self-resolution pre-step sees it; every other `.` receiver
    /// stays bare). A cursor parked just AFTER the name (before
    /// `(`, `.`, or whitespace — the usual call-site spot) counts. `None`
    /// when the point is not on (or immediately after) an identifier run.
    ///
    /// 011-06: the PATH TOKEN is language-aware. Rust's `::` shape is the
    /// char-scan below, byte-for-byte unchanged; a Rust `.` field access
    /// stays bare EXCEPT the `self.` receiver (010-01 — `self.name` →
    /// `self.name`, which the M-. self-resolution pre-step consumes; the
    /// other receivers' fields are not in the index). In a non-Rust
    /// buffer, when the point sits inside the language's dotted path
    /// container, the path token becomes the WHOLE dotted path
    /// (`json.dumps`, `ns.member`, `pkg.Fn`) so the providers' already-
    /// unit-tested dotted handling is reachable from M-.: ONE parse
    /// (`syntax::node::node_at` — 011-03's whole-path machinery, 007-01's
    /// one-parse discipline). When `node_at` returns `None` (no tree / a
    /// shape it does not cover / the identifier is not a full
    /// dot-delimited segment of the container, e.g. a computed member
    /// `a[b]`) the exact current bare extraction stands — never guess.
    pub(super) fn symbol_at_point(
        lang: LanguageId,
        text: &str,
        col: usize,
    ) -> Option<(String, String)> {
        let chars: Vec<char> = text.chars().collect();
        if col > chars.len() {
            return None;
        }
        let is_ident = |c: char| c.is_alphanumeric() || c == '_';
        // 006-02b item 4: a cursor parked on the SECOND colon of a `::`
        // separator (the point's own char and its predecessor are both
        // `:`) sits at the very end of the preceding path segment — treat
        // the point as one char to the left (the segment's end) so the
        // identifier / path token extract per the existing rules
        // (`a::b` → `("a", "a::b")`). The FIRST colon is already covered:
        // the char before the point is the segment's last identifier char.
        let col = if col >= 2
            && col < chars.len()
            && chars[col] == ':'
            && chars[col - 1] == ':'
            && is_ident(chars[col - 2])
        {
            col - 1
        } else {
            col
        };
        // The run the point belongs to: the char AT the point when it is an
        // identifier char, else the run ending immediately BEFORE it.
        let start = if col < chars.len() && is_ident(chars[col]) {
            let mut i = col;
            while i > 0 && is_ident(chars[i - 1]) {
                i -= 1;
            }
            i
        } else if col > 0 && is_ident(chars[col - 1]) {
            let mut i = col - 1;
            while i > 0 && is_ident(chars[i - 1]) {
                i -= 1;
            }
            i
        } else {
            return None;
        };
        let mut end = start;
        while end < chars.len() && is_ident(chars[end]) {
            end += 1;
        }
        let identifier: String = chars[start..end].iter().collect();
        // Extend left across `::` separators to the full path token (kept for
        // the tooling resolver; the workspace index stays name-keyed and is
        // tried with both the last segment and this token).
        let mut pstart = start;
        while pstart >= 3 && chars[pstart - 1] == ':' && chars[pstart - 2] == ':' {
            // The segment before the `::`: the identifier run ending at index
            // `pstart - 3`. A non-identifier there means a leading `::` — stop.
            let after_sep = pstart - 3;
            if !is_ident(chars[after_sep]) {
                break;
            }
            let mut i = after_sep;
            while i > 0 && is_ident(chars[i - 1]) {
                i -= 1;
            }
            pstart = i;
        }
        let mut pend = end;
        while pend + 2 < chars.len()
            && chars[pend] == ':'
            && chars[pend + 1] == ':'
            && is_ident(chars[pend + 2])
        {
            pend += 3;
            while pend < chars.len() && is_ident(chars[pend]) {
                pend += 1;
            }
        }
        let path_token: String = chars[pstart..pend].iter().collect();
        // 010-01: the Rust self-receiver — when the identifier run is the
        // member of `self.` (the run is preceded by `.self`, exactly), the
        // path token carries the receiver (`self.bar`), so the M-.
        // self-resolution pre-step (in `xref_find_definitions`) sees it.
        // Every other Rust `.` access stays bare (byte-for-byte — fields
        // of arbitrary receivers are still not resolvable); a prefix like
        // `myself.` must NOT match (the 4-char window must be exactly
        // `self`, not preceded by an identifier char).
        let path_token = if lang == LanguageId::Rust
            && pstart == start
            && start >= 5
            && chars[start - 1] == '.'
            && chars[start - 5..start - 1] == ['s', 'e', 'l', 'f']
            && (start < 6 || !is_ident(chars[start - 6]))
        {
            format!("self.{}", identifier)
        } else {
            path_token
        };
        // 011-06: non-Rust — upgrade the path token to the whole dotted
        // path when the point sits inside the language's path container.
        // `node_at` takes a byte offset: the identifier run's last char
        // (always in-range — the run is non-empty here). Any miss keeps
        // the bare extraction byte-for-byte (the `::` scan above is the
        // Rust shape; `.` never extends it).
        let path_token = if lang != LanguageId::Rust
            && let Some(byte) = text.char_indices().nth(end - 1).map(|(b, _)| b)
            && let Some(info) = crate::syntax::node::node_at(lang, text, byte)
            && Self::dotted_path_container(lang, &info.kind)
            && info.text.split('.').any(|seg| seg == identifier)
            // 011-06 review P1: EVERY dot-delimited segment must be a bare
            // identifier. The raw container text of a wrong-container shape
            // is NOT a dotted path — `a?.b` (JS/TS optional chaining),
            // `foo().bar` (Python attribute-on-call), `(*p).field` (Go
            // pointer receiver) all match the segment-membership check but
            // their non-identifier segments would be treated as package /
            // module names by the providers (an unintended `npm install
            // "a?"` / `pip install "foo()"` shell-out in online projects).
            // Only a genuine path upgrades; these fall back to the
            // byte-for-byte bare extraction.
            && info
                .text
                .split('.')
                .all(|seg| !seg.is_empty() && seg.chars().all(is_ident))
        {
            info.text
        } else {
            path_token
        };
        Some((identifier, path_token))
    }

    /// 011-06: the per-language DOTTED path-container node kinds — exactly
    /// the containers 011-03's `node_at` whole-path machinery returns, as
    /// pinned per language in `src/syntax/node.rs`: JS/TS/TSX
    /// `member_expression` (`a.b.c`) and the TS-only nested type
    /// identifiers, Python's `attribute` (`a.b.c`), Go's
    /// `selector_expression` / `qualified_type` (`pkg.Fn`), C's
    /// `field_expression` (`o.x` — `p->x` fails the caller's
    /// all-identifier-segment check and stays bare), Cpp's
    /// `field_expression` (its `::` shape is ALREADY carried whole by
    /// `symbol_at_point`'s byte-scan — `qualified_identifier` would be
    /// byte-for-byte the same token, so it is deliberately NOT enumerated
    /// here: no double handling), Toml's `dotted_key` (`a.b.c` — the
    /// index stores the dotted key as ONE symbol name, so without the
    /// upgrade an M-. on a segment could never hit it), Java's
    /// `field_access` (`A.c` — `o.m(…)` is a `method_invocation`, NOT a
    /// container: node.rs returns the bare `m` identifier there) plus
    /// `scoped_identifier` / `scoped_type_identifier` (`com.example.Foo`
    /// — the Rust `::` shape, dot-delimited), C#'s
    /// `member_access_expression` (`o.P` / `a.b.c`) + `qualified_name`
    /// (`N.Inner`), and Ruby's argumentless `call` (`a.b.c` — `node_at`
    /// returns a `call` node only while it is a genuine path container:
    /// a `receiver` field AND no `arguments` field, the same gate as
    /// node.rs's `in_identifier_position`, so an argument-carrying call
    /// NEVER reaches this arm; its inner segments come back as bare
    /// identifiers instead). Ruby's `scope_resolution` (`Foo::Bar`) is
    /// deliberately NOT enumerated: the `::` byte-scan above already
    /// carries it whole, and the all-identifier-segment check would
    /// reject a `Foo::Bar` segment anyway (no double handling). Json has
    /// NO container: the pinned JSON grammar has
    /// no dotted-key node — every key is a standalone string — so a JSON
    /// key's M-. stays the byte-for-byte bare index lookup (judgment: the
    /// index fall-through already lands bare keys; there is no key-PATH
    /// to speak of). Scheme has no dotted-path construct at all (the
    /// flat S-expression grammar — `is_path_segment`'s default arm).
    /// Rust is out of scope here — its `::` shape is extracted
    /// byte-for-byte in `symbol_at_point` itself.
    fn dotted_path_container(lang: LanguageId, kind: &str) -> bool {
        match lang {
            LanguageId::JavaScript | LanguageId::TypeScript | LanguageId::Tsx => {
                matches!(
                    kind,
                    "member_expression" | "nested_type_identifier" | "nested_identifier"
                )
            }
            LanguageId::Python => kind == "attribute",
            LanguageId::Go => {
                matches!(kind, "selector_expression" | "qualified_type")
            }
            LanguageId::C => kind == "field_expression",
            LanguageId::Cpp => kind == "field_expression",
            LanguageId::Toml => kind == "dotted_key",
            LanguageId::Java => matches!(
                kind,
                "field_access" | "scoped_identifier" | "scoped_type_identifier"
            ),
            LanguageId::CSharp => {
                matches!(kind, "member_access_expression" | "qualified_name")
            }
            // node_at returns a `call` node only for the argumentless
            // receiver-carrying shape (node.rs's `in_identifier_position`
            // gate), so `a.b(1)` never arrives here — the caller's
            // all-identifier-segment check additionally rejects any
            // `call` whose text carries a `(...)` segment (`a.b(1).c`
            // → the `b(1)` segment is not a bare identifier → bare
            // extraction, byte-for-byte).
            LanguageId::Ruby => kind == "call",
            _ => false,
        }
    }

    /// `M-i`: imenu — open a picker over the current file's outline.
    pub fn open_imenu(&mut self) {
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let Some(buf) = self.buffers.get(&key) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let Some(path) = buf.path.as_ref() else {
            self.minibuffer_message("no file (scratch buffer)");
            return;
        };
        let Some(project) = self.project.as_ref() else {
            self.minibuffer_message("no project");
            return;
        };
        // 006-03: an EXTERNAL (registry / tooling) buffer gets its outline
        // from the owning crate's index (below) instead of refusing; a
        // non-external buffer outside the root keeps the refusal.
        match path.strip_prefix(&project.root) {
            Ok(_) => {}
            Err(_) if self.external_buffers.contains(&key) => {}
            Err(_) => {
                self.minibuffer_message("buffer not in project");
                return;
            }
        }
        let outline = self.current_buffer_outline();
        if outline.is_empty() {
            self.minibuffer_message("no symbols in current file");
            return;
        }
        // Watchlist (impl-parent grouping): the file's Rust tables (Rung 1)
        // group the impl methods under their impl's type; other languages
        // have no tables and stay flat.
        let tables = self.current_buffer_rust_tables();
        self.open_imenu_picker(outline, tables);
    }

    /// The current buffer's symbol outline: the project index for project
    /// files, the OWNING crate's index for external (registry / tooling)
    /// buffers (006-03), empty for scratch / out-of-project non-external.
    pub(super) fn current_buffer_outline(&mut self) -> Vec<crate::nav::index::Symbol> {
        let Some(key) = self.buffers.current().map(String::from) else {
            return Vec::new();
        };
        let Some(path) = self.buffers.get(&key).and_then(|b| b.path.clone()) else {
            return Vec::new();
        };
        let Some(project) = self.project.as_ref() else {
            return Vec::new();
        };
        match path.strip_prefix(&project.root) {
            Ok(rel) => self.index.outline(&rel.to_string_lossy()).to_vec(),
            Err(_) if self.external_buffers.contains(&key) => self
                .crate_index_arc_for_path(&path)
                .and_then(|(root, arc)| {
                    // 006-03b item 4: a `strip_prefix` miss yields the
                    // empty outline (the existing fallback), never a
                    // panic.
                    let crel = Self::crate_rel(&path, &root)?;
                    let idx = arc.lock().unwrap();
                    Some(idx.outline(&crel).to_vec())
                })
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    /// The current buffer's Rust tables (plan 010 Shape A, rung 1): the
    /// project index for project files, the OWNING crate's index for
    /// external (registry / tooling) buffers (006-03), `None` otherwise
    /// (scratch / out-of-project non-external, or a non-Rust file — the
    /// tables only exist for Rust). The imenu impl-parent grouping's data
    /// source (watchlist item).
    pub(super) fn current_buffer_rust_tables(&mut self) -> Option<crate::syntax::queries::RustTables> {
        let key = self.buffers.current().map(String::from)?;
        let path = self.buffers.get(&key).and_then(|b| b.path.clone())?;
        let project = self.project.as_ref()?;
        match path.strip_prefix(&project.root) {
            Ok(rel) => self.index.tables(&rel.to_string_lossy()).cloned(),
            Err(_) if self.external_buffers.contains(&key) => self
                .crate_index_arc_for_path(&path)
                .and_then(|(root, arc)| {
                    // The same strip_prefix-miss tolerance as
                    // `current_buffer_outline`: `None`, never a panic.
                    let crel = Self::crate_rel(&path, &root)?;
                    Some(arc.lock().unwrap().tables(&crel).cloned())
                })
                .flatten(),
            Err(_) => None,
        }
    }

    /// Open the imenu picker over `outline` + the file's Rust `tables`
    /// (indentation by enclosing extent, plus the Rust impl-parent
    /// grouping — shared by the project and external-buffer paths).
    fn open_imenu_picker(
        &mut self,
        outline: Vec<crate::nav::index::Symbol>,
        tables: Option<crate::syntax::queries::RustTables>,
    ) {
        let candidates: Vec<PickerCandidate> = outline
            .iter()
            .map(|s| Self::imenu_candidate(s, &outline, tables.as_ref()))
            .collect();
        self.open_picker(PickerKind::Imenu, "Imenu: ", candidates);
    }

    /// `C-c p s`: project-wide symbol picker (fuzzy over all index symbols).
    pub fn open_symbol_picker(&mut self) {
        if self.project.is_none() {
            self.minibuffer_message("no project");
            return;
        }
        if self.index.total() == 0 {
            self.minibuffer_message("no symbols indexed yet");
            return;
        }
        // The symbol picker lists the PROJECT index — any open Xref
        // picker's crate root (006-03) must not leak into it.
        self.xref_crate_root = None;
        let candidates: Vec<PickerCandidate> = self
            .index
            .all_locations()
            .into_iter()
            .map(|loc| PickerCandidate {
                name: format!("{}:{}", loc.file, loc.symbol.line + 1),
                display: format!("{}  [{}]  {}", loc.symbol.name, loc.symbol.kind.tag(), loc.file),
                label: loc.symbol.name.clone(),
                detail: format!("[{}] {}", loc.symbol.kind.tag(), loc.file),
                docs: String::new(),
                category: "symbol".to_string(),
            })
            .collect();
        self.open_picker(PickerKind::Symbols, "Symbol: ", candidates);
    }

    /// The enclosing symbol name for the current buffer's cursor line,
    /// for the status-line which-function display.
    pub fn which_function(&self) -> String {
        let Some(key) = self.buffers.current().map(String::from) else {
            return String::new();
        };
        let Some(buf) = self.buffers.get(&key) else {
            return String::new();
        };
        let Some(path) = buf.path.as_ref() else {
            return String::new();
        };
        let Some(project) = self.project.as_ref() else {
            return String::new();
        };
        let Ok(rel) = path.strip_prefix(&project.root) else {
            return String::new();
        };
        let rel = rel.to_string_lossy();
        let line = self.point_line();
        let outline = self.index.outline(&rel);
        crate::nav::index::enclosing_symbol(outline, line)
            .map(|s| s.name.clone())
            .unwrap_or_default()
    }
}

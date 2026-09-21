use super::*;

/// (M-., phase (1)) The point context the symbol-under-point candidate
/// lookup is pure over: the point line's text, the buffer rope, and the
/// point's (line, column). A parameter grouping (the lookup has nine
/// inputs; clippy's argument cap is seven).
struct XrefPointContext<'a> {
    line_text: &'a str,
    rope: &'a Rope,
    line: usize,
    col: usize,
}

impl AppStore {
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

        // The candidate lookup below is pure over the index and the buffer
        // (no `&mut self` inside), so the `buf` borrow still ends before
        // the later `&mut self` work — the 006-03 rule above.
        let defs = at
            .as_ref()
            .and_then(|(ident, path_token)| {
                Self::xref_symbol_at_point_candidates(
                    &self.index,
                    lang,
                    &XrefPointContext {
                        line_text: &line_text,
                        rope: &buf.rope,
                        line,
                        col: self.point_col(),
                    },
                    &rel,
                    ident,
                    path_token,
                )
            })
            .unwrap_or_default();

        let lookup_name: String;
        let defs: Vec<crate::nav::index::Location> = if !defs.is_empty() {
            // (2) Symbol-at-point with definitions: first candidate after the
            // same-file-first order (the picker's lookup label).
            lookup_name = defs[0].symbol.name.clone();
            defs
        } else {
            // (3) Enclosing-symbol fallback (unchanged: by line, not by the
            // point's column).
            match self.xref_enclosing_symbol_fallback(&rel, line) {
                Some((name, defs)) => {
                    lookup_name = name;
                    defs
                }
                None => {
                    // (4) Nothing the workspace knows about under/near the point:
                    // fall through to the tooling resolver when the point sits on
                    // a symbol (otherwise behave as before: no symbol under point).
                    match &at {
                        Some((_, path_token)) => self.start_symbol_resolution(path_token, &rel),
                        None => self.minibuffer_message("no symbol under point"),
                    }
                    return;
                }
            }
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
            self.xref_jump_unique_definition(defs);
        } else {
            self.xref_open_ambiguous_picker(lookup_name, defs);
        }
    }

    /// (M-., phase (1)) The definition candidates for the symbol AT THE
    /// POINT: the 010-01 Rust self-receiver pre-step, then the 010-03
    /// local-binding pre-step, then the name-keyed index lookup
    /// (`xref_definition_candidates`). Pure over the index and the buffer
    /// (no `&mut self`, so the caller's `buf` borrow never spans a
    /// `&mut self` call — the 006-03 rule).
    fn xref_symbol_at_point_candidates(
        index: &SymbolIndex,
        lang: LanguageId,
        point: &XrefPointContext,
        rel: &str,
        ident: &str,
        path_token: &str,
    ) -> Option<Vec<crate::nav::index::Location>> {
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
            let source = point.rope.to_string();
            if let Some(byte) = point_byte_offset(point.rope, point.line, point.col) {
                let cands = Self::self_receiver_candidates(index, rel, &source, byte, member);
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
                Self::rust_dotted_receiver(point.line_text, point.col)
        {
            let source = point.rope.to_string();
            if let Some(byte) = point_byte_offset(point.rope, point.line, member_col) {
                let cands = Self::local_binding_candidates(
                    index,
                    rel,
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
        Self::xref_definition_candidates(index, ident, path_token, rel)
    }

    /// (M-., phase (3)) The enclosing-symbol fallback: the enclosing
    /// symbol's definitions take over, and a workspace hit on THAT never
    /// triggers the resolver (no resolver spam). `None` when the file has
    /// no symbol at `line` — the caller takes the (4) tooling-resolver
    /// seam.
    fn xref_enclosing_symbol_fallback(
        &self,
        rel: &str,
        line: usize,
    ) -> Option<(String, Vec<crate::nav::index::Location>)> {
        let outline = self.index.outline(rel);
        let sym = crate::nav::index::enclosing_symbol(outline, line)?;
        let lookup_name = sym.name.clone();
        let defs = self.index.definitions_of(&lookup_name);
        Some((lookup_name, defs))
    }

    /// (M-., dispatch) The single-definition jump (the
    /// `defs.len() == 1` case).
    fn xref_jump_unique_definition(&mut self, defs: Vec<crate::nav::index::Location>) {
        // Unique (same-file or cross-file): capture origin, navigate,
        // record jump.
        let origin = self.current_jump_entry();
        let def = &defs[0];
        self.open_path(&def.file);
        // Move the point to the definition's line AND the name's
        // column: `start_byte` is an absolute FILE byte offset
        // (tree-sitter's name-node byte range), so land it via the
        // byte→(line, char column) conversion — a line-only landing
        // drops the name's column (a raw byte column is off-by-N on
        // multibyte lines). The line stays `def.symbol.line` (the
        // indexed line); `None` (out of bounds, e.g. a stale index
        // after an external edit) keeps the old col-0 landing.
        let def_col = self
            .buffers
            .current_buffer()
            .and_then(|b| b.try_byte_to_line_col(def.symbol.start_byte))
            .map(|(_, col)| col)
            .unwrap_or(0);
        self.set_point(def.symbol.line, def_col, def_col);
        self.recenter_landing();
        self.ensure_highlight();
        self.record_jump(origin, "M-.");
        self.minibuffer_message(&format!("jumped to {}: {}", def.file, def.symbol.line + 1));
    }

    /// (M-., dispatch) The ambiguous-definition picker (the
    /// `defs.len() > 1` case).
    fn xref_open_ambiguous_picker(
        &mut self,
        lookup_name: String,
        defs: Vec<crate::nav::index::Location>,
    ) {
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
    pub(super) fn xref_definition_candidates(
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
    pub(super) fn self_receiver_candidates(
        index: &SymbolIndex,
        rel: &str,
        source: &str,
        byte: usize,
        member: &str,
    ) -> Vec<crate::nav::index::Location> {
        let Some(type_name) = redline_syntax::queries::rust_self_type_at(source, byte) else {
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
    pub(super) fn local_binding_candidates(
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
            redline_syntax::queries::rust_binding_type_at(tables, source, byte, receiver)
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
        let mk = |file: String, kind: redline_syntax::queries::SymbolKind, line: usize| {
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
                redline_syntax::queries::SymbolKind::Constant,
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
                    redline_syntax::queries::SymbolKind::Function,
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
    pub(in crate::app::store) fn rust_dotted_receiver(line_text: &str, col: usize) -> Option<(String, usize)> {
        let chars: Vec<char> = line_text.chars().collect();
        if col > chars.len() {
            return None;
        }
        // C15: word-constituent is the crate-wide Unicode rule.
        let is_ident = is_word_char;
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
    pub(in crate::app::store) fn resolution_language(&self, from_file: &str) -> Option<String> {
        let lang = self.grammar_registry.language_for(from_file);
        (lang != redline_syntax::registry::LanguageId::Plain).then(|| lang.name().to_string())
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
    pub(in crate::app::store) fn resolver_scope(&self, symbol: &str) -> Vec<String> {
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
        let lang = redline_syntax::registry::resolve_language(&path.display().to_string());
        let source = rope.to_string();
        let Some(byte) = point_byte_offset(rope, line, col) else {
            return Vec::new();
        };
        match lang {
            // 007-03 (Rust): bare → the `use` declaration's path; path-
            // shaped → the enclosing item chain (carried, not consumed).
            redline_syntax::registry::LanguageId::Rust => {
                if !symbol.contains("::") {
                    return Self::use_path_for_symbol(&source, byte, symbol).unwrap_or_default();
                }
                redline_syntax::node::scope_path_at(lang, &source, byte)
            }
            // 011-02: the per-language import walks (bare symbols), plus
            // the JS/TS namespace-member rewrite for path-shaped symbols.
            redline_syntax::registry::LanguageId::JavaScript
            | redline_syntax::registry::LanguageId::TypeScript
            | redline_syntax::registry::LanguageId::Tsx => {
                Self::js_ts_scope_for(lang, &source, byte, symbol)
            }
            redline_syntax::registry::LanguageId::Python => {
                Self::python_scope_for(&source, byte, symbol)
            }
            redline_syntax::registry::LanguageId::Go => Self::go_scope_for(&source, byte, symbol),
            // Every other language (and Plain): no hint — the providers
            // keep their exact no-hint behavior.
            _ => Vec::new(),
        }
    }
}

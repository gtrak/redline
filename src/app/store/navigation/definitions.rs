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

/// (jump-ambiguity) The M-. candidate lookup's outcome, shared by `M-.`
/// (which applies the silent-jump rule) and `M->` (which always opens
/// the picker). The guard messages are delivered here; the caller just
/// returns.
enum XrefMdotOutcome {
    /// Definition candidates (symbol-at-point, or the enclosing-symbol
    /// fallback — `from_enclosing` says which: the fallback is a by-
    /// LINE guess and is therefore never a silent jump).
    Candidates {
        lookup_name: String,
        defs: Vec<crate::nav::index::Location>,
        rel: String,
        from_enclosing: bool,
    },
    /// The (4) tooling-resolver seam: `(path-shaped token, from file)`.
    Resolver(String, String),
    /// The enclosing name has no indexed definition (no symbol under the
    /// point either): the "no definition for X" report.
    NoDefinition(String),
    /// No symbol under the point and no enclosing symbol: the "no symbol
    /// under point" report.
    NoSymbol,
    /// issue-language-aware-symbols (Part 2): a Clojure namespace alias
    /// (`alias/var`) that the current file's top-level `ns` form does not
    /// declare — flagged, NEVER guessed (a wrong jump is worse than no
    /// jump).
    UnresolvableAlias(String),
    /// A guard message ("no buffer", "no file…", …) was already
    /// reported; the caller returns.
    Guarded,
}

/// (jump-ambiguity) The Xref picker row for one definition location:
/// the SOURCE of the row is visible — `"here"` when `d` is in the
/// current file (`cur`, keyed the way the picker's index keys it: project-
/// relative, or crate-relative when the picker is crate-rooted),
/// `"other"` when it is not; no marker when there is no current-file
/// context. `display` (the match target) and `detail` carry the
/// `[<kind>·<here|other>]` tag; `name` stays `file:line` byte-for-byte.
pub(in crate::app::store) fn xref_location_candidate(d: &crate::nav::index::Location, cur: Option<&str>) -> PickerCandidate {
    let marker = match cur {
        Some(c) if c == d.file => "here",
        Some(_) => "other",
        None => "",
    };
    let tag = format!("[{}{}{}]", d.symbol.kind.tag(), if marker.is_empty() { "" } else { "·" }, marker);
    let name = format!("{}:{}", d.file, d.symbol.line + 1);
    PickerCandidate {
        name: name.clone(),
        display: format!("{}  {} {}", name, tag, d.symbol.name),
        label: d.symbol.name.clone(),
        detail: format!("{} {}:{}", tag, d.file, d.symbol.line + 1),
        docs: String::new(),
        category: "xref".to_string(),
        ann_col: None,
    }
}

/// (jump-ambiguity) The tooling row's `file` display string in the
/// picker's keying: crate-relative when the file sits under the
/// picker's crate root (006-03 keying — a registry landing IS a file of
/// its crate), project-relative inside the project, absolute otherwise.
pub(in crate::app::store) fn tooling_file_display(
    source: &ResolvedSource,
    crate_root: &Option<PathBuf>,
    project: &Option<Project>,
) -> String {
    if let Some(root) = crate_root
        && let Ok(rel) = source.file.strip_prefix(root)
    {
        return rel.to_string_lossy().into_owned();
    }
    if let Some(p) = project
        && let Ok(rel) = source.file.strip_prefix(&p.root)
    {
        return rel.to_string_lossy().into_owned();
    }
    source.file.display().to_string()
}

/// (jump-ambiguity) The tooling row's `name` ("file:line", 1-based line,
/// file in the picker's keying): the pure derivation shared by
/// `xref_tooling_candidate` and `run_selected`'s landing discriminator.
pub(in crate::app::store) fn tooling_candidate_name(
    source: &ResolvedSource,
    crate_root: &Option<PathBuf>,
    project: &Option<Project>,
) -> String {
    let file = tooling_file_display(source, crate_root, project);
    format!("{file}:{}", tooling_landing_line(source) + 1)
}

/// (jump-ambiguity) The tooling landing line, 0-based: a provider
/// emitting 0 is "no line" (006-02b item 6) — the top of the file.
pub(in crate::app::store) fn tooling_landing_line(source: &ResolvedSource) -> usize {
    source
        .line
        .filter(|l| *l > 0)
        .map(|l| (l - 1) as usize)
        .unwrap_or(0)
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
    ///
    /// Selection rule 5 (jump-ambiguity): a silent jump happens IFF there
    /// is EXACTLY ONE candidate AND it is in the CURRENT file — the one
    /// case where you can see the target yourself. Everything else opens
    /// the Xref picker with the best candidate preselected (`open_picker`
    /// starts at index 0 and the candidates are same-file-first, so RET
    /// accepts the top guess in one keystroke): a cross-file unique
    /// candidate (a same-named symbol in another file is not "the" one),
    /// 2+ candidates, and the enclosing-symbol fallback (a by-LINE guess,
    /// never a silent jump). A tooling resolve never jumps silently either
    /// (see `apply_resolve_event` → `xref_tooling_resolve_to_picker`).
    pub fn xref_find_definitions(&mut self) {
        match self.xref_mdot_candidates() {
            XrefMdotOutcome::Candidates { lookup_name, defs, rel, from_enclosing } => {
                let silent = defs.len() == 1
                    && !from_enclosing
                    && defs[0].file == rel;
                if silent {
                    self.xref_jump_unique_definition(defs);
                } else {
                    self.xref_open_ambiguous_picker(lookup_name, defs);
                }
            }
            XrefMdotOutcome::UnresolvableAlias(alias) => {
                // issue-language-aware-symbols (Part 2): flagged, never
                // guessed (a wrong jump is worse than no jump).
                self.minibuffer_message(&format!("cannot resolve namespace alias `{alias}`"))
            }
            XrefMdotOutcome::Resolver(token, rel) => {
                self.start_symbol_resolution(&token, &rel, None)
            }
            XrefMdotOutcome::NoDefinition(name) => {
                self.minibuffer_message(&format!("no definition for `{name}`"))
            }
            XrefMdotOutcome::NoSymbol => {
                self.minibuffer_message("no symbol under point")
            }
            XrefMdotOutcome::Guarded => {}
        }
    }

    /// (M->, jump-ambiguity) Force the candidate list: the SAME lookup as
    /// `M-.` but the selection rule is BYPASSED — the Xref picker opens
    /// even for a same-file unique candidate (the "when I know I want to
    /// search for the jump point" escape hatch). No candidates at all
    /// keeps the M-. fall-through discipline (resolver when the point
    /// sits on a symbol, the same guard messages otherwise).
    pub fn xref_find_definitions_picker(&mut self) {
        match self.xref_mdot_candidates() {
            XrefMdotOutcome::Candidates { lookup_name, defs, .. } => {
                self.xref_open_ambiguous_picker(lookup_name, defs);
            }
            XrefMdotOutcome::UnresolvableAlias(alias) => {
                self.minibuffer_message(&format!("cannot resolve namespace alias `{alias}`"))
            }
            XrefMdotOutcome::Resolver(token, rel) => {
                self.start_symbol_resolution(&token, &rel, None)
            }
            XrefMdotOutcome::NoDefinition(name) => {
                self.minibuffer_message(&format!("no definition for `{name}`"))
            }
            XrefMdotOutcome::NoSymbol => {
                self.minibuffer_message("no symbol under point")
            }
            XrefMdotOutcome::Guarded => {}
        }
    }

    /// (jump-ambiguity) The M-. / M-> shared candidate lookup: the guards
    /// (no buffer / scratch / no project / external-buffer redirect / not
    /// in project), the supersede bump (006-02b item 2), the symbol-at-
    /// point candidate gather, and the enclosing-symbol fallback (3).
    /// A superseded tooling row is dropped here (a new M-. press means the
    /// previous request's pending landing is no longer live).
    fn xref_mdot_candidates(&mut self) -> XrefMdotOutcome {
        // A new M-. / M-> press supersedes any pending tooling landing.
        self.xref_tooling_pending = None;
        // Get the current file's project-relative path.
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("no buffer");
            return XrefMdotOutcome::Guarded;
        };
        let Some(buf) = self.buffers.get(&key) else {
            self.minibuffer_message("no buffer");
            return XrefMdotOutcome::Guarded;
        };
        // An OWNED path: the `buf` borrow must not span the `&mut self`
        // calls below (the external-buffer navigation, 006-03).
        let path = match &buf.path {
            Some(p) => p.clone(),
            None => {
                self.minibuffer_message("no file (scratch buffer)");
                return XrefMdotOutcome::Guarded;
            }
        };
        let Some(project) = self.project.as_ref() else {
            self.minibuffer_message("no project");
            return XrefMdotOutcome::Guarded;
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
            return XrefMdotOutcome::Guarded;
        };
        let rel = rel.to_string_lossy().into_owned();

        // 006-02b item 2: supersede any in-flight tooling resolve — a
        // successful workspace hit must not be clobbered by a stale event
        // from a superseded request that lands later. The fall-through
        // path re-bumps in `start_symbol_resolution`, which is fine (a
        // generation only needs to differ; the event carries its own).
        self.resolve_generation += 1;
        // P3-4: the superseded request is no longer the current
        // generation's — its in-flight status does not carry over.
        self.resolve_in_flight = false;

        let line = self.point_line();
        let line_text = buf.line_text(line).unwrap_or_default();
        // (1) The symbol under the point: identifier run around the point's
        // column, plus the path token it belongs to (raw, for the
        // resolver). 011-06: language-aware — in a non-Rust buffer the
        // token is the whole dotted path when the point sits in the
        // language's path container, else the bare extraction. P2-5,
        // gate: the extraction takes the buffer's ROPE (no whole-clone
        // per M-. — the full source materializes only inside the
        // receiver classification, lazily).
        let lang = self.grammar_registry.language_for(&path.to_string_lossy());
        let at = Self::symbol_at_point(lang, &line_text, line, self.point_col(), &buf.rope);

        // The candidate lookup below is pure over the index and the buffer
        // (no `&mut self` inside), so the `buf` borrow still ends before
        // the later `&mut self` work — the 006-03 rule above.
        let defs = match at
            .as_ref()
            .map(|(ident, path_token)| {
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
            }) {
            // issue-language-aware-symbols (Part 2): a flagged
            // unresolvable Clojure alias never jumps (no guess, no
            // fallback — the alias is the whole signal).
            Some(Err(alias)) => return XrefMdotOutcome::UnresolvableAlias(alias),
            other => other.and_then(|r| r.unwrap_or_default()).unwrap_or_default(),
        };

        if !defs.is_empty() {
            // (2) Symbol-at-point with definitions: first candidate after the
            // same-file-first order (the picker's lookup label).
            let lookup_name = defs[0].symbol.name.clone();
            return XrefMdotOutcome::Candidates {
                lookup_name,
                defs,
                rel,
                from_enclosing: false,
            };
        }

        // (3) Enclosing-symbol fallback (unchanged: by line, not by the
        // point's column). Same-file-first order so the picker's
        // preselected row (index 0) is the best guess — the fallback's
        // defs come out of `definitions_of` unsorted.
        match self.xref_enclosing_symbol_fallback(&rel, line) {
            Some((name, mut defs)) if !defs.is_empty() => {
                defs.sort_by(|a, b| {
                    (a.file != rel).cmp(&(b.file != rel)).then_with(|| {
                        (a.file.as_str(), a.symbol.line, &a.symbol.name)
                            .cmp(&(b.file.as_str(), b.symbol.line, &b.symbol.name))
                    })
                });
                XrefMdotOutcome::Candidates {
                    lookup_name: name,
                    defs,
                    rel,
                    from_enclosing: true,
                }
            }
            other => {
                // (4) Nothing the workspace knows about under/near the
                // point: fall through to the tooling resolver when the
                // point sits on a symbol (the point's own path-shaped
                // token — not the enclosing name — is what the resolver
                // gets); the enclosing name with no indexed definition
                // keeps its "no definition for X" report; neither, "no
                // symbol under point".
                if let Some((_, token)) = at {
                    XrefMdotOutcome::Resolver(token, rel)
                } else if let Some((name, _)) = other {
                    XrefMdotOutcome::NoDefinition(name)
                } else {
                    XrefMdotOutcome::NoSymbol
                }
            }
        }
    }

    /// (M-., phase (1)) The definition candidates for the symbol AT THE
    /// POINT: the 010-01 Rust self-receiver pre-step, the 010-03
    /// local-binding pre-step, the issue-language-aware-symbols Part 2
    /// Clojure namespace-alias pre-step, then the name-keyed index lookup
    /// (`xref_definition_candidates`). Pure over the index and the buffer
    /// (no `&mut self`, so the caller's `buf` borrow never spans a
    /// `&mut self` call — the 006-03 rule). `Err(alias)` — the flagged
    /// unresolvable Clojure namespace alias (the caller reports it and
    /// never jumps).
    fn xref_symbol_at_point_candidates(
        index: &SymbolIndex,
        lang: LanguageId,
        point: &XrefPointContext,
        rel: &str,
        ident: &str,
        path_token: &str,
    ) -> Result<Option<Vec<crate::nav::index::Location>>, String> {
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
                    return Ok(Some(cands));
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
                    return Ok(Some(cands));
                }
            }
        }
        // issue-language-aware-symbols (Part 2): the Clojure
        // namespace-alias pre-step. `alias/var` (and the keyword
        // `::alias/var` — the marker is already stripped by the
        // extraction) is a reference to the VAR `var` in the NAMESPACE
        // `alias` stands for: either an `:as` alias THIS file's top-level
        // `ns` form declares, or a FULL namespace name (a dotted `alias`
        // — an alias can never contain a `.`). The jump targets the var,
        // NARROWED to the files the namespace convention places it in
        // (dots → `/`, hyphens → `_`, the three source extensions — the
        // real-layout convention, `redline_syntax::clojure`). A DOTLESS
        // alias the ns form does not declare is flagged, never guessed.
        if lang == LanguageId::Clojure
            && let Some((alias, var)) = path_token.rsplit_once('/')
            && !var.is_empty()
        {
            let ns_name: Option<String> = if alias.contains('.') {
                // A full namespace reference — identity (no declaration
                // needed).
                Some(alias.to_string())
            } else {
                // The alias must be declared by THIS file's ns form (the
                // full source materializes here — the pre-step's own
                // parse, the same lazy shape as the receiver
                // classifications above).
                let source = point.rope.to_string();
                redline_syntax::clojure::ns_aliases(&source)
                    .and_then(|aliases| {
                        aliases
                            .into_iter()
                            .find(|(a, _)| a == alias)
                            .map(|(_, ns)| ns)
                    })
            };
            match ns_name {
                Some(ns) => {
                    let tails = redline_syntax::clojure::namespace_file_tails(&ns);
                    if let Some(all) = Self::xref_definition_candidates(index, var, path_token, rel) {
                        let narrowed: Vec<crate::nav::index::Location> = all
                            .iter()
                            .filter(|loc| tails.iter().any(|t| loc.file.ends_with(t)))
                            .cloned()
                            .collect();
                        // The convention file(s) carry the indexed var: the
                        // narrowed set (ordering — same-file-first — is
                        // preserved). The convention file has no indexed
                        // `var`: degrade to the BARE name-keyed lookup (the
                        // Part-1 superset — never a fabricated target, never
                        // an empty answer where the name is indexed).
                        return Ok(Some(if narrowed.is_empty() { all } else { narrowed }));
                    }
                    return Ok(None);
                }
                None => {
                    // No ns form, or the alias is not declared: FLAG — a
                    // wrong jump is worse than no jump.
                    return Err(alias.to_string());
                }
            }
        }
        Ok(Self::xref_definition_candidates(index, ident, path_token, rel))
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

    /// (M-., dispatch) The Xref picker for the index candidates (every
    /// non-silent M-. / M-> outcome: ambiguous, cross-file unique, the
    /// enclosing-symbol fallback, the forced list). The jump entry is
    /// recorded when the user selects a candidate (run_selected for
    /// Xref).
    fn xref_open_ambiguous_picker(
        &mut self,
        lookup_name: String,
        defs: Vec<crate::nav::index::Location>,
    ) {
        // Index candidates only: the tooling row belongs to the tooling
        // picker (the supersede at the M-. entry already dropped a
        // pending one; drop it here too — belt and braces, since this is
        // the only other Xref-open seam).
        self.xref_tooling_pending = None;
        self.xref_crate_root = None;
        self.xref_lookup_name = lookup_name;
        let cur = self.xref_current_file_rel(None);
        let candidates = defs
            .iter()
            .map(|d| xref_location_candidate(d, cur.as_deref()))
            .collect();
        self.open_picker(PickerKind::Xref, "Definition: ", candidates);
    }

    /// (jump-ambiguity) The current buffer's file key in the Xref
    /// picker's keying: project-relative (the project index's keys) or,
    /// when the picker is crate-rooted (006-03), crate-relative (the
    /// crate index's keys). The rows' `here`/`other` source marker
    /// compares against this.
    pub(in crate::app::store) fn xref_current_file_rel(
        &mut self,
        root: Option<&PathBuf>,
    ) -> Option<String> {
        let key = self.buffers.current()?.to_string();
        let path = self.buffers.get(&key)?.path.clone()?;
        match root {
            Some(root) => Self::crate_rel(&path, root),
            None => self
                .project
                .as_ref()
                .and_then(|p| path.strip_prefix(&p.root).ok())
                .map(|r| r.to_string_lossy().into_owned()),
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
        // the `buf` borrow must not span it — the 006-03 borrow rule). The
        // SAME extraction as M-. runs here, too (P2-5, gate: the ROPE
        // backs it — no whole-clone per M-x i).
        let line = self.point_line();
        let line_text: String = buf
            .line_text(line)
            .map(|c| c.into_owned())
            .unwrap_or_default();
        let at = Self::symbol_at_point(
            lang,
            &line_text,
            line,
            self.point_col(),
            &buf.rope,
        );
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
        let Some((ident, path_token)) = at else {
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
    /// Each candidate carries the member name's start byte (jump-column-
    /// pty: the tables used to carry a line only, so these landings were
    /// column 0 by construction — the user's "field jumps land at the
    /// beginning of the line" report); the silent jump and the picker's
    /// outline re-read both consume it via the byte→(line,col) conversion.
    fn type_member_candidates(
        index: &SymbolIndex,
        rel: &str,
        type_name: &str,
        member: &str,
    ) -> Vec<crate::nav::index::Location> {
        let mk = |
            file: String,
            kind: redline_syntax::queries::SymbolKind,
            line: usize,
            start_byte: usize,
        | {
            crate::nav::index::Location {
                file,
                symbol: crate::nav::index::Symbol {
                    name: member.to_string(),
                    kind,
                    line,
                    end_line: line,
                    start_byte,
                    end_byte: start_byte,
                },
            }
        };
        let mut out: Vec<crate::nav::index::Location> = Vec::new();
        // Fields: the struct's `field_declaration` lines, all files (the
        // same file orders first below); the entry's start byte is the
        // field name node's byte (the landing's char-column source).
        for loc in index.field_locations(type_name, member) {
            out.push(mk(
                loc.file,
                redline_syntax::queries::SymbolKind::Constant,
                loc.line,
                loc.start_byte,
            ));
        }
        // Methods: the same file's impl tables only (lexical — an impl's
        // methods all live in its own file); the method's start byte is
        // the name node's byte, the same way the outline symbols carry
        // theirs.
        if let Some(tables) = index.tables(rel)
            && let Some(methods) = tables.impls.get(type_name)
        {
            for m in methods.iter().filter(|m| m.method == member) {
                out.push(mk(
                    rel.to_string(),
                    redline_syntax::queries::SymbolKind::Function,
                    m.line,
                    m.start_byte,
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
        // The caller gates this scan to Rust (`lang == LanguageId::Rust`);
        // the Rust row is the unchanged `WordRule::Default`, so the
        // extraction is byte-for-byte today's.
        let is_ident = |c: char| is_word_char(LanguageId::Rust, c);
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

    /// (issue-non-rust-receiver-resolution) Is the HEAD segment of a
    /// non-Rust dotted path a LOCAL VALUE at the point — a name the file
    /// declares as a variable, never a module/package head? The M-. path
    /// token must stay BARE for such receivers (`df.head` where
    /// `df = read_data()` — the in-project index lookup is the honest
    /// answer; a local variable is NOT a package and must never reach a
    /// provider as one — that is the 011-06 P1's `pip install df`
    /// shape). A name bound by an import (a module/package head) or a
    /// name the file declares nowhere stays the 011-06 upgrade (the
    /// fetch-confirmation gate guards the install itself).
    ///
    /// Only the fetch-capable languages classify (`python`,
    /// `javascript`/`typescript`/`tsx`, `go`); every other language
    /// returns `false` (byte-for-byte the 011-06 upgrade — their
    /// resolvers have no install to confirm). Rust never reaches this
    /// (its `.` access stays bare by the extraction itself).
    ///
    /// P2-5, gate: the full source is materialized here (the caller's
    /// extraction runs on the point's line with a line-local offset, so
    /// a bare M-. never pays for a whole-file parse) — and only for the
    /// fetch-capable languages. The point's full-file byte offset comes
    /// from the buffer's authoritative primitive (`point_byte_offset`
    /// over the rope — CRLF-correct; the lane's re-derived `\n`-only
    /// line-start arithmetic drifted one byte per preceding CRLF line,
    /// P1-1, gate).
    ///
    /// The classification is bounded by design (never a guess, the
    /// 007-03 discipline): a parse failure or a point outside the tree
    /// returns `false` (the 011-06 behavior stands — the confirm gate
    /// still guards). The three scope approximations below all lean to
    /// the SAFE side (bare, never a fabricated package) — they are not
    /// a blanket claim about the whole classifier (the P2-1…P2-4 gaps,
    /// once unfixed, leaned the OTHER way): a binding visible in a
    /// sibling block still counts (JS block scoping), comprehension /
    /// generator targets are pruned from the enclosing function's scan
    /// (Python 3 generator scope), and nested function bodies are pruned
    /// from the enclosing scope's scan (their locals are not the
    /// enclosing scope's).
    pub(in crate::app::store) fn dotted_head_is_local_value(
        lang: LanguageId,
        rope: &Rope,
        line: usize,
        col: usize,
        head: &str,
    ) -> bool {
        let Some(byte) = point_byte_offset(rope, line, col) else {
            return false;
        };
        let source = rope.to_string();
        match lang {
            LanguageId::Python => Self::python_head_is_local_value(&source, byte, head),
            LanguageId::JavaScript | LanguageId::TypeScript | LanguageId::Tsx => {
                Self::js_head_is_local_value(&source, byte, head, lang)
            }
            LanguageId::Go => Self::go_head_is_local_value(&source, byte, head),
            // No fetching provider (and Rust's own pre-steps own its
            // receivers): byte-for-byte the 011-06 whole-path upgrade.
            _ => false,
        }
    }

    /// One parse per classification (the 007-03 one-parse discipline).
    fn parse_for(lang: LanguageId, source: &str) -> Option<tree_sitter::Tree> {
        let language = redline_syntax::queries::language_for(lang)?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).ok()?;
        parser.parse(source.as_bytes(), None)
    }

    fn receiver_node_text<'a>(
        node: Option<tree_sitter::Node<'a>>,
        source: &'a [u8],
    ) -> Option<&'a str> {
        node.and_then(|n| n.utf8_text(source).ok())
    }

    /// Does the subtree carry a NAME CARRIER of exactly `name` (tuple /
    /// pattern targets, parameter lists)? A coarse containment — a false
    /// positive only ever keeps the token BARE (the safe side). P2-2,
    /// gate: destructuring carriers count too — a `shorthand_property_
    /// identifier_pattern` (`const { df } = x`) and a `property_identifier`
    /// (a destructuring-pair's property name) carry names exactly like a
    /// plain `identifier` does (documented safe-side approximation: a
    /// pair's KEY name counts as a carrier even though only its VALUE
    /// binds — it only ever keeps the token bare, never fabricates a
    /// package).
    fn subtree_contains_identifier(node: tree_sitter::Node, source: &[u8], name: &str) -> bool {
        if matches!(
            node.kind(),
            "identifier"
                | "shorthand_property_identifier_pattern"
                | "property_identifier"
        ) && Self::receiver_node_text(Some(node), source) == Some(name)
        {
            return true;
        }
        for i in 0..node.child_count() {
            if let Some(c) = node.child(i)
                && Self::subtree_contains_identifier(c, source, name)
            {
                return true;
            }
        }
        false
    }

    /// Python: is `head` a local VALUE at `byte`? A name assigned in a
    /// function is local to that function (Python's scoping rule), so
    /// the enclosing function / class chain + the module level are
    /// scanned (innermost first): an assignment / augmented target, a
    /// `for` target, a `with … as` / `except … as` alias
    /// (`as_pattern`), a walrus (`named_expression`) target, a lambda
    /// parameter, a def / class NAME, or a function parameter of the
    /// same name makes it a value. A comprehension's `for_in_clause`
    /// target is local ONLY at points inside that comprehension (the
    /// generator scope — pruned from the enclosing function's scan,
    /// Python 3). An IMPORT binding is NOT a value (the module head
    /// keeps the 011-06 upgrade). Wildcard imports bind unknown names —
    /// not counted (the fetch-confirmation gate covers that residual).
    fn python_head_is_local_value(source: &str, byte: usize, head: &str) -> bool {
        let Some(tree) = Self::parse_for(LanguageId::Python, source) else {
            return false;
        };
        let root = tree.root_node();
        if !(root.start_byte() <= byte && byte < root.end_byte()) {
            return false;
        }
        let bytes = source.as_bytes();
        // Walk down to the innermost node containing `byte`, then up its
        // ancestors (the scope chain).
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
        // P2-3: a comprehension the point sits in: its `for_in_clause`
        // targets are local AT THE POINT (the generator scope), even
        // though they are NOT the enclosing function's bindings.
        {
            let mut anc = leaf.parent();
            while let Some(a) = anc {
                if matches!(
                    a.kind(),
                    "list_comprehension"
                        | "set_comprehension"
                        | "dictionary_comprehension"
                        | "generator_expression"
                )
                    && Self::py_comprehension_binds(a, bytes, head)
                {
                    return true;
                }
                anc = a.parent();
            }
        }
        let mut anc = leaf.parent();
        let mut scopes: Vec<tree_sitter::Node> = Vec::new();
        while let Some(a) = anc {
            // P2-3: a `lambda` is a scope too (its parameters bind at
            // every point in its body).
            if a.kind() == "function_definition"
                || a.kind() == "class_definition"
                || a.kind() == "lambda"
            {
                scopes.push(a);
            }
            anc = a.parent();
        }
        scopes.push(root);
        let head = head.to_string();
        for scope in &scopes {
            let body = if scope.kind() == "module" {
                *scope
            } else {
                scope.child_by_field_name("body").unwrap_or(*scope)
            };
            if Self::py_scope_binds_value(body, bytes, &head) {
                return true;
            }
            // Parameters of an enclosing function (P2-3: lambda too) are
            // local values at every point in it (`self`, `cls`, arguments).
            if (scope.kind() == "function_definition" || scope.kind() == "lambda")
                && scope
                    .child_by_field_name("parameters")
                    .is_some_and(|p| Self::subtree_contains_identifier(p, bytes, &head))
            {
                return true;
            }
        }
        false
    }

    /// Does a comprehension node's `for_in_clause` TARGETS bind `name`
    /// (every clause of the comprehension — a nested `for` chain)?
    fn py_comprehension_binds(node: tree_sitter::Node, source: &[u8], name: &str) -> bool {
        (0..node.child_count())
            .filter_map(|i| node.child(i))
            .filter(|c| c.kind() == "for_in_clause")
            .any(|c| {
                c.child_by_field_name("left")
                    .is_some_and(|left| Self::py_target_binds(left, source, name))
            })
    }

    /// Does `node`'s subtree bind `name` as a VALUE at THIS scope level?
    /// Nested function / class bodies are PRUNED (their locals belong to
    /// their own scope), but a nested def / class NAME is a value visible
    /// here. Comprehension `for_in_clause` TARGETS are pruned too (Python
    /// 3 generator scope — not the enclosing scope's binding; a point
    /// INSIDE the comprehension is handled by the caller's
    /// comprehension-local check), while a walrus in the iterable / body
    /// still binds the enclosing scope (the comprehension's non-target
    /// children stay scanned). A lambda's body stays scanned (its walruses
    /// bind the enclosing scope) but its parameters are not this scope's
    /// (they are the lambda scope's — the scope-chain's parameter check
    /// owns them). An attribute or subscript LHS (`df.x = …`, `df[0] = …`)
    /// binds NOTHING (the object is referenced, not bound).
    fn py_scope_binds_value(node: tree_sitter::Node, source: &[u8], name: &str) -> bool {
        for c in (0..node.child_count()).filter_map(|i| node.child(i)) {
            match c.kind() {
                "function_definition" | "class_definition" => {
                    if Self::receiver_node_text(c.child_by_field_name("name"), source) == Some(name) {
                        return true;
                    }
                    continue; // prune the body (own scope)
                }
                // Comprehension generator scope — prune the targets, keep
                // the rest (P2-3; the lane's `"comprehension"` kind never
                // matched a real grammar node, so this prune was inert
                // and the targets leaked).
                "list_comprehension"
                | "set_comprehension"
                | "dictionary_comprehension"
                | "generator_expression" => {
                    for gc in (0..c.child_count()).filter_map(|i| c.child(i)) {
                        if gc.kind() == "for_in_clause" {
                            // The target: the comprehension's own scope.
                            // The iterable: a walrus in it binds the
                            // enclosing scope — keep scanning it.
                            if let Some(right) = gc.child_by_field_name("right")
                                && Self::py_scope_binds_value(right, source, name)
                            {
                                return true;
                            }
                        } else if Self::py_scope_binds_value(gc, source, name) {
                            return true;
                        }
                    }
                }
                // P2-3: the `as` alias of a `with` item / `except`
                // clause (`with open(…) as df` / `except E as df`) — a
                // plain local value.
                "as_pattern" => {
                    if c
                        .child_by_field_name("alias")
                        .and_then(|a| Self::receiver_node_text(Some(a), source))
                        .is_some_and(|t| t == name)
                    {
                        return true;
                    }
                }
                // P2-3: a walrus target binds the ENCLOSING scope
                // (Python's rule — the expression's position is
                // irrelevant to the binding).
                "named_expression" => {
                    if c
                        .child_by_field_name("name")
                        .and_then(|n| Self::receiver_node_text(Some(n), source))
                        .is_some_and(|t| t == name)
                    {
                        return true;
                    }
                }
                "expression_statement" => {
                    if let Some(op) = c.child(0)
                        && matches!(op.kind(), "assignment" | "augmented_assignment")
                        && op
                            .child_by_field_name("left")
                            .is_some_and(|left| Self::py_target_binds(left, source, name))
                    {
                        return true;
                    }
                }
                "for_statement"
                    if c
                        .child_by_field_name("left")
                        .is_some_and(|left| Self::py_target_binds(left, source, name)) =>
                {
                    return true;
                }
                _ => {}
            }
            if Self::py_scope_binds_value(c, source, name) {
                return true;
            }
        }
        false
    }

    /// Is a Python assignment / for TARGET a plain binding of `name` (a
    /// single identifier, or a tuple / pattern list carrying it)?
    fn py_target_binds(target: tree_sitter::Node, source: &[u8], name: &str) -> bool {
        match target.kind() {
            "identifier" => Self::receiver_node_text(Some(target), source) == Some(name),
            "pattern_list" | "tuple" => {
                Self::subtree_contains_identifier(target, source, name)
            }
            // P2-3: the grammar's single-target wrapper (e.g. a
            // comprehension `for_in_clause` left) — a plain identifier in
            // pattern position.
            "pattern" => Self::subtree_contains_identifier(target, source, name),
            // attribute / subscript / star target: the object is
            // referenced, not bound — never a local-value binding.
            _ => false,
        }
    }

    /// JS / TS: is `head` a local VALUE at `byte`? `this` / `super` never
    /// are packages. A name bound by an import declaration or a CJS
    /// `require(…)` initializer is a PACKAGE head (keeps the 011-06
    /// upgrade — the provider resolves it, the gate guards the fetch);
    /// every other declaration (const / let / var, function, class,
    /// a parameter) makes it a local value. The scan is BOUNDED (the
    /// 007-03 discipline): the program level + the enclosing function /
    /// class / block bodies — a sibling-block binding counting here is a
    /// safe-side approximation of block scoping (it only ever keeps the
    /// token BARE, never fabricates a package).
    fn js_head_is_local_value(source: &str, byte: usize, head: &str, lang: LanguageId) -> bool {
        if head == "this" || head == "super" {
            return true;
        }
        let Some(tree) = Self::parse_for(lang, source) else {
            return false;
        };
        let root = tree.root_node();
        if !(root.start_byte() <= byte && byte < root.end_byte()) {
            return false;
        }
        let bytes = source.as_bytes();
        // A top-level import / require binding is a PACKAGE head (never a
        // local value): the 011-06 upgrade stands for it.
        if Self::js_top_level_imports_bind(root, bytes, head) {
            return false;
        }
        // The enclosing scope chain: statement blocks / class bodies up
        // to the program root (innermost first); the program level
        // always counts.
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
        let mut anc = leaf.parent();
        // P2-1, gate: the FUNCTION FORMS the point sits in
        // (`function_declaration` / `function` / `arrow_function` /
        // `method_definition`) join the scope chain — their PARAMETERS
        // bind at every point in the body (the lane's `function_
        // declaration` arm pruned the `formal_parameters`, so a named
        // function's params / rest param were never seen as local).
        let mut scopes: Vec<tree_sitter::Node> = Vec::new();
        while let Some(a) = anc {
            if matches!(
                a.kind(),
                "statement_block"
                    | "class_body"
                    | "function_declaration"
                    | "function"
                    | "arrow_function"
                    | "method_definition"
            ) {
                scopes.push(a);
            }
            anc = a.parent();
        }
        scopes.push(root);
        let head = head.to_string();
        for scope in &scopes {
            if scope.kind() == "import_statement" {
                continue;
            }
            // P2-1: a function form on the chain is the function the
            // point sits in — its PARAMETERS are its bindings (scan the
            // parameters field only; the body's own scope is the
            // innermost block already on the chain).
            if matches!(
                scope.kind(),
                "function_declaration" | "function" | "arrow_function" | "method_definition"
            ) {
                if Self::js_function_parameters_bind(*scope, bytes, &head) {
                    return true;
                }
                continue;
            }
            if Self::js_scope_binds_value(*scope, bytes, &head) {
                return true;
            }
        }
        false
    }

    /// P2-1: do the function form's PARAMETERS bind `name`? The
    /// `parameters` field (`formal_parameters`) plus the arrow's single
    /// bare-parameter field (`df => …` — `parameter`, a lone identifier).
    fn js_function_parameters_bind(node: tree_sitter::Node, source: &[u8], name: &str) -> bool {
        if let Some(p) = node.child_by_field_name("parameters")
            && Self::js_formal_parameters_bind(p, source, name)
        {
            return true;
        }
        if let Some(p) = node.child_by_field_name("parameter")
            && p.kind() == "identifier"
            && Self::receiver_node_text(Some(p), source) == Some(name)
        {
            return true;
        }
        false
    }

    /// P2-1: does a `formal_parameters` list bind `name`? Every
    /// parameter position: a bare `identifier` (JS 0.25 named function /
    /// arrow params), a `formal_parameter` (`name` field), a TS
    /// `required_parameter` / `optional_parameter` (`pattern` field), or
    /// a pattern-shaped param (`rest_pattern` / `rest_parameter` /
    /// `assignment_pattern` / `object_assignment_pattern` / … — the
    /// subtree walk, P2-2's shorthand / property carriers included).
    fn js_formal_parameters_bind(node: tree_sitter::Node, source: &[u8], name: &str) -> bool {
        for c in (0..node.child_count()).filter_map(|i| node.child(i)) {
            match c.kind() {
                "identifier" => {
                    if Self::receiver_node_text(Some(c), source) == Some(name) {
                        return true;
                    }
                }
                "formal_parameter" => {
                    if let Some(nm) = c.child_by_field_name("name")
                        && (Self::receiver_node_text(Some(nm), source) == Some(name)
                            || Self::subtree_contains_identifier(nm, source, name))
                    {
                        return true;
                    }
                }
                "required_parameter" | "optional_parameter" => {
                    if let Some(nm) = c.child_by_field_name("pattern")
                        && (Self::receiver_node_text(Some(nm), source) == Some(name)
                            || Self::subtree_contains_identifier(nm, source, name))
                    {
                        return true;
                    }
                }
                _ => {
                    if Self::subtree_contains_identifier(c, source, name) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Does a TOP-LEVEL import declaration (or CJS `require` initializer)
    /// bind `name` — a package head, never a local value?
    fn js_top_level_imports_bind(root: tree_sitter::Node, source: &[u8], name: &str) -> bool {
        for c in (0..root.child_count()).filter_map(|i| root.child(i)) {
            match c.kind() {
                "import_statement" => {
                    // `import X from "p"` / `import { A as B }` /
                    // `import * as ns` / `import { A }`: any clause binding
                    // whose LOCAL name is `name` (a named import's original
                    // name counts too — `import { df } from "p"` binds a
                    // package value head, the provider's documented
                    // handling).
                    if Self::js_clause_binds(c, source, name) {
                        return true;
                    }
                }
                "lexical_declaration" | "variable_declaration"
                    if Self::js_declarators_are_require_bindings(c, source, name) =>
                {
                    // CJS: `const df = require("pkg")` / destructured
                    // `const { df } = require("pkg")` — the module object
                    // (or its exports) are package heads.
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    /// Does an `import_clause` bind the local name `name`?
    fn js_clause_binds(stmt: tree_sitter::Node, source: &[u8], name: &str) -> bool {
        let Some(clause) = (0..stmt.child_count())
            .filter_map(|k| stmt.child(k))
            .find(|n| n.kind() == "import_clause")
        else {
            return false;
        };
        for c in (0..clause.child_count()).filter_map(|i| clause.child(i)) {
            match c.kind() {
                "identifier" => {
                    // default import: `import df from "p"`
                    if Self::receiver_node_text(Some(c), source) == Some(name) {
                        return true;
                    }
                }
                "named_imports" => {
                    for j in 0..c.child_count() {
                        if let Some(entry) = c.child(j)
                            && entry.kind() == "import_specifier"
                        {
                            // The LOCAL name (the alias when present) is
                            // the binding; the original counts too.
                            let local = Self::receiver_node_text(
                                entry
                                    .child_by_field_name("alias")
                                    .or_else(|| entry.child_by_field_name("name")),
                                source,
                            );
                            let original = Self::receiver_node_text(
                                entry.child_by_field_name("name"),
                                source,
                            );
                            if local.is_some_and(|t| t == name)
                                || original.is_some_and(|t| t == name)
                            {
                                return true;
                            }
                        }
                    }
                }
                "namespace_import"
                    if Self::clause_ns_ident(c)
                        .is_some_and(|n| Self::receiver_node_text(Some(n), source) == Some(name)) =>
                {
                    // `import * as ns from "p"`
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    /// The namespace import's local identifier (`import * as ns`).
    fn clause_ns_ident(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
        (0..node.child_count()).filter_map(|k| node.child(k)).find(|n| n.kind() == "identifier")
    }

    /// Does a (top-level) `const/let/var` declare `name` from a `require`
    /// (the CJS import shape — a package head)?
    fn js_declarators_are_require_bindings(
        decl: tree_sitter::Node,
        source: &[u8],
        name: &str,
    ) -> bool {
        for d in (0..decl.child_count()).filter_map(|i| decl.child(i)) {
            if d.kind() != "variable_declarator" {
                continue;
            }
            let Some(value) = d.child_by_field_name("value") else {
                continue;
            };
            let is_require = value.kind() == "call_expression"
                && value
                    .child_by_field_name("function")
                    .is_some_and(|f| {
                        f.kind() == "identifier" && Self::receiver_node_text(Some(f), source) == Some("require")
                    });
            if !is_require {
                continue;
            }
            if let Some(nm) = d.child_by_field_name("name")
                && (Self::receiver_node_text(Some(nm), source) == Some(name)
                    || Self::subtree_contains_identifier(nm, source, name))
            {
                return true;
            }
        }
        false
    }

    /// Does `scope`'s subtree declare `name` as a LOCAL VALUE (a
    /// non-import declaration)? Nested function / class bodies are
    /// scanned too — a safe-side approximation of block scoping (it only
    /// ever keeps the token BARE, never fabricates a package).
    fn js_scope_binds_value(scope: tree_sitter::Node, source: &[u8], name: &str) -> bool {
        for i in 0..scope.child_count() {
            if let Some(c) = scope.child(i)
                && Self::js_node_binds_local_value(c, source, name)
            {
                return true;
            }
        }
        false
    }

    /// Does this node (or its subtree) declare `name` as a local value?
    fn js_node_binds_local_value(node: tree_sitter::Node, source: &[u8], name: &str) -> bool {
        match node.kind() {
            "lexical_declaration" | "variable_declaration" => {
                for d in (0..node.child_count()).filter_map(|i| node.child(i)) {
                    if d.kind() != "variable_declarator" {
                        continue;
                    }
                    // A require initializer is a PACKAGE head (handled by
                    // the import check) — not a local value.
                    let is_require = d
                        .child_by_field_name("value")
                        .is_some_and(|v| v.kind() == "call_expression")
                        && d
                            .child_by_field_name("value")
                            .and_then(|v| v.child_by_field_name("function"))
                            .is_some_and(|f| {
                                f.kind() == "identifier"
                                    && Self::receiver_node_text(Some(f), source) == Some("require")
                            });
                    if is_require {
                        continue;
                    }
                    if let Some(nm) = d.child_by_field_name("name")
                        && (Self::receiver_node_text(Some(nm), source) == Some(name)
                            || Self::subtree_contains_identifier(nm, source, name))
                    {
                        return true;
                    }
                    // The initializer may itself carry BINDINGS: an arrow
                    // function's parameter list (`const f = (df) => …`)
                    // binds `df` in the enclosing scope (and a function
                    // expression's params too). Recurse so those are seen;
                    // nested `function`/`class` declarations early-return on
                    // their own name, so their bodies stay pruned.
                    if let Some(v) = d.child_by_field_name("value")
                        && Self::js_node_binds_local_value(v, source, name)
                    {
                        return true;
                    }
                }
                false
            }
            "function_declaration" | "class_declaration" => {
                Self::receiver_node_text(node.child_by_field_name("name"), source) == Some(name)
            }
            // Parameters: a `formal_parameters` list binds every one of
            // its params in the function body. The param nodes are
            // shaped differently by position: a bare `identifier` (arrow
            // `(df) =>`), a `formal_parameter` (named function, `name`
            // field), a `rest_pattern` / `rest_parameter`, or an
            // `assignment_pattern` / `object_pattern` / `array_pattern`
            // (defaults / destructuring — carry the identifier).
            "formal_parameters" => {
                for c in (0..node.child_count()).filter_map(|i| node.child(i)) {
                    match c.kind() {
                        "identifier" => {
                            if Self::receiver_node_text(Some(c), source) == Some(name) {
                                return true;
                            }
                        }
                        "formal_parameter" => {
                            if let Some(nm) = c.child_by_field_name("name")
                                && (Self::receiver_node_text(Some(nm), source) == Some(name)
                                    || Self::subtree_contains_identifier(nm, source, name))
                            {
                                return true;
                            }
                        }
                        "rest_pattern" | "rest_parameter" | "assignment_pattern"
                        | "object_pattern" | "array_pattern" | "pattern"
                        if Self::subtree_contains_identifier(c, source, name) =>
                        {
                            return true;
                        }
                        _ => {}
                    }
                }
                false
            }
            // A lone parameter node reached directly (not via a list).
            "formal_parameter" | "rest_parameter" | "assignment_pattern" => {
                if let Some(nm) = node.child_by_field_name("name")
                    && (Self::receiver_node_text(Some(nm), source) == Some(name)
                        || Self::subtree_contains_identifier(nm, source, name))
                {
                    return true;
                }
                false
            }
            _ => {
                for i in 0..node.child_count() {
                    if let Some(c) = node.child(i)
                        && Self::js_node_binds_local_value(c, source, name)
                    {
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Go: is `head` a local VALUE at `byte`? A local variable (short var
    /// declaration, var declaration, a range target, a function parameter)
    /// is never a package. A name bound by an import (or a name the file
    /// declares nowhere) stays the 011-06 upgrade — Go's `pkg.Fn` selector
    /// is a package-qualified reference unless the receiver is a declared
    /// value.
    fn go_head_is_local_value(source: &str, byte: usize, head: &str) -> bool {
        let Some(tree) = Self::parse_for(LanguageId::Go, source) else {
            return false;
        };
        let root = tree.root_node();
        if !(root.start_byte() <= byte && byte < root.end_byte()) {
            return false;
        }
        let bytes = source.as_bytes();
        // The innermost node (to start the ancestor walk).
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
        let mut anc = leaf.parent();
        // The enclosing function chain: named functions AND closures
        // (a closure's parameters bind at every point in its body, and
        // its short-var locals bind in its own body — both read from
        // this ancestor, never from the outer walk's pruned view).
        // P2-4, gate: the grammar's kind is `func_literal` (tree-sitter-
        // go 0.25 node-types), not `function_literal` — the lane's kind
        // never matched, so a closure's params/locals were never local
        // AND the closure prune below never fired.
        let mut funcs: Vec<tree_sitter::Node> = Vec::new();
        while let Some(a) = anc {
            if a.kind() == "function_declaration" || a.kind() == "func_literal" {
                funcs.push(a);
            }
            anc = a.parent();
        }
        let head = head.to_string();
        for f in funcs.iter().rev() {
            // Parameters first (they bind at every point in the body).
            if f
                .child_by_field_name("parameters")
                .is_some_and(|p| Self::go_subtree_binds(p, bytes, &head))
            {
                return true;
            }
            if f
                .child_by_field_name("body")
                .is_some_and(|b| Self::go_subtree_binds(b, bytes, &head))
            {
                return true;
            }
        }
        Self::go_subtree_binds(root, bytes, &head)
    }

    /// Does `node`'s subtree declare `name` as a local Go VALUE (short var,
    /// var declaration, a range target)? Function literals (closures) are
    /// PRUNED — their locals are their own scope.
    fn go_subtree_binds(node: tree_sitter::Node, source: &[u8], name: &str) -> bool {
        if node.kind() == "func_literal" {
            return false; // closure scope — prune (P2-4: the real grammar kind)
        }
        match node.kind() {
            // Go parameters: a single name or a composite one
            // (`(a, b int)` — repeated `name` fields). Only the NAME
            // fields count (the type — a plain identifier like `int`
            // — must not match).
            "parameter_declaration" | "variadic_parameter_declaration" => {
                (0..node.child_count() as u32)
                    .filter_map(|i| node.child(i as usize).map(|c| (i, c)))
                    .filter(|(i, c)| {
                        c.kind() == "identifier"
                            && node.field_name_for_child(*i) == Some("name")
                    })
                    .any(|(_, c)| Self::receiver_node_text(Some(c), source) == Some(name))
            }
            "short_var_declaration" => node
                .child_by_field_name("left")
                .is_some_and(|l| Self::go_identifier_list_binds(Some(l), source, name)),
            "range_clause" => {
                // `for k, v := range …` / `for i := range …`
                node.child_by_field_name("left")
                    .is_some_and(|l| Self::go_identifier_list_binds(Some(l), source, name))
            }
            "var_spec" => Self::go_identifier_list_binds(node.child_by_field_name("name"), source, name),
            _ => {
                for i in 0..node.child_count() {
                    if let Some(c) = node.child(i)
                        && Self::go_subtree_binds(c, source, name)
                    {
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Does an `expression_list` / identifier LHS carry exactly `name`
    /// (a `k, v := …` target, a `var x, y …` declaration)?
    fn go_identifier_list_binds(node: Option<tree_sitter::Node>, source: &[u8], name: &str) -> bool {
        let Some(n) = node else { return false };
        match n.kind() {
            "identifier" => Self::receiver_node_text(Some(n), source) == Some(name),
            "expression_list" => (0..n.child_count())
                .filter_map(|i| n.child(i))
                .any(|c| c.kind() == "identifier" && Self::receiver_node_text(Some(c), source) == Some(name)),
            _ => false,
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
    pub fn start_symbol_resolution(&mut self, symbol: &str, from_file: &str, crate_root: Option<&PathBuf>) {
        let Some(project) = self.project.as_ref() else {
            self.minibuffer_message("no project");
            return;
        };
        let root = project.root.clone();
        let symbol_owned = symbol.to_string();
        // issue-non-rust-receiver-resolution: a pending fetch prompt
        // belongs to the request this one SUPERSEDES — decline it (the
        // superseded provider's blocked hook unblocks and bails; its
        // event is discarded by the generation mismatch). The new
        // request's own ask (if any) re-arms the prompt fresh.
        self.decline_stale_fetch_confirm();
        self.resolve_generation += 1;
        let generation = self.resolve_generation;
        // (jump-ambiguity) the async origin: capture the jump origin NOW,
        // when the keypress started the request — the tooling path is
        // asynchronous (006-02b), so by the time the resolve lands the
        // point may have moved; a picker recording its origin at open
        // time would make `M-,` return to the wrong place. The pending
        // tooling row is superseded too (this request replaces it), and
        // `crate_root` is the picker's index keying for the landing
        // (None — the project index — for the project path, the origin
        // crate's root for M-. inside an external buffer, 006-03).
        self.xref_tooling_origin = self.current_jump_entry();
        self.xref_tooling_pending = None;
        self.xref_tooling_crate_root = crate_root.cloned();
        self.resolving = Some((format!("resolving `{symbol}`…"), generation));
        if tokio::runtime::Handle::try_current().is_err() {
            self.resolving = None;
            self.minibuffer_message(&format!("no provider resolution for `{symbol}` (no background runtime)"));
            return;
        }
        // P3-4, gate: the request is live from here until its event is
        // applied (the fetch-ask liveness gate — a stale event may clear
        // the `resolving` INDICATOR while this request is still running,
        // so the indicator is not the liveness signal).
        self.resolve_in_flight = true;
        let bus = self.resolve_bus.clone();
        let from = std::path::PathBuf::from(from_file);
        // 007-03: the scope hint (use-declaration path for a bare symbol,
        // the enclosing item chain for a path-shaped one, empty otherwise).
        let scope = self.resolver_scope(symbol);
        // 011-01: the buffer's language (the dispatch key — only providers
        // whose `languages()` contain it are attempted; `None` for an
        // unknown extension keeps the pre-dispatch in-order walk).
        let language = self.resolution_language(from_file);
        // issue-non-rust-receiver-resolution: the fetch-confirmation
        // hook (the providers' `confirm_fetch`): a blocked ask travels
        // the store's fetch-confirm bus — the UI drain arms the `y`/`n`
        // banner (the EXACT command on screen), the keypress routes the
        // reply. With NO receiver for the bus (headless tests, the
        // static render path — the channel is closed), the ask cannot
        // be DELIVERED at all: the send fails, the hook returns `false`
        // immediately, and the provider's gate refuses — no install
        // ever runs unconfirmed (nothing actively declines; the failed
        // delivery IS the signal). The cargo provider's `cargo fetch`
        // is the one pre-gate exception (it never calls the hook —
        // named in the crate docs, lib.rs).
        let confirm_tx = self.fetch_confirm_tx.clone();
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
                confirm_fetch: Some(
                    std::sync::Arc::new(move |req: &redline_resolve::FetchRequest| {
                        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
                        let delivered = confirm_tx.send(crate::app::store::FetchConfirmAsk {
                            generation,
                            command: req.command.clone(),
                            from_file: req.from_file.to_string_lossy().into_owned(),
                            reply: reply_tx,
                        })
                        .is_ok();
                        // No receiver (closed channel — headless / static
                        // render path): the ask is undeliverable — return
                        // `false` at once (the provider's gate refuses;
                        // nothing actively declines, the failed delivery
                        // is the signal). Otherwise block on the
                        // operator's `y`/`n` (or the supersede's decline).
                        delivered && reply_rx.recv().unwrap_or(false)
                    })
                        as std::sync::Arc<
                            dyn Fn(&redline_resolve::FetchRequest) -> bool + Send + Sync,
                        >,
                ),
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
    /// `apply_index_event`); otherwise clears the status activity and
    /// either joins a resolve HIT to the Xref picker (jump-ambiguity)
    /// or reports the miss.
    pub fn apply_resolve_event(&mut self, event: &ResolveEvent) {
        if event.generation != self.resolve_generation {
            // A stale event (a superseded request or a previous project):
            // its result is discarded (it must never open a picker, jump,
            // or report — 006-02b). The drain is latest-wins — this stale
            // send may have OVERWRITTEN the current generation's event in
            // the watch channel (006-02b item 3); if so the current job's
            // event never reaches us and its `resolving` indicator would
            // stick until the next action. Clearing it here is always
            // safe: a still-in-flight current-generation event lands its
            // picker when it arrives (its generation still matches) — at
            // worst the indicator hides a few moments early. The
            // `resolve_in_flight` liveness flag is NOT cleared here
            // (P3-4, gate): this event says nothing about the CURRENT
            // request — clearing it here would make the current
            // generation's fetch ask decline without a banner.
            self.resolving = None;
            return;
        }
        self.resolving = None;
        // P3-4, gate: the current request is done (its event was
        // applied) — its provider will no longer ask.
        self.resolve_in_flight = false;
        match (&event.source, &event.error) {
            (Some(source), _) => {
                // jump-ambiguity: the hit joins the Xref picker (marked
                // `tooling`, preselected) instead of jumping silently.
                self.xref_tooling_resolve_to_picker(source, &event.symbol)
            }
            (None, Some(e)) => {
                self.minibuffer_message(&format!(
                    "no provider resolution for `{}`: {}",
                    event.symbol, e
                ));
            }
            _ => {}
        }
    }

    /// issue-non-rust-receiver-resolution: the fetch-on-demand
    /// confirmation. Take the drain's receiver out of the store (exactly
    /// once — Root's `use_future` drain takes it; a second take returns
    /// `None`, e.g. on the static render path or after a test's take).
    /// Mirrors `search_rx()`.
    pub fn fetch_confirm_rx(&mut self) -> Option<mpsc::UnboundedReceiver<crate::app::store::FetchConfirmAsk>> {
        self.fetch_confirm_rx.take()
    }

    /// issue-non-rust-receiver-resolution: a fetch-confirmation ask
    /// arrived from the in-flight resolve's provider (the drain applied
    /// it). Only an ask for the request STILL LIVE (its generation
    /// matches `resolve_generation` AND that generation's request is
    /// still in flight — `resolve_in_flight`, P3-4, gate: the cleared
    /// `resolving` indicator is NOT the liveness signal, a stale event
    /// may have hidden it while the request runs — and no prompt is
    /// already up) gets the banner and the `y`/`n` key routing; a stale
    /// ask (the request was superseded mid-ask, or the resolve already
    /// finished without a fetch) is declined immediately — that
    /// unblocks the superseded provider's blocked hook (its event is
    /// then discarded by the generation mismatch; the install never
    /// runs unconfirmed).
    pub fn apply_fetch_prompt(&mut self, ask: crate::app::store::FetchConfirmAsk) {
        let live = ask.generation == self.resolve_generation && self.resolve_in_flight;
        if live && self.fetch_confirm.is_none() {
            self.fetch_confirm = Some(ask);
            let a = self.fetch_confirm.as_ref().unwrap();
            self.minibuffer_message(&format!(
                "fetch on demand: {} (from {}) (y/n)?",
                a.command, a.from_file
            ));
        } else {
            let _ = ask.reply.send(false);
        }
    }

    /// Whether a fetch-confirmation prompt is awaiting a `y`/`n`
    /// (issue-non-rust-receiver-resolution).
    pub fn fetch_confirm_active(&self) -> bool {
        self.fetch_confirm.is_some()
    }

    /// The fetch-confirmation state machine (issue-
    /// non-rust-receiver-resolution; same shape as the reload / quit /
    /// toggle-read-only confirms): `y` approves the EXACT command on
    /// screen (the provider's hook unblocks and runs the install step);
    /// `n`, C-g and ESC decline it (the provider bails with a refusal
    /// error naming the command that was NOT run). Every other key is
    /// swallowed (the provider stays blocked on the prompt until an
    /// answer — no install ever runs unconfirmed).
    pub fn fetch_confirm_key(&mut self, key: Key) {
        if key == Key::ctrl_char('g') || key.code == KeyCode::Escape {
            self.fetch_confirm_decline();
            return;
        }
        let Some(c) = key.char_value() else { return };
        match c {
            'y' => self.fetch_confirm_accept(),
            'n' => self.fetch_confirm_decline(),
            _ => {}
        }
    }

    /// The confirm's `y`: approve — the provider's blocked hook returns
    /// `true` and runs the install step (the resolve event then reports
    /// the outcome on the input path as usual).
    fn fetch_confirm_accept(&mut self) {
        let Some(ask) = self.fetch_confirm.take() else { return };
        self.minibuffer_message(&format!("fetching ({}): {}", ask.generation, ask.command));
        let _ = ask.reply.send(true);
    }

    /// The confirm's `n` / C-g / ESC: decline — the provider bails with
    /// the refusal error (the install never runs).
    fn fetch_confirm_decline(&mut self) {
        let Some(ask) = self.fetch_confirm.take() else { return };
        self.minibuffer_message(&format!("fetch declined: {}", ask.command));
        let _ = ask.reply.send(false);
    }

    /// issue-non-rust-receiver-resolution: a superseded fetch prompt
    /// (the in-flight request changed while the operator had not yet
    /// answered) is DECLINED — the superseded provider's blocked hook
    /// unblocks and bails; the new request's own ask (if any) re-arms
    /// the prompt fresh. Called from the supersede points (`start_
    /// symbol_resolution` bumps the generation).
    pub fn decline_stale_fetch_confirm(&mut self) {
        if let Some(ask) = self.fetch_confirm.take() {
            let _ = ask.reply.send(false);
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

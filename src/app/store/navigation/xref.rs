use super::*;
use super::definitions::xref_location_candidate;

impl AppStore {
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
    pub(super) fn xref_in_external_buffer(&mut self, key: &str, path: &Path) {
        // Supersede any in-flight tooling resolve (006-02b item 2: a hit
        // must not be clobbered by a stale event of a superseded
        // request); the fall-through path re-bumps in
        // `start_symbol_resolution`.
        self.resolve_generation += 1;
        // P3-4: the superseded request is no longer the current
        // generation's — its in-flight status does not carry over.
        self.resolve_in_flight = false;
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
        // issue-non-rust-receiver-resolution: the receiver
        // classification reads the FULL file (lazily — the point's line
        // index within it), so the extraction takes the buffer's ROPE;
        // only a fetch-capable language's genuine dotted press ever
        // materializes it (P2-5, gate: no whole-clone per M-.).
        let at = match self.buffers.get(key) {
            Some(buf) => Self::symbol_at_point(lang, &line_text, line, self.point_col(), &buf.rope),
            None => None,
        };
        let Some((root, outcome, jump_name)) =
            self.crate_xref_outcome(key, path, line, lang, at.as_ref())
        else {
            // No crate index for this root yet (build in flight, refused,
            // or no runtime): the miss behaves as the project's — the
            // resolver fall-through (or "no symbol under point").
            match &at {
                // 006-03b item 2: a root-relative `from_file` (never the
                // absolute path). The origin project's metadata keys the
                // picker too (jump-ambiguity: no crate index here, so no
                // crate keying either — the origin project's candidates).
                Some((_, path_token)) => {
                    self.start_symbol_resolution(path_token, &self.resolver_from_file(path), None)
                }
                None => self.minibuffer_message("no symbol under point"),
            }
            return;
        };
        match outcome {
            ExternalXrefOutcome::Jump { file, line } => {
                let origin = self.current_jump_entry();
                let abs = root.join(&file);
                // (jump-column-landings) land on the definition's name
                // column, not the line start: the crate index recorded the
                // name's start_byte for this (file, line); re-read it. The
                // outcome enum carries only a line — the definition NAME
                // travels alongside it in `crate_xref_outcome`'s return
                // (`jump_name`), so the re-read matches by (name, line)
                // rather than line alone (P2-2 — a line can host two
                // symbols); a mod.rs change to the outcome type is still
                // the deferred follow-up. `None` (a stale index) degrades
                // honestly to col 0.
                let start_byte = self.definition_start_byte(
                    Some(root.as_path()),
                    &file,
                    line,
                    jump_name.as_deref().unwrap_or(""),
                );
                if self.open_external_path(&abs).is_some() {
                    let col = self.landing_column_from_start_byte(start_byte);
                    self.set_point(line, col, col);
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
                // (jump-ambiguity) the tooling row belongs to the tooling
                // picker only; a new Xref picker carries index rows.
                self.xref_tooling_pending = None;
                // Compute the current-file key against the crate root
                // BEFORE moving `root` into `xref_crate_root` (a `&mut
                // self` call can't take the same field's reference as an
                // argument).
                let cur = self.xref_current_file_rel(Some(&root));
                self.xref_crate_root = Some(root);
                self.xref_lookup_name = lookup;
                let candidates: Vec<PickerCandidate> = defs
                    .iter()
                    .map(|d| xref_location_candidate(d, cur.as_deref()))
                    .collect();
                self.open_picker(PickerKind::Xref, "Definition: ", candidates);
            }
            ExternalXrefOutcome::Resolver(token) => {
                // 006-03b item 2: a root-relative `from_file` (never the
                // absolute path).
                // (jump-ambiguity) the tooling picker's index keying
                // follows the origin: the OWNING crate's index (006-03),
                // so the landing's rows are crate candidates, not the
                // origin project's.
                self.start_symbol_resolution(&token, &self.resolver_from_file(path), Some(&root))
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
    ) -> Option<(PathBuf, ExternalXrefOutcome, Option<String>)> {
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
        // (jump-column-landings P2-2) the silent-jump's definition NAME, so
        // the landing's outline re-read can match by (name, line) rather
        // than line alone (a line can host two symbols). `None` for every
        // non-Jump outcome (those never do a silent same-file landing).
        let mut jump_name: Option<String> = None;
        let outcome = if !defs.is_empty() {
            let lookup = defs[0].symbol.name.clone();
            // (jump-ambiguity) the tightened rule, the same as the
            // project path: silent jump IFF exactly one candidate AND it
            // is in the current (crate-relative) file; a cross-file
            // unique candidate goes to the picker, best preselected.
            if defs.len() == 1 && defs[0].file == rel {
                jump_name = Some(defs[0].symbol.name.clone());
                ExternalXrefOutcome::Jump {
                    file: defs[0].file.clone(),
                    line: defs[0].symbol.line,
                }
            } else {
                ExternalXrefOutcome::Picker { lookup, defs }
            }
        } else {
            // (3) Enclosing-symbol fallback (unchanged: by line, not by
            // the point's column). Same-file-first order so the picker's
            // preselected row (index 0) is the best guess — the fallback's
            // defs come out of `definitions_of` unsorted.
            let outline = idx.outline(&rel).to_vec();
            match crate::nav::index::enclosing_symbol(&outline, line) {
                Some(sym) => {
                    let lookup = sym.name.clone();
                    let mut defs = idx.definitions_of(&lookup);
                    defs.sort_by(|a, b| {
                        (a.file != rel).cmp(&(b.file != rel)).then_with(|| {
                            (a.file.as_str(), a.symbol.line, &a.symbol.name)
                                .cmp(&(b.file.as_str(), b.symbol.line, &b.symbol.name))
                        })
                    });
                    if defs.is_empty() {
                        // (4) The point's own (path-shaped) token is what
                        // the resolver gets — not the enclosing name.
                        match at {
                            Some((_, token)) => {
                                ExternalXrefOutcome::Resolver(token.clone())
                            }
                            None => ExternalXrefOutcome::NoDefinition(lookup),
                        }
                    } else {
                        // (jump-ambiguity) the enclosing fallback is a
                        // by-LINE guess: it is NEVER a silent jump —
                        // even the same-file unique case goes to the
                        // picker, best preselected.
                        ExternalXrefOutcome::Picker { lookup, defs }
                    }
                }
                None => match at {
                    Some((_, token)) => ExternalXrefOutcome::Resolver(token.clone()),
                    None => ExternalXrefOutcome::NoSymbol,
                },
            }
        };
        Some((root, outcome, jump_name))
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
    /// unit-tested dotted handling is reachable from M-.. When `node_at`
    /// returns `None` (no tree / a shape it does not cover / the identifier
    /// is not a full dot-delimited segment of the container, e.g. a
    /// computed member `a[b]`) the exact current bare extraction stands —
    /// never guess.
    ///
    /// issue-non-rust-receiver-resolution: for the languages whose
    /// providers fetch (`python` / `javascript` / `typescript` / `tsx` /
    /// `go`), the WHOLE-PATH upgrade additionally requires that the path's
    /// HEAD is NOT a local value at the point — a receiver the file
    /// declares as a variable (`df` from `df = read_data()`, `this` in
    /// JS/TS, `x` from `x := …` in Go) must stay BARE (the in-project
    /// lookup is the honest answer); only a module/package head (an import
    /// binding, or a name the file declares nowhere) upgrades. The
    /// classification walks the FULL file (materialized LAZILY inside
    /// `dotted_head_is_local_value`, P2-5, gate) with the language's own
    /// scope machinery — the same shape as Rust's receiver pre-steps,
    /// which resolve `x.<member>` through the binding before anything
    /// else. Languages without a fetching provider (C, Cpp, Java, C#,
    /// Toml, Ruby, …) keep the byte-for-byte 011-06 upgrade (their
    /// resolvers have no install to confirm). Rust is untouched in every
    /// arm (byte-for-byte).
    pub(in crate::app::store) fn symbol_at_point(
        lang: LanguageId,
        text: &str,
        line: usize,
        col: usize,
        rope: &Rope,
    ) -> Option<(String, String)> {
        let chars: Vec<char> = text.chars().collect();
        if col > chars.len() {
            return None;
        }
        // issue-language-aware-symbols: word-constituent is the PER-LANGUAGE
        // rule (`redline_syntax::language` table) for this buffer's language.
        let is_ident = |c: char| is_word_char(lang, c);
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
        // issue-language-aware-symbols (Part 1 + 2 seam): the lisp family's
        // symbol run already carries the whole qualified symbol (the
        // `WordRule::Lisp` alphabet makes `/`, `-`, `.` constituents — no
        // `::`/`.` extension above can split it), and the keyword marker is
        // SPELLING, not name: strip the leading `:` (and the auto-resolving
        // `::`), and the identifier is the LAST `/` segment (the var) —
        // `::jwks/local` → `local`, `jwks/fetch-issuer-info` →
        // `fetch-issuer-info`. The path token keeps the WHOLE stripped run
        // (`jwks/fetch-issuer-info`) so the namespace alias stays visible to
        // the M-. Clojure pre-step (Part 2). A bare symbol (no `/`) keeps
        // identifier == path token; a `:`/`::` marker with no name keeps the
        // raw run (never an empty name).
        let (identifier, path_token) = if matches!(
            lang,
            LanguageId::Clojure | LanguageId::Scheme
        ) {
            let raw: String = chars[start..end].iter().collect();
            let stripped = raw.trim_start_matches(':');
            if stripped.is_empty() {
                (raw.clone(), raw)
            } else {
                let ident = stripped.rsplit('/').next().unwrap_or(stripped).to_string();
                (ident, stripped.to_string())
            }
        } else {
            (identifier, path_token)
        };
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
        // The container check runs on the POINT'S LINE with a LINE-LOCAL
        // byte offset (the 011-06 base shape — CRLF-correct by
        // construction: the line text never carries its terminator, and
        // no full-source offset is involved here). P2-5, gate: the lane
        // parsed the WHOLE file per M-. (462 ms on a 688 KB / 40k-line
        // file vs 0.027 ms for the line) — the full source is now
        // materialized ONLY inside `dotted_head_is_local_value` (a
        // fetch-capable language on a genuine dotted press). Any miss
        // keeps the bare extraction byte-for-byte (the `::` scan above
        // is the Rust shape; `.` never extends it).
        // issue-non-rust-receiver-resolution: for the fetch-capable
        // languages the upgrade is additionally gated on the HEAD not
        // being a local value (a variable, never a package — see
        // `dotted_head_is_local_value`).
        let path_token = if lang != LanguageId::Rust
            && let Some(byte) = text.char_indices().nth(end - 1).map(|(b, _)| b)
            && let Some(info) = redline_syntax::node::node_at(lang, text, byte)
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
            // issue-language-aware-symbols: the segment check uses the
            // language's word rule MINUS Ruby's method-name suffixes
            // (`?` / `!`) — a suffix-carrying segment (`x.empty?`) marks a
            // receiver CALL, not a path: the bare-`empty?` extraction
            // stands (the pre-change behavior). TOML's `-` (a bare-key
            // constituent) and the JS/TS `$` (a name char) stay segments.
            && info
                .text
                .split('.')
                .all(|seg| {
                    !seg.is_empty()
                    && seg.chars().all(|c| {
                        is_ident(c)
                            && !(matches!(lang, LanguageId::Ruby)
                                && matches!(c, '?' | '!'))
                    })
                })
            && !Self::dotted_head_is_local_value(
                lang,
                rope,
                line,
                end - 1,
                info.text.split('.').next().unwrap_or(""),
            )
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
    pub(in crate::app::store) fn current_buffer_outline(&mut self) -> Vec<crate::nav::index::Symbol> {
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
    pub(in crate::app::store) fn current_buffer_rust_tables(&mut self) -> Option<redline_syntax::queries::RustTables> {
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
        tables: Option<redline_syntax::queries::RustTables>,
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
                ann_col: None,
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

use super::*;

impl AppStore {
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
    pub(super) fn use_path_for_symbol(source: &str, byte: usize, symbol: &str) -> Option<Vec<String>> {
        let language = redline_syntax::queries::language_for(
            redline_syntax::registry::LanguageId::Rust,
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
    pub(super) fn js_ts_scope_for(
        lang: redline_syntax::registry::LanguageId,
        source: &str,
        byte: usize,
        symbol: &str,
    ) -> Vec<String> {
        // One parse per miss (the 007-03 discipline): a parse failure or an
        // out-of-range offset degrades to the empty hint.
        let Some(language) = redline_syntax::queries::language_for(lang) else {
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
    pub(in crate::app::store) fn python_scope_for(source: &str, byte: usize, symbol: &str) -> Vec<String> {
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
        let language = redline_syntax::queries::language_for(
            redline_syntax::registry::LanguageId::Python,
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
    pub(in crate::app::store) fn go_scope_for(source: &str, byte: usize, symbol: &str) -> Vec<String> {
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
        let language = redline_syntax::queries::language_for(
            redline_syntax::registry::LanguageId::Go,
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
    pub(in crate::app::store) fn resolver_from_file(&self, path: &Path) -> String {
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
}

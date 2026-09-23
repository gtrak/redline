//! Node-at-point + enclosing scope: for a raw-source byte offset, the
//! identifier-ish syntax node there and the enclosing item chain.
//!
//! Plain Rust — zero iocraft/tokio (plan layering rule). Consumed later by
//! 007-02 (M-. extraction, annotation anchoring) and 007-03 (resolver
//! `scope_path`); this module is a tested, callable API with no app wiring.
//!
//! One fresh parse per call (issue 007-04 adds tree reuse / incremental
//! reparse; a parse of an editor-sized buffer is acceptable today).
//!
//! `#![allow(dead_code)]` — this module ships an unwired public API (007-02
//! / 007-03 consume it; no in-tree caller yet), so the linter's
//! never-used diagnostics are expected until then.
#![allow(dead_code)]
//!
//! Split into submodules (012 A2): path-segment rule in
//! [`paths`], scope walkers + `scope_path_for` in [`scopes`].

use tree_sitter::{Node, Parser};

use crate::registry::LanguageId;

mod paths;
mod scopes;

pub(crate) use paths::is_path_segment;
pub(crate) use scopes::scope_path_for;

/// The syntax node at a byte offset, plus its enclosing scope chain.
#[derive(Clone, Debug)]
pub struct NodeInfo {
    /// The node's source text (e.g. the full `tokio::spawn`, not a segment).
    pub text: String,
    /// The tree-sitter node kind (e.g. `scoped_identifier`).
    pub kind: String,
    /// Byte offset of the node's first byte in the raw source.
    pub start_byte: usize,
    /// Byte offset just past the node's last byte in the raw source.
    pub end_byte: usize,
    /// Enclosing items, outermost to innermost (e.g. `["Foo", "bar"]`).
    /// Empty for top-level code.
    pub scope_path: Vec<String>,
}

/// The identifier-ish syntax node at a byte offset, with its enclosing
/// scope path.
///
/// `byte` is a **byte offset into the raw source string** (not a char
/// offset) in `[0, source.len())`; callers pass a rope slice. The answer is
/// the smallest identifier-ish node containing `byte` — a whole dotted path
/// (`tokio::spawn`, `a.b.c`, `pkg.Fn`) comes back as ONE node regardless
/// of which segment the offset sits on. Offsets at `source.len()` or past
/// EOF return `None` (nothing contains them); an offset at byte 0 is a
/// normal containment check (a keyword at a file start is not
/// identifier-ish, so it yields `None`).
///
/// Rust, JavaScript, TypeScript/TSX, Python, Go, C, C++, Bash, TOML,
/// JSON, Markdown, and Java are implemented (each independently
/// degraded — see the per-language notes in this module); every other
/// `LanguageId` (currently Yaml — intentionally unadopted for node-at,
/// and `Plain`) returns `None` (the plan's "Rust first, graceful
/// degradation" decision — callers degrade to today's behavior).
pub fn node_at(lang: LanguageId, source: &str, byte: usize) -> Option<NodeInfo> {
    let tree = parse_source(lang, source)?;
    let leaf = innermost_at(tree.root_node(), byte)?;
    let node = nearest_identifier(leaf, lang)?;
    let source_bytes = source.as_bytes();
    Some(NodeInfo {
        text: node.utf8_text(source_bytes).ok()?.to_string(),
        kind: node.kind().to_string(),
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        scope_path: scope_path_for(lang, leaf, source_bytes),
    })
}

/// The enclosing item chain (outermost → innermost) at a byte offset —
/// available at offsets where `node_at` returns `None` (e.g. on a keyword
/// or with no identifier-ish ancestor), which is why it is a separate
/// surface rather than reachable only via [`NodeInfo::scope_path`].
/// Empty for top-level code, unsupported languages, and out-of-range
/// offsets.
///
/// `byte` is a byte offset into the raw source string (not a char offset).
pub fn scope_path_at(lang: LanguageId, source: &str, byte: usize) -> Vec<String> {
    let tree = match parse_source(lang, source) {
        Some(t) => t,
        None => return Vec::new(),
    };
    let leaf = match innermost_at(tree.root_node(), byte) {
        Some(n) => n,
        None => return Vec::new(),
    };
    scope_path_for(lang, leaf, source.as_bytes())
}

/// Parse `source` for `lang`: every grammar-bearing language in the
/// descriptor table parses (the old hand-synced 17-language whitelist —
/// a fourth identity list that could silently disable M-. for a new
/// language with zero compile error — is gone; `language::parseable`
/// derives the set from the table's grammar column). `Plain` (no
/// grammar) degrades to `None`; `Yaml` now parses here too, but its row
/// carries no identifier kinds and no scope walker, so `node_at` /
/// `scope_path_at` still degrade to `None` / `[]` exactly as before.
fn parse_source(lang: LanguageId, source: &str) -> Option<tree_sitter::Tree> {
    if !crate::language::parseable(lang) {
        return None;
    }
    // The grammar itself comes from the shared `queries::language_for`
    // wrapper over the descriptor table so the tree-sitter grammar
    // versions live in exactly one place (no second registry, no
    // duplicated pin); `Plain` never reaches the grammar lookup.
    let language = crate::queries::language_for(lang)?;
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    parser.parse(source.as_bytes(), None)
}

/// The innermost node whose byte range contains `byte` (a leaf token, or
/// `None` when `byte` is outside the root's range — i.e. at `source.len()`
/// or past EOF).
fn innermost_at(root: Node, byte: usize) -> Option<Node> {
    if !(root.start_byte() <= byte && byte < root.end_byte()) {
        return None;
    }
    let mut node = root;
    loop {
        let mut child = None;
        for i in 0..node.child_count() {
            if let Some(c) = node.child(i)
                && c.start_byte() <= byte
                && byte < c.end_byte()
            {
                child = Some(c);
                break;
            }
        }
        match child {
            Some(c) => node = c,
            None => return Some(node),
        }
    }
}

/// The nearest identifier-ish node containing the leaf: the leaf itself or
/// the first such ancestor. Any part of a `::` path (a segment
/// `identifier`/`type_identifier`/`primitive_type`, a nested
/// `scoped_identifier`/`scoped_type_identifier` `path` field, or the `::`
/// token) is skipped so the whole path comes back as one node — the outer
/// `scoped_identifier` in value position, the outer
/// `scoped_type_identifier` in type position. `None` when no ancestor is
/// identifier-ish. JSON additionally requires the `pair`/`key` position
/// (its identifier kind `string` also appears in value position — see
/// [`in_identifier_position`]).
fn nearest_identifier(leaf: Node, lang: LanguageId) -> Option<Node> {
    let mut cur = leaf;
    loop {
        if !is_path_segment(cur, lang)
            && cur.is_named()
            && is_identifier_kind(lang, cur.kind())
            && in_identifier_position(lang, cur)
        {
            return Some(cur);
        }
        cur = cur.parent()?;
    }
}

/// Whether `node` may count as an identifier at its position: true for
/// every language except JSON, where the `string` kind is both the key
/// kind and the value-string kind — only a `pair`'s `key` field is
/// identifier-ish (a value string is data, not a navigable name) — and
/// RUBY, where a `call` is identifier-ish only in member-access shape
/// (a `call` with a `receiver` field and NO `arguments` field — `a.b`
/// parses as a receiver-carrying argumentless call, while a bare call
/// with arguments, `puts x`, must not become a path; probe-verified
/// against the pinned tree-sitter-ruby 0.23.1: a bare `foo` with no
/// arguments parses as a plain `identifier`, not a `call`).
fn in_identifier_position(lang: LanguageId, node: Node) -> bool {
    match lang {
        LanguageId::Ruby if node.kind() == "call" => {
            node.child_by_field_name("receiver").is_some()
                && node.child_by_field_name("arguments").is_none()
        }
        LanguageId::Json => node.parent().is_some_and(|p| {
            p.kind() == "pair" && p.child_by_field_name("key") == Some(node)
        }),
        _ => true,
    }
}

/// Whether `kind` is an identifier-ish node kind for `lang` — the
/// per-language kind LISTS are DATA and live in the descriptor table
/// (`language.rs` rows' `identifier_kinds` columns, with the probe
/// rationales on each row); the per-language ALGORITHMS that stay here
/// are the position gate (`in_identifier_position`), the path-segment
/// rules (`is_path_segment`), and the scope walkers. (The old 15
/// hand-synced `is_*_identifier_kind` predicates are gone — an empty
/// list is the honest N/A, the same as the old default arm's
/// `false`.)
fn is_identifier_kind(lang: LanguageId, kind: &str) -> bool {
    crate::language::spec(lang).identifier_kinds.contains(&kind)
}

// ── issue-annotations-symbol-identity: the scope-aware identity ──────────
//
// The annotation re-anchor (the store's `reanchor_for_key`) keys an
// occurrence by MORE than the (kind, name) pair, because a name repeats at
// its definition and at every call site. What the STORE keys on is the
// **(kind, name, enclosing-scope)** triple: a group is usable only when it
// has exactly one member, so a name repeating across scopes is
// disambiguated, and a name repeating WITHIN one scope stays ambiguous
// and orphans rather than guessing.
//
// `SymbolIdentity` also carries an ORDINAL within the scope (a node's scope
// is computed from the node itself in both places, and the ordinal is the
// number of same-key nodes before it). **The ordinal has no production
// consumer** — it exists for callers and for the proposed stage-2b, which
// would key on it *only together with* a content/stability validation that
// forces an orphan when a shift changes which occurrence it names. Keying on
// the raw ordinal is unsafe: a check on the `unit_flow_ann_orphan` fixture
// showed it migrating a note to a sibling line whose text no longer matches
// the anchor when an earlier same-scope occurrence is deleted.
//
// `symbol_identity_at` reads the identity of the symbol under a byte offset;
// `build_annotation_symbol_index` reads every occurrence's identity for a
// whole buffer. Both share the same "nearest identifier" + scope walk AND
// the same position gate, so a captured identity and the index's view of the
// same node agree (gate P2: the index previously filtered on kind alone and
// counted nodes the capture could never return).

/// The scope-aware identity of the identifier-ish node at a byte offset:
/// its (kind, name), its ENCLOSING-SCOPE chain (outermost → innermost, from
/// the per-language scope walk), its ORDINAL within that (kind, name, scope)
/// group in document order, and its start byte (for the store to turn into a
/// column). `None` for out-of-range offsets, non-parseable languages, and
/// offsets with no identifier-ish node (a keyword / whitespace / EOL).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolIdentity {
    pub kind: String,
    pub name: String,
    /// The enclosing definition chain, outermost → innermost (the same
    /// answer `NodeInfo::scope_path` reports). Empty for top-level code.
    pub scope: Vec<String>,
    /// The occurrence's position (0-based, document order) among every node
    /// of the same (kind, name, scope). Stable under an insertion above.
    pub ordinal: usize,
    pub start_byte: usize,
}

/// One occurrence in an annotation symbol index: the 0-based line and the
/// symbol's START column (a char offset within that line), in document
/// order (the ordinal is the index into the owning group's vec).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SymbolOccurrence {
    pub line: usize,
    pub col: usize,
}

/// Every identifier-ish node's (kind, name, scope) identity for a whole
/// buffer, built from ONE parse. `by_name` groups by (kind, name) (the
/// legacy, scope-blind uniqueness rule); `by_scope` groups by
/// (kind, name, enclosing-scope) (the scope-aware rule, where an
/// occurrence's ordinal is its index in the vec). Both are in document
/// order. `None` for non-parseable languages or languages with no
/// identifier-ish kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnnotationSymbolIndex {
    pub by_name: std::collections::HashMap<(String, String), Vec<SymbolOccurrence>>,
    pub by_scope:
        std::collections::HashMap<(String, String, Vec<String>), Vec<SymbolOccurrence>>,
}

/// The byte offset of the FIRST character of each line (line `i` starts at
/// `line_starts[i]`), used to turn an absolute byte offset into a column.
fn line_start_bytes(source: &[u8]) -> Vec<usize> {
    let mut out = vec![0usize];
    for (i, &b) in source.iter().enumerate() {
        if b == b'\n' {
            out.push(i + 1);
        }
    }
    out
}

/// The CHARACTER offset of `start_byte` within its line (never panics: an
/// out-of-range line or malformed slice degrades to 0).
fn char_offset_in_line(source: &[u8], line_starts: &[usize], row: usize, start_byte: usize) -> usize {
    let Some(&ls) = line_starts.get(row) else {
        return 0;
    };
    if start_byte < ls {
        return 0;
    }
    match std::str::from_utf8(&source[ls..start_byte]) {
        Ok(s) => s.chars().count(),
        Err(_) => start_byte - ls,
    }
}

/// The scope-aware identity of the symbol at `byte` (see [`SymbolIdentity`]).
pub fn symbol_identity_at(lang: LanguageId, source: &str, byte: usize) -> Option<SymbolIdentity> {
    let tree = parse_source(lang, source)?;
    let root = tree.root_node();
    let leaf = innermost_at(root, byte)?;
    let node = nearest_identifier(leaf, lang)?;
    let bytes = source.as_bytes();
    let kind = node.kind().to_string();
    let name = node.utf8_text(bytes).ok()?.to_string();
    let scope = scope_path_for(lang, node, bytes);
    let target = node.start_byte();
    // The ordinal: the number of SAME-KEY (kind, name, scope) nodes that
    // start earlier in document order. The walk is a plain kind+name+scope
    // filter (no position gate) — deliberately the same set the index
    // counts, so the captured ordinal and the index's view agree.
    let mut earlier = 0usize;
    count_earlier_same_key(root, lang, (&kind, &name), &scope, target, bytes, &mut earlier);
    Some(SymbolIdentity {
        kind,
        name,
        scope,
        ordinal: earlier,
        start_byte: target,
    })
}

/// Count, in document order, the named nodes with the same
/// (kind, name, scope) that start strictly before `target_byte`.
fn count_earlier_same_key(
    node: Node,
    lang: LanguageId,
    key: (&str, &str),
    scope: &[String],
    target_byte: usize,
    bytes: &[u8],
    count: &mut usize,
) {
    if node.is_named()
        && node.kind() == key.0
        && let Ok(text) = node.utf8_text(bytes)
        && text == key.1
        && scope_path_for(lang, node, bytes) == scope
        && node.start_byte() < target_byte
    {
        *count += 1;
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            count_earlier_same_key(child, lang, key, scope, target_byte, bytes, count);
        }
    }
}

/// Build the whole-buffer scope-aware symbol index (see
/// [`AnnotationSymbolIndex`]).
pub fn build_annotation_symbol_index(
    lang: LanguageId,
    source: &str,
) -> Option<AnnotationSymbolIndex> {
    let kinds = crate::language::spec(lang).identifier_kinds;
    if kinds.is_empty() {
        return None;
    }
    let tree = parse_source(lang, source)?;
    let bytes = source.as_bytes();
    let line_starts = line_start_bytes(bytes);
    let mut index = AnnotationSymbolIndex::default();
    collect_annotatable(tree.root_node(), lang, kinds, bytes, &line_starts, &mut index);
    Some(index)
}

/// Walk the tree collecting every NAMED node of the language's
/// identifier-ish kinds into the index's two maps (document order). A node
/// contributes its (kind, text) to `by_name` and (kind, text, enclosing-scope)
/// to `by_scope`.
fn collect_annotatable(
    node: Node,
    lang: LanguageId,
    kinds: &[&str],
    bytes: &[u8],
    line_starts: &[usize],
    index: &mut AnnotationSymbolIndex,
) {
    if node.is_named()
        && kinds.contains(&node.kind())
        && !is_path_segment(node, lang)
        && in_identifier_position(lang, node)
        // gate P2 (017 symbol-identity): mirror `nearest_identifier`'s FULL
        // gate, not just the kind check. Without this the index counts nodes
        // the capture can never return — e.g. a JSON *value* string whose text
        // repeats its own key (`{"key": "key"}`), which made the key's group
        // length 2 and so unresolvable forever, silently degrading that
        // annotation to the text rules. Degradation only, never a wrong tie,
        // but silent. The two gates must agree by construction.
        && let Ok(text) = node.utf8_text(bytes)
    {
        let row = node.start_position().row;
        let col = char_offset_in_line(bytes, line_starts, row, node.start_byte());
        let occ = SymbolOccurrence { line: row, col };
        let kind = node.kind().to_string();
        let name = text.to_string();
        index
            .by_name
            .entry((kind.clone(), name.clone()))
            .or_default()
            .push(occ);
        let scope = scope_path_for(lang, node, bytes);
        index.by_scope.entry((kind, name, scope)).or_default().push(occ);
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            collect_annotatable(child, lang, kinds, bytes, line_starts, index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Discriminating: both segments of the path must return the whole
    /// `scoped_identifier`, not a fragment.
    #[test]
    fn scoped_call_path_comes_back_whole() {
        let src = "fn main() { tokio::spawn(x); }\n";
        let tokio_at = src.find("tokio").expect("fixture has tokio");
        let spawn_at = src.find("spawn").expect("fixture has spawn");

        let first = node_at(LanguageId::Rust, src, tokio_at).expect("node at `tokio`");
        assert_eq!(first.kind, "scoped_identifier");
        assert_eq!(first.text, "tokio::spawn");
        assert_eq!(first.start_byte, tokio_at);
        assert_eq!(first.end_byte, spawn_at + "spawn".len());

        let second = node_at(LanguageId::Rust, src, spawn_at + 1).expect("node at `spawn`");
        assert_eq!(second.kind, "scoped_identifier");
        assert_eq!(second.text, "tokio::spawn");
        assert_eq!(second.start_byte, first.start_byte);
        assert_eq!(second.end_byte, first.end_byte);
    }

    #[test]
    fn field_access_returns_field_identifier() {
        let src = "struct S { foo: u32 }\nimpl S { fn get(&self) -> u32 { self.foo } }\n";
        let pos = src.find("self.foo").expect("fixture") + "self.".len();
        let info = node_at(LanguageId::Rust, src, pos).expect("node at `foo`");
        assert_eq!(info.kind, "field_identifier");
        assert_eq!(info.text, "foo");
        assert_eq!(info.scope_path, ["S", "get"]);
    }

    #[test]
    fn type_position_plain_and_scoped() {
        let src = "fn f() { let a: Vec<String> = 1; let b: std::vec::Vec<u8> = 2; }\n";
        let vec_at = src.find(": Vec<").expect("fixture") + 2;
        let info = node_at(LanguageId::Rust, src, vec_at).expect("node at `Vec`");
        assert_eq!(info.kind, "type_identifier");
        assert_eq!(info.text, "Vec");

        // Inside the scoped form, the whole path comes back.
        let scoped_at = src.find("std::vec::Vec").expect("fixture") + 4;
        let info = node_at(LanguageId::Rust, src, scoped_at).expect("node at `std::vec::Vec`");
        assert_eq!(info.kind, "scoped_type_identifier");
        assert_eq!(info.text, "std::vec::Vec");
    }

    #[test]
    fn scope_path_inside_impl_fn() {
        let src = "struct Foo;\nimpl Foo { fn bar(&self) { let x = 1; } }\n";
        let let_at = src.find("let x").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Rust, src, let_at),
            vec![String::from("Foo"), String::from("bar")]
        );
        // Same path via NodeInfo at the bound variable.
        let info = node_at(LanguageId::Rust, src, let_at + 4).expect("node at `x`");
        assert_eq!(info.scope_path, vec![String::from("Foo"), String::from("bar")]);
    }

    #[test]
    fn scope_path_includes_nested_mods() {
        let src = "mod outer { mod inner { fn f() { let y = 2; } } }\n";
        let pos = src.find("let y").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Rust, src, pos),
            vec![String::from("outer"), String::from("inner"), String::from("f")]
        );
    }

    #[test]
    fn top_level_code_has_empty_scope_path() {
        // A top-level const's name: no enclosing scope items → empty.
        let src = "const Z: i32 = 3;\n";
        let info = node_at(LanguageId::Rust, src, src.find("Z").expect("fixture")).unwrap();
        assert_eq!(info.scope_path, Vec::<String>::new());
        assert_eq!(scope_path_at(LanguageId::Rust, src, 0), Vec::<String>::new());
        // A fn's own name identifier is a child of the `function_item`, so
        // the ancestor walk includes it: the name `g` sees itself ["g"].
        let src2 = "fn g() { let z = 3; }
";
        let info = node_at(LanguageId::Rust, src2, src2.find("z").expect("fixture")).unwrap();
        assert_eq!(info.scope_path, vec![String::from("g")]);
        // ...and code inside `g` sees the same scope (the enclosing fn).
        let at_name = node_at(LanguageId::Rust, src2, src2.find("g").expect("fixture")).unwrap();
        assert_eq!(at_name.scope_path, vec![String::from("g")]);
    }

    /// Generic `impl` blocks must yield the BARE type name in the scope, not
    /// the raw field text (`Vec<T>`) — 007-03's resolver matches on plain
    /// names, so `Foo<T>` would silently never match `Foo::method`.
    /// (007-01 review P2.)
    #[test]
    fn generic_impl_scope_is_the_bare_type_name() {
        let src = "impl<T> Vec<T> {\n    fn push(&mut self) {}\n}\n";
        // The `Vec` in the impl header: the impl's own scope level.
        let at_type = src.find("Vec").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Rust, src, at_type),
            vec![String::from("Vec")]
        );
        // Inside the method: the impl level is the BARE name, not `Vec<T>`.
        let in_body = src.find("&mut self").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Rust, src, in_body),
            vec![String::from("Vec"), String::from("push")]
        );
        // `impl Trait for u8` keeps the primitive name.
        let src2 = "impl Foo for u8 { fn m(&self) {} }\n";
        let byte2 = src2.find("&self").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Rust, src2, byte2),
            vec![String::from("u8"), String::from("m")]
        );
    }

    /// A bodyless trait method (`fn m(&self);`) is a
    /// `function_signature_item` — it must still contribute its name to the
    /// scope of anything declared inside it. (007-01 review P2.)
    #[test]
    fn trait_signature_item_contributes_scope() {
        let src = "trait T {\n    fn m(&self) -> i32;\n}\n";
        let byte = src.find("i32").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Rust, src, byte),
            vec![String::from("T"), String::from("m")]
        );
    }

    #[test]
    fn boundary_offsets_do_not_panic() {
        let src = "fn main() { foo(); }\n";
        // Byte 0 sits on the `fn` keyword (not identifier-ish → no node),
        // but it is inside `fn main`, so the scope is reported.
        assert!(node_at(LanguageId::Rust, src, 0).is_none());
        assert_eq!(scope_path_at(LanguageId::Rust, src, 0), vec![String::from("main")]);
        // An identifier at byte 0 does resolve; top-level → no scope.
        let src2 = "foo\n";
        let info = node_at(LanguageId::Rust, src2, 0).expect("identifier at byte 0");
        assert_eq!(info.text, "foo");
        assert!(info.scope_path.is_empty());
        // At end-of-file and past EOF: nothing contains the offset.
        assert!(node_at(LanguageId::Rust, src, src.len()).is_none());
        assert!(node_at(LanguageId::Rust, src, src.len() + 4096).is_none());
        assert!(scope_path_at(LanguageId::Rust, src, src.len()).is_empty());
        assert!(scope_path_at(LanguageId::Rust, src, usize::MAX).is_empty());
    }

    /// Languages without a node-at implementation still degrade gracefully
    /// (`node_at` → `None`, `scope_path_at` → `[]`) — 007-01's "Rust
    /// first, graceful degradation" decision now covers the languages not
    /// yet adopted (Yaml is intentionally unadopted for node-at; the six
    /// C/Cpp/Bash/Toml/Json/Markdown languages of this issue all landed).
    #[test]
    fn unimplemented_languages_return_none() {
        let cases: [(LanguageId, &str, &str); 2] = [
            (LanguageId::Yaml, "key: value\n", "key"),
            (LanguageId::Plain, "abc", "abc"),
        ];
        for (id, src, marker) in cases {
            let byte = src.find(marker).unwrap_or(0);
            assert!(node_at(id, src, byte).is_none(), "{id:?} node_at");
            assert!(
                scope_path_at(id, src, byte).is_empty(),
                "{id:?} scope_path_at"
            );
        }
        assert!(node_at(LanguageId::Plain, "abc", 1).is_none());
    }

    /// Shared helper for the whole-path (discriminating) assertions: the
    /// node at BOTH segment offsets must be the identical whole-path node.
    fn assert_whole_path(lang: LanguageId, src: &str, first_at: usize, last_at: usize, kind: &str, text: &str) {
        let first = node_at(lang, src, first_at).expect("node at first segment");
        assert_eq!(first.kind, kind, "kind at first segment");
        assert_eq!(first.text, text, "text at first segment");
        assert_eq!(first.start_byte, first_at);
        assert_eq!(first.end_byte, src.find(text).unwrap() + text.len());
        let last = node_at(lang, src, last_at).expect("node at last segment");
        assert_eq!(last.kind, kind, "kind at last segment");
        assert_eq!(last.text, text, "text at last segment");
        assert_eq!(last.start_byte, first.start_byte);
        assert_eq!(last.end_byte, first.end_byte);
    }

    // ── JavaScript (011-03) ─────────────────────────────────────────

    /// Discriminating: `a.b.c` must come back WHOLE (one
    /// `member_expression` node) for an offset on any segment.
    #[test]
    fn js_member_path_comes_back_whole() {
        let src = "function f() { console.log(a.b.c); }\n";
        let a_at = src.find("a.b").expect("fixture");
        let c_at = src.find(".c").expect("fixture") + 1;
        assert_whole_path(
            LanguageId::JavaScript,
            src,
            a_at,
            c_at,
            "member_expression",
            "a.b.c",
        );
    }

    #[test]
    fn js_plain_identifier_top_level() {
        let src = "const x = 1;\n";
        let info = node_at(LanguageId::JavaScript, src, src.find("x").expect("fixture")).unwrap();
        assert_eq!(info.kind, "identifier");
        assert_eq!(info.text, "x");
        assert!(info.scope_path.is_empty());
    }

    #[test]
    fn js_scope_chain_function_class_method_arrow() {
        let src = "function outer() { class Inner { method() { const inner = () => this.ref; } } }\n";
        let at = src.find("ref").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::JavaScript, src, at),
            vec![
                String::from("outer"),
                String::from("Inner"),
                String::from("method"),
                String::from("inner"),
            ]
        );
        // Same chain via NodeInfo (and `this.ref` is one whole path).
        let info = node_at(LanguageId::JavaScript, src, at).expect("node at `ref`");
        assert_eq!(info.kind, "member_expression");
        assert_eq!(info.text, "this.ref");
        assert_eq!(
            info.scope_path,
            vec![
                String::from("outer"),
                String::from("Inner"),
                String::from("method"),
                String::from("inner"),
            ]
        );
    }

    #[test]
    fn js_scope_chain_method_in_class() {
        let src = "class A { m() { let q = 1; } }\n";
        let at = src.find("q").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::JavaScript, src, at),
            vec![String::from("A"), String::from("m")]
        );
    }

    #[test]
    fn js_boundary_offsets_do_not_panic() {
        let src = "function f() { g(); }\n";
        // Byte 0 sits on the `function` keyword (not identifier-ish → no
        // node), but the scope is still reported.
        assert!(node_at(LanguageId::JavaScript, src, 0).is_none());
        assert_eq!(
            scope_path_at(LanguageId::JavaScript, src, 0),
            vec![String::from("f")]
        );
        // An identifier at byte 0 does resolve; top-level → no scope.
        let src2 = "x\n";
        let info = node_at(LanguageId::JavaScript, src2, 0).expect("identifier at byte 0");
        assert_eq!(info.text, "x");
        assert!(info.scope_path.is_empty());
        // At end-of-file and past EOF: nothing contains the offset.
        assert!(node_at(LanguageId::JavaScript, src, src.len()).is_none());
        assert!(node_at(LanguageId::JavaScript, src, src.len() + 4096).is_none());
        assert!(scope_path_at(LanguageId::JavaScript, src, src.len()).is_empty());
        assert!(scope_path_at(LanguageId::JavaScript, src, usize::MAX).is_empty());
    }

    #[test]
    fn js_broken_source_does_not_panic() {
        let src = "function f() { g(";
        let pos = src.find("f").expect("fixture");
        if let Some(info) = node_at(LanguageId::JavaScript, src, pos) {
            assert_eq!(info.text, "f");
        }
        let _ = scope_path_at(LanguageId::JavaScript, src, pos);
    }

    // ── TypeScript / TSX (011-03) ───────────────────────────────────

    /// Discriminating: `a.b.c` must come back WHOLE (one
    /// `member_expression` node) for an offset on any segment — for both
    /// the TS and TSX grammars.
    #[test]
    fn ts_member_path_comes_back_whole() {
        for lang in [LanguageId::TypeScript, LanguageId::Tsx] {
            let src = "function f() { console.log(a.b.c); }\n";
            let a_at = src.find("a.b").expect("fixture");
            let c_at = src.find(".c").expect("fixture") + 1;
            assert_whole_path(lang, src, a_at, c_at, "member_expression", "a.b.c");
        }
    }

    /// Discriminating: a dotted NAME path in type position (`A.B.C`)
    /// comes back whole as a `nested_type_identifier` — TS-only grammar
    /// kind.
    #[test]
    fn ts_nested_type_identifier_comes_back_whole() {
        for lang in [LanguageId::TypeScript, LanguageId::Tsx] {
            let src = "type T = Outer.Nested.Leaf;\n";
            let outer_at = src.find("Outer").expect("fixture");
            let leaf_at = src.find("Leaf").expect("fixture");
            assert_whole_path(
                lang,
                src,
                outer_at,
                leaf_at,
                "nested_type_identifier",
                "Outer.Nested.Leaf",
            );
        }
    }

    #[test]
    fn ts_scope_chain_function_class_method_arrow() {
        let src = "function outer() { abstract class Inner { method() { const inner = () => this.ref; } } }\n";
        let at = src.find("ref").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::TypeScript, src, at),
            vec![
                String::from("outer"),
                String::from("Inner"),
                String::from("method"),
                String::from("inner"),
            ]
        );
        // A plain class + method, for both grammars.
        for lang in [LanguageId::TypeScript, LanguageId::Tsx] {
            let src2 = "class A { m() { let q = 1; } }\n";
            let at2 = src2.find("q").expect("fixture");
            assert_eq!(
                scope_path_at(lang, src2, at2),
                vec![String::from("A"), String::from("m")]
            );
        }
    }

    #[test]
    fn ts_boundary_offsets_do_not_panic() {
        for lang in [LanguageId::TypeScript, LanguageId::Tsx] {
            let src = "function f() { g(); }\n";
            assert!(node_at(lang, src, 0).is_none());
            assert_eq!(scope_path_at(lang, src, 0), vec![String::from("f")]);
            let src2 = "x\n";
            let info = node_at(lang, src2, 0).expect("identifier at byte 0");
            assert_eq!(info.text, "x");
            assert!(info.scope_path.is_empty());
            assert!(node_at(lang, src, src.len()).is_none());
            assert!(node_at(lang, src, src.len() + 4096).is_none());
            assert!(scope_path_at(lang, src, src.len()).is_empty());
            assert!(scope_path_at(lang, src, usize::MAX).is_empty());
        }
    }

    #[test]
    fn ts_broken_source_does_not_panic() {
        let src = "function f() { g(";
        for lang in [LanguageId::TypeScript, LanguageId::Tsx] {
            let pos = src.find("f").expect("fixture");
            if let Some(info) = node_at(lang, src, pos) {
                assert_eq!(info.text, "f");
            }
            let _ = scope_path_at(lang, src, pos);
        }
    }

    // ── Python (011-03) ─────────────────────────────────────────────

    /// Discriminating: `a.b.c` must come back WHOLE (one `attribute`
    /// node) for an offset on any segment.
    #[test]
    fn python_attribute_path_comes_back_whole() {
        let src = "def f():\n    return a.b.c\n";
        let a_at = src.find("a.b").expect("fixture");
        let c_at = src.find(".c").expect("fixture") + 1;
        assert_whole_path(LanguageId::Python, src, a_at, c_at, "attribute", "a.b.c");
    }

    #[test]
    fn python_plain_identifier_top_level() {
        let src = "x = 1\n";
        let info = node_at(LanguageId::Python, src, 0).expect("identifier at byte 0");
        assert_eq!(info.kind, "identifier");
        assert_eq!(info.text, "x");
        assert!(info.scope_path.is_empty());
    }

    #[test]
    fn python_scope_chain_function_in_class() {
        let src = "class Foo:\n    def bar(self):\n        return 1\n";
        let at = src.find("return").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Python, src, at),
            vec![String::from("Foo"), String::from("bar")]
        );
        let info = node_at(LanguageId::Python, src, src.find("bar").expect("fixture")).unwrap();
        assert_eq!(
            info.scope_path,
            vec![String::from("Foo"), String::from("bar")]
        );
    }

    #[test]
    fn python_boundary_offsets_do_not_panic() {
        let src = "def f():\n    return g()\n";
        // Byte 0 sits on the `def` keyword (not identifier-ish → no
        // node), but the scope is still reported.
        assert!(node_at(LanguageId::Python, src, 0).is_none());
        assert_eq!(scope_path_at(LanguageId::Python, src, 0), vec![String::from("f")]);
        let src2 = "x\n";
        let info = node_at(LanguageId::Python, src2, 0).expect("identifier at byte 0");
        assert_eq!(info.text, "x");
        assert!(info.scope_path.is_empty());
        assert!(node_at(LanguageId::Python, src, src.len()).is_none());
        assert!(node_at(LanguageId::Python, src, src.len() + 4096).is_none());
        assert!(scope_path_at(LanguageId::Python, src, src.len()).is_empty());
        assert!(scope_path_at(LanguageId::Python, src, usize::MAX).is_empty());
    }

    #[test]
    fn python_broken_source_does_not_panic() {
        let src = "def f(:\n    g = ";
        let pos = src.find("f").expect("fixture");
        if let Some(info) = node_at(LanguageId::Python, src, pos) {
            assert_eq!(info.text, "f");
        }
        let _ = scope_path_at(LanguageId::Python, src, pos);
    }

    // ── Go (011-03) ─────────────────────────────────────────────────

    /// Discriminating: `p.a.b` must come back WHOLE (one
    /// `selector_expression` node) for an offset on any segment.
    #[test]
    fn go_selector_path_comes_back_whole() {
        let src = "var v = p.a.b\n";
        let p_at = src.find("p.a").expect("fixture");
        let b_at = src.find(".b").expect("fixture") + 1;
        assert_whole_path(LanguageId::Go, src, p_at, b_at, "selector_expression", "p.a.b");
    }

    /// Discriminating: `x.T` in type position comes back whole as a
    /// `qualified_type`.
    #[test]
    fn go_qualified_type_comes_back_whole() {
        let src = "func f() x.T { return 0 }\n";
        let x_at = src.find("x.T").expect("fixture");
        let t_at = src.find(".T").expect("fixture") + 1;
        assert_whole_path(LanguageId::Go, src, x_at, t_at, "qualified_type", "x.T");
    }

    #[test]
    fn go_plain_identifier_top_level() {
        let src = "const Z = 3\n";
        let info = node_at(LanguageId::Go, src, src.find("Z").expect("fixture")).unwrap();
        assert_eq!(info.kind, "identifier");
        assert_eq!(info.text, "Z");
        assert!(info.scope_path.is_empty());
    }

    #[test]
    fn go_scope_chain_method_and_function() {
        // Go has no nesting: a method is associated with a type by its
        // receiver but declared at TOP LEVEL — so a method body has no
        // enclosing type scope (the "function inside a class/type" case
        // is impossible in Go; not written vacuously, asserted as-is).
        let src = "type Foo struct {\n\tname string\n}\nfunc (f Foo) Bar() {\n\tx := 1\n}\n";
        let at = src.find("x :=").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Go, src, at),
            vec![String::from("Bar")]
        );
        // Top-level function scope only.
        let src2 = "func main() {\n\t_ = 0\n}\n";
        let at2 = src2.find("_ =").expect("fixture");
        assert_eq!(scope_path_at(LanguageId::Go, src2, at2), vec![String::from("main")]);
    }

    #[test]
    fn go_boundary_offsets_do_not_panic() {
        let src = "func f() { g() }\n";
        // Byte 0 sits on the `func` keyword (not identifier-ish → no
        // node), but the scope is still reported.
        assert!(node_at(LanguageId::Go, src, 0).is_none());
        assert_eq!(scope_path_at(LanguageId::Go, src, 0), vec![String::from("f")]);
        let src2 = "var x int\n";
        let info = node_at(LanguageId::Go, src2, src2.find("x").expect("fixture")).unwrap();
        assert_eq!(info.text, "x");
        assert!(info.scope_path.is_empty());
        assert!(node_at(LanguageId::Go, src, src.len()).is_none());
        assert!(node_at(LanguageId::Go, src, src.len() + 4096).is_none());
        assert!(scope_path_at(LanguageId::Go, src, src.len()).is_empty());
        assert!(scope_path_at(LanguageId::Go, src, usize::MAX).is_empty());
    }

    #[test]
    fn go_broken_source_does_not_panic() {
        let src = "func f() {";
        let pos = src.find("f").expect("fixture");
        if let Some(info) = node_at(LanguageId::Go, src, pos) {
            assert_eq!(info.text, "f");
        }
        let _ = scope_path_at(LanguageId::Go, src, pos);
    }

    // ── C (lang-pred) ───────────────────────────────────

    /// Discriminating: `o.x.y` (chained `.` access) must come back WHOLE
    /// (one `field_expression` node) for an offset on any segment — and
    /// `->` member access has the same shape in C (both parse as
    /// `field_expression` in the pinned grammar).
    #[test]
    fn c_member_path_comes_back_whole() {
        let src = "struct S { int x; };\nint f(struct S o, struct S *p) { return o.x.y + p->x; }\n";
        let o_at = src.find("o.x").expect("fixture");
        let y_at = src.find(".y").expect("fixture") + 1;
        assert_whole_path(LanguageId::C, src, o_at, y_at, "field_expression", "o.x.y");
        let p_at = src.find("p->x").expect("fixture");
        let px_at = src.find("->x").expect("fixture") + 2;
        assert_whole_path(LanguageId::C, src, p_at, px_at, "field_expression", "p->x");
    }

    #[test]
    fn c_plain_identifier_top_level() {
        let src = "const int Z = 3;\n";
        let info = node_at(LanguageId::C, src, src.find("Z").expect("fixture")).unwrap();
        assert_eq!(info.kind, "identifier");
        assert_eq!(info.text, "Z");
        assert!(info.scope_path.is_empty());
    }

    #[test]
    fn c_scope_chain_struct_in_struct_and_function() {
        let src = "struct Outer { struct Inner { int v; } inner; };\nint f() { return 0; }\n";
        let at = src.find("v").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::C, src, at),
            vec![String::from("Outer"), String::from("Inner")]
        );
        // `f`'s own name sees its function, and code inside `f` sees it too.
        let at_name = node_at(LanguageId::C, src, src.find("f").expect("fixture")).unwrap();
        assert_eq!(at_name.scope_path, vec![String::from("f")]);
        let in_body = src.find("return 0").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::C, src, in_body),
            vec![String::from("f")]
        );
    }

    #[test]
    fn c_boundary_offsets_do_not_panic() {
        // The `struct` keyword token is anonymous (not identifier-ish → no
        // node) at byte 0, but the scope is still reported. (`int` at byte 0
        // WOULD match: `primitive_type` is identifier-ish, as in Rust.)
        let src = "struct S { int x; }\n";
        assert!(node_at(LanguageId::C, src, 0).is_none());
        assert_eq!(scope_path_at(LanguageId::C, src, 0), vec![String::from("S")]);
        // An identifier at byte 0 does resolve; top-level → no scope.
        let src2 = "x;\n";
        let info = node_at(LanguageId::C, src2, 0).expect("identifier at byte 0");
        assert_eq!(info.text, "x");
        assert!(info.scope_path.is_empty());
        // At end-of-file and past EOF: nothing contains the offset.
        assert!(node_at(LanguageId::C, src, src.len()).is_none());
        assert!(node_at(LanguageId::C, src, src.len() + 4096).is_none());
        assert!(scope_path_at(LanguageId::C, src, src.len()).is_empty());
        assert!(scope_path_at(LanguageId::C, src, usize::MAX).is_empty());
    }

    #[test]
    fn c_broken_source_does_not_panic() {
        let src = "int f() {";
        let pos = src.find("f").expect("fixture");
        if let Some(info) = node_at(LanguageId::C, src, pos) {
            assert_eq!(info.text, "f");
        }
        let _ = scope_path_at(LanguageId::C, src, pos);
    }

    // ── C++ (lang-pred) ──────────────────────────────────

    /// Discriminating: C++ member access `a.b.c` comes back WHOLE (one
    /// `field_expression` — the pinned cpp grammar uses `field_expression`
    /// for both `.` and `->`, no direct/pointer_member_access kinds).
    #[test]
    fn cpp_member_path_comes_back_whole() {
        let src = "struct S { int x; int y; };\nint f(struct S o) { return o.x.y; }\n";
        let o_at = src.find("o.x").expect("fixture");
        let y_at = src.find(".y").expect("fixture") + 1;
        assert_whole_path(LanguageId::Cpp, src, o_at, y_at, "field_expression", "o.x.y");
    }

    /// Discriminating: the cpp `::` path comes back WHOLE as one
    /// `qualified_identifier`, including the nested scope (`ns::Base::C`).
    #[test]
    fn cpp_qualified_path_comes_back_whole() {
        let src = "int w = ns::Base::C;\n";
        let ns_at = src.find("ns::").expect("fixture");
        let c_at = src.find("::C").expect("fixture") + 2;
        assert_whole_path(
            LanguageId::Cpp,
            src,
            ns_at,
            c_at,
            "qualified_identifier",
            "ns::Base::C",
        );
        // A single-segment `A::x` too.
        let src2 = "int w = Base::CONST;\n";
        assert_whole_path(
            LanguageId::Cpp,
            src2,
            src2.find("Base").expect("fixture"),
            src2.find("CONST").expect("fixture"),
            "qualified_identifier",
            "Base::CONST",
        );
    }

    #[test]
    fn cpp_plain_identifier_top_level() {
        let src = "const int Z = 3;\n";
        let info = node_at(LanguageId::Cpp, src, src.find("Z").expect("fixture")).unwrap();
        assert_eq!(info.kind, "identifier");
        assert_eq!(info.text, "Z");
        assert!(info.scope_path.is_empty());
    }

    #[test]
    fn cpp_scope_chain_namespace_class_method() {
        let src = "namespace outer {\nclass Base {\n    void m() { int y = 0; }\n};\n}\n";
        let at = src.find("y = 0").expect("fixture") + 1;
        assert_eq!(
            scope_path_at(LanguageId::Cpp, src, at),
            vec![
                String::from("outer"),
                String::from("Base"),
                String::from("m"),
            ]
        );
        // Same chain via NodeInfo at the method's own name.
        let at_name = node_at(LanguageId::Cpp, src, src.find("m()").expect("fixture")).unwrap();
        assert_eq!(
            at_name.scope_path,
            vec![
                String::from("outer"),
                String::from("Base"),
                String::from("m"),
            ]
        );
    }

    #[test]
    fn cpp_boundary_offsets_do_not_panic() {
        // The `class` keyword token is anonymous (not identifier-ish → no
        // node) at byte 0, but the scope is still reported.
        let src = "class S { int x; }\n";
        assert!(node_at(LanguageId::Cpp, src, 0).is_none());
        assert_eq!(scope_path_at(LanguageId::Cpp, src, 0), vec![String::from("S")]);
        // An identifier at byte 0 does resolve; top-level → no scope.
        let src2 = "x;\n";
        let info = node_at(LanguageId::Cpp, src2, 0).expect("identifier at byte 0");
        assert_eq!(info.text, "x");
        assert!(info.scope_path.is_empty());
        // At end-of-file and past EOF: nothing contains the offset.
        assert!(node_at(LanguageId::Cpp, src, src.len()).is_none());
        assert!(node_at(LanguageId::Cpp, src, src.len() + 4096).is_none());
        assert!(scope_path_at(LanguageId::Cpp, src, src.len()).is_empty());
        assert!(scope_path_at(LanguageId::Cpp, src, usize::MAX).is_empty());
    }

    #[test]
    fn cpp_broken_source_does_not_panic() {
        let src = "class A {";
        let pos = src.find("A").expect("fixture");
        if let Some(info) = node_at(LanguageId::Cpp, src, pos) {
            assert_eq!(info.text, "A");
        }
        let _ = scope_path_at(LanguageId::Cpp, src, pos);
    }

    // ── Bash (lang-pred) ──────────────────────────────────

    /// Discriminating: `node_at` on a command name returns the
    /// `command_name`, not its bare `word` child (the word is a path part
    /// — the pinned grammar has no `identifier` kind; words are `word`).
    #[test]
    fn bash_command_name_resolves_as_command_name() {
        let src = "cmd --flag other\n";
        let at = src.find("cmd").expect("fixture");
        let info = node_at(LanguageId::Bash, src, at).expect("node at `cmd`");
        assert_eq!(info.kind, "command_name");
        assert_eq!(info.text, "cmd");
        assert_eq!(info.start_byte, at);
        assert_eq!(info.end_byte, at + 3);
        // A plain word argument is NOT identifier-ish (deliberate — see
        // the Bash row's `identifier_kinds` note in `language.rs`).
        assert!(node_at(LanguageId::Bash, src, src.find("other").expect("fixture")).is_none());
    }

    #[test]
    fn bash_variable_name_resolves() {
        let src = "echo \"$MY_VAR\"\n";
        let at = src.find("MY_VAR").expect("fixture");
        let info = node_at(LanguageId::Bash, src, at).expect("node at `MY_VAR`");
        assert_eq!(info.kind, "variable_name");
        assert_eq!(info.text, "MY_VAR");
        assert!(info.scope_path.is_empty());
    }

    #[test]
    fn bash_scope_chain_function_body() {
        let src = "my_func() {\n    echo hi\n}\ncmd --flag other\n";
        let at = src.find("echo").expect("fixture");
        assert_eq!(scope_path_at(LanguageId::Bash, src, at), vec![String::from("my_func")]);
        // The function's OWN name sees itself as the enclosing item (the
        // name `word` is not identifier-ish, so `node_at` is None there —
        // the split surface, `scope_path_at`, still answers).
        let name_at = src.find("my_func").expect("fixture");
        assert!(node_at(LanguageId::Bash, src, name_at).is_none());
        assert_eq!(
            scope_path_at(LanguageId::Bash, src, name_at),
            vec![String::from("my_func")]
        );
        // Top-level command: no scope.
        assert!(scope_path_at(LanguageId::Bash, src, src.find("other").expect("fixture")).is_empty());
    }

    #[test]
    fn bash_boundary_offsets_do_not_panic() {
        // The `export` keyword token is anonymous (not identifier-ish →
        // no node) at byte 0; top-level → no scope.
        let src = "export X=1\n";
        assert!(node_at(LanguageId::Bash, src, 0).is_none());
        assert!(scope_path_at(LanguageId::Bash, src, 0).is_empty());
        // An identifier-ish node at byte 0 does resolve.
        let src2 = "MY_VAR=1\n";
        let info = node_at(LanguageId::Bash, src2, 0).expect("variable at byte 0");
        assert_eq!(info.kind, "variable_name");
        assert_eq!(info.text, "MY_VAR");
        assert!(info.scope_path.is_empty());
        // At end-of-file and past EOF: nothing contains the offset.
        assert!(node_at(LanguageId::Bash, src, src.len()).is_none());
        assert!(node_at(LanguageId::Bash, src, src.len() + 4096).is_none());
        assert!(scope_path_at(LanguageId::Bash, src, src.len()).is_empty());
        assert!(scope_path_at(LanguageId::Bash, src, usize::MAX).is_empty());
    }

    #[test]
    fn bash_broken_source_does_not_panic() {
        let src = "my_func() {";
        let pos = src.find("my_func").expect("fixture");
        // On a broken parse the name may or may not stay attached to the
        // `function_definition` — either answer is fine, no panic.
        if let Some(info) = node_at(LanguageId::Bash, src, pos) {
            assert_eq!(info.text, "my_func");
        }
        let _ = scope_path_at(LanguageId::Bash, src, pos);
    }

    // ── TOML (lang-pred) ──────────────────────────────────

    /// Discriminating: the dotted KEY `a.b.c` comes back WHOLE (one
    /// `dotted_key`) for an offset on any segment — TOML dotted keys are
    /// the grammar's path-shaped construct (in pair keys AND table
    /// headers).
    #[test]
    fn toml_dotted_key_comes_back_whole() {
        let src = "a.b.c = 1\n";
        let a_at = src.find("a.b").expect("fixture");
        let c_at = src.find(".c").expect("fixture") + 1;
        assert_whole_path(LanguageId::Toml, src, a_at, c_at, "dotted_key", "a.b.c");
    }

    #[test]
    fn toml_table_header_dotted_key_comes_back_whole() {
        let src = "[table.sub]\nkey = 2\n";
        let table_at = src.find("table.sub").expect("fixture");
        let sub_at = src.find("sub").expect("fixture");
        assert_whole_path(
            LanguageId::Toml,
            src,
            table_at,
            sub_at,
            "dotted_key",
            "table.sub",
        );
        // A plain key inside the table: the scope is the table header text
        // (reported as written — one element, see `toml_scope_path`).
        let key_at = src.find("key = 2").expect("fixture");
        let info = node_at(LanguageId::Toml, src, key_at).expect("node at `key`");
        assert_eq!(info.kind, "bare_key");
        assert_eq!(info.text, "key");
        assert_eq!(info.scope_path, vec![String::from("table.sub")]);
        assert_eq!(
            scope_path_at(LanguageId::Toml, src, key_at + 1),
            vec![String::from("table.sub")]
        );
    }

    #[test]
    fn toml_top_level_dotted_key_has_no_scope() {
        let src = "a.b.c = 1\n";
        let info = node_at(LanguageId::Toml, src, src.find("a").expect("fixture")).unwrap();
        assert_eq!(info.kind, "dotted_key");
        assert!(info.scope_path.is_empty());
    }

    #[test]
    fn toml_boundary_offsets_do_not_panic() {
        // Byte 0 sits on the `[` token (not identifier-ish → no node),
        // but the table header is still reported as the scope.
        let src = "[table]\nk = 1\n";
        assert!(node_at(LanguageId::Toml, src, 0).is_none());
        assert_eq!(scope_path_at(LanguageId::Toml, src, 0), vec![String::from("table")]);
        // A key at byte 0 does resolve; top-level → no scope.
        let src2 = "key = 1\n";
        let info = node_at(LanguageId::Toml, src2, 0).expect("key at byte 0");
        assert_eq!(info.text, "key");
        assert!(info.scope_path.is_empty());
        // At end-of-file and past EOF: nothing contains the offset.
        assert!(node_at(LanguageId::Toml, src, src.len()).is_none());
        assert!(node_at(LanguageId::Toml, src, src.len() + 4096).is_none());
        assert!(scope_path_at(LanguageId::Toml, src, src.len()).is_empty());
        assert!(scope_path_at(LanguageId::Toml, src, usize::MAX).is_empty());
    }

    #[test]
    fn toml_broken_source_does_not_panic() {
        let src = "[table";
        let pos = src.find("table").expect("fixture");
        if let Some(info) = node_at(LanguageId::Toml, src, pos) {
            assert_eq!(info.text, "table");
        }
        let _ = scope_path_at(LanguageId::Toml, src, pos);
    }

    // ── JSON (lang-pred) ──────────────────────────────────

    #[test]
    fn json_index_does_not_count_the_value_string_as_a_key_occurrence() {
        // gate P2 (017 symbol-identity): the index and the capture must use
        // the SAME gate. `collect_annotatable` filtered on kind alone, so for
        // `{"key": "key"}` it ALSO counted the value string — whose text is
        // identical to the key's — making the key's group length 2 and so
        // permanently unresolvable, with the annotation silently falling back
        // to the text rules. `nearest_identifier` never returns a value
        // string (that is what `json_key_resolves_but_value_string_does_not`
        // asserts), so the index must not count one either.
        let src = "{\"key\": \"key\"}\n";
        let idx = build_annotation_symbol_index(LanguageId::Json, src).expect("index");
        assert_eq!(
            idx.by_name
                .get(&(String::from("string"), String::from("\"key\"")))
                .map(|v| v.len()),
            Some(1),
            "only the KEY position is identifier-ish; the value string must not be counted"
        );
    }

    /// Discriminating for the JSON position rule: a `pair` KEY resolves
    /// (kind `string`, raw text WITH its quotes — `NodeInfo.text` is the
    /// node's source text), but a value string at the SAME offset does
    /// not (`string` is not identifier-ish outside the `key` field).
    #[test]
    fn json_key_resolves_but_value_string_does_not() {
        let src = "{\"k\": \"text\"}\n";
        let key_at = src.find("\"k\"").expect("fixture");
        let info = node_at(LanguageId::Json, src, key_at + 1).expect("node at key `k`");
        assert_eq!(info.kind, "string");
        assert_eq!(info.text, "\"k\"");
        assert_eq!(info.scope_path, vec![String::from("k")]);
        let value_at = src.find("text").expect("fixture");
        assert!(node_at(LanguageId::Json, src, value_at).is_none(), "value string must not resolve");
    }

    #[test]
    fn json_scope_chain_is_the_enclosing_key_chain() {
        // JSON's path-shaped concept is STRUCTURAL (nesting, not dotted
        // text): the scope chain is the enclosing keys, unquoted.
        let src = "{\"outer\": {\"inner\": {\"deep\": 1}}}\n";
        let deep_at = src.find("deep").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Json, src, deep_at),
            vec![
                String::from("outer"),
                String::from("inner"),
                String::from("deep"),
            ]
        );
        // The value `1` sits inside the innermost pair only.
        let value_at = src.find("1").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Json, src, value_at),
            vec![
                String::from("outer"),
                String::from("inner"),
                String::from("deep"),
            ]
        );
    }

    #[test]
    fn json_boundary_offsets_do_not_panic() {
        let src = "{\"a\": 1}\n";
        // Byte 0 sits on the `{` token (not identifier-ish → no node),
        // top-level → no scope either (the pair starts at the key).
        assert!(node_at(LanguageId::Json, src, 0).is_none());
        assert!(scope_path_at(LanguageId::Json, src, 0).is_empty());
        // A key at byte 0 does resolve (top-level pair → sees itself).
        let src2 = "{\"x\": 1}\n";
        let info = node_at(LanguageId::Json, src2, 1).expect("key at byte 1");
        assert_eq!(info.text, "\"x\"");
        assert_eq!(info.scope_path, vec![String::from("x")]);
        // At end-of-file and past EOF: nothing contains the offset.
        assert!(node_at(LanguageId::Json, src, src.len()).is_none());
        assert!(node_at(LanguageId::Json, src, src.len() + 4096).is_none());
        assert!(scope_path_at(LanguageId::Json, src, src.len()).is_empty());
        assert!(scope_path_at(LanguageId::Json, src, usize::MAX).is_empty());
    }

    #[test]
    fn json_broken_source_does_not_panic() {
        let src = "{\"a\":";
        let pos = src.find("a").expect("fixture");
        if let Some(info) = node_at(LanguageId::Json, src, pos) {
            assert_eq!(info.text, "\"a\"");
        }
        let _ = scope_path_at(LanguageId::Json, src, pos);
    }

    // ── Markdown (lang-pred) ──────────────────────────────────

    /// The honest N/A pin: a heading's title text is NOT identifier-ish
    /// (it is an `inline` node, shared with paragraphs/code spans — the
    /// block grammar has no identifier kind), so `node_at` returns
    /// `None` on Markdown headings and prose. Path-shaped M-. is N/A
    /// for Markdown; the outline lives in `scope_path_at` instead.
    #[test]
    fn markdown_heading_text_is_not_identifier_ish() {
        let src = "# Heading\n";
        let at = src.find("Heading").expect("fixture");
        assert!(node_at(LanguageId::Markdown, src, at).is_none());
        assert!(node_at(LanguageId::Markdown, src, 0).is_none());
    }

    #[test]
    fn markdown_scope_chain_is_the_enclosing_headings() {
        // The block grammar nests `section` by heading level (probed),
        // so the scope is the heading chain, outermost → innermost.
        let src = "# Top\n\ntext\n\n## Sub\n\nmore\n\n### Deeper\n";
        let at = src.find("Deeper").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Markdown, src, at),
            vec![
                String::from("Top"),
                String::from("Sub"),
                String::from("Deeper"),
            ]
        );
        // Content under Sub (but before Deeper) sees two levels.
        let more_at = src.find("more").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Markdown, src, more_at),
            vec![String::from("Top"), String::from("Sub")]
        );
        // Content directly under Top sees one level.
        let text_at = src.find("text").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Markdown, src, text_at),
            vec![String::from("Top")]
        );
    }

    #[test]
    fn markdown_setext_heading_contributes_scope() {
        let src = "Top\n=====\nbody\n";
        let at = src.find("body").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Markdown, src, at),
            vec![String::from("Top")]
        );
    }

    #[test]
    fn markdown_boundary_offsets_do_not_panic() {
        let src = "# Top\n\ntext\n";
        // Byte 0 sits on the `#` marker (not identifier-ish → no node),
        // but the heading section is still reported as the scope.
        assert!(node_at(LanguageId::Markdown, src, 0).is_none());
        assert_eq!(
            scope_path_at(LanguageId::Markdown, src, 0),
            vec![String::from("Top")]
        );
        // A document without headings has an empty outline everywhere.
        let src2 = "plain text\n";
        assert!(scope_path_at(LanguageId::Markdown, src2, 0).is_empty());
        // At end-of-file and past EOF: nothing contains the offset.
        assert!(node_at(LanguageId::Markdown, src, src.len()).is_none());
        assert!(node_at(LanguageId::Markdown, src, src.len() + 4096).is_none());
        assert!(scope_path_at(LanguageId::Markdown, src, src.len()).is_empty());
        assert!(scope_path_at(LanguageId::Markdown, src, usize::MAX).is_empty());
    }

    #[test]
    fn markdown_broken_source_does_not_panic() {
        // Markdown is near-impossible to break; a lone heading marker
        // without content is the closest thing.
        let src = "#";
        let _ = node_at(LanguageId::Markdown, src, 0);
        let _ = scope_path_at(LanguageId::Markdown, src, 0);
    }

    #[test]
    fn syntactically_broken_source_does_not_panic() {
        // Half a `fn`: unclosed block, ERROR nodes in the tree.
        let src = "fn main() {";
        let pos = src.find("main").expect("fixture");
        // tree-sitter still names the `fn`'s name under a broken parse —
        // if it does, the text must be exactly `main`.
        if let Some(info) = node_at(LanguageId::Rust, src, pos) {
            assert_eq!(info.text, "main");
            assert_eq!(info.kind, "identifier");
        }
        // Scope lookup must not panic either, whatever it finds.
        let _ = scope_path_at(LanguageId::Rust, src, pos);
        // A statement inside a broken fn still resolves (or is `None` —
        // both acceptable).
        let src2 = "fn broken() {\n    let a = 1\n";
        let pos2 = src2.find("a").expect("fixture");
        if let Some(info) = node_at(LanguageId::Rust, src2, pos2) {
            assert_eq!(info.text, "a");
        }
    }

    /// Discriminating for the split surface: at a keyword offset `node_at`
    /// is `None` (no identifier-ish ancestor), but `scope_path_at` still
    /// reports the enclosing items — and a *sibling* name (`bar`) must not
    /// leak in, only ancestors count.
    #[test]
    fn scope_path_at_works_where_node_at_is_none() {
        let src = "struct Foo;\nimpl Foo { fn bar(&self) { if x { y } } }\n";
        let if_at = src.find("if x").expect("fixture");
        assert!(node_at(LanguageId::Rust, src, if_at).is_none());
        assert_eq!(
            scope_path_at(LanguageId::Rust, src, if_at),
            vec![String::from("Foo"), String::from("bar")]
        );
    }

    // ── Java (new-languages lane) ─────────────────────────────────
    /// `A.c` / `o.x` come back whole as one `field_access` node (the
    /// dotted-path rule, probe-verified against tree-sitter-java 0.23.5).
    #[test]
    fn java_member_path_comes_back_whole() {
        let src = "class A { static int c; void f() { int x = A.c; } }\n";
        let at = src.find("A.c").expect("fixture") + 1;
        let info = node_at(LanguageId::Java, src, at).expect("node at `A.c`");
        assert_eq!(info.text, "A.c");
        assert_eq!(info.kind, "field_access");
        // The same node comes back from either segment.
        let info2 = node_at(LanguageId::Java, src, at - 1).expect("node at `A`");
        assert_eq!(info2.text, "A.c");
        let info3 = node_at(LanguageId::Java, src, at + 1).expect("node at `c`");
        assert_eq!(info3.text, "A.c");
    }

    /// `com.example.Foo` in type position comes back whole as one
    /// `scoped_type_identifier` (the Rust `::`-path shape, dot-delimited).
    #[test]
    fn java_scoped_type_path_comes_back_whole() {
        let src = "class B { void f() { com.example.Foo o = null; } }\n";
        let at = src.find("com.example.Foo").expect("fixture") + 4;
        let info = node_at(LanguageId::Java, src, at).expect("node at `example`");
        assert_eq!(info.text, "com.example.Foo");
        assert_eq!(info.kind, "scoped_type_identifier");
    }

    /// `o.m(…)` is NOT a path container: node_at on the bare `m` returns
    /// the plain identifier (the honest degradation — the invocation
    /// carries arguments and is not a dotted path).
    #[test]
    fn java_method_invocation_stays_bare() {
        let src = "class A { void f() { o.m(1); } }\n";
        let at = src.find("o.m(").expect("fixture") + 2;
        let info = node_at(LanguageId::Java, src, at).expect("node at `m`");
        assert_eq!(info.text, "m");
        assert_eq!(info.kind, "identifier");
    }

    #[test]
    fn java_scope_chain_class_and_method() {
        let src = "class A { void f() { A.c; } }\n";
        let at = src.find("A.c").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Java, src, at),
            vec![String::from("A"), String::from("f")]
        );
        // Inside a class body but outside any method: only the class.
        let src2 = "class A { int c; }\n";
        let at2 = src2.find("c;").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Java, src2, at2),
            vec![String::from("A")]
        );
    }

    // ── C# (new-languages lane) ──────────────────────────────────────
    /// `o.P` / `a.b.c` come back whole as one `member_access_expression`
    /// (the dotted-path rule, probe-verified against the pinned
    /// tree-sitter-c-sharp 0.23.5, re-pinned by the grammar-bumps suite).
    #[test]
    fn csharp_member_path_comes_back_whole() {
        let src = "class A { void F() { int v = o.P; } }\n";
        let at = src.find("o.P").expect("fixture") + 1;
        let info = node_at(LanguageId::CSharp, src, at).expect("node at `.P`");
        assert_eq!(info.text, "o.P");
        assert_eq!(info.kind, "member_access_expression");
    }

    /// `N.Inner` in a namespace header comes back whole as one
    /// `qualified_name`.
    #[test]
    fn csharp_qualified_name_comes_back_whole() {
        let src = "namespace N.Inner { class A { } }\n";
        let at = src.find("N.Inner").expect("fixture") + 2;
        let info = node_at(LanguageId::CSharp, src, at)
            .expect("node at `Inner`");
        assert_eq!(info.text, "N.Inner");
        assert_eq!(info.kind, "qualified_name");
    }

    #[test]
    fn csharp_scope_chain_namespace_class_method() {
        let src = "namespace N { class A { void F() { o.P; } } }\n";
        let at = src.find("o.P").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::CSharp, src, at),
            vec![
                String::from("N"),
                String::from("A"),
                String::from("F")
            ]
        );
    }

    // ── Ruby (new-languages lane) ────────────────────────────────────
    /// `obj.name` comes back whole as one argumentless `call` (the
    /// member-access shape), from either segment.
    #[test]
    fn ruby_method_chain_comes_back_whole() {
        let src = "def show(obj)\n  puts obj.name\nend\n";
        let at = src.find("obj.name").expect("fixture");
        for off in 0..=5 {
            let info =
                node_at(LanguageId::Ruby, src, at + off).expect("node in `obj.name`");
            assert_eq!(info.text, "obj.name", "offset {off}");
            assert_eq!(info.kind, "call");
        }
    }

    /// `Foo::Bar` comes back as part of the whole chain: `Foo::Bar.new`
    /// is one argumentless `call` whose receiver is the `scope_
    /// resolution` — the whole chain comes back as one node (the JS
    /// whole-`A.B.C` rule), from either segment.
    #[test]
    fn ruby_scope_resolution_comes_back_whole() {
        let src = "module Foo\n  def f\n    Foo::Bar.new\n  end\nend\n";
        let at = src.find("Foo::Bar").expect("fixture") + 4;
        let info = node_at(LanguageId::Ruby, src, at).expect("node at `Bar`");
        assert_eq!(info.text, "Foo::Bar.new");
        assert_eq!(info.kind, "call");
    }

    /// A bare call WITH arguments is not a path: `puts 1` resolves on
    /// the plain `puts` identifier (the honest degradation — the
    /// argumentless-receiver rule, probe-verified).
    #[test]
    fn ruby_bare_call_stays_bare() {
        let src = "def f\n  puts 1\nend\n";
        let at = src.find("puts").expect("fixture") + 1;
        let info = node_at(LanguageId::Ruby, src, at).expect("node at `uts`");
        assert_eq!(info.text, "puts");
        assert_eq!(info.kind, "identifier");
        // `a.b(1).c` keeps its argumentless outer chain whole, but the
        // inner `b(1)` call (with arguments) does not contribute.
        let src2 = "def g\n  a.b(1).c\nend\n";
        let at2 = src2.find("a.b").expect("fixture");
        let info2 = node_at(LanguageId::Ruby, src2, at2 + 7).expect("node at `c`");
        assert_eq!(info2.text, "a.b(1).c");
        assert_eq!(info2.kind, "call");
    }

    #[test]
    fn ruby_scope_chain_module_class_method() {
        let src = "module M\n  class C\n    def m\n      o.p\n    end\n  end\nend\n";
        let at = src.find("o.p").expect("fixture");
        assert_eq!(
            scope_path_at(LanguageId::Ruby, src, at),
            vec![String::from("M"), String::from("C"), String::from("m")]
        );
    }

    // ── Scheme (new-languages lane) ──────────────────────────────────
    /// A symbol resolves to itself (the flat grammar's only name kind).
    #[test]
    fn scheme_symbol_resolves_bare() {
        let src = "(define (add! x y) (+ x y))\n";
        let at = src.find("add!").expect("fixture") + 1;
        let info = node_at(LanguageId::Scheme, src, at).expect("node at `dd!`");
        assert_eq!(info.text, "add!");
        assert_eq!(info.kind, "symbol");
    }

    /// Scheme has NO path-shaped construct and NO named definition
    /// containers: a symbol inside a `define-library` body resolves as
    /// itself (no container walk), and the scope path stays `[]` even
    /// nested in a library body (the honest N/A, probe-verified against
    /// the flat tree-sitter-scheme 0.24.7 node set).
    #[test]
    fn scheme_has_no_path_or_scope() {
        let src = "(define-library (foo core)\n  (define (inner a) a))\n";
        let at = src.find("inner").expect("fixture") + 1;
        let info = node_at(LanguageId::Scheme, src, at).expect("node at `inner`");
        assert_eq!(info.text, "inner");
        assert_eq!(info.kind, "symbol");
        assert!(scope_path_at(LanguageId::Scheme, src, at).is_empty());
    }

    /// Clojure: a symbol resolves to its whole `sym_lit`, and a
    /// namespaced symbol (`my.lib` — the `.` is INSIDE the single
    /// `sym_name` leaf, probe-verified) comes back whole as ONE
    /// identifier (Clojure has no path-shaped container beyond the
    /// namespaced token itself — the honest N/A for M-. path-shaped,
    /// same posture as Scheme).
    #[test]
    fn clojure_symbol_resolves_bare_and_namespaced() {
        let src = "(defn double [v] v)\n(ns my.lib)\n";
        let at = src.find("double").expect("fixture") + 1;
        let info = node_at(LanguageId::Clojure, src, at).expect("node at `double`");
        assert_eq!(info.text, "double");
        assert_eq!(info.kind, "sym_lit");
        // A meta prefix belongs to the symbol's OWN `sym_lit`
        // (probe-verified: `(def ^:doc x 10)` — the `meta_lit` is a field
        // of the name's `sym_lit` and INSIDE its byte range), so M-. on
        // `^:doc` resolves the whole meta-carrying symbol (the honest
        // degradation — the outline itself captures the bare `sym_name`
        // `x`, so indexing stays clean; an M-. on the meta text bails
        // like any unindexed name).
        let kw_src = "(def ^:doc x 10)\n";
        let kw_at = kw_src.find(":doc").expect("fixture") + 2;
        let info = node_at(LanguageId::Clojure, kw_src, kw_at)
            .expect("meta-prefixed name resolves");
        assert_eq!(info.text, "^:doc x");
        assert_eq!(info.kind, "sym_lit");
        // A BARE keyword is not identifier-ish: `node_at` on `:bar` bails.
        let kw2 = "(foo :bar)\n";
        let kw2_at = kw2.find(":bar").expect("fixture") + 1;
        assert!(node_at(LanguageId::Clojure, kw2, kw2_at).is_none());
        // The namespaced symbol `my.lib` resolves whole, in the
        // single-token namespaced name's own byte range.
        let at = src.find("my.lib").expect("fixture") + 1;
        let info = node_at(LanguageId::Clojure, src, at).expect("node at `my.lib`");
        assert_eq!(info.text, "my.lib");
        assert_eq!(info.kind, "sym_lit");
    }

    /// Clojure has NO named definition containers: a symbol inside a
    /// `defn` body resolves as itself (no container walk), and the scope
    /// path stays `[]` even nested in a defn body (the honest N/A,
    /// probe-verified against the flat tree-sitter-clojure 0.1.0 node
    /// set — `defn` is a `list_lit`, not a named container).
    #[test]
    fn clojure_has_no_scope() {
        let src = "(defn outer [x]\n  (defn inner [] x))\n";
        let at = src.find("inner").expect("fixture") + 1;
        let info = node_at(LanguageId::Clojure, src, at).expect("node at `inner`");
        assert_eq!(info.text, "inner");
        assert_eq!(info.kind, "sym_lit");
        assert!(scope_path_at(LanguageId::Clojure, src, at).is_empty());
    }

    // ── issue-annotations-symbol-identity: the scope-aware identity ────

    /// The Nth occurrence (0-based, document order) of `needle` in `src`, as
    /// a byte offset.
    fn nth(src: &str, needle: &str, n: usize) -> usize {
        let mut start = 0usize;
        for _ in 0..=n {
            let rel = src[start..].find(needle).expect("fixture has more occurrences");
            start = start + rel + needle.len();
        }
        start - needle.len()
    }

    /// Discriminating: a repeated name is identified by its ENCLOSING SCOPE
    /// and its ORDINAL within that scope — `foo` called twice in `bar` and
    /// once in `baz` gives distinct identities, stable under an insertion
    /// above (the scope + ordinal are unchanged by it).
    #[test]
    fn symbol_identity_repeated_name_by_scope_and_ordinal() {
        let src = "fn bar() {\n    foo();\n    foo();\n}\nfn baz() {\n    foo();\n}\n";
        let p0 = nth(src, "foo", 0); // in bar, 1st
        let p1 = nth(src, "foo", 1); // in bar, 2nd
        let p2 = nth(src, "foo", 2); // in baz, 1st
        let a = symbol_identity_at(LanguageId::Rust, src, p0).unwrap();
        assert_eq!((a.kind.as_str(), a.name.as_str()), ("identifier", "foo"));
        assert_eq!(a.scope, vec![String::from("bar")]);
        assert_eq!(a.ordinal, 0, "first `foo` in `bar`");
        let b = symbol_identity_at(LanguageId::Rust, src, p1).unwrap();
        assert_eq!(b.scope, vec![String::from("bar")]);
        assert_eq!(b.ordinal, 1, "second `foo` in `bar`");
        let c = symbol_identity_at(LanguageId::Rust, src, p2).unwrap();
        assert_eq!(c.scope, vec![String::from("baz")], "a different enclosing scope");
        assert_eq!(c.ordinal, 0, "first `foo` in `baz`");
    }

    /// The index agrees with the per-point identity: `by_scope` groups the
    /// same (kind, name, scope) into an ordered vec whose index IS the
    /// ordinal, and `by_name` (the legacy, scope-blind view) counts them
    /// all.
    #[test]
    fn symbol_index_matches_identity_and_keeps_legacy_name_count() {
        let src = "fn bar() {\n    foo();\n    foo();\n}\nfn baz() {\n    foo();\n}\n";
        let idx = build_annotation_symbol_index(LanguageId::Rust, src).unwrap();
        let bar = idx
            .by_scope
            .get(&("identifier".to_string(), "foo".to_string(), vec![String::from("bar")]))
            .expect("(identifier, foo, [bar]) group");
        assert_eq!(bar.len(), 2, "two `foo` in `bar`");
        let baz = idx
            .by_scope
            .get(&("identifier".to_string(), "foo".to_string(), vec![String::from("baz")]))
            .expect("(identifier, foo, [baz]) group");
        assert_eq!(baz.len(), 1, "one `foo` in `baz`");
        // The legacy, scope-blind count: 3 occurrences of (identifier, foo).
        let all = idx
            .by_name
            .get(&("identifier".to_string(), "foo".to_string()))
            .expect("(identifier, foo) legacy group");
        assert_eq!(all.len(), 3, "3 total `foo` (the old uniqueness rule sees them all)");
        // A UNIQUE symbol (the fn name `baz`) is a 1-element group in both.
        let baz_name = idx
            .by_name
            .get(&("identifier".to_string(), "baz".to_string()))
            .expect("(identifier, baz)");
        assert_eq!(baz_name.len(), 1);
    }

    /// The Python repeated-name case (the acceptance matrix language): a
    /// call repeated in two functions is disambiguated by scope + ordinal.
    #[test]
    fn symbol_identity_python_repeated_name() {
        let src = "def bar():\n    foo()\n    foo()\ndef baz():\n    foo()\n";
        let p0 = nth(src, "foo", 0);
        let p2 = nth(src, "foo", 2);
        let a = symbol_identity_at(LanguageId::Python, src, p0).unwrap();
        assert_eq!(a.name, "foo");
        assert_eq!(a.scope, vec![String::from("bar")]);
        assert_eq!(a.ordinal, 0);
        let c = symbol_identity_at(LanguageId::Python, src, p2).unwrap();
        assert_eq!(c.scope, vec![String::from("baz")]);
        assert_eq!(c.ordinal, 0);
    }

    /// A keyword / no-symbol offset yields `None` (the honest answer), and
    /// out-of-range offsets never panic.
    #[test]
    fn symbol_identity_keyword_and_bounds_are_none() {
        let src = "fn bar() {\n    foo();\n}\n";
        // Byte 0 is the `f` of the `fn` keyword → not identifier-ish.
        assert!(symbol_identity_at(LanguageId::Rust, src, 0).is_none());
        // End-of-file / past EOF: nothing contains the offset.
        assert!(symbol_identity_at(LanguageId::Rust, src, src.len()).is_none());
        assert!(symbol_identity_at(LanguageId::Rust, src, usize::MAX).is_none());
    }

    /// A language with no identifier-ish kind (Yaml) builds no index.
    #[test]
    fn symbol_index_none_for_no_kind_language() {
        assert!(build_annotation_symbol_index(LanguageId::Yaml, "a: 1\n").is_none());
    }
}

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

use tree_sitter::{Node, Parser};

use crate::syntax::registry::LanguageId;

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
/// the smallest identifier-ish node containing `byte` — a `scoped_identifier`
/// / `scoped_type_identifier` comes back whole regardless of which segment
/// the offset sits on. Offsets at `source.len()` or past EOF return `None`
/// (nothing contains them); an offset at byte 0 is a normal containment
/// check (the `fn` keyword at a file start is not identifier-ish, so it
/// yields `None`).
///
/// Rust is fully implemented; every other `LanguageId` returns `None`
/// (the plan's "Rust first, graceful degradation" decision — callers
/// degrade to today's behavior).
pub fn node_at(lang: LanguageId, source: &str, byte: usize) -> Option<NodeInfo> {
    let tree = parse_source(lang, source)?;
    let leaf = innermost_at(tree.root_node(), byte)?;
    let node = nearest_identifier(leaf)?;
    let source_bytes = source.as_bytes();
    Some(NodeInfo {
        text: node.utf8_text(source_bytes).ok()?.to_string(),
        kind: node.kind().to_string(),
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        scope_path: rust_scope_path(leaf, source_bytes),
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
    rust_scope_path(leaf, source.as_bytes())
}

/// Parse `source` for `lang`. Per-language extension point: only Rust is
/// implemented today; every other `LanguageId` (including `Plain`) degrades
/// to `None`. Future languages slot in here without restructuring the
/// public surface.
fn parse_source(lang: LanguageId, source: &str) -> Option<tree_sitter::Tree> {
    // Rust only (the plan's "Rust first, graceful degradation" decision);
    // every other id falls through to the wrong-arm guard before the
    // grammar lookup. The grammar itself comes from the shared
    // `queries::language_for` pin so the tree-sitter grammar versions live
    // in exactly one place (no second registry, no duplicated pin).
    if lang != LanguageId::Rust {
        return None;
    }
    let language = crate::syntax::queries::language_for(lang)?;
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
/// identifier-ish.
fn nearest_identifier(leaf: Node) -> Option<Node> {
    let mut cur = leaf;
    loop {
        if !is_path_segment(cur) && cur.is_named() && is_rust_identifier_kind(cur.kind()) {
            return Some(cur);
        }
        cur = cur.parent()?;
    }
}

/// Whether `node` is a part of a larger `::` path rather than a complete
/// identifier of its own: it must be a child of a `scoped_identifier` /
/// `scoped_type_identifier` and be one of that path's segment kinds.
fn is_path_segment(node: Node) -> bool {
    matches!(
        node.parent().map(|p| p.kind()),
        Some("scoped_identifier") | Some("scoped_type_identifier")
    )
        && (node.kind() == "::"
            || (node.is_named()
                && matches!(
                    node.kind(),
                    "identifier"
                        | "type_identifier"
                        | "primitive_type"
                        | "scoped_identifier"
                        | "scoped_type_identifier"
                )))
}

/// The Rust identifier-ish node-kind predicate (per-language extension
/// point — other languages add their own predicate and slot into
/// `nearest_identifier`/`parse_source` later).
fn is_rust_identifier_kind(kind: &str) -> bool {
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

/// Walk ancestors of `leaf` and capture each enclosing scope item's name
/// child, outermost → innermost. Scope items: `mod_item`, `impl_item`
/// (name child in the `type` field), `trait_item`, `function_item` (name
/// child in the `name` field). Blocks, loops, and closures are
/// intentionally not scope items — keep it simple and honest; an empty vec
/// is the valid answer for top-level code.
fn rust_scope_path(leaf: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = Some(leaf);
    while let Some(node) = cur {
        let field = match node.kind() {
            "mod_item" | "trait_item" | "function_item" | "function_signature_item" => {
                Some("name")
            }
            // `impl Foo` / `impl Trait for Foo` put the self type in `type`.
            // Unwrap `impl<T> Foo<T>` (generic_type) to the bare name so a
            // resolver can match `Foo::push` — the raw field text would be
            // `Foo<T>`. `impl Trait for u8` keeps `u8` (primitive_type);
            // `&mut Foo` / `Box<Foo>` keep their written form (a deliberate
            // simplification — follow-up if resolution needs the base).
            "impl_item" => Some("type"),
            _ => None,
        };
        if let Some(field) = field
            && let Some(name_node) = node.child_by_field_name(field)
        {
            // Strip a generic parameter list: `generic_type`'s own `type`
            // field is the bare path (e.g. `Vec` for `Vec<T>`).
            let name_node = if name_node.kind() == "generic_type" {
                name_node
                    .child_by_field_name("type")
                    .unwrap_or(name_node)
            } else {
                name_node
            };
            if let Ok(text) = name_node.utf8_text(source) {
                names.push(text.to_string());
            }
        }
        cur = node.parent();
    }
    names.reverse(); // innermost-first walk → outermost-first answer
    names
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

    #[test]
    fn non_rust_languages_return_none() {
        for id in LanguageId::ALL {
            if *id == LanguageId::Rust {
                continue;
            }
            let src = "def f():\n    return 1\n";
            assert!(node_at(*id, src, 4).is_none(), "{id:?} node_at");
            assert!(
                scope_path_at(*id, src, 4).is_empty(),
                "{id:?} scope_path_at"
            );
        }
        assert!(node_at(LanguageId::Plain, "abc", 1).is_none());
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
}

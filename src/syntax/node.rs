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
/// the smallest identifier-ish node containing `byte` — a whole dotted path
/// (`tokio::spawn`, `a.b.c`, `pkg.Fn`) comes back as ONE node regardless
/// of which segment the offset sits on. Offsets at `source.len()` or past
/// EOF return `None` (nothing contains them); an offset at byte 0 is a
/// normal containment check (a keyword at a file start is not
/// identifier-ish, so it yields `None`).
///
/// Rust, JavaScript, TypeScript/TSX, Python, and Go are implemented;
/// every other `LanguageId` (including `Plain`) returns `None` (the
/// plan's "Rust first, graceful degradation" decision — callers degrade
/// to today's behavior).
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

/// Parse `source` for `lang`: Rust, JavaScript, TypeScript, TSX, Python,
/// and Go are implemented; every other `LanguageId` (including `Plain`)
/// degrades to `None`. Future languages slot in here without
/// restructuring the public surface.
fn parse_source(lang: LanguageId, source: &str) -> Option<tree_sitter::Tree> {
    match lang {
        LanguageId::Rust | LanguageId::JavaScript | LanguageId::TypeScript
        | LanguageId::Tsx | LanguageId::Python | LanguageId::Go
        | LanguageId::C | LanguageId::Cpp | LanguageId::Bash => {}
        _ => return None,
    }
    // The grammar itself comes from the shared `queries::language_for` pin
    // so the tree-sitter grammar versions live in exactly one place (no
    // second registry, no duplicated pin); `Plain` and unimplemented ids
    // never reach the grammar lookup.
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
fn nearest_identifier(leaf: Node, lang: LanguageId) -> Option<Node> {
    let mut cur = leaf;
    loop {
        if !is_path_segment(cur, lang) && cur.is_named() && is_identifier_kind(lang, cur.kind()) {
            return Some(cur);
        }
        cur = cur.parent()?;
    }
}

/// Whether `node` is a part of a larger dotted path rather than a complete
/// identifier of its own (so `nearest_identifier` skips it and returns the
/// whole path as one node). The path container and its segment kinds
/// differ per language (each verified against the pinned grammar's
/// `NODE_TYPES`):
/// - Rust: `scoped_identifier` / `scoped_type_identifier` with `::` tokens
///   and identifier-ish segments;
/// - JS/TS: `member_expression` (`a.b.c` — object, `.`, property) and
///   `nested_type_identifier` / `nested_identifier` (`A.B.C` in type
///   position, TS only);
/// - Python: `attribute` (`a.b.c` — value, `.`, attribute; the value may
///   itself be an `attribute`, so chains nest);
/// - Go: `selector_expression` (`pkg.Fn`) and `qualified_type` (`pkg.T`).
fn is_path_segment(node: Node, lang: LanguageId) -> bool {
    let parent_kind = node.parent().map(|p| p.kind());
    match lang {
        LanguageId::Rust => matches!(
            parent_kind,
            Some("scoped_identifier") | Some("scoped_type_identifier")
        ) && (
            node.kind() == "::"
                || (node.is_named()
                    && matches!(
                        node.kind(),
                        "identifier"
                            | "type_identifier"
                            | "primitive_type"
                            | "scoped_identifier"
                            | "scoped_type_identifier"
                    ))
        ),
        LanguageId::JavaScript | LanguageId::TypeScript | LanguageId::Tsx => {
            matches!(
                parent_kind,
                Some("member_expression")
                    | Some("nested_type_identifier")
                    | Some("nested_identifier")
            ) && (
                node.kind() == "."
                    || (node.is_named()
                        && matches!(
                            node.kind(),
                            "identifier"
                                | "property_identifier"
                                | "type_identifier"
                                | "member_expression"
                                | "nested_identifier"
                                | "nested_type_identifier"
                        ))
            )
        }
        LanguageId::Python => parent_kind == Some("attribute")
            && (node.kind() == "."
                || (node.is_named()
                    && matches!(node.kind(), "identifier" | "attribute"))),
        LanguageId::Go => {
            (parent_kind == Some("selector_expression")
                && node.is_named()
                && matches!(
                    node.kind(),
                    "identifier" | "field_identifier" | "selector_expression"
                ))
                || (parent_kind == Some("qualified_type")
                    && node.is_named()
                    && matches!(
                        node.kind(),
                        "identifier" | "type_identifier" | "qualified_type"
                    ))
        }
        // C member access (`a.b`, `p->x` — both are `field_expression` in
        // the pinned grammar, per its NODE_TYPES probe). The `argument`
        // child may be ANY expression (`o.x.y` nests `field_expression`s;
        // `foo(a).b` nests a `call_expression`), so any NAMED child of a
        // `field_expression` is a path part; the `.`/`->` tokens are
        // anonymous and never match.
        //
        // C++ shares `field_expression` (the pinned tree-sitter-cpp grammar
        // has no direct_member_access/pointer_member_access kinds — probed)
        // and adds `::` qualified names: `qualified_identifier` with a
        // `scope` child (a `namespace_identifier`, `type_identifier`, or a
        // nested `qualified_identifier`) and a `name` child, so `ns::A::x`
        // comes back whole as one node (the app's Rust `::` scan analogue).
        LanguageId::C | LanguageId::Cpp => {
            (parent_kind == Some("field_expression") && node.is_named())
                || (parent_kind == Some("qualified_identifier") && node.is_named())
        }
        // Bash's `command_name` wraps its single `word` child; the word is
        // a path part so `node_at` on the command name returns the
        // `command_name`, not the bare word. (The pinned tree-sitter-bash
        // grammar has no `identifier` kind at all — probed: words are
        // `word`, commands `command_name`, variables `variable_name`.)
        LanguageId::Bash => parent_kind == Some("command_name") && node.is_named(),
        _ => false,
    }
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

/// JavaScript identifier-ish node kinds (verified against the pinned
/// tree-sitter-javascript `NODE_TYPES`: `property_identifier` is JS's
/// name-leaf kind; `type_identifier` / `nested_type_identifier` do NOT
/// exist in the JS grammar — they are TypeScript-only kinds).
fn is_js_identifier_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier" | "property_identifier" | "member_expression"
    )
}

/// TypeScript/TSX identifier-ish node kinds (verified against the pinned
/// tree-sitter-typescript `NODE_TYPES` for both the TS and TSX grammars).
fn is_ts_identifier_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "property_identifier"
            | "type_identifier"
            | "member_expression"
            | "nested_type_identifier"
    )
}

/// Python identifier-ish node kinds (verified against the pinned
/// tree-sitter-python `NODE_TYPES`).
fn is_python_identifier_kind(kind: &str) -> bool {
    matches!(kind, "identifier" | "attribute")
}

/// Go identifier-ish node kinds (verified against the pinned
/// tree-sitter-go `NODE_TYPES`).
fn is_go_identifier_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "field_identifier"
            | "type_identifier"
            | "selector_expression"
            | "qualified_type"
    )
}

/// C identifier-ish node kinds (verified against the pinned
/// tree-sitter-c `NODE_TYPES`): `identifier` (values), `field_identifier`
/// (struct members), `type_identifier` (struct/enum/typedef names),
/// `primitive_type` (`int`, …), and `field_expression` (the whole
/// `a.b` / `p->x` member access).
fn is_c_identifier_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "field_identifier"
            | "type_identifier"
            | "primitive_type"
            | "field_expression"
    )
}

/// C++ identifier-ish node kinds (verified against the pinned
/// tree-sitter-cpp `NODE_TYPES`): the C set plus `namespace_identifier`
/// (a `ns::` scope name) and `qualified_identifier` (the whole `A::x` /
/// `ns::A::x` path).
fn is_cpp_identifier_kind(kind: &str) -> bool {
    is_c_identifier_kind(kind)
        || matches!(kind, "namespace_identifier" | "qualified_identifier")
}

/// Bash identifier-ish node kinds (verified against the pinned
/// tree-sitter-bash `NODE_TYPES`: there is no `identifier` kind —
/// `command_name` for command names and `variable_name` for variables
/// are the identifier-ish kinds). Plain `word`s (arguments, function
/// names) are deliberately NOT identifier-ish — a bare M-. context on
/// an arbitrary word is noise; function names still contribute to the
/// scope chain via `function_definition`.
fn is_bash_identifier_kind(kind: &str) -> bool {
    matches!(kind, "command_name" | "variable_name")
}

/// The identifier-kind predicate for `lang` — the per-language extension
/// point used by `nearest_identifier`.
fn is_identifier_kind(lang: LanguageId, kind: &str) -> bool {
    match lang {
        LanguageId::Rust => is_rust_identifier_kind(kind),
        LanguageId::JavaScript => is_js_identifier_kind(kind),
        LanguageId::TypeScript | LanguageId::Tsx => is_ts_identifier_kind(kind),
        LanguageId::Python => is_python_identifier_kind(kind),
        LanguageId::Go => is_go_identifier_kind(kind),
        LanguageId::C => is_c_identifier_kind(kind),
        LanguageId::Cpp => is_cpp_identifier_kind(kind),
        LanguageId::Bash => is_bash_identifier_kind(kind),
        _ => false,
    }
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

/// Dispatch the enclosing-scope walk to the per-language implementation;
/// unimplemented languages (including `Plain`) contribute nothing.
fn scope_path_for(lang: LanguageId, leaf: Node, source: &[u8]) -> Vec<String> {
    match lang {
        LanguageId::Rust => rust_scope_path(leaf, source),
        LanguageId::JavaScript | LanguageId::TypeScript | LanguageId::Tsx => {
            js_ts_scope_path(leaf, source)
        }
        LanguageId::Python => python_scope_path(leaf, source),
        LanguageId::Go => go_scope_path(leaf, source),
        LanguageId::C => c_scope_path(leaf, source),
        LanguageId::Cpp => cpp_scope_path(leaf, source),
        LanguageId::Bash => bash_scope_path(leaf, source),
        _ => Vec::new(),
    }
}

/// Walk ancestors of `leaf` and capture each enclosing scope item's name
/// child, outermost → innermost. Scope items: `function_declaration`,
/// `method_definition`, `class_declaration`, `abstract_class_declaration`
/// (TS) — name child in the `name` field — plus an `arrow_function`
/// assigned to a variable (`const f = () => …`): the arrow has no name of
/// its own, so the enclosing `variable_declarator`'s `name` field carries
/// it. Blocks, loops, and anonymous functions are intentionally not scope
/// items — keep it simple and honest.
fn js_ts_scope_path(leaf: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = Some(leaf);
    while let Some(node) = cur {
        let name_node = match node.kind() {
            "function_declaration" | "method_definition" | "class_declaration"
            | "abstract_class_declaration" => node.child_by_field_name("name"),
            "arrow_function" => node
                .parent()
                .filter(|p| p.kind() == "variable_declarator")
                .and_then(|p| p.child_by_field_name("name")),
            _ => None,
        };
        if let Some(name_node) = name_node
            && let Ok(text) = name_node.utf8_text(source)
        {
            names.push(text.to_string());
        }
        cur = node.parent();
    }
    names.reverse(); // innermost-first walk → outermost-first answer
    names
}

/// The Python enclosing-scope walk: `function_definition` and
/// `class_definition` (name child in the `name` field), outermost →
/// innermost. Comprehensions and lambdas are intentionally not scope items.
fn python_scope_path(leaf: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = Some(leaf);
    while let Some(node) = cur {
        if (node.kind() == "function_definition"
            || node.kind() == "class_definition")
            && let Some(name_node) = node.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(source)
        {
            names.push(text.to_string());
        }
        cur = node.parent();
    }
    names.reverse(); // innermost-first walk → outermost-first answer
    names
}

/// The Go enclosing-scope walk: `function_declaration`,
/// `method_declaration` (name child in the `name` field), and
/// `type_declaration` (the name lives in its `type_spec` child's `name`
/// field). Multi-spec declarations (`type (A int; B string)`) are
/// reported by the FIRST spec's name — a deliberate simplification; such
/// groupings are rare in Go.
fn go_scope_path(leaf: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = Some(leaf);
    while let Some(node) = cur {
        let name_node = match node.kind() {
            "function_declaration" | "method_declaration" => {
                node.child_by_field_name("name")
            }
            "type_declaration" => (0..node.child_count())
                .filter_map(|i| node.child(i))
                .find(|c| c.kind() == "type_spec")
                .and_then(|spec| spec.child_by_field_name("name")),
            _ => None,
        };
        if let Some(name_node) = name_node
            && let Ok(text) = name_node.utf8_text(source)
        {
            names.push(text.to_string());
        }
        cur = node.parent();
    }
    names.reverse(); // innermost-first walk → outermost-first answer
    names
}

/// The C enclosing-scope walk: `function_definition` (the name sits one
/// level down: `declarator` field → `function_declarator` → its
/// `declarator` field), and `struct_specifier` / `union_specifier` /
/// `enum_specifier` (name child in the `name` field). Blocks, control
/// flow, and nested compound statements are intentionally not scope items.
fn c_scope_path(leaf: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = Some(leaf);
    while let Some(node) = cur {
        let name_node = match node.kind() {
            "function_definition" => node
                .child_by_field_name("declarator")
                .and_then(|decl| decl.child_by_field_name("declarator")),
            "struct_specifier" | "union_specifier" | "enum_specifier" => {
                node.child_by_field_name("name")
            }
            _ => None,
        };
        if let Some(name_node) = name_node
            && let Ok(text) = name_node.utf8_text(source)
        {
            names.push(text.to_string());
        }
        cur = node.parent();
    }
    names.reverse(); // innermost-first walk → outermost-first answer
    names
}

/// The C++ enclosing-scope walk: `function_definition` (same
/// `declarator` → `function_declarator` → `declarator` name shape as C,
/// where a class method's name is a `field_identifier`),
/// `class_specifier` / `struct_specifier` / `union_specifier` /
/// `enum_specifier` (name child in the `name` field), and
/// `namespace_definition` (name child in the `name` field). Blocks and
/// control flow are intentionally not scope items.
fn cpp_scope_path(leaf: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = Some(leaf);
    while let Some(node) = cur {
        let name_node = match node.kind() {
            "function_definition" => node
                .child_by_field_name("declarator")
                .and_then(|decl| decl.child_by_field_name("declarator")),
            "class_specifier" | "struct_specifier" | "enum_specifier"
            | "union_specifier" | "namespace_definition" => {
                node.child_by_field_name("name")
            }
            _ => None,
        };
        if let Some(name_node) = name_node
            && let Ok(text) = name_node.utf8_text(source)
        {
            names.push(text.to_string());
        }
        cur = node.parent();
    }
    names.reverse(); // innermost-first walk → outermost-first answer
    names
}

/// The Bash enclosing-scope walk: `function_definition` (the `name`
/// field carries the function name — a `word` node). Bash has no other
/// meaningful scoping construct for navigation (no blocks/loops as
/// scopes), so a function body is the only non-empty scope; top-level
/// commands yield `[]`.
fn bash_scope_path(leaf: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = Some(leaf);
    while let Some(node) = cur {
        if node.kind() == "function_definition"
            && let Some(name_node) = node.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(source)
        {
            names.push(text.to_string());
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

    /// Languages without a node-at implementation still degrade gracefully
    /// (`node_at` → `None`, `scope_path_at` → `[]`) — 007-01's "Rust
    /// first, graceful degradation" decision now covers the languages not
    /// yet adopted (C landed in this issue; the rest follow).
    #[test]
    fn unimplemented_languages_return_none() {
        let cases: [(LanguageId, &str, &str); 5] = [
            (LanguageId::Toml, "[table]\nkey = 1\n", "table"),
            (LanguageId::Json, "{\"key\": 1}", "key"),
            (LanguageId::Yaml, "key: value\n", "key"),
            (LanguageId::Markdown, "# Heading\n", "Heading"),
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
        // `is_bash_identifier_kind`).
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


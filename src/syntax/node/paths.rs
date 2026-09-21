//! Per-language path-segment rule: which nodes are parts of a
//! larger dotted path rather than complete identifiers of their own
//! (split out of `node.rs` by 012 A2 — pure move).

use tree_sitter::Node;

use crate::syntax::registry::LanguageId;


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
pub(crate) fn is_path_segment(node: Node, lang: LanguageId) -> bool {
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
        // TOML dotted keys are path-shaped: a `dotted_key` nests
        // `bare_key`/`quoted_key`/inner `dotted_key` segments around `.`
        // tokens (in BOTH pair keys and `[table.sub]` headers — probed),
        // so `a.b.c` comes back whole as one node, same rule as the Rust
        // `::` path.
        LanguageId::Toml => parent_kind == Some("dotted_key") && node.is_named(),
        // C# dotted paths (probe-verified against the pinned
        // tree-sitter-c-sharp 0.23.5, re-pinned by the grammar-bumps
        // suite): `o.P` / `a.b.c` nest
        // `member_access_expression`s (named `expression` + `name`
        // children), and `N.Inner` / `A.B` nest `qualified_name`s
        // (named `qualifier` + `name` children; the qualifier may itself
        // be a `qualified_name`). Any NAMED child of either container is
        // a path part — same rule as the JS `member_expression`.
        LanguageId::CSharp => {
            matches!(
                parent_kind,
                Some("member_access_expression") | Some("qualified_name")
            ) && node.is_named()
        }
        // Ruby dotted paths (probe-verified against the pinned
        // tree-sitter-ruby 0.23.1): `a.b.c` nests `call`s (named
        // `receiver` + `method` children — a `call`'s receiver may
        // itself be a `call`, an `identifier`, or a `scope_resolution`),
        // and `Foo::Bar` nests `scope_resolution`s (named `scope` +
        // `name` children, both `constant`s; the scope may itself be a
        // `scope_resolution`). Any NAMED child of either container is a
        // path part — same rule as the JS `member_expression` — EXCEPT a
        // `call`'s children count only while the `call` is a genuine path
        // container (a `receiver` field AND no `arguments` field, the same
        // gate as `in_identifier_position`): otherwise the `method` child
        // of an argument-carrying call (`puts` in `puts 1`) would be
        // swallowed as a segment and resolve to nothing.
        LanguageId::Ruby => match parent_kind {
            Some("scope_resolution") => node.is_named(),
            Some("call") => node.is_named()
                && node.parent().is_some_and(|p| {
                    p.child_by_field_name("receiver").is_some()
                        && p.child_by_field_name("arguments").is_none()
                }),
            _ => false,
        }
        // Java dotted paths (probe-verified against the pinned
        // tree-sitter-java 0.23.5): `com.example.Foo` nests
        // `scoped_identifier`/`scoped_type_identifier` around `.` tokens
        // (exactly the Rust `::` shape, dot-delimited), and `A.c` /
        // `o.x` is a `field_access` (named `object` + `field` children;
        // `a.b.c` nests `field_access`es in the `object` field).
        // `method_invocation` is deliberately NOT a path container —
        // `o.m(…)` carries arguments; the bare `m` identifier comes back
        // instead (the honest degradation, same as JS/TS calls).
        LanguageId::Java => {
            matches!(
                parent_kind,
                Some("scoped_identifier")
                    | Some("scoped_type_identifier")
                    | Some("field_access")
            ) && (
                node.kind() == "."
                    || (node.is_named()
                        && matches!(
                            node.kind(),
                            "identifier"
                                | "type_identifier"
                                | "scoped_identifier"
                                | "scoped_type_identifier"
                                | "field_access"
                        ))
            )
        }
        _ => false,
    }
}

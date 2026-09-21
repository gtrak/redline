//! Enclosing-scope walks per language and the `scope_path_for`
//! dispatcher (split out of `node.rs` by 012 A2 — pure move).

use tree_sitter::Node;

use crate::syntax::registry::LanguageId;


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
pub(crate) fn scope_path_for(lang: LanguageId, leaf: Node, source: &[u8]) -> Vec<String> {
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
        LanguageId::Toml => toml_scope_path(leaf, source),
        LanguageId::Json => json_scope_path(leaf, source),
        LanguageId::Markdown => markdown_scope_path(leaf, source),
        LanguageId::Java => java_scope_path(leaf, source),
        LanguageId::CSharp => csharp_scope_path(leaf, source),
        LanguageId::Ruby => ruby_scope_path(leaf, source),
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

/// The TOML enclosing-scope walk: a `table` node (a `[a.b]` header and
/// the pairs it opens) contributes its header key — the first key child
/// (`dotted_key` for multi-segment headers, `bare_key` for single-key
/// headers; the header's children carry no field names, so the first
/// key-kind child is taken). The dotted header text is reported AS WRITTEN
/// (one scope element, e.g. `"a.b"`) — a deliberate simplification; the
/// resolver matches on this text, and splitting quoted segments is out
/// of scope for basic navigation.
fn toml_scope_path(leaf: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = Some(leaf);
    while let Some(node) = cur {
        if node.kind() == "table" {
            let header = (0..node.child_count())
                .filter_map(|i| node.child(i))
                .find(|c| matches!(c.kind(), "dotted_key" | "bare_key" | "quoted_key"));
            if let Some(header) = header
                && let Ok(text) = header.utf8_text(source)
            {
                names.push(text.to_string());
            }
        }
        cur = node.parent();
    }
    names.reverse(); // innermost-first walk → outermost-first answer
    names
}

/// The JSON enclosing-scope walk: each enclosing `pair` contributes its
/// key name (the `key` field's `string` node → its `string_content`
/// child, so the UNQUOTED name — a resolver matching `outer` must not
/// see `"outer"`), outermost → innermost. This is JSON's structural
/// "path" (there is no dotted-key syntax to walk instead).
fn json_scope_path(leaf: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = Some(leaf);
    while let Some(node) = cur {
        if node.kind() == "pair" {
            // The key's `string` node holds its unquoted name in the
            // `string_content` CHILD (kind, not field name — probed);
            // fall back to the raw `string` (quoted text) when the content
            // child is absent (broken parse / empty string).
            let key = node.child_by_field_name("key");
            let key = key
                .and_then(|k| {
                    (0..k.child_count())
                        .filter_map(|i| k.child(i))
                        .find(|c| c.kind() == "string_content")
                })
                .or(key);
            if let Some(key) = key
                && let Ok(text) = key.utf8_text(source)
            {
                names.push(text.to_string());
            }
        }
        cur = node.parent();
    }
    names.reverse(); // innermost-first walk → outermost-first answer
    names
}

/// The Markdown enclosing-scope walk (the outline): each enclosing
/// `section` (the block grammar nests sections by heading level —
/// probed) contributes its opening heading's title. The heading's
/// `heading_content` field carries the title directly in both kinds
/// (probed: an `inline` node for `atx_heading`; a `paragraph` wrapping
/// the `inline` for `setext_heading` — the field node's text is the
/// title either way; the setext `paragraph` carries a trailing newline,
/// so the title is read with it trimmed). A document with no headings
/// yields `[]`.
fn markdown_scope_path(leaf: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = Some(leaf);
    while let Some(node) = cur {
        if node.kind() == "section" {
            let title = (0..node.child_count())
                .filter_map(|i| node.child(i))
                .find(|c| matches!(c.kind(), "atx_heading" | "setext_heading"))
                .and_then(|h| h.child_by_field_name("heading_content"));
            if let Some(title) = title
                && let Ok(raw) = title.utf8_text(source)
            {
                names.push(raw.trim_end_matches('\n').to_string());
            }
        }
        cur = node.parent();
    }
    names.reverse(); // innermost-first walk → outermost-first answer
    names
}

/// The Java enclosing-scope walk (new-languages lane): `class_declaration`,
/// `interface_declaration`, `enum_declaration`, and `method_declaration`
/// (name child in the `name` field), outermost → innermost. Packages,
/// anonymous classes, and blocks are intentionally not scope items —
/// keep it simple and honest; an empty vec is the valid answer for
/// top-level code.
fn java_scope_path(leaf: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = Some(leaf);
    while let Some(node) = cur {
        if matches!(
            node.kind(),
            "class_declaration"
                | "interface_declaration"
                | "enum_declaration"
                | "method_declaration"
        ) && let Some(name_node) = node.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(source)
        {
            names.push(text.to_string());
        }
        cur = node.parent();
    }
    names.reverse(); // innermost-first walk → outermost-first answer
    names
}

/// The C# enclosing-scope walk (new-languages lane): `namespace_
/// declaration` (name child in the `name` field — a `qualified_name`
/// such as `Foo.Bar`, its text as written), `class_declaration`,
/// `interface_declaration`, `struct_declaration`, `enum_declaration`,
/// `record_declaration`, and `method_declaration` (name field),
/// outermost → innermost. Local blocks and lambdas are intentionally
/// not scope items — keep it simple and honest.
fn csharp_scope_path(leaf: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = Some(leaf);
    while let Some(node) = cur {
        if matches!(
            node.kind(),
            "namespace_declaration"
                | "class_declaration"
                | "interface_declaration"
                | "struct_declaration"
                | "enum_declaration"
                | "record_declaration"
                | "method_declaration"
        ) && let Some(name_node) = node.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(source)
        {
            names.push(text.to_string());
        }
        cur = node.parent();
    }
    names.reverse(); // innermost-first walk → outermost-first answer
    names
}

/// The Ruby enclosing-scope walk (new-languages lane): `module` and
/// `class` (name child in the `name` field — a `constant`) plus
/// `method` and `singleton_method` (name field), outermost → innermost.
/// Blocks / case arms / begin-end are intentionally not scope items —
/// keep it simple and honest.
fn ruby_scope_path(leaf: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = Some(leaf);
    while let Some(node) = cur {
        if matches!(
            node.kind(),
            "module" | "class" | "method" | "singleton_method"
        ) && let Some(name_node) = node.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(source)
        {
            names.push(text.to_string());
        }
        cur = node.parent();
    }
    names.reverse(); // innermost-first walk → outermost-first answer
    names
}

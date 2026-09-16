//! Symbol (definition) queries: one tree-sitter QUERY per language that
//! captures top-level *definitions* only — functions, methods,
//! types/structs/enums/traits/interfaces, constants, macros — plus the
//! definition item's extent (for which-function / imenu nesting).
//!
//! All tree-sitter churn lives here (plan layering rule): the grammar
//! crates, the `Language`, and the query strings are touched only in
//! this module + `registry.rs`. `nav/` and the app layer consume the
//! opaque `Symbol` records this module produces and never touch
//! tree-sitter directly.

use std::cell::RefCell;
use std::collections::HashMap;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor};

use crate::syntax::registry::LanguageId;

/// The category of a definition symbol (drives imenu indentation, the
/// short status-line tag, and which-function preference).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// A free function (`fn`, `func`, `def`, `function`).
    Function,
    /// A method (member function of a type).
    Method,
    /// A type: struct, class, interface, trait, enum, union, typedef.
    Type,
    /// A constant: `const`, `static`, top-level variable.
    Constant,
    /// A macro / macro definition.
    Macro,
    /// A markdown heading (outline entry for docs).
    Heading,
    /// A key in TOML / JSON / YAML (the "definition" of a config field).
    Key,
}

impl SymbolKind {
    /// Short tag for the status line / imenu prefix.
    pub fn tag(self) -> &'static str {
        match self {
            Self::Function => "fn",
            Self::Method => "method",
            Self::Type => "type",
            Self::Constant => "const",
            Self::Macro => "macro",
            Self::Heading => "H",
            Self::Key => "key",
        }
    }
}

/// One extracted definition. All fields are owned (no tree borrows), so a
/// `Vec<Symbol>` is safe to send across threads and store in the index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// Start line of the definition (0-based).
    pub line: usize,
    /// End line of the definition's extent (0-based, inclusive). Used for
    /// which-function (enclosing) and imenu nesting.
    pub end_line: usize,
    /// Byte offset of the name's start (for column display).
    pub start_byte: usize,
    /// Byte offset of the name's end.
    pub end_byte: usize,
}

// ── per-language definition queries ─────────────────────────────────────
// Each pattern captures `@name` (the identifier) and `@item` (the whole
// definition node, for its extent). Node names / field names were verified
// against the pinned grammar versions (see the dump tests in issue 05's
// history); `@item`'s node kind drives the `SymbolKind` via `kind_of`.

const RUST_QUERY: &str = r#"
(function_item name: (identifier) @name) @item
(struct_item name: (type_identifier) @name) @item
(enum_item name: (type_identifier) @name) @item
(trait_item name: (type_identifier) @name) @item
(mod_item name: (identifier) @name) @item
(const_item name: (identifier) @name) @item
(static_item name: (identifier) @name) @item
(macro_definition name: (identifier) @name) @item
"#;

const TYPESCRIPT_QUERY: &str = r#"
(function_declaration name: (identifier) @name) @item
(class_declaration name: (type_identifier) @name) @item
(interface_declaration name: (type_identifier) @name) @item
(type_alias_declaration name: (type_identifier) @name) @item
(method_definition name: (property_identifier) @name) @item
(program (lexical_declaration (variable_declarator name: (identifier) @name)) @item)
(program (variable_declaration (variable_declarator name: (identifier) @name)) @item)
(export_statement declaration: (lexical_declaration (variable_declarator name: (identifier) @name)) @item)
(export_statement declaration: (variable_declaration (variable_declarator name: (identifier) @name)) @item)
"#;

const JAVASCRIPT_QUERY: &str = r#"
(function_declaration name: (identifier) @name) @item
(class_declaration name: (identifier) @name) @item
(method_definition name: (property_identifier) @name) @item
(program (lexical_declaration (variable_declarator name: (identifier) @name)) @item)
(program (variable_declaration (variable_declarator name: (identifier) @name)) @item)
(export_statement declaration: (lexical_declaration (variable_declarator name: (identifier) @name)) @item)
(export_statement declaration: (variable_declaration (variable_declarator name: (identifier) @name)) @item)
"#;

const PYTHON_QUERY: &str = r#"
(function_definition name: (identifier) @name) @item
(class_definition name: (identifier) @name) @item
"#;

const GO_QUERY: &str = r#"
(function_declaration name: (identifier) @name) @item
(method_declaration name: (field_identifier) @name) @item
(type_declaration (type_spec name: (type_identifier) @name)) @item
(const_spec name: (identifier) @name) @item
(var_spec name: (identifier) @name) @item
"#;

const C_QUERY: &str = r#"
(function_definition (function_declarator (identifier) @name)) @item
(struct_specifier name: (type_identifier) @name) @item
(preproc_def name: (identifier) @name) @item
"#;

const CPP_QUERY: &str = r#"
(function_definition (function_declarator (identifier) @name)) @item
(class_specifier name: (type_identifier) @name) @item
(struct_specifier name: (type_identifier) @name) @item
"#;

const TOML_QUERY: &str = r#"
(table (bare_key) @name) @item
(table (dotted_key) @name) @item
(pair (bare_key) @name) @item
(pair (dotted_key) @name) @item
"#;

const JSON_QUERY: &str = r#"
(pair key: (string (string_content) @name)) @item
"#;

const YAML_QUERY: &str = r#"
(block_mapping_pair key: (flow_node) @name) @item
"#;

const BASH_QUERY: &str = r#"
(function_definition (word) @name) @item
"#;

const MARKDOWN_QUERY: &str = r#"
(atx_heading (inline) @name) @item
((setext_heading) @item (inline) @name)
"#;

/// The definition query for a language; `None` for plain text (the
/// documented empty fallback — plain files contribute no outline).
pub fn query_for(lang: LanguageId) -> Option<&'static str> {
    match lang {
        LanguageId::Rust => Some(RUST_QUERY),
        LanguageId::TypeScript => Some(TYPESCRIPT_QUERY),
        LanguageId::Tsx => Some(TYPESCRIPT_QUERY),
        LanguageId::JavaScript => Some(JAVASCRIPT_QUERY),
        LanguageId::Python => Some(PYTHON_QUERY),
        LanguageId::Go => Some(GO_QUERY),
        LanguageId::C => Some(C_QUERY),
        LanguageId::Cpp => Some(CPP_QUERY),
        LanguageId::Toml => Some(TOML_QUERY),
        LanguageId::Json => Some(JSON_QUERY),
        LanguageId::Yaml => Some(YAML_QUERY),
        LanguageId::Bash => Some(BASH_QUERY),
        LanguageId::Markdown => Some(MARKDOWN_QUERY),
        LanguageId::Plain => None,
    }
}

/// The `Language` (grammar) for a language id.
pub(crate) fn language_for(lang: LanguageId) -> Option<Language> {
    Some(match lang {
        LanguageId::Rust => Language::from(tree_sitter_rust::LANGUAGE),
        LanguageId::TypeScript => Language::from(tree_sitter_typescript::LANGUAGE_TYPESCRIPT),
        LanguageId::Tsx => Language::from(tree_sitter_typescript::LANGUAGE_TSX),
        LanguageId::JavaScript => Language::from(tree_sitter_javascript::LANGUAGE),
        LanguageId::Python => Language::from(tree_sitter_python::LANGUAGE),
        LanguageId::Go => Language::from(tree_sitter_go::LANGUAGE),
        LanguageId::C => Language::from(tree_sitter_c::LANGUAGE),
        LanguageId::Cpp => Language::from(tree_sitter_cpp::LANGUAGE),
        LanguageId::Toml => Language::from(tree_sitter_toml_ng::LANGUAGE),
        LanguageId::Json => Language::from(tree_sitter_json::LANGUAGE),
        LanguageId::Yaml => Language::from(tree_sitter_yaml::LANGUAGE),
        LanguageId::Bash => Language::from(tree_sitter_bash::LANGUAGE),
        LanguageId::Markdown => Language::from(tree_sitter_md::LANGUAGE),
        LanguageId::Plain => return None,
    })
}

// One parser + a per-language query cache, thread-confined. A rayon worker
// thread reuses its parser and the already-parsed queries across the (many)
// files it processes. (`thread_local!` expands to an item the doc-comment
// lint rejects, so this is a line comment.)
thread_local! {
    static TL: RefCell<ThreadLocal> = RefCell::new(ThreadLocal::new());
}

struct ThreadLocal {
    parser: Parser,
    queries: HashMap<LanguageId, Option<Query>>,
}

impl ThreadLocal {
    fn new() -> Self {
        Self {
            parser: Parser::new(),
            queries: HashMap::new(),
        }
    }
}

/// Extract the definition symbols in `source` for `lang`. Plain text and
/// an unparseable source yield an empty list. Runs on the calling thread
/// (a rayon worker during indexing); the parser and query cache are
/// thread-local so they are cheap to reuse.
pub fn extract_symbols(lang: LanguageId, source: &str) -> Vec<Symbol> {
    let query_str = match query_for(lang) {
        Some(q) => q,
        None => return Vec::new(),
    };
    let language = match language_for(lang) {
        Some(l) => l,
        None => return Vec::new(),
    };

    TL.with(|tl| {
        let mut tl = tl.borrow_mut();
        if tl.parser.set_language(&language).is_err() {
            return Vec::new();
        }
        let tree = match tl.parser.parse(source, None) {
            Some(t) => t,
            None => return Vec::new(),
        };

        // Build (and cache) the query for this language. `Query` is not
        // `Clone`, so it is stored by value and borrowed. Construct inside
        // the `or_insert_with` closure so the cache actually avoids
        // re-construction on subsequent calls.
        let cached = tl.queries
            .entry(lang)
            .or_insert_with(|| Query::new(&language, query_str).ok());
        let query = match cached.as_ref() {
            Some(q) => q,
            None => return Vec::new(),
        };

        let bytes = source.as_bytes();
        let name_idx = query.capture_index_for_name("name");
        let item_idx = query.capture_index_for_name("item");
        let mut out = Vec::new();
        let mut cursor = QueryCursor::new();
        // `matches()` yields one item per definition (with all its captures);
        // `captures()` would yield one item per capture and double the result.
        // Both are `StreamingIterator`s; `QueryMatch` is `!Send`, so extract
        // owned fields per match here.
        let mut matches = cursor.matches(query, tree.root_node(), bytes);
        while let Some(m) = matches.next() {
            let name_node = name_idx
                .and_then(|i| m.captures.iter().find(|c| c.index == i))
                .map(|c| c.node);
            let item_node = item_idx
                .and_then(|i| m.captures.iter().find(|c| c.index == i))
                .map(|c| c.node);

            let name = name_node
                .or(item_node)
                .and_then(|n| n.utf8_text(bytes).ok())
                .unwrap_or("")
                .trim()
                .to_string();
            if name.is_empty() {
                continue;
            }

            let kind_node = item_node.or(name_node).unwrap();
            let name_node = name_node.unwrap_or(kind_node);
            let extent_node = item_node.unwrap_or(name_node);
            out.push(Symbol {
                name,
                kind: kind_of(kind_node.kind()),
                line: name_node.start_position().row,
                end_line: extent_node.end_position().row,
                start_byte: name_node.start_byte(),
                end_byte: name_node.end_byte(),
            });
        }
        // Deterministic order: by line, then byte, then name.
        out.sort_by(|a, b| (a.line, a.start_byte, &a.name).cmp(&(b.line, b.start_byte, &b.name)));
        out
    })
}

/// Map a definition node's tree-sitter `kind()` string to a `SymbolKind`
/// (cross-language: the node kinds named in the per-language queries are
/// distinct enough to map in one table).
fn kind_of(kind: &str) -> SymbolKind {
    match kind {
        "function_item" | "function_definition" | "function_declaration" => SymbolKind::Function,
        "method_signature" | "method_definition" | "method_declaration" => SymbolKind::Method,
        "struct_item"
        | "enum_item"
        | "trait_item"
        | "struct_declaration"
        | "struct_specifier"
        | "class_declaration"
        | "class_specifier"
        | "class_definition"
        | "interface_declaration"
        | "type_alias_declaration"
        | "abstract_class_member_definition"
        | "type_spec"
        | "type_declaration"
        | "mod_item" => SymbolKind::Type,
        "const_item"
        | "const_spec"
        | "variable_declarator"
        | "variable_spec"
        | "var_spec"
        | "static_item"
        | "static_initializer"
        | "constant"
        | "enumerator" => SymbolKind::Constant,
        "macro_definition" | "preproc_def" => SymbolKind::Macro,
        "atx_heading" | "setext_heading" | "heading" => SymbolKind::Heading,
        // config keys (TOML / JSON / YAML) and any unclassified definition
        "pair"
        | "block_mapping_pair"
        | "table"
        | "bare_key"
        | "dotted_key"
        | "array_key"
        | "key"
        | "string_content"
        | "flow_node"
        | "declaration" => SymbolKind::Key,
        _ => SymbolKind::Function,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All (name, kind) pairs in the extracted outline, in order.
    fn names(lang: LanguageId, src: &str) -> Vec<(String, SymbolKind)> {
        extract_symbols(lang, src)
            .into_iter()
            .map(|s| (s.name, s.kind))
            .collect()
    }

    fn find<'a>(syms: &'a [Symbol], name: &str) -> Option<&'a Symbol> {
        syms.iter().find(|s| s.name == name)
    }

    // ── Rust: fn + method + struct + const ────────────────────────────
    #[test]
    fn rust_extracts_fn_method_struct_const() {
        let src = "struct Point { x: i32 }\n\
                   const TWO: i32 = 2;\n\
                   impl Point {\n\
                   \x20   fn new() -> Self { Point { x: 0 } }\n\
                   }\n\
                   fn main() {}\n";
        let syms = extract_symbols(LanguageId::Rust, src);
        let point = find(&syms, "Point").expect("struct Point");
        assert_eq!(point.kind, SymbolKind::Type);
        assert_eq!(point.line, 0);
        let two = find(&syms, "TWO").expect("const TWO");
        assert_eq!(two.kind, SymbolKind::Constant);
        assert_eq!(two.line, 1);
        let new = find(&syms, "new").expect("method new (in impl)");
        assert_eq!(new.kind, SymbolKind::Function);
        assert_eq!(new.line, 3);
        let main = find(&syms, "main").expect("fn main");
        assert_eq!(main.kind, SymbolKind::Function);
        assert_eq!(main.line, 5);
    }

    // ── TS: interface + method ─────────────────────────────────────────
    #[test]
    fn ts_extracts_interface_and_method() {
        let src = "interface Foo { bar: string }\n\
                   class A {\n\
                   \x20   baz() { return 1 }\n\
                   }\n";
        let syms = extract_symbols(LanguageId::TypeScript, src);
        let foo = find(&syms, "Foo").expect("interface Foo");
        assert_eq!(foo.kind, SymbolKind::Type);
        assert_eq!(foo.line, 0);
        let a = find(&syms, "A").expect("class A");
        assert_eq!(a.kind, SymbolKind::Type);
        let baz = find(&syms, "baz").expect("method baz");
        assert_eq!(baz.kind, SymbolKind::Method);
        assert_eq!(baz.line, 2);
    }

    // ── Python: class + def ────────────────────────────────────────────
    #[test]
    fn python_extracts_class_and_def() {
        let src = "class A:\n\
                   \x20   def m(self):\n\
                   \x20       return 1\n\n\
                   def f():\n\
                   \x20   return 1\n";
        let syms = extract_symbols(LanguageId::Python, src);
        let a = find(&syms, "A").expect("class A");
        assert_eq!(a.kind, SymbolKind::Type);
        assert_eq!(a.line, 0);
        let m = find(&syms, "m").expect("method m");
        assert_eq!(m.kind, SymbolKind::Function);
        assert_eq!(m.line, 1);
        let f = find(&syms, "f").expect("def f");
        assert_eq!(f.kind, SymbolKind::Function);
        assert_eq!(f.line, 4);
    }

    // ── Go: func + type ────────────────────────────────────────────────
    #[test]
    fn go_extracts_func_and_type() {
        let src = "type T struct{}\nfunc f() {}\n";
        let syms = extract_symbols(LanguageId::Go, src);
        let t = find(&syms, "T").expect("type T");
        assert_eq!(t.kind, SymbolKind::Type);
        assert_eq!(t.line, 0);
        let f = find(&syms, "f").expect("func f");
        assert_eq!(f.kind, SymbolKind::Function);
        assert_eq!(f.line, 1);
    }

    // ── C: function + struct ───────────────────────────────────────────
    #[test]
    fn c_extracts_function_and_struct() {
        let src = "struct S { int x; };\nint f(int a) { return a; }\n";
        let syms = extract_symbols(LanguageId::C, src);
        let s = find(&syms, "S").expect("struct S");
        assert_eq!(s.kind, SymbolKind::Type);
        assert_eq!(s.line, 0);
        let f = find(&syms, "f").expect("func f");
        assert_eq!(f.kind, SymbolKind::Function);
        assert_eq!(f.line, 1);
    }

    // ── Markdown: headings outline ─────────────────────────────────────
    #[test]
    fn markdown_extracts_headings() {
        let src = "# One\n\n## Two\n\ntext\n";
        let syms = extract_symbols(LanguageId::Markdown, src);
        let one = find(&syms, "One").expect("heading One");
        assert_eq!(one.kind, SymbolKind::Heading);
        assert_eq!(one.line, 0);
        let two = find(&syms, "Two").expect("heading Two");
        assert_eq!(two.kind, SymbolKind::Heading);
        assert_eq!(two.line, 2);
        // Exactly the two headings (no paragraph, no false positives).
        assert_eq!(syms.len(), 2, "{syms:?}");
    }

    // ── JSON: keys ─────────────────────────────────────────────────────
    #[test]
    fn json_extracts_keys() {
        let src = "{\n  \"a\": 1,\n  \"b\": {\"c\": 2}\n}\n";
        let got = names(LanguageId::Json, src);
        let got_names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
        assert!(got_names.contains(&"a"), "{got:?}");
        assert!(got_names.contains(&"b"), "{got:?}");
        assert!(got_names.contains(&"c"), "{got:?}");
        for (_, k) in &got {
            assert_eq!(*k, SymbolKind::Key);
        }
    }

    // ── TOML: table + pair keys ────────────────────────────────────────
    #[test]
    fn toml_extracts_keys() {
        let src = "[package]\nname = \"t\"\n[dep.a]\nb = 2\n";
        let got = names(LanguageId::Toml, src);
        let got_names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
        assert!(got_names.contains(&"package"), "{got:?}");
        assert!(got_names.contains(&"name"), "{got:?}");
        assert!(got_names.contains(&"dep.a"), "{got:?}");
        assert!(got_names.contains(&"b"), "{got:?}");
        for (_, k) in &got {
            assert_eq!(*k, SymbolKind::Key);
        }
    }

    // ── YAML: mapping keys ─────────────────────────────────────────────
    #[test]
    fn yaml_extracts_keys() {
        let src = "a:\n  b: 1\nc: 2\n";
        let got = names(LanguageId::Yaml, src);
        let got_names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
        assert!(got_names.contains(&"a"), "{got:?}");
        assert!(got_names.contains(&"b"), "{got:?}");
        assert!(got_names.contains(&"c"), "{got:?}");
        for (_, k) in &got {
            assert_eq!(*k, SymbolKind::Key);
        }
    }

    // ── Bash: functions ────────────────────────────────────────────────
    #[test]
    fn bash_extracts_functions() {
        let src = "#!/usr/bin/env bash\nfn() {\n  echo hi\n}\nmain() { :; }\n";
        let syms = extract_symbols(LanguageId::Bash, src);
        let fn_ = find(&syms, "fn").expect("function fn");
        assert_eq!(fn_.kind, SymbolKind::Function);
        assert_eq!(fn_.line, 1);
        let main = find(&syms, "main").expect("function main");
        assert_eq!(main.kind, SymbolKind::Function);
        assert_eq!(main.line, 4);
    }

    // ── TS/JS: exported constants (finding #7) ───────────────────────────
    #[test]
    fn ts_extracts_exported_constants() {
        let src = "export const X = 1;\nexport let Y = 2;\nexport var Z = 3;\n";
        let syms = extract_symbols(LanguageId::TypeScript, src);
        let x = find(&syms, "X").expect("export const X");
        assert_eq!(x.line, 0);
        let y = find(&syms, "Y").expect("export let Y");
        assert_eq!(y.line, 1);
        let z = find(&syms, "Z").expect("export var Z");
        assert_eq!(z.line, 2);
    }

    #[test]
    fn js_extracts_exported_constants() {
        let src = "export const A = 1;\n";
        let syms = extract_symbols(LanguageId::JavaScript, src);
        let a = find(&syms, "A").expect("export const A");
        assert_eq!(a.line, 0);
    }

    // ── C++: function + class + struct ─────────────────────────────────
    #[test]
    fn cpp_extracts_function_class_struct() {
        let src = "class A { public: void f() {} };\nstruct S { int x; };\nvoid g() {}\n";
        let syms = extract_symbols(LanguageId::Cpp, src);
        let a = find(&syms, "A").expect("class A");
        assert_eq!(a.kind, SymbolKind::Type);
        assert_eq!(a.line, 0);
        let s = find(&syms, "S").expect("struct S");
        assert_eq!(s.kind, SymbolKind::Type);
        assert_eq!(s.line, 1);
        let g = find(&syms, "g").expect("function g");
        assert_eq!(g.kind, SymbolKind::Function);
        assert_eq!(g.line, 2);
    }


    // ── Documented empty fallback: plain text ──────────────────────────
    #[test]
    fn plain_text_has_no_query_and_empty_outline() {
        // Plain has no definition query (query_for -> None), so the outline
        // is empty. This is the documented empty fallback.
        assert!(query_for(LanguageId::Plain).is_none());
        assert!(extract_symbols(LanguageId::Plain, "some text\nfn fake() {}\n").is_empty());
    }
}

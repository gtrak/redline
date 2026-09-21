//! The per-language descriptor table: the single source of truth for
//! everything that is DATA about a language — name, extensions, grammar,
//! highlight/injections/locals queries, definition query, identifier
//! kinds, token-class strategy, and the reuse policy flag. One row per
//! `LanguageId` (19 rows, Plain included).
//!
//! Before this table each of these facts was repeated in hand-synced
//! match arms across five modules: `registry.rs` (`name` / `ALL` /
//! `ext_map` / `build` / `highlight_query_for`), `queries.rs`
//! (`query_for` / `language_for`), `highlight.rs` (`supports_reuse` /
//! `reuse_language`), `node.rs` (the `parse_source` whitelist + 15
//! identifier-kind predicates), and `tokens.rs`
//! (`token_class_query_for`). Adding a language meant editing six or
//! more sites, and three of the gaps were correctness bugs: the grammar
//! was pinned in three places (a miss silently fell back to plain text),
//! the highlight query was pinned in two, and the `parse_source`
//! whitelist was a fourth identity list that disabled M-. with zero
//! compile error.
//!
//! What is NOT here is the per-language *algorithm*: the position gates
//! (`node::in_identifier_position`), the path-segment rules
//! (`node::is_path_segment`), the scope walkers, and the flat-S-expression
//! head-text gate (an algorithm over [`FlatDefineGate`]) stay in their
//! home modules. The table removes the declarative duplication only.

use tree_sitter::Language;

use crate::syntax::queries::{
    BASH_QUERY, C_SHARP_QUERY, CLOJURE_QUERY, C_QUERY, CPP_QUERY, GO_QUERY, JAVA_QUERY,
    JAVASCRIPT_QUERY, JSON_QUERY, MARKDOWN_QUERY, PYTHON_QUERY, RUBY_QUERY, RUST_QUERY,
    RUST_TABLES_QUERY, SCHEME_QUERY, TOML_QUERY, TYPESCRIPT_QUERY, YAML_QUERY,
};
use crate::syntax::registry::LanguageId;

/// The token-class (comment/string) query strategy for a row. Most rows
/// derive the token-class query from their own highlight query; a few
/// grammars need an exception (the dedicated query for the minimal
/// highlight queries, the documented inactive filter for Markdown).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenClass {
    /// Use the row's own `highlight_query` (most pinned highlight queries
    /// already capture the `comment` / `string*` faces).
    FromHighlight,
    /// A dedicated supplementary query: the pinned highlight query is
    /// minimal and captures no comment/string faces (TypeScript/TSX, C++).
    Dedicated(&'static str),
    /// The filter is inactive: the grammar has no comment or string node
    /// kinds at all (Markdown — the documented exception; hits are kept,
    /// the plain-search fallback behavior).
    Inactive,
}

/// The flat-S-expression-grammar gate: Scheme and Clojure capture every
/// two-symbol list candidate and gate on the head symbol's text
/// (`queries::flat_define_kind`), which stays an algorithm over this
/// enum rather than a `LanguageId` match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlatDefineGate {
    Scheme,
    Clojure,
}

/// One row per language — the single source of truth for everything that
/// is DATA about a language (not algorithm).
pub struct LanguageSpec {
    pub id: LanguageId,
    /// Display name (used in the store for language identification and
    /// debugging).
    pub name: &'static str,
    /// File extensions (policy quirks included verbatim: `h` → C, not
    /// Cpp; `mdx` → Markdown; `ini`/`conf` → Toml; `jsx`/`mjs`/`cjs` →
    /// JavaScript).
    pub extensions: &'static [&'static str],
    /// The grammar. `None` only for Plain. This column is the ONE
    /// grammar pin: the registry's `build`, the extraction engine, and
    /// the reuse pipeline all read it (the old three-way pin is gone).
    pub grammar: Option<fn() -> Language>,
    /// The highlight query the `HighlightConfiguration` (and the reuse /
    /// token-class pipelines) use. May point at the vendored
    /// `include_str!` consts (C#, Clojure). `None` only for Plain.
    pub highlight_query: Option<&'static str>,
    /// The injections query ("" when the language carries none).
    pub injections_query: &'static str,
    /// The locals query ("" when the language tracks no local variable
    /// scopes). C13: the incremental-reuse pipeline is byte-identical to
    /// the full `Highlighter` only when this is empty.
    pub locals_query: &'static str,
    /// The token-class (comment/string) query strategy.
    pub token_class: TokenClass,
    /// The definition query for the outline. TypeScript and TSX share
    /// one — the alias is now a visible row pair instead of a hidden
    /// match arm. `None` only for Plain.
    pub definition_query: Option<&'static str>,
    /// The Rust-only second definition query (010-01/010-03 association,
    /// struct-field, and local-binding tables); `None` elsewhere.
    pub rust_tables_query: Option<&'static str>,
    /// Scheme/Clojure: definition candidates gate on the head symbol's
    /// text (`queries::flat_define_kind`); `None` elsewhere.
    pub flat_define: Option<FlatDefineGate>,
    /// Reuse-policy flag: true for the 10 languages whose single-layer
    /// reuse pipeline is byte-identical to the full `Highlighter`
    /// (empty locals query + no injected-language layer). Policy, not
    /// data: JS/TS/TSX track locals; Java/C#/Ruby/Scheme/Clojure ride
    /// the Highlighter path deliberately; Plain has no grammar. A new
    /// language must choose consciously — the sync test cross-checks
    /// this flag against the stated policy.
    pub supports_reuse: bool,
    /// The identifier-ish node kinds `node::nearest_identifier` accepts
    /// (the old 15 hand-synced `is_*_identifier_kind` predicates are gone
    /// — this list is the data, the position/path/scope algorithms stay
    /// in `node.rs`). Empty = none (the honest N/A for Yaml, Markdown,
    /// and Plain).
    pub identifier_kinds: &'static [&'static str],
}

/// The table itself — 19 rows, in `LanguageId` variant order. Adding a
/// language is adding ONE row here (plus its query consts and grammar
/// pin in `queries.rs` / `Cargo.toml`); every consumer derives from it.
pub static LANGUAGES: [LanguageSpec; 19] = [
    LanguageSpec {
        id: LanguageId::Rust,
        name: "rust",
        extensions: &["rs", "rsi"],
        grammar: Some(|| Language::from(tree_sitter_rust::LANGUAGE)),
        highlight_query: Some(tree_sitter_rust::HIGHLIGHTS_QUERY),
        injections_query: tree_sitter_rust::INJECTIONS_QUERY,
        // C13: no locals query — the incremental pipeline is
        // byte-identical to the Highlighter for Rust.
        locals_query: "",
        token_class: TokenClass::FromHighlight,
        definition_query: Some(RUST_QUERY),
        // 010-01/010-03: the Rust-only association / struct-field /
        // local-binding tables run on the same tree.
        rust_tables_query: Some(RUST_TABLES_QUERY),
        flat_define: None,
        supports_reuse: true,
        // Probed against the pinned tree-sitter-rust NODE_TYPES: the six
        // identifier-ish kinds; `::` path segments handled by
        // `node::is_path_segment`.
        identifier_kinds: &[
            "identifier",
            "field_identifier",
            "type_identifier",
            "scoped_identifier",
            "scoped_type_identifier",
            "primitive_type",
        ],
    },
    LanguageSpec {
        id: LanguageId::TypeScript,
        name: "typescript",
        extensions: &["ts"],
        grammar: Some(|| Language::from(tree_sitter_typescript::LANGUAGE_TYPESCRIPT)),
        highlight_query: Some(tree_sitter_typescript::HIGHLIGHTS_QUERY),
        injections_query: "",
        // Local-variable tracking lives in the tree-sitter Highlighter,
        // so the reuse pipeline (single-layer) does NOT cover TS.
        locals_query: tree_sitter_typescript::LOCALS_QUERY,
        // The pinned TS highlight query captures no comment/string
        // faces — the dedicated query targets them directly
        // (`string_fragment` covers template-string content).
        token_class: TokenClass::Dedicated("(comment) @comment\n(string_fragment) @string"),
        // Shared with TSX: one definition query, two grammars.
        definition_query: Some(TYPESCRIPT_QUERY),
        rust_tables_query: None,
        flat_define: None,
        supports_reuse: false,
        // Probed against the pinned tree-sitter-typescript NODE_TYPES
        // (used for both the TS and TSX grammars).
        identifier_kinds: &[
            "identifier",
            "property_identifier",
            "type_identifier",
            "member_expression",
            "nested_type_identifier",
        ],
    },
    LanguageSpec {
        id: LanguageId::Tsx,
        name: "tsx",
        extensions: &["tsx"],
        grammar: Some(|| Language::from(tree_sitter_typescript::LANGUAGE_TSX)),
        highlight_query: Some(tree_sitter_typescript::HIGHLIGHTS_QUERY),
        injections_query: "",
        locals_query: tree_sitter_typescript::LOCALS_QUERY,
        token_class: TokenClass::Dedicated("(comment) @comment\n(string_fragment) @string"),
        // The TS/TSX aliasing, made visible: TSX borrows TS's
        // definition query and identifier kinds (one row pair).
        definition_query: Some(TYPESCRIPT_QUERY),
        rust_tables_query: None,
        flat_define: None,
        supports_reuse: false,
        identifier_kinds: &[
            "identifier",
            "property_identifier",
            "type_identifier",
            "member_expression",
            "nested_type_identifier",
        ],
    },
    LanguageSpec {
        id: LanguageId::JavaScript,
        name: "javascript",
        extensions: &["js", "jsx", "mjs", "cjs"],
        grammar: Some(|| Language::from(tree_sitter_javascript::LANGUAGE)),
        highlight_query: Some(tree_sitter_javascript::HIGHLIGHT_QUERY),
        injections_query: tree_sitter_javascript::INJECTIONS_QUERY,
        locals_query: tree_sitter_javascript::LOCALS_QUERY,
        token_class: TokenClass::FromHighlight,
        definition_query: Some(JAVASCRIPT_QUERY),
        rust_tables_query: None,
        flat_define: None,
        supports_reuse: false,
        // Probed against the pinned tree-sitter-javascript NODE_TYPES:
        // `property_identifier` is JS's name-leaf kind;
        // `type_identifier` / `nested_type_identifier` do NOT exist in
        // the JS grammar (TypeScript-only kinds).
        identifier_kinds: &["identifier", "property_identifier", "member_expression"],
    },
    LanguageSpec {
        id: LanguageId::Python,
        name: "python",
        extensions: &["py", "pyi"],
        grammar: Some(|| Language::from(tree_sitter_python::LANGUAGE)),
        highlight_query: Some(tree_sitter_python::HIGHLIGHTS_QUERY),
        injections_query: "",
        locals_query: "",
        token_class: TokenClass::FromHighlight,
        definition_query: Some(PYTHON_QUERY),
        rust_tables_query: None,
        flat_define: None,
        supports_reuse: true,
        identifier_kinds: &["identifier", "attribute"],
    },
    LanguageSpec {
        id: LanguageId::Go,
        name: "go",
        extensions: &["go"],
        grammar: Some(|| Language::from(tree_sitter_go::LANGUAGE)),
        highlight_query: Some(tree_sitter_go::HIGHLIGHTS_QUERY),
        injections_query: "",
        locals_query: "",
        token_class: TokenClass::FromHighlight,
        definition_query: Some(GO_QUERY),
        rust_tables_query: None,
        flat_define: None,
        supports_reuse: true,
        identifier_kinds: &[
            "identifier",
            "field_identifier",
            "type_identifier",
            "selector_expression",
            "qualified_type",
        ],
    },
    LanguageSpec {
        id: LanguageId::C,
        name: "c",
        // Policy quirk: `h` resolves to C, not Cpp.
        extensions: &["c", "h"],
        grammar: Some(|| Language::from(tree_sitter_c::LANGUAGE)),
        highlight_query: Some(tree_sitter_c::HIGHLIGHT_QUERY),
        injections_query: "",
        locals_query: "",
        token_class: TokenClass::FromHighlight,
        definition_query: Some(C_QUERY),
        rust_tables_query: None,
        flat_define: None,
        supports_reuse: true,
        // Probed against the pinned tree-sitter-c NODE_TYPES:
        // `identifier` (values), `field_identifier` (struct members),
        // `type_identifier` (struct/enum/typedef names),
        // `primitive_type` (`int`, …), and `field_expression` (the whole
        // `a.b` / `p->x` member access).
        identifier_kinds: &[
            "identifier",
            "field_identifier",
            "type_identifier",
            "primitive_type",
            "field_expression",
        ],
    },
    LanguageSpec {
        id: LanguageId::Cpp,
        name: "cpp",
        extensions: &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
        grammar: Some(|| Language::from(tree_sitter_cpp::LANGUAGE)),
        highlight_query: Some(tree_sitter_cpp::HIGHLIGHT_QUERY),
        injections_query: "",
        locals_query: "",
        // The pinned C++ highlight query captures no comment/string
        // faces — the dedicated query targets the grammar's
        // `string_literal` / `char_literal` kinds too.
        token_class: TokenClass::Dedicated(
            "(comment) @comment\n(string_literal) @string\n(char_literal) @string",
        ),
        definition_query: Some(CPP_QUERY),
        rust_tables_query: None,
        flat_define: None,
        supports_reuse: true,
        // The C set plus `namespace_identifier` (a `ns::` scope name)
        // and `qualified_identifier` (the whole `A::x` / `ns::A::x`
        // path).
        identifier_kinds: &[
            "identifier",
            "field_identifier",
            "type_identifier",
            "primitive_type",
            "field_expression",
            "namespace_identifier",
            "qualified_identifier",
        ],
    },
    LanguageSpec {
        id: LanguageId::Toml,
        name: "toml",
        // Policy quirks: `ini` / `conf` resolve to TOML.
        extensions: &["toml", "ini", "conf"],
        grammar: Some(|| Language::from(tree_sitter_toml_ng::LANGUAGE)),
        highlight_query: Some(tree_sitter_toml_ng::HIGHLIGHTS_QUERY),
        injections_query: "",
        locals_query: "",
        token_class: TokenClass::FromHighlight,
        definition_query: Some(TOML_QUERY),
        rust_tables_query: None,
        flat_define: None,
        supports_reuse: true,
        // Probed against the pinned tree-sitter-toml-ng NODE_TYPES:
        // `bare_key` / `quoted_key` (the key leaves), `dotted_key` (the
        // whole `a.b.c` path).
        identifier_kinds: &["bare_key", "quoted_key", "dotted_key"],
    },
    LanguageSpec {
        id: LanguageId::Json,
        name: "json",
        extensions: &["json", "jsonc"],
        grammar: Some(|| Language::from(tree_sitter_json::LANGUAGE)),
        highlight_query: Some(tree_sitter_json::HIGHLIGHTS_QUERY),
        injections_query: "",
        locals_query: "",
        token_class: TokenClass::FromHighlight,
        definition_query: Some(JSON_QUERY),
        rust_tables_query: None,
        flat_define: None,
        supports_reuse: true,
        // `string` — but ONLY in a `pair`'s `key` field (position-gated
        // by `node::in_identifier_position`); a value `string` is data.
        identifier_kinds: &["string"],
    },
    LanguageSpec {
        id: LanguageId::Yaml,
        name: "yaml",
        extensions: &["yaml", "yml"],
        grammar: Some(|| Language::from(tree_sitter_yaml::LANGUAGE)),
        highlight_query: Some(tree_sitter_yaml::HIGHLIGHTS_QUERY),
        injections_query: "",
        locals_query: "",
        token_class: TokenClass::FromHighlight,
        definition_query: Some(YAML_QUERY),
        rust_tables_query: None,
        flat_define: None,
        supports_reuse: true,
        // No identifier-ish kind: Yaml is intentionally unadopted for
        // node-at (its "key" shapes are too loose); M-. degrades.
        identifier_kinds: &[],
    },
    LanguageSpec {
        id: LanguageId::Bash,
        name: "bash",
        extensions: &["sh", "bash", "zsh"],
        grammar: Some(|| Language::from(tree_sitter_bash::LANGUAGE)),
        highlight_query: Some(tree_sitter_bash::HIGHLIGHT_QUERY),
        injections_query: "",
        locals_query: "",
        token_class: TokenClass::FromHighlight,
        definition_query: Some(BASH_QUERY),
        rust_tables_query: None,
        flat_define: None,
        supports_reuse: true,
        // Probed against the pinned tree-sitter-bash NODE_TYPES: there
        // is no `identifier` kind — `command_name` for command names
        // and `variable_name` for variables. Plain `word`s
        // (arguments, function names) are deliberately NOT
        // identifier-ish — a bare M-. context on an arbitrary word is
        // noise.
        identifier_kinds: &["command_name", "variable_name"],
    },
    LanguageSpec {
        id: LanguageId::Markdown,
        name: "markdown",
        // Policy quirk: `mdx` resolves to Markdown.
        extensions: &["md", "markdown", "mdx"],
        grammar: Some(|| Language::from(tree_sitter_md::LANGUAGE)),
        highlight_query: Some(tree_sitter_md::HIGHLIGHT_QUERY_BLOCK),
        injections_query: tree_sitter_md::INJECTION_QUERY_BLOCK,
        locals_query: "",
        // Documented exception: the Markdown grammar has no comment or
        // string node kinds at all — the token filter stays inactive
        // (hits are kept, the plain-search fallback behavior).
        token_class: TokenClass::Inactive,
        definition_query: Some(MARKDOWN_QUERY),
        rust_tables_query: None,
        flat_define: None,
        supports_reuse: true,
        // No identifier-ish kind (probed against the pinned
        // tree-sitter-md 0.5.1 block grammar): a heading's title is an
        // `inline` node spanning whole paragraphs and code spans alike —
        // treating it identifier-ish would make `node_at` resolve on
        // arbitrary prose. What IS meaningful is the outline
        // (`markdown_scope_path` reports the enclosing heading chain).
        identifier_kinds: &[],
    },
    LanguageSpec {
        id: LanguageId::Java,
        name: "java",
        extensions: &["java"],
        grammar: Some(|| Language::from(tree_sitter_java::LANGUAGE)),
        highlight_query: Some(tree_sitter_java::HIGHLIGHTS_QUERY),
        injections_query: "",
        locals_query: "",
        token_class: TokenClass::FromHighlight,
        definition_query: Some(JAVA_QUERY),
        rust_tables_query: None,
        flat_define: None,
        // Rides the Highlighter path deliberately (new-languages lane).
        supports_reuse: false,
        // Probed against the pinned tree-sitter-java 0.23.5 NODE_TYPES:
        // `identifier` (values), `type_identifier` (type names),
        // `scoped_identifier` / `scoped_type_identifier` (the whole
        // `a.b.c` path in value / type position), and `field_access`
        // (the whole `o.x` member access). There is no
        // `field_identifier` kind in this grammar.
        identifier_kinds: &[
            "identifier",
            "type_identifier",
            "scoped_identifier",
            "scoped_type_identifier",
            "field_access",
        ],
    },
    LanguageSpec {
        id: LanguageId::CSharp,
        name: "csharp",
        extensions: &["cs"],
        grammar: Some(|| Language::from(tree_sitter_c_sharp::LANGUAGE)),
        // The pinned crate ships `queries/highlights.scm` but does not
        // export a `HIGHLIGHTS_QUERY` constant, so the query is vendored
        // verbatim (checksum-pinned at copy time) below.
        highlight_query: Some(C_SHARP_HIGHLIGHTS),
        injections_query: "",
        locals_query: "",
        token_class: TokenClass::FromHighlight,
        definition_query: Some(C_SHARP_QUERY),
        rust_tables_query: None,
        flat_define: None,
        // Rides the Highlighter path deliberately.
        supports_reuse: false,
        // Probed against the pinned tree-sitter-c-sharp 0.23.5
        // NODE_TYPES: this grammar has no `type_identifier` (type names
        // are plain `identifier`s); `predefined_type` (`int`, `string`,
        // …) is C's `primitive_type` analog; `member_access_expression`
        // and `qualified_name` are the whole-path kinds.
        identifier_kinds: &[
            "identifier",
            "predefined_type",
            "member_access_expression",
            "qualified_name",
        ],
    },
    LanguageSpec {
        id: LanguageId::Ruby,
        name: "ruby",
        extensions: &["rb"],
        grammar: Some(|| Language::from(tree_sitter_ruby::LANGUAGE)),
        highlight_query: Some(tree_sitter_ruby::HIGHLIGHTS_QUERY),
        injections_query: "",
        locals_query: tree_sitter_ruby::LOCALS_QUERY,
        token_class: TokenClass::FromHighlight,
        definition_query: Some(RUBY_QUERY),
        rust_tables_query: None,
        flat_define: None,
        // Local-variable tracking lives in the tree-sitter Highlighter.
        supports_reuse: false,
        // Probed against the pinned tree-sitter-ruby 0.23.1 NODE_TYPES:
        // `constant` (type / class names, bare or in a
        // `scope_resolution` chain), `identifier` (method names, local
        // variables, bare calls), `instance_variable` (`@x`), `call`
        // (the whole `a.b` member chain — position-gated by
        // `node::in_identifier_position`), and `scope_resolution` (the
        // whole `Foo::Bar` path).
        identifier_kinds: &[
            "constant",
            "identifier",
            "instance_variable",
            "call",
            "scope_resolution",
        ],
    },
    LanguageSpec {
        id: LanguageId::Scheme,
        name: "scheme",
        // Scheme (the lisp-family landing — see docs/language-coverage.md
        // gap 8 for the family choice rationale).
        extensions: &["scm", "ss", "sls", "sld"],
        grammar: Some(|| Language::from(tree_sitter_scheme::LANGUAGE)),
        highlight_query: Some(tree_sitter_scheme::HIGHLIGHTS_QUERY),
        injections_query: "",
        locals_query: "",
        token_class: TokenClass::FromHighlight,
        definition_query: Some(SCHEME_QUERY),
        rust_tables_query: None,
        // The flat S-expression grammar: definitions gate on the head
        // symbol's text (`queries::flat_define_kind`).
        flat_define: Some(FlatDefineGate::Scheme),
        // Rides the Highlighter path deliberately.
        supports_reuse: false,
        // Probed against the pinned tree-sitter-scheme 0.24.7
        // NODE_TYPES: `symbol` — the flat S-expression grammar's ONLY
        // name kind. There is no path-shaped construct in the grammar
        // (Lisp has no dotted paths; module paths like `(foo core)` are
        // `list`s, not path containers), so `node::is_path_segment` has
        // no Scheme arm and `scope_path_at` stays `[]` (the honest N/A,
        // pinned by `scheme_has_no_path_or_scope`).
        identifier_kinds: &["symbol"],
    },
    LanguageSpec {
        id: LanguageId::Clojure,
        name: "clojure",
        // The runtime-bump lane — the first 0.25-generation grammar.
        extensions: &["clj", "cljs", "cljc"],
        grammar: Some(|| Language::from(tree_sitter_clojure::LANGUAGE)),
        // The pinned crate ships `grammar-src/queries/highlights.scm`
        // but exports no highlights constant, so the query is vendored
        // verbatim (sha256-pinned at copy time) below — the same pattern
        // as C#.
        highlight_query: Some(CLOJURE_HIGHLIGHTS),
        injections_query: "",
        locals_query: "",
        token_class: TokenClass::FromHighlight,
        definition_query: Some(CLOJURE_QUERY),
        rust_tables_query: None,
        flat_define: Some(FlatDefineGate::Clojure),
        // Rides the Highlighter path deliberately.
        supports_reuse: false,
        // Probed against the pinned tree-sitter-clojure 0.1.0
        // NODE_TYPES: `sym_lit` — the flat S-expression grammar's name
        // kind (a bare `foo`, a namespaced `ns.var/foo` — the `.` and
        // `/` are INSIDE the single `sym_name` leaf, probe-verified —
        // and a meta-prefixed name all parse as one `sym_lit`).
        // Keywords (`kwd_lit`) are deliberately not identifier-ish: a
        // keyword names a key, not a var.
        identifier_kinds: &["sym_lit"],
    },
    LanguageSpec {
        id: LanguageId::Plain,
        name: "plain",
        // The fallback: unknown extensions resolve here. No grammar, no
        // queries, no identifier kinds.
        extensions: &[],
        grammar: None,
        highlight_query: None,
        injections_query: "",
        locals_query: "",
        token_class: TokenClass::Inactive,
        definition_query: None,
        rust_tables_query: None,
        flat_define: None,
        supports_reuse: false,
        identifier_kinds: &[],
    },
];

/// The row for `lang` — exhaustive over `LanguageId` by construction
/// (a new variant fails to compile here until its row lands).
pub fn spec(lang: LanguageId) -> &'static LanguageSpec {
    // Row order matches the `LanguageId` variant order (pinned by the
    // sync test).
    match lang {
        LanguageId::Rust => &LANGUAGES[0],
        LanguageId::TypeScript => &LANGUAGES[1],
        LanguageId::Tsx => &LANGUAGES[2],
        LanguageId::JavaScript => &LANGUAGES[3],
        LanguageId::Python => &LANGUAGES[4],
        LanguageId::Go => &LANGUAGES[5],
        LanguageId::C => &LANGUAGES[6],
        LanguageId::Cpp => &LANGUAGES[7],
        LanguageId::Toml => &LANGUAGES[8],
        LanguageId::Json => &LANGUAGES[9],
        LanguageId::Yaml => &LANGUAGES[10],
        LanguageId::Bash => &LANGUAGES[11],
        LanguageId::Markdown => &LANGUAGES[12],
        LanguageId::Java => &LANGUAGES[13],
        LanguageId::CSharp => &LANGUAGES[14],
        LanguageId::Ruby => &LANGUAGES[15],
        LanguageId::Scheme => &LANGUAGES[16],
        LanguageId::Clojure => &LANGUAGES[17],
        LanguageId::Plain => &LANGUAGES[18],
    }
}

/// True when the language has a grammar in the table — the derived
/// replacement for `node::parse_source`'s old hand-synced 17-language
/// whitelist: every grammar-bearing row is parseable automatically, and
/// adding a language can no longer leave M-. silently dead.
pub fn parseable(lang: LanguageId) -> bool {
    spec(lang).grammar.is_some()
}

/// The C# highlight query — vendored VERBATIM from `tree-sitter-c-sharp`
/// 0.23.5's `queries/highlights.scm` (the crate ships the file but does
/// not export a `HIGHLIGHTS_QUERY` constant — its binding is commented
/// out in `bindings/rust/lib.rs`). Copy lives in
/// `third_party/tree-sitter-c-sharp-0.23.5/highlights.scm` (sha256 at
/// vendor time: ab8a9930aeeee70fa2dbfde82e4763170b7e826bc642338ad0683772c20c060f;
/// the 0.23.5 copy adds the `..` range operator to the operator list —
/// the only content change vs the 0.23.1 copy); keep it in lockstep with
/// the pinned crate version — do not edit or re-flow.
pub const C_SHARP_HIGHLIGHTS: &str =
    include_str!("../../third_party/tree-sitter-c-sharp-0.23.5/highlights.scm");

/// The Clojure highlight query — vendored VERBATIM from
/// `tree-sitter-clojure` 0.1.0's `grammar-src/queries/highlights.scm`
/// (the crate exports only `LANGUAGE` / `NODE_TYPES` — no highlights
/// constant). Copy lives in
/// `third_party/tree-sitter-clojure-0.1.0/highlights.scm` (sha256 at
/// vendor time: 424b3b60f43cbb008c8d87730845855e0c1dde657f1a6f2e1408caf4f16914de);
/// keep it in lockstep with the pinned crate version — do not edit or
/// re-flow.
pub const CLOJURE_HIGHLIGHTS: &str =
    include_str!("../../third_party/tree-sitter-clojure-0.1.0/highlights.scm");

#[cfg(test)]
mod tests {
    use super::*;

    /// The sync test: the table must agree with itself and with the
    /// extension resolver, and the reuse/local policy must agree with the
    /// stated policy. A wrong row (missing grammar/definition query,
    /// duplicate extension, stale resolver entry, or a `supports_reuse`
    /// flipped against the stated policy) fails here.
    #[test]
    fn language_table_rows_are_complete_and_consistent() {
        use std::collections::HashSet;

        // Exactly one row per `LanguageId`, and `spec()` points each row
        // back at itself (the match indices agree with the row order).
        let mut seen = HashSet::new();
        for (i, row) in LANGUAGES.iter().enumerate() {
            assert!(
                seen.insert(row.id),
                "duplicate table row for {id:?} at index {i}",
                id = row.id
            );
            assert!(
                std::ptr::eq(spec(row.id), row),
                "spec() does not point at LANGUAGES[{i}] for {id:?}",
                id = row.id
            );
        }
        assert_eq!(seen.len(), 19, "expected one row per LanguageId variant");

        // Every non-Plain row carries a name, at least one extension, a
        // grammar, a highlight query, and a definition query.
        for row in &LANGUAGES {
            if row.id == LanguageId::Plain {
                assert!(row.extensions.is_empty());
                assert!(row.grammar.is_none());
                assert!(row.highlight_query.is_none());
                assert!(row.definition_query.is_none());
                assert!(row.rust_tables_query.is_none());
                assert!(row.flat_define.is_none());
                assert!(matches!(row.token_class, TokenClass::Inactive));
                assert!(!row.supports_reuse);
                continue;
            }
            assert!(!row.name.is_empty(), "{:?}: empty name", row.id);
            assert!(
                !row.extensions.is_empty(),
                "{:?}: no extensions",
                row.id
            );
            assert!(row.grammar.is_some(), "{:?}: no grammar", row.id);
            assert!(
                row.highlight_query.is_some(),
                "{:?}: no highlight query",
                row.id
            );
            assert!(
                row.definition_query.is_some(),
                "{:?}: no definition query",
                row.id
            );
        }

        // Extensions are unique across rows and round-trip through the
        // resolver (a stale `ext_map` entry, a duplicate, or a missing
        // extension fails here).
        let mut exts = HashSet::new();
        for row in &LANGUAGES {
            for ext in row.extensions {
                assert!(
                    exts.insert(*ext),
                    "extension `{ext}` claimed by more than one row"
                );
                assert_eq!(
                    crate::syntax::registry::resolve_language(&format!("f.{ext}")),
                    row.id,
                    "resolver does not map `{ext}` back to {id:?}",
                    id = row.id
                );
            }
        }

        // Reuse/local cross-check (the stated policy): for every non-Plain
        // row, `supports_reuse` is true exactly when the locals query is
        // empty AND the language is not one of the Highlighter-path
        // languages (Java/C#/Ruby/Scheme/Clojure — local-variable or
        // minimal-grammar tracking stays in the full Highlighter).
        for row in &LANGUAGES {
            if row.id == LanguageId::Plain {
                continue;
            }
            let stated_policy = row.locals_query.is_empty()
                && !matches!(
                    row.id,
                    LanguageId::Java
                        | LanguageId::CSharp
                        | LanguageId::Ruby
                        | LanguageId::Scheme
                        | LanguageId::Clojure
                );
            assert_eq!(
                row.supports_reuse, stated_policy,
                "{:?}: supports_reuse disagrees with the stated policy \
                 (locals_query empty and not a Highlighter-path language)",
                row.id
            );
        }
    }

    // ── F-8: vendored highlight query integrity ─────────────────────────
    /// The vendored `include_str!` files must not have been locally edited.
    /// This test hashes both `third_party/*/highlights.scm` files and
    /// compares against the sha256 recorded at copy time (the same hashes
    /// in the doc comments on `C_SHARP_HIGHLIGHTS` and
    /// `CLOJURE_HIGHLIGHTS`). A local edit to either file fails here.
    #[test]
    fn vendored_highlight_queries_match_copy_time_sha256() {
        use std::process::Command;

        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

        // Copy-time sha256 (recorded at vendor time — do not update these
        // without re-verifying the source crate version).
        let c_sharp_expected =
            "ab8a9930aeeee70fa2dbfde82e4763170b7e826bc642338ad0683772c20c060f";
        let clojure_expected =
            "424b3b60f43cbb008c8d87730845855e0c1dde657f1a6f2e1408caf4f16914de";

        let c_sharp_path =
            manifest_dir.join("third_party/tree-sitter-c-sharp-0.23.5/highlights.scm");
        let clojure_path =
            manifest_dir.join("third_party/tree-sitter-clojure-0.1.0/highlights.scm");

        let hash_file = |path: &std::path::Path| -> String {
            let out = Command::new("sha256sum")
                .arg(path)
                .output()
                .expect("sha256sum failed");
            String::from_utf8(out.stdout)
                .expect("sha256sum output is not UTF-8")
                .split_whitespace()
                .next()
                .expect("no hash in sha256sum output")
                .to_string()
        };

        assert_eq!(
            hash_file(&c_sharp_path),
            c_sharp_expected,
            "C# highlights.scm has been modified since vendor time"
        );
        assert_eq!(
            hash_file(&clojure_path),
            clojure_expected,
            "Clojure highlights.scm has been modified since vendor time"
        );
    }
}

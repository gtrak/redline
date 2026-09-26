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

use crate::queries::{
    BASH_QUERY, C_SHARP_QUERY, CLOJURE_QUERY, C_QUERY, CPP_QUERY, GO_QUERY, JAVA_QUERY,
    JAVASCRIPT_QUERY, MARKDOWN_QUERY, PYTHON_QUERY, RUBY_QUERY, RUST_QUERY,
    RUST_TABLES_QUERY, SCHEME_QUERY, TOML_QUERY, TYPESCRIPT_QUERY,
};
use crate::registry::LanguageId;

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

/// Per-language WORD CONSTITUENTS (issue-language-aware-symbols): the set
/// of chars that form one symbol/word for the word motions (M-f/M-b),
/// symbol extraction (M-? / M-.), the ripgrep word-boundary search sink,
/// and the kill-word walks. The historical crate-wide rule
/// (`alphanumeric || _`) is the `Default` row — it was never a language
/// fact, and the reported bug (a Clojure `jwks/fetch-issuer-info` split
/// into three tokens by the global rule) is fixed by reading each
/// language's own symbol alphabet from its pinned grammar instead of
/// assuming one for all.
///
/// The audit table — every language this registry highlights, its
/// symbol-constituent rule, the pinned-grammar evidence, and whether the
/// historical rule was already correct:
///
/// | Language        | Rule     | Grammar/reader evidence (pinned probe: `tests/probe_langsym.rs`)                          | Historical rule |
/// |-----------------|----------|--------------------------------------------------------------------------------------------|-----------------|
/// | Rust            | Default  | identifiers are alnum/`_` leaves; a lifetime `'a` is a separate `lifetime` node, a char literal separate — alnum+`_` is already right | unchanged |
/// | Python          | Default  | identifier = alnum/`_` (a leading digit is illegal but no char-set rule can express it)    | unchanged |
/// | Go              | Default  | identifier = alnum/`_`                                                                      | unchanged |
/// | C / Cpp         | Default  | identifier = alnum/`_` (`-` is arithmetic — stays a boundary)                               | unchanged |
/// | Java / CSharp   | Default  | identifier = alnum/`_`                                                                      | unchanged |
/// | Bash            | Default  | a `$` is a variable-marker prefix, not an identifier char (probe: `$var` is a `variable_name` node AFTER the `$`) | unchanged |
/// | Json / Yaml /   | Default  | keys are data (string / plain scalar), not navigable symbols — the historical rule is the  | unchanged |
/// | Markdown / Plain|          | honest plain-text behavior                                                                  |                 |
/// | JavaScript /    | Dollar   | probe: tree-sitter-javascript 0.25.0 parses `$foo`, `bar$`, `$bar` each as ONE `identifier` | `$foo` WAS split |
/// | TypeScript / TSX|          | node (`$` IS an identifier char; the historical rule rejected it)                          | by the rule |
/// | Ruby            | Suffix   | probe: tree-sitter-ruby 0.23.1 parses `empty?` / `save!` as ONE `identifier` node; the      | `empty?` WAS split |
/// |                 |          | ternary `?` in `1 ? 2 : 3` stays its own `?` node (whitespace-separated), so `-`/`?`/`!`    | by the rule |
/// |                 |          | arithmetic still split by whitespace                                                        |                 |
/// | Scheme          | Lisp     | probe: tree-sitter-scheme 0.24.7 parses `fetch-issuer-info`, `a/b`, `c.d`, `:e` each as ONE  | `bar-baz` WAS split |
/// |                 |          | `symbol` node (the quote `'` is a separate `quote` node, so it stays non-word)              | by the rule |
/// | Clojure         | Lisp     | probe: tree-sitter-clojure 0.1.0 parses `jwks/fetch-issuer-info` as ONE `sym_lit` (children  | the reported bug: |
/// |                 |          | `sym_ns` + `/` + `sym_name`), `::jwks/local` as ONE `kwd_lit` (children `::` + `kwd_ns` +    | three tokens |
/// |                 |          | `kwd_name`), and a bare hyphenated `helper-x` as ONE `sym_name` leaf. The symbol alphabet   |                 |
/// |                 |          | is the Clojure/Scheme reader's `[a-zA-Z0-9*+!?:_.-/]` — letters, digits, `* + ! - _ ' ? < > |                 |
/// |                 |          | =`, `.` (the Scheme probe shows the `.` inside one `symbol`), `/` (namespace separator), and |                 |
/// |                 |          | `:` (keyword marker / auto-resolve `::`). The quote `'` is a SEPARATE form in both (probe:   |                 |
/// |                 |          | `quoting_lit` / `quote` hold their own `'` leaf) — it stays a boundary, not a constituent.   |                 |
/// | Toml            | BareKey  | probe: tree-sitter-toml-ng 0.7.0 parses the bare keys `a-b` and `key-x` each as ONE          | `key-x` WAS split |
/// |                 |          | `bare_key` node — `-` is a bare-key constituent in the TOML spec (not an operator)           | by the rule |
///
/// The CRUX both directions: `-` REMAINS a boundary in every language where
/// it is an operator (Rust / Go / JS / C / Cpp / Java / C# / Ruby / Python /
/// Bash / Markdown / JSON / YAML / Plain) — arithmetic like `a-b` must keep
/// splitting. Only the grammars that prove `-` inside a name (Scheme /
/// Clojure / TOML) absorb it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordRule {
    /// Alphanumeric + `_` (the historical crate-wide rule; already correct
    /// for Rust, Python, Go, C, Cpp, Java, C#, Bash, and the data/plain rows).
    Default,
    /// Alphanumeric + `_`, plus `$` (JavaScript / TypeScript / TSX — a
    /// valid identifier char the historical rule rejected: `$foo` split
    /// into `$` and `foo`).
    Dollar,
    /// Alphanumeric + `_`, plus the `?` `!` method-name suffixes (Ruby
    /// `empty?` / `save!` — one `identifier` node in the pinned grammar;
    /// the ternary `?` stays a separate whitespace-separated node, so
    /// `a ? b : c` still splits on whitespace).
    Suffix,
    /// Alphanumeric + `_`, plus the Lisp reader's symbol alphabet
    /// `* + ! - ? < > = . / :` (Scheme / Clojure — probe:
    /// `jwks/fetch-issuer-info` and `fetch-issuer-info` each parse as ONE
    /// grammar node; the quote `'` is a separate form, see the audit
    /// table).
    Lisp,
    /// Alphanumeric + `_`, plus `-` (TOML bare keys — probe: `a-b`
    /// parses as one `bare_key`).
    BareKey,
}

impl WordRule {
    /// Whether `c` is a word constituent under this rule.
    pub fn contains(self, c: char) -> bool {
        if c.is_alphanumeric() || c == '_' {
            return true;
        }
        match self {
            WordRule::Default => false,
            WordRule::Dollar => c == '$',
            WordRule::Suffix => matches!(c, '?' | '!'),
            WordRule::Lisp => matches!(c, '*' | '+' | '!' | '-' | '?' | '<' | '>' | '=' | '.' | '/' | ':'),
            WordRule::BareKey => c == '-',
        }
    }
}

/// The per-language word-constituent test: `is_word_char(Clojure, '-')`
/// is `true`, `is_word_char(Rust, '-')` is `false`. Every consumer that
/// holds a buffer (word motion, symbol extraction, the ripgrep
/// word-boundary sink, kill-word) consults this with the buffer's
/// `LanguageId` — the historical global rule survives only as
/// `WordRule::Default` (the Plain / no-language fallback).
pub fn is_word_char(lang: LanguageId, c: char) -> bool {
    spec(lang).word_rule.contains(c)
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
    /// match arm. `None` for Plain, JSON, and YAML (config/data formats
    /// whose keys are not navigation targets — issue-json-yaml-no-symbols).
    pub definition_query: Option<&'static str>,
    /// The Rust-only second definition query (010-01/010-03 association,
    /// struct-field, and local-binding tables); `None` elsewhere.
    pub rust_tables_query: Option<&'static str>,
    /// Scheme/Clojure: definition candidates gate on the head symbol's
    /// text (`queries::flat_define_kind`); `None` elsewhere.
    pub flat_define: Option<FlatDefineGate>,
    /// Per-language word constituents (issue-language-aware-symbols) —
    /// see the [`WordRule`] audit table. Every consumer that holds a
    /// buffer consults this row instead of assuming the global rule.
    pub word_rule: WordRule,
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
        word_rule: WordRule::Default,
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
        // redline's own superset query (upstream TS highlights + the
        // locals/properties/calls/literals the grammar supports but the
        // pinned crate's 35-line query omits — issue-lang-highlight-parity).
        highlight_query: Some(TYPESCRIPT_HIGHLIGHTS),
        injections_query: "",
        // Local-variable tracking lives in the tree-sitter Highlighter,
        // so the reuse pipeline (single-layer) does NOT cover TS.
        locals_query: tree_sitter_typescript::LOCALS_QUERY,
        // The token-class filter targets `string_fragment` (the
        // template-string CONTENT, quote-free) directly — a finer
        // target than the highlight query's whole-node string capture,
        // so it stays a dedicated query.
        token_class: TokenClass::Dedicated("(comment) @comment\n(string_fragment) @string"),
        // Shared with TSX: one definition query, two grammars.
        definition_query: Some(TYPESCRIPT_QUERY),
        rust_tables_query: None,
        flat_define: None,
        word_rule: WordRule::Dollar,
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
        // Shared with TypeScript: the same superset query is valid
        // against the TSX grammar (every node kind it uses exists in
        // both grammars' node-types; the load smoke test pins this).
        highlight_query: Some(TYPESCRIPT_HIGHLIGHTS),
        injections_query: "",
        locals_query: tree_sitter_typescript::LOCALS_QUERY,
        token_class: TokenClass::Dedicated("(comment) @comment\n(string_fragment) @string"),
        // The TS/TSX aliasing, made visible: TSX borrows TS's
        // definition query and identifier kinds (one row pair).
        definition_query: Some(TYPESCRIPT_QUERY),
        rust_tables_query: None,
        flat_define: None,
        word_rule: WordRule::Dollar,
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
        word_rule: WordRule::Dollar,
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
        word_rule: WordRule::Default,
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
        word_rule: WordRule::Default,
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
        word_rule: WordRule::Default,
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
        // redline's own superset query (upstream C++ highlights + the
        // variables/members/literals/operators the grammar supports but
        // the pinned crate's 70-line query omits — issue-lang-highlight-parity).
        highlight_query: Some(CPP_HIGHLIGHTS),
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
        word_rule: WordRule::Default,
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
        word_rule: WordRule::BareKey,
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
        // JSON keys are not navigation targets (issue-json-yaml-no-symbols).
        definition_query: None,
        rust_tables_query: None,
        flat_define: None,
        word_rule: WordRule::Default,
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
        // YAML keys are not navigation targets (issue-json-yaml-no-symbols).
        definition_query: None,
        rust_tables_query: None,
        flat_define: None,
        word_rule: WordRule::Default,
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
        word_rule: WordRule::Default,
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
        word_rule: WordRule::Default,
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
        word_rule: WordRule::Default,
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
        word_rule: WordRule::Default,
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
        word_rule: WordRule::Suffix,
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
        word_rule: WordRule::Lisp,
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
        word_rule: WordRule::Lisp,
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
        word_rule: WordRule::Default,
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

/// The TypeScript/TSX highlight query — redline's own SUPERSET of the
/// pinned tree-sitter-typescript 0.23.2 `queries/highlights.scm` (the
/// upstream patterns are kept verbatim inside the file, extended with
/// the locals/properties/calls/literals/tokens patterns the grammar
/// supports — see the file's header for the node-name provenance). It
/// is used by BOTH the TS and TSX rows; only node kinds present in both
/// grammars may appear in it.
pub const TYPESCRIPT_HIGHLIGHTS: &str =
    include_str!("../queries/typescript/highlights.scm");

/// The C++ highlight query — redline's own SUPERSET of the pinned
/// tree-sitter-cpp 0.23.4 `queries/highlights.scm` (same shape as the
/// TypeScript superset — see the file's header).
pub const CPP_HIGHLIGHTS: &str = include_str!("../queries/cpp/highlights.scm");

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
    include_str!("../../../third_party/tree-sitter-c-sharp-0.23.5/highlights.scm");

/// The Clojure highlight query — vendored VERBATIM from
/// `tree-sitter-clojure` 0.1.0's `grammar-src/queries/highlights.scm`
/// (the crate exports only `LANGUAGE` / `NODE_TYPES` — no highlights
/// constant). Copy lives in
/// `third_party/tree-sitter-clojure-0.1.0/highlights.scm` (sha256 at
/// vendor time: 424b3b60f43cbb008c8d87730845855e0c1dde657f1a6f2e1408caf4f16914de);
/// keep it in lockstep with the pinned crate version — do not edit or
/// re-flow.
pub const CLOJURE_HIGHLIGHTS: &str =
    include_str!("../../../third_party/tree-sitter-clojure-0.1.0/highlights.scm");

#[cfg(test)]
mod tests {
    use super::*;

    /// issue-language-aware-symbols: the per-language word-constituent
    /// rule, pinned BOTH directions per changed language — the new
    /// constituents are one unit, AND the operators that should split
    /// still split. Mutating any row reddens its entry here.
    #[test]
    fn word_rule_is_per_language() {
        use LanguageId as L;
        // ── Clojure (the reported bug) ─────────────────────────────────
        // `jwks/fetch-issuer-info` is ONE word: every char in the run is a
        // word char (the reader's symbol alphabet [a-zA-Z0-9*+!?:_.-/]).
        for c in "jwks/fetch-issuer-info".chars() {
            assert!(is_word_char(L::Clojure, c), "clojure: `{c}` is a word char");
        }
        for c in [':', '.', '*', '+', '!', '<', '>', '=', '?', '-'].iter().copied() {
            assert!(is_word_char(L::Clojure, c), "clojure: `{c}` is a word char");
        }
        // The boundary chars stay boundaries: whitespace + the list
        // delimiters.
        // The quote form is a SEPARATE grammar node (probe: `quoting_lit`
        // holds its own `'` leaf before the bare `sym_lit`) — a boundary.
        for c in [' ', '(', ')', '[', ']', '{', '}', '\'', '\n'].iter().copied() {
            assert!(!is_word_char(L::Clojure, c), "clojure: `{c}` is NOT a word char");
        }
        // Scheme: the same reader family (probe: `fetch-issuer-info`,
        // `a/b`, `c.d`, `:e` each parse as ONE `symbol` node); the spec
        // row shares the full lisp alphabet with Clojure.
        for c in [ '-', '/', '.', ':', '*', '+', '!', '?', '<', '>', '=' ].iter().copied() {
            assert!(is_word_char(L::Scheme, c), "scheme: `{c}` is a word char");
        }
        // The list delimiters stay boundaries in Scheme too.
        // The quote form is a SEPARATE grammar node in Scheme too (probe: the
        // `quote` node holds its own `'` leaf before the bare `symbol`) — a
        // boundary, as in Clojure.
        for c in [' ', '(', ')', '[', ']', '{', '}', ';', '\'', '\n'].iter().copied() {
            assert!(!is_word_char(L::Scheme, c), "scheme: `{c}` is NOT a word char");
        }
        // ── JavaScript / TypeScript / TSX ───────────────────────────────
        // `$` IS an identifier char (probe: `$foo`, `bar$`, `$bar` are
        // single `identifier` nodes) and…
        for lang in [L::JavaScript, L::TypeScript, L::Tsx].iter().copied() {
            assert!(is_word_char(lang, '$'), "js: `$` is a word char");
            // … `-` STAYS a boundary (`a-b` is subtraction, probe: the
            // grammar parses it as two identifiers around a `-`).
            assert!(!is_word_char(lang, '-'), "js: `-` is NOT a word char");
        }
        // ── Ruby ────────────────────────────────────────────────────────
        // `empty?` / `save!` are one `identifier` node (probe); the
        // ternary `?` is a separate whitespace-separated node, and `-`
        // arithmetic keeps splitting.
        assert!(is_word_char(L::Ruby, '?'), "ruby: `?` is a word char");
        assert!(is_word_char(L::Ruby, '!'), "ruby: `!` is a word char");
        assert!(!is_word_char(L::Ruby, '-'), "ruby: `-` is NOT a word char");
        // ── Rust / Go / C / Cpp / Java / C# / Python / Bash ────────────
        // `-` REMAINS the boundary where it is an operator (arithmetic
        // `a-b` must not become one symbol) — the crux of "language
        // aware". Rust additionally: a lifetime tick is NOT a word char
        // (probe: `'a` is a separate `lifetime` node, not an identifier).
        for lang in [L::Rust, L::Go, L::C, L::Cpp, L::Java, L::CSharp, L::Python, L::Bash]
            .iter().copied()
        {
            assert!(!is_word_char(lang, '-'), "{lang:?}: `-` is NOT a word char");
            assert!(!is_word_char(lang, '$'), "{lang:?}: `$` is NOT a word char");
            assert!(!is_word_char(lang, '?'), "{lang:?}: `?` is NOT a word char");
            assert!(is_word_char(lang, '_'), "{lang:?}: `_` is a word char");
        }
        assert!(!is_word_char(L::Rust, '\''), "rust: `'` is NOT a word char");
        // ── TOML ────────────────────────────────────────────────────────
        // Bare keys carry `-` (probe: `a-b`, `key-x` are single
        // `bare_key` nodes); it is not an operator there.
        assert!(is_word_char(L::Toml, '-'), "toml: `-` is a word char");
        assert!(!is_word_char(L::Toml, '$'), "toml: `$` is NOT a word char");
        // ── Data / plain rows: the historical rule ─────────────────────
        for lang in [L::Json, L::Yaml, L::Markdown, L::Plain].iter().copied() {
            assert!(!is_word_char(lang, '-'), "{lang:?}: `-` is NOT a word char");
            assert!(is_word_char(lang, 'x'), "{lang:?}: `x` is a word char");
        }
    }

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
        // grammar, and a highlight query. Every non-Plain, non-JSON, non-YAML
        // row additionally carries a definition query (issue-json-yaml-no-symbols:
        // JSON and YAML have no symbols — their keys are not navigation targets).
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
            let expect_no_def = matches!(row.id, LanguageId::Json | LanguageId::Yaml);
            if expect_no_def {
                assert!(
                    row.definition_query.is_none(),
                    "{:?}: JSON/YAML must have no definition query", row.id
                );
            } else {
                assert!(
                    row.definition_query.is_some(),
                    "{:?}: no definition query",
                    row.id
                );
            }
        }

        // Extensions are unique across rows and round-trip through the
        // resolver. The resolver is now built *from* this table, so the two
        // failure modes left here are a duplicate extension and a row/resolver
        // id mismatch (a "stale ext_map entry" or a "missing extension" is no
        // longer expressible — that is the point of the table).
        let mut exts = HashSet::new();
        for row in &LANGUAGES {
            for ext in row.extensions {
                assert!(
                    exts.insert(*ext),
                    "extension `{ext}` claimed by more than one row"
                );
                assert_eq!(
                    crate::registry::resolve_language(&format!("f.{ext}")),
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

    // ── highlight query load smoke (issue-lang-highlight-parity) ────────
    /// Every language's highlight query must COMPILE against its grammar:
    /// a tree-sitter query that names a node/field the grammar does not
    /// have is a load-time error, and a load failure silently degrades the
    /// language to plain text at render time. This covers both redline's
    /// own superset queries (TS shared by TSX, C++) and every upstream /
    /// vendored query, so a superset edit that names a node absent from
    /// ONE of the two TS grammars fails here, not in a user's terminal.
    #[test]
    fn every_language_highlight_query_loads() {
        for spec in &LANGUAGES {
            let Some(grammar) = spec.grammar else { continue };
            let Some(query) = spec.highlight_query else { continue };
            assert!(
                tree_sitter::Query::new(&grammar(), query).is_ok(),
                "{name}: highlight query does not compile against the pinned grammar",
                name = spec.name
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

        let c_sharp_path = manifest_dir
            .join("../../third_party/tree-sitter-c-sharp-0.23.5/highlights.scm");
        let clojure_path =
            manifest_dir.join("../../third_party/tree-sitter-clojure-0.1.0/highlights.scm");

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

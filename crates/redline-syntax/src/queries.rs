//! Symbol (definition) queries: one tree-sitter QUERY per language that
//! captures top-level *definitions* only — functions, methods,
//! types/structs/enums/traits/interfaces, constants, macros — plus the
//! definition item's extent (for which-function / imenu nesting).
//!
//! 010-01 adds the Rust-only per-file tables (struct fields, impl methods
//! with their impl kind — plan 010 Shape A rung 1) on the SAME parse
//! (`extract_all`); the M-. self-receiver consumption in `app/store.rs`
//! queries them synchronously, and the bare-symbol consumers keep using
//! `extract_symbols` (the tables discarded). 010-03 (rung 3) extends the
//! same tables with the per-file local binding map (written-down types
//! only) consumed by the M-. local-binding pre-step.
//!
//! All tree-sitter churn lives here (plan layering rule): the query
//! strings are touched only in this module; the per-language facts that
//! are DATA (grammar pin, highlight/injections/locals queries, which
//! query each language uses) live in the descriptor table
//! (`language.rs`), which names the consts defined here. `nav/` and the
//! app layer consume the opaque `Symbol` records this module produces
//! and never touch tree-sitter directly.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use streaming_iterator::StreamingIterator;
use tree_sitter::{
    Language, ParseOptions, Parser, Point, Query, QueryCursor, QueryCursorOptions,
};

use crate::registry::LanguageId;

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

// ── 010-01: Rust association + struct field tables (rung 1) ─────────────
// Per-file tables built in the SAME pass as the symbols (same tree, zero
// extra parse cost) — the data the M-. self-receiver consumption queries
// synchronously. A struct field's location and an impl method's location
// (with the impl's kind: inherent or the implemented trait's path).

/// One struct field: its name and the line the `field_declaration` starts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructField {
    pub field: String,
    /// 0-based line of the field declaration.
    pub line: usize,
}

/// How an impl block associates a method with its type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImplKind {
    /// `impl Type { … }` (no trait).
    Inherent,
    /// `impl Trait for Type { … }`: the trait's full path text
    /// (e.g. `std::fmt::Display`).
    Trait(String),
}

/// (010-03, plan 010 Shape A rung 3) One local binding's WRITTEN-DOWN
/// type: a `let x: Type` annotation (only a bare `type_identifier`
/// annotation contributes — generic, path-shaped, and non-struct types
/// degrade) or a `let x = Type { … }` struct-literal RHS (`&T { … }` /
/// `T::<u8> { … }` literals contribute nothing). `let mut x: T` records
/// the same binding as `let x: T`. PATTERN bindings (`if let` / `while
/// let` / match arm patterns) are `let_expression` / pattern nodes, not
/// `let_declaration`s — they contribute nothing. Never inferred — an
/// unannotated binding contributes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalBinding {
    /// The (start_byte, end_byte) of the binding's innermost enclosing
    /// `block` scope in the indexed source (probe-verified: a fn body IS
    /// a `block` node, as are closure / loop / arm / nested-block bodies).
    pub scope: (usize, usize),
    /// The binding name.
    pub binding: String,
    /// The written-down type name (a bare identifier).
    pub type_name: String,
    /// 0-based line of the `let`.
    pub line: usize,
    /// The `let`'s start byte (shadowing: within one scope, the last
    /// `let` before the use site wins).
    pub let_byte: usize,
}

/// One method of an impl block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImplMethod {
    pub method: String,
    /// 0-based line where the method's `fn` starts.
    pub line: usize,
    /// 0-based line where the `impl` block starts (the find-implementations
    /// table key — Rung 4's read-only view).
    pub impl_line: usize,
    pub kind: ImplKind,
}

/// The per-file Rust tables: `fields` is struct name → its fields, `impls`
/// is the impl's self-type BASE name → its methods (a generic self type
/// `Foo<T>` is keyed by `Foo`; a non-identifier self type contributes
/// nothing — honest degradation); `bindings` (010-03) is the file's local
/// binding map — written-down types keyed by the binding's innermost
/// enclosing `block` scope, sorted by (scope, let byte, name, type).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RustTables {
    pub fields: HashMap<String, Vec<StructField>>,
    pub impls: HashMap<String, Vec<ImplMethod>>,
    pub bindings: Vec<LocalBinding>,
}

/// 010-01 accumulator for one `impl_item` while the table query runs (keyed
/// by the impl node's start byte, which dedupes the double pattern match of
/// a trait impl): its self-type base name, the trait path (if any), the
/// impl's start line, and the methods seen so far as `(name, line)`.
#[derive(Default)]
struct ImplAcc {
    base: String,
    trait_name: Option<String>,
    impl_line: usize,
    methods: Vec<(String, usize)>,
}

// ── per-language definition queries ─────────────────────────────────────
// Each pattern captures `@name` (the identifier) and `@item` (the whole
// definition node, for its extent). Node names / field names were verified
// against the pinned grammar versions (see the dump tests in issue 05's
// history); `@item`'s node kind drives the `SymbolKind` via `kind_of`.

pub(crate) const RUST_QUERY: &str = r#"
(function_item name: (identifier) @name) @item
(struct_item name: (type_identifier) @name) @item
(enum_item name: (type_identifier) @name) @item
(trait_item name: (type_identifier) @name) @item
(mod_item name: (identifier) @name) @item
(const_item name: (identifier) @name) @item
(static_item name: (identifier) @name) @item
(macro_definition name: (identifier) @name) @item
"#;

// 010-01 (plan 010 Shape A, rung 1): the RUST-ONLY association + struct
// field tables. Node shapes verified against the pinned tree-sitter-rust
// 0.24.2 NODE_TYPES (throwaway S-expr probe, issue 010-01; re-pinned by
// the grammar-bumps suite from the probed 0.23.3): an `impl_item`
// names its self type in the `type` field and its implemented trait in a
// SEPARATE `trait` field (a field match can't be made optional in a query,
// so the trait impl and the inherent impl are two patterns — a trait impl
// matches both, the extraction dedupes per impl node); methods are
// `function_item`s under `body: (declaration_list …)`, struct fields are
// `field_declaration`s under `body: (field_declaration_list …)`. The table
// query runs on the SAME tree as `RUST_QUERY` (zero extra parse cost).
pub(crate) const RUST_TABLES_QUERY: &str = r#"
(impl_item
  trait: (_) @impl_trait
  type: (_) @impl_type
  body: (declaration_list (function_item name: (identifier) @impl_method))) @impl_item
(impl_item
  type: (_) @impl_type
  body: (declaration_list (function_item name: (identifier) @impl_method))) @impl_item
(struct_item name: (type_identifier) @struct_name body: (field_declaration_list (field_declaration name: (field_identifier) @struct_field)))
;  010-03 (plan 010 Shape A, rung 3): local binding types — written-down
;  types ONLY (never inferred). Node shapes verified against the pinned
;  tree-sitter-rust 0.24.2 NODE_TYPES (throwaway S-expr probe, issue
;  010-03): a `let_declaration` names its binding in the `pattern` field
;  (NOT `name`), its annotation in the `type` field, and a struct-literal
;  RHS is `value: (struct_expression name: (type_identifier) …)` — the
;  literal's type lives in the struct_expression's `name` field. Only a
;  bare `type_identifier` contributes on either side: `let mut x: T` adds
;  an anonymous `mutable_specifier` child (the captures are unaffected),
;  while `Vec<i32>` (`generic_type`), `std::path::PathBuf`
;  (`scoped_type_identifier`), `&T { … }` (a `reference_expression`),
;  and `T::<u8> { … }` (`generic_type_with_turbofish`) all miss
;  deliberately. Probe note on macros: the probe's `macro!(let m: H;)`
;  invocation parsed as ANONYMOUS token-tree tokens (no `let_declaration`
;  node), so that macro contributed nothing — which is exactly what we
;  want; if a grammar version ever parsed token-tree contents as real
;  nodes, only a WRITTEN-DOWN type would still be recorded (never an
;  inference), so either direction is safe.
(let_declaration pattern: (identifier) @bnd_name type: (type_identifier) @bnd_type) @bnd_let
(let_declaration pattern: (identifier) @bnd_name value: (struct_expression name: (type_identifier) @bnd_lit)) @bnd_let
"#;

pub(crate) const TYPESCRIPT_QUERY: &str = r#"
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

pub(crate) const JAVASCRIPT_QUERY: &str = r#"
(function_declaration name: (identifier) @name) @item
(class_declaration name: (identifier) @name) @item
(method_definition name: (property_identifier) @name) @item
(program (lexical_declaration (variable_declarator name: (identifier) @name)) @item)
(program (variable_declaration (variable_declarator name: (identifier) @name)) @item)
(export_statement declaration: (lexical_declaration (variable_declarator name: (identifier) @name)) @item)
(export_statement declaration: (variable_declaration (variable_declarator name: (identifier) @name)) @item)
"#;

pub(crate) const PYTHON_QUERY: &str = r#"
(function_definition name: (identifier) @name) @item
(class_definition name: (identifier) @name) @item
"#;

pub(crate) const GO_QUERY: &str = r#"
(function_declaration name: (identifier) @name) @item
(method_declaration name: (field_identifier) @name) @item
(type_declaration (type_spec name: (type_identifier) @name)) @item
(const_spec name: (identifier) @name) @item
(var_spec name: (identifier) @name) @item
"#;

pub(crate) const C_QUERY: &str = r#"
(function_definition (function_declarator (identifier) @name)) @item
(struct_specifier name: (type_identifier) @name) @item
(preproc_def name: (identifier) @name) @item
"#;

pub(crate) const CPP_QUERY: &str = r#"
(function_definition (function_declarator (identifier) @name)) @item
(class_specifier name: (type_identifier) @name) @item
(struct_specifier name: (type_identifier) @name) @item
"#;

pub(crate) const TOML_QUERY: &str = r#"
(table (bare_key) @name) @item
(table (dotted_key) @name) @item
(pair (bare_key) @name) @item
(pair (dotted_key) @name) @item
"#;

pub(crate) const JSON_QUERY: &str = r#"
(pair key: (string (string_content) @name)) @item
"#;

pub(crate) const YAML_QUERY: &str = r#"
(block_mapping_pair key: (flow_node) @name) @item
"#;

pub(crate) const BASH_QUERY: &str = r#"
(function_definition (word) @name) @item
"#;

pub(crate) const MARKDOWN_QUERY: &str = r#"
(atx_heading (inline) @name) @item
(setext_heading (paragraph (inline) @name)) @item
"#;

// New-languages lane: Java. Node shapes verified against the pinned
// tree-sitter-java 0.23.5 NODE_TYPES + S-expr probe: class /
// interface / enum / method / constructor all carry a `name` field.
// The grammar version does not parse standalone record declarations
// (probe-verified parse error), so records are not captured — fields
// are deliberately not in the outline either (a `static final` filter
// is not expressible in the query; honest minimal classes/methods
// outline).
pub(crate) const JAVA_QUERY: &str = r#"
(class_declaration name: (identifier) @name) @item
(interface_declaration name: (identifier) @name) @item
(enum_declaration name: (identifier) @name) @item
(method_declaration name: (identifier) @name) @item
(constructor_declaration name: (identifier) @name) @item
"#;

// New-languages lane: C#. Node shapes verified against the pinned
// tree-sitter-c-sharp 0.23.5 NODE_TYPES + S-expr probe (re-pinned by the
// grammar-bumps suite from the probed 0.23.1). The namespace name may be
// a `qualified_name` (`Foo.Bar`), a `generic_name`, or a plain
// `identifier` (probe-verified field types), so it is captured with `(_)`. Enum members, local variables, and events are
// deliberately out of the outline (honest minimal classes/methods/
// properties set). `this.X` parse errors in this grammar version
// (probe-verified) — do not build fixtures around it.
pub(crate) const C_SHARP_QUERY: &str = r#"
(namespace_declaration name: (_) @name) @item
(class_declaration name: (identifier) @name) @item
(interface_declaration name: (identifier) @name) @item
(enum_declaration name: (identifier) @name) @item
(struct_declaration name: (identifier) @name) @item
(record_declaration name: (identifier) @name) @item
(property_declaration name: (identifier) @name) @item
(method_declaration name: (identifier) @name) @item
(constructor_declaration name: (identifier) @name) @item
(delegate_declaration name: (identifier) @name) @item
"#;

// Runtime-bump lane: Clojure (the 0.25-generation landing — the first
// grammar whose crate hard-requires the 0.25 runtime, `tree-sitter
// ^0.25.6` as a NORMAL dependency — which is what the 0.24.7→0.25.10
// runtime bump unblocked). The pinned tree-sitter-clojure 0.1.0 is a
// FLAT S-expression grammar (probe-verified node set: source /
// list_lit / sym_lit / kwd_lit / num_lit / … — there is no `def` / `defn`
// node kind; a definition is a bare `sym_lit` head inside a `list_lit`).
// LITERAL content matches on symbol-ish kinds are not usable (the Scheme
// `symbol` precedent, probe-verified), so the query captures every
// two-symbol `list_lit` candidate and `flat_define_kind` gates on the
// head symbol's text (unqualified — a namespaced head
// `clojure.core/defn` never matches, the honest minimal set). Application
// forms (`(+ a b)`, `map f coll`), literals, and `ns` forms are captured
// as candidates and then rejected — their head is not a define form.
pub(crate) const CLOJURE_QUERY: &str = r#"
(list_lit . (sym_lit) @head . (sym_lit name: (sym_name) @name)) @item
"#;

// New-languages lane: Ruby. Node shapes verified against the pinned
// tree-sitter-ruby 0.23.1 NODE_TYPES + S-expr probe: `module` /
// `class` name their `constant` in the `name` field; `def` is a
// `method` (a `def self.helper` is a `singleton_method`); a top-level
// `CONST = …` is an `assignment` whose `left` is a `constant` (an
// `@x = …` instance-variable assignment does NOT match — its `left`
// is an `instance_variable`). Local variables and methods with
// default/blocked bodies stay out (honest minimal outline).
pub(crate) const RUBY_QUERY: &str = r#"
(module name: (constant) @name) @item
(class name: (constant) @name) @item
(method name: (identifier) @name) @item
(singleton_method name: (identifier) @name) @item
(assignment left: (constant) @name) @item
"#;

// New-languages lane: Scheme (the lisp-family landing). The pinned
// tree-sitter-scheme 0.24.7 is a FLAT S-expression grammar (probe-
// verified: the only node kinds are program/list/symbol/number/
// string/character/vector/comment/quote/…) — there is no `defun` /
// `define_library` node. LITERAL content matches on the `symbol` kind
// are rejected at query compile time (probe-verified: `QueryError
// NodeType` on `(symbol "define")`), so the query captures every
// first-child-anchored `(list HEAD …)` candidate instead — the `.`
// anchors are load-bearing (probe-verified: WITHOUT them, the engine
// binds `@head`/`@name` to ANY children of the list, producing
// spurious candidates like `name=core` for `(define-library (foo
// core) …)`). `extract_all` keeps only the true define forms
// (`scheme_kind` gates on `(pattern_index, head)`). Application forms
// (`map name=lambda`, …), `export`, `set!`, and record accessors are
// captured as candidates and then rejected (their head is not a define
// form) — the honest minimal outline.
pub(crate) const SCHEME_QUERY: &str = r#"
(list . (symbol) @head . (list . (symbol) @name)) @item
(list . (symbol) @head . (symbol) @name) @item
"#;

/// The symbol category of a flat-S-expression-grammar match (SCHEME or
/// CLOJURE) — keyed by the descriptor-table row's gate
/// (`language::FlatDefineGate`), the query pattern's SOURCE ORDER
/// (`QueryMatch::pattern_index`), and the captured head symbol's text.
/// The flat grammars give every definition the same item node kind
/// (`list` in Scheme, `list_lit` in Clojure) and their symbol kinds
/// refuse literal content matches, so the gating happens here, not in
/// the query. `None` = the captured candidate is not a define form
/// (rejected by the extraction loop).
fn flat_define_kind(
    gate: crate::language::FlatDefineGate,
    pattern_index: usize,
    head: &str,
) -> Option<SymbolKind> {
    use crate::language::FlatDefineGate;
    match gate {
        FlatDefineGate::Scheme => match (head, pattern_index) {
            ("define", 0) => Some(SymbolKind::Function), // (define (f .) …)
            ("define", 1) => Some(SymbolKind::Constant), // (define x …)
            ("define-library", 0) => Some(SymbolKind::Type), // (define-library (name .) …)
            ("define-record-type", 1) => Some(SymbolKind::Type), // (define-record-type name …)
            ("define-macro", 0) => Some(SymbolKind::Macro), // (define-macro (m .) …)
            _ => None,
        },
        FlatDefineGate::Clojure => match head {
            "defn" | "defn-" => Some(SymbolKind::Function), // (defn name [args] …)
            "def" => Some(SymbolKind::Constant), // (def name …) — fn-or-value undecidable, honest Constant
            "defmacro" | "defmulti" => Some(SymbolKind::Macro), // (defmacro name …) / (defmulti name …)
            "defmethod" => Some(SymbolKind::Method), // (defmethod name dispatch …)
            "defrecord" | "deftype" | "defprotocol" => Some(SymbolKind::Type), // (defrecord Name [fields]) / (deftype Name …) / (defprotocol Name …)
            _ => None,
        },
    }
}

/// The definition query for a language; `None` for plain text (the
/// documented empty fallback — plain files contribute no outline).
/// Thin wrapper over the descriptor table (the old 18-arm match, which
/// duplicated the table's `definition_query` column, is gone).
pub fn query_for(lang: LanguageId) -> Option<&'static str> {
    crate::language::spec(lang).definition_query
}

/// The `Language` (grammar) for a language id — the thin wrapper the
/// app layer and `node.rs` call; the grammar pin itself lives in the
/// descriptor table (the old 18-arm match, the second copy of the pin,
/// is gone). `None` for plain text. Part of the crate contract: the bin
/// drives parsers for import/notes extraction directly (plan 014 stage 1
/// widened it from `pub(crate)`).
pub fn language_for(lang: LanguageId) -> Option<Language> {
    crate::language::spec(lang).grammar.map(|grammar| grammar())
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
    /// The Rust-only table query (impl association + struct fields, 010-01);
    /// built on first Rust extraction (None until then, never for other
    /// languages).
    rust_tables: Option<Query>,
}

impl ThreadLocal {
    fn new() -> Self {
        Self {
            parser: Parser::new(),
            queries: HashMap::new(),
            rust_tables: None,
        }
    }
}

// ── per-file deadline (the file budget) ─────────────────────────────────
// A single deadline per file, SHARED across the parse and every query run
// on its tree, so a file cannot spend its allowance twice (per-phase
// budgets would hand a file 2x its allowance). The budget is a SAFETY NET
// that must never fire on a legitimate file: the goal is bounding the
// worst case, not optimising any particular project.
//
// Derivation of the constants (RESOLVED CONTEXT — the user's real
// 10,150-file project, release, 32 cores, with the non-gitignored
// `node_modules` dependency tree EXCLUDED): 3,596 files index in 368 ms;
// the legitimate per-file distribution is extract_ms p50 = 0.541, p90 =
// 5.512, p99 = 32.686, max = 98.049 ms, with the largest legitimate file
// at 1.5 MB (the earlier "pathological" 11.3 MB .cpp at 55,351 ms / 4.9
// ms/KB turned out to be a generated dependency-tree blob inside
// `node_modules` — that is a separate issue, `issue-dependency-dir-guard`;
// the 8.7 MB JavaScript file at 0.12 ms/KB and the ~0.4 ms/KB cpp ceiling
// remain the worst OBSERVED legitimate rates). So a legitimate ~10 MB
// source file at the worst observed rate takes ~4 s; the budget must
// keep a 5x-class margin above that while still bounding a 4.9 ms/KB
// outlier at ~10-11 MB (55 s) well below what it would otherwise take:
// * base 500 ms ≈ 15x the observed small-file p99 (32.7 ms) — a generous
//   floor that covers machine variance for small files and is a rounding
//   error for anything above ~250 KB;
// * rate 2.0 ms/KB is 5x ABOVE the worst observed legitimate rate
//   (0.4 ms/KB — a 10 MB legitimate cpp file gets a ~21 s budget against
//   a ~4 s cost; the 1.5 MB / 98 ms largest legitimate file gets ~3.4 s
//   against its 98 ms) and 2.5x BELOW the pathological 4.9 ms/KB: an
//   11.3 MB outlier at that rate gets ~23.6 s instead of 55.4 s — the
//   startup is bounded, and every legitimate file measured keeps a
//   5-35x margin (in doubt, err generous: a budget that fires on a
//   legitimate file is a regression).
pub const FILE_BUDGET_BASE: Duration = Duration::from_millis(500);
pub const FILE_BUDGET_PER_KB: Duration = Duration::from_millis(2);

/// The default per-file deadline for `size_bytes` of source:
/// `base + rate * size_kb` (KB counted up — a 1-byte file pays one KB).
pub fn default_file_budget(size_bytes: usize) -> Duration {
    let kb = u32::try_from(size_bytes.saturating_add(1023) / 1024).unwrap_or(u32::MAX);
    FILE_BUDGET_BASE + FILE_BUDGET_PER_KB.checked_mul(kb).unwrap_or(Duration::MAX)
}

/// The ONE deadline a file's extraction is held to, shared by the parse
/// and every query run on its tree: both progress callbacks ask the same
/// question (`expired`), so the budget is per-file, not per-phase.
struct FileDeadline {
    start: Instant,
    limit: Duration,
}

impl FileDeadline {
    fn new(limit: Duration) -> Self {
        Self { start: Instant::now(), limit }
    }
    fn expired(&self) -> bool {
        self.start.elapsed() >= self.limit
    }
}

/// The stage timings of one [`extract_all_timed`] call: the tree-sitter
/// PARSE versus the QUERY execution on the parsed tree (the split the
/// headless index profiler reports). `query_compile` is the one-time
/// `Query::new` cost paid by the first file of each (language, rayon
/// thread); it is cached afterwards and is NOT per-file steady state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtractTimings {
    /// `Parser::parse` time.
    pub parse: Duration,
    /// All query execution on the parsed tree (definition query + the
    /// Rust tables query when present).
    pub queries: Duration,
    /// One-time `Query::new` compilation (see the struct doc).
    pub query_compile: Duration,
    /// True when the per-file deadline fired (the parse or a query run
    /// was cancelled): the symbols/tables are INCOMPLETE — a first-class
    /// "aborted" outcome that the report must distinguish from a plain
    /// zero-symbol file (never a silent one).
    pub aborted: bool,
    /// The per-file deadline that governed this call (the size-aware
    /// default or the caller's override). 0 on a defaulted instance that
    /// never ran an extraction (e.g. the profiler's unreadable rows).
    pub budget: Duration,
}

/// Extract the definition symbols AND, for Rust, the 010-01 per-file
/// tables (struct fields + impl methods) from `source`. One parse serves
/// both (the table query runs on the same tree — zero extra parse cost).
/// Plain text and an unparseable source yield the empty result. Runs on
/// the calling thread (a rayon worker during indexing); the parser and
/// query caches are thread-local so they are cheap to reuse.
///
/// Every file is held to a per-file deadline — `base + rate * size_kb`
/// (the constants and their derivation: [`FILE_BUDGET_BASE`],
/// [`FILE_BUDGET_PER_KB`], [`default_file_budget`]) — shared across the
/// parse and every query run, so no single file can own the startup. An
/// expired deadline CANCELS the extraction (a first-class "aborted"
/// outcome — an incomplete, possibly empty result — not a silent
/// zero-symbol file). The deadline governs the INDEX extraction path
/// only: the interactive highlight/token paths parse through their own
/// `Highlighter`/parser calls and are deliberately unbounded.
///
/// The production hot path: `timings` is `None`, and the behavior is
/// identical to before the profiler existed (the deadline still applies
/// — that is the point). The cost on that path is exactly one
/// predictable branch set (the `if let Some(...)` timing blocks — the
/// `Instant::now()` calls inside are skipped via `bool::then`) plus one
/// hash lookup per file (`contains_key` for the query-cache compile check
/// at the query stage). The timed sibling is [`extract_all_timed`].
pub fn extract_all(lang: LanguageId, source: &str) -> (Vec<Symbol>, RustTables) {
    extract_all_timed(lang, source, None, None)
}

/// The additive timed sibling of [`extract_all`]: identical behavior,
/// but when `timings` is `Some` the parse stage and the query-execution
/// stage are accumulated into it (the headless index profiler uses it;
/// nothing else in production does).
///
/// `budget` overrides the per-file deadline (the `None` default is the
/// size-aware default, [`default_file_budget(source.len())`]) and is
/// recorded in [`ExtractTimings::budget`]; cancellation sets
/// [`ExtractTimings::aborted`].
pub fn extract_all_timed(
    lang: LanguageId,
    source: &str,
    budget: Option<Duration>,
    mut timings: Option<&mut ExtractTimings>,
) -> (Vec<Symbol>, RustTables) {
    let query_str = match query_for(lang) {
        Some(q) => q,
        None => return (Vec::new(), RustTables::default()),
    };
    // The descriptor-table row: the Rust-only tables query and the
    // flat-S-expression gate are per-language DATA (the old
    // `lang == LanguageId::Rust` / `matches!(lang, Scheme | Clojure)`
    // special-casing is gone).
    let spec = crate::language::spec(lang);
    let language = match language_for(lang) {
        Some(l) => l,
        None => return (Vec::new(), RustTables::default()),
    };
    // The ONE deadline this file's parse + queries are held to (shared —
    // a file cannot spend its allowance twice). `None` budget applies the
    // size-aware default.
    let deadline = FileDeadline::new(budget.unwrap_or_else(|| default_file_budget(source.len())));
    if let Some(t) = timings.as_mut() {
        t.budget = deadline.limit;
    }
    let bytes = source.as_bytes();

    TL.with(|tl| {
        let mut tl = tl.borrow_mut();
        if tl.parser.set_language(&language).is_err() {
            return (Vec::new(), RustTables::default());
        }
        // `None` when `timings` is `None` (the production call) — the
        // `if let Some(...)` blocks below are then never entered.
        let p0 = timings.is_some().then(Instant::now);
        // The chunked input the parser reads (`parse_with_options` takes a
        // byte-offset chunker, like `parse`'s own full-text chunker);
        // offset >= EOF yields the empty slice.
        let mut input = |offset: usize, _point: Point| {
            &bytes[offset.min(bytes.len())..]
        };
        // The deadline's progress callback: returning `true` cancels the
        // parse and `parse` yields `None` (tree-sitter 0.25.10).
        let mut parse_cb = |_state: &tree_sitter::ParseState| deadline.expired();
        let tree = match tl.parser.parse_with_options(
            &mut input,
            None,
            Some(ParseOptions::new().progress_callback(&mut parse_cb)),
        ) {
            Some(t) => t,
            // The language IS set, so `None` can only mean the deadline
            // cancelled the parse: an ABORTED file (a first-class
            // outcome — the report must distinguish it from a plain
            // zero-symbol file).
            None => {
                if let Some(t) = timings.as_mut() {
                    t.aborted = true;
                }
                return (Vec::new(), RustTables::default());
            }
        };
        if let Some(p0) = p0
            && let Some(t) = timings.as_mut()
        {
            t.parse += p0.elapsed();
        }

        // Build (and cache) the query for this language. `Query` is not
        // `Clone`, so it is stored by value and borrowed. Construct inside
        // the `or_insert_with` closure so the cache actually avoids
        // re-construction on subsequent calls.
        let compiled = !tl.queries.contains_key(&lang);
        let c0 = timings.is_some().then(Instant::now);
        let cached = tl.queries
            .entry(lang)
            .or_insert_with(|| Query::new(&language, query_str).ok());
        if let Some(c0) = c0
            && compiled
            && let Some(t) = timings.as_mut()
        {
            t.query_compile += c0.elapsed();
        }
        let query = match cached.as_ref() {
            Some(q) => q,
            None => return (Vec::new(), RustTables::default()),
        };

        let name_idx = query.capture_index_for_name("name");
        let item_idx = query.capture_index_for_name("item");
        // Flat S-expression grammars (Scheme + Clojure): the head-symbol
        // capture the define-form gate reads (see `flat_define_kind`).
        let gate = spec.flat_define;
        let gated = gate.is_some();
        let head_idx = if gated {
            query.capture_index_for_name("head")
        } else {
            None
        };
        let mut out = Vec::new();
        let mut cursor = QueryCursor::new();
        // `matches()` yields one item per definition (with all its captures);
        // `captures()` would yield one item per capture and double the result.
        // Both are `StreamingIterator`s; `QueryMatch` is `!Send`, so extract
        // owned fields per match here. The progress callback holds the query
        // run to the SAME shared deadline the parse already spent part of —
        // query execution (the larger half in aggregate) was the exposure a
        // parse-only budget would have left open.
        let q0 = timings.is_some().then(Instant::now);
        let mut query_cb = |_state: &tree_sitter::QueryCursorState| deadline.expired();
        let mut matches = cursor.matches_with_options(
            query,
            tree.root_node(),
            bytes,
            QueryCursorOptions::new().progress_callback(&mut query_cb),
        );
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
            // Flat-S-expression gate (Scheme + Clojure; see
            // `flat_define_kind`): the query captures every candidate
            // definition form; only the true define forms survive.
            // Rejected candidates contribute no symbol.
            let head_text = gated
                .then(|| {
                    head_idx
                        .and_then(|i| m.captures.iter().find(|c| c.index == i))
                        .and_then(|c| c.node.utf8_text(bytes).ok())
                        .map(str::to_string)
                })
                .flatten();
            let gate_reject = gated
                && head_text
                    .as_deref()
                    .and_then(|h| flat_define_kind(gate.unwrap(), m.pattern_index, h))
                    .is_none();
            if gate_reject {
                continue;
            }

            out.push(Symbol {
                name,
                kind: if gated {
                    // The flat S-expression grammars: the (gate, pattern,
                    // head) triple carries the category (see
                    // `flat_define_kind`).
                    flat_define_kind(
                        gate.unwrap(),
                        m.pattern_index,
                        head_text.as_deref().unwrap_or("")
                    )
                    .unwrap_or(SymbolKind::Function)
                } else {
                    kind_of(kind_node.kind())
                },
                line: name_node.start_position().row,
                end_line: extent_node.end_position().row,
                start_byte: name_node.start_byte(),
                end_byte: name_node.end_byte(),
            });
        }
        // Deterministic order: by line, then byte, then name.
        out.sort_by(|a, b| (a.line, a.start_byte, &a.name).cmp(&(b.line, b.start_byte, &b.name)));
        if let Some(q0) = q0
            && let Some(t) = timings.as_mut()
        {
            t.queries += q0.elapsed();
        }

        // 010-01: the Rust tables — the second query on the SAME tree (the
        // row's `rust_tables_query` column; the non-Rust rows contribute
        // no tables).
        let tables = if let Some(tables_query) = spec.rust_tables_query {
            let tc0 = timings.is_some().then(Instant::now);
            let compiled_now = tl.rust_tables.is_none();
            if compiled_now {
                tl.rust_tables = Query::new(&language, tables_query).ok();
            }
            if let Some(tc0) = tc0
                && compiled_now
                && let Some(t) = timings.as_mut()
            {
                t.query_compile += tc0.elapsed();
            }
            let t0 = timings.is_some().then(Instant::now);
            let tables = match tl.rust_tables.as_ref() {
                Some(q) => extract_rust_tables(q, tree.root_node(), bytes, &deadline),
                None => RustTables::default(),
            };
            if let Some(t0) = t0
                && let Some(t) = timings.as_mut()
            {
                t.queries += t0.elapsed();
            }
            tables
        } else {
            RustTables::default()
        };

        // The deadline expired during the query runs (a loop ended early
        // by cancellation): the symbols/tables are incomplete. Checked
        // AFTER the runs, so the parse-side cancellation (returned above)
        // is the only other abort path.
        if deadline.expired() && let Some(t) = timings.as_mut() {
            t.aborted = true;
        }

        (out, tables)
    })
}

/// Extract the definition symbols in `source` for `lang` (the tables
/// discarded — call sites that only need the outline, pre-010-01). One
/// parse; identical result to before 010-01. Production indexing goes
/// through [`extract_all`] (symbols + tables in one pass); this is the
/// stable symbols-only seam the test suites pin against.
#[allow(dead_code)] // production uses `extract_all`; tests exercise this seam
pub fn extract_symbols(lang: LanguageId, source: &str) -> Vec<Symbol> {
    extract_all(lang, source).0
}

/// Run `RUST_TABLES_QUERY` over an already-parsed Rust tree (010-01):
/// the per-file struct field + impl method tables, and (010-03) the local
/// binding map. A trait impl matches
/// BOTH table patterns, so impl entries are deduped per impl node (its
/// start byte); a method is recorded once per impl block. The self type
/// is keyed by its BASE name: a bare `type_identifier` as-is, a
/// `generic_type` (`Foo<T>`) by its type child — any other self-type shape
/// contributes nothing (honest degradation, never a guess). A `let`
/// matching BOTH 010-03 patterns (`let x: T = T { … }`) is deduped per
/// `let` (the annotation pattern is listed first, so the annotation wins).
fn extract_rust_tables(
    query: &Query,
    root: tree_sitter::Node,
    bytes: &[u8],
    deadline: &FileDeadline,
) -> RustTables {
    // The capture for `name` in this match (`None` when the pattern lacks
    // it — the two impl patterns differ exactly in `impl_trait`).
    fn cap<'t>(
        query: &Query,
        m: &tree_sitter::QueryMatch<'t, '_>,
        name: &str,
    ) -> Option<tree_sitter::Node<'t>> {
        query
            .capture_index_for_name(name)
            .and_then(|i| m.captures.iter().find(|c| c.index == i))
            .map(|c| c.node)
    }
    let mut fields: HashMap<String, Vec<StructField>> = HashMap::new();
    let mut impls: HashMap<usize, ImplAcc> = HashMap::new();
    let mut bindings: Vec<LocalBinding> = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut query_cb = |_state: &tree_sitter::QueryCursorState| deadline.expired();
    let mut matches = cursor.matches_with_options(
        query,
        root,
        bytes,
        QueryCursorOptions::new().progress_callback(&mut query_cb),
    );
    while let Some(m) = matches.next() {
        // The 010-03 local-binding patterns (annotation or struct
        // literal) — captured by the shared `bnd_let` capture.
        if let Some(let_node) = cap(query, m, "bnd_let") {
            let Some(name_node) = cap(query, m, "bnd_name") else { continue };
            // The annotation pattern is listed first, so on the double
            // match the annotation's type wins the dedupe.
            let Some(type_node) = cap(query, m, "bnd_type")
                .or_else(|| cap(query, m, "bnd_lit"))
            else { continue };
            let Some(binding) = name_node.utf8_text(bytes).ok() else { continue };
            let Some(type_name) = type_node.utf8_text(bytes).ok() else { continue };
            // Dedupe per `let` (the double pattern match).
            if bindings.iter().any(|b| b.let_byte == let_node.start_byte()) {
                continue;
            }
            // The innermost enclosing `block` scope (a fn body is a
            // `block` too); a `let` with no block ancestor contributes
            // nothing (honest degradation).
            let mut scope = None;
            let mut ancestor = let_node.parent();
            while let Some(p) = ancestor {
                if p.kind() == "block" {
                    scope = Some((p.start_byte(), p.end_byte()));
                    break;
                }
                ancestor = p.parent();
            }
            let Some(scope) = scope else { continue };
            bindings.push(LocalBinding {
                scope,
                binding: binding.to_string(),
                type_name: type_name.to_string(),
                line: let_node.start_position().row,
                let_byte: let_node.start_byte(),
            });
            continue;
        }
        let item = cap(query, m, "impl_item");
        if let Some(item) = item {
            // The impl's self type (the `type` field); only a bare
            // identifier or `Foo<T>` contributes — anything else skips the
            // impl (honest degradation, never a guess).
            let type_node = cap(query, m, "impl_type").filter(|n| {
                matches!(n.kind(), "type_identifier" | "generic_type")
            });
            let Some(type_node) = type_node else { continue };
            let base = if type_node.kind() == "generic_type" {
                match type_node.child_by_field_name("type") {
                    Some(t) => match t.utf8_text(bytes).ok() {
                        Some(s) => s.to_string(),
                        None => continue,
                    },
                    None => continue,
                }
            } else {
                match type_node.utf8_text(bytes).ok() {
                    Some(s) => s.to_string(),
                    None => continue,
                }
            };
            let trait_name = cap(query, m, "impl_trait")
                .and_then(|n| n.utf8_text(bytes).ok())
                .map(ToOwned::to_owned);
            let Some(method_node) = cap(query, m, "impl_method") else { continue };
            let Some(method) = method_node.utf8_text(bytes).ok() else { continue };
            let method = method.to_string();
            let method_line = method_node.start_position().row;
            let impl_line = item.start_position().row;
            let entry = impls.entry(item.start_byte()).or_insert_with(|| {
                ImplAcc {
                    base: base.clone(),
                    trait_name: trait_name.clone(),
                    impl_line,
                    methods: Vec::new(),
                }
            });
            if !entry.methods.iter().any(|(n, _)| n == &method) {
                entry.methods.push((method, method_line));
            }
            continue;
        }
        // The struct-field pattern.
        let Some(struct_node) = cap(query, m, "struct_name") else { continue };
        let Some(field_node) = cap(query, m, "struct_field") else { continue };
        let Some(s) = struct_node.utf8_text(bytes).ok() else { continue };
        let Some(f) = field_node.utf8_text(bytes).ok() else { continue };
        fields.entry(s.to_string()).or_default().push(StructField {
            field: f.to_string(),
            line: field_node.start_position().row,
        });
    }
    let mut impls_out: HashMap<String, Vec<ImplMethod>> = HashMap::new();
    for acc in impls.into_values() {
        let kind = match acc.trait_name {
            Some(t) => ImplKind::Trait(t),
            None => ImplKind::Inherent,
        };
        impls_out
            .entry(acc.base)
            .or_default()
            .extend(acc.methods.into_iter().map(|(method, line)| ImplMethod {
                method,
                line,
                impl_line: acc.impl_line,
                kind: kind.clone(),
            }));
    }
    for v in fields.values_mut() {
        v.sort_by(|a, b| (a.line, &a.field).cmp(&(b.line, &b.field)));
    }
    for v in impls_out.values_mut() {
        v.sort_by(|a, b| (a.line, &a.method).cmp(&(b.line, &b.method)));
    }
    bindings.sort_by(|a, b| {
        (a.scope, a.let_byte, &a.binding, &a.type_name).cmp(&(b.scope, b.let_byte, &b.binding, &b.type_name))
    });
    RustTables { fields, impls: impls_out, bindings }
}

/// (010-01) The PLAIN self type of the innermost `impl_item` containing
/// `byte` — only when that self type is a bare `type_identifier`:
/// `impl Foo` / `impl std::fmt::Display for Foo` → `"Foo"`.
///
/// The M-. self-receiver consumption's lexical seam: "which impl am I
/// lexically inside". `None` — never a guess — when the point is outside
/// every impl block, the self type is generic (`impl<T> Foo<T>`),
/// path-shaped, or missing, the source is not Rust-shaped (parse miss),
/// or `byte` falls outside the root. One parse (the same one-parse
/// discipline as 007-01's `scope_path_at`).
pub fn rust_self_type_at(source: &str, byte: usize) -> Option<String> {
    let language = language_for(LanguageId::Rust)?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source.as_bytes(), None)?;
    let root = tree.root_node();
    if !(root.start_byte() <= byte && byte < root.end_byte()) {
        return None;
    }
    // Innermost impl_item containing `byte`: smallest span wins (nested
    // impls inside a method body are legal Rust and shadow the outer one).
    let mut best: Option<(usize, tree_sitter::Node)> = None;
    fn visit<'a>(
        node: tree_sitter::Node<'a>,
        byte: usize,
        best: &mut Option<(usize, tree_sitter::Node<'a>)>,
    ) {
        if node.kind() == "impl_item" && node.start_byte() <= byte && byte < node.end_byte() {
            let span = node.end_byte() - node.start_byte();
            if best.as_ref().is_none_or(|(b, _)| span < *b) {
                *best = Some((span, node));
            }
        }
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                visit(child, byte, best);
            }
        }
    }
    visit(root, byte, &mut best);
    let (_, impl_node) = best?;
    let type_node = impl_node.child_by_field_name("type")?;
    (type_node.kind() == "type_identifier")
        .then(|| type_node.utf8_text(source.as_bytes()).ok())?
        .map(|s| s.to_string())
}

/// (010-03, plan 010 Shape A rung 3) The WRITTEN-DOWN type name of the
/// local binding `name` at `byte` in `source`, per this file's binding
/// table `tables` (the M-. local-binding consumption's lexical seam):
/// the enclosing `block` scope chain (innermost first — probe-verified:
/// a fn body IS a `block`, as are closure / loop / arm / nested-block
/// bodies) is consulted innermost-first and the FIRST scope holding a
/// binding `name` whose `let` precedes the use wins — innermost binding
/// wins (a shadow); within a scope the LAST `let` before the use wins.
///
/// `None` — never inferred — when no scope records the binding, the
/// source is not Rust-shaped (parse miss), `byte` falls outside the
/// root, or the node at the use is not the use itself (an
/// `identifier` / `field_identifier` — `x.field` / `x.method()`), which
/// keeps a use inside a string literal from ever resolving. One parse
/// (the same one-parse discipline as 007-01's `scope_path_at` and
/// 010-01's `rust_self_type_at`).
pub fn rust_binding_type_at(
    tables: &RustTables,
    source: &str,
    byte: usize,
    name: &str,
) -> Option<String> {
    if tables.bindings.is_empty() {
        return None;
    }
    let language = language_for(LanguageId::Rust)?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source.as_bytes(), None)?;
    let root = tree.root_node();
    if !(root.start_byte() <= byte && byte < root.end_byte()) {
        return None;
    }
    // The deepest node containing `byte`; its kind must be the use
    // itself (an `identifier` — `x.method()` — or a `field_identifier`
    // — `x.field`).
    let mut node = root;
    loop {
        let mut next = None;
        for i in 0..node.child_count() {
            if let Some(c) = node.child(i)
                && c.start_byte() <= byte
                && byte < c.end_byte()
            {
                next = Some(c);
                break;
            }
        }
        match next {
            Some(c) => node = c,
            None => break,
        }
    }
    if !matches!(node.kind(), "identifier" | "field_identifier") {
        return None;
    }
    // The scope chain: every enclosing `block`, innermost first.
    let mut chain: Vec<(usize, usize)> = Vec::new();
    let mut ancestor = node.parent();
    while let Some(p) = ancestor {
        if p.kind() == "block" {
            chain.push((p.start_byte(), p.end_byte()));
        }
        ancestor = p.parent();
    }
    for scope in &chain {
        // The last recorded `let` in this scope that precedes the use
        // (a later `let` in the same scope is a not-yet-active shadow →
        // fall through to the outer scopes).
        let mut best: Option<&LocalBinding> = None;
        for b in &tables.bindings {
            if b.scope == *scope && b.binding == name && b.let_byte <= byte {
                match best {
                    Some(cur) if cur.let_byte >= b.let_byte => {}
                    _ => best = Some(b),
                }
            }
        }
        if let Some(b) = best {
            return Some(b.type_name.clone());
        }
    }
    None
}

/// Map a definition node's tree-sitter `kind()` string to a `SymbolKind`
/// (cross-language: the node kinds named in the per-language queries are
/// distinct enough to map in one table).
fn kind_of(kind: &str) -> SymbolKind {
    match kind {
        "function_item" | "function_definition" | "function_declaration" => SymbolKind::Function,
        "method_signature" | "method_definition" | "method_declaration"
        | "constructor_declaration" | "method" | "singleton_method" => {
            SymbolKind::Method
        }
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
        | "enum_declaration"
        | "mod_item"
        | "namespace_declaration"
        | "record_declaration"
        | "delegate_declaration"
        | "module"
        | "class" => SymbolKind::Type,
        "const_item"
        | "const_spec"
        | "variable_declarator"
        | "variable_spec"
        | "var_spec"
        | "static_item"
        | "static_initializer"
        | "constant"
        | "property_declaration"
        | "assignment"
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

    // ── 010-01: Rust tables (struct fields + impl methods) ──────────
    /// 010-01 (discriminating): the per-file tables come out of the same
    /// pass — fields keyed by struct, methods keyed by the impl's self
    /// type base name with their impl kind; a trait impl's double pattern
    /// match dedupes (each method recorded once), the generic impl keys by
    /// its base name, a nested mod impl joins the same type's list.
    #[test]
    fn rust_tables_extract_struct_fields_and_impls() {
        let src = "struct Foo { a: i32, pub b: String }\n\
                   struct Generic<T> { x: T }\n\
                   impl Foo {\n\
                   \x20   fn m(&self) { let _ = self.a; }\n\
                   \x20   const C: i32 = 1;\n\
                   }\n\
                   impl std::fmt::Display for Foo {\n\
                   \x20   fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) }\n\
                   }\n\
                   impl<T> Generic<T> {\n\
                   \x20   fn g(&self) { let _ = self.x; }\n\
                   }\n\
                   mod nested {\n\
                   \x20   impl Foo {\n\
                   \x20       fn extra(&self) {}\n\
                   \x20   }\n\
                   }\n";
        let (_syms, tables) = extract_all(LanguageId::Rust, src);
        // Fields: the visibility modifier is ignored, lines are the
        // field_declaration's own line.
        assert_eq!(
            tables.fields.get("Foo"),
            Some(&vec![
                StructField { field: "a".into(), line: 0 },
                StructField { field: "b".into(), line: 0 },
            ]),
            "Foo's fields: {:?}",
            tables.fields
        );
        assert_eq!(
            tables.fields.get("Generic"),
            Some(&vec![StructField { field: "x".into(), line: 1 }]),
        );
        assert_eq!(tables.fields.len(), 2, "exactly the two structs");
        // Impl methods: inherent + trait (the trait's FULL path) + the
        // nested mod impl's method, deduped across the double pattern
        // match, sorted by line.
        let foo = tables.impls.get("Foo").expect("impls[Foo]");
        assert_eq!(foo.len(), 3, "three Foo methods, no duplicates: {foo:?}");
        assert_eq!(foo[0], ImplMethod { method: "m".into(), line: 3, impl_line: 2, kind: ImplKind::Inherent });
        assert_eq!(
            foo[1],
            ImplMethod {
                method: "fmt".into(),
                line: 7,
                impl_line: 6,
                kind: ImplKind::Trait("std::fmt::Display".into()),
            }
        );
        assert_eq!(
            foo[2],
            ImplMethod { method: "extra".into(), line: 14, impl_line: 13, kind: ImplKind::Inherent }
        );
        // The generic self type `Generic<T>` keys by its base name.
        let generic = tables.impls.get("Generic").expect("impls[Generic]");
        assert_eq!(
            generic,
            &[ImplMethod { method: "g".into(), line: 10, impl_line: 9, kind: ImplKind::Inherent }]
        );
        assert_eq!(tables.impls.len(), 2);
        // The symbol pass is byte-for-byte the pre-010-01 outline (the
        // tables are a sidecar — no outline churn).
        let names: Vec<String> = extract_symbols(LanguageId::Rust, src)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(
            names,
            ["Foo", "Generic", "m", "C", "fmt", "g", "nested", "extra"]
                .map(|s| s.to_string())
                .to_vec()
        );
    }

    /// 010-01 (degradation): non-Rust languages contribute no tables (the
    /// second query is Rust-only), plain text none at all. 010-03: the
    /// binding map is Rust-only in the same pass.
    #[test]
    fn rust_tables_non_rust_and_plain_are_empty() {
        let py = "class A:\n    def m(self):\n        pass\n";
        let (_syms, tables) = extract_all(LanguageId::Python, py);
        assert!(
            tables.fields.is_empty() && tables.impls.is_empty() && tables.bindings.is_empty(),
            "{tables:?}"
        );
        let (_syms, tables) = extract_all(LanguageId::Plain, "struct Foo {}");
        assert!(
            tables.fields.is_empty() && tables.impls.is_empty() && tables.bindings.is_empty()
        );
    }

    /// 010-01 (discriminating): the M-. self-receiver's lexical seam — the
    /// innermost impl's self type, only for a PLAIN `type_identifier`.
    #[test]
    fn rust_self_type_at_innermost_and_plain_only() {
        let src = "struct A { f: i32 }\n\
                   struct B { g: i32 }\n\
                   impl A {\n\
                   \x20   fn outer(&self) { impl B { fn inner(&self) { let _ = self.g; } } }\n\
                   }\n\
                   impl std::fmt::Display for A {\n\
                   \x20   fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) }\n\
                   }\n\
                   struct W<T> { w: T }\n\
                   impl<T> W<T> {\n\
                   \x20   fn wg(&self) { let _ = self.w; }\n\
                   }\n";
        let bytes = src.as_bytes();
        // The `self.g` of the NESTED impl B (line 3): the innermost impl
        // shadows the outer one.
        let at = |needle: &str| -> usize {
            src.find(needle).expect("needle in source")
        };
        assert_eq!(rust_self_type_at(src, at("self.g")), Some("B".into()), "nested impl B wins");
        // The trait impl's self type is the `type` field (`A`), not the trait.
        assert_eq!(rust_self_type_at(src, at("fn fmt")), Some("A".into()));
        // The GENERIC self type degrades: no plain identifier.
        assert_eq!(rust_self_type_at(src, at("self.w")), None, "generic impl: never a guess");
        // Outside every impl.
        assert_eq!(rust_self_type_at(src, at("struct A")), None);
        // Byte outside the root (past EOF).
        assert_eq!(rust_self_type_at(src, src.len() + 8), None);
        // Unparseable source (binary garbage): None.
        assert_eq!(rust_self_type_at("\u{ff}\u{fe}impl??", 0), None);
        let _ = bytes;
    }

    // ── 010-03: local binding types (rung 3) ──────────────────────────
    /// 010-03 (discriminating): the per-file local binding map comes out
    /// of the SAME pass — written-down types only: the `let x: Type`
    /// annotation (bare `type_identifier` only) and the
    /// `let x = Type { … }` struct literal; `let mut` records the same
    /// binding; the double pattern match (`let x: Pt = Pt { a: 1 }`) is
    /// deduped (the annotation wins); generics, path-shaped types, and
    /// unannotated lets contribute nothing. Scope key: the innermost
    /// enclosing `block` — the nested block gets a DIFFERENT key than
    /// the fn body, and the symbol outline is byte-for-byte unchanged
    /// (the bindings are a sidecar).
    #[test]
    fn rust_tables_extract_local_bindings() {
        let src = "struct Pt { pub a: i32 }\n\
                   struct Other { pub q: i32 }\n\
                   fn f() {\n\
                   \x20   let x: Pt = Pt { a: 1 };\n\
                   \x20   let mut m: Pt;\n\
                   \x20   let refd = &Pt { a: 1 };\n\
                   \x20   let turbo = Pt::<i32> { a: 1 };\n\
                   \x20   let plain = 5;\n\
                   \x20   let gen: Vec<i32>;\n\
                   \x20   let pathed: std::path::PathBuf;\n\
                   \x20   {\n\
                   \x20       let x: Other;\n\
                   \x20       let y = Other { q: 1 };\n\
                   \x20   }\n\
                   \x20   let late: Other;\n\
                   }\n";
        let (_syms, tables) = extract_all(LanguageId::Rust, src);
        // Exactly five written-down bindings: x:Pt (annotation + literal
        // deduped to one), mut m:Pt, the nested block's x:Other +
        // y:Other, and late:Other. The `&Pt { … }` literal (a
        // reference_expression, not a struct_expression), the
        // `Pt::<i32> { … }` turbofish literal (a
        // generic_type_with_turbofish name), plain / gen / pathed
        // contribute nothing — the documented literal misses pinned here.
        assert_eq!(tables.bindings.len(), 5, "{:?}", tables.bindings);
        let get = |binding: &str, type_name: &str| -> &LocalBinding {
            tables.bindings
                .iter()
                .find(|b| b.binding == binding && b.type_name == type_name)
                .unwrap_or_else(|| panic!("missing {binding}:{type_name}: {:?}", tables.bindings))
        };
        // The annotation + literal double match recorded ONCE, by the
        // annotation.
        let x_pt = get("x", "Pt");
        assert_eq!(x_pt.line, 3);
        // `let mut m: Pt` is the same as `let m: Pt`.
        assert_eq!(get("m", "Pt").line, 4);
        let x_other = get("x", "Other");
        let y_other = get("y", "Other"); // the struct literal's type
        let late = get("late", "Other");
        assert_eq!((x_other.line, y_other.line, late.line), (11, 12, 14));
        // Scope key: the innermost enclosing `block`. The fn-body lets
        // share ONE scope; the nested block's lets share ANOTHER, inside
        // the first (the nested block shadows the fn body's scope).
        let fn_scope = x_pt.scope;
        let inner = x_other.scope;
        assert_eq!(get("m", "Pt").scope, fn_scope, "mut m shares the fn-body scope");
        assert_eq!(late.scope, fn_scope, "late shares the fn-body scope");
        assert_eq!(y_other.scope, inner, "y shares the nested block's scope");
        assert!(
            inner.0 > fn_scope.0 && inner.1 < fn_scope.1,
            "nested block scope ({inner:?}) is strictly inside the fn body scope ({fn_scope:?})"
        );
        // Source order within a scope: x before m before late.
        assert!(x_pt.let_byte < get("m", "Pt").let_byte && get("m", "Pt").let_byte < late.let_byte);
        // The symbol pass is byte-for-byte the pre-010-03 outline (the
        // bindings are a sidecar — no outline churn).
        let names: Vec<String> = extract_symbols(LanguageId::Rust, src)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["Pt", "Other", "f"].map(|s| s.to_string()).to_vec());
    }

    /// 010-03 (discriminating + shadow pin): the written-down type at a
    /// use site — the innermost scope wins (a shadow), within a scope
    /// the LAST `let` before the use wins, an inner block sees the outer
    /// binding only while it records no yet-active binding of its own,
    /// and nothing is ever inferred (unparseable / out-of-root / in a
    /// string literal → `None`).
    #[test]
    fn rust_binding_type_at_scopes_and_shadow() {
        let src = "fn f() {\n\
                   \x20   let x: A;\n\
                   \x20   let _o = x.f;\n\
                   \x20   {\n\
                   \x20       let _i2 = x.f;\n\
                   \x20       let x: B;\n\
                   \x20       let _i = x.f;\n\
                   \x20   }\n\
                   \x20   let x: C;\n\
                   \x20   let _c = x.f;\n\
                   }\n";
        let (_syms, tables) = extract_all(LanguageId::Rust, src);
        assert_eq!(tables.bindings.len(), 3, "{:?}", tables.bindings);
        // `x`'s byte offset after each `let <marker> = ` prefix.
        let at = |marker: &str, pad: usize| -> usize {
            src.find(marker).expect("marker in source") + pad
        };
        // Outer use, before any shadow: `A`.
        assert_eq!(rust_binding_type_at(&tables, src, at("let _o = x.f", 9), "x"), Some("A".into()));
        // Inner use BEFORE the inner shadow: the inner scope records no
        // yet-active binding → the outer `A` (never the inner `B`).
        assert_eq!(rust_binding_type_at(&tables, src, at("let _i2 = x.f", 10), "x"), Some("A".into()));
        // Inner use AFTER the inner shadow: innermost binding wins → `B`
        // (the shadow pin).
        assert_eq!(rust_binding_type_at(&tables, src, at("let _i = x.f", 9), "x"), Some("B".into()));
        // Same-scope shadow after the block: the last `let` before the
        // use wins → `C`.
        assert_eq!(rust_binding_type_at(&tables, src, at("let _c = x.f", 9), "x"), Some("C".into()));
        // A different binding name in the same scopes: nothing.
        assert_eq!(rust_binding_type_at(&tables, src, at("let _o = x.f", 9), "y"), None);
        // Empty table: never a parse, never a guess.
        assert_eq!(rust_binding_type_at(&RustTables::default(), src, at("let _o = x.f", 9), "x"), None);
        // Non-Rust source (parse miss): None.
        assert_eq!(rust_binding_type_at(&tables, "def f():\n    x = 1\n", 20, "x"), None);
        // Byte outside the root (past EOF): None.
        assert_eq!(rust_binding_type_at(&tables, src, src.len() + 8, "x"), None);
        // A use inside a string literal is not a use (the node at the
        // byte is not an identifier / field_identifier): None.
        let str_src = "fn f() {\n    let x: A;\n    let s = \"x.f\";\n}\n";
        let (_syms, str_tables) = extract_all(LanguageId::Rust, str_src);
        assert_eq!(str_tables.bindings.len(), 1);
        let at_str = str_src.find("\"x.f\"").unwrap() + 2;
        assert_eq!(rust_binding_type_at(&str_tables, str_src, at_str, "x"), None, "string-literal use: never resolves");
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
        let src = "# One\n\n## Two\n\nSetext\n====\n\ntext\n";
        let syms = extract_symbols(LanguageId::Markdown, src);
        let one = find(&syms, "One").expect("atx heading One");
        assert_eq!(one.kind, SymbolKind::Heading);
        assert_eq!(one.line, 0);
        let two = find(&syms, "Two").expect("atx heading Two");
        assert_eq!(two.kind, SymbolKind::Heading);
        assert_eq!(two.line, 2);
        let setext = find(&syms, "Setext").expect("setext heading");
        assert_eq!(setext.kind, SymbolKind::Heading);
        assert_eq!(setext.line, 4);
        // Exactly the three headings (no paragraph, no false positives).
        assert_eq!(syms.len(), 3, "{syms:?}");
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


    // ── Java (new-languages lane) ─────────────────────────────────────
    /// Classes / interfaces / enums / methods / constructors land with
    /// their kinds; fields are deliberately out of the outline (a
    /// `static final` filter is not expressible in the query) and the
    /// grammar version does not parse standalone record declarations
    /// (probe-verified) — the honest minimal outline.
    #[test]
    fn java_extracts_class_and_method() {
        let src = "public class Foo {\n\
                   \x20   static final int C = 1;\n\
                   \x20   public int getX() { return 1; }\n\
                   \x20   Foo() {}\n\
                   \x20   public interface Bar { void doIt(); }\n\
                   \x20   enum Color { RED }\n\
                   }\n\
                   class Outer { void main() {} }\n";
        let syms = extract_symbols(LanguageId::Java, src);
        let foo = find(&syms, "Foo").expect("class Foo");
        assert_eq!(foo.kind, SymbolKind::Type);
        // Two `Foo` entries: the class (Type) and the constructor (Method).
        let ctor = syms
            .iter()
            .find(|s| s.name == "Foo" && s.kind == SymbolKind::Method)
            .expect("constructor Foo");
        assert_eq!(ctor.line, 3);
        let getx = find(&syms, "getX").expect("method getX");
        assert_eq!(getx.kind, SymbolKind::Method);
        let bar = find(&syms, "Bar").expect("interface Bar");
        assert_eq!(bar.kind, SymbolKind::Type);
        let color = find(&syms, "Color").expect("enum Color");
        assert_eq!(color.kind, SymbolKind::Type);
        let main = find(&syms, "main").expect("method main");
        assert_eq!(main.kind, SymbolKind::Method);
        // Exactly the eight named definitions — the field `C` and the enum
        // constant `RED` are NOT in the outline.
        assert_eq!(syms.len(), 8, "outline: {syms:?}");
    }

    // ── C# (new-languages lane) ──────────────────────────────────────
    /// Namespaces (qualified), classes, properties, methods,
    /// constructors, enums, and records land with their kinds; enum
    /// members and local variables stay out (honest minimal outline).
    #[test]
    fn csharp_extracts_class_and_method() {
        let src = "namespace Foo {\n\
                   \x20   public class Bar {\n\
                   \x20       public int P { get; set; }\n\
                   \x20       public int GetP() => P;\n\
                   \x20       Bar() {}\n\
                   \x20   }\n\
                   \x20   enum E { A, B }\n\
                   \x20   record Pt(int X);\n\
                   }\n";
        let syms = extract_symbols(LanguageId::CSharp, src);
        let foo = find(&syms, "Foo").expect("namespace Foo");
        assert_eq!(foo.kind, SymbolKind::Type);
        let bar = syms
            .iter()
            .find(|s| s.name == "Bar" && s.kind == SymbolKind::Type)
            .expect("class Bar");
        assert_eq!(bar.line, 1);
        // Two `Bar` entries: the class (Type) and the constructor (Method).
        let ctor = syms
            .iter()
            .find(|s| s.name == "Bar" && s.kind == SymbolKind::Method)
            .expect("constructor Bar");
        assert_eq!(ctor.line, 4);
        let p = find(&syms, "P").expect("property P");
        assert_eq!(p.kind, SymbolKind::Constant);
        let getp = find(&syms, "GetP").expect("method GetP");
        assert_eq!(getp.kind, SymbolKind::Method);
        let e = find(&syms, "E").expect("enum E");
        assert_eq!(e.kind, SymbolKind::Type);
        let pt = find(&syms, "Pt").expect("record Pt");
        assert_eq!(pt.kind, SymbolKind::Type);
        // Exactly the seven named definitions — the enum members `A` /
        // `B` are NOT in the outline.
        assert_eq!(syms.len(), 7, "outline: {syms:?}");
    }

    // ── Ruby (new-languages lane) ────────────────────────────────────
    /// Modules / classes / defs / singleton defs / top-level constants
    /// land with their kinds; local variables, instance-variable
    /// assignments, and class names referenced as superclasses stay
    /// out (honest minimal outline).
    #[test]
    fn ruby_extracts_module_class_method() {
        let src = "module Foo\n\
                   \x20  class Bar < Baz\n\
                   \x20    CONST = 1\n\
                   \x20    def initialize(x)\n\
                   \x20      @x = x\n\
                   \x20    end\n\
                   \x20    def self.helper\n\
                   \x20      42\n\
                   \x20    end\n\
                   \x20  end\n\
                   \x20  def top\n\
                   \x20    Foo::Bar.new(1)\n\
                   \x20  end\n\
                   end\n";
        let syms = extract_symbols(LanguageId::Ruby, src);
        let foo = find(&syms, "Foo").expect("module Foo");
        assert_eq!(foo.kind, SymbolKind::Type);
        let bar = find(&syms, "Bar").expect("class Bar");
        assert_eq!(bar.kind, SymbolKind::Type);
        let const_c = find(&syms, "CONST").expect("top-level constant");
        assert_eq!(const_c.kind, SymbolKind::Constant);
        let init = find(&syms, "initialize").expect("def initialize");
        assert_eq!(init.kind, SymbolKind::Method);
        let helper = find(&syms, "helper").expect("def self.helper");
        assert_eq!(helper.kind, SymbolKind::Method);
        let top = find(&syms, "top").expect("def top");
        assert_eq!(top.kind, SymbolKind::Method);
        // Exactly the six named definitions — the superclass `Baz`, the
        // local `x`, and the `@x` assignment are NOT in the outline.
        assert_eq!(syms.len(), 6, "outline: {syms:?}");
    }

    // ── Scheme (new-languages lane) ──────────────────────────────────
    /// The five define forms land with their pattern-index-derived kinds;
    /// `export` / `set!` / record accessors stay out (honest minimal
    /// outline for the flat S-expression grammar).
    #[test]
    fn scheme_extracts_define_and_library() {
        let src = "(define (add! x y) (+ x y))\n\
                  (define x 10)\n\
                  (define-library (foo core)\n\
                  \x20 (export add!)\n\
                  \x20 (define (inner a) a))\n\
                  (define-record-type point\n\
                  \x20 (make-point x y)\n\
                  \x20 point?\n\
                  \x20 (x point-x))\n\
                  (define-macro (my-if c a b) a)\n";
        let syms = extract_symbols(LanguageId::Scheme, src);
        let add = find(&syms, "add!").expect("define (add! …)");
        assert_eq!(add.kind, SymbolKind::Function);
        let x = find(&syms, "x").expect("define x");
        assert_eq!(x.kind, SymbolKind::Constant);
        let foo = find(&syms, "foo").expect("define-library (foo core)");
        assert_eq!(foo.kind, SymbolKind::Type);
        let inner = find(&syms, "inner").expect("nested define");
        assert_eq!(inner.kind, SymbolKind::Function);
        let point = find(&syms, "point").expect("define-record-type");
        assert_eq!(point.kind, SymbolKind::Type);
        let myif = find(&syms, "my-if").expect("define-macro");
        assert_eq!(myif.kind, SymbolKind::Macro);
        // Exactly the six named definitions — the `export add!` form,
        // the record constructor, and the accessors are NOT in the
        // outline.
        assert_eq!(syms.len(), 6, "outline: {syms:?}");
    }

    // ── Clojure (runtime-bump lane) ─────────────────────────────────
    /// `defn` / `def` / `defmacro` / `defmulti` / `defmethod` /
    /// `defrecord` / `deftype` / `defprotocol` land with their kinds;
    /// application forms, literals, and `ns` forms stay out (honest
    /// minimal outline for the flat S-expression grammar — the same
    /// candidate-and-gate shape as Scheme).
    #[test]
    fn clojure_extracts_defn_and_defs() {
        let src = "(ns my.lib\n\n\n  (:require [clojure.core :as c]))\n\
                  (def ^:doc x 10)\n\
                  (defn double [v] (c/* 2 v))\n\
                  (defn- helper [] :ok)\n\
                  (defmacro my-if [cond a b] `(if ~cond ~a ~b))\n\
                  (defmulti m class)\n\
                  (defmethod m Integer [v] v)\n\
                  (defrecord Point [px py])\n\
                  (deftype Counter [n])\n\
                  (defprotocol P (get-x [this]))\n\
                  (let [v (double 2)]\n\n    (println v))\n";
        let syms = extract_symbols(LanguageId::Clojure, src);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["x", "double", "helper", "my-if", "m", "m", "Point", "Counter", "P"],
            "outline: {syms:?}"
        );
        let get = |n: &str| syms.iter().find(|s| s.name == n).expect(n);
        assert_eq!(get("x").kind, SymbolKind::Constant, "def ^:doc x");
        assert_eq!(get("double").kind, SymbolKind::Function);
        assert_eq!(get("helper").kind, SymbolKind::Function, "defn-");
        assert_eq!(get("my-if").kind, SymbolKind::Macro);
        assert_eq!(get("Point").kind, SymbolKind::Type, "defrecord");
        assert_eq!(get("Counter").kind, SymbolKind::Type, "deftype");
        assert_eq!(get("P").kind, SymbolKind::Type, "defprotocol");
        // The multimethod lands TWICE (the class+constructor precedent):
        // `defmulti m` (Macro) and `defmethod m Integer` (Method — its
        // name IS the multimethod var; the dispatch value `Integer` is
        // not in the outline).
        let ms: Vec<SymbolKind> = syms.iter().filter(|s| s.name == "m").map(|s| s.kind).collect();
        assert_eq!(ms, [SymbolKind::Macro, SymbolKind::Method], "{syms:?}");
        // `ns`, `let`, `println`, the `c/*` namespaced application, and
        // the `v` binding are NOT in the outline (head gate).
        assert_eq!(syms.len(), 9, "outline: {syms:?}");
    }

    // ── Documented empty fallback: plain text ──────────────────────────
    #[test]
    fn plain_text_has_no_query_and_empty_outline() {
        // Plain has no definition query (query_for -> None), so the outline
        // is empty. This is the documented empty fallback.
        assert!(query_for(LanguageId::Plain).is_none());
        assert!(extract_symbols(LanguageId::Plain, "some text\nfn fake() {}\n").is_empty());
    }

    // ── Per-file deadline (the file budget) ─────────────────────────────
    /// A ~330 KB source with ONE definition: the body is 15_000 plain
    /// `let`s (no written-down types — the Rust tables query matches
    /// nothing), so the extraction is PARSE-dominated: the C-level parse
    /// scales with bytes (~0.5 ms/KB even in a debug build), while the
    /// query work is a single symbol plus a 0-match tables walk. The
    /// fixture the default-budget and tiny-budget tests need — the parse
    /// alone takes far longer than a 1 ms budget (so a tiny-budget run is
    /// a real cancellation, not a file that merely finished before the
    /// deadline), yet the whole extraction sits comfortably inside its
    /// `base + rate * size_kb` deadline with margin even in a debug
    /// build under test-suite contention.
    fn one_big_function(lets: usize) -> String {
        let mut s = String::from("fn big() {\n");
        for i in 0..lets {
            s.push_str(&format!("    let _x{i} = {i};\n"));
        }
        s.push_str("}\n");
        s
    }

    /// A ~300 KB source with `n` top-level functions, each carrying a
    /// small multi-statement body: in a debug build the parse and the
    /// 4_000-match definition-query run are BOTH large (same order of
    /// magnitude — the query loop is far slower per symbol in an
    /// unoptimized build), which is the shape the shared-deadline probe
    /// needs: parse time ≥ query/2, so a `parse + query/2` probe aborts
    /// the file as a whole while every single phase fits it on its own.
    fn many_fat_functions(n: usize) -> String {
        let mut s = String::with_capacity(n * 76);
        for i in 0..n {
            s.push_str(&format!(
                "fn f{{}}() {{\n    let _a = {i};\n    let _b = {i};\n    let _c = {i};\n    let _d = {i};\n    let _e = {i};\n}}\n"
            ));
        }
        s
    }

    /// End-to-end cancellation with a TINY budget: a 1 ms deadline on a
    /// ~330 KB source must yield an aborted file (no tree / 0 symbols /
    /// 0 tables), must not panic or hang, and the surrounding extraction
    /// must complete normally. Discriminating: on the unfixed code there
    /// is no budget seam at all — this call would run the full parse and
    /// return the file's one symbol, so `syms.is_empty() && t.aborted`
    /// fails on the pre-fix behavior.
    #[test]
    fn tiny_budget_cancels_end_to_end() {
        let src = one_big_function(15_000);
        let mut t = ExtractTimings::default();
        let (syms, tables) = extract_all_timed(
            LanguageId::Rust,
            &src,
            Some(Duration::from_millis(1)),
            Some(&mut t),
        );
        assert!(
            syms.is_empty(),
            "a 1 ms budget on a ~330 KB source must cancel before any symbol lands, got {}",
            syms.len()
        );
        assert!(
            tables.fields.is_empty() && tables.impls.is_empty() && tables.bindings.is_empty(),
            "an aborted file carries no tables: {tables:?}"
        );
        assert!(
            t.aborted,
            "the 1 ms deadline must mark the file aborted (parse elapsed {:?})",
            t.parse
        );
        assert_eq!(t.budget, Duration::from_millis(1), "the injected budget is recorded");
        // The surrounding extraction completes normally (the thread-local
        // parser is not poisoned by the cancellation, nothing hung).
        let (small, _) = extract_all(LanguageId::Rust, "fn a() {}\nfn b() {}\n");
        assert_eq!(small.len(), 2, "the file right after an aborted one extracts fully");
    }

    /// A NORMAL file is unaffected by the DEFAULT budget: a ~330 KB
    /// parse-dominated source under the size-aware default (500 ms +
    /// 2.0 ms/KB ≈ 1150 ms here) yields its one symbol and does not trip
    /// the deadline — the default must keep legitimate files and abort
    /// none (with margin even in a debug build, where the Rust query
    /// loop is far slower than release).
    #[test]
    fn default_budget_does_not_abort_a_normal_file() {
        let src = one_big_function(15_000);
        let mut t = ExtractTimings::default();
        let (syms, _) = extract_all_timed(LanguageId::Rust, &src, None, Some(&mut t));
        assert!(
            !t.aborted,
            "a ~330 KB file (parse {:?} + query {:?}) must fit the default budget {:?}",
            t.parse, t.queries, t.budget
        );
        assert_eq!(syms.len(), 1, "the file's one definition lands: {syms:?}");
        assert_eq!(syms[0].name, "big");
        assert_eq!(syms[0].kind, SymbolKind::Function);
    }

    /// The budget is ONE deadline per file, not per phase: a file cannot
    /// get 2x its allowance by splitting across parse and queries.
    /// Sharedness is STRUCTURAL here (one `FileDeadline` feeds every
    /// progress callback of the file's extraction — parse and both query
    /// runs — there is no per-phase allocation in the code), so per the
    /// spec this is pinned the cheapest way that actually discriminates:
    /// a probe budget of `parse + query/2` (measured warm on the same
    /// thread) that the WHOLE file cannot fit — but under a per-phase
    /// budget every phase would fit its own fresh `parse + query/2`
    /// allowance (measured on this fixture: parse time ≥ query/2, and
    /// query ≤ parse + query/2), completing without aborting. So
    /// consumed. The probe is re-drawn up to three times, each time from a
    /// FRESH same-thread measurement of this very file (the estimate must
    /// stay seconds old, not minutes); which half of the query phase an
    /// abort lands in is load-dependent (the pinned cursor checks the
    /// deadline every 1,000 operations), so the abort is asserted, not
    /// the outline's exact length.
    #[test]
    fn the_budget_is_one_shared_deadline_not_per_phase() {
        let src = many_fat_functions(4_000);
        // Each attempt: a fresh same-thread warm run of THIS file measures
        // its parse (p) and query (q) right now; the probe budget is p +
        // q/2. The warm run uses an explicit large budget (not the
        // default): the probe pins sharedness, and a debug-build
        // query-heavy source has no room for the default's margin here
        // (that margin is pinned by
        // `default_budget_does_not_abort_a_normal_file` on the
        // parse-dominated fixture).
        // The probe: the whole file exceeds it (shared: parse + query >
        // parse + query/2 — it aborts), but a per-phase budget would let
        // every phase fit its own fresh allowance (parse ≤ probe since
        // probe ≥ parse; query ≤ parse + query/2 since parse ≥ query/2 on
        // this fixture) and complete without aborting. One probe aborts
        // unless it is ≥ 50% faster in TOTAL than the warm run measured
        // seconds earlier on the same thread; under a per-phase
        // implementation NO attempt ever aborts (each phase fits its own
        // fresh allowance), so the retry cannot mask that bug.
        let mut aborted = false;
        let mut last = ExtractTimings::default();
        let mut last_syms = 0usize;
        for _ in 0..3 {
            let mut warm = ExtractTimings::default();
            let (full, _) = extract_all_timed(
                LanguageId::Rust,
                &src,
                Some(Duration::from_secs(120)),
                Some(&mut warm),
            );
            assert!(
                !warm.aborted,
                "precondition: the warm run completes under the 120 s budget"
            );
            assert_eq!(full.len(), 4_000, "precondition: the warm run is the full outline");
            let (p, q) = (warm.parse, warm.queries);
            assert!(
                p > Duration::ZERO && q > Duration::ZERO,
                "precondition: both phases actually ran (p={p:?}, q={q:?})"
            );
            let probe = p + q / 2;
            let mut t = ExtractTimings::default();
            let (syms, _) = extract_all_timed(LanguageId::Rust, &src, Some(probe), Some(&mut t));
            last = t;
            last_syms = syms.len();
            if t.aborted {
                aborted = true;
                break;
            }
        }
        assert!(
            aborted,
            "the file's parse+query exceeds its probe budget — under the SHARED deadline the query must run out of what the parse already consumed; a per-phase budget would let every phase fit its own fresh allowance (last attempt: parse={:?}, queries={:?}, symbols={} of 4000)",
            last.parse, last.queries, last_syms
        );
    }

}

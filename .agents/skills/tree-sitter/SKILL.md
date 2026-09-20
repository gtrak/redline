---
name: tree-sitter
description: >-
  Reference for the tree-sitter 0.25.10 stack pinned in redline's Cargo.toml:
  runtime API (Parser, Language, Tree, Node, TreeCursor, Query, QueryCursor,
  InputEdit), the tree-sitter-highlight flow, the exact exports of the 17 pinned
  grammar crates behind all 19 registry languages, the dev-dependency ABI
  finding, the C#/Clojure vendored-highlights pattern, and ABI pinning rules.
  Use when writing or modifying src/syntax/ (grammar registry, highlight
  pipeline, symbol queries), touching tree-sitter dependencies in Cargo.toml,
  or debugging parse/highlight/query code.
---

# tree-sitter

Reference for the pinned tree-sitter stack. Every API fact below was verified
against the exact versions in `Cargo.toml` / `Cargo.lock`: the 0.25.10
signatures were re-verified after the ts-bump lane against the local registry
source (`~/.cargo/registry/src/…/tree-sitter-0.25.10/binding_rust/lib.rs` and
`tree-sitter-highlight-0.25.10/src`), and the 0.24-era facts carried over
unchanged — the workspace's query/highlight code compiles and its full test
suite passes against 0.25.10. The 0.25 Rust API is nearly identical to 0.24 on
the surfaces redline uses — do not trust even-older-version memory.

## Version & compatibility

| Crate | Pin | Notes |
|---|---|---|
| tree-sitter | =0.25.10 | runtime; ABI check at `set_language`; window **13..=15** |
| tree-sitter-highlight | =0.25.10 | requires tree-sitter ^0.25.10 (tracks the runtime 1:1) |
| tree-sitter-language | 0.1.8 (lock) | `LanguageFn` — ABI bridge; shared by runtime + all grammars |
| tree-sitter-rust | 0.23.3 | ABI 14; 0.24.1/0.24.2 are the 0.25-gen follow-up lane (dev req ^0.25) |
| tree-sitter-javascript | 0.23.1 | ABI 14; 0.25.0 is the 0.25-gen follow-up lane (dev req ^0.25.8) |
| tree-sitter-typescript | 0.23.2 | ABI 14; both TS and TSX grammars (0.23.2 is newest) |
| tree-sitter-python | 0.23.6 | ABI 14; 0.25.0 is the follow-up lane |
| tree-sitter-go | 0.23.4 | ABI 14; 0.25.0 is the follow-up lane |
| tree-sitter-c | 0.23.4 | ABI 14; 0.24.0–0.24.2 are the follow-up lane |
| tree-sitter-cpp | 0.23.4 | ABI 14; 0.23.4 is newest |
| tree-sitter-bash | 0.23.3 | ABI 14; 0.25.0/0.25.1 are the follow-up lane |
| tree-sitter-json | 0.24.8 | ABI 14; 0.24.8 is newest |
| tree-sitter-yaml | =0.7.0 | ABI 14; 0.7.1+ are 0.25-gen (dev req ^0.25.4) — follow-up lane |
| tree-sitter-md | 0.3.2 | ABI 13/14; 0.5.1 is ABI 15 (in-window), 0.5.2+ want ^0.26 — follow-up lane |
| tree-sitter-toml-ng | 0.7.0 | maintained TOML (^0.24-era); original `tree-sitter-toml` stuck at ^0.20; 0.7.0 is newest |
| tree-sitter-java | =0.23.5 | ABI 14 (new-languages lane); 0.23.5 is newest |
| tree-sitter-c-sharp | =0.23.1 | ABI 14; exports NO highlight constants → vendored (below); 0.23.5 is the follow-up lane |
| tree-sitter-ruby | =0.23.1 | ABI 14; 0.23.1 is newest |
| tree-sitter-scheme | =0.24.7 | ABI 14; flat S-expression grammar; 0.24.7 is newest |
| tree-sitter-clojure | =0.1.0 | **the only grammar with a NORMAL `tree-sitter` req: ^0.25.6** — the hard dep that made the 0.24.7 → 0.25.10 bump necessary; 0.1.0 is its only release; exports no highlight constants → vendored (below) |

That is 17 grammar crates behind ALL 19 registry `LanguageId`s (typescript
supplies both TS and TSX; md supplies Markdown block + inline; Plain has no
grammar).

- All grammar pins resolve against a SINGLE tree-sitter runtime (=0.25.10) —
  one runtime in the lock. A second runtime crate is the failure mode that
  produces silent `set_language` errors. The guard
  `all_grammars_set_language_succeeds` (registry.rs) runs `set_language`
  over ALL 19 `LanguageId`s and pins this.
- **The dev-dependency finding (why the bump forced zero grammar moves):** in
  every pinned grammar crate EXCEPT clojure, the `tree-sitter` (RUNTIME)
  requirement is a **dev-dependency**. Cargo does not resolve dev-deps when a
  crate is consumed as a dependency, so those reqs impose **NO resolution
  constraint** on which runtime redline links. What constrains a grammar at
  load time is ONLY the runtime's **ABI window**: `set_language` succeeds iff
  `MIN_COMPATIBLE_LANGUAGE_VERSION ≤ grammar ABI ≤ LANGUAGE_VERSION` — for
  0.25.10 that window is **13..=15**, and every pinned grammar is ABI 13 or
  14, so all 18 non-Plain grammars load unchanged. Full evidence (per-crate
  normal vs dev reqs, ABIs, registry-index source) lives in
  `docs/tree-sitter-runtime-matrix.md`.
- **Clojure is the exception:** `tree-sitter-clojure =0.1.0` declares
  `tree-sitter ^0.25.6` as a NORMAL dependency — a real resolution constraint
  (under the old 0.24.7 pin it did not even resolve: cargo `links` conflict).
  This is the unblock the bump bought.
- Exact `=` pins guard yaml/md and the new-lane grammars (java, c-sharp,
  ruby, scheme, clojure) against a bump silently changing the ABI or pulling
  a 0.25-gen release; the remaining caret pins currently land on
  0.24-compatible releases — re-check the registry-index dependency metadata
  (the matrix doc records what each 0.25-gen release looks like) before any
  bump.

## Core API

Verified signatures (tree-sitter 0.25.10):

```
Parser::new() -> Parser
Parser::set_language(&mut self, &Language) -> Result<(), LanguageError>
Parser::parse(&mut self, impl AsRef<[u8]>, Option<&Tree>) -> Option<Tree>
Language::new(LanguageFn) -> Language;  impl From<LanguageFn> for Language
Language::version(&self) -> usize   // compare vs LANGUAGE_VERSION / MIN_COMPATIBLE_LANGUAGE_VERSION
Tree::root_node(&self) -> Node
Tree::edit(&mut self, &InputEdit)
Tree::walk(&self) -> TreeCursor
Node::kind(&self) -> &'static str
Node::start_byte / end_byte / start_position / end_position / byte_range   // start_position/end_position -> Point { row, column: usize } in the 0.25.10 Rust binding — row is the 0-based line, no cast needed
Node::child_count / child(i) / named_child(i) / child_by_field_name(impl AsRef<[u8]>)
Node::is_named / is_missing / has_error / is_error / parent()
Node::utf8_text(&self, &[u8]) -> Result<&str, Utf8Error>
TreeCursor::node / goto_first_child / goto_next_sibling / goto_parent / depth / field_name
Query::new(&Language, &str) -> Result<Query, QueryError>
Query::capture_names(&self) -> &[&str]
Query::capture_index_for_name(&self, &str) -> Option<u32>
QueryCursor::new() -> QueryCursor
QueryCursor::captures(&mut self, &Query, Node, T: TextProvider) -> QueryCaptures
QueryCursor::matches(&mut self, &Query, Node, T: TextProvider) -> QueryMatches
QueryCursor::set_byte_range(Range<usize>) / set_point_range(Range<Point>) / set_max_start_depth(Option<u32>)
InputEdit { start_byte, old_end_byte, new_end_byte,
            start_position, old_end_position, new_end_position }  // Point::new(row, column)
QueryMatch { pattern_index: usize, captures: &[QueryCapture] }    // !Send !Sync
QueryCapture { node: Node<'tree>, index: u32 }                    // index into capture_names()
```

Parser, Language, Tree, Query, QueryCursor, TreeCursor, Node are all `Send + Sync`; `QueryMatch`/`QueryCaptures`
are **not** (they borrow the cursor) — extract `Node` (Copy) and indices before crossing threads.

Build + parse + incremental reparse:

```rust
use tree_sitter::{InputEdit, Language, Parser, Point};

let mut parser = Parser::new();
parser
    .set_language(&Language::from(tree_sitter_rust::LANGUAGE))
    .expect("Rust grammar ABI mismatch");
let tree = parser.parse("fn main() {}", None).expect("no language set?");
// after a buffer edit: tell the old tree what changed, then reuse it
tree.edit(&InputEdit {
    start_byte: 8,
    old_end_byte: 8,
    new_end_byte: 15,
    start_position: Point::new(0, 8),
    old_end_position: Point::new(0, 8),
    new_end_position: Point::new(0, 15),
});
let new_tree = parser.parse(new_text, Some(&tree)); // Option, not Result
```

Run a query (symbol extraction):

```rust
use streaming_iterator::StreamingIterator; use tree_sitter::{Query, QueryCursor};

let query = Query::new(&language, "(function_item name: (identifier) @def)").unwrap();
let mut cursor = QueryCursor::new();
let matches = cursor.matches(&query, tree.root_node(), source.as_bytes()); // source: &str
while let Some(m) = matches.next() {           // Item = &QueryMatch (one per definition, not per capture)
    for cap in m.captures {
        let name = query.capture_names()[cap.index as usize]; // "def"
        let node = cap.node; // Copy: kind(), start_byte(), utf8_text(source.as_bytes())
    }
}
```

- **Use `matches()`, not `captures()`, for multi-capture patterns.** `captures()`
  yields one item *per capture*, so a match capturing `@name` + `@item` would emit
  the same symbol twice. `matches()` yields one `&QueryMatch` per definition with
  all its captures in `m.captures`. For a single-capture query either works; for
  two captures (name + enclosing-item extent) `matches()` is required.

## Highlighting

```rust
use tree_sitter::Language; use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

let mut config = HighlightConfiguration::new(
    Language::from(tree_sitter_rust::LANGUAGE),
    "rust",
    tree_sitter_rust::HIGHLIGHTS_QUERY,
    tree_sitter_rust::INJECTIONS_QUERY,
    "", // locals query — "" for grammars that don't ship one
).unwrap();
config.configure(&["function", "keyword", "string", "type", "variable"]);

let mut highlighter = Highlighter::new(); // one per thread; owns a Parser
let events = highlighter
    .highlight(&config, source.as_bytes(), None, |_| None)
    .unwrap(); // Result<impl Iterator<Item = Result<HighlightEvent, Error>>, Error>
for event in events {
    match event.unwrap() {
        HighlightEvent::Source { start, end } => { /* plain text [start, end) */ }
        HighlightEvent::HighlightStart(h) => { /* face = names[h.0] */ }
        HighlightEvent::HighlightEnd => {}
    }
}
```

(tree-sitter-highlight =0.25.10; signatures re-verified in its source.)

- `HighlightConfiguration::new(language: Language, name: impl Into<String>, highlights: &str, injections: &str, locals: &str)
  -> Result<Self, QueryError>`; `configure(&mut self, &[impl AsRef<str>])`; `names() -> &[&str]`.
- `Highlighter::highlight<'a>(&'a mut self, &'a HighlightConfiguration,
  &'a [u8], Option<&'a AtomicUsize>, impl FnMut(&str) -> Option<&'a HighlightConfiguration> + 'a)`.
- `Highlight(pub usize)` — an index into the list passed to `configure`.
- `HighlightConfiguration` is `Send + Sync`, immutable after `configure` — build once per language, share everywhere.
  `Highlighter` wraps a stateful `Parser` (public field `parser`) — keep one per worker thread.
- Per-language queries ship as `&'static str` constants in the grammar crates
  (table below). The injection callback (last `highlight` arg) enables embedded languages
  (Markdown/TOML); redline v1 passes `|_| None`.

## Grammar crates

All verified against the pinned crate sources (0.25-gen stack). Every crate
exports a `LanguageFn` constant and `NODE_TYPES` (node-types.json content,
`&'static str`). The highlight constant name is **inconsistent** across
crates:

| Crate (pin) | Language constant(s) | Highlight query | Also exports |
|---|---|---|---|
| tree-sitter-rust (0.23.3) | `LANGUAGE` | `HIGHLIGHTS_QUERY` | `INJECTIONS_QUERY`, `TAGS_QUERY` |
| tree-sitter-javascript (0.23.1) | `LANGUAGE` | `HIGHLIGHT_QUERY` | `INJECTIONS_QUERY`, `LOCALS_QUERY`, `JSX_HIGHLIGHT_QUERY`, `TAGS_QUERY` |
| tree-sitter-typescript (0.23.2) | `LANGUAGE_TYPESCRIPT`, `LANGUAGE_TSX` | `HIGHLIGHTS_QUERY` (TS only) | `LOCALS_QUERY`, `TAGS_QUERY`, `TYPESCRIPT_NODE_TYPES`, `TSX_NODE_TYPES` |
| tree-sitter-python (0.23.6) | `LANGUAGE` | `HIGHLIGHTS_QUERY` | `TAGS_QUERY` |
| tree-sitter-go (0.23.4) | `LANGUAGE` | `HIGHLIGHTS_QUERY` | `TAGS_QUERY` |
| tree-sitter-c (0.23.4) | `LANGUAGE` | `HIGHLIGHT_QUERY` | `TAGS_QUERY` |
| tree-sitter-cpp (0.23.4) | `LANGUAGE` | `HIGHLIGHT_QUERY` | `TAGS_QUERY` |
| tree-sitter-bash (0.23.3) | `LANGUAGE` | `HIGHLIGHT_QUERY` | — (no TAGS_QUERY) |
| tree-sitter-json (0.24.8) | `LANGUAGE` | `HIGHLIGHTS_QUERY` | — |
| tree-sitter-yaml (=0.7.0) | `LANGUAGE` | `HIGHLIGHTS_QUERY` | — |
| tree-sitter-md (0.3.2) | `LANGUAGE` (block) + `INLINE_LANGUAGE` | `HIGHLIGHT_QUERY_BLOCK` / `HIGHLIGHT_QUERY_INLINE` | `INJECTION_QUERY_BLOCK`, `INJECTION_QUERY_INLINE`, `NODE_TYPES_BLOCK`, `NODE_TYPES_INLINE`, `MarkdownParser`, `MarkdownTree`, `MarkdownCursor` |
| tree-sitter-toml-ng (0.7.0) | `LANGUAGE` | `HIGHLIGHTS_QUERY` | — |
| tree-sitter-java (=0.23.5) | `LANGUAGE` | `HIGHLIGHTS_QUERY` | `TAGS_QUERY` |
| tree-sitter-c-sharp (=0.23.1) | `LANGUAGE` | **NONE — vendored** (see below) | — (the crate's query constants are commented out in `bindings/rust/lib.rs`) |
| tree-sitter-ruby (=0.23.1) | `LANGUAGE` | `HIGHLIGHTS_QUERY` | `LOCALS_QUERY`, `TAGS_QUERY` |
| tree-sitter-scheme (=0.24.7) | `LANGUAGE` | `HIGHLIGHTS_QUERY` | `INJECTIONS_QUERY`, `LOCALS_QUERY`, `TAGS_QUERY` (registry uses only `HIGHLIGHTS_QUERY`, passes `""` for the rest) |
| tree-sitter-clojure (=0.1.0) | `LANGUAGE` | **NONE — vendored** (see below) | — (exports only `LANGUAGE` + `NODE_TYPES`) |

- No crate exports a `language()` fn in these versions — the constant is the API (the `language()` calls in some crate doc examples are stale).
- `TAGS_QUERY` (LSP-tag-style symbol queries) is exported for rust/js/ts/python/go/c/cpp and, in the new-lane crates, java/ruby/scheme. bash/json/yaml/toml/md/c-sharp/clojure have none usable — redline writes small custom queries in `src/syntax/` for those (JSON keys, YAML keys, the lisp family's flat `define`-form outlines, C#/Clojure outlines).
- tree-sitter-md is special: a plain `Parser` + `LANGUAGE` parses block structure only. Full markdown uses its
  `MarkdownParser`, which returns a `MarkdownTree` (block tree + inline trees per block node).

## Vendored highlight queries (the C# + Clojure pattern)

Two pinned crates ship a highlights file but export NO Rust constant for it:
`tree-sitter-c-sharp` 0.23.1 (its `HIGHLIGHTS_QUERY` et al. are
**commented out** in the crate's `bindings/rust/lib.rs`) and
`tree-sitter-clojure` 0.1.0 (exports only `LANGUAGE` + `NODE_TYPES`). For
these, redline vendors the query:

- Copy the crate's `highlights.scm` **verbatim** into
  `third_party/tree-sitter-<crate>-<version>/highlights.scm`. Never edit or
  re-flow the vendored copy — it must stay byte-identical to the pinned
  crate's file.
- Pin it by **sha256 at copy time**: the sha256 of the vendored file is
  recorded in the doc comment of the `include_str!` constant in
  `src/syntax/queries.rs` (e.g. `CLOJURE_HIGHLIGHTS` pins
  `424b3b60f43cbb008c8d87730845855e0c1dde657f1a6f2e1408caf4f16914de`;
  the C# copy follows the same checksum rule at `C_SHARP_HIGHLIGHTS`).
  Re-copy + re-pin on any grammar bump.
- The constants (`C_SHARP_HIGHLIGHTS`, `CLOJURE_HIGHLIGHTS`) are passed to
  `HighlightConfiguration` exactly like a crate constant (registry.rs);
  `all_grammars_have_configs` pins that both vendored queries still compile.

## Usage in redline

Maps to plan 001 issues 03/04/05:

- `src/syntax/` registry (issue 03) is the only module touching grammar crates:
  one entry per language — `LanguageFn` constant, highlight query (crate
  constant or vendored), injections query (or `""`), symbol query; plain-text fallback.
- Highlight pipeline: build one `HighlightConfiguration` per language at startup,
  `configure` with the recognized-name list from config, cache highlight events keyed by
  (path, mtime, theme). `Highlighter` per worker.
- Symbol index (issue 05): shared `Query` per language (Send+Sync, built once); each rayon thread owns a `QueryCursor`
  (stateful, not shared). Prefer each crate's `TAGS_QUERY` where available; custom queries for bash/json/yaml/toml/md
  and the no-export lisp family (c-sharp/clojure outlines are custom; scheme's flat define forms are captured
  candidate-and-gated, as is clojure's).
- File watching (issue 04): on external change, `Tree::edit(&InputEdit {..})`
  then `parse(new_text, Some(&old_tree))` for the incremental reparse.

## Gotchas

- **`set_language` takes `&Language`, not `LanguageFn`.** Convert with `Language::from(grammars::LANGUAGE)`. ABI mismatch
  returns `Err(LanguageError)` — compare `Language::version()` against `LANGUAGE_VERSION` / `MIN_COMPATIBLE_LANGUAGE_VERSION`
  (0.25.10 window: 13..=15).
- **`QueryCapture.index`** (0.24/0.25) — older versions called this `name_index`.
- **`Node::utf8_text(source)`** — the old `Node::text(source)` is gone.
- **Query iterators are `StreamingIterator`, not std `Iterator`.** `QueryCaptures`/`QueryMatches` come from the
  `streaming-iterator` crate; need `use streaming_iterator::StreamingIterator;` for `.next()`. `QueryCaptures` yields
- **`QueryMatches` vs `QueryCaptures` item types differ** (0.25.10): `captures().next()`
  yields `(QueryMatch, usize)` (a tuple, per capture); `matches().next()` yields
  `&QueryMatch` (a reference, per match) — use `matches()` for definition queries
  that capture a name and its enclosing item (see "Run a query"). `QueryMatch` is !Send/!Sync.
- **`captures`/`matches` take a third `TextProvider` argument** (pass
  `source.as_bytes()`; the blanket impl covers `&[u8]`).
- **`parse` returns `Option<Tree>`**, not `Result` — `None` on timeout,
  cancellation, or no language set.
- **Highlight constant naming**: `HIGHLIGHT_QUERY` (js/c/cpp/bash) vs
  `HIGHLIGHTS_QUERY` (rust/ts/python/go/json/yaml/toml-ng/java/ruby/scheme). c-sharp
  and clojure export NO highlight constant at all — vendored (above). Don't guess.
- **`Highlighter::highlight` takes `&[u8]`**, not `&str`.
- The tree-sitter-highlight crate-level doc example (uses `tree_sitter_javascript::language()`, older crate versions)
  is stale — don't copy it verbatim. tree-sitter-c's doc prose also mentions a `language()` fn its item list does not expose.
- tree-sitter-md with a plain `Parser` gives block-level structure only —
  inline tokens (links, code spans) need `INLINE_LANGUAGE`/`MarkdownParser`.
- Bumping a grammar crate silently changes the ABI if the new release requires tree-sitter ^0.25+
  (cargo pulls a second runtime) — remember the dev-dependency finding: only a NORMAL
  runtime req (clojure is the only one today) constrains resolution; an in-window
  ABI does not. Keep the `=` pins (yaml, md, java, c-sharp, ruby, scheme, clojure)
  and re-check the registry-index metadata before any bump; the 0.25-gen follow-up
  releases (js 0.25.0, c-sharp 0.23.5, …) are recorded — deliberately not taken —
  in `docs/tree-sitter-runtime-matrix.md`.

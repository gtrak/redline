---
name: tree-sitter
description: >-
  Reference for the tree-sitter 0.24.7 stack pinned in redline's Cargo.toml:
  runtime API (Parser, Language, Tree, Node, TreeCursor, Query, QueryCursor,
  InputEdit), the tree-sitter-highlight flow, the exact exports of the 12 pinned
  grammar crates, and ABI pinning rules. Use when writing or modifying src/syntax/
  (grammar registry, highlight pipeline, symbol queries), touching tree-sitter
  dependencies in Cargo.toml, or debugging parse/highlight/query code.
---

# tree-sitter

Reference for the pinned tree-sitter stack. Every API fact below was verified against
docs.rs for the exact version in `Cargo.toml` / `Cargo.lock` (2026-09-16). The 0.24
API differs from pre-0.24 in several places (see Gotchas) — do not trust older-version memory.

## Version & compatibility

| Crate | Pin | Notes |
|---|---|---|
| tree-sitter | 0.24.7 | runtime; ABI check at `set_language` |
| tree-sitter-highlight | 0.24.7 | requires tree-sitter ^0.24.5 |
| tree-sitter-language | 0.1.8 (lock) | `LanguageFn` — ABI bridge; shared by runtime + all grammars |
| tree-sitter-rust | =0.24.0 | 0.24.1+ require tree-sitter ^0.25! |
| tree-sitter-javascript | 0.23.1 | |
| tree-sitter-typescript | 0.23.2 | both TS and TSX grammars |
| tree-sitter-python | 0.23.6 | |
| tree-sitter-go | 0.23.4 | |
| tree-sitter-c | 0.23.4 | |
| tree-sitter-cpp | 0.23.4 | |
| tree-sitter-bash | 0.23.3 | |
| tree-sitter-json | 0.24.8 | |
| tree-sitter-yaml | =0.7.0 | 0.7.1+ require tree-sitter ^0.25.4! |
| tree-sitter-md | =0.5.1 | 0.5.2+ require tree-sitter ^0.26! |
| tree-sitter-toml-ng | 0.7.0 | maintained TOML (^0.24); original `tree-sitter-toml` stuck at ^0.20 |

- All grammar pins resolve against a SINGLE tree-sitter runtime (0.24.7). Bumping a grammar to a release requiring
  `^0.25`/`^0.26` pulls a second runtime crate — incompatible ABIs are how you get silent `set_language` failures.
- rust/yaml/md use exact `=` pins because their next release already moved to a newer runtime requirement; other
  caret pins currently land on ^0.24 — re-check crates.io dependency metadata before any bump.

## Core API

Verified signatures (tree-sitter 0.24.7):

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
Node::start_byte / end_byte / start_position / end_position / byte_range
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
let captures = cursor.captures(&query, tree.root_node(), source.as_bytes()); // source: &str
while let Some((m, _)) = captures.next() {          // Item = (QueryMatch, usize)
    for cap in m.captures {
        let name = query.capture_names()[cap.index as usize]; // "def"
        let node = cap.node; // Copy: kind(), start_byte(), utf8_text(source.as_bytes())
    }
}
```

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

- `HighlightConfiguration::new(language: Language, name: impl Into<String>, highlights: &str, injections: &str, locals: &str)
  -> Result<Self, QueryError>`; `configure(&mut self, &[impl AsRef<str>])`; `names() -> &[&str]`.
- `Highlighter::highlight<'a>(&'a mut self, &'a HighlightConfiguration,
  &'a [u8], Option<&'a AtomicUsize>, impl FnMut(&str) -> Option<&'a HighlightConfiguration> + 'a)`.
- `Highlight(pub usize)` — an index into the list passed to `configure`.
- `HighlightConfiguration` is `Send + Sync`, immutable after `configure` — build once per language, share everywhere.
  `Highlighter` wraps a stateful `Parser` (public field `parser`) — keep one per worker thread.
- Per-language queries ship as `&'static str` constants in the grammar crates (table
  below). The injection callback (last `highlight` arg) enables embedded languages
  (Markdown/TOML); redline v1 passes `|_| None`.

## Grammar crates

All verified against docs.rs. Every crate exports a `LanguageFn` constant and
`NODE_TYPES` (node-types.json content, `&'static str`). The highlight constant name is **inconsistent** across crates:

| Crate (pin) | Language constant(s) | Highlight query | Also exports |
|---|---|---|---|
| tree-sitter-rust (=0.24.0) | `LANGUAGE` | `HIGHLIGHTS_QUERY` | `INJECTIONS_QUERY`, `TAGS_QUERY` |
| tree-sitter-javascript (0.23.1) | `LANGUAGE` | `HIGHLIGHT_QUERY` | `INJECTIONS_QUERY`, `LOCALS_QUERY`, `JSX_HIGHLIGHT_QUERY`, `TAGS_QUERY` |
| tree-sitter-typescript (0.23.2) | `LANGUAGE_TYPESCRIPT`, `LANGUAGE_TSX` | `HIGHLIGHTS_QUERY` (TS only) | `LOCALS_QUERY`, `TAGS_QUERY`, `TYPESCRIPT_NODE_TYPES`, `TSX_NODE_TYPES` |
| tree-sitter-python (0.23.6) | `LANGUAGE` | `HIGHLIGHTS_QUERY` | `TAGS_QUERY` |
| tree-sitter-go (0.23.4) | `LANGUAGE` | `HIGHLIGHTS_QUERY` | `TAGS_QUERY` |
| tree-sitter-c (0.23.4) | `LANGUAGE` | `HIGHLIGHT_QUERY` | `TAGS_QUERY` |
| tree-sitter-cpp (0.23.4) | `LANGUAGE` | `HIGHLIGHT_QUERY` | `TAGS_QUERY` |
| tree-sitter-bash (0.23.3) | `LANGUAGE` | `HIGHLIGHT_QUERY` | — (no TAGS_QUERY) |
| tree-sitter-json (0.24.8) | `LANGUAGE` | `HIGHLIGHTS_QUERY` | — |
| tree-sitter-yaml (=0.7.0) | `LANGUAGE` | `HIGHLIGHTS_QUERY` | — |
| tree-sitter-md (=0.5.1) | `LANGUAGE` (block) + `INLINE_LANGUAGE` | `HIGHLIGHT_QUERY_BLOCK` / `HIGHLIGHT_QUERY_INLINE` | `INJECTION_QUERY_BLOCK`, `INJECTION_QUERY_INLINE`, `NODE_TYPES_BLOCK`, `NODE_TYPES_INLINE`, `MarkdownParser`, `MarkdownTree`, `MarkdownCursor` |
| tree-sitter-toml-ng (0.7.0) | `LANGUAGE` | `HIGHLIGHTS_QUERY` | — |

- No crate exports a `language()` fn in these versions — the constant is the API (the `language()` calls in some crate doc examples are stale).
- `TAGS_QUERY` (LSP-tag-style symbol queries) exists only for rust/js/ts/python/go/c/cpp. bash/json/yaml/toml/md have
  none — write small custom queries (e.g. JSON keys, YAML keys) in `src/syntax/` for those.
- tree-sitter-md is special: a plain `Parser` + `LANGUAGE` parses block structure only. Full markdown uses its
  `MarkdownParser`, which returns a `MarkdownTree` (block tree + inline trees per block node).

## Usage in redline

Maps to plan 001 issues 03/04/05:

- `src/syntax/` registry (issue 03) is the only module touching grammar crates:
  one entry per language — `LanguageFn` constant, highlight query, injections query
  (or `""`), symbol query; plain-text fallback.
- Highlight pipeline: build one `HighlightConfiguration` per language at startup,
  `configure` with the recognized-name list from config, cache highlight events keyed by
  (path, mtime, theme). `Highlighter` per worker.
- Symbol index (issue 05): shared `Query` per language (Send+Sync, built once); each rayon thread owns a `QueryCursor`
  (stateful, not shared). Prefer each crate's `TAGS_QUERY` where available; custom queries for bash/json/yaml/toml/md.
- File watching (issue 04): on external change, `Tree::edit(&InputEdit {..})`
  then `parse(new_text, Some(&old_tree))` for the incremental reparse.

## Gotchas

- **`set_language` takes `&Language`, not `LanguageFn`.** Convert with `Language::from(grammars::LANGUAGE)`. ABI mismatch
  returns `Err(LanguageError)` — compare `Language::version()` against `LANGUAGE_VERSION` / `MIN_COMPATIBLE_LANGUAGE_VERSION`.
- **`QueryCapture.index`** (0.24) — older versions called this `name_index`.
- **`Node::utf8_text(source)`** — the old `Node::text(source)` is gone.
- **Query iterators are `StreamingIterator`, not std `Iterator`.** `QueryCaptures`/`QueryMatches` come from the
  `streaming-iterator` crate; need `use streaming_iterator::StreamingIterator;` for `.next()`. `QueryCaptures` yields
  `(QueryMatch, usize)`; `QueryMatch` is !Send/!Sync.
- **`captures`/`matches` take a third `TextProvider` argument** (pass
  `source.as_bytes()`; the blanket impl covers `&[u8]`).
- **`parse` returns `Option<Tree>`**, not `Result` — `None` on timeout,
  cancellation, or no language set.
- **Highlight constant naming**: `HIGHLIGHT_QUERY` (js/c/cpp/bash) vs
  `HIGHLIGHTS_QUERY` (rust/ts/python/go/json/yaml/toml-ng). Don't guess.
- **`Highlighter::highlight` takes `&[u8]`**, not `&str`.
- The tree-sitter-highlight crate-level doc example (uses `tree_sitter_javascript::language()`, older crate versions)
  is stale — don't copy it verbatim. tree-sitter-c's doc prose also mentions a `language()` fn its item list does not expose.
- tree-sitter-md with a plain `Parser` gives block-level structure only —
  inline tokens (links, code spans) need `INLINE_LANGUAGE`/`MarkdownParser`.
- Bumping a grammar crate silently changes the ABI if the new release requires tree-sitter ^0.25+
  (cargo pulls a second runtime). Keep the `=` pins on rust/yaml/md and re-check crates.io metadata before any bump.

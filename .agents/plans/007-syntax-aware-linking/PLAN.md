# 007 — Redline: syntax-aware linking (tree-sitter beyond highlighting)

Status: planned (roadmap; not yet scheduled)
Phases: 1 · Issues: 01–04
Depends on: plan 005 (annotations) for 02; plan 006 (resolver chain) for 03
Origin: user question 2026-09-18 — "could our linking benefit from
tree-sitter concepts?" — with the answer grounded in the existing tree
(`src/syntax/queries.rs`, `src/nav/index.rs`).

## Why

Redline already uses tree-sitter, but only as a **name → location table**:
`query_for(lang)` (13 languages) captures `@name`/`@item`, producing
`Symbol {name, kind, line, end_line, start_byte, end_byte}`
(`src/syntax/queries.rs:58-70`), indexed project-wide by rayon
(`src/nav/index.rs`) and consumed for Xref/imenu/which-function. Three
capabilities are currently *blocked on* the missing piece — a query for
**the syntax node at a byte offset** (and its enclosing scope):

1. **M-. symbol extraction is text-splitting, not syntax.** `xref_find_definitions`
   tokenizes the line on `!is_alphanumeric() && != '_'`, so `tokio::spawn`
   becomes two unrelated candidates (`tokio`, `spawn`). This is the same
   seam plan 006-02 must cross to resolve external paths.
2. **Annotations anchor to line text, not syntax.** `.redline-notes.md`
   stores `anchor: target_one();` — a *string*. Any reformat (rustfmt),
   insertion, or rename above it orphans the note; the ±25-line search is a
   heuristic band-aid. A syntax anchor (`function_item` named
   `target_one`) is stable under all three.
3. **The resolver chain refuses bare symbols.** All four language providers
   answer a bare symbol (`Deserialize`) with "needs scope info
   (tree-sitter)" — literally waiting for scope resolution from a syntax
   tree (walk to the `use` declaration / enclosing `impl`).
4. **Every edit re-parses from scratch.** The highlight pipeline returns
   `HighlightResult` and discards the `Tree` (`src/syntax/highlight.rs:69`),
   so `invalidate_highlight_for_key` clears the whole cache and
   `ensure_highlight` re-parses the file. `Tree::edit(InputEdit)` +
   re-parse would reuse the unchanged prefix/suffix.

## What

1. **Issue 01 — node-at-point + enclosing scope (the foundation).**
   A per-language query for the identifier/scoped-identifier node
   containing a byte offset, plus an enclosing-scope resolver (function,
   impl, module, `use` path). Expose a plain-Rust API
   (`syntax::node_at(lang, source, byte) -> Option<NodeInfo>` with
   `{text, kind, start_byte, end_byte, scope_path}`) usable by the app and
   by `redline-resolve` inputs. Rust first; other languages fall back to
   `None` so callers degrade to today's behavior.
2. **Issue 02 — syntax-anchored annotations.** Extend the annotation record
   with an optional syntax anchor `{kind, name}` captured at creation from
   issue 01. Re-anchoring prefers the syntax anchor (find the node by kind
   + name anywhere in the file), falls back to the existing line-text
   `anchor` + ±25 search, then to `orphaned`. This is the "automatic
   linking machinery" the user asked for, and it survives reformatting and
   insertion. Serialization stays tolerant (unknown keys preserved).
3. **Issue 03 — scope-aware resolution for the resolver chain.** Feed
   `scope_path` into `SymbolContext` so bare symbols resolve
   (`Deserialize` → `serde::Deserialize` via the `use` path) and remove the
   "needs scope info" refusal where a language has an implementation.
   Depends on 006-01's provider trait (already landed, app-free crate).
4. **Issue 04 — incremental parse reuse (perf).** Retain the `Tree` per
   buffer, apply `Tree::edit(InputEdit)` on each edit, and re-parse
   incrementally; keep the highlight pipeline's output identical (the tree
   is an internal cache). Measure on a large file in edit mode.

## Key decisions

- **Rust first, graceful degradation elsewhere.** Each capability is
  per-language; the 13-language breadth multiplies work by 13 for a repo
  that is Rust. Non-matching languages keep today's behavior (line-text
  anchors, text-split symbols) rather than breaking.
- **Queries live with the existing ones** in `src/syntax/queries.rs`,
  verified against the pinned grammar versions (the skill's ABI rules).
- **Plans 006 and 007 overlap deliberately at issue 03**: 006-02 wires the
  provider chain; 007-03 gives it the scope info it needs to accept bare
  symbols. Sequence 006-02 first (it works for path-shaped symbols), then
  007-03.
- **Not a one-way door**: each issue degrades to current behavior, so any
  subset can ship.

## Success criteria

- M-. on `tokio::spawn` extracts ONE path-shaped symbol (not two
  fragments) and resolves through the provider chain (006-02 + 007-01/03).
- An annotation survives `rustfmt` reformatting and a 100-line insertion
  above it, without orphaning (007-02).
- A bare `Deserialize` resolves via its `use` declaration (007-03).
- Editing a large file re-parses incrementally (007-04; measure).

## Task order

| Phase | Issue | Depends on |
|---|---|---|
| 1 | 01 node-at-point + scope | — |
| 1 | 02 syntax anchors | 01, plan 005 |
| 1 | 03 scope-aware resolution | 01, plan 006-01 |
| 2 | 04 incremental parse | — (perf, independent) |

## Issue index

- 01 — node-at-point and enclosing scope (the foundation)
- 02 — syntax-anchored annotations
- 03 — scope-aware resolution for the resolver chain
- 04 — incremental parse reuse (perf)

When complete, archive per plan-process.

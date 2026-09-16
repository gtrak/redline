# 05 — Symbols & jump navigation

Phase 3 · Navigate · Depends on: 03, 04

## Objective

Emacs navigation across the whole project: imenu outline, `M-.` jump to
definition, `M-,` pop, a project-wide symbol picker with preview, and
which-function in the status line.

## Key decisions

- **Background definition index**: ignore-aware walk + per-language
  tree-sitter symbol queries, parsed rayon-parallel; in-memory per session,
  refreshed incrementally from watcher events (only changed files reparse).
- Navigation sits behind an **`Xref` trait** (tree-sitter backend now, LSP
  later).
- Ambiguous or fuzzy lookups route through the Picker.

## Files

| Area | Change |
|---|---|
| `src/nav/index.rs` | background indexer, per-file outlines |
| `src/nav/xref.rs` | `Xref` trait + tree-sitter backend |
| `src/syntax/queries/` | definition queries per language |
| `src/app/store.rs` | jump stack |
| `src/ui/` | symbol picker source; which-function in status line |
| `src/app/commands.rs` | `M-.`, `M-,`, `C-i`, `M-i` |

## Steps

1. Definition queries per language (functions, methods, types, constants,
   macros, …).
2. Background indexer with progress indicator in the status line; UI never
   blocks.
3. imenu (`M-i`): nested outline of the current file.
4. `M-.`: symbol under point → definition (unique) or Picker (ambiguous).
5. Jump stack: `M-,` back, `C-i` forward; every navigation records an entry.
6. Symbol Picker over all project symbols, fuzzy, preview at definition.
7. which-function: enclosing symbol name in the status line.
8. Incremental reindex on watcher events.

## Verification

- On a large Rust/TS/Python repo: `M-.` from a call site lands on the
  definition (cross-file); `M-,` returns to the exact prior position.
- `M-i` lists nested symbols; `RET` jumps.
- Symbol picker finds symbols by fuzzy prefix; preview shows definition
  context.
- Status line shows the enclosing function while scrolling.
- After an agent edits one file, only that file's index entries refresh —
  no full rebuild.

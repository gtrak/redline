# 03 — Syntax & file view

Phase 2 · Browse · Depends on: 02

## Objective

The reading surface: tree-sitter-highlighted file view with fast scrolling
over very large files, incremental in-buffer search (`C-s`), and emacs
motion commands.

## Key decisions

- **Grammar registry** pins all grammars + queries in one module —
  tree-sitter API churn is isolated there.
- **Highlight cache** keyed by (path, mtime, theme); rendered window =
  visible lines ± margin (virtualization), so memory stays bounded.
- **ropey** buffer per open file; files >~10MB render as plain text
  (graceful degradation, never a hang).
- Theme = face map (token → color/style), driven from config.

## Files

| Area | Change |
|---|---|
| `src/syntax/` | registry (Rust, TS/TSX, JS, Python, Go, C/C++, TOML, JSON, YAML, Bash, Markdown), highlight pipeline, per-language query files |
| `src/model/buffer.rs` | ropey-backed text buffer |
| `src/ui/file_view.rs` | virtualized file view component |
| `src/app/commands.rs` | motion commands, isearch, goto-line |

## Steps

1. Grammar registry for the polyglot set + plain-text fallback.
2. ropey buffer + highlight pipeline + highlight cache.
3. Virtualized FileView; smooth scroll (line/half/page, `g`/`G`, `M-g g`,
   `M-<`/`M->`).
4. Theme faces wired from config.
5. Incremental isearch: `C-s`/`C-r`, `n`/`N`, match count in minibuffer.
6. Big-file fallback path.

## Verification

- Sample files in all 11 languages render with correct highlighting and
  theme colors.
- A 50k-line file scrolls with no visible lag; memory stays bounded.
- `C-s` is incremental with live counts and wrap; `n`/`N` navigate; exits
  cleanly.
- Highlight cache invalidates when a file changes on disk (pre-watcher: on
  reopen).

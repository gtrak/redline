# 06 — Search & references

Phase 3 · Navigate · Depends on: 05

## Objective

Project-wide search (projectile-ag), symbol references (`M-?`), and occur
(`M-s o`) — streaming, cancelable, and jumpable.

## Key decisions

- **Embedded ripgrep crates** (`grep-searcher`, `grep-regex`, `ignore`):
  single binary, no PATH dependence, cancelable result streams.
- Results view = grouped, counted, jumpable; `RET` records jump-stack
  entries so `M-,` returns to the results.
- **References** = word-boundary search filtered by tree-sitter token class
  (comment/string hits dropped) where a grammar exists; plain search
  fallback elsewhere.

## Files

| Area | Change |
|---|---|
| `src/search/rg.rs` | streaming, cancelable search pipeline |
| `src/search/references.rs` | `M-?` with token-class filtering |
| `src/search/occur.rs` | per-buffer occurrences |
| `src/ui/results_view.rs` | grouped results view |
| `src/app/commands.rs` | `M-?`, `C-c p s s`, `M-s o` |

## Steps

1. Streaming rg pipeline (pattern, path/glob, type filters, cancel).
2. Results view: file groups, match lines, running counts, live updates.
3. Jump-from-results via jump stack; `g` re-runs; `n`/`p` between matches.
4. References (`M-?`) with token-class filtering; results view reuse.
5. Occur for the current buffer.

## Verification

- Searching a large repo shows first hits immediately; `ESC` cancels
  cleanly mid-search.
- `M-?` on a common identifier returns a plausible reference set with no
  comment/string hits in supported languages.
- `RET` jumps and `M-,` returns to results; counts match the `rg` CLI for
  the same query.
- Occur lists all in-file matches with counts.

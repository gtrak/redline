# 03 — node-at-point per language

Phase 2 · Depends on: 01

## Objective

007-01's `node_at`/`scope_path_at` is Rust-only by design. Add the identifier
predicates + enclosing-scope walk for js/ts/python/go behind the same
per-language extension point.

## Key decisions

- One predicate fn + one scope walk per language, in `src/syntax/node.rs`
  (the frozen Rust behavior is not modified).
- Each language independently returns None/[] where unimplemented.
- Verified against the pinned grammars (skills are truth; no registry reads).

## Files

| File | Change |
|---|---|
| `src/syntax/node.rs` | per-language predicates + scope walks |
| tests | per-language node/scope cases (scoped paths come back whole) |

## Verification

- Per-language unit tests mirroring 007-01's Rust set (whole scoped path,
  field access, type position, scope chain, boundaries, broken source).

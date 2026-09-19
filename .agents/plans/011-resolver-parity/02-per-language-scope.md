# 02 — per-language scope hints (bare symbols)

Phase 1 · Depends on: 01, 007-03

## Objective

Extend 007-03's scope/import hint beyond Rust so a BARE symbol resolves:
JS `import … from`, Python `import …`/`from … import`, Go `import` blocks.

## Key decisions

- A bounded per-language import-declaration walk (mirrors the Rust one);
  absent → the existing accurate bail, never a guess.
- `use x as y`-style aliases resolve to the aliased path per language.
- Independently degradable per language.

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | per-language import walk feeding the scope hint |
| `crates/redline-resolve/src/providers/*` | consume the hint where a bare symbol now resolves |

## Verification

- Bare symbol resolves per language where a declaration exists; stdlib/prelude
  not guessed; no-declaration still bails with the existing message.

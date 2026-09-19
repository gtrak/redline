# 04 — per-language source index

Phase 2 · Depends on: 01

## Objective

006-03's crate index walks `**/*.rs`. Generalize to the owning language's file
extensions so M-./imenu work INSIDE a landed JS/Python/Go dependency (the
"follow types inside a library buffer" capability, extended).

## Key decisions

- Extension set derived from the resolved source's language, not hardcoded.
- The cache/LRU/indicator machinery from 006-03 is reused unchanged.
- The >N-file refusal cap still applies.

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | extension-aware walk for the crate index |
| tests | index builds for a non-Rust source tree; M-. inside it |

## Verification

- Unit: index for a synthetic JS/Python tree; same-file-first ordering.
- PTY: land in a non-Rust dependency, M-. again inside the crate.

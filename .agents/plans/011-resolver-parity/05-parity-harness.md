# 05 — parity harness + provider matrix

Phase 2 · Depends on: 01–04

## Objective

Lock parity in with drives, and document exactly what resolves per language.

## Key decisions

- One PTY drive per language (own repo + own per-repo flock — the
  `drive_external_crate.py` pattern), skipped-with-a-loud-note when the
  toolchain is unavailable rather than silently passing.
- A `docs/` provider matrix: per language, what resolves today (path-shaped /
  bare / in-library) — the honest answer to "do we have parity?".

## Files

| File | Change |
|---|---|
| `tools/drive_*_resolve.py` | per-language drives |
| `docs/` | provider matrix |
| `tools/gate.sh` | register the new drives |

## Verification

- Each drive green on a real toolchain, or explicitly reported unavailable.

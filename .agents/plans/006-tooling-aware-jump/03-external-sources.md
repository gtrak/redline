# 02 — external source polish

Phase 1 · Depends on: 01

## Objective

External source trees (registry sources) behave well once jumped into.

## Key decisions

- External sources open read-only; edit mode (plan 005) is unavailable
  there by default (registry files are cargo-managed).
- On-demand symbol indexing for the external tree (follow-up jumps inside
  tokio land correctly); indexes are per-session, evictable.
- Recents/history handle out-of-workspace paths without polluting the
  project registry (per-session list only).

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | external-path classification (read-only, per-session recents), on-demand index. |
| tests + tools/ | follow-up jump inside the fetched crate; recents isolation. |

## Verification

- Gates green; PTY: M-. → M-. inside tokio lands; buffers list shows the
  external file; project recents/registry untouched after quit.

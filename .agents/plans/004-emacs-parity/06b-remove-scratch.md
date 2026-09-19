# 06b — remove the remaining scratch affordances

Phase 1 · Depends on: 06a

## Objective

Second half of 004-06, after 06a lands the empty table + Home view: retire the
remaining `*scratch*` affordances and update everything that assumed them.

## Key decisions

- The explicit `open-scratch` command's fate is decided here (keep as a
  non-default entry, or drop it) — state the choice and why.
- Flow key expectations, buffer-list counts, and the parity log's
  splash-divergence row (KEEP, deliberate) are updated here.
- `q`-on-home rules are confirmed/refined against emacs parity.

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | retirement of scratch entry points; `kv`/list counts |
| `tools/` flows | buffer-list/picker count expectations |
| `docs/emacs-parity-log.md` | the splash divergence row (KEEP) |

## Verification

- No `*scratch*` reachable by default; counts updated and explained.
- `tools/gate.sh full` green.

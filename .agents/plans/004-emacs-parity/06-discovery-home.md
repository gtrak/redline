# 06 — discovery home (drop *scratch*)

Phase 1 · Depends on: 04 (serial on store.rs)

## Objective

User directive 2026-09-17: "I think we also need a top-level hydra menu
when no buffer is open to discover all the functionality. We don't need
*scratch*." When no buffer is open, show a discovery home — the derived
transient-menu machinery as a full top-level map — instead of an empty
*scratch* buffer.

## Key decisions

- **No default *scratch* buffer**: a fresh session (and any state where
  the buffer list is empty) renders the HOME view; it is not a buffer
  (nothing enters the buffer list/recents).
- **Derived, anti-drift**: home content is generated from the live keymap×
  registry (the transient-menu engine from 002-01) — all top-level groups
  at once: single keys, prefix groups with their members, chords. Never a
  hand-maintained string.
- Header row: project name + dirty counts + "redline"; footer: the
  standard help line (C-x C-c quits, ? opens the descendable menu).
- Opening anything (file, magit, search, notes via C-x n) replaces home.
  q unbound on home; C-x C-c quits (immediate — nothing to save when no
  buffers, per 004-04 semantics).
- Parity log: divergence from emacs splash is deliberate (KEEP) — log it.

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | drop scratch auto-create; empty-stack render state (home); menu derivation exposure for home. |
| `src/ui/` (new home view or root arm) | home render from derived groups. |
| `src/ui/transient_menu.rs` | reuse/extend the derived-menu renderer for the all-groups layout. |
| tests + tools/ | home renders derived groups (anti-drift test: registry change changes home); opening C-x C-f from home; C-x n from home; q-quit suite updated (q unbound on home). |

## Verification

- Gates green (test count changes: scratch-related tests updated); boot →
  home with derived groups; every listed binding works from home; q-quit
  and C-x C-c semantics per above; sweep_flows updated for the new boot
  state, 43+ flows green.

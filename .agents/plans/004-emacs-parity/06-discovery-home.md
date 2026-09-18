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

## Orchestrator pre-check (2026-09-18, against committed tree)

- **Derive machinery exists**: `menu_entries_for_path(&KeySeq)`,
  `menu_entries()`, `menu_rows()`, `menu_height()`, `menu_bindings()` in
  `store.rs` (~4034-4195) already turn the live keymap × command registry
  into grouped rows. Home reuses exactly this — call it with an "all
  top-level groups" query rather than a path; do not hand-write content.
- **`open_scratch` call sites** (3 real ones): command registration
  (`command.rs:154`), `kill_buffer` when the last buffer dies
  (`store.rs:2602`), a stale-reference fallback in the buffer-list open
  path (`store.rs:5411`), plus the boot path. Dropping `*scratch*` means
  those fallbacks must render HOME instead (empty stack = home), not
  create a buffer. Beware: tests call `store.open_scratch()` directly
  (`store.rs:7503`) — the helper may stay (an explicit command) while the
  *auto-create* paths change.
- **`*scratch*` is also a buffer key** (`Buffers` docs ~363) and appears
  in buffer-list expectations (e.g. `C-x b` picker "3 of 3" including
  `*scratch*` in the parity log). Those assertions must move to the home
  state (no scratch entry).
- `q` is currently bound to `quit` only in the transient/list contexts;
  on home it must be unbound (verify where the bare `q` → quit binding
  lives so home does not inherit it).

## Verification

- Gates green (test count changes: scratch-related tests updated); boot →
  home with derived groups; every listed binding works from home; q-quit
  and C-x C-c semantics per above; sweep_flows updated for the new boot
  state, 43+ flows green.

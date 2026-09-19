# 06a — empty buffer table + HOME view

Phase 1 · Depends on: 04 (serial on store.rs)

## Objective

First half of the original 004-06 (split per that spec's own instruction —
the orchestrator pre-check confirmed it is too large for one issue): the
buffer table starts EMPTY, `current` is `None`, and a new `ViewId::Home`
renders the derived-groups discovery home. `*scratch*` is no longer
auto-created; home is not a buffer.

## Key decisions

- Split boundary: **06a owns boot state + Home render**; 06b owns removing
  the remaining scratch affordances, flow key expectations, the parity-log
  splash-divergence row, and `q`-on-home rules.
- Home content is DERIVED from the live keymap × command registry (the 002-01
  transient-menu machinery) — never a hand-maintained list; an anti-drift test
  proves it.
- `SCRATCH_NAME` / explicit scratch creation remain available (06b decides the
  command's fate); only the AUTO-create at boot is removed.
- `q` is unbound on home; `C-x C-c` quits immediately (no buffers ⇒ nothing to
  save, per 004-04).

## Files

| File | Change |
|---|---|
| `src/model/buffer.rs` | `BufferTable::new()` starts empty, `current: None` |
| `src/app/store.rs` | `ViewId::Home` (+ name/keymap), all-groups menu query, boot-`None` audits |
| `src/ui/root.rs` (and/or a home module) | the Home render arm |
| `src/ui/transient_menu.rs` | reuse/extend the derived renderer |
| tests + `tools/` | anti-drift test; boot-home flow; scratch-assertion updates |

## Verification

- Anti-drift test (registry mutation changes home).
- PTY: boot renders home with real derived groups; entry points work from home
  and replace it; `q` no-op; `C-x C-c` immediate; `?` opens the menu;
  `*scratch*` absent at boot.
- `tools/gate.sh full` green with every changed count explained.

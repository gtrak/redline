# Task: window splits (C-x 2 / C-x 1 / C-x 0) — the pre-existing ux_sweep findings

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `.agents/skills/*.md` and `docs/ux-testing-plan.md`'s known-issue
watchlist (the 3 pre-existing ux_sweep findings: unbound C-x 2/1/0).

## Origin

Every ux_sweep run since plan 002 reports 3 findings: C-x 2 (split
window), C-x 1 (close all but current), C-x 0 (close current) are
UNBOUND — the keymap table has `close_view` machinery (the M-. picker
path closes views) but no split concept. Emacs parity (plan 004's frame)
wants these; the user's UX-testing loop keeps surfacing them.

## What to build

1. **Read the existing windowing model FIRST** (`src/app/store.rs`'s
   view stack / ViewId, `drive_windowing(+panes)` suites, and how the
   picker/search views stack) — redline has a VIEW STACK, not a split
   layout. Decide honestly: do C-x 2/1/0 map onto the existing stack
   (e.g. C-x 2 = duplicate the current buffer into a second view entry?
   a vertical split is a RENDER concept the root renderer may not have)
   or is a split-layout model needed? The renderer (iocraft) supports
   layout — but the honest scope decision is yours: (a) full vertical
   splits (two file panes side by side) = a renderer + state change;
   (b) emacs-degraded semantics on the existing stack (C-x 2 opens the
   OTHER buffer in the stack's next view? doesn't match emacs). State
   the model choice and its UX consequences before building. If (a) is
   too big for one lane, implement C-x 1/C-x 0 (stack semantics, cheap)
   + record the C-x 2 split as a scoped follow-up.
2. **Keybinds**: C-x 2 / C-x 1 / C-x 0 (emacs-standard), bound when a
   buffer view is active (home has none — the 06a discipline).
3. **Tests**: unit twins per the loop-03 discipline (state + render80)
   for each key's behavior incl. degradation (C-x 0 on the last view,
   C-x 1 with one view, C-x 2 of a non-file view). Update ux_sweep's
   findings (the 3 pre-existing ones should clear) + the known-issue
   watchlist row.
4. **The battery**: the converted windowing suites may interact — run
   the full gate.

## Constraints

- Gate: `cargo test --workspace` + clippy (PIPESTATUS) + `tools/gate.sh
  full` (flock, cargo build first). Budget ~50 tool calls; honest-stop
  at half.
- Scope fence: `src/app/store.rs`, `src/app/keymap.rs`, `src/ui/root.rs`
  (IF the chosen model needs render changes), `tools/ux_sweep.py`,
  `docs/ux-testing-plan.md`. NO provider/syntax changes. Parallel lane:
  none — the tree is yours.

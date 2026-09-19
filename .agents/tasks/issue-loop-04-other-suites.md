# Task: loop-04 — demote the remaining suites (loop-03 follow-up)

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `.agents/skills/*.md` as ground truth.

## Origin

loop-03 (merged, `cf1be9d`) demoted sweep_flows to a 15-record thin PTY
tier (15/15 in 23.7s; 60 unit twins in `src/app/flow_tests.rs`) and
published the kept/converted ledger in `docs/ux-testing-plan.md`. The
other-suites table DEFERRED: drive_windowing(+panes), drive_xref,
drive_external_*, ux_sweep — all "state+text, still run in the battery".
This issue converts them.

## What to build

1. **Convert to unit twins** (same discipline as loop-03: driven through
   the same entry points the live app uses, assert state +
   `render80`/`render_at_width(80)`, no verdict relaxation):
   - `drive_windowing.py` (28 steps) + `drive_windowing_panes.py` (4
     scenarios): view-stack/scroll/split state — store-level.
   - `drive_xref.py` (12 legs): resolution + landing + recenter — store
     level (the resolver corpus suites now cover provider-level).
   - `drive_external_notes.py` (16), `drive_external_crate.py` (12),
     `drive_external_use.py` (6), `drive_syntax_notes.py` (8): annotation
     state on external buffers — store-level (the 008/006 machinery is
     store-level; the bus/indicator wiring has unit patterns).
   - `ux_sweep.py`: keymap coverage derivation — unit-testable.
2. **Keep PTY for what only a live app proves**: one smoke per family
   (like loop-03's tier). Reduce each converted file to its terminal-only
   residue or retire it into a smoke leg; publish the ledger rows.
3. **Battery target**: the pooled battery (currently ~96s) should drop
   toward ~50-60s. Measure with a pinned REDLINE_BIN and `cargo build`
   first (pool.py does not rebuild).
4. **Hard rule: no verdict may be relaxed.** Sample-verify each converted
   step against the original (git history has the originals).

## Constraints

- Gate: `tools/gate.sh full` green throughout (suites shrink as you
  convert); final pooled run measured. Budget ~65 tool calls; honest-stop
  at half budget (commit partial + ledger) per the loop-03 provision.
- Scope fence: `tools/drive_windowing.py`, `drive_windowing_panes.py`,
  `drive_xref.py`, `drive_external_notes.py`, `drive_external_crate.py`,
  `drive_external_use.py`, `tools/ux_sweep.py`, `tools/gate.sh`,
  `src/app/flow_tests.rs` (appending — the loop-03 file; do NOT touch the
  existing 60 twins), `src/ui/root.rs` ONLY if a helper is missing,
  `docs/ux-testing-plan.md`. NO store.rs logic changes, no provider changes.
- A parallel lane owns `docs/provider-matrix.md` — do not edit it.
- PTY flock discipline; `cargo build` before batteries.

# Task: plan 004 issue 05b follow-up — update the two stale motion flows

You are the implementation worker. Repo root is your cwd. This spec is
self-contained.

Context: 004-05b (file-view point + emacs motion keys) is implemented in
the working tree: 405 tests green, check_cursor_stream 31/31 (file-view
leg with row+col CUP tracking, wrap, goal-column, scroll-holds-row). Two
`tools/sweep_flows.py` flows still assert the RETIRED plan-001 stopgap
(window-scroll semantics with the point pinned at the top) and therefore
fail:

- `U-C1` (`C-d` repaint): previously the window scrolled with the point
  pinned; now `C-d` moves the window and HOLDS the point's screen row.
- `U-M8` (`C-x C-x` exchange): the flow asserts "exchange-scrolled" — a
  stopgap artifact; the real contract is that the point moves to the
  mark's line and the window follows to keep the point visible.

The user directive (arrows + C-f/C-b move the point; emacs semantics)
SUPERSEDES those assertions. Your job: update exactly those two flows to
assert the NEW semantics — no src/ changes (the implementation is
complete and reviewed separately).

## Requirements

1. `flow` for U-C1: use a fixture tall enough that a half-page scroll is
   observable (e.g. ≥60 lines); assert (a) the window's top visible line
   CHANGES after `C-d` (a real scroll, non-vacuous), and (b) the point's
   screen row is preserved across the scroll (emacs behavior) — read the
   point's rendered row from the pyte frame / the status-line position
   display + which-function as needed.
2. `flow` for U-M8: assert the NEW exchange contract — set mark at line A,
   move the point to line B, `C-x C-x` → the point is at A's line (and the
   window follows so the point is visible), then exchange again → back to
   B. Remove the "exchange-scrolled" stopgap assertion.
3. Keep every other flow byte-identical. `sweep_flows.py` must report
   46/46 PASS (or 46 driven, 0 FAIL) after your change.

## Verification

- `python3 tools/sweep_flows.py` → 0 FAIL, all driven flows PASS.
- `cargo build`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
  unchanged-green (405 / 2 ignored) — you should not need to touch src/.
- Scope: tools/sweep_flows.py ONLY.

## Report

Per-flow change (old assertion → new assertion, why), gate output, and
confirmation that no other flow changed.

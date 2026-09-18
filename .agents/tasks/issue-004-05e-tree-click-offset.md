# Task: plan 004 issue 05e — clicks and cursor must respect the tree sidebar offset

You are the implementation worker. Repo root is your cwd. This spec is
self-contained.

## Reported defect (orchestrator UX sweep, 2026-09-18; carried from the
## 05c review as a "known limitation" — now quantified and directed)

When the tree sidebar is visible, the code pane is offset to the right but
mouse-click columns are NOT offset to compensate. Measured live (pty/pyte,
80 cols, tree visible):

- The tree sidebar is a **fixed 34 columns wide** (`src/ui/tree.rs:119`,
  `View(width: 34, flex_shrink: 0.0, …)`), so the code pane's column 0 is
  terminal column 34.
- A click at 1-based terminal col 40 (i.e. inside the code pane, on the
  code text) lands the point at CUP col **22** — clamped to EOL and on the
  wrong character.
- Clicks inside the tree's own columns (e.g. 1-based col 10/20/30) still
  set the point in the CODE pane (they should not touch the code point at
  all, or should select the tree row — see below).

With the tree hidden, click mapping is correct (1-based col N → 0-based
col N−1; verified).

## Read first

1. `src/ui/root.rs` — the mouse arm (~297-310): the comment explicitly
   notes "with the tree sidebar visible, a file-view click's column is
   shifted by the tree's width" and then does nothing about it.
   `mouse_click_position(row, col)` is called with the raw terminal column.
2. `src/ui/tree.rs:119` — the fixed `width: 34` sidebar.
3. `src/app/store.rs` — `mouse_click_position(row, col)` (~2874).

## What to build

1. Define the tree sidebar width as a **shared constant** (e.g.
   `pub const TREE_WIDTH: u16 = 34;` in `src/ui/tree.rs` or `src/ui/mod.rs`)
   and use it both in the `View(width: …)` and in the click offset — so
   they cannot drift.
2. In the root mouse arm, when `tree_visible`:
   - If the click column is **within the tree width**, do NOT forward it to
     the code pane. Prefer selecting the tree row under the click if that
     is cheap (the tree already has a `selected` index and a row list; a
     click → row index maps like the file view's click → line). If
     click-to-select in the tree needs more than a small amount of
     plumbing, make it a no-op for now and say so — but it must not move
     the code point.
   - Otherwise, pass `col − TREE_WIDTH` to the file-view click mapping.
3. Verify the same offset logic is applied anywhere else the file view's
   column is derived from a terminal column (grep for other
   `mouse_click_position` callers; today there is one).
4. Note (do NOT fix here): the tree's fixed 34-column width is a separate
   layout question (a narrow terminal wastes/overflows). Record it in
   `docs/ux-testing-plan.md` as a backlog row if you find it is genuinely
   problematic; do not change the width in this issue.

## Constraints

- Scope fence: `src/ui/root.rs`, `src/ui/tree.rs` (the constant),
  `src/app/store.rs` (click mapping signature/offset only), tests,
  `tools/check_cursor_stream.py`. No dependency changes; no point-motion
  semantic changes; the 34-col width itself is unchanged.
- All suites green: `cargo test`, `tools/sweep.py`, `tools/sweep_flows.py`,
  `tools/drive_all.py`, `tools/drive_windowing.py`,
  `tools/drive_windowing_panes.py`, `tools/check_cursor_stream.py`.
- Wrap EVERY python PTY invocation in `timeout`.

## Verification (iterate until ALL pass)

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test green.
- Unit: offset arithmetic (tree visible/hidden, click inside/outside the
  tree region, col beyond the code pane's EOL clamps correctly).
- Raw-PTY leg (extend `tools/check_cursor_stream.py`, tree-visible case):
  with the tree visible, a click at 1-based col `34 + k` on a code row
  lands CUP col `k−1` char index (i.e. the intended character), and a
  click inside the tree columns does NOT move the code point. The
  tree-hidden leg must stay green.
- Report the before/after CUP columns from the repro above.

## Report format

Offset/constant design. Before/after repro numbers. Gate outputs (exact
counts). Backlog row if you recorded one. Deviations.

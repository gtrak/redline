# Task: plan 004 issue 05g — hardware cursor must be absolute (tree-visible offset)

You are the implementation worker. Repo root is your cwd. Self-contained.

## Reported defect (orchestrator live probe, 2026-09-18, on commit 95f24b1)

With the tree sidebar VISIBLE, mouse clicks are now correctly offset by the
sidebar width (05e) — but the HARDWARE CURSOR is not. `cursor_cell`
computes the point's column as code-pane-relative, so the terminal cursor is
drawn at that raw column, i.e. **inside the tree sidebar**.

Measured live (80 cols, tree visible, `src/lib.rs` open, code pane starts at
terminal col 34):

- After 3× `C-f` (point at char 3 of the code line), the terminal CUP is
  **col 3** — over the tree's `README.md` row — where it must be **col 37**
  (34 + 3).

The tree-HIDDEN case is correct (code pane starts at col 0, so relative ==
absolute). This is purely the missing sidebar offset in the cursor
placement.

## Read first

1. `src/ui/root.rs` — `cursor_cell` (the Buffer arm computes
   `col = <display col of the point>`), and the `Snapshot` fields
   (`tree_visible` is already carried: see the element tree's
   `#(if snap.tree_visible { … TreeSidebar … })`).
2. `src/ui/tree.rs` — `pub const TREE_WIDTH: u16 = 34` (added by 05e; reuse
   it — do NOT hardcode 34).
3. `src/ui/root.rs` — the deferred post-render cursor write (the `?25h` +
   CUP task from 004-05) that consumes `cursor_cell`.

## What to build

1. In `cursor_cell`, when `snap.tree_visible`, add `TREE_WIDTH` to the
   column (clamp to the terminal width so a huge column cannot address
   outside the screen; the row is unchanged).
2. Verify the SAME offset question for any other cursor placement path
   (grep for `cursor_cell` callers and any other `CUP`/`goto` writer) —
   one place today, but check.
3. Note in the code WHY: the terminal cursor is in absolute screen
   coordinates, while the point column is pane-relative; the two panes are
   laid out by the flex row.

## Also in scope (carried 05e review P2, same layering fix as 05d's)

`src/app/store.rs` now reaches into `crate::ui::tree::{TREE_VISIBLE_ROWS}`
(`tree_click_row`) — the ONLY `crate::ui` reference in `src/app`, and the
same app→ui inversion the 05d review flagged and 05e fixed for the width
helpers. Complete the pattern: move `TREE_WIDTH` and `TREE_VISIBLE_ROWS`
into a non-UI module (e.g. `src/model/tree_layout.rs`, next to
`src/model/text_width.rs`) and have `src/ui/tree.rs` import them (it may
re-export for existing UI callers). `src/model` must not import `ui`/`app`.
Keep it a pure move: same values (34, 8), same tests.

## Constraints

- Scope fence: `src/ui/root.rs` (cursor placement), `src/ui/tree.rs` +
  `src/model/tree_layout.rs` (the const move), `src/app/store.rs` (import
  path only), tests, `tools/check_cursor_stream.py`. No click-mapping changes (05e is
  correct); no point-motion changes; the 34-col width unchanged.
- All suites green: `cargo test`, `tools/sweep.py`, `tools/sweep_flows.py`,
  `tools/drive_all.py`, `tools/drive_windowing.py`,
  `tools/drive_windowing_panes.py`, `tools/check_cursor_stream.py`.
- Wrap EVERY python PTY invocation in `timeout`.

## Verification (iterate until ALL pass)

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test green.
- Unit: `cursor_cell` (or the placement helper) returns
  `pane_col + TREE_WIDTH` when the tree is visible, `pane_col` when hidden,
  and clamps at the terminal width.
- Raw-PTY leg (extend `tools/check_cursor_stream.py`): with the tree
  visible, after N× `C-f` on a code line the CUP column equals
  `TREE_WIDTH + N` (and matches the code pane's actual start column read
  from the frame, so the test cannot drift if the layout changes); the
  tree-hidden leg stays green.
- Report the before/after CUP columns from the repro above.

## Report format

Placement design. Before/after repro numbers. Gate counts. Deviations.

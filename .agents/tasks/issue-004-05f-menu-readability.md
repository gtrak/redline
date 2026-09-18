# Task: plan 004 issue 05f — transient-menu readability (truncation + column separation)

You are the implementation worker. Repo root is your cwd. Self-contained.

## Reported defect (orchestrator UX sweep, 2026-09-18)

The transient menu (`?`) renders two fixed 40-char columns and hard-truncates
each description mid-word. Observed at 80 columns:

```
[C-a] Point to the beginning of the line[C-b] Point backward one character, wrap
[M-f] Point forward one word, wrapping a[M-v] Scroll up one page (M-v)
```

Problems: (1) descriptions are cut mid-word ("wrapping a" — should read
"wrapping across lines"); (2) adjacent columns run together with no
separator (`line[C-b]`), which is hard to read.

## Read first

1. `src/ui/transient_menu.rs` — `draw`: `const COLS = 2; let cell_w = (w /
   COLS).max(1);` then `canvas.set_text((x * cell_w), y, &truncate(&text,
   cell_w), …)`. `truncate` is a hard char cut.
2. `src/app/store.rs` — `menu_rows()` / `TransientMenuRow` (the label text
   and `is_prefix`/`is_header` flags).

## What to build (keep it simple; this is presentation)

1. **Ellipsis, not a mid-word cut**: when a description does not fit its
   cell, truncate with a trailing `…` so the cut is visibly intentional
   (e.g. `Point forward one word, wrapping acr…`). Never exceed the cell.
2. **A visible gutter between the columns**: reserve a 1-2 char gap (e.g.
   `cell_w = w / COLS` and draw each cell's text with a leading space, or
   reduce the text width to `cell_w - 1`). Ensure two cells never abut.
   If the width is too small for two readable columns (say `w < 40`), fall
   back to ONE column (full-width rows) so descriptions stay legible on
   narrow terminals.
3. Do NOT change which commands/rows are listed, their order, or the
   `key_display` strings — layout only.
4. If the fix is cheap, ensure a description that fits exactly is not
   ellipsized (off-by-one on the ellipsis budget).

## Also in scope (carried 05g review P2)

Add ONE PTY leg covering the list-view CUP with the tree visible (the
05g offset applies to every view arm, but only the Buffer arm is
asserted): open magit, toggle the tree on (`C-c p t` then `RET`, or
`M-x toggle-tree`), and assert the surviving CUP column equals the pane's
start column read from the frame (= 34). Low risk, but it is the one
uncovered arm of a uniform mechanism.

## Constraints

- Scope fence: `src/ui/transient_menu.rs`, tests, `docs/`. Layout only: no
  command/keymap/registry changes.
- All suites green: `cargo test`, `tools/sweep.py`, `tools/sweep_flows.py`,
  `tools/drive_all.py`, `tools/drive_windowing.py`,
  `tools/drive_windowing_panes.py`, `tools/check_cursor_stream.py`.
- Wrap EVERY python PTY invocation in `timeout`.

## Verification

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test green.
- Unit tests for the fit/ellipsis helper (fits exactly, one over, much
  over, wide/emoji chars, cell width 1). If the helper lands in the view
  module, keep it testable (a free function).
- Raw-PTY leg: at 80 cols, `?` renders two columns with a visible gap
  (assert no row contains `][` or a letter immediately followed by `[`);
  every rendered row length ≤ 80; at ~30 cols the menu falls back to one
  column (assert a long description is present whole or ellipsized, and
  no two-cell collision).
- Report before/after lines from the repro above.

## Report format

Layout/ellipsis design. Before/after rendered lines. Gate counts.
Deviations.

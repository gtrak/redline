# Task: plan 005 issue 02c — fill the canvas with the largest annotation-aware span

You are the implementation worker. Repo root is your cwd. Self-contained.

## Context (reviewer-recommended follow-up, backlog #14)

005-02b fixed a BLOCKING blank-view P1 by flooring the code-row span at 1:
`code_span = viewport_lines.saturating_sub(n_notes).max(1)`. That is
correct but under-fills: when most/all in-window lines are annotated the
span collapses to 1, so the canvas shows ~2 rows of 21 (measured live:
a 25-line file with all 25 lines annotated → 3 non-blank rows) and
suppresses annotations the feature exists to show.

The 02b review agreed the right rule is: **choose the largest span
`s ∈ [1, viewport_lines]` such that `s + notes_in_window(s) <=
viewport_lines`**, keeping the existing final `notes_left` recount/cap as
the safety net (that cap is what handles many-records-on-one-line). For
the repro (25 lines all annotated, viewport 21) that is `s = 10` → 10 code
+ 10 note rows = 20 rows.

## Read first

1. `src/app/store.rs` `file_view_rows()` (~4019-4095) — the current span
   cap (`saturating_sub(n_notes).max(1)`), the point-advance block, and the
   FINAL budget (`code_rows`, `notes_left`, the emission loop's
   `notes_left == 0` break). Keep all of those; change only how the span
   is chosen.
2. `src/app/store.rs` — `notes_all_lines_annotated_budget_is_final`
   (~14035; 4 legs) — the test to extend.
3. `src/ui/file_view.rs` — the `↑` indicator now keys off
   `rows.first().line`; ensure it stays correct with a larger span.
4. `tools/check_cursor_stream.py` — `annotation_gutter_checks()` legs (the
   all-annotated leg currently tolerates the 2-row output).

## What to build

1. Replace the floored span with the **largest-span search**: pick the max
   `s` in `[1, viewport_lines]` whose window `[start, start+s)` (adjusted
   by the existing point-advance rule) contains `notes_in_window(s)` notes
   with `s + notes_in_window(s) <= viewport_lines`. `notes_in_window` is
   monotonic non-decreasing in `s`, so a scan from `viewport_lines` down
   (or a binary search) is correct and cheap — a linear scan down is fine
   (≤ viewport iterations, and this runs once per frame).
2. **Preserve**: the point-advance behavior, the emitted-range recount and
   `notes_left` cap, the guarantee the point's code row is always drawn,
   the `↑` indicator semantics, and the invariant
   `code_rows + emitted_note_rows <= viewport_lines` in every case.
3. If `s` cannot be > 1 (e.g. every line annotated AND many notes on one
   line), fall back to the current behavior — the floor at 1 stays as the
   final guarantee against a blank view.

## Constraints

- Scope fence: `src/app/store.rs` (the span choice only), tests,
  `tools/check_cursor_stream.py`. No renderer/model/storage/keymap changes;
  no dependency changes.
- All suites green: `cargo test`, `tools/sweep.py`, `tools/sweep_flows.py`,
  `tools/drive_all.py`, `tools/drive_windowing.py`,
  `tools/drive_windowing_panes.py`, `tools/check_cursor_stream.py`,
  `tools/ux_sweep.py`.
- The PTY suite takes an exclusive flock on the shared fixture; if you see
  "shared PTY fixture is busy" and exit 3, wait and retry. Never run two
  PTY suites concurrently.
- Wrap EVERY python PTY invocation in `timeout`.

## Verification (iterate until ALL pass)

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test green.
- Extend `notes_all_lines_annotated_budget_is_final` with legs asserting
  **canvas fill** (not just non-blank): for the all-annotated repro assert
  the emitted row count is close to `viewport_lines` (e.g. >= 18 of 21,
  and specifically the 10+10 shape), and that the point's line is drawn.
  Keep the existing blank-guard, point-advance, and 22-records-on-one-line
  legs passing unchanged (the last must still cap to <= 21).
- Strengthen the `tools/check_cursor_stream.py` all-annotated leg to assert
  the fill (e.g. the number of non-blank content rows is >= a threshold),
  not merely that the view is non-blank.
- Live check: reproduce the 25-line-all-annotated case and report the row
  count before (3) and after.

## Report format

Span-selection design (and why monotonicity makes it safe). Before/after
row counts from the repro. Gate counts (honest, from actual output).
Deviations.

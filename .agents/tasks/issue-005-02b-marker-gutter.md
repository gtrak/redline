# Task: plan 005 issue 02b — annotation rendering: marker gutter + note rows must not overflow the canvas

NOTE: this spec now carries TWO fixes in the same rendering/coordinate area
(both in src/ui/file_view.rs + the coordinate consumers), so the file is
touched once:

(A) **BLOCKING (005-02 review P1, orchestrator-reproduced)**: note rows
    make the rendered slice exceed the canvas. `file_view_rows()` slices
    buffer lines `[scroll_top, scroll_top + viewport_lines)` and then
    APPENDS one row per annotated line in that slice, with no scroll
    adjustment and no cap; `cursor_cell` clamps `content_row` to
    `viewport_lines - 1 - banner`. Repro (verified live, 60-line file,
    annotation on line 0, viewport 21): 20x C-n puts the point at
    `tall_20`, the visible canvas ends at `tall_19`, and the hardware
    cursor sits on `tall_19` — the point's OWN LINE IS NOT DRAWN and the
    cursor is one row off. `C-c a` (hide note rows) instantly corrects it
    to `tall_20`, proving the note row is stealing a canvas row without
    the window compensating. Fix: make the file-view window aware of
    rendered rows — either cap the emitted code-row span so
    `rendered rows <= viewport_lines`, or advance `start`/`scroll_top` by
    the note-row count so the point's rendered row stays
    `<= viewport_lines - 1 - banner` — and clamp `content_row` against the
    RENDERED slice, not the buffer-line viewport. Choose one, state why,
    and add the reviewer's suggested leg: open a >viewport file, `A` on
    line 0, C-n to the window bottom, assert the position's line is drawn
    AND the cursor row is that same row.

(B) **Known defect (orchestrator-reproduced)**: the `▎` marker is drawn at
    cell 0 OVER the code text, clobbering each annotated line's first
    character (`fn target_one() {}` -> `▎n target_one() {}`).

You are the implementation worker. Repo root is your cwd. Self-contained.

## Reported defect (orchestrator live drive, 2026-09-18, on commit f70fae7)

The annotation margin marker `▎` is overlaid at **cell 0 of the code text**,
so it CLOBBERS the first character of every annotated line:

- source `fn target_one() {}` renders as `▎n target_one() {}` (the `f` is gone)
- source `target_one();` renders as `▎arget_one();` (the `t` is gone)

Reproduced two ways (a live `A` annotation, and a pre-seeded record), and
with the note rows both shown and hidden (`C-c a`) — it is systematic, not a
typing artifact. This is a correctness bug: the rendered line no longer
matches the file, which also corrupts the reading of the file being browsed.

## Read first

1. `src/ui/file_view.rs` — the draw loop (~line 104-118): after
   `draw_line(&mut canvas, row, w, &r.text, &r.spans, &t)` it does
   `if r.annotated { canvas.set_text(0, row, "▎", …) }` — i.e. writes the
   marker INTO the same cell the text starts at.
2. `src/ui/file_view.rs` — the note-row rendering (`  ▸ text`) and
   `truncate`/`draw_line`'s width budget; whatever gutter you add must be
   accounted for in the truncation width (today it uses the full `w`).
3. `src/model/text_width.rs` — use the shared display-width helpers for the
   gutter arithmetic (wide chars: the marker is 1 cell, but the code shift
   must be in display cells, not chars).
4. `tools/check_cursor_stream.py` — the cursor column legs
   (`cursor_cell`'s Buffer arm) and the click mapping: **the gutter must be
   consistent across render, cursor, and click** (this is the same class of
   pane-relative-vs-absolute bug as 05e/05g).

## What to build

1. Give the marker its own **left gutter column**: annotated lines render
   their code starting at cell 1 (or a fixed `ANNOTATION_GUTTER: usize`,
   your call — state it), with the marker drawn at cell 0. NON-annotated
   lines keep starting at cell 0 (do not shift the whole file).
   - Truncation width becomes `w - gutter` for annotated lines so the line
     still fits and the `↓`/`↑` indicators are unaffected.
2. **Keep the three consumers consistent** for an annotated line:
   - the renderer (code at gutter offset, marker at 0),
   - `cursor_cell` (the point's display column must add the SAME gutter —
     otherwise the hardware cursor sits one cell left, exactly the 05g bug),
   - `mouse_click_position` (a click in the gutter, or at code cell k, must
     map back to the right character; clicking the marker itself should
     either clamp to col 0 or be ignored — pick and document).
   Note rows (`  ▸ note`) should align under the same gutter so the visual
   gutter reads as one column.
3. Do NOT change the annotation model, the storage format, the re-anchoring
   logic, or the `A`/`d`/`C-c a` behavior — this is a rendering/coordinate
   fix only.

## Constraints

- Scope fence: `src/ui/file_view.rs`, `src/ui/root.rs` (cursor column only),
  `src/app/store.rs` (click mapping only), tests,
  `tools/check_cursor_stream.py`. No dependency changes; no model/storage
  changes.
- All suites green: `cargo test`, `tools/sweep.py`, `tools/sweep_flows.py`,
  `tools/drive_all.py`, `tools/drive_windowing.py`,
  `tools/drive_windowing_panes.py`, `tools/check_cursor_stream.py`,
  `tools/ux_sweep.py`.
- Wrap EVERY python PTY invocation in `timeout`.

## SECOND ROUND (re-review P1, orchestrator-reproduced) — the cap degenerates to a BLANK view

`store.rs` (~4027-4044) caps the code span as
`code_span = viewport_lines.saturating_sub(n_notes)` and then
`end = start + code_span`. When EVERY line in the window is annotated
(`n_notes == viewport_lines`), `code_span == 0`, so `end == start` and the
emission loop `for line in start..end` runs ZERO times — and because the
note rows are emitted INSIDE that loop, `rows` is EMPTY: the entire file
view renders blank and the cursor parks on a blank content row.

REPRODUCED LIVE (orchestrator): a 25-line file with all 25 lines
annotated, viewport 21 → 25 code lines + 25 notes invisible, cursor at
row 1, essentially nothing drawn.

REQUIRED FIX (the reviewer's, and it must be exactly this shape — a bare
`saturating_sub(..).max(1)` is NOT sufficient because it re-overflows):
make the budget counts FINAL. After choosing `start`/`end`, recount the
notes in the emitted range and cap the number of emitted NOTE rows so
`code_rows + note_rows <= viewport_lines`, ALWAYS keeping the point's code
row visible. The cleanest formulation: emit the code rows for the window
first (>= 1 row, including the point's line), then fill note rows only up
to the remaining budget (`viewport_lines - code_rows`). Add a leg for the
all-lines-annotated case asserting the point's line IS drawn and the view
is not blank.

## Also in this round (re-review P2s, cheap and in-fence)

- **`↑` indicator consistency**: the window now advances `start` above
  `scroll_top` (to keep the point drawn) but `file_view_scroll_info`
  still reports `scroll_top` and the renderer shows `↑` only when
  `top_line > 0`. In the repro (scroll_top 0, window actually starts at
  line 1) line 0 is hidden with no `↑`. Key the indicator off the emitted
  window's first buffer line (it is already in `rows`) — do not change
  `scroll_top` semantics for the other consumers.
- **Tighten the regression leg**: `tools/check_cursor_stream.py` asserts
  `1 <= r <= 21` for a 1-based CUP, but the canvas occupies CUP rows 2..22
  (title CUP 1, 21 content rows). Tighten to `2 <= r <= 22`.
- **Strengthen the leg's text check**: the cursor-row assertion must also
  assert that the row UNDER the cursor contains the point's line text
  (e.g. `row_text(cursor_row)` contains the expected `fn ...` text), not
  just that the row is in-range — otherwise a future off-by-one that lands
  the cursor on a NEIGHBOURING drawn row would pass.

## Verification (iterate until ALL pass)

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test green.
- Unit: for an annotated line, the rendered text (after the gutter) equals
  the source line exactly (assert the full string, not a prefix — a
  prefix-only assertion is what let this through); truncation respects
  `w - gutter`; the cursor column for a point on an annotated line equals
  gutter + display col; the click mapping inverts that.
- Raw-PTY leg: with an annotation on a known line, assert the rendered row
  contains the source line VERBATIM at the gutter offset (e.g. the row
  equals `"▎fn target_one() {}"`), the marker occupies exactly cell 0, and
  `C-c a` toggling note rows does not change the code row's text. Add a
  cursor-column leg on an annotated line (the hardware cursor must sit ON
  the character, not one cell left).
- Report the before/after rendered rows from the repro above.

## Report format

Gutter design (size, where defined, why). Before/after repro rows. The
three-consumer consistency check. Gate counts (honest, from actual output).
Deviations.

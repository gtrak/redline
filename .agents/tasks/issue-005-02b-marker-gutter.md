# Task: plan 005 issue 02b — annotation marker must not overwrite the first character

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

# issue-annotations-layout: pack non-overlapping note rows onto one line; longer leader glyphs when they must overlap

**User report (verbatim):** *"now the annotations need better layout. If they are non-overlapping, they
can be on the same line. If they must overlap, then make the glyphs longer for the further out ones."*

## Current layout

Every annotation gets its own note row, placed immediately above (or below) the code row it anchors
to, with the corner glyph `╭` at **the same display cell as the code row's `▴`** and a `─` bend one
cell right of it, then the note text (`src/ui/file_view.rs:111-127`; the anchor relationship is
asserted per cell in the tests around `src/ui/file_view.rs:1512-1564`, deliberately as a
*relationship* — "the note row's `╭` sits at the SAME cell as the code row's `▴`" — because a mere
"the note row has a `╭`" assertion cannot catch a misplacement).

Nested annotations already differ in glyph: the **shallowest** nesting uses `╭`, deeper ones use
other curve pieces (`src/ui/file_view.rs:1526`, `:1553-1564`).

So today: one row per annotation, all of them stacked, regardless of how much horizontal room is
wasted. On a line with two or three annotations the file view grows rows faster than it needs to.

## What the user wants

1. **Non-overlapping annotations share a line.** Two (or more) note rows whose *text spans* do not
   collide should be drawn on the **same** row, each still at its own anchor column — the anchor
   relationship above is the contract and must hold for every one of them.
2. **When they must overlap, the further-out ones get longer glyphs.** Overlap forces stacking; the
   one that is "further out" then gets a **longer** glyph/leader so its connector still reads as
   reaching its own anchor rather than its neighbour's.

**Resolve the ambiguity explicitly, with a stated rule.** "Further out" can mean (a) the annotation
whose anchor is horizontally further from where the note text sits, or (b) the *outer* (shallower)
nesting — which is what the existing `╭`-vs-deeper glyphs already encode. Pick one, write the rule
down in the code and in the test that pins it, and say why. Do not leave the reader to infer it.

## Acceptance

- Two annotations on one line whose texts do not overlap → **one** note row carrying both, each glyph
  at its own anchor cell (assert the anchor relationship for **both**, not just the first).
- Two whose texts would overlap → stacked rows, with a **measurably longer** glyph/leader on the
  further-out one. "Longer" must be an assertion about cells, not a screenshot.
- The note text is never truncated or overwritten by a neighbour.
- The existing anchor-relationship tests stay green **and stay meaningful** — a packed row must not
  be able to pass them by accident. If the packing changes what those tests observe, re-express them
  so they still fail when a glyph is misplaced by one cell (mutate the anchor and confirm RED).
- Rendering-only: no change to the notes format, the record identity, or the anchor model. If you
  find yourself editing `notes.rs`'s records, stop — that is a different issue.

## Context that will save you time

- The anchor is the record's **display** column: `col` is a CHAR offset converted char→display
  (wide = 2 cells, tab = next 8-stop) — see `display_col_of_char_with_gaps` and the marker-cell work
  (`issue-annotation-marker-cell`, landed). A note row's glyph cell is that value.
- The marker-cell change means a code row can be **wider** than its source (a cell is inserted when
  the preceding cell is not whitespace), and spans/matches/click mapping are re-based accordingly.
  Any packing you do must respect the same display-cell model, not char counts.
- Annotations may be `orphaned` (the tie could not be resolved); an orphaned record's placement rule
  already differs — keep it consistent with whatever rule you state.
- Multi-annotation lines are now **producible through the UI** (`A` on a second symbol creates a
  second record — `issue-annotation-per-symbol-creation`, landed). Several tests build such lines by
  hand; their comments were recently corrected because they had claimed the UI could not produce them.
  Do not reintroduce that assumption.

## Verification notes for the implementer

- The known battery failure is `check_cursor_stream.py`'s `menu@80` (filed, pre-existing) — do not
  chase it; confirm it is the only one.
- A layout change is exactly the kind that a passing suite hides: assert the **cells**, and mutate
  your own rule (drop the packing, shorten the leader) to confirm a test reddens for each.
- Fence: the note-row layout in `src/ui/file_view.rs` and its tests. Disclose anything else.

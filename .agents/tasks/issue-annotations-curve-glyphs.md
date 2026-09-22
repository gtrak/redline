# issue-annotations-curve-glyphs — the note branch uses a CURVED corner (design A)

**The user picked A** after seeing both candidates rendered. This replaces the diagonal `╱` on
the note row with a **curved corner** `╭`, so the branch rises from the arrow and *turns* right
into the note instead of slanting through the cell.

## Target rendering (this is the acceptance criterion)

```
FOLDED      ▸ fn foo() {

EXPANDED    ╭─ my note
            ▾─ fn foo() {
```

Per-cell — **cell 0 is the anchor column for both rows**:

| row | cell 0 | cell 1 | cell 2 → |
|---|---|---|---|
| unannotated code | *source line as-is* | | |
| annotated code, FOLDED | `▸` (U+25B8, dim `preview` face) | blank | source line |
| annotated code, EXPANDED | `▾` (U+25BE, bright `view_title` face) | `─` (U+2500) | source line |
| note row (EXPANDED only) | `╭` (U+256D) | `─` (U+2500) | note text |

## Changes from the landed state (`7e97812`)

1. The note row's **`╱` (U+2571) at cell 0 is replaced by `╭` (U+256D)** — the curved corner
   whose stroke comes up from the line below and bends right.
2. **The note row gains a `─` (U+2500) at cell 1**, so the curve turns into a straight run to
   the note. (Today cell 1 on the note row is blank; the note text already starts at cell 2, and
   it must stay there.)
3. **Nothing else changes** — the arrow glyphs and faces, the code-row arm, the fold behaviour,
   the binding table, and the note row's position above its code row all stay exactly as they are.

## Requirements

1. **The glyphs exactly as above**, with the note text still starting at **cell 2** and the code
   still starting at **cell 2** in both fold states.
2. **⚠ Assert the ANCHOR RELATIONSHIP, not merely the glyph's presence.** The `╭` on the note row
   must be at the **same cell** as the `▾` on the code row directly below it, because that
   adjacency is the whole reason the branch reads as attached. The previous `╱` passed a gate
   while being attached to nothing, because the tests asserted only that the note row *contained*
   a glyph — a presence assertion cannot catch a misplacement. So the test must assert the
   columns line up (and that the two rows are adjacent), and it must **fail** if the corner is
   moved one cell left or right. Show that mutation.
3. **The no-jitter property still holds**: the leading width is a constant 2, so the code's start
   column is identical folded and expanded. The existing test for this must keep passing.
4. **Update every test and drive that asserts the old `╱`** — the unit tests, the
   `tools/check_cursor_stream.py` annotation legs, and any `sweep_flows.py` assertion — to the new
   glyphs. **Update them; do not loosen them**, and keep a flow that fails if the note stops being
   drawn above its code line.
5. The **cell-1 `─` on the note row** is part of the look; assert it so a later edit cannot
   silently drop it.
6. **Do not change the fold semantics, the `C-c a h` toggle, or the binding table** — this is a
   glyph/layout change only.

## Acceptance

* The rendered rows match the target above, asserted per cell (not by substring on a whole row).
* The anchor test discriminates: moving the corner one cell makes it RED.
* The no-jitter test still passes.
* The PTY drives are updated and still discriminate.
* `cargo test --workspace` + clippy `--workspace --all-targets -- -D warnings` clean;
  `tools/gate.sh full` green.
* The commit message carries the **per-cell dump** of both states, so the layout is on the record.

## Fence

`src/ui/file_view.rs` (the renderer), `tools/` drives, and tests. Disclose anything else with
before/after. Note: another branch may be landing a click-column pin in the same file — if you
see a conflict there, resolve it by keeping **both** changes (they are in different functions).
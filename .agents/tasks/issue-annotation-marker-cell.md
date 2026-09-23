# issue-annotation-marker-cell — when no whitespace cell precedes the symbol, INSERT one

**The user's decision (2026-09-23):** *"adding a cell when no whitespace is available is totally fine,
we should mark whatever the cursor is on."*

## The bug (reproduced live)

```
    map: HashMap<String, u32>,      cursor on the inner `String`
```
```
record: col 17   syntax_kind type_identifier   syntax_name String      <- the tie is RIGHT
frame:    3|   ▴map: HashMap<String, u32>,                             <- the marker is WRONG
              arrow col 3 = the cell before the FIELD NAME
```

**Mechanism.** `record_anchor`'s rule 1 places the marker one cell before the symbol **only if that
cell is whitespace**. `String` is preceded by `<`, so rule 1 is skipped and the code falls back to the
**line's indent anchor** — which is the cell before the field name. The fallback therefore silently
points at a **different symbol** from the one the record is tied to.

That is the part that makes this hard to see: the marker exists, the note row is well-formed, the tie
is correct, and no test objects. It surfaced only because the user annotated a nested type and looked.

## Requirement

**The marker must sit on the symbol the record is tied to.** When the cell before the symbol is not
whitespace, do not fall back to the line: **insert a cell** at the symbol's start, shifting the symbol
and its tail right by one, and put the marker in the new cell.

```
    map: HashMap<String, u32>,   ->      map: HashMap▴String, u32>,
                                        ^ `map` and `HashMap<` byte-identical
```

## Rules

1. **Whitespace before the symbol** — unchanged: the marker overwrites that cell, the code does not move.
2. **NON-whitespace before the symbol** — insert one cell at the symbol's start; the marker occupies it;
   only the symbol and its tail shift. This is the same trade the column-0 rule already makes (there
   the whole line shifts by exactly 1, because there is nothing before the symbol).
3. **Column 0** — unchanged: shift by exactly 1.
4. **No symbol at the point** (EOL / whitespace / comment) — unchanged: the raw cursor column, line-tied.
5. **Several annotations on one line** — the line shifts at most once; a marker that shifted moves +1
   WITH the code (the existing `rule_one`/shift logic), and the anchors stay distinct.

## The hazards — this is a MID-LINE insertion, which the renderer has never done

The existing shift is a **prefix** shift (the whole line moves by a constant). This inserts a cell in
the **middle**, which is a new class of change for the row model:

- Every **highlight span and search-match range after the insertion** must be re-based by one. *A
  one-byte error here is silently mis-coloured text on exactly the annotated lines*, and no existing
  test would catch it. Pin it with a span that starts **after** the marker.
- The **click mapping** (`display_col_to_char_index`) and **`cursor_cell`** must account for the
  inserted cell, or a click after the marker lands one character off — and the cursor's cell on that
  line will be wrong.
- The **note row's `╭`** must stay at the same column as the marker (the anchor relationship), and the
  note row must **not** be shifted by the code's insertion.
- A **tab** before the symbol: the rule is about display cells, not bytes, so a tab's expansion
  participates the same way whitespace does.

## Acceptance

- The reproduction above renders `HashMap▴String`, the marker's cell is **adjacent** to `String`
  (assert the anchor relationship, not merely the presence of a glyph), and `map:` and `HashMap<` are
  byte-identical.
- A click after the inserted cell maps to the character under it; the cursor's cell on that line is right.
- Highlight spans and search matches on a shifted line are correct, pinned with a span that starts after
  the marker.
- Multi-annotation: one shift, and each marker adjacent to its own symbol.
- The whitespace, column-0 and no-symbol cases behave exactly as today (those are the regression net).
- `cargo test --workspace`, clippy `--workspace --all-targets -- -D warnings`, `tools/gate.sh full`.

## Fence

`src/app/store/file_view.rs` (the row model, `record_anchor`, the click mapping),
`src/ui/file_view.rs` (the renderer), `src/ui/root/geometry.rs` (`cursor_cell`), tests and drives.

## Note — two related defects, filed separately

`issue-annotation-stage-2b`'s addendum records that struct fields get **no scope** (so common type
names collide and the note degrades to the text rules) and that a **reference type (`&'a str`)
captures nothing at all**. Those are identity defects; this issue is the marker. Both matter for the
same user-visible outcome, so whichever lands first should say so in the tracker.

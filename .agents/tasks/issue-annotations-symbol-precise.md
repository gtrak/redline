# issue-annotations-symbol-precise — the anchor uses the annotation's recorded column

**User's decision (2026-09-22):** the anchor should be **symbol-precise** — the indicator sits before
**the symbol the annotation was made on**, not before the line's first token.

Builds on the landed anchor work (`78989f7`), which made the anchor **line-based**
(`anchor_col = indent_width - 1`). This generalises that rule; when the annotated symbol *is* the
line's first token the two rules coincide, so the existing tests should mostly keep passing.

## The rule

Let `char_col` be the annotation record's stored column and `display_col` its **display** column.

1. **`display_col > 0` and the cell at `display_col - 1` is whitespace** → the anchor is
   `display_col - 1`. The indicator overwrites a whitespace cell, so **the code does not move**.
2. **The cell before the symbol is NOT whitespace** (e.g. `x+y` annotated at `y`), **or the
   record is at display column 0** (no cell before the symbol exists) → **fall back** to the
   line's indent anchor, `indent_width - 1`. This is the case that makes "the code never moves"
   absolute.
3. **The fallback lands on column 0 only when the line has NO leading indentation** — then the
   line shifts right by **exactly 1** cell (the landed option (a), and the only case where the
   code moves).

   **⚠ AMENDED 2026-09-22 after the gate.** As originally written, rule 2 said a record at
   display column 0 → anchor 0 **and a shift**, unconditionally. That is wrong, and the code
   does not implement it: a record at column 0 on an *indented* line would move the whole line
   — code included — one cell right, in order to annotate a character that is whitespace rather
   than a symbol. That is precisely the "the code moved" defect this issue exists to fix. The
   as-built behaviour (fall back to the indent cell, no shift) is the correct one, and it is
   pinned by `line_anchors_indented_first_token_coincides_with_landed_rule` and the indented
   click pin. A reader implementing the literal old rule 2 would write a branch the code does
   not have.

## ⚠ The units trap — read this before writing code

**The record's column is a CHAR offset, not a display column.** `notes.rs` sets it with
`self.point_col()`, and the code already comments that it is a char offset converted to a byte
offset elsewhere. So you must convert char → **display** column against the line's text, where:
- a **wide** character (CJK, emoji) occupies 2 cells, and
- a **tab** advances to the next 8-column stop.

A symbol after two CJK characters is **2 display columns further right** than its char offset
suggests; after a tab it can be up to 7 further. Getting this wrong puts the indicator on the
wrong cell **only on lines with wide characters or tabs before the symbol** — so pin it with
exactly those fixtures. (`src/model/text_width.rs` is the shared width helper; the anchor must
agree with the renderer's own column arithmetic, or the indicator lands one cell off the symbol
it is supposed to point at.)

## Requirements

1. **The anchor rule above**, including both fallbacks.
2. **The code must not move** — assert it **cell-for-cell against the actual buffer line** (the
   existing test does this; extend it to a **mid-line** symbol, which is the new case).
3. **One indicator per annotation, at its own anchor.** This replaces the landed "several records
   on one line share one anchor" rule: two annotations on one line now get **two indicators at
   two columns** and **two note rows**, each note row's `╭` at its own anchor. Note rows stack
   above the code row (keep the landed order), and folding leaves **one `▸` per annotation**.
   - The **code row therefore carries a set of anchors**, not one — the row model currently holds
     a single `anchor_col`, so this is a model change. Say what you did.
   - If two annotations both **fall back** to the same indent anchor, they share it; say so.
4. **The note row is unchanged**: `╭` at the anchor, `─` at anchor+1, note text at anchor+2, and
   it sits **immediately above** its code row (keep the existing discriminator).
5. **The arrow is unchanged**: `▴` (U+25B4) shown, `▸` (U+25B8) folded, at the anchor cell.
6. **The click/cursor mapping stays consistent**: clicking an indicator cell maps to the
   **symbol's char** (the annotation's column), which is the landed semantics — re-verify rather
   than assume, since the indicator is now at a different column.
7. **Do not change** the fold semantics, the `C-c a h` toggle, or the note row's `╭─` shape.

## Acceptance

* A **mid-line** symbol: the indicator sits at `display_col - 1`, the code is cell-for-cell
  identical to the source, and the note row's `╭` is at the same column as the indicator
  (the anchor relationship — *presence is not placement*).
* **Wide-char and tab fixtures**: the indicator lands on the correct cell when the symbol is
  preceded by CJK/emoji and by a tab. These are the tests that catch a char-vs-display error.
* The **no-whitespace fallback** (`x+y` annotated at `y`) falls back to the indent anchor and does
  not move the code.
* **Column 0** still shifts by exactly 1.
* **Two annotations on one line** → two indicators at their own anchors + two note rows; folding
  leaves one `▸` each. (The `A` key path dedupes per line, so a hand-built fixture is expected —
  say so in the commit rather than weakening the test.)
* The existing anchor/fold/click tests are **re-expressed, not deleted**, and still discriminate.
* `cargo test --workspace` + clippy `--workspace --all-targets -- -D warnings` clean;
  `tools/gate.sh full` green.

## Fence

`src/app/store/file_view.rs` (the row model + the click mapping), `src/ui/file_view.rs` (the
renderer), `src/ui/root/geometry.rs` if the cursor arithmetic moves, `tools/` drives, and tests.
Disclose anything else with before/after.

## Reporting

Include **literal per-cell frames** for: a mid-line symbol, a wide-char-preceded symbol, a tab
case, the no-whitespace fallback, and two annotations on one line. Those are what the user judges,
and a real PTY capture is worth more than an ASCII transcription if you can produce one cheaply
(the driver's `screen_text()` does it — note that `cargo test --bin` does **not** refresh the
plain binary the PTY driver launches, so run `cargo build --bin redline` first or you will capture
a stale frame).
# issue-annotations-anchor-at-symbol — the indicator moves to the symbol, and the arrow points up

**User's decisions (2026-09-22):** the annotation anchor moves from the **left gutter** to the
**symbol** — with option **(a)** for the column-0 case — and the arrow **points UP when the note
is shown**.

Builds on the landed fold work (`7e97812`, `f08f984`). **This removes the annotation gutter**, so
it is not a pure glyph change: the cursor/click arithmetic, the click pin landed in `ed7a524`,
and the PTY legs that assert gutter offsets all move with it. That is a net simplification (the
offset math disappears) but it must be done deliberately, not discovered.

## Target rendering

Source `    if x > 0 {` — the symbol `if` is at column 4. The indicator borrows the **last cell
of the line's indentation**, so no code character moves:

```
fn main() {
    let x = 1;
   ╭─check the bounds
   ▴if x > 0 {          <- ▴ at col 3, `if` still at col 4
        println!("hi");
       ╭─dead branch
       ▴} else {        <- ▴ at col 7, `}` still at col 8
    }
}
```

FOLDED, the same lines (no note row; the indicator stays, because folding must not erase the
*fact* of an annotation):

```
fn main() {
    let x = 1;
   ▸if x > 0 {
        println!("hi");
       ▸} else {
    }
}
```

**Arrow directions:** `▴` (U+25B4, up) when the note is **shown** — it points at the note, which
is above — and `▸` (U+25B8, right) when **folded**. Both sit at the anchor cell.

**Column 0 (option (a))** — a symbol with no indentation to borrow shifts that line by exactly
**one** cell:

```
╭─the entry point
▴fn main() {
```

## Requirements

1. **The anchor cell is the symbol's column minus one** (`col - 1`), borrowing the last
   indentation cell. The **code text must not move** — assert it per cell (see Acceptance).
2. **Column-0 symbols shift their line by exactly 1 cell** (option (a)). This is the *only* case
   where annotated code moves, and it must be asserted as exactly 1, not "small".
3. **The note row** carries `╭` (U+256D) at the **anchor cell**, `─` (U+2500) at anchor+1, and the
   note text at anchor+2 — so a note's indentation **mirrors the nesting** of the code it
   annotates. Truncation width becomes `w - (anchor + 2)`.
4. **The arrow is `▴` shown / `▸` folded**, at the anchor cell, in both states.
5. **⚠ The gutter is REMOVED — find every consumer.**
   - `src/ui/file_view.rs`'s `gutter = if annotated { 2 } else { 0 }` logic goes away (code starts
     at the source's own column).
   - `src/ui/root/geometry.rs::cursor_cell` and the **click mapping** in
     `src/app/store/file_view.rs` must stop adding a gutter.
   - **The click pin landed in `ed7a524`** (`mouse_click_annotated_line_pins_gutter_column_mapping`)
     asserted `cell 2 → char 0`, `cell 3 → char 1`, gutter cells `0/1 → char 0`. Re-express it
     against the new model rather than deleting it: a click on the **code's own column** maps to
     char 0, a click on the indicator cell maps to the symbol's char (char 0), and on an
     unannotated line column 0 → char 0. **Keep it discriminating** (show the mutation).
   - `tools/check_cursor_stream.py`'s legs and `tools/sweep_flows.py`'s flows that assert the
     `(3,3)`-style gutter offsets and the `▾─`/`╭─` tokens must be updated — **updated, not
     loosened** — including the note-row token and the glyph census.
6. **The note row's text indent is now data the renderer needs per row**: the store builds the
   row, so the anchor column must be carried on the row (a new field) rather than recomputed in
   the renderer from the code text. Say where you put it and why.
7. **⚠ Tabs.** The anchor is a **display-column** position. A line whose indentation contains tabs
   advances to the next tab stop, so "the last indentation cell" must be defined in display
   columns. State the rule you implement and pin it with a tab-indented fixture — do not leave it
   to chance.
8. **Multiple annotations on one line.** The model allows it. Define the behaviour — recommended:
   one indicator per annotation at its own column, with the note rows stacked above in column
   order — and state what folding does to them. Pin whatever you choose.
9. **The fold invariant is re-expressed, not dropped.** The old "constant leading width 2" test is
   obsolete under this design; its *purpose* (folding must not move the code) still holds and must
   be re-aimed: assert the code's start column is identical folded and expanded, for both an
   indented symbol and a column-0 symbol.
10. **Do not change the fold semantics or the `C-c a h` toggle.**

## Acceptance

* **⚠ The strongest test: for an indented symbol, the rendered code row equals the plain source
  row CELL FOR CELL** (compare against the buffer line, not a hardcoded string) — that is what
  "the code does not move" means, and it is what a future refactor is most likely to break.
* The column-0 case shifts by **exactly 1**, asserted.
* The anchor relationship holds (the note row's `╭` is at the same column as the indicator on the
  row directly below) — keep the existing anchor test's spirit, since *presence is not placement*.
* The arrow is `▴` shown / `▸` folded, asserted in both states.
* The note row is **immediately above** its code row (existing discriminator intact).
* The click/cursor pins are re-expressed and **discriminate** (mutation shown).
* The note's indent mirrors nesting: a note for a deeper symbol starts further right, asserted.
* Tabs: a tab-indented fixture behaves by the stated rule.
* `cargo test --workspace` + clippy `--workspace --all-targets -- -D warnings` clean;
  `tools/gate.sh full` green.

## Fence

`src/ui/file_view.rs`, `src/ui/root/geometry.rs`, `src/app/store/file_view.rs` (the row model and
the click mapping), `tools/` drives, and tests. Disclose anything else with before/after.

## Reporting

Include a **literal per-cell frame** for: an indented symbol, the column-0 case, a folded line,
and a note whose indent mirrors nesting. Those frames are the deliverable the user judges.

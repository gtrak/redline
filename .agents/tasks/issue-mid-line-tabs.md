# issue-mid-line-tabs — a tab INSIDE the code makes the anchor model disagree with the renderer

**Found by:** the `annot-symbol` gate (P2-2), as the exposure the symbol-precise anchor created.
**Severity:** an annotation indicator can land **on a code cell** — visible corruption, not just a
misplacement. Rare (a tab *inside* the code, not the leading run) but real.

## The mismatch, established from the vendored sources (not inferred)

The anchor is computed in **display columns** where a tab advances to the next 8-column stop
(`record_anchor`'s char→display loop). The renderer does not agree:

- `unicode_width::UnicodeWidthChar::width('\t') == None` (`unicode-width-0.2.2/src/tables.rs:224`
  — control codes), so
- iocraft's canvas does `c.width().unwrap_or(0)` (`iocraft-0.9.1/src/canvas.rs:225`) → a `\t`
  occupies **0 cells and is merged into the preceding character's cell**, and `write_row_impl`
  streams cell values with no per-cell cursor move (`canvas.rs:380–390`) — so **the terminal**,
  not the canvas, decides where a mid-line tab lands.
- The app's own arithmetic says tab = 1 cell in two more places (`char_display_width` uses
  `width().unwrap_or(1)`, `src/model/text_width.rs:12–17`; `draw_line` advances by
  `display_width(&segment)`), and the store's own comment already concedes it
  ("the canvas would render a tab as ~1 cell, not the next 8-column tab stop",
  `src/app/store/file_view.rs:~585`).

**Three models, three answers.** The anchor's 8-column rule is correct **only where the tab is in
the leading run**, because the store strips that run and owns `code_start`. For a tab inside the
code the models diverge, and the knock-on effects are not confined to the indicator: the **click
mapping** and the **cursor position** are computed in the same cell space, so they are wrong after
a mid-line tab too.

## Why it passed

Every existing tab fixture is a **leading** tab — `\tlet x = 1;` (frame test) and `\tlet z = 3;`
(PTY leg) — which is stripped and never exercises the mid-line arithmetic. The unit test
`record_anchor_tabs_advance_to_eight_column_stops` does pin the arithmetic, but nothing pins the
**rendered** result, which is where the disagreement lives.

## Reproducer (from the gate)

Fixture `"x\t" + "y"*9 + " " + "Z=123456789;"`, record on `Z`: the model computes anchor **17**,
while the canvas/terminal place `Z` around cell **11** — six cells of separation, so the indicator
lands in the middle of the code.

## Fix — needs a decision

1. **Expand mid-line tabs to spaces (to the next 8-column stop) in the row text, store-side**, so
   the canvas cells, the terminal, the anchor, the click mapping and the cursor all agree. This is
   the principled option (it makes the screen match the model everywhere), but it **changes the
   text the renderer draws**, so every highlight span and search-match range must be re-based —
   *a tab is 1 byte but 8 columns*, and a one-byte error here is silently mis-coloured text on
   exactly the annotated lines.
2. **Declare the tab-stop rule leading-run-only** and render a mid-line tab as one cell
   everywhere. Cheaper, but the model must then say so explicitly rather than being right by
   accident for leading tabs.

Whichever is chosen: add **one PTY leg with a mid-line tab** asserting the indicator sits exactly
one cell left of the symbol. Without that leg the sliver stays unpinned.

## Fence

`src/app/store/file_view.rs` (the row build / the tab arithmetic), `src/ui/file_view.rs` (the
renderer), `src/model/text_width.rs`, plus spans/matches if option 1 is taken, tests and drives.

## Acceptance

- A mid-line tab: the indicator is **one cell left of the symbol in the frame**, cell-for-cell.
- Leading tabs unchanged (the existing fixtures must keep passing — they are the regression net).
- If option 1: highlight spans and search matches are still correctly coloured on lines carrying a
  tab, pinned by a test that would catch a one-byte error.
- The click mapping and the cursor position agree with the rendered cell after a mid-line tab.
- `cargo test --workspace`, clippy `--workspace --all-targets -- -D warnings`, `tools/gate.sh full`.
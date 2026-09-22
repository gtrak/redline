# issue-annotations-fold-visual — the fold arrow and the 2-branch tree-line

**User's request (2026-09-22), verbatim:** *"I want the arrow on the left, then when open, I
want a kind of visual 2-branch tree-line, diagonal and up to the note, and straight out to the
real text. And I want 'C-c a h' to be a toggle, no separate 'C-c a s'."*

Builds on the **landed** `issue-annotations-render-fold` work (`eefd0eb`): notes already render
**above** their anchored line, folding already works, and the margin glyph is currently a
stateful **bar** (`U+258E` `▎` when visible, `U+258F` `▏` when folded).

## What changes

1. **The margin glyph becomes a fold ARROW, not a bar.** `▾` (down, open) when the note is shown,
   `▸` (right, folded) when it is hidden, and **no glyph at all** on an unannotated line (the
   arrow *is* the annotation indicator, which is cleaner than the bar). Follow the existing
   convention that anything else in the gutter must stay legible on the dark theme.
2. **`C-c a h` becomes the TOGGLE.** Bind it to the toggle command and **remove
   `C-c a s` entirely** — its binding, its help/doc text, its tests, and any tracker/doc mention.
   The M-x command `annotate-toggle` already exists (it was the bare `C-c a` toggle before `C-c a`
   became a prefix) — **the user's `C-c a h` is that command's proper home**, so prefer reusing it
   over inventing a second toggle. `C-c a n` (new) and `C-c a l` (list) are unchanged.
3. **When the note is shown, draw a 2-branch tree-line** joining the arrow to (a) the note above
   and (b) the code text beside it:
   - a branch that goes **diagonally up** to the note row, and
   - a branch that goes **straight out** to the real text.
   A candidate rendering to start from (the user will react to what they see, so get something
   coherent on screen rather than agonising over the final glyph choice):

   ```
   folded:     ▸ fn foo() {
   expanded:   ╱ note text
               ▾─ fn foo() {
   ```

   The intent to preserve: the arrow is the junction, the diagonal visibly rises from it to the
   note, and the horizontal runs from it to the code. Pick the diagonal glyph from the
   box-drawing diagonals (`U+2571` `╱`, `U+2572` `╲`) or a corner (`╭╰`) if it reads better —
   **say which you chose and why**, and show the rendered rows.
4. **⚠ The code text must NOT shift horizontally when toggling.** If the collapsed state is
   `▸ fn …` and the expanded state is `▾─ fn …`, the code text starts one cell further right when
   open and the whole view jitters on every fold. Make the leading width identical in both states
   (e.g. a space in place of the horizontal when folded) so only the *glyphs* change. **Pin this
   with a test that asserts the code text's start column is the same folded and expanded.**
5. **The note row stays immediately ABOVE its code row**, and the note's own text no longer needs
   the old `▸ ` prefix (the arrow and the diagonal now carry that meaning). Keep the existing
   "nearest subsequent code row is its own line" discriminator intact — do not weaken it.
6. **The fold invariants must still hold** (these are pinned today and must keep passing): folding
   with the cursor **mid-file** leaves the cursor on the same **code** line, the scroll window
   stable, the full row map round-tripping, and the row count genuinely changing.
7. **You own the PTY drives that assert this.** `tools/check_cursor_stream.py` currently asserts
   the `▎`/`▏` bar and a "CUP row moved +1 (note above)" leg; those expectations must be updated
   to the arrow and the tree-line (and any leading-width change updates the cursor column
   arithmetic). Update the `ann-*` sweep flows too — **update them for the new rendering, never
   loosen them** — and make sure a flow still fails if the note stops being drawn above.
8. **A literal rendering dump in your report and commit**: the actual rows for an annotated line
   in both states, as characters, so the user can see the layout without running the TUI.

## Acceptance

* `C-c a h` toggles (shown → folded → shown); `C-c a s` is **gone** (binding, docs, tests).
* The arrow glyphs are `▾`/`▸` per state and absent on unannotated lines, asserted per state.
* The expanded state draws both branches (diagonal to the note, horizontal to the code), asserted
  by glyph in the rendered row text.
* The code text's start column is **identical** folded vs expanded (the jitter test).
* The note is immediately above its code row (existing discriminator still passes).
* The fold-trap invariants still pass (cursor mid-file, scroll, round-trip, row count).
* The PTY drives are updated and still discriminate.
* `cargo test --workspace` + clippy `--workspace --all-targets -- -D warnings` clean;
  `tools/gate.sh full` green.

## Fence

`src/ui/` (the renderer + geometry/offset math), `src/app/store/mod.rs` (the bindings and the fold
state), `src/app/` keymap/command/help text, `tools/` drives, and tests. Disclose anything else
with before/after. Do **not** change the annotation data model or the notes-buffer machinery —
this is rendering and one binding.

## Note on scope

This is a **visual iteration** the user will react to. Prefer getting a coherent, correct,
non-jittering rendering landed with the toggle change, and state honestly in the commit which
glyph choices are provisional, over attempting an elaborate layout that risks the fold invariants.
A one-cell-narrower or differently-angled branch is a fine follow-up; a jittering view or a broken
row map is not.
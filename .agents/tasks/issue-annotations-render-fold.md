# issue-annotations-render-fold — margin indicator, note above the line, folding

**User's decisions (2026-09-22), do not re-litigate:**
- **A margin indicator**, not a note row hanging under the line.
- **The note renders ABOVE the anchored line**, not below.
- **Fold the note blocks in and out.**
- **The command tree is `C-c a`** — `C-c a n` (new), `C-c a h` (hide), `C-c a s` (show),
  `C-c a l` (list). **`C-a` was explicitly declined** because it is emacs's
  `point-line-start`; `C-c` is the emacs convention for mode-specific commands and we already
  have an annotation family there (`C-c a` toggles, `C-c n a` is the picker).
- Shift-as-a-fold-accelerator: **conditional**, see requirement 5.

## What exists today (verified)

`src/ui/file_view.rs`, in the row loop:
- An **annotated code line** gets a 1-cell left gutter: `\u{258e}` (`▎`) at cell 0 and the code
  shifted to cell 1. The marker uses `t.view_title.foreground`.
- A **note row** is a *virtual* row inserted **directly after (below)** the anchored code row,
  rendered dim+italic from `t.preview.foreground`, text prefixed `▸ `, and **truncated** to
  `w.saturating_sub(1)`.
- Note rows **count as rows** in the precomputed row map: `is_note` on `RenderedRow`, the row
  map round-trip test in `file_view.rs`, `src/ui/root/geometry.rs`, and the
  `find(|r| !r.is_note …)` code-row lookup in `src/ui/root/snapshot.rs` all depend on it.

## Requirements

1. **Note blocks render ABOVE the anchored line.** The virtual note row moves from after the
   code row to **before** it, so the note reads as a header for the code it annotates. Every
   place that assumes "the note is below" must be found and fixed, not just the emitter: the
   row map, the code-row lookup (`snapshot.rs`'s `!r.is_note` search must still find the
   *code* row, and any "the row below this note is its code" assumption is now wrong), cursor
   placement, scroll math, and the selection/region painting. Confirm the round-trip test
   still means something after the move (it must assert the note is *immediately before* its
   code row, otherwise it would pass under either ordering and be vacuous).
2. **The margin indicator.** Keep the 1-cell gutter, and make it carry state rather than being
   a constant: it must distinguish at least (a) a line with a visible note, (b) a line whose
   note is folded/hidden, and (c) no annotation. Use `t.view_title.foreground` for the
   ordinary case and say what you used for the others (and why it stays legible on the dark
   theme). The indicator must be present whether the note block is shown or folded, since
   folding must not erase the fact that an annotation exists.
3. **Folding.** `C-c a h` hides the note blocks; `C-c a s` shows them. **State your scoping
   decision** — global (all notes in the current view) vs per-annotation — and justify it;
   `C-c a l` exists for finding individual notes, which is an argument for a global fold.
   The fold state must be reflected in the margin indicator (requirement 2) and must not
   corrupt the row map: folding changes the number of rendered rows, so the map, the cursor's
   code row, and the scroll position must all stay consistent across a fold toggle. Pin that
   with a test that folds with the cursor mid-file and asserts the cursor is still on the same
   **code** row.
4. **`C-c a n` (new) and `C-c a l` (list)** must reach the commands that already exist —
   `A` prompts for a new annotation (plan 005 issue 02) and `C-c n a` opens the annotations
   picker (015-01). Bind them under the tree as genuine aliases (same command), and pin each
   binding with a keymap test so a later refactor cannot silently drop one.
5. **Shift-to-fold is conditional — verify, do not assume.** crossterm 0.29 *can* represent a
   bare modifier (`KeyCode::Modifier(ModifierKeyCode::LeftShift)`), but **only** for keycode
   `57441` in the enhanced/kitty keyboard protocol range; a byte-based terminal sends nothing
   for a bare Shift press. So: **first determine whether our terminal setup actually enables
   keyboard enhancement** (look at how the terminal is initialised — iocraft owns it; check
   whether the enhancement flags are pushed). If they are **not** enabled, do **not** fight the
   framework to enable them: report that finding and leave Shift unbound, with `C-c a h`/`c a s`
   as the fold path. If they *are* enabled, bind bare Shift as an accelerator **in read-only
   mode only** (a fold toggle) and pin it; state plainly in the commit which terminals it
   works on. A binding that silently never fires on the user's terminal is worse than no binding.
6. **The PTY drives own this behaviour.** `tools/` has annotation flows (`ann-create`,
   `ann-toggle`, `ann-crossing`, `ann-notes-editable`, `ann-drift`, `ann-orphan`,
   `ann-delete`, `ann-cu-scroll`) whose expectations encode the note **below** the line. A lane
   that changes behaviour owns the drives that assert it: update them for the new position,
   add a fold flow, and do not leave a flow passing only because it does not check the
   position.
7. **Do not regress the invariants the annotation work already established**: annotations land
   on their recorded **column**, the notes buffer stays editable, and a note row must not be
   counted as a code line by the cursor or by goto-line.

## Acceptance

* The note renders **immediately above** its anchored line, asserted by a test that fails if
  the ordering is reversed (not merely a "a note row exists" assertion).
* The margin indicator distinguishes visible note / folded note / no annotation, asserted.
* `C-c a h` then `C-c a s` toggles the note blocks, and the cursor stays on the same code row
  across the toggle; the row map round-trips after each toggle.
* `C-c a n` reaches the new-annotation prompt and `C-c a l` reaches the annotations picker,
  each pinned by a keymap test.
* The Shift finding is reported explicitly (enabled or not), with the evidence — and if bound,
  the test says which terminals it works on.
* The PTY flows are updated for the new position and a fold flow exists.
* `cargo test --workspace` + clippy `--workspace --all-targets -- -D warnings` clean;
  `tools/gate.sh full` green.

## Fence

`src/ui/file_view.rs`, `src/ui/root/geometry.rs`, `src/ui/root/snapshot.rs`, `src/ui/mod.rs`,
`src/app/store/mod.rs` (the bindings + the fold state), `src/app/keys.rs`, `tools/` drives, and
tests. **Do not edit `src/app/store/notes.rs`, `src/app/store/buffers.rs` or
`src/model/buffer.rs`** — another lane (016-03, the dirty-flag work) holds those right now; if
the annotation model genuinely needs a change there, **stop and report** rather than colliding.
Disclose anything else with before/after.
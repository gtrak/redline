# issue-yank-followups — C-y does not advance the point, can land off-screen, and leaves the notes doc stale

**Found by:** the `015-04` (yank) gate, as P3-1, P3-2 and P3-3. Each was measured by the gate with
a temporary probe that was then removed; the measurements are quoted below.

## 1. `C-y` does not advance the point — a real emacs-parity gap (UX)

**Measured:** after `set_point(0,1,1)` then `yank("XYZ")` the buffer is `"aXYZb\ncd\n"` and
**`point_col()` is still 1**. `yank` never calls `land_point_at_char`; the point is a stored
`(line,col)` that no edit path advances here.

**Emacs:** `C-y` leaves point **after** the inserted text — that is the whole point of yanking, you
type on from where the text landed. So redline's behaviour is wrong in a way users will feel
immediately: yank, type, and the text appears *before* the yank instead of after it.

This was hiding behind a **false comment** in three places (the code, the test comments, and the
spec) claiming "in Accurate mode the point moves past the inserted text". The implementation was
correct anyway (`yank_pos` is the inserted range and that is what `M-y` replaces), which is exactly
why nobody noticed the rationale was fiction. **Lesson worth keeping: a comment asserting behaviour
is a claim — this one was measurably false and masked a user-visible bug.**

## 2. A yank can land off-screen with no feedback (UX)

The comment "No scroll adjustment: the insertion is at the current top line" is false in
**Annotation** mode, where the insertion is the buffer **end**. Self-insert calls
`buffer_keep_insert_visible`; `yank` deliberately does not. So a `C-y` into a long notes buffer can
append text the user cannot see, with no indication it happened.

## 3. A yank does not mark the notes doc dirty — a functional bug

**Measured:** `notes_buffer_dirty` is `false` **before and after** a yank in **both** modes, while
every other notes edit path (`notes_insert_char`, `notes_backspace`, `mark_notes_dirty_if_current`)
sets it. So a `C-y` into the notes buffer leaves the annotation doc **un-reparsed** — the yank
appears to do nothing until some other edit triggers a reparse.

Pre-existing (the base `yank` lacked it too) and outside 015-04's named hygiene trio, which is why
the gate recorded it rather than blocking. It is still a bug.

## Acceptance

- After `C-y`, the point is at the **end of the inserted text** (emacs), in both modes, pinned by a
  test that fails on the current behaviour.
- After `C-y`, the insertion is **visible** — the view scrolls it into range, matching self-insert.
- A yank into the notes buffer marks the doc dirty so it is reparsed, pinned by a test asserting
  the reparsed doc contains the yanked note.
- `M-y` still replaces the **inserted range** (`yank_pos`), not the moved point — the landed tests
  must keep passing unchanged, and this change must not re-split 016-02's one-step coalescing.
- `cargo test --workspace`, clippy `--workspace --all-targets -- -D warnings`, `tools/gate.sh full`.

## Note (no action) — P3-4, a coverage nuance, not a defect

Two of 015-04's four new Annotation tests do **not** discriminate the append position: in
`yank_annotation_append_is_char_accurate_with_multibyte` and
`yank_pop_coalesces_with_annotation_yank_into_one_undo_step` the point already sits at the trailing
empty line (= buffer end), so point-insert ≡ append there. They correctly pin char-accuracy and
undo-coalescing respectively. The append semantics themselves are pinned by
`yank_in_annotation_mode_appends_at_the_end` and the Annotation half of
`yank_pop_replaces_at_the_original_yank_position_in_both_modes`. Coverage is complete — just do not
read those two as mode-discriminators.

# issue-016-03-dirty-flag-and-reload — the saved-state marker and history clearing

Parent plan: `.agents/plans/016-undo/PLAN.md` — read it in full first; it is the design.
This issue is **03 of 04**. 01 (landed) built the stack + the guard and deliberately left
`drop_undo_history(key)` **unwired** for you. 02 (landed) retrofitted the five remaining edit
paths. **04** owns redo and the self-insert-run coalescing rule — do not pull them forward,
but see the seam note at the bottom, because **redo will move the history position** and your
marker has to survive that.

## Why this is the trap issue

The plan's own risk list puts it first: *"Getting it wrong produces a buffer that is
permanently 'modified' or, worse, one that reports clean while holding unsaved edits — a
data-loss path."*

**The two failure directions are not equally bad, and that asymmetry is the design.** A
false "modified" is an annoyance: the quit prompt asks an extra question, and a save writes
identical bytes. A false "clean" is **data loss**: `C-x C-c` closes without asking and the
user's edits are gone. So when the marker cannot *prove* the buffer matches the disk state,
it must report **modified**. State this tie-break in the code and pin it.

## Requirements

1. **A saved-state marker in the history, not a content comparison.** Track *where in the
   edit history the saved/loaded state sits*. Recommended shape (justify your choice if you
   depart from it): each recorded step carries a unique monotonically increasing id; the
   buffer stores the identity of the **top-of-history position at the last save/load**, using
   a sentinel for "empty history / freshly loaded". The buffer is clean **iff** the current
   position's identity equals the marker. This makes an edit-then-undo-back-to-saved report
   clean, which a naive `locally_modified = false`-on-save cannot do.
   - **The cap, the clears and the coalescing all break naive position arithmetic**, which is
     where this goes wrong. Two specific traps to handle and pin:
     - **Cap eviction**: 01's cap drops the *oldest* steps. If the marker's step is evicted,
       the saved state is no longer reachable by undo — so the buffer **must** report
       modified (conservative), never clean-by-default.
     - **The mutation hints**: 02 added an `M-y` coalescing helper that **pops two steps and
       pushes one**. Decide what id the merged step carries and say why; the marker must not
       be able to point at a step that a merge silently replaced, or the buffer can go clean
       while holding edits. Pin it with a test that saves, yanks, `M-y`-rotates, and undoes
       back to the marker.
2. **Wire `drop_undo_history(key)` at all THREE rope-assigning sites** — and say in the
   commit that there are three, because this class has had one more instance than we thought
   three separate times (the fifth reload route; then the third rope-assigning site, whose
   omission was a **reachable panic** in 01):
   - `reload_in_place` (`src/app/store/index_wiring.rs`) — the chokepoint;
   - `toggle_ro_accept` (`src/app/store/buffers.rs`) — assigns `buf.rope` directly;
   - `replace_buffer_text` (`src/app/store/buffers.rs`) — the **notes-buffer sync**, called by
     `sync_notes_from_doc`, reachable from annotation **save/delete/reanchor** on the
     *editable* notes buffer. This is the site the 01 gate found the hard way.
   Clearing the history must also **reset the marker** — a cleared history can no longer
   prove anything, so the buffer stays modified until the next save/load.
3. **The reload contract composes with the ask policy that already landed.**
   `issue-external-change-reload` implemented policy **(a) ask** (emacs `revert-buffer`):
   on an externally changed file, a **dirty** reopen arms a confirm (`y` reloads, `n`/`C-g`/ESC
   keeps the edits and flags `changed_on_disk`); a **clean** reopen reloads silently; and a
   **held** key is never rebuilt. Verify your work against the *landed* behaviour rather than
   re-deriving it: after a `y` reload the history **must** be cleared and the marker reset
   (the recorded offsets refer to content that no longer exists); after `n` the history stays
   **intact** (the user kept their edits) and the buffer stays modified. Assert both.
4. **Set the marker at every save/load**, and find them all rather than the obvious one.
   Known sites to check: `save_buffer` / `save_buffer_key` (`src/app/store/buffers.rs:871`,
   `:885`, the `locally_modified = false` at `:925`), the notes path
   (`src/app/store/notes.rs:408`), and the two reload sites that already clear the flag
   (`index_wiring.rs:268`, `:318`). If a save path does **not** update the marker, saving
   leaves the buffer permanently modified — pin every path you find, and list any you did not.
5. **Never claim clean on evidence you do not have.** If the marker is `None`, unresolvable,
   evicted, or the history was cleared, the answer is modified. A test must demonstrate the
   conservative direction explicitly (force the marker unresolvable → the buffer reports
   modified), not merely the happy path.
6. **Buffer kill drops the history and the marker** (01 covers the history; confirm the
   marker cannot survive a reopen and leak a stale clean state to a *different* file).
7. **Do not regress the read-only / annotation-mode story.** `locally_modified` participates
   in `buffer_baseline_editable` and the mode line; 015-03 made it buffer-relative
   (`Buffer.is_notes`). Do not re-freeze that behaviour.

## Acceptance

* **The round-trip matrix**, each asserted on the text *and* the flag: edit → save → clean;
  edit → save → edit → modified; edit → save → edit → **undo** → clean; then **edit again →
  modified** (the reverse direction the plan calls out).
* **The data-loss direction is pinned**: a test that would fail if the buffer reported clean
  while holding unsaved edits. Say plainly which test carries this.
* **Save at an intermediate point**: edit → edit → save → undo → undo → **modified** (the
  buffer is now *before* the saved state, so it differs from disk) — this is the case a
  depth-comparison gets wrong and a marker gets right.
* **Cap eviction**: with the cap exceeded, a buffer whose saved marker was evicted reports
  **modified**, not clean.
* **All three rope-assigning sites** clear the history *and* reset the marker — a test per
  site, at least one driven through the **notes** path (`sync_notes_from_doc`) since that is
  the reachable one; and undo after each clear is a no-op with a message, not a panic.
* **Reload contract**: `y` clears + resets, `n` preserves the history and stays modified
  (composed with the landed ask policy, not re-derived).
* **Kill/reopen**: a reopened file starts with no history and a clean-or-fresh marker.
* Undo still goes through the reparse hook (01/02 invariant) and the history cap still holds.
* `cargo test --workspace` + clippy `--workspace --all-targets -- -D warnings` clean;
  `tools/gate.sh full` green.

## Fence

`src/app/store/buffers.rs` (the three sites, the save paths, the marker), `src/app/store/
index_wiring.rs` (the reload chokepoint), `src/app/store/notes.rs` (the notes save path),
`src/model/buffer.rs` (the marker field + `locally_modified` derivation), and tests.
Disclose anything else with before/after.

## Seams

* **04 (redo)** — your marker defines "where the saved state sits"; redo moves the position
  **forward** again. Leave the marker expressed in a way that survives a redo stack existing
  (i.e. do not encode it as `history.len()`, which redo will make wrong). Say in the commit
  exactly what 04 must do to keep the marker correct.
* **04 (coalescing)** — requirement 1's merge question is the only coalescing you touch.
  Do not implement the general self-insert-run rule.
* Do not wire anything into the **commit editor** (its own text model; out of scope per the
  plan) and do not change the recording hook's shape.
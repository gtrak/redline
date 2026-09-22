# issue-016-01-undo-stack — the undo stack (the foundation)

Parent plan: `.agents/plans/016-undo/PLAN.md` — read it in full first; it is the design.
This issue is **01 of 04**: the stack and the inverse-edit recording, **undo only**, for
**self-insert and backspace**. Retrofit width (`RET`, `C-k`, `C-y`/`M-y`, `C-w`) is 02; the
dirty-flag + reload contract is 03; redo + coalescing is 04. Do not pull them forward.

## Why this is its own issue

Undo is a **subsystem, not a command** — an undo stack, coalescing rules, and a dirty-flag
contract. Plan 015 deliberately excluded it for that reason. This issue lands the stack and
proves the recording mechanism on the two simplest edit paths, so 02–04 have something
sound to extend.

## The good news the plan already established

Every text edit already calls `retain_rope_edit(key, old_rope, char_start, char_end, text)`
for the incremental reparse. **That call already receives exactly the data an inverse edit
needs**: the affected char range and the text that was there. So undo does not need new
bookkeeping at the call sites — it needs a stack that records the inverse of each edit, and
**the hook belongs beside `retain_rope_edit`** rather than at each command. Record the
inverse in **one** place so 02's retrofit is a no-op for the recording logic, and say in the
commit whether that held.

## Requirements

1. **Record an inverse edit, not a document snapshot.** Shape it as
   `{ range: Range<usize>, removed: String, inserted: String }` (char indices, matching the
   units `retain_rope_edit` takes — a byte/char mix-up here is this project's most recurring
   defect class). A rope-per-step would work (ropey clones cheaply) but the history would
   grow with the **document** instead of with the **edits**. State the choice.
2. **Per-buffer, emacs-style.** Undo in buffer A must never touch buffer B, and **killing a
   buffer drops its history** (a reopened file must not inherit stale offsets). Test both.
3. **Undo re-applies through the same path as an edit**: `retain_rope_edit` +
   `invalidate_highlight_for_key` (and `locally_modified` set). A shortcut that mutates the
   rope without the reparse hook drifts the retained tree from the rope — the exact class
   `retain_rope_edit` exists to prevent. Pin it.
4. **Bindings — SETTLED by the user, do not re-litigate: undo is bound to BOTH `C-x u` and
   `C-/`.** Both are currently free; `C-x u` fits the existing `C-x` prefix family with no
   collision. **CORRECTED (this issue's original premise was FALSE):** `C-/` does **not**
   pick up `C-_` for free. crossterm 0.29.0 decodes control byte `0x1F` to
   `Char('7') + CONTROL` (= `C-7`), so on a byte-based terminal the physical Ctrl+/ arrives
   as **`C-7`** and the `C-/` binding never fires; `C-/` fires only on CSI-u / kitty
   terminals. `C-x u` works everywhere. The general rule this typo teaches: *assert what
   crossterm reports, do not reason about what it should report.* Binding `C-7` is
   **issue 02's**, since it is the same physical key on byte-based terminals.
5. **Only in editable buffers, and only where an edit is possible.** Undo must be a no-op
   (with a message) in a read-only buffer, and must not resurrect the mode or `editable`
   state. `Accurate ⟹ editable` stays untouched.
6. **Bounds.** Cap the history (entries and/or bytes) so a long session cannot grow without
   limit, and state what dropping the oldest step means (undo simply stops earlier). A test
   that exceeds the cap.

## Acceptance

* An edit sequence can be **fully undone**, with the buffer text byte-identical to the
  expected intermediate state after each step — assert the text at every step, not just the
  end state.
* Per-buffer isolation, and history dropped on buffer kill.
* A test proving undo goes through the reparse hook: after undo, the highlight cache and the
  rope agree (or the retained tree is demonstrably consistent) — a rope-only undo must fail
  this.
* The multibyte case: an edit and its undo on a line with non-ASCII before the edit point
  restores the exact text and the exact point (char indices, not bytes).
* Both bindings work (`C-x u` and `C-/`), and the control-code fact for `C-/` is asserted
  rather than assumed.
* Read-only buffer: undo is a no-op with a message.
* The cap is enforced and demonstrated.
* `cargo test --workspace` + clippy `-- -D warnings` clean; `tools/gate.sh full` green.

## Fence

`src/app/store/buffers.rs` (the edit sites and the recording hook), `src/app/store/mod.rs`
(the per-buffer state), `src/model/buffer.rs` (if the history lives on the buffer), the key
tables for the two bindings, and tests. Disclose anything else with before/after.

## Seams the later issues own — leave them clean

* **03** owns the saved-state marker and `locally_modified` correctness, and the
  history-clearing on reload. Note the reload site is **two places, not one**: the
  `reload_in_place` chokepoint (`src/app/store/index_wiring.rs`) **and**
  `toggle_ro_accept` (`src/app/store/buffers.rs`), which assigns `buf.rope` directly — see
  `issue-external-change-reload.md` F3. Make the seam obvious so 03 cannot miss one.
* **04** owns redo and the emacs self-insert-run coalescing rule. **Do not implement
  coalescing here** — without it every keystroke is one undo step, which is expected at this
  stage; say so in the commit so it is not mistaken for a bug.

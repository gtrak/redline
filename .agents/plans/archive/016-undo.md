# Plan 016 — undo (a subsystem, not a command)

**Status:** design only; no issues specced. Deliberately separate from plan 015.

**Settled since (user directive, 2026-09-21):** undo is bound to **both `C-x u` and
`C-/`** (see §3 item 4). The user reported that `C-x u` is what they reach for, so it is
not optional.

## 1. Why its own plan

There is **no undo anywhere in the codebase** — no command, no history, nothing. That is
survivable while editing is coarse (append at the end; a mistake is one keystroke to fix)
but **plan 015-03 makes it acute**: once accurate mode can insert mid-line, split lines
and kill lines, there is no way back from a mistake. The user's cutline deliberately
excludes undo from 015 ("defer deliberately") precisely because it is a *subsystem* — an
undo stack, coalescing rules, and a dirty-flag contract — not a command.

## 2. The good news: the edit sites already carry what undo needs

Every text edit already calls `retain_rope_edit(key, old_rope, char_start, char_end, text)`
for the incremental reparse. That call already receives exactly the data an **inverse
edit** requires: the affected char range and the text that was there. So undo does not
need new bookkeeping at the call sites — it needs a stack that *records* the inverse of
each edit, and the hook belongs beside `retain_rope_edit`.

## 3. Design questions to settle (with recommendations)

1. **The record shape.** Prefer an **inverse edit** — `{ range: Range<usize>, removed:
   String, inserted: String }` — over storing a whole `Rope` per step. (ropey has cheap
   clone semantics, so a rope-per-step would work, but the history's memory would grow
   with the *document*, not with the *edits*; an inverse edit grows with the edit.)
2. **Granularity.** Per buffer (emacs is buffer-local). Undo in buffer A must never touch
   buffer B, and killing a buffer should drop its history.
3. **Coalescing.** Without it, every keystroke is one undo step, which is unusable for
   typing. Emacs's rule: consecutive self-inserts coalesce; any other command ends the
   run. Implement that rule explicitly and state it, rather than a time-based heuristic.
4. **Undo vs redo.** Simplest coherent model: one undo stack; an undo pushes the inverse
   onto a redo stack; **any new edit clears the redo stack** (emacs's behaviour).
   **SETTLED (user directive): undo is bound to BOTH `C-x u` and `C-/`** — both verified
   free in the keymap tables, and `C-x u` fits the existing `C-x` prefix family with no
   collision (`C-x C-x`, `C-x 0`, `C-x b`, …). Note a terminal subtlety worth pinning
   rather than being surprised by: **`C-/` and `C-_` are the same control code (0x1F)** on
   most terminals, so they are indistinguishable. **CORRECTED after verification (the
   original claim here — "binding `C-/` picks up `C-_` for free" — is FALSE):** crossterm
   0.29.0 decodes byte `0x1F` to `Char('7') + CONTROL`, i.e. **`C-7`**, so a byte-based
   terminal never produces `Char('/') + ctrl` and the `C-/` binding does **not** fire
   there; it fires only on CSI-u / kitty-protocol terminals. `C-x u` works everywhere.
   **Therefore `C-7` must be bound too** (it is the same physical key on byte-based
   terminals, and it is free in the table) — see issue 02. The general rule the original
   mistake teaches: *assert what crossterm reports; do not reason about what it should
   report.* Redo has no emacs
   default — state your choice and justify it.
5. **The dirty flag — the requirement that is easy to get wrong.** Undoing back to the
   content that was last saved/loaded must clear `locally_modified`, or the buffer keeps
   claiming unsaved changes forever. This needs a **saved-state marker** in the history
   (emacs records a boundary in the undo list), and it must also handle the reverse: a new
   edit after reaching the saved state re-sets the flag.
6. **Interaction with the highlight cache and the watcher.** Each undo is an edit: it must
   call `retain_rope_edit` + `invalidate_highlight_for_key` like any other edit. A file
   **reloaded from disk** (or changed-on-disk conflict resolved) must **clear** the
   history — the recorded offsets are invalid.
7. **Which edits are undoable.** All *text* modifications (self-insert, backspace, `RET`,
   `C-k`, `C-y`/`M-y`, `C-w`). Not point motion, marks, or scroll. `M-y` (yank-pop) is a
   *replacement* — decide whether it coalesces with the preceding yank into one step
   (emacs effectively does) and state it.
8. **Bounds.** Cap the history (entries and/or bytes) so a long session cannot grow
   without limit; state the cap and what dropping the oldest step means (undo simply
   stops earlier).
9. **Scope.** The notes buffer and any editable file buffer. The **commit editor** has its
   own text model and is **out of scope** initially — say so rather than silently leaving
   it inconsistent.

## 4. Success criteria

- Undo/redo round-trips: an edit sequence can be fully undone and redone, with the buffer
  text byte-identical to the expected intermediate states.
- The dirty flag is *correct*: undo to the saved state clears it; a further edit restores it.
- Per-buffer isolation, and history dropped on buffer kill / disk reload.
- No highlight-cache drift after undo (the incremental reparse stays consistent — the same
  invariant `retain_rope_edit` exists to preserve).
- Memory bounded by the cap, demonstrated by a test that exceeds it.

## 5. Task order (to be specced when picked up)

| Issue | Scope |
|---|---|
| `01-undo-stack.md` | the stack + inverse-edit recording beside `retain_rope_edit`, undo only, for self-insert/backspace |
| `02-edit-coverage.md` | `RET`, `C-k`, `C-y`/`M-y`, `C-w`; the yank-pop coalescing decision |
| `03-dirty-flag-and-reload.md` | the saved-state marker, `locally_modified` correctness, history clearing on reload/kill |
| `04-redo-and-coalescing.md` | the redo stack + the emacs self-insert-run coalescing rule |

## 6. Risks

- **The dirty flag is the trap.** Getting it wrong produces a buffer that is permanently
  "modified" or, worse, one that reports clean while holding unsaved edits — a data-loss
  path. It deserves its own issue (03) and its own tests.
- **Coalescing heuristics can eat edits**: if a typing run is coalesced too aggressively,
  undo skips content the user expects back. Pin the rule with tests over a known edit
  sequence rather than trusting the heuristic.
- **Ordering against the highlight cache**: undo must go through the same
  `retain_rope_edit` path, or the retained tree drifts from the rope and highlighting goes
  subtly wrong (this is the exact class the `retain_rope_edit` invariant exists for).

# issue-016-04-redo-and-coalescing — the redo stack and the self-insert-run rule

Parent plan: `.agents/plans/016-undo/PLAN.md` — read it in full first. This is **04 of 04**,
the last issue of plan 016. 01 (stack + guard) and 02 (edit coverage + the `M-y` coalescing)
have landed; **03 (the saved-state marker) is in review** — its seam notes for you are at the
bottom of this file and you must honour them.

## What is missing

- **No redo.** `C-x u` / `C-/` undo, and an undo throws the inverse away; there is no way back
  forward. The plan's model: *one* undo stack; an undo pushes the inverse onto a **redo**
  stack; **any new edit clears the redo stack** (emacs's behaviour).
- **No general coalescing.** 02 added a *scoped* rule for the yank sequence only (`M-y`
  merging with the preceding `C-y`). General typing is still one undo step per keystroke, which
  is unusable for real typing — 01/02 both stated this is expected at their stage, not a bug.

## Requirements

1. **The redo stack.** An undo pushes the inverse of what it undid onto a redo stack; redo
   re-applies it and pushes the inverse back onto the undo stack. **Any new edit clears the
   redo stack** — this is the emacs rule and it is not optional: keeping a stale redo branch
   after an edit lets the user redo onto content that no longer exists, which is the
   correctness problem the rule exists to prevent. Pin it with a test that edits, undoes,
   then edits again, and asserts redo is unavailable (or is a no-op with a message).
2. **The redo stack obeys the same discipline as the undo stack**: per buffer, dropped with
   the buffer, cleared wherever the undo history is cleared (the **three** rope-assigning
   sites, through 01's `drop_undo_history` helper), and bounded by the same cap. Do not let it
   become an unbounded second history.
3. **⚠ The marker seam (03 owns the mechanism, you must not break it).** 03 replaced the
   stored `locally_modified` flag with a **derived** value: clean iff
   `undo.position_id() == saved_marker`, where a step's `id` is a unique monotonic value from a
   buffer-level counter (`None` marker = no evidence = modified). Its doc states that a step
   re-entering the stack **must keep the same id**, which is what makes redo safe for the
   marker. So: **verify that a redone step re-enters with its original id**, and pin the
   round trip — edit → save → edit → undo → **redo** → the buffer must read **modified**, and
   edit → save → undo past → redo back to the saved position must read **clean**. If a redo
   has to mint a fresh id, the marker must be updated in the same breath; either way, **state
   what you did and prove the clean/modified readings with tests**.
4. **The self-insert-run coalescing rule — the emacs rule, stated explicitly, not a timer.**
   Consecutive **self-inserts** coalesce into one undo step; **any other command ends the
   run**. Do not implement a time-based heuristic (the plan explicitly prefers the stated rule
   over a timer). State precisely what counts as continuing the run: same buffer, contiguous
   insertion point, and no intervening command. Pin it over a **known edit sequence** —
   e.g. type `abc`, move point with an arrow, type `def`: undo must remove `def` as its own
   step and then the `abc` run — rather than trusting the heuristic to "feel right".
5. **Do not break 02's `M-y` rule.** 02 merges the `M-y` rotation with the preceding `C-y`
   into **one** step, and its gate proved the merge cannot lose a step. Your general rule must
   not re-split that sequence or merge across it. Say how the two rules compose.
6. **The redo binding is an open decision — do not guess it.** Settled with the emacs 30.2
   oracle (`emacs -Q --batch`, `where-is-internal`): emacs binds `undo-redo` to **`C-?`**
   (plus a menu-bar entry), `C-x u` is `undo`, and **`C-x r` is the register prefix** (so it is
   *not* free, which is the obvious wrong guess). `C-?` is DEL/`0x7F`, which a terminal
   typically delivers as Backspace — so emacs's own binding **cannot be copied into a TUI**.
   Therefore: implement redo as a **command** with a clearly stated binding of your choosing
   (justify it, keep it free in the table, note the terminal caveat), and flag the choice in
   your report for the user to confirm. Bind it so it is reachable on a byte-based terminal —
   a binding that only fires under the kitty protocol is not acceptable as the *only* path.
7. **The reparse invariant still holds for redo**: a redone edit must go through the same
   `retain_rope_edit` + `invalidate_highlight_for_key` path, or the retained tree drifts from
   the rope (the exact class that invariant exists to prevent).

## Acceptance

* A **full round trip**: an edit sequence undone completely and then redone completely, with
  the buffer text **byte-identical** to the expected state asserted at **every** step (not
  just the endpoints).
* A new edit **clears** the redo stack, pinned.
* Coalescing pinned over a known sequence, including the "any other command ends the run"
  half (a point-motion between two typing runs must produce two steps, not one).
* The `M-y` sequence from 02 still coalesces as one step (no regression).
* The marker readings through redo: the two cases in requirement 3, pinned.
* The redo stack is per-buffer, dropped on kill, cleared at the three rope-assigning sites,
  and capped.
* Multibyte: a coalesced typing run with non-ASCII restores exactly, on **char** indices.
* `cargo test --workspace` + clippy `--workspace --all-targets -- -D warnings` clean;
  `tools/gate.sh full` green.

## Fence

`src/app/store/buffers.rs` + the undo/redo state (`src/model/buffer.rs` if the stacks live
there), the key table for the redo binding, and tests. **Do not touch the recording hook's
shape**, and do not re-do 03's marker work — if redo needs something from the marker, state it
and implement the minimum. Disclose anything else with before/after.

## Notes from 03 (the marker issue) — read before you design

* `Buffer.saved_marker: Option<usize>` = the identity of the top-of-history position at the
  last save/load: `Some(0)` = fresh/empty history, `Some(id)` = the saved top step's id,
  `None` = **no evidence → modified**. `locally_modified()` is derived, never stored.
* The `id` comes from a **buffer-level counter**, not a stack index, precisely so that cap
  eviction and redo cannot invalidate it. If your redo re-pushes a step, **re-use the step's
  own id**; minting a new one silently breaks the marker's clean/modified reading.
* The rope-*replacement* path `insert_rope` discards the whole `Buffer` (so it is fresh/clean
  by construction); the three sites that replace content **in place** are the ones that must
  clear both stacks.
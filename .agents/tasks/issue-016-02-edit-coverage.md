# issue-016-02-edit-coverage — retrofit undo onto every edit path, and bind `C-7`

Parent plan: `.agents/plans/016-undo/PLAN.md` — read it in full first; it is the design.
This issue is **02 of 04**. 01 (landed) built the stack and the hook, for self-insert and
backspace only. 03 owns the dirty-flag + reload contract; 04 owns redo and the
self-insert-run coalescing rule. Do not pull them forward.

## The claim this issue must TEST, not assume

01's central bet was that **every** text edit already funnels through
`retain_rope_edit(key, old_rope, char_start, char_end, text)`, so recording the inverse
**once, beside that call**, means retrofitting the remaining paths needs **no recording
change at all**. 01 verified that by enumerating the text-mutating paths and proving they
reach the hook; a gate independently reproduced it and also verified by execution that
`RET`, `C-k`, `C-y` and `C-w` undos already restore correctly with the hook untouched.

**So the headline deliverable here is an honest per-path verdict, not new machinery.** For
each of `RET`, `C-k`, `C-y`, `M-y`, `C-w`: state whether undo already restores it **with the
recording code unmodified**, or whether that path needed a change — and if it did, show the
diff and explain why the hook missed it. A path that turned out *not* to be covered is the
most valuable thing you can report; do not quietly add a hook and call it expected.

## Requirements

1. **Bind `C-7` for undo.** On a byte-based terminal the physical Ctrl+/ sends control byte
   `0x1F`, which crossterm 0.29.0 decodes as `Char('7') + CONTROL` — `C-7` — so the existing
   `C-/` binding never fires there (`C-/` works only on CSI-u / kitty terminals). `C-7` is
   currently free in the table. Add it next to the other two undo bindings, keep the table
   comment honest (it already documents the decode correctly — make sure the *code* now
   matches the *comment*), and pin it with a test that asserts the app-level mapping for the
   `0x1F` byte reaches the undo command. Say in the commit which terminals each of the three
   bindings covers, so the user is not left guessing which key works where.
2. **Cover the five remaining edit paths.** `RET`, `C-k` (kill-line), `C-y` (yank),
   `M-y` (yank-pop) and `C-w` (kill-region). Each is a *text* modification and must be
   undoable; point motion, marks, scroll and mode changes are **not** undoable and must not
   enter the history.
3. **`M-y` is a *replacement*, and that is a decision to state.** `M-y` swaps the
   just-yanked text for the previous kill, so its inverse is not a plain insertion. Decide
   and state whether `M-y` **coalesces with the preceding `C-y` into one undo step** (emacs
   effectively does — one undo should remove the whole yank-and-rotate sequence) or is its
   own step. Implement the choice and pin it with a test over a known sequence
   (`C-k` … `C-y` … `M-y`, then undo, asserting the text at each step). This is a *scoped*
   coalescing decision for the yank sequence only — 04 owns the general self-insert-run rule.
4. **The kill-ring interaction must be stated, not left implicit.** Undoing a `C-k`/`C-w`
   restores the killed text to the buffer; say whether it also touches the kill ring
   (emacs's undo does **not** pop the kill ring) and assert the chosen behaviour. A silent
   divergence between the buffer and the kill ring after undo is the bug this requirement
   exists to prevent.
5. **Every new path keeps the reparse invariant.** An undo of any of the five must go
   through the same `retain_rope_edit` + `invalidate_highlight_for_key` path as the forward
   edit, so the retained tree cannot drift from the rope. A rope-only undo must fail a test.
6. **Bounds and the read-only no-op still hold** for the new paths: undoing in a read-only
   buffer is a no-op with a message, and a long sequence of the new edits stays inside the
   history cap (01's cap must not be bypassed by a path that records more than one step).
7. **The staleness guard must not become a silent editing brake.** 01 added a guard that
   rejects a step whose recorded range/text no longer matches the rope ("undo history is
   stale — ignored"). With five more paths feeding the history, a path that records an
   inverse in the *wrong unit* (byte vs char — this project's most recurring defect class)
   would be **silently refused** rather than failing loudly. So for at least one new path,
   prove by mutation that a byte/char mix-up in that path's recording is caught — either by
   a failing test or by the guard's message — rather than producing a quietly-does-nothing
   undo. If you find any path where a wrong-unit inverse is silently absorbed, that is a
   finding.

## Acceptance

* A per-path verdict table (`RET`, `C-k`, `C-y`, `M-y`, `C-w`): undoable, and whether the
  recording code needed a change for it. Both answers are acceptable; an unsupported claim
  is not.
* Undo of each of the five restores the buffer **byte-identically**, asserted at **every**
  step of the sequence, not just the end state — and point is where the edit happened.
* At least one new path exercised with **multibyte** content before the edit point, asserting
  chars (not bytes) for both the restored text and the point.
* The `M-y` coalescing decision is implemented and pinned over a known sequence.
* The kill-ring statement in requirement 4 is implemented and asserted.
* `C-7` reaches undo, asserted at the app level; the three bindings' terminal coverage is
  stated in the commit.
* The reparse invariant holds for the new paths (a rope-only undo fails a test), and the
  read-only no-op and the cap still hold.
* `cargo test --workspace` + clippy `--workspace --all-targets -- -D warnings` clean;
  `tools/gate.sh full` green.

## Fence

The edit sites in `src/app/store/buffers.rs` (and `keys.rs` if a path is routed there), the
key tables for the `C-7` binding, and tests. **Do not touch** the recording hook's shape
(01 owns it — if a path genuinely needs a change there, that is a finding: report it before
doing it), and do not add redo or the general coalescing rule (04). Disclose anything else
with before/after.

## Seams the later issues own — leave them clean

* **03** owns the saved-state marker, `locally_modified` correctness, and history clearing on
  reload/kill. There are **three** rope-*assigning* sites, now named in their docs and
  reachable through the single helper `drop_undo_history(key)` (01 built it and left it
  unwired on purpose): `reload_in_place` (`index_wiring.rs`), `toggle_ro_accept`
  (`buffers.rs`), and `replace_buffer_text` (`buffers.rs`, the notes-buffer sync that 01's
  gate found the hard way — it is reachable from annotation save/delete/reanchor). Do not
  wire the helper in; leave the seam obvious.
* **04** owns redo and the self-insert-run coalescing rule. Without coalescing every
  keystroke is one undo step — expected at this stage, not a bug; say so in the commit.
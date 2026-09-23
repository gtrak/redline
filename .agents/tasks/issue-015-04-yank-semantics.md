# Task: 015-04 — yank semantics per mode

**Depends on 015-02** (the honest point + the mode flag).

**⚠ REFRESHED 2026-09-22 against the current tree — the baseline below was stale.**
`yank` is now `buffers.rs:1430` (not 557; the STATUS row says 1223 and is stale too).
**015-02 landed the honest point**, so `yank` now inserts at `point_char` — which means
**Accurate mode already has the behaviour it wants**, and this issue is really about the
**Annotation** mode only. The original text below claimed the yank inserted at the line
start and was wrong under BOTH modes; that is no longer true. Re-measure before believing
any position claim in this file.

### Original (stale) baseline, kept for the reasoning

Before 015-02, `yank` inserted `kill_ring.top()` at `current_point_byte()` — which was the
**line start**. That was neither of the two coherent behaviours: not the append-at-end
model that self-insert uses, and not emacs's insert-at-point. That is now fixed for
Accurate mode; the Annotation half remains.

The user's decision:

| Mode | `C-y` |
|---|---|
| **Annotation** | **append at the end** (matching self-insert — "try append") |
| **Accurate** | **insert at the point** (emacs) |

## Key decisions

- **The append path is `insert_text`** (which already appends at `rope.len_chars()`) — if
  the annotation-mode yank can simply route through it, do that rather than duplicating
  the append logic. Say which you did.
- **`editable` still gates modification** (unchanged): yank into a non-editable buffer
  does nothing but must still say so (today it messages). Read-only buffers must not
  silently discard the yank.
- **Edit hygiene is mandatory**, exactly as the other edit paths: `locally_modified`,
  `retain_rope_edit`, `invalidate_highlight_for_key`. Missing any is a P1 (the highlight
  cache and the watcher depend on them).
- **Undo (016-01/02) interacts with this, and one mistake silently breaks the coalescing.**
  016-02 made `M-y` **coalesce** with the preceding `C-y` into ONE undo step
  (`coalesce_yank_pop_with_preceding_yank`), pinned by six probes that assert undo-to-empty
  at every step. Two consequences:
  (i) the yank must record **exactly one** undo step. `insert_text` already records via the
  hook, so if you route the Annotation append through it, do **not** also call
  `retain_rope_edit` yourself — a double step breaks the merge and no existing test may
  catch it.
  (ii) 016-02's coalescing tests must be **re-run and still pass**. A lane that changes the
  yank position owns the drives that assert the yank's undo shape.
- **In Annotation mode set `yank_pos` to the PRE-insert length** (the append position), not
  to the post-insert end, or `M-y` will replace the wrong range.
- **Yank-pop (`M-y`) must keep working in both modes.** It re-replaces the previous yank,
  so it needs the *same* position the original yank used: `M-y` must replace the range that
  was **inserted** (`yank_pos`), not the current point — in Annotation mode the insertion is
  at the buffer end while the point is elsewhere, so a point-based pop would corrupt it.
  Check `yank_pos`/`yank_len`/`yank_ring_index` and state how you keep pop correct — this is
  the subtle part of the issue.

  **CORRECTED 2026-09-23 by the gate (P3-1).** This paragraph used to say "in Accurate mode
  the point moves with the inserted text, so `M-y` must replace at the yank's start, not at
  the moved point". **Measured: the point does NOT move.** The point is a stored `(line,col)`
  and `yank` never calls `land_point_at_char`, so after a `C-y` at `(1,0)` the point is still
  `(1,0)`. The implementation was correct either way — `yank_pos` is the inserted range and
  that is what must be replaced — but the rationale was false. In emacs `C-y` leaves point
  *after* the inserted text, so **redline not advancing the point is a real parity gap**, filed
  as `issue-yank-followups` along with the off-screen case in Annotation mode. Do not
  re-introduce a comment claiming the point moves.
- **Do not touch** `kill_region`/`copy_region` (they already push to the kill ring) or
  the kill ring itself.

## Files

`src/app/store/buffers.rs` (`yank` + its pop path), tests (`tests/buffers.rs`).

## Verification

- `cargo build`; `cargo test --workspace` — reconcile against the current baseline
  (measure it; 02/03 will have moved it) and account for every change.
- `cargo clippy --workspace --all-targets -- -D warnings` (read `${PIPESTATUS[0]}`).
- **Tests to add** (discriminating — the old behaviour inserted at the line start, so a
  fixture whose point is mid-line and whose buffer does not end at that line will fail on
  the old code):
  (a) Accurate mode: yank at a mid-line point inserts **at the point**;
  (b) Annotation mode: yank **appends at the end**;
  (c) `M-y` after a yank replaces the inserted text correctly **in both modes**;
  (d) a non-editable buffer: yank is refused with a message and the buffer is unchanged;
  (e) a multibyte case for the position arithmetic;
  (f) **016-02's coalescing probes still pass** with the Annotation-mode append (the
  yank-and-rotate sequence still collapses to one undo step).
- **`timeout 900 tools/gate.sh full`** — the cursor is PTY-visible.
- Report: the routing (reused `insert_text` or a new path), how `M-y` stays correct per
  mode, the edit-hygiene calls, the tests with why each discriminates, and the gate
  output.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.

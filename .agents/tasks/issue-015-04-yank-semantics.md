# Task: 015-04 — yank semantics per mode

**Depends on 015-02** (the honest point + the mode flag).

Today `yank` (`buffers.rs:557`) inserts `kill_ring.top()` at
`current_point_byte()` — which, before 02, was the **line start**. That is neither of
the two coherent behaviours: not the append-at-end model that self-insert uses, and not
emacs's insert-at-point. It is a leftover, and it is wrong under **both** modes.

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
- **Yank-pop (`M-y`) must keep working in both modes.** It re-replaces the previous yank,
  so it needs the *same* position the original yank used: in Accurate mode the point
  moves with the inserted text, so `M-y` must replace at the yank's start, not at the
  moved point. Check `yank_pos`/`yank_len`/`yank_ring_index` and state how you keep
  pop correct — this is the subtle part of the issue. In annotation mode the append
  position is the end, so pop replaces the tail.
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
  (e) a multibyte case for the position arithmetic.
- **`timeout 900 tools/gate.sh full`** — the cursor is PTY-visible.
- Report: the routing (reused `insert_text` or a new path), how `M-y` stays correct per
  mode, the edit-hygiene calls, the tests with why each discriminates, and the gate
  output.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.

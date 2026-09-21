# Task: 015-02 — the honest point + the per-buffer edit mode

Plan 015's keystone. Everything in 03/04 depends on this; it is also the highest
blast-radius change in the plan, so it lands as **its own commit**.

## The lie to remove

```rust
// src/app/store/buffers.rs:362
/// ... The region mark is a line-start byte offset, so the point's column
/// does not extend the region (plan 004 issue 05b: region semantics unchanged).
fn current_point_byte(&self) -> Option<usize> {
    buf.rope.try_line_to_byte(self.point_line())      // ← returns the LINE START
}
```

It reports the point's line start as if it were the point. Everything coarse follows
from it: the region is line-granular (and a same-line mark+point yields `None`),
`C-y` lands at the line start (wrong under **both** modes), and the recent
`C-x C-x`/`C-x C-x` column work is invisible.

**Make it honest:** the point's byte = the byte of `(line, point_col())`. `point_col()`
is a **char** index (`file_view.rs:548`), so convert explicitly, e.g.
`let char_idx = rope.line_to_char(line) + col; rope.char_to_byte(char_idx)` — clamped to
the line/buffer length, and non-panicking on an out-of-range line (the current fn
already clamps with `line.min(line_count - 1)`).

## The mode: do NOT add a second source of truth for editability

`editable` already means **"this buffer's text may be modified"**, and it is already
per-buffer:
- file buffers open **`editable = false`** (read-only),
- the notes buffer is created **`editable = true`** (`open_notes`, `notes.rs` — *"Editable
  + locally-owned: the user types notes here"*),
- `C-x C-q` (`toggle_read_only`) already toggles it per buffer, with a scratch guard and
  the locally-modified/owned handling.

So the opt-in gesture the user asked for **already exists**. What is missing is that
the *behaviour* does not vary by mode: with `editable = true` the editing is still
coarse (append-at-end / backspace-at-end).

**Add a per-buffer edit mode** (`Annotation` default, `Accurate` opt-in) that selects
*how* modification behaves, and state the invariant:

> **`Accurate` ⟹ `editable`.** `editable` remains the gate for whether text may be
> modified at all; the mode selects the *shape* of that modification. `Annotation` does
> **not** imply read-only (the notes buffer is `editable` + `Annotation` — that is how
> annotations are typed).

`C-x C-q` becomes the mode toggle:
- entering `Accurate` → `mode = Accurate`, `editable = true`;
- leaving `Accurate` → `mode = Annotation`, and `editable` returns to the buffer's
  baseline. **Decide and state how you express that baseline** — either a per-buffer
  `kind`/`is_notes` predicate (cleaner, and useful later) or remembering the prior
  `editable` value (smaller). Say which and why. A buffer that is inherently editable
  (the notes document) must still accept typed annotations in `Annotation` mode.

**Show the mode in the status line** — the user must be able to see which mode a buffer
is in, or the toggle is invisible. One short indicator is enough.

## What is explicitly NOT in this issue

- The accurate-mode *commands* (insert at point, backspace-before-point, RET, `C-k`) —
  that is 03.
- Yank semantics — that is 04.
- **The region is NOT rounded to lines in annotation mode.** The user's call: *"a fine
  mark can still make sense"*. So the honest point makes the region **exact in both
  modes** — that is the intended change, not a regression. Note it in the report, since
  the current line-granular region is pinned by tests (see below).

## Blast radius — walk every consumer

`current_point_byte()` feeds: `region_byte_range`, `region_size_bytes`,
`region_line_range`, `set_mark`, `kill_region`, `copy_region`, `yank`,
`exchange_point_and_mark`. For each, decide whether the honest point is correct as-is
(it usually is) and report the decision. **`region_line_range`** is the one that needs
thought: it feeds the file view's region *face* rendering, which is line-oriented — an
exact char range still renders as whole lines there, which is fine, but confirm the
range conversion is still right (it converts bytes → lines).

## Key decisions

- **Pure honesty change + the mode flag.** No other behaviour changes in this commit.
- **Test authority applies.** Existing tests pin the line-granular region and the
  line-start point. Those are **implementation-level** pins of the behaviour being
  deliberately changed: update them and say so. But any test pinning *requirement-level*
  behaviour (e.g. "cancel restores the pre-search position", "the mark is set") must
  keep passing unchanged.
- **Do not break the notes document** or the annotation records.

## Files

`src/app/store/buffers.rs` (the honest point + region/kill/copy/yank consumers),
`src/app/store/mod.rs` (the mode enum/field + `Buffer`), `src/app/store/file_view.rs`
(`point_col`/`set_point` if needed), `src/app/store/keys.rs` (the `C-x C-q` path if it
routes through the dispatch), the status-line construction (`src/ui/root/*` or the
snapshot), and tests (`tests/buffers.rs`, `tests/views.rs`, `tests/file_view.rs`).

## Verification

- `cargo build`; `cargo test --workspace` — reconcile against the **current** baseline
  (**713** redline + **154** `redline-syntax` = 867, 2 ignored; resolver 123/0/4;
  integration 7,2,3,1,1; doctests 0 — measure it) and account for every change.
- `cargo clippy --workspace --all-targets -- -D warnings` (read `${PIPESTATUS[0]}`).
- **Tests to add** (each must DISCRIMINATE — a col-0 fixture cannot):
  (a) `current_point_byte()` on a mid-line point returns the mid-line byte, not the line
      start (and a multibyte case where byte ≠ char);
  (b) the region with mark and point on the **same line** is now **non-empty** and exact
      (under the old behaviour it was `None` — so this pin fails on the old code);
  (c) `kill_region`/`copy_region` copy the exact text (not whole lines);
  (d) `C-x C-q` enters/exits the mode and the notes buffer still accepts typed text in
      `Annotation` mode;
  (e) the status line shows the mode.
- **`timeout 900 tools/gate.sh full`** — the cursor is PTY-visible and the region face is
  rendered, so the battery is the real check. If swap blocks it, run the workspace suite
  + both windowing drives + `check_cursor_stream.py` and report it DEFERRED.
- Report: the honest-point implementation, the baseline mechanism you chose for leaving
  `Accurate`, every consumer's decision, the re-pinned tests (with the old assertion
  quoted), the new tests with why each discriminates, and the gate output.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.

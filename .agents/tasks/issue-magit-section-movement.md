# Task: magit-consistent section movement (`n`/`p` boundary behavior)

## Authority (do not guess this — it is settled)
**Real magit 4.7.1 does not wrap.** Source of truth: `lisp/magit-section.el:805-831`
(magit repo, commit `5059906`):
- `magit-section-forward` (`n`): at `eobp`, and also when no next sibling/parent-sibling
  exists, it calls `(user-error "No next section")` — it **stays put and reports**.
- `magit-section-backward` (`p`): at `bobp` it calls `(user-error "No previous section")`.
So the boundary behavior is symmetric: **no wrap + an echo-area message**.

## Current redline behavior (wrong in both halves)
- `StatusTree::move_down` (`src/model/sections.rs`) **wraps to the first section**.
- `StatusTree::move_up` does not wrap (silently stays).
- The `move_down` doc comment says "wraps nowhere; a no-op at the last section" — the
  doc contradicts the code, and **nothing pins either behavior**.

## Required change
1. `move_down`: at the last visible section the cursor **stays unchanged** (no wrap).
2. `move_up`: at the first visible section the cursor **stays unchanged** (unchanged from
   today, but now pinned).
3. At either boundary, report the magit-consistent message through the minibuffer
   (redline's echo area): exactly `No next section` / `No previous section`. Emit it from
   the store's magit cursor handlers (the `StatusTree` should not need to know about the
   minibuffer — returning "did it move?" is fine; choose the cleanest seam and say which).
4. Make the doc comment truthful.

## Do NOT change the traversal
Magit's `n`/`p` move to the next/previous **visible section** in document order (descending
into children, skipping diff/body lines). Redline's cursor addresses **section ids only**
(`visible_ids()`), while `visible_rows()` also emits non-selectable body lines — so
redline's document order already matches magit's. Change **only** the boundary behavior.

## Pins (required; nothing covers this today)
Unit tests, in the module that owns the behavior:
- `move_down` at the last visible section → cursor unchanged **and** the message is set.
- `move_up` at the first visible section → cursor unchanged **and** the message is set.
- Normal `n`/`p` mid-list still move (regression guard).
- The `cursor == None` path still behaves sensibly (state what it does).
If an existing test asserts the wrap, that assertion is **implementation-level** and must be
updated to the magit-consistent behavior (greenfield test-authority policy) — report it.

## Fence and gate
Fence: `src/model/sections.rs`, `src/app/store.rs`, tests in those modules. Nothing else.
Gate: `cargo build` first, then `cargo test --workspace`, `cargo clippy --workspace --all-targets`
(read `${PIPESTATUS[0]}`), and `tools/gate.sh full`.
Budget ~30 tool calls; honest-stop at half. Report: the exact message strings, the tests added,
any pre-existing test that pinned the wrap, gate counts, deviations.

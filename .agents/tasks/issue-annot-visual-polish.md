# issue-annot-visual-polish — the gate's three P3s, plus dependency batch B

Two small, independent pieces of maintenance. **Two separate commits** (one per concern) so the
stories stay readable.

## Part 1 — the three P3 findings from the `annot-visual` gate

The gate verdict is in `/tmp/gr_visual_verdict.md`; the landing commit is `7e97812`.

1. **⚠ Commit the missing pin for the annotated-line click COLUMN mapping.** The gate verified the
   mapping is correct by *writing its own test in a scratch clone*, because the repo has none: only
   **col-0** clicks on annotated rows are covered. Since the gutter widened from 1 to 2 cells, the
   click→column inverse (`col.saturating_sub(2)`, in `src/app/store/file_view.rs`'s click handling,
   mirroring `src/ui/root/geometry.rs::cursor_cell`) is load-bearing and currently unpinned — a
   future change could break clicking on annotated lines **silently**. Re-derive and commit the
   pin: on an annotated line, cell 2 → char 0, cell 3 → char 1, gutter cells (0 and 1) → char 0;
   on an unannotated line, cell 0 → char 0. Confirm the test **fails** if the offset is wrong
   (mutate `saturating_sub(2)` → `saturating_sub(1)` and show it RED) — an assertion that passes
   under both is not a pin.
2. **Fix the stale comment** at `src/app/keymap.rs:726`: it still says *"Net +3 stands."* while its
   neighbour was updated to *"net +2 → 125"*. Make the two agree with the actual count (and if the
   count in the comment is derivable, prefer stating it so it cannot drift silently).
3. **Add `no-jitter=` to `ann-fold`'s recorded detail** in `tools/sweep_flows.py` (the flow folds
   the property into `ok`, but the recorded detail omits it — which is exactly what made
   diagnosing a glyph revert take an extra step during the gate). Keep the flow's assertion
   strength unchanged; this is diagnosability only.

## Part 2 — dependency batch B: `unicode-width` 0.1.14 → 0.2.2

The sweep's **one genuinely actionable bump**. Context, so you do not re-litigate the others:
`tree-sitter` / `tree-sitter-highlight` (`=0.25.10`) and `tree-sitter-md` (`=0.5.1`) are **held on
purpose** — `docs/tree-sitter-runtime-matrix.md` records the verdict that the grammars wanting
`^0.26` "remain outside the ABI window and are NOT bumped", and the manifest comment says 0.5.2+
want `^0.26`. `tokio` is already at `1.53.1` in the lock. Batch A (8 transitive bumps) landed in
`d665ce3`. **Do not touch anything but `unicode-width`.**

**Why this one needs behavioural verification, not just a compile:** the API surface we use is
tiny — a single call site, `src/model/text_width.rs:11-12` (`UnicodeWidthChar::width()` — confirm
that is still the only one) — but 0.2 **changes the width TABLES**, i.e. what counts as
double-width, and we have a wide-char issue in the tree. So:

1. **Record the before-state first**: with the current pin, run the width-related tests and note
   the results, and dump the computed width of a **fixed sample** of characters — at least one
   unambiguous narrow, one unambiguous wide (CJK), one emoji, one combining mark, and a couple of
   **ambiguous-width** characters (the ones whose East Asian Width class is Ambiguous, which is
   where tables tend to differ).
2. Bump to `0.2.2` (manifest + lock) and fix the call site if the API changed.
3. **Compare the same sample after** and report a **before/after table**. Any width that changes
   must be explained, not just observed: say whether the new value is *more correct* (and why) or
   a regression, and whether it affects cursor placement, truncation, or the gutter math. If a
   character's width changes and that changes layout, say so plainly.
4. **Run the wide-char / multibyte tests** (including the existing wide-char issue's tests if any)
   and the full workspace battery. If a test now fails because its expectation encoded the *old*
   table, that is a decision to make explicitly — state whether you updated the expectation
   because the new width is correct, or reverted the bump.

## Acceptance

* The click-column pin exists, is committed, and **discriminates** (mutation shown).
* The stale comment agrees with reality; `ann-fold` records `no-jitter=`.
* A before/after width table for the character sample, with any change explained.
* `cargo test --workspace` + clippy `--workspace --all-targets -- -D warnings` clean;
  `tools/gate.sh full` green.
* Two commits, with the reasoning in each message.

## Fence

`src/app/store/file_view.rs` + `src/ui/root/geometry.rs` (the pin), `src/app/keymap.rs` (the
comment), `tools/sweep_flows.py` (the detail), `Cargo.toml`/`Cargo.lock` and
`src/model/text_width.rs` (batch B). Disclose anything else with before/after.
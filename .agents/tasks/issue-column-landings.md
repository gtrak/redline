# Task: the remaining column landings — C-x C-x, isearch cancel, search RET, unique-def M-.

The user-reported isearch bug (`afa3702`) was one instance of a class. The audit that
lane ran found **four more sites that drop a column that exists**, plus two wrong
docs. The `try_byte_to_line_col` seam is now in place, so each of these is small.

## The sites (verified by the isearch gate — re-derive before relying on them)

| # | Site | The column that exists | Conversion needed |
|---|---|---|---|
| 1 | **`buffers.rs:458` `exchange_point_and_mark` (`C-x C-x`)** | `mark` is a **byte** offset (`buf.mark`, set from `current_point_byte()`); the code takes only `try_byte_to_line(mark)` and lands col 0 | `try_byte_to_line_col(mark)` |
| 2 | **`search.rs:166` isearch cancel (`C-g`)** | *nothing is recorded* — `IsearchState` stores only `pre_search_line` (set in `isearch_start`). Cancelling therefore lands col 0 instead of restoring the original column | record a **`pre_search_col`** (a **char** col, from `point_col()`) at `isearch_start` and land with it |
| 3 | **`search.rs:339` project-search RET** | `Hit.col: Option<u64>` — a **byte** column, `None` for regex hits | byte→char (within the line); `None` ⇒ col 0 |
| 4 | **`definitions.rs:186` unique-def `M-.` jump** | `Symbol.start_byte` — an absolute **byte** offset | `try_byte_to_line_col(start_byte)` |

Plus two **doc** fixes:
- **`set_point_line`'s doc** (`file_view.rs:591-602`) — it folds the *direct* unique-def
  `M-.` jump (`definitions.rs:186`) into the pickers' "no column" justification, though a
  column is derivable there, and omits it from the follow-up list. Fix while updating the
  caller list for sites 1–4.
- **`JumpEntry.col`'s doc** (`store/mod.rs:616`) — says "byte offset within the line";
  it is actually a **char index** (recorded from `point_col()`, consumed by `set_point`).

**Priority: `C-x C-x` first** — it is the most emacs-visible of the four, and the new
helper is exactly the conversion it needs. If you must stop early, land it alone.

## Key decisions

- **BYTE vs CHAR, again — per site.** Sites 1, 3, 4 are **byte** offsets and need
  `try_byte_to_line_col` (or an equivalent explicit conversion). Site 2 needs a
  **char** col recorded from `point_col()` — do **not** store a byte there and then
  convert, because `point_col()` already yields a char index and `set_point` consumes
  one. Getting a site's unit wrong is a silent off-by-N on multibyte lines.
- **`goal_col`**: follow the established convention — a landing that knows its column
  sets `goal_col = col` (so a following `C-n`/`C-p` holds it). Site 3's `None` (regex)
  keeps col 0 / goal 0.
- **`C-x C-x` semantics**: emacs `exchange-point-and-mark` swaps point and mark. Verify
  the current function's behaviour before changing it — if it also *moves the mark* to
  the old point, keep that; this task only adds the column to the point's landing.
- **Each fix needs a DISCRIMINATING test**: a fixture whose target is at a **nonzero
  column**, asserting the **column** (not just the line), plus a **multibyte** case
  where the byte and char columns differ. The isearch lane's lesson: a col-0 fixture
  cannot tell the bug from the fix. The gate proved the isearch pins discriminate by
  running them against the old code in a scratch worktree — do the equivalent
  reasoning for each new pin (state why it would fail under the old behaviour).
- **Do not touch `set_point_line`'s behaviour** — it remains the col-0 landing for the
  callers whose target genuinely has no column (`goto-line`, pickers, resolver
  landings, external xref).
- **Optional, folded in (routing change)**: the isearch lane's `#[allow(dead_code)]` on
  `try_byte_to_line` (`model/buffer.rs:185`) suppresses a *standing* warning — the item
  now has zero production callers because `try_byte_to_line_col` reaches into `self.rope`
  directly, and the stated reason ("public API: spec-required") is inaccurate. The
  gate's minimal fix: have `try_byte_to_line_col` call
  `self.try_byte_to_line(byte_idx)?` — one line, keeps the item live and its pinned
  test, drops the allow. Do it if budget allows (it was routed to the hygiene sweep;
  this file is already in scope, so doing it here is cheaper — say so in the report).

## Files

`src/app/store/buffers.rs`, `src/app/store/search.rs`,
`src/app/store/navigation/definitions.rs`, `src/app/store/file_view.rs` (doc),
`src/app/store/mod.rs` (the `JumpEntry.col` doc), `src/model/buffer.rs` (optional),
and the test files: `src/app/store/tests/buffers.rs`, `.../search.rs`,
`.../navigation/*`.

## Verification

- `cargo build`; `cargo test --workspace` — reconcile against the **current** baseline,
  which is **859** passed / 0 failed / 2 ignored redline (the isearch lane added 4 to
  the old 855 — measure it yourself, do not copy my number), resolver 123/0/4,
  integration 7,2,3,1,1, doctests 0; `cargo clippy --workspace --all-targets` (read
  `${PIPESTATUS[0]}`); **`timeout 900 tools/gate.sh full`** — these are cursor landings,
  so the PTY battery matters. If it fails for swap, run the workspace suite + both
  windowing drives + `check_cursor_stream.py` and report the battery DEFERRED.
- Report per site: the unit you found (byte or char) and how you verified it, the
  before/after landing, the test you added and **why it discriminates**, the two doc
  fixes, and whether you did the optional allow fix.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.

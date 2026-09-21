# Task: landings must honour a column — isearch (user-reported) and jump history

**User-reported (hands-on UX):** *"I-search will move the cursor to the line but not
the actual match."*

Confirmed, and the same defect affects jump history. Root cause is one line:

```rust
// src/app/store/file_view.rs:595
/// A landing that moves the point to buffer line `line` at column 0
/// (isearch, goto-line, xref, imenu, jump, search-RET, click):
/// `set_point` with the point's column reset (the emacs landing is at the
/// start of the target line).
pub(super) fn set_point_line(&mut self, line: usize) {
    self.set_point(line, 0, 0);
}
```

The blanket rationale ("the emacs landing is at the start of the target line") is
**false for isearch and for jump history** — emacs lands point on the match/saved
column. The column is available at both call sites and is thrown away:

| Caller | Evidence |
|---|---|
| **isearch** — `search.rs:129` | `isearch_jump_to_current` computes `match_byte` from `self.isearch.matches` (byte offsets), converts it to a **line** (`try_byte_to_line`) and calls `set_point_line(line)` — the byte's column within the line is discarded. |
| **jump history** — `navigation/mod.rs:73` | `current_jump_entry()` **records** `col: self.point_col()` (`:89`), and `jump_back`'s own doc says *"`M-,`: pop back to the prior position (**line + column**)"* — but `navigate_to_entry` lands with `set_point_line(line)`. The stored column is never restored. |

The renderer already uses the point's column, so this is user-visible:
`ui/root/geometry.rs::cursor_cell`'s `Buffer` branch builds the cursor's display
column from `snap.file_view_point_col` (char-index → display-column). No hscroll
exists, so a match beyond the pane width will clamp at the edge — **that is a
separate known limitation, not this bug; do not fix it here.**

## What to build

1. **isearch lands on the match.** In `isearch_jump_to_current`, derive the match's
   **column** as well as its line and call `set_point(line, col, goal_col)`.
2. **jump history lands on the recorded column.** `navigate_to_entry` must restore
   `entry.col` instead of zeroing it. (This makes the existing `jump_back` doc true.)
3. **Correct `set_point_line`'s doc** — it is now the *col-0* landing, used by the
   paths where col 0 is genuinely right (e.g. `goto-line`, which emacs lands at the
   line start). Remove isearch/jump from its list and state why each remaining caller
   keeps col 0.
4. **Report, do not change, the other callers.** Audit every `set_point_line` caller
   (`xref.rs:52`, `definitions.rs:186`, `search.rs:329`, `buffers.rs:458`,
   `file_view.rs:969`, `picker.rs:721/731/746`) and say for each whether a column is
   *available* at that site. If one is available and dropped, list it as a follow-up
   — **do not expand this lane's scope**, and do not touch `goto-line`.

## Key decisions

- **BYTE vs CHAR INDEX is the trap.** `isearch.matches` are **byte** offsets;
  `JumpEntry.col`'s doc says *"byte offset within the line"*; but `set_point`'s `col`
  parameter is a **char index** (see the click path's comment in `file_view.rs`).
  Determine what each source actually holds (read `point_col()` and `set_point`'s
  consumers — do not trust the doc) and convert explicitly. A multibyte bug here
  would be silent and off-by-N.
- **`goal_col`**: after a landing, emacs keeps the landing column as the goal column
  (so a following `C-n`/`C-p` holds that column). Set `goal_col = col` for both
  landings and say so — unless the codebase's convention says otherwise.
- **The existing test cannot discriminate.** `isearch_lands_point_on_match`
  (`store/tests/search.rs:71`) asserts only `point_line() == 4`, and its fixture's
  match (`omega`) is at **column 0** — which is exactly why the bug survived. **Add a
  test whose match is at a NONZERO column** (e.g. `"xx omega"` → col 3) and assert the
  point's column, not just the line. Same for the jump-history test: record a point
  at a nonzero column, jump away, `M-,` back, and assert the column is restored.
  Include a **multibyte** case (there is already `isearch_multibyte_no_panic` to
  extend) so the byte→char conversion is pinned.
- **Pure, minimal change.** No refactor of the landing machinery beyond what the two
  fixes need; if a shared helper falls out naturally (e.g. `land_on(line, col)`),
  fine — but the existing `set_point_line` must keep working for its remaining callers.

## Files

`src/app/store/search.rs`, `src/app/store/navigation/mod.rs`,
`src/app/store/file_view.rs`, `src/model/buffer.rs` (if a byte→char/line→char helper
is needed — the wrapper currently exposes only `byte_to_line`/`try_byte_to_line`;
`ropey` has `byte_to_char`/`line_to_char`), `src/app/store/tests/{search,views}.rs`.

**Fence note:** another lane (`a3-modal-chain`) is editing `src/app/store/keys.rs` in
slot 1. Do not touch `keys.rs`.

## Verification

- `cargo build`; `cargo clippy --workspace --all-targets` (read `${PIPESTATUS[0]}`).
- **Targeted tests only** while the other lane is in flight (to avoid two concurrent
  heavy suites — the documented flake class): `cargo test --bin redline
  app::store::tests::search app::store::tests::views app::store::tests::navigation`
  plus any test module you touched. **Defer the full `cargo test --workspace` and the
  PTY battery to the serial pass**, and say so — this is the session's established
  deferral pattern, not a skipped check.
- Report: the exact conversion you used for byte→char (with the multibyte test that
  pins it), the two landings' before/after behaviour, the `set_point_line` doc
  correction, the per-caller column-availability audit, and the tests you added with
  their assertions quoted.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.

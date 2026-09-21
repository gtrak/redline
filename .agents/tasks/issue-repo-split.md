# Task: split `git/repo.rs` (plan 012 A6, mechanics corrected)

## Context
`src/git/repo.rs` is 1,713 lines: a `GitRepo` struct + a ~641-line `impl`, free helpers
(pure byte-level hunk math and index-entry plumbing), and an ~805-line test module. It
mixes three concerns: **status/diff queries**, **index mutations** (stage/unstage/
discard), and **pure hunk math**.

`02-language-descriptor-design.md` §A6 claims the impl "stays whole: Rust does not allow
splitting one impl across files without traits." **That claim is wrong** — verified with
`rustc` during the store split: an `impl GitRepo` block placed in a **child module** of
the module that defines `GitRepo` accesses its **private fields with no visibility
change** (privacy exposes a module's private items to its descendants). A private
*method* called from a **sibling** submodule does need `pub(super)`
(`E0624`). So the impl can be split the same way `AppStore` was.

## Target layout
`src/git/repo/{mod.rs, index_ops.rs, hunk.rs}` (adjust if reading the file suggests
better boundaries, and say so):
- **`mod.rs`** — the `GitRepo` struct, `discover`, and the query surface
  (`branch`, `status`, `diff`, …), plus `pub use` re-exports so every existing
  `crate::git::repo::X` path keeps working.
- **`index_ops.rs`** — a second `impl GitRepo` block with the mutations
  (`stage_file`, `unstage_*`, `stage_hunk`, `discard_*`, `apply_hunk_to_index`,
  `reset_index_entry_to_head`, `remove_index_entry`, and the private index helpers
  `head_tree`/`raw_diff` if they belong here).
- **`hunk.rs`** — the free pure byte math (`revert_hunk_in_content`,
  `reverse_apply_hunk_in_content`, `split_lines_inclusive`, `ensure_utf8`) and the
  index-entry plumbing (`copy_index_entry`, `find_path_in_tree`, `default_entry`).
Confirm boundaries by reading the file — do not trust remembered line numbers.

## Rules
- **Pure move, behavior-preserving.** No logic edits, renames, reordering, or
  "while I'm here". Note that a previous lane already deduplicated the blob-newline
  twins and the 4× hunk lookup into `blob_side_ends_with_newline`/`find_hunk_in` — keep
  those as they are.
- **`pub(super)` only where the compiler demands it** (cross-sibling private calls);
  report the count. Do not add `pub`/`pub(crate)`.
- **Prove the move faithful**: extract the pre-move file and check every item body
  appears **exactly once** across the new files with only visibility/`use` deltas
  (a comment-stripped multiset partition is the shape the other gates used; state your
  method and report the item counts).
- **Tests**: the test module must stay a **descendant** of the module defining `GitRepo`
  (it reads private state). You may move it into `repo/tests.rs` or keep it in `mod.rs`
  — your call, but say which and why, and **no test may be lost, renamed, or weakened**.
- Fence: `src/git/repo.rs` → `src/git/repo/*`, plus `src/git/mod.rs` if the module
  declaration needs it. **Nothing else** (another lane owns `src/ui/root.rs` and
  `tools/check_cursor_stream.py`).
- Honest-stop at half budget; a partial split that is green is a valid landing.

## Gate
`cargo build` first, then `cargo test --workspace`, `cargo clippy --workspace
--all-targets` (read `${PIPESTATUS[0]}`), and `timeout 900 tools/gate.sh full`.
**Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` **and swap** before the
battery — this box has been swap-exhausted, which correlates with a known `git::repo`
test flake. If memory is tight, run `cargo test --workspace` only and report the battery
as deferred (do not sleep-wait). If a gate run fails, re-run once, capture the full log,
and **name the flaking suite**.
Budget ~40 tool calls. Report: layout + line counts, faithfulness method + item counts,
the `pub(super)` count, where the tests went, gate counts.

# Task: split the store test module (plan 012 phase 3)

## Why
`src/app/store/mod.rs` is 13,701 lines and **~82% of it is the `#[cfg(test)] mod tests`**
(lines ~2432–13693). It is now the largest monolith in the repo, and it is the reason
`mod.rs` still looks huge. `docs/architecture.md` §3 is the map.

## Target
Split the test module into per-concern test files under `src/app/store/tests/`:
`src/app/store/tests/{mod.rs, views.rs, buffers.rs, notes.rs, file_view.rs, search.rs,
magit.rs, commit.rs, navigation.rs, index_wiring.rs, picker.rs, project.rs,
minibuffer.rs, keys.rs, core.rs}` — mirroring the concern modules the production code
already uses, with `mod.rs` holding the **shared fixtures/helpers** and the `mod`
declarations.

**Critical mechanic**: tests must remain **descendants of the module defining
`AppStore`** (they read its private fields today), so `tests` is a child of `store` and
each test file is a child of `tests` — that keeps private access with no visibility
change. If a test needs a *sibling* module's private item, that is a signal the test
belongs elsewhere or the item needs `pub(super)` — report rather than widen silently.
Use `#[path]` wiring only if the layout requires it (and remember `#[path]` resolves
relative to the **containing file's directory**).

## Hard constraints
1. **Pure move.** No test may be lost, renamed, weakened, or reordered-within-a-file.
   No assertion edits. No "while I'm here" cleanup — in particular **do not** yet
   deduplicate the 9 nested `git_cli` helpers or the repeated fixtures; that is a
   separate follow-up (mixing a move with a dedup makes the move unverifiable).
2. **Carry the nested fns.** The census is **379 top-level fns + 9 `fn git_cli` variants
   nested inside test bodies + 2 nested `main`s**. A naive top-level-only move loses the
   11 nested ones. Verify the counts before and after.
3. **Shared fixtures stay shared**: whatever the tests currently share (e.g. `store()`,
   `render80`, `flat`, `rows_containing`, the project/tempdir builders, `notes_store`)
   must live in `tests/mod.rs` and be imported — not copy-pasted into 15 files. If a
   fixture is currently duplicated *within* the test module, leave it duplicated (pure
   move) and note it for the dedup follow-up.
4. **Prove the move faithful**: extract the pre-move test module
   (`git show <base>:src/app/store/mod.rs`) and check that every test fn body appears
   **exactly once** across the new files, with the only deltas being `use` lines and
   `pub(super)`/visibility keywords where the compiler demands them. State your method
   (a comment-stripped multiset partition is the shape the other gates used) and report
   the counts: top-level, nested, and total.
5. Honest-stop at half budget: a subset of concern test files that is green is a valid
   landing — but the tree must always be consistent (no test in two places, no test
   lost). If you stop mid-way, say exactly which concerns are done.

## Fence
`src/app/store/mod.rs` + new `src/app/store/tests/*.rs`. **Nothing else.** Other lanes
own `src/syntax/*` (node split) and `app/command.rs`/`ui/*`/`git/repo.rs`/
`model/sections.rs`/`app/keymap.rs` (mechanical sweep) and the deflake lane
(`tools/check_cursor_stream.py` + `src/git/repo.rs` tests).

## Gate
`cargo build` after each batch; `cargo test --workspace`, `cargo clippy --workspace
--all-targets` (read `${PIPESTATUS[0]}`), and `timeout 900 tools/gate.sh full` at least
once mid-way and at the end.
**Known flake**: a gate run can FAIL on a frozen tree and pass on re-run (load-correlated,
see `issue-deflake-timing.md`). If that happens, re-run once and capture the full log,
and name the flaking suite — that is a finding.
**Resource guard**: `export CARGO_BUILD_JOBS=4`; if `free -g` "available" < 8 GB run
`cargo test --workspace` only and report the full gate as deferred (do not sleep-wait).
Budget ~60 tool calls. Report: the layout, the faithfulness method + counts, which
concerns landed, gate counts, and anything you had to escalate rather than widen.

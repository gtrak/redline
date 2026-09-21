# Task: split `syntax/node.rs` (plan 012 A2, revised post-descriptor-table)

## Context
`syntax/node.rs` is 2,038 lines. The descriptor table (landed, `6229039`) **removed
the 15 `is_*_identifier_kind` predicates** from it — identifier kinds are now data on
the table — so the file's remaining content is smaller and differently shaped than
`02-language-descriptor-design.md` §A2 assumed. **Re-read the file first** and derive
the real split from what is actually there.

## Target layout
`src/syntax/node/{mod.rs, paths.rs, scopes.rs}`:
- **`paths.rs`** — `is_path_segment` (the per-language structural predicate, ~156
  lines of match arms) and its tests. This is an *algorithm*, not data, which is why
  the descriptor table deliberately kept it.
- **`scopes.rs`** — the per-language `*_scope_path` walkers + the `scope_path_for`
  dispatcher, and their tests.
- **`mod.rs`** — the entry points and everything not moved: `NodeInfo`, `node_at`,
  `scope_path_at`, `parse_source`, `innermost_at`, `nearest_identifier`,
  `in_identifier_position`, plus `pub use` re-exports so every existing
  `crate::syntax::node::X` path keeps working.
Confirm the exact boundaries by reading the file. If some other coherent group exists
that the design did not anticipate, you may take it — but say so in the report.

## Rules
- **Pure move, behavior-preserving.** No logic edits, no renames, no reordering of
  arms, no "while I'm here". Keep visibility exactly as-is (`pub(crate)` stays
  `pub(crate)`; a moved private fn called from a *sibling* submodule needs
  `pub(super)` — add it only where the compiler demands it, and report the count).
- **Prove the move faithful**: extract the pre-move file (`git show <base>:src/syntax/node.rs`)
  and check that every moved body appears **exactly once** across the new files, with
  the only delta being the intended visibility keywords. State your method (a
  comment-stripped multiset partition is the shape the other gates used).
- Tests move with their subjects; **no test may be lost, renamed, or weakened**.
- Fence: `src/syntax/node.rs` → `src/syntax/node/*`, plus `src/syntax/mod.rs` if the
  module declaration needs it. **Nothing else.**
- Honest-stop at half budget; a partial split that is green is a valid landing.

## Gate
`cargo build` first, then `cargo test --workspace`, `cargo clippy --workspace
--all-targets` (read `${PIPESTATUS[0]}`), and `timeout 900 tools/gate.sh full`.
**Known flake**: a `gate.sh full` run can report FAIL on a frozen tree and pass on an
identical re-run (seen once). If that happens, **re-run once and capture the full log**
so the flaking suite can be identified — that is a finding, not a nuisance.
**Resource guard**: `export CARGO_BUILD_JOBS=4`; if `free -g` "available" < 8 GB, run
`cargo test --workspace` only and report the full gate as deferred (do not sleep-wait).
Budget ~40 tool calls. Report: the resulting layout + line counts, the faithfulness
method + result, the `pub(super)` count, re-export handling, gate counts.

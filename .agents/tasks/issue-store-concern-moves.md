# Task: move `AppStore` concerns out of the impl (012-03+)

The module pattern is established and proven: `src/app/store/mod.rs` (the struct +
the impl) plus `helpers.rs` and `notes_doc.rs` (012-02, landed). This lane moves
**concern methods out of the single `impl AppStore`** into per-concern submodules.

## Preconditions and the map
`docs/architecture.md` §3 is the concern inventory: 421 methods, per-concern method
lists, and per-concern line runs. **Its line numbers were current at `850034d` and
will drift as you move code** — so for each concern, **re-derive its method list and
current line runs from the file** before moving it (grep the method names; do not
trust remembered numbers). The counts are stable: 421 methods total.

## The mechanics (proven; do not re-litigate)
- New files are **children** of the module that defines `AppStore` (`app::store::*`),
  so all 83 private fields stay reachable with **no field-visibility change**.
- A private method called from **another** concern needs `pub(super)` — add it **only
  where the compiler demands it**, and report the count per concern.
- Keep `pub use` re-exports for anything the rest of the crate imports, so external
  paths never change.
- **Do NOT move the test module** (that is a later lane). Note for that lane: the map
  records **9 helper fns nested inside test bodies** — they must not be lost.
- `point_byte_offset` stays in `mod.rs` for now (its 4 call sites are in-impl).

## Order (small first, to prove the pattern cheaply, then continue)
1. `views` (18) 2. `project` (17) 3. `magit` (24) 4. `minibuffer` (1) 5. `keys` (11)
6. `buffers` (29) 7. `notes` (30) 8. `search` (39) 9. `picker` (41) 10. `file_view` (52)
11. `index_wiring` (34) 12. `navigation` (51) 13. `commit` (56)
Stop wherever the budget runs out — **each concern is a valid landing** as long as the
tree is green and no method exists in two places.

## Per-concern procedure (each one a commit)
1. Re-derive the concern's method list + line runs from the current file.
2. Move the methods **verbatim** into `src/app/store/<concern>.rs` as an
   `impl AppStore { … }` block (keep doc comments and ordering).
3. `mod <concern>;` in `mod.rs`; add `pub use` if an external path needs it.
4. `pub(super)` only where the compiler demands it.
5. Prove the move is faithful: comment-strip the pre-move file and the new one and
   check that every moved line appears exactly once (the gate used a multiset
   partition + in-order subsequence; a simple equivalent is fine, but **state the
   method**). No logic edits, no renames, no reordering.
6. Commit; run `cargo test -p redline --lib` (fast) before moving on.

## Fence (strict)
`src/app/store/mod.rs` + new `src/app/store/*.rs`. **Nothing else.** Other lanes own
`src/syntax/*` (descriptor table) and `app/command.rs`/`ui/*`/`git/repo.rs`/
`model/sections.rs`/`app/keymap.rs` (mechanical sweep). If a move seems to need an
outside file, stop and report.

## Gate
`cargo build` after each concern; `cargo test --workspace`, `cargo clippy --workspace
--all-targets` (read `${PIPESTATUS[0]}`), and `timeout 900 tools/gate.sh full` at the
end (and at least once mid-way).
**Resource guard**: `export CARGO_BUILD_JOBS=4`; if `free -g` "available" < 8 GB, run
only the lib tests and report the full gate as deferred (do not sleep-wait).
Budget ~60 tool calls. Report per concern: methods moved, the faithfulness method +
result, the `pub(super)` count, gate counts, and exactly which concerns remain.

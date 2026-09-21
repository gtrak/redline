# 012 — Redline: project organization & cleanup

Status: planned
Phases: 4 · Issues: 01–09 (staged; each stage lands gate-green)
Depends on: nothing (behavior-preserving refactor + cleanup)

## Why

`src/app/store.rs` is **22,992 lines with 869 methods in a single
`impl AppStore` block** — roughly 40% of the 56 k-line codebase. It mixes
buffers, views, the view stack, file-view cursor/scroll, search, magit
status/staging, log/blame/commit/diff, notes, annotations, navigation
(M-., jump stack, xref/imenu/impls pickers), index wiring, crate indexes,
minibuffer, project/files/recents, keymap dispatch — plus a very large test
module. Consequences observed this session:
- every lane that touches anything in the app needs `store.rs`, so lanes
  cannot run in parallel (file-disjoint autonomy collapses);
- reviews are expensive (`file:line` citations drift constantly — e.g. 4 of
  6 doc citations in one review round pointed at the wrong lines);
- readers (human and agent) pay a full-file scan to find one concern.

Adjacent offenders: `src/app/flow_tests.rs` 3,624 · `src/syntax/node.rs`
2,251 · `src/ui/root.rs` 1,752 · `src/git/repo.rs` 1,713 ·
`src/syntax/queries.rs` 1,697.

## What

**Phase 1 — plan the split on facts (no behavior change).**
Inventory the 869 methods into coherent concerns with line ranges; publish
`docs/architecture.md` (module map + the split plan + the mechanical
constraints). Reviewer checks the inventory against the file.

**Phase 2 — split `store.rs`, one concern per stage.**
Target layout (names indicative; the inventory confirms):
`src/app/store/mod.rs` (the struct + state + shared helpers),
`buffers.rs`, `views.rs`, `file_view.rs`, `search.rs`, `magit.rs`,
`commit.rs` (log/blame/diff/editor), `notes.rs`, `navigation.rs`
(M-./jump/xref/imenu/impls), `index_wiring.rs`, `picker.rs`,
`minibuffer.rs`, `project.rs` (files/recents/tree), `keys.rs` (dispatch).
Mechanics (**compiler-verified** in round 2): Rust allows `impl AppStore` blocks
in other modules of the same crate, and a module's **private** items are visible
to its **descendant** modules — so placing the files under
`app::store::*` (children of the module defining `AppStore`) lets them read and
mutate its private fields with **no `pub(crate)` pass at all**. Files must be
children, not siblings (`app::store_buffers` would need the visibility pass).
**Each stage is behavior-preserving and lands gate-green**; no stage mixes a
behavior change with a move. Warm-up stages first (`00-worklist.md` §Round-2
structural deltas): free helpers · notes doc · test module, then per-concern
`impl` moves.

**Phase 3 — split the tests.** `store.rs`'s test module → per-concern test
files alongside the new modules; `flow_tests.rs` split by the same concerns.

**Phase 4 — the other oversized files + general cleanup.**
`node.rs` per language, `queries.rs` per-language consts, `git/repo.rs`
(status/log/blame/commit); a dead-code/`allow(dead_code)` audit; duplicated
helpers (e.g. `truncate` exists in more than one ui module); stale comments
(several found this session); a `docs/README.md` index.

## Success criteria

- No `src/` file over ~1,500 lines; `store.rs` concerns split as planned.
- **Zero behavior change**: the full workspace suite + PTY gate stay green
  at every stage (829+ tests, 12/12 pooled, `gate.sh full` OK) with no test
  assertion weakened — moves are `git mv`-shaped, not rewrites.
- Two lanes can work on different app concerns file-disjointly.
- `docs/architecture.md` exists and matches the tree.

## Task order

| Phase | Issue | Depends on |
|---|---|---|
| 1 | 01 concern inventory + module map | — |
| 2 | 02 split pattern + buffers/views | 01 |
| 2 | 03 search | 02 |
| 2 | 04 magit + commit/log/blame | 02 |
| 2 | 05 notes + annotations | 02 |
| 2 | 06 navigation + index wiring | 02 |
| 2 | 07 picker + minibuffer + project/files | 02 |
| 3 | 08 tests split (store tests + flow_tests) | 02–07 |
| 4 | 09 other files + general cleanup | 08 |

Serialization: each store split stage owns `src/app/store*` alone (no
parallel app lanes), because the moves touch shared field visibility.
Stage 09's per-file work is file-disjoint and can fan out.

When complete, archive per plan-process.

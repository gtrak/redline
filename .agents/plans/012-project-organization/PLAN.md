# 012 — Redline: project organization & cleanup

Status: planned
Phases: 4 · Issues: 01–09 (staged; each stage lands gate-green)
Depends on: nothing (behavior-preserving refactor + cleanup)

## Why

`src/app/store.rs` is **22,993 lines** — roughly 40% of the 56 k-line codebase —
with an `impl AppStore` of **421 methods** (253 `pub` / 168 private) spanning lines
1701–11588, plus 11 associated free functions. One impl block holds every concern:
buffers, views, file-view cursor/scroll, search, magit, log/blame/commit, notes,
navigation, index wiring, pickers, minibuffer, project/files, key dispatch.
(Two count corrections on the way here: the widely-quoted "869" was a bad grep
that swept in the 11 k-line test module, and "432" counted methods + free fns.) It mixes
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
Inventory the methods into coherent concerns with line ranges; publish
`docs/architecture.md` (module map + the split plan + the mechanical
constraints). Reviewer checks the inventory against the file.

**Phase 2 — split `store.rs`, one concern per stage.**

**Step 0 (mandatory, do it first):** the authoritative concern map is
`docs/architecture.md`, but its **line numbers are baseline-pinned** to the commit
they were measured at and `store.rs` has already moved once (+45/−1 from the
picker-density landing). Re-run the census against the current file, update the
map, and only then start moving code. The counts (421 methods = 253 `pub` +
168 private) are stable; the line numbers are not.
Target layout (names indicative; the inventory confirms):
`src/app/store/mod.rs` (the struct + state + shared helpers),
`buffers.rs`, `views.rs`, `file_view.rs`, `search.rs`, `magit.rs`,
`commit.rs` (log/blame/diff/editor), `notes.rs`, `navigation.rs`
(M-./jump/xref/imenu/impls), `index_wiring.rs`, `picker.rs`,
`minibuffer.rs`, `project.rs` (files/recents/tree), `keys.rs` (dispatch).
Mechanics (**compiler-verified**, in both directions): Rust allows `impl AppStore`
blocks in other modules of the same crate, and a module's **private items are
visible to its descendant modules** — verified with `rustc`: a child module reads
and mutates private *fields* with no visibility change. **But** a private
*method* defined in one submodule and called from a **sibling** submodule fails
with `E0624: method is private`; `pub(super)` on it compiles. So the split needs:
files placed under `app::store::*` (**children** of the module defining
`AppStore`, not siblings like `app::store_buffers`), **no field-visibility pass**,
and a **`pub(super)` pass on the private methods that cross concern
boundaries** (measured at **73** when the 13 concern moves landed — far below the
168 private methods the original estimate assumed, because most private methods
are called from within their own concern). Two further facts the review
established, both easy to get wrong: `#[path = "flow_tests.rs"]` must become
**`"../flow_tests.rs"`** (the attribute resolves relative to the containing
file's directory, so it would otherwise point at a nonexistent
`src/app/store/flow_tests.rs` → E0583), and `collect_syntax_anchor_nodes` (2602)
+ `is_syntax_anchor_kind` (2627) are **impl methods formatted at column 0**, so
indent-based tooling must not move them out of the impl. A third rule came from
splitting `syntax/node.rs`: **a `pub use` re-export cannot re-export an item less
visible than itself** (`pub use` of a `pub(super)`/private item → **E0364**); for
internal-only items use `pub(crate) use` (or widen the item to `pub(crate)`), which
keeps the visibility narrowing rather than adding public API.
**Each stage is behavior-preserving and lands gate-green**; no stage mixes a
behavior change with a move. Warm-up stages first (`00-worklist.md` §Round-2
structural deltas): free helpers · notes doc · test module, then per-concern
`impl` moves. The authoritative concern/module map is `docs/architecture.md`
(from `012-01`).

**Phase 3 — split the tests.** `store.rs`'s test module → per-concern test
files alongside the new modules; `flow_tests.rs` split by the same concerns.

**Phase 4 — the other oversized files + general cleanup.**
`node.rs` per language, `queries.rs` per-language consts, `git/repo.rs`
(status/log/blame/commit); a dead-code/`allow(dead_code)` audit; duplicated
helpers (e.g. `truncate` exists in more than one ui module); stale comments
(several found this session); a `docs/README.md` index.

## Success criteria

- No `src/` file over ~1,500 lines; `store.rs` concerns split as planned.
  **PARTIALLY MET (audited after all 17 lanes landed)**: `store.rs`'s concerns ARE split
  (403 methods into 13 concern files + an 18-method core), but six `src/` files still
  exceed 1,500 — three are test files (cohesive), and the production misses are
  `store/mod.rs` 2,445 (cohesive core), `syntax/queries.rs` 1,653 (cohesive), and
  **`store/navigation.rs` 2,394, which is a grab-bag and is staged as A7** (see
  `00-worklist.md` § A7).
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

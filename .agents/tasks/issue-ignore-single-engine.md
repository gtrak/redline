# issue-ignore-single-engine — let the crate own the ignore decision

## Why

Redline already uses `ignore 0.4.33` (BurntSushi's ripgrep crate, `Cargo.toml`
line 46). The project walk (`FileList::build`), the search pipeline
(`src/search/rg.rs`), and the dependency-tree walk all go through it, and both
per-path predicates use its `Gitignore` / `GitignoreBuilder` **matchers**. But
the *decision* is hand-rolled in **two separate places**, and that is precisely
where every gitignore bug in this project has come from:

1. **`is_gitignored`** (`src/model/files.rs`) — the incremental filter's
   per-path decision, plus the marker-only walk's filter. Assembles the chain
   itself: `.ignore` then `.gitignore` per directory, `.git/info/exclude`,
   `Gitignore::global()`, `hidden`, plus the `graft/` prune.
2. **`git_ignored_by_ancestors`** (`src/search/rg.rs:435`) — the search walk's
   equivalent, with its own memo.

Neither is an accident. The crate exposes **no public single-path "is this
ignored?" API** (verified against 0.4.33: the only `is_ignore()` is a getter on
`DirEntry`), and the crate's `require_git(false)` is **not** a clean substitute
— its own doc (walk.rs:937-945) says that with it, "`.gitignore` files will be
read from parent directories above the git root directory containing `.git`,
which is different from the git behavior". So both predicates exist to buy two
things the crate's defaults do not give: honouring `.gitignore` in a
marker-only tree, and *not* reading ignore files above the git root.

**The cost is now measurable.** Three consecutive lanes have been spent making
these predicates agree with the crate's walk. Each closed one divergence and
exposed another:

* global-excludes root — fixed (`2503740`, `b4f1516`)
* `.ignore` not read — fixed (`b4f1516`)
* an ignore file ABOVE the project root — **still open**, documented in
  `src/model/files.rs`
* cross-level negation precedence — **still open**, documented
* and the two predicates are **already known to disagree with each other**: the
  `b4f1516` gate's PROBE-D showed the file-list walk and the search walk
  disagreeing on `.ignore` in a marker-only tree.

That is a treadmill: we maintain two imitations of a decision the crate already
makes, and grade ourselves on how closely they imitate it.

## Required — the shape to aim for

**One engine, used by all three paths, with the crate owning the decision.**

The lever is `WalkBuilder::require_git(false)`: with it, the crate applies the
git-related rules (`.gitignore`, `.git/info/exclude`, global excludes) **even
outside a git repository**, which is the behaviour we hand-rolled for
marker-only trees. If the walks take that setting:

* the marker-only filter in `FileList::build` becomes unnecessary;
* `git_ignored_by_ancestors` becomes unnecessary;
* `is_gitignored`'s chain assembly becomes unnecessary — the *incremental*
  filter can then be "one walk per debounced batch, intersected with the
  changed set" (changed ∧ in-walk → re-index; changed ∧ absent → `remove_file`),
  which agrees with the full build **by construction** rather than by
  maintenance.

## The two things that must be decided honestly, not assumed

1. **The semantic change is real and must be disclosed, not smuggled.**
   `require_git(false)` does not only switch on `.gitignore` in non-git trees —
   it also reads `.gitignore` from parents above the git root and applies
   global excludes in non-git trees. That is *more* ignoring than today, and
   more than git itself does. State it in the commit and in the module doc.
   If it turns out to be unacceptable, fall back to the narrower variant: keep
   the walk config as-is and only replace the *incremental* filter with the
   walk, leaving one predicate instead of two — and say which one survives and
   why.

2. **The walk is O(tree) per batch; the predicate is O(changed paths).** Do not
   decide this on vibes. Measure on `tools/ignore_fixture.py` (20,000 files,
   depth-4, 33-line root `.gitignore`, `.gitignore` every 5th dir) and on a git
   repo of comparable size:
   (a) one full `FileList::build` walk, and (b) the current memoized predicate
   over the same file set. `--index-profile` reports the walk phase directly.
   For calibration, a real user's 10,150-file project measured a **73 ms**
   walk. If the walk is not clearly worse, take it — deleting two predicates
   and their memos is a large simplification. If it is clearly worse for a
   plausible watcher-burst pattern, propose the cheaper variant (e.g. reuse the
   `FileList` already built and invalidate it only when an ignore file changes)
   rather than silently keeping the predicate.

## Acceptance

* The incremental path's decisions come from the same walk configuration as the
  full build — ideally literally the same function — and a reader can see that
  where it matters.
* The existing agreement tests (`git_repo_walk_and_filter_agree_*`,
  `walk_and_filter_agree_on_dot_ignore_files`, the F2 memo tests) are
  **re-shaped, not deleted**. The behaviour they pin is still required; they
  should assert the end-to-end outcome (what ends up in the index after a
  batch) instead of the predicate's internals. Replacing an assertion requires
  saying what replaces it — silently dropping one is not acceptable.
* **A test for the case the predicates could never handle**: an ignore file
  ABOVE the project root. With one walk this must agree with the full build by
  construction. Prove it end to end, with a fixture that actually has the
  property (an ignore file in a directory above the root — not merely a
  subdirectory `.ignore`, which the existing tests already cover).
* The `graft/` prune keeps its behaviour, and the dependency-tree exception
  (`crate_source_files`) is **not** folded into the project walk — it is a
  deliberate exception (dependency trees are exempt from project ignore rules
  so that jumping inside them works).
* `cargo test --workspace` + clippy `-- -D warnings` clean; `tools/gate.sh
  full` green.

## Fence

`src/model/files.rs`, `src/app/store/index_wiring.rs`, `src/search/rg.rs` (for
the second predicate), and the test files pinning either predicate's behaviour.
Disclose anything else.

## Rejected alternative — state it in the commit

Swapping the filter to a *different* per-path engine (e.g. `gix-ignore`, which
does expose a per-path API with git-faithful semantics) reintroduces exactly
the failure mode being removed: two engines, guaranteed to disagree. One
engine used by every path is the point.

## Sequencing

Do NOT start while `index-file-budget` is in flight — both lanes touch
`src/app/store/index_wiring.rs`. Land that first.

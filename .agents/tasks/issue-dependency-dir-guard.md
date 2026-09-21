# issue-dependency-dir-guard — a missing .gitignore line must not become a 56-second startup

## What happened (the evidence)

A user pointed redline at a real project and the cold index took **56,220 ms**
for 10,150 files. Almost all of it was ONE 11.3 MB `.cpp` file producing 0
symbols (55,351 ms). The cause was mundane and entirely predictable:
**`node_modules` was not in the project's `.gitignore`**, so the walk indexed
the whole dependency tree — 3,664 `.cpp`/`.hpp`, 698 `.c`/`.h`, the big
`.js`/`.ts`/`.json` files, and one generated C++ blob.

With `node_modules` excluded, the same project indexes in **368 ms** — a **153x**
difference — and the walk drops from 10,150 files to 3,596.

So the pathological file was never a legitimate project file. The real defect is
that **redline silently treated a dependency tree as project source**, and gave
the user a 56-second startup with no explanation, no warning, and no way to tell
why.

## Required

1. **Detect and disclose.** After the project walk, if it included files under
   well-known dependency/build directories that the project's ignore rules did
   NOT exclude, say so on the startup path: the directory, the file count, and
   the `.gitignore` line that fixes it. This alone would have turned 56 seconds
   of mystery into an actionable message. Cover at least: `node_modules`,
   `target`, `.venv`/`venv`, `vendor`, `dist`, `build`, `__pycache__`, `.next`,
   `.cache`, `Pods`, `bower_components`. Justify additions; keep the list in one
   place with a comment explaining the policy.
2. **Decide the default honestly, and state it.** **SETTLED by the user (2026-09-21):
   WARN — do not prune by default.** The user's words: *"dep-dir warning sounds good"*.
   So: detect and disclose (directory, file count, and the `.gitignore` line that fixes
   it), and leave the files in the index unless the user excludes them. Rationale worth
   recording: pruning by default would silently contradict the project's own ignore
   rules, and this spec exists because redline was *silent* about a dependency tree —
   replacing one silence with another would miss the point. The remaining requirements
   stand:
   * it must be **overridable** (a user may legitimately want to browse a
     directory that happens to be named `build/`);
   * it must NOT break the deliberate dependency-navigation exception
     (`crate_source_files`): dependencies stay *navigable after landing* — the
     guard is about what enters the *project* index, which is exactly the
     distinction the library-dep work already draws.
3. **The rule must be applied consistently by the walk and by the incremental
   filter.** A file the full build excludes must not be re-indexed by the
   incremental path. That is the same invariant the ignore work has been
   chasing, and the cheapest way to satisfy it is the single-engine change
   specced in `issue-ignore-single-engine.md` — so **sequence this after that**,
   or state explicitly how it stays consistent if done first.

## Acceptance

* A fixture with a **non-gitignored** `node_modules` (the property that
  triggers the bug — a fixture where it IS gitignored proves nothing) asserts
  the disclosure appears and reports the right count.
* A test that a `.gitignore`d `node_modules` produces **no** warning (no false
  positives).
* A test that the override restores the previous behaviour.
* The full-build/incremental agreement invariant holds for the guarded
  directories.
* `cargo test --workspace` + clippy `-- -D warnings` clean; `tools/gate.sh
  full` green.

## Fence

`src/model/files.rs` (the walk + the guard), `src/app/store/index_wiring.rs`
(if the incremental path must agree), the startup/status surface where the
message appears, and the tests. Disclose anything else.

## Not this lane

The per-file parse budget (`issue-index-file-budget.md`) — that is a safety net
for a *legitimate* pathological file, and its constants are justified against
the legitimate distribution (max 98 ms on the project above), not against this
dependency-tree outlier.

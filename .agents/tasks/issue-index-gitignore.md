# Task: the symbol indexer's incremental path must respect `.gitignore`

**User-reported:** *"I want the indexer to process .gitignore."*

## The diagnosis (verified — read this first, it is not what it looks like)

**The full index build already respects `.gitignore`.** `start_indexing`
(`src/app/store/index_wiring.rs:302`) does:

```rust
if self.ensure_files().is_none() { … }
let files = self.files.get(&root).map(|f| f.files.clone()).unwrap_or_default();
```

`self.files` is the **`FileList`**, and `FileList::build` (`src/model/files.rs`) **does**
consult gitignore (`load_gitignore` / `gitignore_matches`, with a nested-`.gitignore`
stack at `files.rs:119-143`). So the initial index is filtered.

**The incremental path is not.** `refresh_index(&change.paths)` is called
**unconditionally** on every watcher batch (`index_wiring.rs:171` — the `any_tracked`
git filter above it gates only `refresh_magit()`, not the index). It hands the watcher's
paths straight to the job, and `refresh_in_place` (`src/nav/index/builder.rs:63`) does:

```rust
match std::fs::read_to_string(abs) {
    Ok(text) => { … index.set_file(&rel, syms); index.set_file_tables(&rel, tables); }
    Err(_)   => { index.remove_file(&rel); }
}
```

for **any** changed path that reads as text — including files that were never in the
index because the full build excluded them. So an ignored file that changes (a build
artifact, a log, anything in `target/`) gets parsed and **added to the symbol index**.

## What to build

**Make the incremental path agree with the walk.** The shared predicate already exists:
`model::files::{load_gitignore, gitignore_matches}` (the R3 extraction). Filter the
watcher's changed paths through it before reparsing.

**The substantive part is nesting.** A changed path's ignore status must be evaluated
against the `.gitignore` chain from the project root down to that path — the file walk
does exactly this with a stack (`files.rs:119-143`: load the root matcher, then push a
matcher per directory as it descends). A one-shot `load_gitignore(root)` check would be
wrong for a `.gitignore` in a subdirectory, and would regress the nested-gitignore
behaviour the R3 lane added. **Extract a reusable helper** (e.g.
`fn is_gitignored(root: &Path, path: &Path, is_dir: bool) -> bool` in `model/files.rs`)
that walks the ancestor chain, and have **both** the file walk and the new index filter
call it — one source of truth, per the standing rule.

**Where to filter.** Two defensible places; pick one and justify it:
- **In the store** (`refresh_index`, before the job) — the store has the project root and
  the gitignore helpers, and `refresh_in_place` stays a pure "reparse exactly these files"
  function. **Recommended.**
- In `refresh_in_place` — safer against direct callers, but pushes filesystem/gitignore
  policy into the `nav` layer (which is otherwise a pure-ish indexer over a given file
  list).

## The invariant this closes

R3 established that *the file-list walk and the search pipeline must agree on gitignore
semantics* and pinned it with `finder_and_search_agree_on_gitignore`. The indexer is a
**third walker**, and its incremental path currently disagrees. So:

- **Extend that agreement test to three walkers** — file walk, search pipeline, and the
  index's changed-path filter — over one fixture with a root `.gitignore`, a **nested**
  one, a negation (`!important.log`), and a directory rule (`build/`).
- Note the **known, documented asymmetry** in the same fixture: the file walk prunes
  `graft/` while the search pipeline does not. Do **not** "fix" that here — it is a
  separate, documented decision. If your new helper changes it, that is a P1 (say so).

## Key decisions

- **Do not change the full-build path** — it is already correct; touching it risks
  regressing the filtered `FileList` behaviour.
- **Do not change what is indexed for a file that is NOT ignored** — the filter must be
  purely subtractive.
- **Deleted-file handling must keep working**: an ignored file that is deleted should not
  need an index removal (it was never indexed), and a non-ignored file that is deleted must
  still be removed. Check the filter does not accidentally skip the `Err` arm for a file
  that *is* in the index.
- **The watcher still reports ignored files** — that is fine and out of scope; this task
  filters them at the index boundary.

## Files

`src/model/files.rs` (the extracted `is_gitignored` helper + the walk refactor),
`src/app/store/index_wiring.rs` (the `refresh_index` filter), tests
(`src/model/files.rs`'s gitignore tests for the three-walker agreement,
`src/app/store/tests/index_wiring.rs` for the incremental filter).

## Verification

- `cargo build`; `cargo test --workspace` — reconcile against the **current** baseline
  (measure it) and account for every change.
- `cargo clippy --workspace --all-targets -- -D warnings` (read `${PIPESTATUS[0]}`).
- **Tests to add** (each must discriminate — a root-only `.gitignore` fixture cannot catch
  the nesting bug):
  (a) a changed path matching a **nested** `.gitignore` is **not** indexed;
  (b) a changed path matching the root `.gitignore` is not indexed;
  (c) a negation (`!keep.log`) **is** indexed;
  (d) a non-ignored changed path **is** still indexed (the filter is subtractive);
  (e) the three-walker agreement test (extending the R3 one).
- **`timeout 900 tools/gate.sh full`** — the index feeds `M-.`/imenu, so the battery
  matters. If swap blocks it, run the workspace suite + `drive_xref.py` + both windowing
  drives and report it DEFERRED.
- Report: the diagnosis you confirmed, where you put the filter and why, the helper's
  signature and its two callers, the `graft/` asymmetry's status (unchanged, expected), the
  tests with why each discriminates, and the gate output.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.

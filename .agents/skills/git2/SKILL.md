---
name: git2
description: In-repo reference for git2 0.21.0 covering exactly the magit-subset API surface redline needs (plan issues 07/08): status classification, diff hunk/line iteration, file and hunk-level staging, commit creation, log/revwalk, blame, branch checkout, and stash. All signatures and semantics verified against the vendored sources at ~/.cargo/registry/src/.../git2-0.21.0/ (and its libgit2-sys 1.9.7 C source for defaults the Rust bindings don't show). Use when implementing src/git/ wrappers; do not consult docs.rs for these topics — this file is authoritative for this version.
---

# git2

Version-pinned reference for **git2 0.21.0** (redline's `git2 = "0.21"` dependency).
Ground truth: vendored crate source
`/home/gary/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/git2-0.21.0/`
and its libgit2 C source
`.../libgit2-sys-0.18.8+1.9.7/libgit2/` (libgit2 1.9.7).
Everything below was read from those sources; items that could not be verified are marked **NOT VERIFIED**.

## Version & features

Pinned: `git2 = "0.21"` → resolves to 0.21.0 (edition 2021). Depends on `libgit2-sys 0.18.x`
(vendored copy in this env: 0.18.8+1.9.7, i.e. libgit2 1.9.7).

Actual `[features]` from the vendored Cargo.toml:

```toml
cred = ["dep:url"]
default = []                     # redline currently uses defaults
https = ["libgit2-sys/https", "openssl-sys", "openssl-probe", "cred"]
ssh = ["libgit2-sys/ssh", "cred"]
unstable = []
unstable-sha256 = ["libgit2-sys/unstable-sha256"]
vendored-libgit2 = ["libgit2-sys/vendored"]
vendored-openssl = ["openssl-sys/vendored", "libgit2-sys/vendored-openssl"]
zlib-ng-compat = ["libgit2-sys/zlib-ng-compat"]
```

- All of issues 07/08 (status, diff, staging, commit, log, blame, branch, stash) work with
  **default features** — they are local-only operations.
- Issue 08's optional push/pull needs `https` (pulls in `openssl-sys` + `openssl-probe` on
  non-macOS unix, plus `url`) and/or `ssh`. Add only if that work lands.
- `unstable-sha256` gates SHA-256 repo support (`Oid::from_str_ext`, `ObjectFormat`); ignore for v1.

## Status

Construct:

```rust
Repository::statuses(&self, options: Option<&mut StatusOptions>) -> Result<Statuses<'_>, Error>
```

`StatusOptions` (builder, all methods return `&mut Self`):
- `show(StatusShow)` — `Index` | `Workdir` | `IndexAndWorkdir` (default = IndexAndWorkdir)
- `include_untracked(bool)`, `include_ignored(bool)`, `include_unmodified(bool)`
- `renames_head_to_index(bool)`, `renames_index_to_workdir(bool)` — rename detection
- `recurse_untracked_dirs(bool)`, `exclude_submodules(bool)`, `disable_pathspec_match(bool)`
- `pathspec(T: IntoCString)` — fnmatch patterns; `rename_threshold(u16)` (default 50)
- `update_index(bool)` (refresh stat cache), `no_refresh(bool)`,
  `sort_case_sensitively(bool)` / `sort_case_insensitively(bool)`

⚠ **The default-flags trap (verified in libgit2 1.9.7 `src/libgit2/status.c`):**
`git_status_list_new` computes `flags = opts ? opts->flags : GIT_STATUS_OPT_DEFAULTS`, and
`GIT_STATUS_OPT_DEFAULTS = INCLUDE_IGNORED | INCLUDE_UNTRACKED | RECURSE_UNTRACKED_DIRS`
(`include/git2/status.h`). So:
- `repo.statuses(None)` → untracked AND ignored entries are included.
- `repo.statuses(Some(&mut StatusOptions::new()))` with no flags set → **nothing** is included
  (flags = 0). A freshly built `StatusOptions` does NOT inherit the defaults.

For a magit status view, build options explicitly:

```rust
let mut opts = StatusOptions::new();
opts.include_untracked(true)
    .renames_head_to_index(true)
    .renames_index_to_workdir(true);
// do NOT set include_ignored — ignored files must not appear
let statuses = repo.statuses(Some(&mut opts))?;
```

(If you ever pass `None`, filter `Status::IGNORED` entries out of the result.)

Iteration:

```rust
Statuses<'repo>::len(&self) -> usize
Statuses<'repo>::is_empty(&self) -> bool
Statuses<'repo>::iter(&self) -> StatusIter<'_>   // DoubleEndedIterator, ExactSizeIterator, FusedIterator
Statuses<'repo>::get(&self, index: usize) -> Option<StatusEntry<'_>>

StatusEntry<'_>::path(&self) -> Result<&str, Error>   // Err on non-UTF-8; use path_bytes() for robustness
StatusEntry<'_>::path_bytes(&self) -> &[u8]
StatusEntry<'_>::status(&self) -> Status              // bitflags
StatusEntry<'_>::head_to_index(&self) -> Option<DiffDelta<'_>>
StatusEntry<'_>::index_to_workdir(&self) -> Option<DiffDelta<'_>>
```

`Status` bitflags (all `u32`; helpers `is_index_new()`, `is_wt_modified()`, … exist for each):

| const | meaning (per lib.rs doc) |
|---|---|
| `CURRENT` | no change |
| `INDEX_NEW`, `INDEX_MODIFIED`, `INDEX_DELETED`, `INDEX_RENAMED`, `INDEX_TYPECHANGE` | **index relative to HEAD** (staged side) |
| `WT_NEW`, `WT_MODIFIED`, `WT_DELETED`, `WT_TYPECHANGE`, `WT_RENAMED`, `WT_UNREADABLE` | **workdir relative to index** (unstaged side) |
| `IGNORED` | file matches ignore rules |
| `CONFLICTED` | merge conflict in index |

Classification for the section tree:
- **staged** = any `INDEX_*` bit set (`status.intersects(INDEX_NEW | INDEX_MODIFIED | INDEX_DELETED | INDEX_RENAMED | INDEX_TYPECHANGE)`)
- **unstaged** = any `WT_*` bit set
- **untracked** = `WT_NEW` with no `INDEX_*` bits
- a file can be both staged and unstaged (e.g. `INDEX_NEW | WT_MODIFIED` = "A + M")

Single-file helpers (no rename detection, per their doc comments):
```rust
Repository::status_file(&self, path: &Path) -> Result<Status, Error>  // NotFound if no match
Repository::status_should_ignore(&self, path: &Path) -> Result<bool, Error>
```
Doc note (repo.rs): with a `pathspec` set, rename-detection results "may not be accurate" —
call with no pathspec if renames matter.

## Diff

Constructors on `Repository` (all take `opts: Option<&mut DiffOptions>`, return `Result<Diff<'_>, Error>`;
"old" side = first arg, "new" side = second):

| method | equivalent |
|---|---|
| `diff_tree_to_tree(old_tree: Option<&Tree>, new_tree: Option<&Tree>, opts)` | `git diff <old> <new>`; `None` = empty tree; both `None` is an error |
| `diff_tree_to_index(old_tree: Option<&Tree>, index: Option<&Index>, opts)` | `git diff --cached <tree>` (staged view; tree = old) |
| `diff_index_to_index(old_index: &Index, new_index: &Index, opts)` | index vs index |
| `diff_index_to_workdir(index: Option<&Index>, opts)` | `git diff` (unstaged view; index = old) |
| `diff_tree_to_workdir(old_tree: Option<&Tree>, opts)` | strict tree↔workdir, **ignores index state** (not `git diff <tree>`) |
| `diff_tree_to_workdir_with_index(old_tree: Option<&Tree>, opts)` | emulates `git diff <tree>` (blends staged deletes) |

For redline: **staged** = `diff_tree_to_index(head_tree, None, opts)`; **unstaged** =
`diff_index_to_workdir(None, opts)`; **total (log `RET` view, `git diff HEAD`)** =
`diff_tree_to_workdir_with_index(head_tree, opts)`.

`DiffOptions` (builder; defaults: context 3, interhunk 0, no untracked/ignored):
- `pathspec(T: IntoCString)` — constrain to paths; `disable_pathspec_match(bool)` for literal paths
- `context_lines(u32)` (default 3), `interhunk_lines(u32)` (default 0)
- `include_untracked(bool)`, `include_ignored(bool)`, `include_unmodified(bool)`,
  `recurse_untracked_dirs(bool)`, `skip_binary_check(bool)`, `force_text(bool)`,
  `reverse(bool)`, `ignore_filemode(bool)`, `show_untracked_content(bool)`

Iteration / rendering:

```rust
Diff<'repo>::deltas(&self) -> Deltas<'_>          // iterator of DiffDelta (DoubleEndedIterator, ExactSizeIterator)
Diff<'repo>::get_delta(&self, i: usize) -> Option<DiffDelta<'_>>
Diff<'repo>::print(&self, format: DiffFormat, cb: FnMut(DiffDelta, Option<DiffHunk>, DiffLine) -> bool) -> Result<(), Error>
Diff<'repo>::foreach(&self, file_cb, binary_cb: Option, hunk_cb: Option, line_cb: Option) -> Result<(), Error>
Diff<'repo>::stats(&self) -> Result<DiffStats, Error>   // files_changed/insertions/deletions
Diff<'repo>::find_similar(&mut self, opts: Option<&mut DiffFindOptions>) -> Result<(), Error>
```

`DiffFormat` variants: `Patch`, `PatchHeader`, `Raw`, `NameOnly`, `NameStatus`, `PatchId`.
Returning `false` from any callback aborts iteration with an error (`ErrorCode::User`).

Types for colored rendering (all verified):

```rust
DiffDelta::status(&self) -> Delta        // Unmodified, Added, Deleted, Modified, Renamed, Copied,
                                         // Ignored, Untracked, Typechange, Unreadable, Conflicted
DiffDelta::old_file(&self) -> DiffFile   // DiffDelta::new_file()
DiffFile::id(&self) -> Oid               // ZERO oid on the absent side; check exists()
DiffFile::path(&self) -> Option<&Path>
DiffFile::mode(&self) -> FileMode        // Blob, BlobExecutable, Link, Tree, …
DiffFile::exists(&self) -> bool

DiffHunk::old_start(&self) -> u32        // 1-based line in old file
DiffHunk::old_lines(&self) -> u32
DiffHunk::new_start(&self) -> u32        // 1-based line in new file
DiffHunk::new_lines(&self) -> u32
DiffHunk::header(&self) -> &[u8]         // "@@ -a,b +c,d @@"

DiffLine::origin_value(&self) -> DiffLineType
DiffLine::origin(&self) -> char          // ' ' '+', '-', '=', '>', '<', 'F', 'H', 'B'
DiffLine::content(&self) -> &'a [u8]     // includes trailing newline
DiffLine::old_lineno(&self) -> Option<u32>  // None for added lines
DiffLine::new_lineno(&self) -> Option<u32>  // None for deleted lines

DiffLineType: Context, Addition, Deletion, ContextEOFNL, AddEOFNL, DeleteEOFNL,
              FileHeader, HunkHeader, Binary
```

`FileHeader`/`HunkHeader`/`Binary` line types are only emitted through `print` (patch format),
not through `foreach`/delta-hunk iteration. Use `diff.print(DiffFormat::Patch, cb)` and match on
`origin_value()` for the magit-style colored diff renderer.

## Staging

```rust
Repository::index(&self) -> Result<Index, Error>

Index::add_path(&mut self, path: &Path) -> Result<(), Error>
    // add/update ONE file from disk; BYPASSES gitignore (like `git add <file>`);
    // fails on bare index instances
Index::add_all(&mut self, pathspecs: I, flags: IndexAddOption, cb: Option<&mut IndexMatchedPath>) -> Result<(), Error>
    // glob match against workdir; respects gitignore unless FORCE
Index::update_all(&mut self, pathspecs: I, cb: Option<&mut IndexMatchedPath>) -> Result<(), Error>
    // sync existing index entries with workdir: updates changed files, DELETES entries
    // whose workdir file no longer exists (this is how unstaging a deletion works)
Index::remove_path(&mut self, path: &Path) -> Result<(), Error>   // unstage a file
Index::remove_all(&mut self, pathspecs: I, cb) -> Result<(), Error>
Index::get_path(&self, path: &Path, stage: i32) -> Option<IndexEntry>  // stage 0 = normal
Index::read(&mut self, force: bool) -> Result<(), Error>
Index::has_conflicts(&self) -> bool
Index::write(&mut self) -> Result<(), Error>          // persist index to disk (atomic lock)
Index::write_tree(&mut self) -> Result<Oid>           // root tree oid; fails if index has conflicts
Index::add_frombuffer(&mut self, entry: &IndexEntry, data: &[u8]) -> Result<(), Error>
    // low-level: add/replace entry with in-memory content (bypasses gitignore)
```

`IndexAddOption` bitflags: `DEFAULT`, `FORCE`, `DISABLE_PATHSPEC_MATCH`, `CHECK_PATHSPEC`.
`IndexMatchedPath<'a> = dyn FnMut(&Path, &[u8]) -> i32` (0 = proceed, >0 = skip, <0 = abort).
`IndexEntry` is a plain struct with public fields: `ctime, mtime, dev, ino, mode: u32, uid, gid,
file_size, id: Oid, flags: u16, flags_extended: u16, path: Vec<u8>` — there is no
`IndexStatus`/`entry.status()` API in 0.21.0.

File-level magit ops:
- stage file: `index.add_path(path)` then `index.write()`
- unstage file (tracked): `index.remove_path(path)` then `index.write()`
- unstage a deletion: `index.update_all([path], None)` (re-adds from workdir) or `add_path`

### Commit recipe (verified — mirrors the crate's own tests in `blame.rs`)

```rust
let mut index = repo.index()?;
// ... staging operations ...
index.write()?;                                  // 1. persist index
let tree_id = index.write_tree()?;               // 2. Oid of root tree
let tree = repo.find_tree(tree_id)?;             // 3. Tree handle
let head = repo.head()?;                         // 4. resolved (direct) reference
let parents: Vec<Commit> = match head.target() { // None => unborn branch
    Some(oid) => vec![repo.find_commit(oid)?],
    None => vec![],
};
let sig = repo.signature()?;                     // 5. user.name/user.email from config
let parents_ref: Vec<&Commit> = parents.iter().collect();
let oid = repo.commit(                           // 6. create commit + update branch ref
    Some("HEAD"), &sig, &sig, message, &tree, &parents_ref,
)?;
```

```rust
Repository::commit(
    &self,
    update_ref: Option<&str>,          // Some("HEAD") updates the current branch atomically
    author: &Signature<'_>,
    committer: &Signature<'_>,
    message: &str,
    tree: &Tree<'_>,
    parents: &[&Commit<'_>],
) -> Result<Oid, Error>
```

Notes:
- **There is no `Commit::create` in 0.21.0** — commit creation is `Repository::commit` only.
- Passing `Some("HEAD")` makes the branch-ref update part of the same call; no separate
  `set_head` needed. Per the doc: if the ref exists, "the first parent must be the tip of this
  branch" (concurrent commit → error); if it doesn't exist, it is created.
- `repo.signature()` returns `ErrorCode::NotFound` when `user.name`/`user.email` are unset.
  Fallback: `Signature::now(name, email)` (errors if either contains angle brackets) or
  `repo.committer_from_env()`.
- `write_tree` errors (`Unmerged`) if the index has conflicts.
- `TreeBuilder` is the alternative for building trees without the index:
  `repo.treebuilder(Option<&Tree>)`, `.insert(filename, oid, filemode: i32)` (mode must be
  0o100644/0o100755/0o040000/0o120000/0o160000), `.remove(filename)`, `.write() -> Result<Oid>`.

## Hunk-level staging

**What 0.21.0 actually provides** (verified — the old `Diff::apply` /
`Diff::apply_to_tree` / `DiffApplyOptions` APIs are **gone** in 0.21; do not write code
expecting them):

```rust
// repo.rs
Repository::apply(&self, diff: &Diff<'_>, location: ApplyLocation,
                  options: Option<&mut ApplyOptions<'_>>) -> Result<(), Error>
Repository::apply_to_tree(&self, tree: &Tree<'_>, diff: &Diff<'_>,
                          options: Option<&mut ApplyOptions<'_>>) -> Result<Index, Error>

// apply.rs
ApplyLocation: WorkDir | Index | Both     // emulates `git apply` / `--cached` / `--index`
ApplyOptions::new()
ApplyOptions::check(&mut self, bool)                                  // dry run
ApplyOptions::hunk_callback(&mut self, F: FnMut(Option<DiffHunk<'_>>) -> bool)
ApplyOptions::delta_callback(&mut self, F: FnMut(Option<DiffDelta<'_>>) -> bool)
```

Callback contract (apply.rs): returning `true` applies that hunk/delta, `false` skips it.

**Verified workable recipe for staging one hunk** (pattern proven by the crate's own test
`apply_hunks_and_delta`, apply.rs:179):

```rust
// stage hunk at 1-based new-file start line `target_new_start` in `path`
let mut dopts = DiffOptions::new();
dopts.pathspec(path);
let diff = repo.diff_index_to_workdir(None, Some(&mut dopts))?;  // index = old, workdir = new

let mut aopts = ApplyOptions::new();
aopts.hunk_callback(move |hunk| match hunk {
    Some(h) => h.new_start() == target_new_start,   // false => hunk skipped
    None => true,
});
repo.apply(&diff, ApplyLocation::Index, Some(&mut aopts))?;
```

Verified behaviors (libgit2 1.9.7 `src/libgit2/apply.c`):
- `ApplyLocation::Index` = `git apply --cached`: applies **only to the index**, ignores the
  workdir; the index is written to disk by an internal `git_indexwriter` — **do not call
  `index.write()` after `apply`** (it is already persisted).
- Deletions/renames in the diff remove index entries; other changes add updated entries.
- Use `check(true)` first as a dry-run validation.

For **unstaging a hunk** (index → partially back toward HEAD): there is no reverse-apply flag
in 0.21.0. Workable approach with verified building blocks:
1. `index.get_path(path, 0)` → current `IndexEntry` (has `id`).
2. `repo.find_blob(entry.id)?.content()` → current index content.
3. Compute the new content in memory by reverting the chosen hunk (use the diff's
   `DiffLine` data: keep old lines, drop new lines for that hunk).
4. `index.add_frombuffer(&entry_with_new_id, new_content)` where the entry's `id` must be set
   to the oid of the new content (`repo.blob(new_content)?`) — `add_frombuffer` hashes the
   buffer, but the entry's other fields (path, mode) must be copied from the existing entry.
5. `index.write()`.
Marked **NOT VERIFIED** as a turnkey recipe — the hunk-reversion computation is
redline implementation work; each API used above is individually verified.

`git2::Patch` (patch.rs) is also available for per-hunk random access:
`Patch::from_diff(&diff, delta_idx) -> Result<Option<Patch>>`, `num_hunks()`,
`hunk(idx) -> Result<(DiffHunk, usize)>`, `line_in_hunk(h, l) -> Result<(DiffLine, usize)>`,
`to_buf() -> Result<Buf>` (full patch text for the delta). Useful for rendering; it does not
expose per-hunk patch text.

## Log

```rust
Repository::revwalk(&self) -> Result<Revwalk<'_>, Error>

Revwalk::push(&mut self, oid: Oid) -> Result<(), Error>
Revwalk::push_head(&mut self) -> Result<(), Error>
Revwalk::push_ref(&mut self, reference: &str) -> Result<(), Error>   // e.g. "refs/heads/main"
Revwalk::push_glob(&mut self, glob: &str) -> Result<(), Error>
Revwalk::push_range(&mut self, range: &str) -> Result<(), Error>     // "<c>..<c>"
Revwalk::hide(&mut self, oid: Oid) -> Result<(), Error>             // paging: hide the last page
Revwalk::hide_head / hide_glob / hide_ref
Revwalk::set_sorting(&mut self, sort_mode: Sort) -> Result<(), Error>
Revwalk::reset(&mut self) -> Result<(), Error>                      // reconfigure (auto-reset on exhaustion)
Revwalk: Iterator, Item = Result<Oid, Error>
```

`Sort` bitflags (NOT `Revsort` — 0.21.0's type is `Sort`): `NONE`, `TOPOLOGICAL`, `TIME`,
`REVERSE`. For a magit log: `walk.set_sorting(Sort::TIME)`; combine with `TOPOLOGICAL` if
desired. Branch-aware log: `push_ref("refs/heads/<name>")` instead of `push_head`.

Per-commit accessors (`Commit<'repo>`):
```rust
commit.id() -> Oid
commit.summary() -> Result<Option<&str>, Error>    // first paragraph, whitespace trimmed/squashed
commit.message() -> Result<&str, Error>
commit.time() -> Time                              // COMMITTER time (seconds + offset minutes)
commit.author() -> Signature<'_>                   // .name()/.email() -> Result<&str>, .when() -> Time
commit.committer() -> Signature<'_>
commit.tree() -> Result<Tree<'_>, Error>
commit.parent_count() -> usize; commit.parent(i) -> Result<Commit<'_>, Error>
```

## Blame

```rust
Repository::blame_file(&self, path: &Path, opts: Option<&mut BlameOptions>) -> Result<Blame<'_>, Error>

Blame::len / is_empty / iter() -> BlameIter<'_>    // Item = BlameHunk
Blame::get_index(&self, index: usize) -> Option<BlameHunk<'_>>
Blame::get_line(&self, lineno: usize) -> Option<BlameHunk<'_>>  // hunk containing 1-based line
Blame::blame_buffer(&self, buffer: &[u8]) -> Result<Blame<'_>, Error>  // blame modified-in-memory content

BlameHunk::final_commit_id(&self) -> Oid           // zero oid for uncommitted lines (buffer blame)
BlameHunk::final_start_line(&self) -> usize        // 1-based
BlameHunk::lines_in_hunk(&self) -> usize           // (this is the "num_lines" accessor; there is no num_lines())
BlameHunk::final_signature(&self) -> Option<Signature<'_>>   // author of final commit
BlameHunk::final_committer(&self) -> Option<Signature<'_>>
BlameHunk::orig_commit_id / orig_signature / orig_committer / orig_start_line / path()
BlameHunk::is_boundary(&self) -> bool
BlameHunk::summary(&self) -> Result<Option<&str>, Error>
```

`BlameOptions` (builder): `track_copies_same_file(bool)` (this is what people usually mean by
"track lines within a file" — there is no `track_lines()` in 0.21.0),
`track_copies_same_commit_moves`, `track_copies_same_commit_copies`,
`track_copies_any_commit_copies`, `first_parent(bool)`, `use_mailmap(bool)`,
`ignore_whitespace(bool)`, `newest_commit(Oid)`, `oldest_commit(Oid)`, `min_line(usize)`,
`max_line(usize)`.

## Branches & checkout

```rust
Repository::branches(&self, filter: Option<BranchType>) -> Result<Branches<'_>, Error>
    // BranchType::Local | Remote; iterator Item = Result<(Branch<'_>, BranchType), Error>
Repository::find_branch(&self, name: &str, branch_type: BranchType) -> Result<Branch<'_>, Error>
Repository::branch(&self, branch_name: &str, target: &Commit<'_>, force: bool) -> Result<Branch<'_>, Error>

Branch::name(&self) -> Result<Option<&str>, Error>
Branch::is_head(&self) -> bool
Branch::get(&self) -> &Reference<'_>               // or into_reference()
Branch::delete / rename
```

`Reference` essentials:
```rust
Reference::name(&self) -> Result<&str, Error>      // "refs/heads/main"
Reference::shorthand(&self) -> Result<&str, Error> // "main"
Reference::target(&self) -> Option<Oid>            // direct refs only
Reference::resolve(&self) -> Result<Reference<'_>, Error>  // follow symbolic chain
Reference::peel_to_commit(&self) -> Result<Commit<'_>, Error>
Reference::set_target(&mut self, id: Oid, reflog_msg: &str) -> Result<Reference<'_>, Error>
Repository::head(&self) -> Result<Reference<'_>, Error>     // returns the RESOLVED (direct) ref
Repository::head_detached(&self) -> Result<bool, Error>
Repository::set_head(&self, refname: &str) -> Result<(), Error>  // "refs/heads/main"
Repository::set_head_detached(&self, commitish: Oid) -> Result<(), Error>
```

**Checkout recipe (verified against the `checkout_head` doc comment, repo.rs:2166):**
`checkout_head` is explicitly documented as NOT the branch-switch mechanism. The correct
order for switching to an existing branch is:

```rust
let target = repo.find_branch(name, BranchType::Local)?.get().peel_to_commit()?;
let obj = target.as_object();
let mut cb = CheckoutBuilder::new();   // build.rs: dry_run/force/safe/update_only/update_index/…
repo.checkout_tree(obj, Some(&mut cb))?;          // 1. update workdir + index to target
repo.set_head(&format!("refs/heads/{name}"))?;    // 2. then move HEAD
```

`checkout_tree(treeish: &Object<'_>, opts)` matches "the content of the treeish" in both index
and workdir. `CheckoutBuilder::new()` with `.force()` is the safe default for a TUI branch
switch (refuses to clobber uncommitted changes otherwise — surface that error to the user).
There is **no `checkout_branch`** method in 0.21.0.

Create branch from HEAD: `repo.branch(name, &head_commit, false)`.

## Stash

All verified in repo.rs/stash.rs; note the `&mut self` receivers:

```rust
Repository::stash_save(&mut self, stasher: &Signature, message: &str, flags: Option<StashFlags>) -> Result<Oid, Error>
Repository::stash_save2(&mut self, stasher, message: Option<&str>, flags: Option<StashFlags>) -> Result<Oid, Error>
Repository::stash_save_ext(&mut self, opts: Option<&mut StashSaveOptions<'_>>) -> Result<Oid, Error>
Repository::stash_apply(&mut self, index: usize, opts: Option<&mut StashApplyOptions<'_>>) -> Result<(), Error>
Repository::stash_pop(&mut self, index: usize, opts: Option<&mut StashApplyOptions<'_>>) -> Result<(), Error>
Repository::stash_drop(&mut self, index: usize) -> Result<(), Error>
Repository::stash_foreach<C: FnMut(usize, &str, &Oid) -> bool>(&mut self, cb: C) -> Result<(), Error>
    // (index, message like "On main: msg", stash commit oid); return false to stop
```

`StashFlags` bitflags: `DEFAULT`, `KEEP_INDEX`, `INCLUDE_UNTRACKED`, `INCLUDE_IGNORED`, `KEEP_ALL`.
`StashApplyOptions::new()` with `reinstantiate_index()` (sets `REINSTATE_INDEX`),
`checkout_options(CheckoutBuilder)`, `progress_cb(FnMut(StashApplyProgress) -> bool)`.
Stash list for the picker: `stash_foreach` (index 0 = newest).

## Threading & errors

Verified facts (source-quoted):
- `Repository` is **`Send` but NOT `Sync`**: `unsafe impl Send for Repository {}` with the
  comment "It is the current belief that a `Repository` can be sent among threads, or even
  shared among threads in a mutex" (repo.rs:114). Share via `Arc<Mutex<Repository>>` or
  move into the worker thread; do not share `&Repository` across threads.
- `Diff<'repo>` is `Send` (diff.rs:26). `Odb` is `Send + Sync`. `Patch` is `Send`.
- **Everything else is NOT Send/Sync**: `Index`, `Commit`, `Tree`, `Reference`, `Branch`,
  `Statuses`, `Blame`, `Revwalk`, `Blob` all hold raw pointers with no `unsafe impl`.
  Pattern for magit workers: move the `Repository` into the worker thread and create all
  derived objects there. `Repository` is **not `Clone`** in 0.21.0 (no `Clone` impl in
  repo.rs) — for multiple workers, open a fresh `Repository::open` per thread.
- libgit2 itself is thread-safe (crate description: "both threadsafe and memory safe").

`git2::Error` (error.rs):
```rust
err.code() -> ErrorCode      // GenericError, NotFound, Exists, Ambiguous, User, BareRepo,
                             // UnbornBranch, Unmerged, NotFastForward, InvalidSpec, Conflict,
                             // Locked, Modified, Auth, ApplyFail, IndexDirty, Uncommitted, …
err.class() -> ErrorClass    // None, Library, Os, Indexer, Checkout, Merge, Invalid, User
err.message() -> &str
```
Useful matches: `NotFound` (no HEAD / missing ref / unset user.name), `BareRepo` (workdir
ops on bare repo), `Unmerged` (commit with conflicts), `ApplyFail` (hunk staging conflict),
`NotFastForward` (branch moved under you).

## Usage in redline

Mapping to the planned `src/git/` modules (plan issues 07/08; git2 types must not leak into UI):

| redline module | git2 surface |
|---|---|
| `src/git/repo.rs` | `Repository::open/discover`, `head`, `head_detached`, `index`, `signature`, `workdir` |
| `src/git/status.rs` | `statuses` with explicit `StatusOptions` (untracked + both rename flags, no ignored); classify via `INDEX_*`/`WT_*` bits; `status_file` for the status line; dirty counts from `Statuses::len` per class |
| `src/git/diff.rs` | `diff_tree_to_index` (staged), `diff_index_to_workdir` (unstaged), `diff_tree_to_workdir_with_index` (total/log view); `DiffOptions::pathspec` per file; render via `print(DiffFormat::Patch, cb)` + `DiffLineType` |
| staging (07) | `index.add_path` / `remove_path` / `update_all` + `index.write`; hunk stage via `Repository::apply` + `ApplyLocation::Index` + `ApplyOptions::hunk_callback`; hunk unstage via `add_frombuffer` recipe |
| commit (08) | the verified commit recipe above; `C-c C-c`/`C-c C-k` = run/skip it |
| log (08) | `revwalk` + `push_ref`/`push_head` + `Sort::TIME`; page by `hide(last_shown_oids)`; `find_commit` per oid; commit diff via `diff_tree_to_tree(parent_tree, tree)` |
| blame (08) | `blame_file` + `BlameOptions::track_copies_same_file`; render per-line from `final_start_line`/`lines_in_hunk` |
| branches (08) | `branches(Some(Local))`, `find_branch`, `branch(name, commit, false)`; checkout recipe (`checkout_tree` then `set_head`) |
| stash (08) | `stash_save2`, `stash_foreach` (picker source), `stash_pop`, `stash_drop` |

Watcher-driven re-runs (plan 04 bus):
- **any file change event** → re-run `statuses` (it is a snapshot; nothing is cached) and
  refresh affected diff views; cheap path: `status_file` per watched path for the status line.
- **after any staging op** (file or hunk) → re-run status + the staged/unstaged diffs; note
  `apply(…, Index)` already persisted the index, so no extra write.
- **after commit / branch checkout / stash pop** → full refresh: status, log, branch line,
  and re-resolve `head` (the old `Reference`/`Commit` handles are stale; drop and re-fetch).
- Status options `update_index(true)` (or `Index::read(false)`) keeps stat caches warm so
  watcher-triggered status calls stay cheap.

## Gotchas

Verified surprises only:

1. **`StatusOptions` default trap**: passing `Some(&mut StatusOptions::new())` yields flags=0
   (no untracked, no renames, no ignored); only `statuses(None)` gets libgit2's
   `GIT_STATUS_OPT_DEFAULTS` — which *includes* `INCLUDE_IGNORED`, so `None` also leaks
   `Status::IGNORED` entries into the list. Always build options explicitly and never set
   `include_ignored`. (libgit2 1.9.7 `status.c` / `status.h`.)
2. **No `Commit::create` in 0.21.0** — only `Repository::commit`; use `Some("HEAD")` as
   `update_ref` to fold the branch-ref update into the commit.
3. **No `Diff::apply` / `DiffApplyOptions` in 0.21.0** — hunk staging is
   `Repository::apply` + `ApplyLocation` + `ApplyOptions::hunk_callback`.
4. **`apply(…, ApplyLocation::Index)` writes the index to disk itself** (internal
   `git_indexwriter_commit`) — don't follow it with `index.write()`.
5. **Branch switch order**: `checkout_tree` FIRST, then `set_head` — the `checkout_head`
   doc comment explicitly warns that changing HEAD first "would leave you with checkout
   conflicts". There is no `checkout_branch`.
6. **`Repository` is `Send`, not `Sync`; derived types are not `Send` at all.** Magit workers
   must create `Index`/`Statuses`/`Diff`/`Blame` on the worker thread from a moved
   `Repository`.
7. **`DiffFile::id()` is a zero `Oid` on the absent side** (e.g. `old_file` of an `Added`
   delta) — check `exists()` before using ids.
8. **`status_file` does no rename detection** and returns `NotFound` for unknown paths;
   `statuses` with a pathspec may mis-report renames (doc note) — filter in redline instead
   of pathspec-filtering when renames matter.
9. **`IndexEntry` has no status API in 0.21.0** (no `IndexStatus`); it's a field-struct.
   `add_path` bypasses gitignore while `add_all` respects it — pick deliberately.
10. **`write_tree` fails on conflicted index**; `Repository::commit` fails if the branch tip
    moved since the parent was read ("first parent must be the tip").
11. **`Revwalk` items are `Result<Oid, Error>`**; call `reset()` before re-pushing after a
    completed walk (auto-reset on exhaustion). `Sort` is the type name, not `Revsort`.
12. **`BlameHunk` line-count accessor is `lines_in_hunk()`**, not `num_lines()`; uncommitted
    (buffer-blame) hunks have a zero `final_commit_id`.
13. **`stash_*` take `&mut self`** — the wrapper needs a mutable `Repository`.
14. **`Diff::print`/`foreach` callbacks abort the whole iteration with `ErrorCode::User`
    when they return `false`** — a panic inside a callback is caught (`panic::wrap`) and also
    surfaces as that error; don't rely on callbacks for fallible work without handling it.
15. **`Oid` vs `Reference`**: `head().target()` is `Option<Oid>` (None on symbolic refs —
    but `head()` already resolves, so expect `Some`). `Oid::from_str` accepts **abbreviated**
    hex (it wraps `git_oid_fromstrn`, zero-padding the rest) — do not use it to parse user
    input; use `repo.find_commit_by_prefix(prefix_hash)` for short-hash lookup.
16. **`StatusEntry::path()` is `Result<&str, Error>`** (non-UTF-8 paths fail); use
    `path_bytes()` for byte-safe handling.
17. **Unborn branch**: `repo.head()` errors (no HEAD commit) — the commit recipe must treat
    `parents` as empty and `repo.commit(Some("HEAD"), …)` will create the branch ref.

# Task: the ignore predicate — git-repo correctness, and stop re-parsing `.gitignore` per entry

Follow-up to `ca4e743` (the incremental index path now respects `.gitignore`). Its gate
verified the core fix by execution and filed four findings, **all measured** — this lane
closes them. Everything here is in the same two files, so it is one coherent change.

## F1 (P2) — the git-repo divergence (demonstrated by execution)

The **walk** uses `WalkBuilder.git_ignore(true)`, and `IgnoreBuilder`'s defaults
(`ignore-0.4.33/src/dir.rs:792-801`) also set `git_global: true, git_exclude: true,
require_git: true` — so the walk honours **`.git/info/exclude`** and **global excludes** (and
`parents`). The **filter** reads only `.gitignore` files.

Demonstrated: with `.git/info/exclude` containing `local/` and `secret.rs`, the walk yields
only `src/a.rs`, while `is_gitignored(root, "src/secret.rs", false)` returns **false**.

**Impact**: a changed file ignored *solely* by `.git/info/exclude` or a global exclude is
reparsed into the index and persists **for the whole session** as spurious `M-.`/imenu
candidates. `.git/info/exclude` is a normal home for local build artifacts and generated
sources, so this is not purely editor junk.

**Fix**: for a repo (`root/.git` exists), also consult `.git/info/exclude`
(`GitignoreBuilder::add`) and global excludes (`Gitignore::global()` — both available in
0.4.33). Keep the marker-only path unchanged.

**Bundle the fixture with the fix, not after it.** Add a **git-repo** agreement fixture. A
fixture alone would pin green an invariant that is currently false and give false comfort —
which is why the gate explicitly recommended bundling. Note the existing test's `project()`
has **no `.git`**, so both arms share `is_gitignored` and its agreement is *structural*
rather than behavioural.

**Test hazard**: global excludes come from the user's git config, so a test must not depend
on the machine. Use the established hermetic pattern (`GIT_CONFIG_GLOBAL=/dev/null` + a
temp `HOME`) or build the matcher from explicit temp files — see the git test-harness work
for the pattern.

## F2 (P2, MEASURED) — memoize the `.gitignore` chain; the walk regressed 6.2×

The gate measured this on a synthetic **20,000-file, depth-4 marker-only tree** (33-line root
`.gitignore`, a `.gitignore` every 5th directory), release build:

```
OLD (matcher stack):     342.9 ms   10,422 gitignore loads
NEW (per-entry chain):  2112.3 ms  161,240 gitignore loads      -> 6.2x, 15x the probes
```

Cause: the new per-entry ancestor chain **re-parses the root `.gitignore` for every entry**
(~12 µs per load) instead of one matcher per directory.

**Fix**: memoize `Option<Gitignore>` **per directory** in a `Mutex<HashMap<PathBuf,
Option<Gitignore>>>`, exactly as the search pipeline's `git_ignored_by_ancestors` already
does (`src/search/rg.rs:437-446`).

**Memo lifetime — decide and state it.** It must be **per invocation** (created inside the
walk / inside `indexable_changes`), **never** a long-lived global: `.gitignore` files change
while the app runs, and a persistent cache would serve stale verdicts with no invalidation
path. Say which scope you chose and why.

**Prove the fix with the profiler we just landed** — this is what it is for:
`redline --index-profile=<fixture>` reports **walk ms separately**, so measure before/after
on a large fixture and report both numbers. Do not just assert that it got faster.

## F3 (P2) — the same I/O is now synchronous on the UI thread

`indexable_changes` does one `is_dir()` plus O(depth) `.gitignore` parses **per changed
path, synchronously in the store's change handler**, where the pre-change code did zero I/O
before `spawn_blocking`. At ~12 µs/load that is ≈45 ms for a 1,000-path batch and ≈0.5 s for
a 10,000-path checkout/branch-switch burst. The F2 memo removes it — verify the memo is
actually shared across the paths in one batch (a memo created per *path* would not help).

## F4 (P2, honesty) — soften the claim, and close the `hidden` divergence

`src/model/files.rs:10-14` and the previous commit claim "one source of truth" / "closes the
R3 invariant for the third walker". That is true **only for marker-only trees**: in git repos
the filter does not reproduce the walk (F1), and **in both**, the walk applies `hidden(true)`
(`files.rs:41`) while the filter does not — so a changed path under a hidden directory that
holds source-extension files (`.venv/.../x.py`, `.cargo/...`, `.gradle/...`) is reparsed in
after the full build excluded it. That divergence is pre-existing and outside the original
`.gitignore` ask, but it contradicts the claim.

- **Soften the claim** to say exactly what is true, and say which cases remain.
- **Mirror `hidden(true)` in the filter** (skip any hidden component), so the filter
  reproduces another walk decision — with a test. This is the same class of bug as F1: *the
  incremental path must not index what the full build excluded.* If you find a reason not to
  mirror it, say so explicitly rather than leaving the divergence silent.

## P3s (cheap, do them while you are here)

- `is_gitignored` uses the **ancestor form for files but plain `matched` for directories**, so
  `build/sub` (is_dir=true) is not reported ignored by a root `build/` rule. Harmless in
  practice (`read_to_string` on a dir errors into `remove_file`), but inconsistent — use the
  ancestor form for dirs too, or add an assertion that pins the intent.
- `matched_path_or_any_parents` **panics** (`assert!(!path.has_root())`,
  `gitignore.rs:243`) when handed a path not under the matcher root — executed by the gate.
  It is guarded here by the early `strip_prefix`, but `is_gitignored` is now **`pub`**, so
  document the precondition on the function.
- Cross-level negation precedence (a deeper `!pat` cannot rescue a path ignored by a
  shallower `.gitignore`) is **not** real git semantics — but it is pre-existing in all three
  walkers, so they still agree with each other. Note it; do not fix it here.

## Files

`src/model/files.rs` (the predicate + the memo + the doc claims),
`src/app/store/index_wiring.rs` (`indexable_changes`), and the tests in both.

## Verification

- `cargo build`; `cargo test --workspace` — reconcile against the **current** baseline
  (measure it; the bin is at **748** passing) and account for every change.
- `cargo clippy --workspace --all-targets -- -D warnings` (`${PIPESTATUS[0]}`) — and if the
  run is suspiciously fast, **force a real re-lint** (change the fingerprint) rather than
  accepting a cached pass; the last gate had to do exactly that.
- **The walk measurement (required, executed)**: build a large marker-only fixture, run
  `--index-profile` on it before and after the memo, and report the **walk ms** both times.
- **Tests to add** (each must discriminate):
  (a) a **git-repo** fixture: `.git/info/exclude` and a global exclude both make a changed
      path non-indexable, and the filter agrees with the walk;
  (b) the marker-only path is unchanged (existing tests still pass);
  (c) a **hidden** component is not indexed (F4), matching the walk;
  (d) the memo: one batch with many paths under the same directories loads each `.gitignore`
      a bounded number of times (count the loads, or assert the walk time).
- **`timeout 900 tools/gate.sh full`** — the index feeds `M-.`/imenu, so the battery matters.
  Other lanes may be running; if it fails under load, report `uptime`/`free -g` and label it.
- Report: the git-repo fix and how the fixture proves it, the **before/after walk ms** from
  the profiler, the memo scope you chose and why, the `hidden` decision, the softened claim
  wording, the tests with why each discriminates, and the gate output.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.

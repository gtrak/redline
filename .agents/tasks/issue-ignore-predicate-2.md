# Task: ignore-predicate follow-up — faithful global-excludes, and three hygiene fixes

Follow-up to `2503740`. Its gate passed the lane (**PASS-with-findings**, full battery OK, F5's
pre-fix failures re-executed) and filed five findings, **all measured or read with executed
corroboration**. None is blocking; all are small and in the same two files.

## P2a — the global-excludes match is not faithful to the walk (CORRECTNESS; fix first)

`is_gitignored` (`src/model/files.rs`, the global-matcher arm ~`:275/:284`, doc claim ~`:329`)
uses `Gitignore::global()`, whose root is the **process cwd** — and the walk's global matcher is
rooted the same way, so the *matcher* is right. The **input** is wrong: the walk feeds the
**cwd-relative/absolute** path, the filter feeds the **project-relative** path. They therefore
disagree whenever a global pattern contains a **slash** (or is anchored).

The gate measured it (hermetic config, 13 candidates, repo root both under and outside cwd):

```
sub/inner.rs        walk_keeps=true   filter_ignored=true   <<< DIVERGE
anchored.rs (/pat)  walk_keeps=true   filter_ignored=true   <<< DIVERGE
(plain.rs, dirpat/x.rs, a.log, nested/plain.rs, excl/plain2.rs, /anchored3.rs ... agree)
```

Real git agrees with the **filter** here (`git check-ignore -v` with `core.excludesFile` reports
both ignored) — so the filter matches git while the **walk** does not, and the consequence is the
**opposite** of the bug F1 fixed: the filter **over-ignores**, so a changed file the full build
*did* index is never re-indexed and its symbols go **stale** until a full rebuild.

**Fix**: pass the **absolute `path`** (not `rel`) to the global matcher only — same matcher, same
input as the walk, therefore the same verdict. **Keep `rel` for `.git/info/exclude`**, which the
gate confirmed is faithful as written (probe agrees with the walk for a slash pattern and an
anchored pattern, and both keep `nested/anchored3.rs`).

**Test that discriminates** (must fail before the fix): a hermetic global excludes file containing
a **slash pattern** and an **anchored pattern**, asserting the filter's verdict equals **walk
membership** on every candidate — and, where practical, that it agrees with real
`git check-ignore` for the same paths. Extend the existing hermetic fixture rather than adding a
parallel one.

## P2b — a false doc claim, and a test whose stated discrimination is false

`src/app/store/index_wiring.rs:552-558` says `parents + git_ignore + require_git` make
`ignore::Walk::new` apply the PROJECT's `.gitignore` "(e.g. `node_modules/`) … silently zeroing
the file list". **Executed pre-fix: the old walker returned `["index.js"]`** for a repo whose
`.gitignore` is `node_modules/` — a pattern matching an ancestor **above** the walk root does not
apply. A second probe (`.gitignore` = `node_modules/`, `*.min.js`, `dist/`; exclude =
`localgen/`) returned `["index.js"]`, dropping `app.min.js`, `dist/x.js`, `localgen/y.js`.

So the real pre-fix failure modes are: **hidden pruning**, **`.git/info/exclude`/global
excludes**, and **below-root `.gitignore` patterns** — not the ancestor-directory rule. Correct
the comment accordingly (drop the `node_modules/` example and the "zero files" claim).

Consequence: `git_repo_node_modules_dependency_resolves_in_crate_via_m_dot`
(`tests/index_wiring.rs:711-714`, doc "Pre-F5 the walk saw zero files … M-. fell through")
**passes on the pre-fix walker**, so its stated discrimination is false. It is still a fine e2e
regression test — **fix its doc comment**, do not delete it, and do not pretend it discriminates.
(The two tests that *do* discriminate are the node_modules and hidden-venv ones, which the gate
re-executed failing pre-fix.)

## P3a — F4's wording is still incomplete (a parent `.ignore` file)

The module doc (`src/model/files.rs:21-31`) says the filter reproduces the walk for marker-only
trees. **False with a parent `.ignore` file**: probe gave `walk=["keep.rs"] filter_ignored=false`
— the walk drops a path the filter re-indexes, i.e. the original F1-class bug, pre-existing and
outside the F1–F5 fence. The `ignore` crate's walk honours `.ignore` (its `ignore: true`
default); the filter does not.

**Decide and state**: read `.ignore` too (it is symmetric — the same per-directory chain with a
second filename), or name the divergence in the doc alongside the negation one. **Reading it is
the faithful choice**; take it unless you find a concrete reason not to. A subdir-of-repo was
checked by the gate and is **not** a problem (the `root/.git` gate is empirically consistent).

## P3b — `EnvGuard` cites a precedent that contradicts it

`EnvGuard` (`src/model/files.rs:566-596`) mutates process-global env **without a lock**, and its
doc cites `git/commit.rs`'s `IsolatedHome` as "the established pattern … accepted race window".
That precedent **actually serializes behind a `static LOCK`** and comments that other threads
reading env vars concurrently is UB — so the citation is backwards. Exposure is low (no other
fixture contains the temp excludes' only pattern) and no flake was observed in four full runs,
but **share one crate-level mutex** and fix the citation.

## P3c — F2's numbers are not reproducible from the tree

The lane reported 7359.7 → 555.0 ms but committed **no fixture generator** and stated no profile,
so the gate could not reproduce the absolutes (its own debug pair: 8751.5 → 1529.8 ms cold,
5.7×, same direction). **Either commit the generator** (a small `tools/` script, or make the
existing fixture parametrizable) **and state the profile**, **or restate the result as a ratio
with the profile named** — do not leave an unverifiable absolute in the record.

## Verification

- `cargo build`; `cargo test --workspace` — reconcile against the **current** baseline (measure
  it; the bin is at **755**) and account for every change.
- `cargo clippy --workspace --all-targets -- -D warnings` (`${PIPESTATUS[0]}`) — **force a real
  re-lint** if the run looks cached (the last two gates both had to).
- **P2a's test must be shown failing before the fix** (revert just that change in a scratch copy)
  — a claim of discrimination that was not re-executed is the failure mode this repo has been
  bitten by three times now.
- **`timeout 900 tools/gate.sh full`** — the index feeds `M-.`/imenu, so the battery matters.
  Another lane is under review concurrently; if it fails, report `uptime`/`free -g` and attribute
  it rather than asserting a regression.
- Report: the P2a fix and the pre-fix failure you observed, the `.ignore` decision with its
  reasoning, the corrected doc claims (quote the new wording), the mutex change, what you did
  about P3c, and the gate output.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.

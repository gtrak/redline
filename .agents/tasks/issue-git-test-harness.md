# Task: one crate-wide git test harness

The store test dedup (`a1c2e47`) consolidated **10** `git_cli` copies inside
`src/app/store/tests/*` into one shared helper in `tests/mod.rs`. But that lane was
fenced to `store/tests/*`, so it could not see a **second cluster** of the identical
harness elsewhere — and the scanner (`tools/cleanup_scan.py dup`) proves it is
byte-identical:

## The duplication (measured)

| Duplicate | Sites | Size |
|---|---|---|
| `fn git()` | `src/git/blame.rs:81`, `src/git/log.rs:173`, `src/git/refs.rs:111`, `src/git/repo/tests.rs:14` | ~20 lines each |
| `fn init_repo()` | `src/git/blame.rs:102`, `src/git/log.rs:194`, `src/git/repo/tests.rs:35` | ~7 lines each |
| `fn git_cli()` | `src/ui/magit_status.rs:30`, `src/ui/rows_view.rs:34` | ~14 lines each |
| the hermetic env block | all 6 sites above | 6 lines each |

All are inside `#[cfg(test)]`. Plus the already-shared `store/tests/mod.rs` pair
(`git_cli(dir, args, name, email)` at `:12`, `git_repo_init` at `:32`).

## Why this matters (not cosmetics)

The store dedup's one near-miss was a variant that had drifted — which is exactly
how a **host-identity flake** is born: a copy that forgets
`GIT_CONFIG_GLOBAL=/dev/null` / `GIT_CONFIG_SYSTEM=/dev/null` inherits the
developer's `~/.gitconfig` and starts depending on the machine. **Nine
hand-maintained copies of the hermetic env block are nine chances to forget it.**
This lane makes the hermeticity a single fact, stated once.

## Design

**`src/test_support.rs`**, declared in `src/main.rs` as
`#[cfg(test)] pub(crate) mod test_support;` — available to every `#[cfg(test)]`
module in the bin via `crate::test_support::…`, compiled into nothing in a release
build.

It provides, as the **single source of truth**:
- `git_cli(dir: &Path, args: &[&str], name: &str, email: &str)` — run git in `dir`
  with the hermetic env and identity, asserting success (matching the shared
  helper's existing assertion text verbatim).
- `git_repo_init(dir: &Path, name: &str, email: &str, gpgsign: bool)` — init +
  prime the config, mirroring which callers actually set `commit.gpgsign`.
- The hermetic env block, stated **once**.

**Subsume, do not duplicate:** `src/app/store/tests/mod.rs`'s `git_cli` /
`git_repo_init` must **delegate to** (or be replaced by) the shared one, so there is
genuinely one implementation. Do not leave two.

**Keep honest differences at call sites.** The fixture identities are real and must
not be collapsed: the standard `Test`/`test@example.com`, the magit windowing test's
`T`/`t@e.com`, and the windowing wrapper's mixed `Test`/`t@e.com`. They are
parameters with explanatory comments, exactly as the store dedup left them. Same for
the `gpgsign` flag.

## Constraints

- **Assertion preservation is the bar.** This is a dedup: every test must still
  assert the same thing. Only helper bodies and imports may change. Prove it by
  diffing the test bodies before/after (comment-stripped, whitespace-normalized)
  and reporting every delta — the store dedup's gate did exactly this and found
  "67 tests, none added/removed, all bodies identical except 11 whose only deltas
  are removed helper definitions and rewired call sites".
- **Do not touch** `git/repo/tests.rs`'s test logic, the 807-line test file's
  contents, or any git behaviour. This is a harness move.
- **Report the hermeticity audit**: confirm every one of the 9+ copies already had
  both env vars (or say which did not — that is a latent flake worth naming).
- Fence: `src/test_support.rs` (new), `src/main.rs` (the `mod` declaration only),
  `src/git/{blame,log,refs,repo/tests}.rs`, `src/ui/{magit_status,rows_view}.rs`,
  `src/app/store/tests/mod.rs` (the delegation only). Nothing else.

## Sequencing

**Do not run this while plan 012's A7 lane is in flight** — A7 edits
`src/app/store/mod.rs` and `src/app/store/tests/mod.rs` (mod declarations), so this
lane would collide. Start after A7 lands.

## Forward note (plan 014)

When `redline-git` is extracted (`.agents/plans/014-crate-extraction/02-extract-git.md`),
this module is promoted to `crates/redline-testutil` as a **dev-dependency**, which
also makes it available to `redline-resolve`'s tests. Keep the helper free of any
app types (`std::path::Path` + `std::process::Command` only) so that promotion is a
move, not a rewrite.

## Verification

`cargo build`; `cargo test --workspace` (reconcile the per-target `test result:`
lines against the pre-lane baseline: redline 854 passed / 0 failed / 2 ignored,
resolver 123/0/4, integration 7,2,3,1,1, doctests 0 — the totals must not change);
`cargo clippy --workspace --all-targets` (read `${PIPESTATUS[0]}`);
`timeout 900 tools/gate.sh full`. **Resource guard**: `export CARGO_BUILD_JOBS=4`;
check `free -g` and swap first; if swap is exhausted, run the workspace suite plus
the two windowing drives and report the PTY battery as DEFERRED, not failed.

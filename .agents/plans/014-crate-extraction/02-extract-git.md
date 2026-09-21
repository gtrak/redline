# 014-02 — extract `redline-git` (and promote the git test harness)

**Objective.** Move `src/git/` (3,262 lines, 12 files) into `crates/redline-git`,
and promote the duplicated git test harness into `crates/redline-testutil` as a
**dev-dependency** so it survives the move and serves every crate.

**Depends on 014-01** (do it after `redline-syntax` has proven the approach).

**Why it is viable:** `git` is a production leaf — the only inbound edge is
**test-only** (`src/git/repo/tests.rs:8` uses `model::sections::StatusTree`). That
becomes a `[dev-dependencies]` on `redline-model` (cargo permits a dev-dep edge
without a build cycle). The production direction is one-way:
`model/sections.rs` → `git::diff` / `git::status` types.

## Key decisions

- **Move `git` before `model`.** `git`'s dependency on `model` is test-only, so it
  is a leaf in the build graph; `model` is not (it reads git types in production).
  Moving `git` first keeps each stage a clean leaf extraction.
- **Promote the test harness to `crates/redline-testutil`** in this stage, because
  the git tests are the reason it exists and they are about to change address. See
  `.agents/tasks/issue-git-test-harness.md` — if that lane has already landed, the
  harness currently lives at `src/test_support.rs`; this stage moves it into the
  new crate and switches the call sites to `redline_testutil::…`.
  A dev-dependency crate is available to `#[cfg(test)]` unit tests and to
  `tests/`, so this works for the bin, `redline-git`, and `redline-resolve`.
- **The git tests move with the code.** `git/repo/tests.rs` (807 lines) and the
  test modules in `blame.rs` / `log.rs` / `refs.rs` / `commit.rs` move into the
  crate. Their `model::sections::StatusTree` use becomes a dev-dep import.
- **Preserve every assertion** — this is a move; a test that changes meaning is a P1.
- **Do not touch `docs/architecture.md`'s module inventory beyond the crate map**,
  and do not "improve" any git logic while moving.

## Files

| File | Change |
|---|---|
| `crates/redline-git/Cargo.toml` | new: package + `redline-model` as a dev-dep if needed |
| `crates/redline-git/src/*` | the moved `src/git/*` |
| `crates/redline-testutil/` | new (or moved from `src/test_support.rs`) |
| `Cargo.toml` (root) | `redline-git` dependency; `redline-testutil` dev-dependency |
| `src/git/` | deleted |
| `use crate::git::…` sites | → `use redline_git::…` |
| `docs/architecture.md` | the crate map |

## Steps

1. Land the test-harness promotion first if it is not already landed (smaller,
   independently verifiable), or fold it in as the first commit of this stage.
2. Scaffold `crates/redline-git`, `git mv` the files, move the deps, compile.
3. Rewrite the bin's `use crate::git::` imports; fix the `pub` surface from the
   compiler errors; report each widening.
4. Move the git test modules; point them at `redline-testutil`; add the
   `redline-model` dev-dep for `StatusTree`.

## Verification

- `cargo build --workspace`; `cargo test --workspace`;
  `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test -p redline-git` — the moved tests, count reported and reconciled
  against the pre-move total (nothing dropped).
- Assertion preservation: diff the test bodies before/after (comment-stripped,
  whitespace-normalized) and report any change; only helper/import lines may differ.
- `timeout 900 tools/gate.sh full` green.
- Compile-time evidence for touching `src/app/` before/after.

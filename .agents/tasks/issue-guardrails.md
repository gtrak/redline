# Task: guardrails — document the unsafe blocks and make the fast gate hard to skip

The last unspecced Tier-1 item. Three independent parts; **A and B are cheap and are the
real content — C is convenience**. Land them together but do not let C's plumbing absorb
the budget.

## Measured facts (re-derived, whole artifact — do not trust older notes)

- **8 `unsafe` sites**, and **zero `SAFETY:` comments anywhere** in `src/` or `crates/`.
  - **6 production sites, all in `src/main.rs`** — the quit-dump tty reroute:
    - `:74` the `unsafe extern "C"` block declaring `dup`/`dup2`/`close`;
    - `:100` `dup(1)`; `:114` and `:125` `close(original)`; `:118`
      `dup2(tty.as_raw_fd(), 1)`; `:205` `File::from_raw_fd(fd)`.
  - **2 test-only sites in `src/git/commit.rs`** (`:158`, `:173`) — inside
    `#[cfg(test)] mod tests` (cfg at `:63`, `mod tests` at `:64`): `std::env::set_var`
    in the `IsolatedHome` guard.
- **No `[lints]` table** in the workspace or any crate `Cargo.toml`; **no crate-level lint
  attributes** in `main.rs` or any `lib.rs`.
- **No `.githooks/`, no `.github/workflows/`, no `.git/hooks/pre-push`.**
- `tools/gate.sh fast` = **build + clippy + tests, no PTY** (verified) — so it is a
  legitimate hook payload: no PTY concurrency risk, ~30 s.

## Part A — document the unsafe blocks honestly

Each production site gets a `// SAFETY:` comment stating **the actual invariant that makes
it sound**, not a restatement of the call:

- `dup(1)`: fd 1 is valid because the process has stdout; the returned fd is **ours to
  close** (and the code closes it on every path).
- `close(original)`: the fd came from our `dup`, and the three paths that close it are
  mutually exclusive (**no double close** — that would be UB, not a leak).
- `dup2(tty.as_raw_fd(), 1)`: `tty` is an owned `File` so the source fd is valid; **and the
  ownership consequence** — after success, fd 1 *is* a dup of `tty`, which is why `tty` is
  kept for the process lifetime and must outlive the render loop. This is the one with a
  real, subtle invariant; the comment must say it, because the `ReroutedStdout` doc
  currently carries that reasoning instead of the call site.
- `from_raw_fd(fd)`: takes **unique ownership** of a raw fd — state that it is not closed
  elsewhere (a double close is UB).

**The test sites need a real investigation, not a comment.** `set_var` is unsafe in edition
2024 because mutating the environment races with concurrent readers — and **cargo's test
harness runs test functions on threads by default**. So the honest question is whether
`IsolatedHome` is *actually* sound:

- If only one test ever constructs it, or the harness is serialized for it, the comment can
  say so **with the evidence**.
- If tests can touch the env concurrently, **this is a genuine unsoundness (P1), not a
  documentation gap** — report it as a finding with the test names involved and the
  mechanism, and propose the fix (a serializing mutex/`--test-threads=1` for that module, or
  a scoped env guard).
- **Do not write a `// SAFETY:` comment that claims a justification you have not
  verified.** A lying SAFETY comment is worse than none.

Note for the reviewer: `IsolatedHome` is also one of the duplicated hermeticity blocks that
`issue-git-test-harness.md` will consolidate (`GIT_CONFIG_GLOBAL` appears in
`git/{blame,repo/tests,log,refs,commit}.rs`). **If that lane lands first, rebase and
document whatever remains** rather than resurrecting a deleted copy.

## Part B — make the lints enforceable

- Add to the workspace `Cargo.toml`:
  ```toml
  [workspace.lints.rust]
  undocumented_unsafe_blocks = "deny"
  ```
  and `[lints] workspace = true` in the bin and each crate manifest. (`undocumented_unsafe_blocks`
  is a **rustc** lint, not clippy — it belongs in `[workspace.lints.rust]`.)
- Consider `unsafe_op_in_unsafe_fn = "deny"` too, if it is clean here.
- **Prove the lint actually fires** (gates must execute): with the lint in place, temporarily
  delete one `SAFETY:` comment, confirm `cargo clippy --workspace --all-targets -- -D warnings`
  **fails** with the expected lint name, then restore it. Report the observed output. An
  enforcement you never saw fail is theater.
- Read clippy's exit code via `${PIPESTATUS[0]}` (the standing rule).
- **Do NOT add `cargo fmt --check` to the gate or the hook as part of this task.** Measured on
  `main` (before any lane's changes): it reports **744 diff hunks** across the repo,
  including files in `crates/redline-resolve/` that the lane under review never touched. The
  installed `rustfmt` is **1.9.0-stable (2026-07-14)** — a newer style edition than the tree
  was formatted with — so the repo is fmt-dirty **environment-wide**. A fmt check added today
  would fail for reasons unrelated to any change and train everyone to bypass the hook. If
  formatting should be enforced, that is its own lane: either pin the toolchain (a
  `rust-toolchain.toml`) or do the one-off repo-wide `cargo fmt` and land it deliberately —
  **then** add the check. Record whichever you choose; do not silently skip it.

## Part C — a pre-push hook that runs the fast gate

- A **repo-tracked** hook at `.githooks/pre-push` running `tools/gate.sh fast` (never a PTY
  tier — the concurrency flake class), plus a documented install step:
  `git config core.hooksPath .githooks` (a tiny `tools/install-hooks.sh` and a README line).
  `.git/hooks/` is not versioned, so `core.hooksPath` is the only way this is shared.
- State the honest limits in the hook's own comment: it is **advisory** (`--no-verify`
  bypasses it), it only helps a clone that ran the install step, and `core.hooksPath` is
  repo config shared by worktrees — so a push from `.agents/worktrees/<n>` runs the hook in
  that worktree, which is what you want (it gates the branch being pushed).
- **Do not install it automatically from a build script** — that is intrusive and would
  surprise contributors.
- Keep the payload fast; if `fast` proves too slow in practice, say so rather than silently
  dropping a stage from it.

## Files

`src/main.rs`, `src/git/commit.rs`, `Cargo.toml` + `crates/*/Cargo.toml`, `.githooks/pre-push`,
`tools/install-hooks.sh`, `README.md`.

## Verification

- `cargo build`; `cargo test --workspace` — reconcile against the **current** baseline
  (measure it) and account for every change.
- `cargo clippy --workspace --all-targets -- -D warnings` (`${PIPESTATUS[0]}`), including the
  **negative test** in Part B with its observed output.
- `timeout 900 tools/gate.sh fast` — must pass; time it and report the wall clock (that is
  the hook's cost).
- Exercise the hook for real: install it, push a throwaway branch (or run the hook script
  directly with the pre-push args), and confirm it **fails** when the gate fails (e.g. a
  deliberate `unused` variable) and passes when clean. Report both observations.
- **`timeout 900 tools/gate.sh full`** — the lint table and the `main.rs` comments touch
  production paths, so run the battery (defer only with an explicit reason; note the other
  in-flight lanes).
- Report: each SAFETY comment and the invariant it states, **the `set_var` verdict with its
  evidence**, the observed lint-fires output, the hook's measured cost and the fail/pass
  observations, and the gate output.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.

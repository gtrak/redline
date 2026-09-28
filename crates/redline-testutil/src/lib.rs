//! The shared git test harness, promoted from the bin's `src/test_support.rs`
//! (plan 014 stage 2) and now a dev-dependency of every crate whose tests
//! shell out to git.
//!
//! Single source of truth for running the git CLI hermetically in tests:
//! the hermetic env block is stated ONCE here, and the former copies in
//! `crates/redline-git/src/{blame,commit,log,refs,repo/tests}.rs`,
//! the bin's `ui/magit_status.rs`, `ui/rows_view.rs`, `app/flow_tests.rs`
//! and `app/store/tests/mod.rs` all delegate to it.
//!
//! Hermeticity: `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM` point at
//! `/dev/null`, so a developer's `~/.gitconfig` (identity,
//! `commit.gpgsign`, aliases, hooks) can never leak in; the
//! author/committer identity is a parameter per call. The fixture
//! identities are real and deliberately NOT collapsed: the standard
//! `"Test"/"test@example.com"`, the magit windowing test's short
//! `"T"/"t@e.com"`, and the windowing fixture's mixed `"Test"/"t@e.com"`.
//!
//! **The env-mutation lock lives in this crate (plan 014 stage 2), not in
//! any dependent.** `ENV_LOCK` (below) is the ONE lock every test that
//! mutates process-global environment holds while its env guard is live.
//! Soundness of the split: each test BINARY is a separate PROCESS with its
//! own environment and its own copy of this static, so in-process
//! serialization is preserved — every env-mutating test in a given binary
//! (the bin's, `redline-git`'s) shares THAT binary's one copy — and no
//! cross-process lock is needed or possible, because process environment
//! never crosses process boundaries (the git CLI harness below sets vars
//! per-child via `Command::env`, never in-process).
//!
//! **The harness helpers take NO `ENV_LOCK` — and a future change must not
//! add one.** These vars are set on `std::process::Command` (a per-child
//! `envp`), which never mutates this process's environment: there is no
//! shared state, so there is no race for the env-mutation lock to guard.
//! Locking here would be worse than useless — `redline-git`'s `commit.rs`
//! `EnvScope` oracle tests already hold `ENV_LOCK` for their whole body
//! and call into this helper inside it, and a non-reentrant lock would
//! deadlock every one of them.

use std::path::Path;
use std::process::Command;

/// The ONE lock every test that mutates process-global environment holds
/// while its env guard is live (moved from the bin's `main.rs` with the
/// `redline-git` extraction, plan 014 stage 2). See the module doc for the
/// process-level soundness argument.
pub static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Run `git <args>` in `dir` with the hermetic env block and the fixture
/// identity, asserting success (the store-dedup helper's assertion text,
/// kept verbatim), and return stdout.
pub fn git_cli(dir: &Path, args: &[&str], name: &str, email: &str) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", name)
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_COMMITTER_NAME", name)
        .env("GIT_COMMITTER_EMAIL", email)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The shared repo-priming sequence (init + identity config).
/// `gpgsign_false` mirrors which callers actually set `commit.gpgsign`
/// (`false` = don't — the magit windowing test never did, so it stays
/// honest to its original body).
pub fn git_repo_init(dir: &Path, name: &str, email: &str, gpgsign_false: bool) {
    git_cli(dir, &["init", "-q", "-b", "main"], name, email);
    git_cli(dir, &["config", "user.name", name], name, email);
    git_cli(dir, &["config", "user.email", email], name, email);
    if gpgsign_false {
        git_cli(dir, &["config", "commit.gpgsign", "false"], name, email);
    }
}

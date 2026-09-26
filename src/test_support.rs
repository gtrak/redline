//! The crate-wide git test harness, shared by every `#[cfg(test)]` module.
//!
//! Single source of truth for running the git CLI hermetically in tests:
//! the hermetic env block is stated ONCE here, and the former copies in
//! `git/blame.rs`, `git/log.rs`, `git/refs.rs`, `git/repo/tests.rs`,
//! `git/commit.rs`, `ui/magit_status.rs`, `ui/rows_view.rs`,
//! `app/flow_tests.rs` and `app/store/tests/mod.rs` all delegate to it.
//!
//! Hermeticity: `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM` point at
//! `/dev/null`, so a developer's `~/.gitconfig` (identity,
//! `commit.gpgsign`, aliases, hooks) can never leak in; the
//! author/committer identity is a parameter per call. The fixture
//! identities are real and deliberately NOT collapsed: the standard
//! `"Test"/"test@example.com"`, the magit windowing test's short
//! `"T"/"t@e.com"`, and the windowing fixture's mixed `"Test"/"t@e.com"`.
//!
//! **No `crate::ENV_LOCK` is taken here — and a future change must not
//! add one.** These vars are set on `std::process::Command` (a per-child
//! `envp`), which never mutates this process's environment: there is no
//! shared state, so there is no race for the env-mutation lock to guard.
//! Locking here would be worse than useless — `git/commit.rs`'s
//! `EnvScope` oracle tests already hold `ENV_LOCK` for their whole body
//! and call into this helper inside it, and a non-reentrant lock would
//! deadlock every one of them.
//!
//! Kept free of app types on purpose (plan 014): when `redline-git` is
//! extracted, this module is promoted to `crates/redline-testutil` as a
//! dev-dependency — a move, not a rewrite.

use std::path::Path;
use std::process::Command;

/// Run `git <args>` in `dir` with the hermetic env block and the fixture
/// identity, asserting success (the store-dedup helper's assertion text,
/// kept verbatim), and return stdout.
pub(crate) fn git_cli(dir: &Path, args: &[&str], name: &str, email: &str) -> String {
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
pub(crate) fn git_repo_init(dir: &Path, name: &str, email: &str, gpgsign_false: bool) {
    git_cli(dir, &["init", "-q", "-b", "main"], name, email);
    git_cli(dir, &["config", "user.name", name], name, email);
    git_cli(dir, &["config", "user.email", email], name, email);
    if gpgsign_false {
        git_cli(dir, &["config", "commit.gpgsign", "false"], name, email);
    }
}

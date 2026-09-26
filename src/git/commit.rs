//! Commit creation (issue 08). git2 types stay in this module; the store
//! calls `GitRepo::commit` after the commit editor finishes.
//!
//! Identity resolution is delegated to git itself (`git var`), so the
//! author/committer identities always equal what `git commit` (and magit,
//! which shells out) would record in the same environment.

use std::path::PathBuf;

use crate::git::error::GitError;
use crate::git::repo::GitRepo;

impl GitRepo {
    /// Create a commit of the current index (the staged changes) with
    /// `message`, updating HEAD (the current branch ref). The author and
    /// committer identities are resolved exactly the way git does it, by
    /// asking git: `git var GIT_AUTHOR_IDENT` / `git var GIT_COMMITTER_IDENT`
    /// (the same delegation magit performs). That runs git's full chain —
    /// `user.name`/`user.email` from the config stack (repo → global →
    /// system), then `GIT_AUTHOR_*` / `GIT_COMMITTER_*`, then the
    /// `EMAIL`-env / `user@hostname` invention — and honours
    /// `user.useConfigOnly`. Delegating (instead of reading config through
    /// libgit2) means redline cannot drift from git, and the config files
    /// are re-resolved from the live process environment on every commit
    /// (a libgit2 config snapshot is taken at repo open, which is how a
    /// differing `$HOME` or a stale global-config path went unnoticed).
    ///
    /// When git itself cannot resolve an identity — the honest failure it
    /// produces, e.g. `user.useConfigOnly` with nothing configured — this
    /// fails with a self-diagnosing [`GitError::NoAuthor`].
    ///
    /// Follows the verified git2 0.21.0 recipe: `index.write_tree` for the
    /// tree, HEAD for the parent(s), and `Repository::commit(Some("HEAD"),
    /// …)` to fold the branch-ref update into the commit. On an unborn
    /// branch (no HEAD) the parent list is empty and the commit creates the
    /// branch ref. Returns the new commit's full 40-hex oid.
    pub fn commit(&self, message: &str) -> Result<String, GitError> {
        let author = self.identity("GIT_AUTHOR_IDENT", "author")?;
        let committer = self.identity("GIT_COMMITTER_IDENT", "committer")?;

        let mut index = self.inner.index()?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = self.inner.find_tree(tree_id)?;

        let parents: Vec<git2::Commit> = match self.inner.head() {
            Ok(ref_) => match ref_.target() {
                Some(oid) => vec![self.inner.find_commit(oid)?],
                None => vec![],
            },
            Err(e) if e.code() == git2::ErrorCode::NotFound => vec![],
            Err(e) => return Err(e.into()),
        };
        let parents_ref: Vec<&git2::Commit> = parents.iter().collect();

        let oid = self
            .inner
            .commit(Some("HEAD"), &author, &committer, message, &tree, &parents_ref)?;
        Ok(oid.to_string())
    }

    /// `git var <ident>` in the repo directory, parsed into a `Signature`
    /// carrying the timestamp git reported (so the recorded ident is exactly
    /// git's). `git var` is an identity command: it only needs the git
    /// directory, so bare and disabled-workdir repos work too.
    fn identity(&self, ident: &str, role: &str) -> Result<git2::Signature<'static>, GitError> {
        // gate P2-2: use the GITDIR for a bare repo. `workdir()` is `None`
        // there, and the old fallback to `path().parent()` pointed `-C` at the
        // bare repo's PARENT — so git never read the repo's own config and
        // recorded a different identity than `git var` run in the gitdir does
        // (measured: bare clone recorded the global identity while `git var`
        // in the gitdir returned the repo's local one). The gitdir itself is
        // the right cwd, and `unwrap_or_else(path)` was already unreachable
        // behind that `.parent()`.
        let cwd = self
            .inner
            .workdir()
            .unwrap_or_else(|| self.inner.path());
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(cwd)
            .arg("var")
            .arg(ident)
            .output()
            .map_err(GitError::IdentityCommand)?;
        if !out.status.success() {
            return Err(GitError::NoAuthor {
                detail: no_author_detail(
                    cwd,
                    self.inner.path().join("config"),
                    role,
                    &out.stderr,
                ),
            });
        }
        parse_ident(&String::from_utf8_lossy(&out.stdout), ident)
    }
}

/// `git var` output `Name <email> <unix-ts> ±hhmm` → a `Signature` with
/// that exact name, email, and timestamp.
fn parse_ident(stdout: &str, ident: &str) -> Result<git2::Signature<'static>, GitError> {
    let line = stdout.trim();
    let unparseable = || GitError::IdentityUnparseable(format!("`git var {ident}` output: {line}"));
    let (name, rest) = line.rsplit_once('<').ok_or_else(unparseable)?;
    let (email, tail) = rest.rsplit_once('>').ok_or_else(unparseable)?;
    let (ts, offset) = tail
        .trim()
        .split_once(' ')
        .ok_or_else(unparseable)?;
    let timestamp = ts
        .parse()
        .map_err(|_| GitError::IdentityUnparseable(line.to_string()))?;
    // gate P1 (BLOCKING): `git2::Time::new`'s second argument is an offset in
    // **MINUTES**, and `git var` prints **±hhmm**. Feeding the digits through
    // unchanged turned `-0400` into -400 minutes (-6h40m) — a silent wrong
    // offset on every non-UTC box, and a REGRESSION: the previous
    // `Signature::now` used the system offset correctly. Measured before this
    // fix: `-0400` → recorded `-0640`; `+0530` → `+0850`; `+0930` → `+1530`,
    // with the whole 953-test suite green (nothing observed the offset).
    let (sign, digits) = match offset.as_bytes().first() {
        Some(b'-') => (-1i32, &offset[1..]),
        Some(b'+') => (1i32, &offset[1..]),
        _ => return Err(unparseable()),
    };
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(unparseable());
    }
    let (hh, mm) = digits.split_at(2);
    let minutes = sign
        * (hh.parse::<i32>().map_err(|_| unparseable())? * 60
            + mm.parse::<i32>().map_err(|_| unparseable())?);
    let time = git2::Time::new(timestamp, minutes);
    git2::Signature::new(name, email, &time).map_err(GitError::Git)
}

/// The self-diagnosing text for [`GitError::NoAuthor`]: distinguishes
/// `user.useConfigOnly` forbidding the invented-identity fallback from a
/// plain "git could not resolve anything in this environment", quotes
/// git's own error, and names the config files this environment consults
/// plus the identity env vars that are (not) set.
fn no_author_detail(
    cwd: &std::path::Path,
    local_config: PathBuf,
    role: &str,
    git_stderr: &[u8],
) -> String {
    let git_msg = String::from_utf8_lossy(git_stderr)
        .lines()
        .find(|l| l.starts_with("fatal:"))
        .unwrap_or("git var failed")
        .trim()
        .to_string();

    // Failure path only: does the environment set user.useConfigOnly?
    let use_config_only = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .arg("config")
        .arg("user.useConfigOnly")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false);

    let files = config_files_consulted(local_config)
        .into_iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let env_state = identity_env_state();

    if use_config_only {
        format!(
            "user.useConfigOnly is set, so git refuses to fall back to an invented identity. \
             Set user.name and user.email in one of: {files} (or unset user.useConfigOnly). \
             git said: {git_msg}"
        )
    } else {
        format!(
            "git could not resolve the {role} identity in this environment: no user.name/user.email \
             in the config chain and no usable fallback. git said: {git_msg}. Config files \
             consulted (local → global → system): {files}. {env_state}"
        )
    }
}

/// The config files git consults in this process's environment, in
/// precedence order: repo-local (the opened repo's config file), then
/// `$XDG_CONFIG_HOME/git/config` (or `$HOME/.config/git/config`), then
/// global (`GIT_CONFIG_GLOBAL`, or `$HOME/.gitconfig`), then system
/// (`GIT_CONFIG_SYSTEM`, or the compile-time default `/etc/gitconfig`
/// unless `GIT_CONFIG_NOSYSTEM` is set to a true value).
fn config_files_consulted(local_config: PathBuf) -> Vec<PathBuf> {
    let mut files = vec![local_config];
    match std::env::var_os("GIT_CONFIG_GLOBAL") {
        Some(p) => files.push(p.into()),
        None => {
            // gate P2-4: git ALSO reads the XDG config, and this list omitted
            // it — proven with `git config --show-origin` under a probe
            // XDG_CONFIG_HOME. It precedes `$HOME/.gitconfig`.
            let xdg = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
            if let Some(xdg) = xdg {
                files.push(xdg.join("git").join("config"));
            }
            if let Some(home) = std::env::var_os("HOME") {
                files.push(std::path::PathBuf::from(home).join(".gitconfig"));
            }
        }
    }
    // gate P2-4: git treats an EMPTY GIT_CONFIG_NOSYSTEM as unset (its
    // `git_env_bool` semantics); `Some(v) if v != "false" && v != "0"` treated
    // `""` as set, so the message dropped a system file git really did read.
    let nosystem = matches!(
        std::env::var_os("GIT_CONFIG_NOSYSTEM"),
        Some(v) if !v.is_empty() && v != "false" && v != "0"
    );
    if !nosystem {
        match std::env::var_os("GIT_CONFIG_SYSTEM") {
            Some(p) => files.push(p.into()),
            None => files.push(PathBuf::from("/etc/gitconfig")),
        }
    }
    files
}

/// Which identity-relevant environment variables are set in this process
/// (names only — the values are the user's identity).
fn identity_env_state() -> String {
    const VARS: [&str; 5] = [
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
        "EMAIL",
    ];
    let set: Vec<&str> = VARS
        .iter()
        .filter(|v| std::env::var_os(v).is_some())
        .copied()
        .collect();
    if set.is_empty() {
        "None of GIT_AUTHOR_NAME, GIT_AUTHOR_EMAIL, GIT_COMMITTER_NAME, \
         GIT_COMMITTER_EMAIL, EMAIL is set in this process's environment."
            .to_string()
    } else {
        format!("Set identity env vars: {}.", set.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            // The git CLI (used only for setup/verification commits) needs an
            // author. Set per-child so they never reach the process env that
            // the wrapper's own `git var` subprocesses inherit — the oracle
            // scenario's env is whatever the test's EnvScope pins.
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn init_repo(dir: &Path, with_author: bool) -> GitRepo {
        git(dir, &["init", "-q", "-b", "main"]);
        if with_author {
            git(dir, &["config", "user.name", "Test"]);
            git(dir, &["config", "user.email", "test@example.com"]);
        }
        git(dir, &["config", "commit.gpgsign", "false"]);
        GitRepo::discover(dir).expect("discover the repo")
    }

    /// Oracle: `git var <var>` in the CURRENT process environment — the same
    /// environment redline's `commit` subprocess sees. `None` when git
    /// itself cannot resolve the identity.
    fn git_var(dir: &Path, var: &str) -> Option<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .arg("var")
            .arg(var)
            .output()
            .expect("run git var");
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// `git var` output `Name <email> <ts> ±hhmm` → `(name, email)`. The
    /// name is right-trimmed: `git var` prints `Name <email>`, and libgit2
    /// (which records the signature) trims that separator space.
    fn ident_name_email(ident: &str) -> (&str, &str) {
        let (name, rest) = ident.rsplit_once('<').expect("oracle ident: {ident}");
        let (email, _) = rest.rsplit_once('>').expect("oracle ident: {ident}");
        (name.trim_end(), email)
    }

    /// The last commit's `(author name, author email, committer name,
    /// committer email)`.
    fn last_author_committer(root: &Path) -> (String, String, String, String) {
        let log = git(root, &["log", "-1", "--pretty=%an%n%ae%n%cn%n%ce"]);
        let mut it = log.lines();
        (
            it.next().expect("author name").to_string(),
            it.next().expect("author email").to_string(),
            it.next().expect("committer name").to_string(),
            it.next().expect("committer email").to_string(),
        )
    }

    /// Stage a change to `a.txt` (already committed once) so a commit has
    /// something to record.
    fn stage_change(root: &Path, g: &GitRepo) {
        std::fs::write(root.join("a.txt"), "a\nA\n").unwrap();
        g.stage_file("a.txt").unwrap();
    }

    #[test]
    fn commit_moves_head_and_records_message_and_author() {
        // The wrapper's identity subprocesses inherit the process env, so
        // pin it (ENV_LOCK + EnvScope): local `user.*` = the only identity
        // level in play, exactly as this test has always meant.
        let _lock = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _scope = EnvScope::apply();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root, true);

        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(root, &["add", "a.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);
        let head_before = git(root, &["rev-parse", "HEAD"]).trim().to_string();

        // Stage a change, then commit through the wrapper.
        std::fs::write(root.join("a.txt"), "a\nA\n").unwrap();
        g.stage_file("a.txt").unwrap();
        let oid = g.commit("the commit message").unwrap();

        // HEAD moved to the new commit, which is exactly the wrapper's oid.
        let head_after = git(root, &["rev-parse", "HEAD"]).trim().to_string();
        assert_ne!(head_before, head_after);
        assert_eq!(head_after, oid);

        // The message and author (from config) are recorded.
        let log = git(root, &["log", "-1", "--pretty=%s%n%an%n%ae"]);
        assert_eq!(
            log.trim(),
            "the commit message\nTest\ntest@example.com",
            "log:\n{log}"
        );
        // … and that author is exactly what git itself resolves here.
        let oracle = git_var(root, "GIT_AUTHOR_IDENT").expect("git resolves the local identity");
        assert_eq!(ident_name_email(&oracle), ("Test", "test@example.com"));

        // The tree is now clean (07's status wrapper reports no changes).
        let status = g.status().unwrap();
        assert_eq!(status.staged_count(), 0, "status after commit: {status:#?}");
        assert_eq!(status.unstaged_count(), 0);
        assert_eq!(status.untracked_count(), 0);
    }

    /// Process-level env guard for the oracle tests: pins `$HOME` to a
    /// scratch dir (whose `.gitconfig` the test writes), empties the system
    /// config, and removes every identity-relevant var, so each scenario's
    /// environment is exactly what the test says it is. Every touched var
    /// is saved and restored. Must be held under `crate::ENV_LOCK` (other
    /// threads reading env vars concurrently is UB; the same lock is held
    /// by `model::files`'s `EnvGuard`). In every user, the `ENV_LOCK`
    /// guard is acquired BEFORE the `EnvScope`, so the `Drop` restore
    /// also runs under the lock.
    struct EnvScope {
        home: std::path::PathBuf,
        prev: Vec<(String, Option<std::ffi::OsString>)>,
    }
    impl EnvScope {
        fn apply() -> Self {
            const REMOVE_VARS: [&str; 6] = [
                "GIT_CONFIG_GLOBAL",
                "GIT_AUTHOR_NAME",
                "GIT_AUTHOR_EMAIL",
                "GIT_COMMITTER_NAME",
                "GIT_COMMITTER_EMAIL",
                "EMAIL",
            ];
            let home = tempfile::tempdir().unwrap().keep();
            // Snapshot every touched var before touching anything. gate P2-3:
            // XDG_CONFIG_HOME is included because git's global search also
            // reads `$XDG_CONFIG_HOME/git/config`; leaving it ambient made two
            // tests FAIL (not skip) on a correct implementation whenever the
            // developer had that file — measured with a probe config.
            let prev: Vec<_> = std::iter::once("HOME")
                .chain(std::iter::once("XDG_CONFIG_HOME"))
                .chain(std::iter::once("GIT_CONFIG_SYSTEM"))
                .chain(REMOVE_VARS.iter().copied())
                .map(|v| (v.to_string(), std::env::var_os(v)))
                .collect();
            // SAFETY: `set_var`/`remove_var` are unsafe in edition 2024
            // because they race with ANY concurrent `env::var`/`var_os`
            // reader on another thread (the race is process-wide, not
            // per-variable). This block runs only while the caller's
            // `crate::ENV_LOCK` guard is live (acquired before the
            // `EnvScope` in all eight tests), and every env-mutating test
            // in the crate takes that same lock — so no concurrent
            // environment access can interleave.
            unsafe {
                std::env::set_var("HOME", &home);
                std::env::set_var("XDG_CONFIG_HOME", &home);
                std::env::set_var("GIT_CONFIG_SYSTEM", "/dev/null");
                for var in REMOVE_VARS {
                    std::env::remove_var(var);
                }
            }
            Self { home, prev }
        }
    }
    impl Drop for EnvScope {
        fn drop(&mut self) {
            // SAFETY: same invariant as `apply` — this `Drop` runs in the
            // calling test's body, where its `ENV_LOCK` guard (declared
            // before the `EnvScope`) is still live, so the restore cannot
            // race a concurrent environment reader.
            unsafe {
                for (var, saved) in &self.prev {
                    match saved {
                        Some(v) => std::env::set_var(var, v),
                        None => std::env::remove_var(var),
                    }
                }
            }
            // Keep the home dir alive until after the restore.
            let _ = &self.home;
        }
    }

    /// The user's reported case: a repo with NO local `user.*`, identity
    /// only in the (controlled) global `$HOME/.gitconfig`. The commit must
    /// succeed and record exactly what `git var` resolves in this
    /// environment.
    #[test]
    fn commit_global_only_identity_equals_git_var() {
        let _lock = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _scope = EnvScope::apply();
        std::fs::write(
            std::path::Path::new(&std::env::var_os("HOME").expect("HOME pinned by EnvScope"))
                .join(".gitconfig"),
            "[user]\n\tname = Global Guy\n\temail = global@example.com\n",
        )
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root, false); // no local user.*
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(root, &["add", "a.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);
        stage_change(root, &g);

        let oracle =
            git_var(root, "GIT_AUTHOR_IDENT").expect("git resolves the global-only identity");
        let committer_oracle =
            git_var(root, "GIT_COMMITTER_IDENT").expect("git resolves the committer");
        let (o_name, o_email) = ident_name_email(&oracle);
        let (c_name, c_email) = ident_name_email(&committer_oracle);

        let oid = g
            .commit("global only")
            .expect("global-only identity must commit (the user's case)");
        assert_eq!(git(root, &["rev-parse", "HEAD"]).trim(), oid);
        let (an, ae, cn, ce) = last_author_committer(root);
        assert_eq!(
            (an, ae),
            (o_name.to_string(), o_email.to_string()),
            "author must equal git var GIT_AUTHOR_IDENT"
        );
        assert_eq!(
            (cn, ce),
            (c_name.to_string(), c_email.to_string()),
            "committer must equal git var GIT_COMMITTER_IDENT"
        );
    }

    /// No `user.*` anywhere; the identity arrives only through
    /// `GIT_AUTHOR_*` / `GIT_COMMITTER_*`. Git honours both pairs
    /// independently — the recorded author and committer must each equal
    /// their own `git var` oracle (the old code used one config-read
    /// signature for both).
    #[test]
    fn commit_records_gits_own_offset_under_a_non_utc_tz() {
        // gate P1 (was BLOCKING). `git2::Time::new`'s second argument is an
        // offset in MINUTES while `git var` prints `±hhmm`, so the digits must
        // be converted: before the fix `-0400` was recorded as `-0640`. The
        // whole suite stayed green regardless, and this box is UTC — so the
        // test must set TZ itself, and must assert it is non-UTC (a UTC
        // fixture would pass against the broken code).
        // Asia/Kolkata is deliberate: +0530 catches "hh*60 + mm" vs "hhmm".
        let _guard = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _scope = EnvScope::apply();
        for tz in ["America/New_York", "Asia/Kolkata"] {
            // SAFETY: the test holds `crate::ENV_LOCK` for its whole body,
            // and every env-mutating test in the crate takes that lock —
            // no concurrent environment access can interleave.
            unsafe { std::env::set_var("TZ", tz) };
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let g = init_repo(root, true);
            std::fs::write(root.join("a.txt"), "a\n").unwrap();
            git(root, &["add", "a.txt"]);
            // An initial commit so HEAD exists: without it the wrapper takes the
            // unborn-branch path and `find_commit` on the new oid fails with
            // `reference 'refs/heads/main' not found` (which is how these two
            // tests first failed — a fixture omission, not a product bug).
            git(root, &["commit", "-q", "-m", "init"]);
            std::fs::write(root.join("a.txt"), "a\nA\n").unwrap();
            g.stage_file("a.txt").unwrap();
            let oracle = git(root, &["var", "GIT_AUTHOR_IDENT"]);
            let off = oracle.split_whitespace().last().unwrap().to_string();
            let want: i32 = {
                let (sign, digits) = match off.as_bytes().first() {
                    Some(b'-') => (-1, &off[1..]),
                    _ => (1, off.strip_prefix('+').unwrap_or(&off)),
                };
                sign * (digits[..2].parse::<i32>().unwrap() * 60
                    + digits[2..].parse::<i32>().unwrap())
            };
            assert_ne!(want, 0, "TZ={tz}: fixture must be non-UTC or this proves nothing");
            let oid: git2::Oid = g.commit("tz").unwrap().parse().unwrap();
            let recorded = g.inner.find_commit(oid).unwrap().author().when().offset_minutes();
            assert_eq!(
                recorded, want,
                "TZ={tz}: recorded offset must be git's ({off}), not the digits reinterpreted"
            );
            // SAFETY: as above — the `ENV_LOCK` guard spans the whole
            // loop, so removing `TZ` cannot race a concurrent reader.
            unsafe { std::env::remove_var("TZ") };
        }
    }

    #[test]
    fn identity_is_resolved_per_commit_not_cached() {
        // gate P2-1: the ENTIRE value of delegating to `git var` over libgit2's
        // config snapshot is that git re-resolves every time — a cached
        // resolution is what made the reporter's identity invisible. Memoizing
        // it (per (ident, cwd), process lifetime) left all 953 tests green, so
        // the fix's headline property had no pin at all.
        let _guard = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _scope = EnvScope::apply();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root, true);
        let who = |oid: &str| {
            let oid: git2::Oid = oid.parse().unwrap();
            let c = g.inner.find_commit(oid).unwrap();
            (c.author().name().unwrap().to_string(), c.committer().name().unwrap().to_string())
        };
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(root, &["add", "a.txt"]);
        // HEAD must exist before the wrapper commits (see the TZ test).
        git(root, &["commit", "-q", "-m", "init"]);
        assert_eq!(who(&g.commit("one").unwrap()).0, "Test");
        // A config change between two commits on the SAME handle must be seen.
        git(root, &["config", "user.name", "Second Name"]);
        git(root, &["config", "user.email", "second@example.com"]);
        std::fs::write(root.join("a.txt"), "a\nB\n").unwrap();
        g.stage_file("a.txt").unwrap();
        assert_eq!(
            who(&g.commit("two").unwrap()),
            ("Second Name".to_string(), "Second Name".to_string()),
            "a config change between commits must be picked up (no caching)"
        );
        // The env layer too — a level the old code never read at all — and it
        // must move the AUTHOR only (independence).
        // SAFETY: the test holds `crate::ENV_LOCK` for its whole body;
        // every env-mutating test in the crate takes that same lock.
        unsafe { std::env::set_var("GIT_AUTHOR_NAME", "Env Three") };
        // SAFETY: same as the line above (the lock guard is unchanged).
        unsafe { std::env::set_var("GIT_AUTHOR_EMAIL", "three@example.com") };
        std::fs::write(root.join("a.txt"), "a\nB\nC\n").unwrap();
        g.stage_file("a.txt").unwrap();
        assert_eq!(
            who(&g.commit("three").unwrap()),
            ("Env Three".to_string(), "Second Name".to_string()),
            "GIT_AUTHOR_* moves the author, the config still supplies the committer"
        );
    }

    #[test]
    fn commit_env_identities_equal_git_var() {
        let _lock = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _scope = EnvScope::apply();
        // SAFETY: the test holds `crate::ENV_LOCK` for its whole body;
        // every env-mutating test in the crate takes that same lock.
        unsafe {
            std::env::set_var("GIT_AUTHOR_NAME", "Env Author");
            std::env::set_var("GIT_AUTHOR_EMAIL", "env@example.com");
            std::env::set_var("GIT_COMMITTER_NAME", "Env Committer");
            std::env::set_var("GIT_COMMITTER_EMAIL", "envc@example.com");
        }

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root, false); // no user.* in any config
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(root, &["add", "a.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);
        stage_change(root, &g);

        let oracle_author =
            git_var(root, "GIT_AUTHOR_IDENT").expect("git honours GIT_AUTHOR_*");
        let oracle_committer =
            git_var(root, "GIT_COMMITTER_IDENT").expect("git honours GIT_COMMITTER_*");
        let (a_name, a_email) = ident_name_email(&oracle_author);
        let (c_name, c_email) = ident_name_email(&oracle_committer);
        assert_ne!(
            (a_name, a_email),
            (c_name, c_email),
            "scenario sanity: the two oracles must differ"
        );

        let oid = g.commit("from env").expect("env-only identity must commit");
        assert_eq!(git(root, &["rev-parse", "HEAD"]).trim(), oid);
        let (an, ae, cn, ce) = last_author_committer(root);
        assert_eq!(
            (an, ae),
            (a_name.to_string(), a_email.to_string()),
            "author must equal git var GIT_AUTHOR_IDENT"
        );
        assert_eq!(
            (cn, ce),
            (c_name.to_string(), c_email.to_string()),
            "committer must equal git var GIT_COMMITTER_IDENT"
        );
    }

    /// Nothing configured and no `GIT_*` env: git invents an identity (the
    /// account name here, `EMAIL` for the email). Redline must record the
    /// SAME invented identity git resolves — not refuse it.
    #[test]
    fn commit_invented_identity_equals_git_var() {
        let _lock = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _scope = EnvScope::apply();
        // SAFETY: the test holds `crate::ENV_LOCK` for its whole body;
        // every env-mutating test in the crate takes that same lock.
        unsafe {
            std::env::set_var("EMAIL", "invented@example.com");
        }

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root, false); // no user.* in any config
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(root, &["add", "a.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);
        stage_change(root, &g);

        let oracle =
            git_var(root, "GIT_AUTHOR_IDENT").expect("git resolves the EMAIL-env identity");
        let (o_name, o_email) = ident_name_email(&oracle);
        assert_eq!(o_email, "invented@example.com");

        let oid = g.commit("invented").expect("invented identity must commit");
        assert_eq!(git(root, &["rev-parse", "HEAD"]).trim(), oid);
        let (an, ae, _, _) = last_author_committer(root);
        assert_eq!(
            (an, ae),
            (o_name.to_string(), o_email.to_string()),
            "the invented identity must equal git var GIT_AUTHOR_IDENT"
        );
    }

    /// No `user.*` in any config, no identity env vars at all. Whatever git
    /// does here, redline must do the same: on a box where git can invent
    /// an identity (dotted hostname) the commit records it; where git
    /// cannot, redline fails `NoAuthor` with a self-diagnosing message
    /// that names the config files consulted.
    #[test]
    fn commit_identity_agrees_with_git_when_nothing_configured() {
        let _lock = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _scope = EnvScope::apply(); // no .gitconfig in the pinned HOME

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root, false);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(root, &["add", "a.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);
        stage_change(root, &g);

        match git_var(root, "GIT_AUTHOR_IDENT") {
            Some(oracle) => {
                let (o_name, o_email) = ident_name_email(&oracle);
                let oid = g
                    .commit("invented")
                    .expect("git invented an identity; redline must too");
                assert_eq!(git(root, &["rev-parse", "HEAD"]).trim(), oid);
                let (an, ae, _, _) = last_author_committer(root);
                assert_eq!((an, ae), (o_name.to_string(), o_email.to_string()));
            }
            None => {
                let err = g.commit("should not commit").unwrap_err();
                let GitError::NoAuthor { detail } = err else {
                    panic!("expected NoAuthor, got {err:?}")
                };
                // Self-diagnosing: names what was consulted and what git said.
                assert!(detail.contains(".gitconfig"), "detail: {detail}");
                assert!(detail.contains("git said:"), "detail: {detail}");
                assert!(detail.contains("GIT_AUTHOR_NAME"), "detail: {detail}");
                // HEAD did not move.
                assert_eq!(git(root, &["log", "-1", "--pretty=%s"]).trim(), "init");
            }
        }
    }

    /// `user.useConfigOnly = true` with nothing configured: git's explicit
    /// way to demand a real identity. Git itself fails here (`git var`
    /// errors) and redline must fail with `NoAuthor` whose message
    /// distinguishes the useConfigOnly refusal from a plain miss.
    /// (Re-expressed `commit_without_author_fails`: "no config" is no
    /// longer an error in git — this is now the honest-failure pin.)
    #[test]
    fn commit_fails_when_use_config_only() {
        let _lock = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _scope = EnvScope::apply();
        std::fs::write(
            std::path::Path::new(&std::env::var_os("HOME").expect("HOME pinned by EnvScope"))
                .join(".gitconfig"),
            "[user]\n\tuseConfigOnly = true\n",
        )
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root, false);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(root, &["add", "a.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);
        stage_change(root, &g);

        // The oracle agrees: git itself cannot resolve an identity here.
        assert!(
            git_var(root, "GIT_AUTHOR_IDENT").is_none(),
            "git must also fail under useConfigOnly"
        );

        let err = g.commit("should not commit").unwrap_err();
        let GitError::NoAuthor { detail } = err else {
            panic!("expected NoAuthor, got {err:?}")
        };
        assert!(
            detail.contains("useConfigOnly"),
            "the message must name the useConfigOnly refusal: {detail}"
        );
        assert!(
            detail.contains(".gitconfig"),
            "the message must name the config files consulted: {detail}"
        );
        // HEAD did not move.
        assert_eq!(
            git(root, &["log", "-1", "--pretty=%s"]).trim(),
            "init",
            "an uncommitted staged change must not create a commit"
        );
    }
}

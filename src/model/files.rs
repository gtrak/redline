//! Per-project file lists: an `ignore`-crate walk of the project root
//! (respects `.gitignore`, skips hidden files/dirs, never descends into
//! `.git`), cached in memory per project root. The walk runs once per
//! project (measured ~20ms for a 10k-file repo, debug build); the
//! `re-walk` command invalidates the cache (live refresh via the file
//! watcher in `src/app/watcher.rs`).
//!
//! `.gitignore` handling has two modes (verified against the ignore
//! crate 0.4.33): inside a git repo the crate applies the ignore
//! sources natively — `.gitignore`, `.git/info/exclude`, and the global
//! git excludes (`IgnoreBuilder` defaults `git_ignore/git_exclude/
//! git_global: true, require_git: true`); for marker-only (non-git)
//! projects it applies **none** of them (`require_git` gates all of
//! them), so this module evaluates the per-path predicate
//! (`is_gitignored`) itself and filters the walk.
//!
//! `is_gitignored` is the shared decision of the marker-only walk and
//! the incremental index filter (`AppStore::indexable_changes`); the
//! search pipeline's parallel walk carries the memoized equivalent
//! (`git_ignored_by_ancestors`) — R3: the walkers agree. Behaviourally,
//! the filter reproduces the walk for marker-only trees (`.gitignore`
//! chain + `hidden(true)`), and in git repos for the sources above
//! (`.gitignore` chain + `.git/info/exclude` + global excludes +
//! `hidden(true)`). The one remaining divergence is the pre-existing
//! cross-level negation precedence (a deeper `!pat` cannot rescue a
//! path a shallower `.gitignore` ignores — not real git semantics, and
//! shared by all three walkers).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::WalkBuilder;

/// A cached file list for one project root.
pub struct FileList {
    /// Project-relative file paths (slash-separated), sorted.
    pub files: Vec<String>,
}

impl FileList {
    /// Walk `root` with the `ignore` crate and collect regular files
    /// as relative paths. Gitignore patterns are honored in git and
    /// non-git projects alike; hidden entries are skipped.
    pub fn build(root: &Path) -> std::io::Result<Self> {
        if !root.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "project root is not a directory",
            ));
        }
        let is_git_repo = root.join(".git").exists();
        let mut builder = WalkBuilder::new(root);
        builder
            .hidden(true) // skip dotfiles/dotdirs
            .git_ignore(is_git_repo); // native gitignore handling in git repos
        if is_git_repo {
            // Native gitignore (above); the only extra prune is the
            // `graft` cache directory (issue 05, finding 3): graft cache
            // cards are agent output, not project source, and must not
            // pollute the file finder or the tree sidebar (the tree
            // inherits the fix from this same walk). Grep (the search
            // pipeline, gitignore-based) keeps its own walk and still
            // sees graft/ (the documented asymmetry, R3 item 2).
            let root_owned = root.to_path_buf();
            builder.filter_entry(move |entry| {
                let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
                !under_graft(&root_owned, entry.path(), is_dir)
            });
        } else {
            // Marker-only (non-git) project: the ignore crate applies
            // **none** of the ignore sources at all (`require_git`), so
            // filter through the shared per-path predicate
            // (`is_gitignored_memo` + `under_graft`) — the SAME decisions
            // the incremental index filter and the search pipeline make
            // (R3: one source of truth).
            let root_owned = root.to_path_buf();
            // One memo for the whole walk (F2): each directory's
            // `.gitignore` is parsed at most once per walk, not once
            // per entry. Per-invocation on purpose: `.gitignore` files
            // change while the app runs, and a persistent cache would
            // serve stale verdicts with no invalidation path. Both are
            // moved into the `filter_entry` closure (it must be 'static;
            // the memo is consumed by the walk, as in the search
            // pipeline's `filter_entry` move).
            let memo: GitignoreMemo = Mutex::new(HashMap::new());
            builder.filter_entry(move |entry| {
                let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
                !under_graft(&root_owned, entry.path(), is_dir)
                    && !is_gitignored(&root_owned, entry.path(), is_dir, &memo)
            });
        }
        let mut files = Vec::new();
        for entry in builder.build() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue, // unreadable entry: skip
            };
            if entry.file_type().is_none_or(|ft| !ft.is_file()) {
                continue;
            }
            let Some(rel) = entry.path().strip_prefix(root).ok() else {
                continue;
            };
            if rel.as_os_str().is_empty() {
                continue;
            }
            files.push(rel.to_string_lossy().into_owned());
        }
        files.sort();
        Ok(FileList { files })
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }
}

/// True when any DIRECTORY component of `path`'s project-relative path is
/// `graft` — i.e. `path` is the agent-cache directory itself or lies under
/// one (issue 05, finding 3). A *file* merely named `graft` (the final
/// component, not a directory) does not match, and paths outside `root`
/// never do. Shared by the file-list walk and the incremental index
/// filter; the search pipeline's gitignore-based walk intentionally does
/// NOT prune graft/ (the documented R3 item-2 asymmetry).
pub fn under_graft(root: &Path, path: &Path, is_dir: bool) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    let mut comps = rel.components();
    if !is_dir {
        // The final component may be a FILE name (not a directory);
        // only the directory components count.
        let _ = comps.next_back();
    }
    comps.any(|c| {
        matches!(c, std::path::Component::Normal(n) if n == "graft")
    })
}

/// A per-invocation memo of the ignore matchers: each directory's
/// `.gitignore` (keyed by that directory's path) and, for git repos,
/// the repo's `.git/info/exclude` and the process's global git excludes
/// (keyed by sentinel paths under `root/.git/info/`). A `None` value is
/// a cached "absent/unparseable" verdict, not just a matcher cache.
///
/// Scope: **per invocation** — created inside `FileList::build` (one
/// per walk) and inside `AppStore::indexable_changes` (one per change
/// batch), mirroring the search pipeline's memo (`src/search/rg.rs`).
/// Never a long-lived global: `.gitignore` files change while the app
/// runs, and a persistent cache would serve stale verdicts with no
/// invalidation path. Sharing one memo across a batch is what keeps
/// the incremental filter's I/O off the UI thread: each directory's
/// `.gitignore` is parsed at most once per batch, not once per
/// changed path.
pub type GitignoreMemo = Mutex<HashMap<PathBuf, Option<Gitignore>>>;

/// True when `path` (absolute, under `root`) is excluded from the
/// `FileList` walk for ignore reasons. Evaluates, in git precedence
/// order:
///
/// 1. any HIDDEN component of the project-relative path — the walk's
///    `hidden(true)` (F4: the incremental filter must not index what
///    the full build excluded);
/// 2. the `.gitignore` ANCESTOR CHAIN (the path's own directory for a
///    dir, its parent for a file, up to `root`), memoized per
///    directory in `memo`; each matcher is asked in the ancestor form
///    (`matched_path_or_any_parents`) — a directory rule like `gen/`
///    ignores the files beneath it, and a DIRECTORY is judged the same
///    way, so `build/sub` under a root `build/` rule is reported
///    ignored (the walk prunes `build` wholesale);
/// 3. git repos only (F1): `.git/info/exclude`, then the process's
///    global git excludes (`core.excludesFile`, `Gitignore::global`) —
///    the sources the walk's `IgnoreBuilder` defaults
///    (`git_exclude/git_global: true, require_git: true`) apply
///    natively. An explicit `.gitignore` match (either direction)
///    takes precedence over both: git reads `.gitignore` last.
///
/// Paths outside `root` and `root` itself are never reported ignored.
///
/// # Precondition
///
/// For a real candidate, `path` must be under `root` (otherwise the
/// early `strip_prefix` returns `false` and nothing is evaluated). The
/// chain matchers are rooted at `path`'s ancestor directories, so
/// `matched_path_or_any_parents` (which panics on a path outside its
/// matcher's root) is only ever called with paths under that root.
///
/// Known divergence (pre-existing in all three walkers, not fixed
/// here): cross-level negation precedence is not real git semantics —
/// a deeper `!pat` cannot rescue a path a shallower `.gitignore`
/// ignores; git gives the deepest matching rule the final word.
///
/// The shared decision for the file-list walk (non-git branch), the
/// incremental index filter (`AppStore::indexable_changes`), and any
/// other per-path consumer. The search pipeline's parallel walk keeps
/// its own memoized equivalent (`git_ignored_by_ancestors`) built on
/// the same `load_gitignore` + `gitignore_matches` primitives (R3).
pub fn is_gitignored(
    root: &Path,
    path: &Path,
    is_dir: bool,
    memo: &GitignoreMemo,
) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    if rel.as_os_str().is_empty() {
        return false; // the root itself is never ignored here
    }
    // (1) the walk's `hidden(true)`: a hidden component anywhere in the
    // project-relative path is never walked, so never indexed.
    if rel.components().any(|c| {
        matches!(
            c,
            std::path::Component::Normal(n) if n.to_string_lossy().starts_with('.')
        )
    }) {
        return true;
    }
    let mut map = memo.lock().unwrap();
    // (2) the `.gitignore` ancestor chain, deepest to root. An explicit
    // match in either direction is decisive: `.gitignore` outranks the
    // repo-level sources (step 3).
    let mut ignored = false;
    let mut explicit = false;
    let mut dir = if is_dir {
        Some(path.to_path_buf())
    } else {
        path.parent().map(|p| p.to_path_buf())
    };
    while let Some(d) = dir {
        // The matcher's root `d` is `path` itself (a dir) or an ancestor
        // of it, so the ancestor-form call below is always inside the
        // matcher's root (its precondition).
        let matcher = map
            .entry(d.clone())
            .or_insert_with(|| load_gitignore(&d));
        if let Some(gi) = matcher {
            let m = gi.matched_path_or_any_parents(path, is_dir);
            if m.is_ignore() {
                ignored = true;
            } else if m.is_whitelist() {
                explicit = true;
            }
        }
        if d == root {
            break; // the root's own `.gitignore` was the last check
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    if ignored {
        return true;
    }
    if explicit {
        return false; // `.gitignore` says "keep": the exclude sources cannot override
    }
    // (3) git repos only: `.git/info/exclude` (higher precedence) and
    // the global excludes. Marker-only trees have no `.git`, so the
    // walk (`require_git`) applies none of them there.
    if root.join(".git").exists() {
        let exclude_key = root.join(".git").join("info/exclude");
        // Sentinel key (not a real file) for the process-global matcher.
        let global_key = root.join(".git").join("info/global-excludes");
        // git precedence: `.git/info/exclude` is checked first; an
        // explicit verdict (either direction) decides, and the global
        // excludes are consulted only when exclude did not match.
        if let Some(keep) = {
            let exclude = map
                .entry(exclude_key.clone())
                .or_insert_with(|| load_excludes(root, &exclude_key));
            exclude
                .as_ref()
                .and_then(|gi| rel_decision(gi, rel, is_dir))
        } {
            return keep;
        }
        let global = map
            .entry(global_key.clone())
            .or_insert_with(load_global_excludes);
        return global
            .as_ref()
            .is_some_and(|gi| matches!(rel_decision(gi, rel, is_dir), Some(true)));
    }
    false
}

/// The decision a repo-level source (`.git/info/exclude` or the global
/// excludes) makes about `rel` — the project-relative path, exactly
/// what the walk feeds its `Ignore` matchers (entries are evaluated
/// relative to the walked root, so a root-anchored pattern like
/// `/build/` lands where git puts it). Ancestor form, because the walk
/// prunes an ignored directory wholesale: a directory rule (`build/`)
/// must drop the directory and everything under it. `Some(true)` =
/// ignore, `Some(false)` = explicit keep, `None` = no match.
fn rel_decision(gi: &Gitignore, rel: &Path, is_dir: bool) -> Option<bool> {
    let m = gi.matched(rel, is_dir);
    if !m.is_none() {
        return Some(m.is_ignore());
    }
    let mut parent = rel.parent();
    while let Some(p) = parent {
        let m = gi.matched(p, true);
        if !m.is_none() {
            return Some(m.is_ignore());
        }
        parent = p.parent();
    }
    None
}

/// Load `.git/info/exclude` as a matcher rooted at `root` — git
/// interprets it as the repo root's `.gitignore` (the `add` form with
/// the explicit root is the faithful one; `Gitignore::new` would root
/// it at `.git/info/`). `None` when absent or unparseable.
fn load_excludes(root: &Path, file: &Path) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(root);
    if builder.add(file).is_some() {
        return None;
    }
    builder.build().ok()
}

/// The process's global git excludes (`core.excludesFile`), resolved
/// from the git-config environment exactly as the walk's `IgnoreBuilder`
/// resolves them (`Gitignore::global`; the matcher's base is the
/// process cwd, and `rel_decision` matches against the project-relative
/// path — the same relative input the walk's matcher sees). `Some`
/// (possibly empty) when nothing is configured or the read fails: the
/// walk applies the same empty matcher, so caching "nothing to ignore"
/// keeps the two agreeing.
fn load_global_excludes() -> Option<Gitignore> {
    Some(Gitignore::global().0)
}

/// Load a `.gitignore` file from `dir` as a matcher. `None` when the
/// file is absent or unparseable. Shared by the file-list walk, the
/// incremental index filter, and the search pipeline (R3: all walkers
/// must agree on gitignore semantics).
pub(crate) fn load_gitignore(dir: &Path) -> Option<Gitignore> {
    match Gitignore::new(dir.join(".gitignore")) {
        (gi, None) => Some(gi),
        _ => None,
    }
}

/// True when `gi` ignores `path` (a directory when `is_dir`, a file
/// otherwise). The single shared gitignore decision used by the
/// file-list walk, the incremental index filter, and the search
/// pipeline.
pub(crate) fn gitignore_matches(gi: &Gitignore, path: &Path, is_dir: bool) -> bool {
    gi.matched(path, is_dir).is_ignore()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Single-path test shim: the predicate with a throwaway memo.
    /// Production callers always share one memo per invocation (one
    /// walk, one change batch — see `GitignoreMemo`).
    fn ignored(root: &Path, path: &Path, is_dir: bool) -> bool {
        let memo: GitignoreMemo = Mutex::new(HashMap::new());
        is_gitignored(root, path, is_dir, &memo)
    }

    fn file(path: impl AsRef<std::path::Path>, content: &str) {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        file(dir.path().join("Cargo.toml"), "[package]\n");
        dir
    }

    fn has(list: &FileList, rel: &str) -> bool {
        list.files.iter().any(|f| f == rel)
    }

    #[test]
    fn walk_collects_regular_files_as_relative_sorted_paths() {
        let dir = project();
        file(dir.path().join("README.md"), "hi\n");
        file(dir.path().join("src/main.rs"), "fn main() {}\n");
        file(dir.path().join("src/lib.rs"), "// lib\n");

        let list = FileList::build(dir.path()).unwrap();
        assert_eq!(
            list.files,
            vec!["Cargo.toml", "README.md", "src/lib.rs", "src/main.rs"]
        );
        assert!(has(&list, "src/main.rs"));
        assert!(!has(&list, "main.rs"));
    }

    #[test]
    fn walk_respects_gitignore_outside_git_repos() {
        let dir = project();
        fs::write(
            dir.path().join(".gitignore"),
            "ignored.txt\nbuild/\n*.log\n",
        )
        .unwrap();
        file(dir.path().join("kept.txt"), "k\n");
        file(dir.path().join("ignored.txt"), "i\n");
        file(dir.path().join("build/output.bin"), "b\n");
        file(dir.path().join("crash.log"), "l\n");
        file(dir.path().join("src/deep.rs"), "d\n");

        let list = FileList::build(dir.path()).unwrap();
        assert!(has(&list, "kept.txt"));
        assert!(has(&list, "src/deep.rs"));
        for absent in ["ignored.txt", "build/output.bin", "crash.log"] {
            assert!(!has(&list, absent), "`{absent}` must be ignored");
        }
    }

    #[test]
    fn walk_respects_nested_gitignore_outside_git_repos() {
        let dir = project();
        file(dir.path().join("src/keep.rs"), "k\n");
        file(dir.path().join("src/gen/out.rs"), "g\n");
        file(dir.path().join("src/gen/.gitignore"), "out.rs\n");
        file(dir.path().join("src/gen/visible.rs"), "v\n");

        let list = FileList::build(dir.path()).unwrap();
        assert!(has(&list, "src/keep.rs"));
        assert!(has(&list, "src/gen/visible.rs"));
        assert!(!has(&list, "src/gen/out.rs"), "nested .gitignore must apply");
    }

    #[test]
    fn walk_respects_gitignore_inside_git_repos() {
        let dir = project();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".gitignore"), "ignored.txt\nbuild/\n").unwrap();
        file(dir.path().join("kept.txt"), "k\n");
        file(dir.path().join("ignored.txt"), "i\n");
        file(dir.path().join("build/x.bin"), "b\n");

        let list = FileList::build(dir.path()).unwrap();
        assert!(has(&list, "kept.txt"));
        assert!(!has(&list, "ignored.txt"));
        assert!(!has(&list, "build/x.bin"));
    }

    #[test]
    fn walk_skips_hidden_files_and_dirs() {
        let dir = project();
        file(dir.path().join("visible.txt"), "v\n");
        file(dir.path().join(".dotfile"), "d\n");
        file(dir.path().join(".hidden/inside.txt"), "h\n");

        let list = FileList::build(dir.path()).unwrap();
        assert!(has(&list, "visible.txt"));
        assert!(!has(&list, ".dotfile"));
        assert!(!has(&list, ".hidden/inside.txt"));
    }

    #[test]
    fn walk_never_descends_into_git_dirs() {
        let dir = project();
        file(dir.path().join("src/a.rs"), "a\n");
        file(dir.path().join("vendor/.git/config"), "[core]\n");
        file(dir.path().join("top-level/.git/HEAD"), "ref: refs/heads/main\n");

        let list = FileList::build(dir.path()).unwrap();
        assert!(has(&list, "src/a.rs"));
        assert!(!list.files.iter().any(|f| f.contains(".git")));
    }

    #[test]
    fn build_requires_a_directory_root() {
        let dir = project();
        let not_dir = dir.path().join("Cargo.toml");
        assert!(FileList::build(&not_dir).is_err());
    }

    #[test]
    fn walk_prunes_graft_cache_directory() {
        // The graft cache (agent output) must not appear in the file walk
        // that feeds the finder and the tree sidebar (issue 05, finding 3).
        let dir = project();
        file(dir.path().join("src/main.rs"), "fn main() {}\n");
        file(dir.path().join("graft/src/main.md"), "# graft card\n");
        file(dir.path().join("graft/cache/other.md"), "cache\n");

        let list = FileList::build(dir.path()).unwrap();
        assert!(has(&list, "src/main.rs"));
        // No graft path may survive the walk (the whole subtree is pruned).
        assert!(
            !list.files.iter().any(|f| f == "graft/src/main.md" || f.starts_with("graft/")),
            "graft/ must be pruned from the file walk: {:?}",
            list.files
        );
    }

    #[test]
    fn walk_prunes_graft_directory_inside_git_repo() {
        let dir = project();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        file(dir.path().join("src/main.rs"), "fn main() {}\n");
        file(dir.path().join("graft/card.md"), "# graft\n");

        let list = FileList::build(dir.path()).unwrap();
        assert!(has(&list, "src/main.rs"));
        assert!(!has(&list, "graft/card.md"), "graft/ must be pruned");
    }

    /// The shared ancestor-chain predicate (R3 third walker): judged
    /// against the `.gitignore` chain from the root down to the path —
    /// a root-only check would pass the nested case, so every arm here
    /// must discriminate.
    #[test]
    fn is_gitignored_evaluates_the_full_ancestor_chain() {
        let dir = project();
        fs::write(
            dir.path().join(".gitignore"),
            "*.log\n!important.log\nbuild/\n",
        )
        .unwrap();
        file(dir.path().join("src/.gitignore"), "gen/\n");
        file(dir.path().join("src/gen/out.rs"), "g\n");
        file(dir.path().join("src/visible.rs"), "v\n");
        file(dir.path().join("build/out.bin"), "b\n");
        file(dir.path().join("crash.log"), "l\n");
        file(dir.path().join("important.log"), "i\n");
        file(dir.path().join("kept.txt"), "k\n");
        let root = dir.path();
        // Nested `.gitignore` (a root-only check would miss this).
        assert!(
            ignored(root, &root.join("src/gen/out.rs"), false),
            "nested .gitignore must apply"
        );
        assert!(!ignored(root, &root.join("src/visible.rs"), false));
        // Root `.gitignore` rules (file rule + directory rule).
        assert!(ignored(root, &root.join("crash.log"), false));
        assert!(ignored(root, &root.join("build/out.bin"), false));
        // Negation: `!important.log` un-ignores despite `*.log`.
        assert!(!ignored(root, &root.join("important.log"), false));
        // Plain non-ignored path.
        assert!(!ignored(root, &root.join("kept.txt"), false));
        // Directory form: judged with the SAME ancestor form as files
        // (P3: no separate plain-`matched` arm) — the walk prunes
        // `build` wholesale, so `build` and everything beneath it
        // (`build/sub`) are reported ignored by the root `build/` rule.
        assert!(ignored(root, &root.join("build"), true));
        assert!(
            ignored(root, &root.join("build/sub"), true),
            "a directory UNDER an ignored directory is ignored"
        );
        assert!(!ignored(root, &root.join("src"), true));
        // Out-of-root and the root itself are never "ignored" here.
        assert!(!ignored(root, Path::new("/elsewhere/x.rs"), false));
        assert!(!ignored(root, root, false));
    }

    /// Hermetic git-config environment for the global-excludes test:
    /// `GIT_CONFIG_GLOBAL` replaces `$HOME/.gitconfig` and the XDG file
    /// (git 2.32+), and the temp `HOME`/`XDG_CONFIG_HOME` guard the
    /// fallbacks. Restored on drop, the established pattern of the git
    /// test harness (process-global env change, accepted race window as
    /// in `git/commit.rs`'s `IsolatedHome`).
    struct EnvGuard(HashMap<String, Option<std::ffi::OsString>>);

    impl EnvGuard {
        fn set(pairs: &[(&str, &str)]) -> Self {
            let restore = pairs
                .iter()
                .map(|(k, _)| (k.to_string(), std::env::var_os(k)))
                .collect();
            for (k, v) in pairs {
                unsafe { std::env::set_var(k, v); }
            }
            Self(restore)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in self.0.drain() {
                match v {
                    Some(v) => unsafe { std::env::set_var(k, v); },
                    None => unsafe { std::env::remove_var(k); },
                }
            }
        }
    }

    /// (a) Git-repo agreement (F1): the walk's `IgnoreBuilder` defaults
    /// honour `.git/info/exclude` AND the global git excludes
    /// (`git_exclude/git_global: true, require_git: true`) — a file
    /// ignored SOLELY by those sources must not be re-indexed. The
    /// existing fixtures have no `.git`, so this is the behavioural
    /// arm: the same paths are fed to the walk and to the filter, and
    /// each source independently drops a distinct path (drop one and
    /// the assertion pins which source went missing).
    #[test]
    fn git_repo_walk_and_filter_agree_on_exclude_and_global() {
        let dir = project();
        let root = dir.path();
        // Repo marker + per-repo excludes (the gate's demonstrated case).
        fs::create_dir_all(root.join(".git/info")).unwrap();
        fs::write(
            root.join(".git/info/exclude"),
            "local/\nsecret.rs\n",
        )
        .unwrap();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        // Global excludes, hermetic (see `EnvGuard`).
        let home = tempfile::tempdir().unwrap();
        let global_ig = home.path().join("global-ignore");
        fs::write(&global_ig, "gignored.rs\n").unwrap();
        let gitconfig = home.path().join("gitconfig");
        fs::write(
            &gitconfig,
            format!("[core]\n\texcludesFile = {}\n", global_ig.display()),
        )
        .unwrap();
        let _env = EnvGuard::set(&[
            ("GIT_CONFIG_GLOBAL", gitconfig.to_str().unwrap()),
            ("GIT_CONFIG_SYSTEM", "/dev/null"),
            ("HOME", home.path().to_str().unwrap()),
            ("XDG_CONFIG_HOME", home.path().to_str().unwrap()),
        ]);
        // Candidate paths, one drop source each (+ one plain kept file):
        //   src/a.rs          kept
        //   src/secret.rs     .git/info/exclude (file rule)
        //   local/cache.tmp   .git/info/exclude (directory rule `local/`)
        //   gignored.rs       global excludes
        //   note.log          .gitignore (chain)
        //   top.rs            kept (root file)
        for rel in [
            "src/a.rs",
            "src/secret.rs",
            "local/cache.tmp",
            "gignored.rs",
            "note.log",
            "top.rs",
        ] {
            file(root.join(rel), "x\n");
        }

        // The WALK (native ignore-crate handling):
        let walk = FileList::build(root).unwrap();
        let walk_set: std::collections::BTreeSet<&str> =
            walk.files.iter().map(|s| s.as_str()).collect();
        // Expected membership — proves each source is honoured by the
        // walk itself (the invariant that was previously false).
        assert_eq!(
            walk_set,
            std::collections::BTreeSet::from(["Cargo.toml", "src/a.rs", "top.rs"]),
            "walk must drop the exclude/global/gitignore paths: {:?}",
            walk.files
        );
        // The FILTER (incremental index path) agrees with the walk on
        // every candidate:
        for rel in [
            "src/a.rs",
            "src/secret.rs",
            "local/cache.tmp",
            "gignored.rs",
            "note.log",
            "top.rs",
        ] {
            let p = root.join(rel);
            let in_index = !ignored(root, &p, false) && !under_graft(root, &p, false);
            assert_eq!(
                walk_set.contains(rel),
                in_index,
                "`{rel}`: in-walk={:?} in-index={in_index}",
                walk_set.contains(rel)
            );
        }
    }

    /// (c) Hidden components (F4): the walk's `hidden(true)` is mirrored
    /// in the filter — a changed path under a hidden directory that
    /// holds source-extension files (`.venv/...`, `.cargo/...`) must
    /// not be re-indexed after the full build excluded it. Marker-only
    /// root, so this isolates the hidden arm from the git-repo sources.
    #[test]
    fn hidden_component_is_never_indexed() {
        let dir = project();
        let root = dir.path();
        file(root.join(".venv/lib/site.py"), "x\n");
        file(root.join(".cargo/registry/crate.rs"), "x\n");
        file(root.join("src/ok.rs"), "x\n");
        // Filter side (the predicate): every hidden component, at any
        // depth, in either a file path or a directory event.
        assert!(ignored(root, &root.join(".venv/lib/site.py"), false));
        assert!(ignored(root, &root.join(".venv"), true));
        assert!(ignored(root, &root.join(".cargo/registry/crate.rs"), false));
        assert!(!ignored(root, &root.join("src/ok.rs"), false));
        // Walk side agrees (both branches of the walk set hidden(true)).
        let list = FileList::build(root).unwrap();
        assert!(
            !list.files.iter().any(|f| f.starts_with(".venv/") || f.starts_with(".cargo/")),
            "walk must exclude hidden trees: {:?}",
            list.files
        );
        assert!(list.files.iter().any(|f| f == "src/ok.rs"));
    }

    /// (d) The memo (F2): one batch with many paths under the same
    /// directories parses each `.gitignore` a bounded number of times —
    /// exactly once per distinct directory (each memo entry is one
    /// `or_insert_with` load; a per-path memo would grow with the
    /// batch and re-parse the root `.gitignore` 300 times).
    #[test]
    fn memo_bounds_gitignore_loads_per_directory() {
        let dir = project();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "*.log\nbuild/\n").unwrap();
        file(root.join("src/.gitignore"), "gen/\n");
        let mut paths = Vec::new();
        let mut ignored = Vec::new();
        for i in 0..300 {
            let (name, drop) = match i % 3 {
                // nested rule (`src/.gitignore`'s `gen/`) — directory rule
                0 => (format!("src/gen/f{i}.rs"), true),
                // root rule (`*.log`), file
                1 => (format!("src/f{i}.log"), true),
                // kept, root level
                _ => (format!("f{i}.txt"), false),
            };
            paths.push(root.join(&name));
            ignored.push(drop);
        }
        let memo: GitignoreMemo = Mutex::new(HashMap::new());
        for (p, want) in paths.iter().zip(ignored.iter()) {
            assert_eq!(
                is_gitignored(root, p, false, &memo),
                *want,
                "verdict through the memo: {:?}",
                p
            );
        }
        // 300 paths across exactly 3 directories (src/gen, src, root)
        // -> at most 3 memo entries (one per distinct directory).
        let map = memo.lock().unwrap();
        assert_eq!(
            map.len(),
            3,
            "memo must hold one entry per directory, got: {map:?}"
        );
        // A second identical batch hits the memo: no new entries, same
        // verdicts (the memo is per invocation, not per path).
        drop(map);
        for (p, want) in paths.iter().zip(ignored.iter()).take(50) {
            assert_eq!(is_gitignored(root, p, false, &memo), *want);
        }
        assert_eq!(memo.lock().unwrap().len(), 3, "second batch must not re-load");
    }

    /// R3: the file-list walk and the search pipeline must agree on
    /// gitignore semantics. This fixture exercises a nested `.gitignore`
    /// (root + subdir), a negation (`!important.log`), and a directory
    /// rule (`build/`). The only expected difference is the documented
    /// `graft/` asymmetry (Item 2): the file-list walk prunes `graft/`,
    /// the search pipeline does not. Both walkers call the same
    /// `load_gitignore` + `gitignore_matches` helpers (R3 extraction).
    #[test]
    fn finder_and_search_agree_on_gitignore() {
        use crate::search::rg::{SearchBus, SearchConfig, SearchEvent, spawn};
        use std::sync::atomic::AtomicBool;

        let dir = project();
        // Root .gitignore: ignore *.log (except important.log), ignore build/
        fs::write(
            dir.path().join(".gitignore"),
            "*.log\n!important.log\nbuild/\n",
        )
        .unwrap();
        file(dir.path().join("kept.txt"), "hello world\n");
        file(dir.path().join("crash.log"), "hello world\n");
        file(dir.path().join("important.log"), "hello world\n");
        file(dir.path().join("build/out.bin"), "hello world\n");
        // Subdirectory .gitignore: ignore gen/
        file(dir.path().join("src/.gitignore"), "gen/\n");
        file(dir.path().join("src/keep.rs"), "hello world\n");
        file(dir.path().join("src/gen/out.rs"), "hello world\n");
        file(dir.path().join("src/visible.rs"), "hello world\n");
        // graft/ directory (the documented asymmetry)
        file(dir.path().join("graft/card.md"), "hello world\n");

        // FileList walk (the finder side)
        let list = FileList::build(dir.path()).unwrap();
        let finder_files: std::collections::BTreeSet<&str> =
            list.files.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            finder_files,
            std::collections::BTreeSet::from([
                "Cargo.toml", "important.log", "kept.txt", "src/keep.rs", "src/visible.rs"
            ]),
            "FileList should contain exactly the non-ignored, non-graft files"
        );

        // Search pipeline (the grep side)
        let cfg = SearchConfig {
            root: dir.path().to_path_buf(),
            pattern: "hello world".to_string(),
            word: false,
            fixed: true,
            case_smart: false,
            case_insensitive: false,
            glob: None,
            file_type: None,
            filter: None,
            cancel: std::sync::Arc::new(AtomicBool::new(false)),
        };
        let (bus, mut rx) = SearchBus::new();
        spawn(cfg, &bus, 0);
        drop(bus);

        // Drain until Finished
        let mut search_files: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        let start = std::time::Instant::now();
        loop {
            match rx.try_recv() {
                Ok(SearchEvent::Hit { file, .. }) => {
                    search_files.insert(file);
                }
                Ok(SearchEvent::Finished { .. }) => break,
                Ok(_) => {}
                Err(_) => {
                    if start.elapsed() > std::time::Duration::from_secs(5) {
                        panic!(
                            "search did not finish within 5s; files so far: {search_files:?}"
                        );
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }

        // The only expected difference is graft/card.md (Item 2: the
        // file-list walk prunes graft/, the search pipeline does not).
        // All files except Cargo.toml (whose content doesn't match the
        // search pattern) contain "hello world".
        let expected_search: std::collections::BTreeSet<String> =
            ["graft/card.md", "important.log", "kept.txt", "src/keep.rs", "src/visible.rs"]
                .into_iter()
                .map(|s| s.to_string())
                .collect();
        assert_eq!(
            search_files, expected_search,
            "search and finder must agree on gitignore (modulo the documented graft/ asymmetry)"
        );

        // Index filter (the THIRD walker, this issue): the incremental
        // reindex must keep exactly the walk's file set over the same
        // fixture — the nested `.gitignore` (`src/gen/`), the root rules
        // (`*.log`, `build/`), the negation (`!important.log`), and the
        // graft/ prune all agree with the walk.
        let every_path = [
            "Cargo.toml", "kept.txt", "important.log", "crash.log", "build/out.bin",
            "src/keep.rs", "src/gen/out.rs", "src/visible.rs", "graft/card.md",
        ]
        .map(|rel| dir.path().join(rel));
        let index_kept: std::collections::BTreeSet<String> = every_path
            .iter()
            .filter(|p| {
                !ignored(dir.path(), p, false) && !under_graft(dir.path(), p, false)
            })
            .map(|p| {
                p.strip_prefix(dir.path())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            index_kept,
            list.files.iter().cloned().collect::<std::collections::BTreeSet<_>>(),
            "the index's changed-path filter must agree with the file walk"
        );
        // Pin the shared predicate on each rule kind (the one that
        // discriminates a root-only check is the nested one).
        assert!(
            ignored(dir.path(), &dir.path().join("src/gen/out.rs"), false),
            "nested .gitignore"
        );
        assert!(
            ignored(dir.path(), &dir.path().join("crash.log"), false),
            "root .gitignore file rule"
        );
        assert!(
            ignored(dir.path(), &dir.path().join("build/out.bin"), false),
            "directory rule (build/)"
        );
        assert!(
            !ignored(dir.path(), &dir.path().join("important.log"), false),
            "negation (!important.log)"
        );
        assert!(
            !ignored(dir.path(), &dir.path().join("src/visible.rs"), false),
            "non-ignored path stays"
        );
    }
}

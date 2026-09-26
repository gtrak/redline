//! Per-project file lists: an `ignore`-crate walk of the project root
//! (respects `.gitignore` and `.ignore`, skips hidden files/dirs,
//! never descends into `.git`), cached in memory per project root. The
//! walk runs once per project (measured ~20ms for a 10k-file repo,
//! debug build); the `re-walk` command invalidates the cache (live
//! refresh via the file watcher in `src/app/watcher.rs`).
//!
//! `.gitignore` handling has two modes (verified against the ignore
//! crate 0.4.33): inside a git repo the crate applies the ignore
//! sources natively — `.gitignore`, `.git/info/exclude`, and the global
//! git excludes (`IgnoreBuilder` defaults `git_ignore/git_exclude/
//! git_global: true, require_git: true`); for marker-only (non-git)
//! projects it applies **none** of the GIT sources (`require_git`
//! gates them all — per-directory `.ignore` files are NOT gated and
//! still apply natively), so this module evaluates the per-path
//! predicate (`is_gitignored`) itself and filters the walk.
//!
//! `is_gitignored` is the shared decision of the marker-only walk and
//! the incremental index filter (`AppStore::indexable_changes`); the
//! search pipeline's parallel walk carries the memoized equivalent
//! (`git_ignored_by_ancestors`) — R3: the walkers agree. Behaviourally,
//! the filter reproduces the walk for marker-only trees (the
//! `.gitignore` and `.ignore` chain, `hidden(true)`), and in git repos
//! for the sources above (`.gitignore` and `.ignore` chain, `.git/
//! info/exclude`, global excludes, `hidden(true)`).
//!
//! Two divergences remain, both PRE-EXISTING and narrow:
//!
//! 1. **Cross-level negation precedence**: a deeper `!pat` cannot
//!    rescue a path that a shallower ignore-file rule ignores — not real
//!    git semantics. The file-list walk and the incremental filter share
//!    the predicate, so they agree with each other; the SEARCH pipeline's
//!    walk uses the `ignore` crate's native resolution (deepest match
//!    wins) and can therefore disagree with them in this corner, in
//!    marker-only trees. In git repos the file-list walk and the search
//!    walk are both native, so they agree.
//! 2. **An ignore file ABOVE the project root**: the walk's
//!    `parents(true)` applies an ancestor directory's `.ignore` /
//!    `.gitignore` to the walk, but `is_gitignored` stops its chain at
//!    `root`. A path excluded by such an above-root ignore file is
//!    therefore excluded by the full build and RE-INDEXED by the
//!    incremental filter — the same class as the bug this predicate
//!    exists to prevent, just narrow (it needs an ignore file in a
//!    directory above the project root). Closing it means walking the
//!    chain past `root`; until then it is a known, documented
//!    divergence, not an accident.

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
            // **none** of the GIT ignore sources (`require_git` gates
            // `.gitignore`, `.git/info/exclude` and the global
            // excludes — per-directory `.ignore` files are not gated
            // and apply natively), so filter through the shared
            // per-path predicate (`is_gitignored` + `under_graft`) —
            // the SAME decisions the incremental index filter and the
            // search pipeline make (R3: one source of truth). The
            // predicate also covers `.ignore` (P3a), so the native
            // application and the filter agree.
            let root_owned = root.to_path_buf();
            // One memo for the whole walk (F2): each directory's
            // ignore files (`.gitignore` + `.ignore`) are parsed at
            // most once per walk, not once per entry. Per-invocation on purpose: `.gitignore` files
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
/// `.gitignore` and `.ignore` (keyed by the ignore FILE's path) and,
/// for git repos, the repo's `.git/info/exclude` and the process's
/// global git excludes (keyed by sentinel paths under `root/.git/
/// info/`). A `None` value is a cached "absent/unparseable" verdict,
/// not just a matcher cache.
///
/// Scope: **per invocation** — created inside `FileList::build` (one
/// per walk) and inside `AppStore::indexable_changes` (one per change
/// batch), mirroring the search pipeline's memo (`src/search/rg.rs`).
/// Never a long-lived global: `.gitignore` files change while the app
/// runs, and a persistent cache would serve stale verdicts with no
/// invalidation path. Sharing one memo across a batch is what keeps
/// the incremental filter's I/O off the UI thread: each directory's
/// `.gitignore` and `.ignore` are parsed at most once per batch, not
/// once per changed path.
pub type GitignoreMemo = Mutex<HashMap<PathBuf, Option<Gitignore>>>;

/// True when `path` (absolute, under `root`) is excluded from the
/// `FileList` walk for ignore reasons. Evaluates, in the walk's source
/// order:
///
/// 1. any HIDDEN component of the project-relative path — the walk's
///    `hidden(true)` (F4: the incremental filter must not index what
///    the full build excluded);
/// 2. the ignore-file ANCESTOR CHAIN — each directory's `.ignore`
///    (the ignore crate's second per-directory source, P3a) then its
///    `.gitignore`; the path's own directory for a dir, its parent for
///    a file, up to `root` — memoized per ignore file in `memo`; each
///    matcher is asked in the ancestor form
///    (`matched_path_or_any_parents`) — a directory rule like `gen/`
///    ignores the files beneath it, and a DIRECTORY is judged the same
///    way, so `build/sub` under a root `build/` rule is reported
///    ignored (the walk prunes `build` wholesale). A `.ignore` match
///    (either direction) outranks the `.gitignore` chain, mirroring
///    the walk's evaluation order (`m_ignore` before `m_gi` in the
///    ignore crate);
/// 3. git repos only (F1): `.git/info/exclude`, then the process's
///    global git excludes (`core.excludesFile`, `Gitignore::global`) —
///    the sources the walk's `IgnoreBuilder` defaults
///    (`git_exclude/git_global: true, require_git: true`) apply
///    natively. An explicit ignore-file match (either direction) takes
///    precedence over both. The global excludes are matched against
///    the FULL (absolute) path — the same input the walk feeds its
///    cwd-rooted global matcher (P2a: the project-relative input made
///    slash/anchored global patterns over-ignore against the walk).
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
/// a deeper `!pat` cannot rescue a path a shallower ignore-file rule
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
    // (2) The ignore-file ancestor chain, deepest to root: each
    // directory's `.ignore` (the ignore crate's second per-directory
    // source, P3a) and its `.gitignore` — the walk's source order.
    // An explicit match in either direction is decisive over the
    // repo-level sources (step 3); a `.ignore` match outranks the
    // `.gitignore` chain.
    let mut ig_ignored = false;
    let mut ig_explicit = false;
    let mut gi_ignored = false;
    let mut gi_explicit = false;
    let mut dir = if is_dir {
        Some(path.to_path_buf())
    } else {
        path.parent().map(|p| p.to_path_buf())
    };
    while let Some(d) = dir {
        // The matcher's root `d` is `path` itself (a dir) or an ancestor
        // of it, so the ancestor-form call below is always inside the
        // matcher's root (its precondition).
        for (name, ignored, explicit) in [
            (".ignore", &mut ig_ignored, &mut ig_explicit),
            (".gitignore", &mut gi_ignored, &mut gi_explicit),
        ] {
            let key = d.join(name);
            let matcher = map
                .entry(key)
                .or_insert_with(|| load_ignore_file(&d, name));
            if let Some(gi) = matcher {
                let m = gi.matched_path_or_any_parents(path, is_dir);
                if m.is_ignore() {
                    *ignored = true;
                } else if m.is_whitelist() {
                    *explicit = true;
                }
            }
        }
        if d == root {
            break; // the root's own ignore files were the last check
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    if ig_ignored {
        return true;
    }
    if ig_explicit {
        // `.ignore` says "keep": the `.gitignore` chain and the exclude
        // sources cannot override it.
        return false;
    }
    if gi_ignored {
        return true;
    }
    if gi_explicit {
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
        // P2a: the global matcher is rooted at the process cwd (like
        // the walk's), so it gets the FULL path — the same input the
        // walk feeds. Feeding the project-relative path made slash and
        // anchored global patterns match at the cwd root instead of
        // where the walk sees them, so the filter OVER-ignored files
        // the full build indexed (stale symbols until a full rebuild).
        // `.git/info/exclude` above keeps `rel`: it is repo-rooted,
        // exactly as the walk roots it.
        return global
            .as_ref()
            .is_some_and(|gi| matches!(rel_decision(gi, path, is_dir), Some(true)));
    }
    false
}

/// The decision a repo-level source (`.git/info/exclude` or the global
/// excludes) makes about `input` — the same input the walk feeds that
/// matcher (entries are evaluated relative to the matcher's root, so a
/// root-anchored pattern like `/build/` lands where git puts it):
/// the PROJECT-relative path for `.git/info/exclude` (repo-rooted),
/// the FULL (absolute) path for the global excludes (cwd-rooted —
/// P2a). Ancestor form, because the walk prunes an ignored directory
/// wholesale: a directory rule (`build/`) must drop the directory and
/// everything under it. `Some(true)` = ignore, `Some(false)` = explicit
/// keep, `None` = no match.
fn rel_decision(gi: &Gitignore, input: &Path, is_dir: bool) -> Option<bool> {
    let m = gi.matched(input, is_dir);
    if !m.is_none() {
        return Some(m.is_ignore());
    }
    let mut parent = input.parent();
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
/// process cwd, and `rel_decision` matches it against the FULL
/// (absolute) path — the same input the walk's matcher sees, P2a;
/// the old project-relative input made slash/anchored patterns
/// disagree with the walk). `Some` (possibly empty) when nothing is
/// configured or the read fails: the walk applies the same empty
/// matcher, so caching "nothing to ignore" keeps the two agreeing.
fn load_global_excludes() -> Option<Gitignore> {
    Some(Gitignore::global().0)
}

/// Load a per-directory ignore file (`.gitignore` or `.ignore`) from
/// `dir` as a matcher rooted at `dir`. `None` when the file is absent
/// or unparseable. Shared by the file-list walk, the incremental index
/// filter, and the search pipeline (R3: all walkers must agree on
/// gitignore semantics).
pub(crate) fn load_ignore_file(dir: &Path, name: &str) -> Option<Gitignore> {
    match Gitignore::new(dir.join(name)) {
        (gi, None) => Some(gi),
        _ => None,
    }
}

/// Load a `.gitignore` file from `dir` as a matcher. `None` when the
/// file is absent or unparseable. Shared by the file-list walk, the
/// incremental index filter, and the search pipeline (R3: all walkers
/// must agree on gitignore semantics).
pub(crate) fn load_gitignore(dir: &Path) -> Option<Gitignore> {
    load_ignore_file(dir, ".gitignore")
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
    /// fallbacks. Restored on drop.
    ///
    /// P3b: env mutation is process-global and other threads reading
    /// env vars concurrently is UB, so the guard holds the crate-level
    /// `ENV_LOCK` for its whole life — the pattern of `git/commit.rs`
    ///'s `IsolatedHome` precedent, which SERIALIZES behind that lock
    /// (the old doc here cited it as an "accepted race window", which
    /// was backwards).
    struct EnvGuard {
        restore: HashMap<String, Option<std::ffi::OsString>>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn set(pairs: &[(&str, &str)]) -> Self {
            let _lock = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let restore = pairs
                .iter()
                .map(|(k, _)| (k.to_string(), std::env::var_os(k)))
                .collect();
            // SAFETY: `set_var` is unsafe in edition 2024 because it races
            // with ANY concurrent `env::var`/`var_os` reader on another
            // thread (process-wide, not per-variable). This runs while
            // holding `crate::ENV_LOCK` (kept in the struct, declared
            // after `restore`, so it drops LAST — the `Drop` restore below
            // also runs under the lock), and every other env-mutating
            // test in this crate (`git::commit`'s `EnvScope`, the
            // `ui::file_view` tint tests) takes that same lock.
            // Known residual (reported, issue-guardrails P1): the
            // `app::store` fetch tests still mutate/read `PATH` under
            // their own `PATH_LOCK` — to be moved onto `ENV_LOCK`.
            for (k, v) in pairs {
                // SAFETY: as above — under `crate::ENV_LOCK`, held for
                // this guard's whole life.
                unsafe { std::env::set_var(k, v); }
            }
            Self { restore, _lock }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: same invariant as `set` above — `_lock` is declared
            // after `restore`, so it drops LAST: the restore completes
            // (every `set_var`/`remove_var` here) before the
            // `ENV_LOCK`-serialized window closes.
            for (k, v) in self.restore.drain() {
                match v {
                    Some(v) => {
                        // SAFETY: same invariant as `set` above — the
                        // `ENV_LOCK` guard is still live (`_lock` drops
                        // last), so this restore cannot race a concurrent
                        // environment reader.
                        unsafe { std::env::set_var(k, v); }
                    }
                    None => {
                        // SAFETY: same invariant as the arm above.
                        unsafe { std::env::remove_var(k); }
                    }
                }
            }
        }
    }

    /// (a) Git-repo agreement (F1 + P2a): the walk's `IgnoreBuilder`
    /// defaults honour `.git/info/exclude` AND the global git excludes
    /// (`git_exclude/git_global: true, require_git: true`) — a file
    /// ignored SOLELY by those sources must not be re-indexed. The
    /// existing fixtures have no `.git`, so this is the behavioural
    /// arm: the same paths are fed to the walk and to the filter, and
    /// each source independently drops a distinct path (drop one and
    /// the assertion pins which source went missing).
    ///
    /// P2a: the global-excludes file also carries a SLASH pattern
    /// (`sub/inner.rs`) and an ANCHORED pattern (`/anchored.rs`). The
    /// global matcher is cwd-rooted (the walk's and the filter's are
    /// the same matcher), so both must be fed the FULL path — with
    /// the project-relative input the filter over-ignored exactly
    /// these two (it is the pre-fix failure this test pins). Real
    /// `git check-ignore` reports `sub/inner.rs`/`anchored.rs`
    /// ignored here too — the walk (and thus the fixed filter) is the
    /// one that keeps them, an ignore-crate quirk of cwd-rooting; the
    /// filter's contract is agreement with the walk.
    #[test]
    fn git_repo_walk_and_filter_agree_on_exclude_and_global() {
        let dir = project();
        let root = dir.path();
        // Repo marker + per-repo excludes (the gate's demonstrated
        // case + a slash pattern and an anchored pattern, repo-rooted
        // like the walk's exclude matcher).
        fs::create_dir_all(root.join(".git/info")).unwrap();
        fs::write(
            root.join(".git/info/exclude"),
            "local/\nsecret.rs\nexsub/inner.rs\n/anchored_excl.rs\n",
        )
        .unwrap();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        // Global excludes, hermetic (see `EnvGuard`) — plain, slash,
        // and anchored patterns (the P2a inputs).
        let home = tempfile::tempdir().unwrap();
        let global_ig = home.path().join("global-ignore");
        fs::write(
            &global_ig,
            "gignored.rs\nsub/inner.rs\n/anchored.rs\n",
        )
        .unwrap();
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
        // Candidate paths, one drop source each (+ plain kept files):
        //   src/a.rs            kept
        //   src/secret.rs       .git/info/exclude (file rule)
        //   local/cache.tmp     .git/info/exclude (directory rule `local/`)
        //   exsub/inner.rs      .git/info/exclude (slash rule, repo-rooted)
        //   anchored_excl.rs    .git/info/exclude (anchored rule)
        //   gignored.rs         global excludes (plain rule)
        //   sub/inner.rs        global excludes (SLASH rule): the walk
        //                       KEEPS it (cwd-rooting quirk) — pre-P2a
        //                       the filter dropped it (over-ignore)
        //   anchored.rs         global excludes (anchored rule): same
        //                       shape, same quirk
        //   note.log            .gitignore (chain)
        //   top.rs              kept (root file)
        //   nested/plain.rs     kept
        //   excl/plain2.rs      kept
        for rel in [
            "src/a.rs",
            "src/secret.rs",
            "local/cache.tmp",
            "exsub/inner.rs",
            "anchored_excl.rs",
            "gignored.rs",
            "sub/inner.rs",
            "anchored.rs",
            "note.log",
            "top.rs",
            "nested/plain.rs",
            "excl/plain2.rs",
        ] {
            file(root.join(rel), "x\n");
        }

        // The WALK (native ignore-crate handling):
        let walk = FileList::build(root).unwrap();
        let walk_set: std::collections::BTreeSet<&str> =
            walk.files.iter().map(|s| s.as_str()).collect();
        // Expected membership — proves each source is honoured by the
        // walk itself (the invariant that was previously false), and
        // pins the P2a quirk: the cwd-rooted global matcher does not
        // reach the slash/anchored candidates, so the walk keeps them.
        assert_eq!(
            walk_set,
            std::collections::BTreeSet::from([
                "Cargo.toml",
                "src/a.rs",
                "top.rs",
                "sub/inner.rs",
                "anchored.rs",
                "nested/plain.rs",
                "excl/plain2.rs"
            ]),
            "walk must drop the exclude/global/gitignore paths (and keep the \
             slash/anchored global ones the cwd-rooted matcher cannot reach): {:?}",
            walk.files
        );
        // The FILTER (incremental index path) agrees with the walk on
        // every candidate — the P2a contract is filter verdict == walk
        // membership, on every path, not just the plain-rule ones:
        for rel in [
            "src/a.rs",
            "src/secret.rs",
            "local/cache.tmp",
            "exsub/inner.rs",
            "anchored_excl.rs",
            "gignored.rs",
            "sub/inner.rs",
            "anchored.rs",
            "note.log",
            "top.rs",
            "nested/plain.rs",
            "excl/plain2.rs",
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
        // Directory-event form: the walk prunes the ignored directory
        // wholesale, so the filter must report the directory itself
        // ignored — and keep the directories the walk keeps.
        assert!(
            ignored(root, &root.join("local"), true),
            "`local/` exclude rule must ignore the directory itself"
        );
        assert!(
            !ignored(root, &root.join("exsub"), true),
            "the slash exclude rule matches the PATH exsub/inner.rs, not the \
             parent dir — the walk keeps `exsub/` and drops the file, the filter must"
        );
        assert!(
            !ignored(root, &root.join("sub"), true),
            "the walk keeps `sub/`; the filter must too (P2a)"
        );
    }

    /// (b) `.ignore` files (P3a): the walk's per-directory chain also
    /// honours `.ignore` (the ignore crate's `ignore: true` default —
    /// NOT gated by `require_git`, so it applies in marker-only trees
    /// too), so the predicate must read `.ignore` as the same
    /// per-directory chain with a second filename — the faithful
    /// choice (decision: reading it). Without it, a changed path the
    /// full build dropped via a parent `.ignore` would be re-indexed —
    /// the F1 bug class, pre-existing and outside the F1–F5 fence.
    /// Hermetic agreement: the filter's verdict equals walk membership
    /// on every candidate, for both a root `.ignore` and a PARENT
    /// (subdir) `.ignore`.
    #[test]
    fn walk_and_filter_agree_on_dot_ignore_files() {
        let dir = project();
        let root = dir.path();
        // Marker-only tree (no `.git`): the native GIT ignore sources
        // are inert (`require_git`); `.ignore` still applies to the
        // walk natively, and the predicate must agree.
        fs::write(root.join(".ignore"), "*.tmp\nbuild2/\n").unwrap();
        file(root.join("src/.ignore"), "gen2/\n");
        file(root.join("src/keep.rs"), "k\n");
        file(root.join("src/gen2/out.rs"), "g\n");
        file(root.join("src/vis.rs"), "v\n");
        file(root.join("scratch.tmp"), "t\n");
        file(root.join("build2/x.bin"), "b\n");
        file(root.join("plain.rs"), "p\n");

        let walk = FileList::build(root).unwrap();
        let walk_set: std::collections::BTreeSet<&str> =
            walk.files.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            walk_set,
            std::collections::BTreeSet::from([
                "Cargo.toml",
                "plain.rs",
                "src/keep.rs",
                "src/vis.rs"
            ]),
            "walk must drop the .ignore paths (root and parent level): {:?}",
            walk.files
        );
        // Filter verdict == walk membership on every candidate.
        for rel in [
            "plain.rs",
            "scratch.tmp",
            "build2/x.bin",
            "src/keep.rs",
            "src/gen2/out.rs",
            "src/vis.rs",
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
        // Directory-event form: the walk prunes the directory
        // wholesale, so the filter must report it ignored.
        assert!(
            ignored(root, &root.join("build2"), true),
            "root .ignore directory rule must ignore the directory"
        );
        assert!(
            ignored(root, &root.join("src/gen2"), true),
            "parent (subdir) .ignore directory rule must ignore the directory"
        );
        assert!(!ignored(root, &root.join("src"), true));
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
    /// directories parses each ignore file a bounded number of times —
    /// exactly once per distinct (directory, file) (each memo entry is
    /// one `or_insert_with` load; a per-path memo would grow with the
    /// batch and re-parse the root `.gitignore` 300 times).
    ///
    /// Perf record (P3c — restated as a ratio with the profile named,
    /// not as unverifiable absolutes): the F2 fixture is now
    /// reproducible from the tree — `python3 tools/ignore_fixture.py
    /// /tmp/ignore_fixture` (20,000-file depth-4 marker-only tree,
    /// 33-line root `.gitignore`, a `.gitignore` every 5th directory),
    /// measured by the `--index-profile` walk leg. Release build,
    /// lane's box: pre-memo 7359.7 ms vs memoized 555.0 ms — a
    /// ~13× walk-speedup. This session re-measured the memoized walk
    /// on the same fixture (release build, `--index-profile`): 171.6 ms
    /// over 18,401 files. The gate's independent debug-build re-run
    /// reproduced the direction: 8751.5 → 1529.8 ms cold, 5.7×.
    /// Absolute ms are machine- and profile-dependent; regenerate the
    /// fixture and re-measure to confirm, and compare the ratio,
    /// not the absolutes.
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
        // -> exactly 6 memo entries (one per distinct (directory,
        // ignore FILE): `.gitignore` + `.ignore` each).
        let map = memo.lock().unwrap();
        assert_eq!(
            map.len(),
            6,
            "memo must hold one entry per (directory, ignore file), got: {map:?}"
        );
        // A second identical batch hits the memo: no new entries, same
        // verdicts (the memo is per invocation, not per path).
        drop(map);
        for (p, want) in paths.iter().zip(ignored.iter()).take(50) {
            assert_eq!(is_gitignored(root, p, false, &memo), *want);
        }
        assert_eq!(
            memo.lock().unwrap().len(),
            6,
            "second batch must not re-load"
        );
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

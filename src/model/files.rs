//! Per-project file lists: an `ignore`-crate walk of the project root
//! (respects `.gitignore`, skips hidden files/dirs, never descends into
//! `.git`), cached in memory per project root. The walk runs once per
//! project (measured ~20ms for a 10k-file repo, debug build); the
//! `re-walk` command invalidates the cache (live refresh via the file
//! watcher in `src/app/watcher.rs`).
//!
//! `.gitignore` handling has two modes (verified against the ignore
//! crate 0.4.33): inside a git repo the crate applies `.gitignore`
//! natively; for marker-only (non-git) projects the crate does **not**
//! apply `.gitignore` at all, so this module evaluates the per-path
//! `.gitignore` ancestor chain (`is_gitignored`) itself and filters the
//! walk — the same decision the incremental index filter and the search
//! pipeline make (R3: one source of truth).

use std::path::Path;

use ignore::gitignore::Gitignore;
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
            // Marker-only (non-git) project: the ignore crate does **not**
            // apply `.gitignore` at all, so filter through the shared
            // ancestor-chain predicates (`is_gitignored` + `under_graft`) —
            // the SAME decisions the incremental index filter and the
            // search pipeline make (R3: one source of truth).
            let root_owned = root.to_path_buf();
            builder.filter_entry(move |entry| {
                let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
                !under_graft(&root_owned, entry.path(), is_dir)
                    && !is_gitignored(&root_owned, entry.path(), is_dir)
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

/// True when `path` (absolute, under `root`) is gitignored by the
/// `.gitignore` CHAIN from the project root down to `path`: the root's
/// matcher plus every intermediate directory's, exactly the ancestor
/// chain the file walk and the search pipeline evaluate. For a FILE,
/// each ancestor's matchers are asked about the file AND its parent
/// directories (`matched_path_or_any_parents` — a directory rule like
/// `gen/` must ignore the files beneath it); a DIRECTORY is judged on
/// itself the same way (a directory's OWN `.gitignore` applies to its
/// contents, never to itself). Paths outside `root` and `root` itself
/// are never reported ignored.
///
/// The shared decision for the file-list walk (non-git branch), the
/// incremental index filter (`AppStore::indexable_changes`), and any
/// other per-path consumer. The search pipeline's parallel walk needs a
/// per-directory memo cache and keeps its own equivalent
/// (`git_ignored_by_ancestors`), built on the same
/// `load_gitignore` + `gitignore_matches` primitives (R3).
pub fn is_gitignored(root: &Path, path: &Path, is_dir: bool) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    if rel.as_os_str().is_empty() {
        return false; // the root itself is never ignored here
    }
    let mut dir = if is_dir {
        Some(path.to_path_buf())
    } else {
        path.parent().map(|p| p.to_path_buf())
    };
    while let Some(d) = dir {
        if let Some(gi) = load_gitignore(&d) {
            let hits = if is_dir {
                gitignore_matches(&gi, path, true)
            } else {
                // The file itself, or any parent directory of it, up to
                // this matcher's own root: this is what makes a
                // directory rule (`gen/`) ignore the files beneath it.
                gi.matched_path_or_any_parents(path, false).is_ignore()
            };
            if hits {
                return true;
            }
        }
        if d == root {
            break; // the root's own `.gitignore` was the last check
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    false
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
            is_gitignored(root, &root.join("src/gen/out.rs"), false),
            "nested .gitignore must apply"
        );
        assert!(!is_gitignored(root, &root.join("src/visible.rs"), false));
        // Root `.gitignore` rules (file rule + directory rule).
        assert!(is_gitignored(root, &root.join("crash.log"), false));
        assert!(is_gitignored(root, &root.join("build/out.bin"), false));
        // Negation: `!important.log` un-ignores despite `*.log`.
        assert!(!is_gitignored(root, &root.join("important.log"), false));
        // Plain non-ignored path.
        assert!(!is_gitignored(root, &root.join("kept.txt"), false));
        // Directory form: judged the same way the walk judges `build/`.
        assert!(is_gitignored(root, &root.join("build"), true));
        assert!(!is_gitignored(root, &root.join("src"), true));
        // Out-of-root and the root itself are never "ignored" here.
        assert!(!is_gitignored(root, Path::new("/elsewhere/x.rs"), false));
        assert!(!is_gitignored(root, root, false));
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
                !is_gitignored(dir.path(), p, false) && !under_graft(dir.path(), p, false)
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
            is_gitignored(dir.path(), &dir.path().join("src/gen/out.rs"), false),
            "nested .gitignore"
        );
        assert!(
            is_gitignored(dir.path(), &dir.path().join("crash.log"), false),
            "root .gitignore file rule"
        );
        assert!(
            is_gitignored(dir.path(), &dir.path().join("build/out.bin"), false),
            "directory rule (build/)"
        );
        assert!(
            !is_gitignored(dir.path(), &dir.path().join("important.log"), false),
            "negation (!important.log)"
        );
        assert!(
            !is_gitignored(dir.path(), &dir.path().join("src/visible.rs"), false),
            "non-ignored path stays"
        );
    }
}

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
//! apply `.gitignore` at all, so this module builds per-directory
//! matchers from the `.gitignore` files itself and filters the walk.

use std::path::Path;
use std::sync::Mutex;

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
        // Prune the `graft` cache directory at the walk level (issue 05,
        // finding 3): graft cache cards are agent output, not project source,
        // and must not pollute the file finder or the tree sidebar (the tree
        // inherits the fix from this same walk). Grep (the search pipeline,
        // gitignore-based) keeps its own walk and still sees graft/.
        if is_git_repo {
            builder.filter_entry(|entry| !is_graft_dir(entry));
        } else {
            attach_gitignore_filter(&mut builder, root);
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

/// True when `entry` is a `graft` directory (the agent cache) that the file
/// finder / tree walk must prune (issue 05, finding 3). Pruning the directory
/// also prunes its whole subtree; the search pipeline's gitignore-based walk
/// is unaffected.
fn is_graft_dir(entry: &ignore::DirEntry) -> bool {
    entry.file_type().is_some_and(|ft| ft.is_dir()) && entry.file_name().to_str() == Some("graft")
}

/// Load a `.gitignore` file from `dir` as a matcher. `None` when the
/// file is absent or unparseable. Shared by the file-list walk and the
/// search pipeline (R3: both must agree on gitignore semantics).
pub(crate) fn load_gitignore(dir: &Path) -> Option<Gitignore> {
    match Gitignore::new(dir.join(".gitignore")) {
        (gi, None) => Some(gi),
        _ => None,
    }
}

/// True when `gi` ignores `path` (a directory when `is_dir`, a file
/// otherwise). The single shared gitignore decision used by both the
/// file-list walk and the search pipeline.
pub(crate) fn gitignore_matches(gi: &Gitignore, path: &Path, is_dir: bool) -> bool {
    gi.matched(path, is_dir).is_ignore()
}

/// Honor `.gitignore` files for non-git projects (the ignore crate
/// skips them there): a depth-truncated stack of per-directory
/// matchers — the root's matcher plus each subdirectory's — checked
/// for every file entry against its ancestor chain.
///
/// Used only by this (sequential) file-list walk: the stack is shared
/// mutable state that is only safe with a single-threaded walk. The
/// search pipeline (`src/search/rg.rs`) runs a PARALLEL walk,
/// where a shared per-directory stack is unsound, so it implements the
/// same per-entry ancestor-chain semantics independently
/// (`attach_entry_filters` / `git_ignored_by_ancestors`, with a
/// per-directory memo cache instead of the shared stack). Both call the
/// same `load_gitignore` + `gitignore_matches` helpers (R3).
pub(crate) fn attach_gitignore_filter(builder: &mut WalkBuilder, root: &Path) {
    // The root entry is not passed through `filter_entry`, so seed the
    // stack with the root's own `.gitignore` matcher (depth 0).
    let root_matcher = load_gitignore(root);
    let stack: Mutex<Vec<Option<Gitignore>>> = Mutex::new(vec![root_matcher]);
    builder.filter_entry(move |entry: &ignore::DirEntry| {
        // Prune the graft cache directory (issue 05, finding 3) before the
        // gitignore stack logic.
        if is_graft_dir(entry) {
            return false;
        }
        let mut stack = stack.lock().unwrap();
        let depth = entry.depth();
        if entry.file_type().is_some_and(|ft| ft.is_dir()) {
            // An ignored directory prunes its whole subtree.
            if stack.iter().take(depth).any(|m| {
                m.as_ref().is_some_and(|gi| gitignore_matches(gi, entry.path(), true))
            }) {
                return false;
            }
            // Keep only the ancestor matchers, then record this
            // directory's own `.gitignore` (if any).
            stack.truncate(depth);
            stack.push(load_gitignore(entry.path()));
            true
        } else {
            !stack.iter().take(depth).any(|m| {
                m.as_ref().is_some_and(|gi| gitignore_matches(gi, entry.path(), false))
            })
        }
    });
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
    }
}

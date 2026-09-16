//! Per-project file lists: an `ignore`-crate walk of the project root
//! (respects `.gitignore`, skips hidden files/dirs, never descends into
//! `.git`), cached in memory per project root. The walk runs once per
//! project (measured ~20ms for a 10k-file repo, debug build); the
//! `re-walk` command invalidates the cache (automatic refresh arrives
//! with issue 04's watcher).
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
        if !is_git_repo {
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

/// Honor `.gitignore` files for non-git projects (the ignore crate
/// skips them there): a depth-truncated stack of per-directory
/// matchers — the root's matcher plus each subdirectory's — checked
/// for every file entry against its ancestor chain.
fn attach_gitignore_filter(builder: &mut WalkBuilder, root: &Path) {
    // The root entry is not passed through `filter_entry`, so seed the
    // stack with the root's own `.gitignore` matcher (depth 0).
    let root_matcher = match Gitignore::new(root.join(".gitignore")) {
        (gi, None) => Some(gi),
        (_, Some(_)) => None,
    };
    let stack: Mutex<Vec<Option<Gitignore>>> = Mutex::new(vec![root_matcher]);
    builder.filter_entry(move |entry: &ignore::DirEntry| {
        let mut stack = stack.lock().unwrap();
        let depth = entry.depth();
        if entry.file_type().is_some_and(|ft| ft.is_dir()) {
            // An ignored directory prunes its whole subtree.
            if stack.iter().take(depth).any(|m| {
                m.as_ref()
                    .is_some_and(|gi| gi.matched(entry.path(), true).is_ignore())
            }) {
                return false;
            }
            // Keep only the ancestor matchers, then record this
            // directory's own `.gitignore` (if any).
            stack.truncate(depth);
            let matcher = match Gitignore::new(entry.path().join(".gitignore")) {
                (gi, None) => Some(gi),
                (_, Some(_)) => None,
            };
            stack.push(matcher);
            true
        } else {
            !stack.iter().take(depth).any(|m| {
                m.as_ref()
                    .is_some_and(|gi| gi.matched(entry.path(), false).is_ignore())
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
}

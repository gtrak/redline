//! The git repository wrapper: opens/discovers a repo from the project
//! root and exposes the magit-subset operations (status, branch, per-file
//! diffs, file + hunk staging) as plain data. git2 types never leave this
//! module.
//!
//! Threading: `git2::Repository` is `Send` (not `Sync`), and the whole
//! `GitRepo` lives behind the store's `Mutex`, so it is safe to hold here.
//!
//! Layout: the `GitRepo` struct, `discover`, and the query surface
//! (`branch`, `status`, `diff`, ...) live here; the index mutations
//! (stage/unstage/discard) are a second `impl GitRepo` block in `index_ops`;
//! the pure hunk byte math and index-entry plumbing are the free functions
//! in `hunk`.

use std::path::Path;

use crate::git::diff::{DiffSide, FileDiff, extract};
use crate::git::error::GitError;
use crate::git::status::{BranchInfo, FileStatus, RepoStatus, StatusKind};

mod hunk;
mod index_ops;

#[cfg(test)]
mod tests;

/// A thin wrapper around an open `git2::Repository`.
pub struct GitRepo {
    /// The underlying libgit2 handle. `pub(crate)` so the sibling `src/git`
    /// submodules (log/blame/commit/refs) can drive git2 without leaking its
    /// types past this module tree.
    pub(crate) inner: git2::Repository,
}

impl GitRepo {
    /// Open (or discover, walking up) the git repo at `path`. Errors with
    /// `NotARepository` when no enclosing repository exists.
    pub fn discover(path: &Path) -> Result<Self, GitError> {
        let inner = git2::Repository::discover(path)
            .map_err(|_| GitError::NotARepository {
                path: path.to_path_buf(),
            })?;
        Ok(Self { inner })
    }

    /// The current branch (or a detached/unborn marker).
    pub fn branch(&self) -> Result<BranchInfo, GitError> {
        match self.inner.head() {
            Ok(ref_) => {
                let detached = self.inner.head_detached()?;
                if detached {
                    let oid = ref_.target().map(|o| o.to_string()).unwrap_or_default();
                    let short = oid.chars().take(7).collect::<String>();
                    let name = if short.is_empty() {
                        "HEAD".to_string()
                    } else {
                        format!("({short})")
                    };
                    Ok(BranchInfo {
                        name,
                        detached: true,
                        unborn: false,
                    })
                } else {
                    let name = ref_
                        .shorthand()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|_| "HEAD".to_string());
                    Ok(BranchInfo {
                        name,
                        detached: false,
                        unborn: false,
                    })
                }
            }
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(BranchInfo {
                name: "(no commits)".to_string(),
                detached: false,
                unborn: true,
            }),
            Err(e) => Err(e.into()),
        }
    }

    /// `git status`: staged/unstaged/untracked classification with rename
    /// detection, plus the current branch. Files are returned sorted by
    /// path (git2's iteration order is not sorted).
    pub fn status(&self) -> Result<RepoStatus, GitError> {
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .renames_head_to_index(true)
            .renames_index_to_workdir(true)
            .update_index(true);
        // Deliberately do NOT set `include_ignored`: ignored files must not
        // appear in the status buffer.
        let statuses = self.inner.statuses(Some(&mut opts))?;

        let mut files: Vec<FileStatus> = Vec::new();
        for entry in statuses.iter() {
            let status = entry.status();

            let staged = if status.intersects(git2::Status::INDEX_RENAMED) {
                StatusKind::Renamed
            } else if status.intersects(git2::Status::INDEX_NEW) {
                StatusKind::Added
            } else if status.intersects(git2::Status::INDEX_DELETED) {
                StatusKind::Deleted
            } else if status.intersects(git2::Status::INDEX_MODIFIED) {
                StatusKind::Modified
            } else if status.intersects(git2::Status::INDEX_TYPECHANGE) {
                StatusKind::Typechange
            } else {
                StatusKind::None
            };

            let unstaged = if status.intersects(git2::Status::WT_RENAMED) {
                StatusKind::Renamed
            } else if status.intersects(git2::Status::WT_DELETED) {
                StatusKind::Deleted
            } else if status.intersects(git2::Status::WT_MODIFIED) {
                StatusKind::Modified
            } else if status.intersects(git2::Status::WT_TYPECHANGE) {
                StatusKind::Typechange
            } else {
                StatusKind::None
            };

            // Untracked: a workdir-new file with no index-side change.
            let untracked = status.intersects(git2::Status::WT_NEW)
                && staged == StatusKind::None;

            // Canonical path + pre-rename path. For a rename, libgit2
            // reports the entry under the OLD (head/index) path; the new
            // path lives on the delta's new file.
            let (path, orig) = if staged == StatusKind::Renamed {
                let d = entry.head_to_index();
                rename_paths(
                    d.as_ref()
                        .and_then(|d| d.new_file().path())
                        .map(|p| p.to_string_lossy().into_owned()),
                    d.as_ref()
                        .and_then(|d| d.old_file().path())
                        .map(|p| p.to_string_lossy().into_owned()),
                    entry.path()?.to_string(),
                )
            } else if unstaged == StatusKind::Renamed {
                let d = entry.index_to_workdir();
                rename_paths(
                    d.as_ref()
                        .and_then(|d| d.new_file().path())
                        .map(|p| p.to_string_lossy().into_owned()),
                    d.as_ref()
                        .and_then(|d| d.old_file().path())
                        .map(|p| p.to_string_lossy().into_owned()),
                    entry.path()?.to_string(),
                )
            } else {
                (entry.path()?.to_string(), None)
            };

            files.push(FileStatus {
                path,
                orig,
                staged,
                unstaged,
                untracked,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));

        let branch = self.branch().ok();
        Ok(RepoStatus { branch, files })
    }

    /// Cheap pre-filter (PART A fix, item 3): `true` when at least one of the
    /// (absolute) `paths` is a TRACKED file (present in the git index). Used
    /// to skip the expensive `refresh_magit` (git status + per-file diffs)
    /// when a watcher batch touches only untracked / ignored files, which do
    /// not appear in the magit status's tracked-file sections. Repo-relative
    /// resolution via `root` (the workdir). A fresh untracked file is NOT
    /// tracked, so an all-untracked batch is skipped (re-run on the next
    /// tracked change or a manual `g`).
    pub fn any_tracked(&self, paths: &[std::path::PathBuf], root: &Path) -> bool {
        let Ok(index) = self.inner.index() else {
            return false;
        };
        for abs in paths {
            if let Ok(rel) = abs.strip_prefix(root) {
                // Skip empty relative paths (a changed path that IS the root,
                // or a directory) — `index.get_path("")` errors with
                // "repo path should not be empty".
                if !rel.as_os_str().is_empty() && index.get_path(rel, 0).is_some() {
                    return true;
                }
            }
        }
        false
    }

    /// A single file's unified diff on one side (staged or unstaged).
    /// An empty diff (no change on that side) yields an empty `FileDiff`.
    pub fn diff(&self, side: DiffSide, path: &str) -> Result<FileDiff, GitError> {
        let mut dopts = git2::DiffOptions::new();
        dopts.pathspec(path)
            .disable_pathspec_match(true); // literal path, not a fnmatch pattern
        let diff = match side {
            DiffSide::Staged => {
                let head_tree = self.head_tree()?;
                self.inner
                    .diff_tree_to_index(head_tree.as_ref(), None, Some(&mut dopts))?
            }
            DiffSide::Unstaged => self.inner.diff_index_to_workdir(None, Some(&mut dopts))?,
        };
        Ok(extract(&diff, path))
    }
}

/// The canonical (new) path for a rename delta, plus the pre-rename (old)
/// path. `fallback` is the entry path when the new path is unavailable.
fn rename_paths(new: Option<String>, old: Option<String>, fallback: String) -> (String, Option<String>) {
    (new.unwrap_or(fallback), old)
}

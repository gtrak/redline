//! The git repository wrapper: opens/discovers a repo from the project
//! root and exposes the magit-subset operations (status, branch, per-file
//! diffs, file + hunk staging) as plain data. git2 types never leave this
//! module.
//!
//! Threading: `git2::Repository` is `Send` (not `Sync`), and the whole
//! `GitRepo` lives behind the store's `Mutex`, so it is safe to hold here.

use std::path::Path;

use crate::git::diff::{DiffLine, DiffOrigin, DiffSide, FileDiff, DiffHunk, extract};
use crate::git::error::GitError;
use crate::git::status::{BranchInfo, FileStatus, RepoStatus, Side, StatusKind};

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

    /// Stage (add) one file from the workdir to the index. Works for new,
    /// modified, and (re-adding) tracked files. When the file has been
    /// deleted from the workdir, `add_path` fails with `NotFound`; in
    /// that case the index entry is removed to stage the deletion (the
    /// same behavior as `git add <deleted-file>`).
    pub fn stage_file(&self, path: &str) -> Result<(), GitError> {
        let mut index = self.inner.index()?;
        match index.add_path(Path::new(path)) {
            Ok(()) => {}
            Err(e) if e.code() == git2::ErrorCode::NotFound => {
                index.remove_path(Path::new(path))?;
            }
            Err(e) => return Err(e.into()),
        }
        index.write()?;
        Ok(())
    }

    /// Unstage one file: reset its index entry to HEAD. A staged addition
    /// is removed from the index; a staged modification/deletion reverts to
    /// HEAD's version. `orig` is the pre-rename path for a staged rename.
    pub fn unstage_file(&self, path: &str, orig: Option<&str>) -> Result<(), GitError> {
        let head_tree = self.head_tree()?;
        let head_oid = head_tree.as_ref().map(|t| t.id());

        if find_path_in_tree(&self.inner, head_oid, path).is_some() {
            // HEAD has this path: restore the index entry to HEAD's version.
            self.reset_index_entry_to_head(path)?;
        } else if let Some(orig) = orig {
            // A staged rename whose new path is not in HEAD: restore the
            // original path (if HEAD still has it) and drop the new one.
            if find_path_in_tree(&self.inner, head_oid, orig).is_some() {
                self.reset_index_entry_to_head(orig)?;
            }
            self.remove_index_entry(path)?;
        } else {
            // A staged addition (or a change with no HEAD counterpart): drop
            // the index entry.
            self.remove_index_entry(path)?;
        }
        Ok(())
    }

    /// Stage one hunk of a file: apply that hunk's patch to the index only
    /// (`git apply --cached`), leaving the workdir untouched. The hunk is
    /// matched by its new-side (workdir) start line.
    pub fn stage_hunk(&self, path: &str, target_new_start: u32) -> Result<(), GitError> {
        let raw = self.raw_diff(DiffSide::Unstaged, path)?;
        self.find_hunk_in(&raw, path, target_new_start)?;
        // Dry-run validation, then the real apply. `apply(…, Index)` writes
        // the index to disk itself, so no `index.write()` afterwards.
        self.apply_hunk_to_index(&raw, target_new_start, true)?;
        self.apply_hunk_to_index(&raw, target_new_start, false)?;
        Ok(())
    }

    /// Unstage one hunk of a file: reverse-apply the hunk to the index only,
    /// reverting just that hunk's index content back toward HEAD while
    /// leaving the rest of the index and the whole workdir untouched.
    /// Whether the blob on the requested diff side of `path` ends with a
    /// newline (false when the file is absent on that side). The diff markers
    /// under-determine this in the both-sides-lack-LF shape, so the splice
    /// paths derive it from the actual blob bytes. In both diffs the
    /// requested side is `old_file()`: HEAD in the staged diff (HEAD ->
    /// index), the index in the unstaged diff (index -> workdir).
    fn blob_side_ends_with_newline(
        &self,
        side: DiffSide,
        path: &str,
    ) -> Result<bool, GitError> {
        let raw = self.raw_diff(side, path)?;
        match raw.get_delta(0).map(|d| d.old_file().exists()) {
            Some(true) => {
                let blob = self
                    .inner
                    .find_blob(raw.get_delta(0).unwrap().old_file().id())?
                    .content()
                    .to_vec();
                Ok(blob.ends_with(b"\n"))
            }
            _ => Ok(false),
        }
    }

    /// Find the hunk of `path`'s diff (already materialized in `raw`) whose
    /// new-side start line matches `target_new_start`, else `HunkNotFound`.
    fn find_hunk_in(
        &self,
        raw: &git2::Diff,
        path: &str,
        target_new_start: u32,
    ) -> Result<DiffHunk, GitError> {
        extract(raw, path)
            .hunks
            .iter()
            .find(|h| h.new_start == target_new_start)
            .cloned()
            .ok_or(GitError::HunkNotFound {
                file: path.to_string(),
                start: target_new_start,
            })
    }



    pub fn unstage_hunk(&self, path: &str, target_new_start: u32) -> Result<(), GitError> {
        // Staged diff: HEAD (old) vs index (new). The new side is the index.
        let raw = self.raw_diff(DiffSide::Staged, path)?;
        let hunk = self.find_hunk_in(&raw, path, target_new_start)?;

        let mut index = self.inner.index()?;
        let existing = index
            .get_path(Path::new(path), 0)
            .map(copy_index_entry)
            .ok_or(GitError::IndexEntryNotFound(path.to_string()))?;
        let content: Vec<u8> = {
            let blob = self.inner.find_blob(existing.id)?;
            blob.content().to_vec()
        };
        // The reverse-apply splices this hunk's old-side (HEAD) line text —
        // carried as strings in the diff model — back into the index blob.
        // Any non-UTF-8 byte on either side would be silently replaced with
        // U+FFFD, so refuse such files rather than corrupt the index.
        ensure_utf8(&content, path)?;
        // The old side (HEAD blob)'s trailing-newline truth drives the
        // splice's final-line handling; the diff markers under-determine it.
        let old_ends_nl = match raw.get_delta(0).map(|d| d.old_file().exists()) {
            Some(true) => {
                let old_content: Vec<u8> = self
                    .inner
                    .find_blob(raw.get_delta(0).unwrap().old_file().id())?
                    .content()
                    .to_vec();
                ensure_utf8(&old_content, path)?;
                old_content.ends_with(b"\n")
            }
            _ => false, // no old file: the old side is empty (pure addition)
        };

        let content_str = String::from_utf8(content).expect("checked UTF-8 above");
        // Rebuild the index content with this hunk's new-side span replaced
        // by the hunk's old-side (HEAD) lines, byte-exact.
        let new_content = revert_hunk_in_content(content_str.as_bytes(), &hunk, old_ends_nl);
        // PART A fix (item 5): a fully-staged-added file (no HEAD counterpart)
        // has one hunk spanning its whole content; reverting it empties the
        // index blob. Removing the index entry restores the file to untracked
        // (matching `git reset HEAD <file>`) instead of writing an empty blob.
        let old_exists = raw
            .get_delta(0)
            .map(|d| d.old_file().exists())
            .unwrap_or(false);
        if new_content.is_empty() && !old_exists {
            index.remove_path(Path::new(path))?;
            index.write()?;
            return Ok(());
        }
        let new_oid = self.inner.blob(&new_content)?;

        let mut new_entry = existing;
        new_entry.id = new_oid;
        index.add_frombuffer(&new_entry, &new_content)?;
        index.write()?;
        Ok(())
    }

    // ── discard (issue 002: magit `k`) ──────────────────────────────────

    /// Discard an UNSTAGED file: restore the workdir file to its index
    /// version. For a pure unstaged change the index equals HEAD, so this
    /// restores the file to HEAD. When the path is not in the index, the
    /// workdir file is removed.
    pub fn discard_unstaged_file(&self, path: &str) -> Result<(), GitError> {
        let index = self.inner.index()?;
        match index.get_path(Path::new(path), 0) {
            Some(e) => {
                let content: Vec<u8> = self.inner.find_blob(e.id)?.content().to_vec();
                self.write_workdir(path, &content)?;
            }
            None => {
                self.remove_workdir_file(path)?;
            }
        }
        Ok(())
    }

    /// Discard a STAGED file: reset the index entry to HEAD (or drop it for a
    /// staged addition) AND restore the workdir file to HEAD, so the change
    /// is gone from both index and workdir. `orig` is the pre-rename path for
    /// a staged rename.
    pub fn discard_staged_file(&self, path: &str, orig: Option<&str>) -> Result<(), GitError> {
        let head_tree = self.head_tree()?;
        let head_oid = head_tree.as_ref().map(|t| t.id());
        if head_tree.is_some() && find_path_in_tree(&self.inner, head_oid, path).is_some() {
            // HEAD has this path: index → HEAD and workdir → HEAD.
            self.reset_index_entry_to_head(path)?;
            let (oid, _) = find_path_in_tree(&self.inner, head_oid, path)
                .ok_or_else(|| GitError::IndexEntryNotFound(path.to_string()))?;
            let content: Vec<u8> = self.inner.find_blob(oid)?.content().to_vec();
            self.write_workdir(path, &content)?;
        } else if let Some(orig) = orig
            && head_tree.is_some()
            && find_path_in_tree(&self.inner, head_oid, orig).is_some()
        {
            // A staged rename whose new path is not in HEAD: restore the
            // original path (index + workdir) and drop the new one.
            self.reset_index_entry_to_head(orig)?;
            let (oid, _) = find_path_in_tree(&self.inner, head_oid, orig)
                .ok_or_else(|| GitError::IndexEntryNotFound(orig.to_string()))?;
            let content: Vec<u8> = self.inner.find_blob(oid)?.content().to_vec();
            self.write_workdir(orig, &content)?;
            self.remove_index_entry(path)?;
            self.remove_workdir_file(path)?;
        } else {
            // A staged addition (or a change with no HEAD counterpart): drop
            // the index entry and remove the workdir file.
            self.remove_index_entry(path)?;
            self.remove_workdir_file(path)?;
        }
        Ok(())
    }

    /// Discard an UNTRACKED file: remove the workdir file (magit deletes
    /// untracked files on discard).
    pub fn discard_untracked_file(&self, path: &str) -> Result<(), GitError> {
        self.remove_workdir_file(path)
    }

    /// Discard a HUNK: revert that hunk in the workdir (matched by its
    /// new-side start line). For an unstaged hunk the hunk comes from the
    /// index→workdir diff and only the workdir is reverted. For a staged
    /// hunk the hunk comes from the HEAD→index diff; the workdir is reverted
    /// to HEAD for that hunk AND the hunk is unstaged, so the change is
    /// removed from both workdir and index (magit semantics).
    pub fn discard_hunk(
        &self,
        path: &str,
        side: Option<Side>,
        target_new_start: u32,
    ) -> Result<(), GitError> {
        match side {
            Some(Side::Staged) => {
                let raw = self.raw_diff(DiffSide::Staged, path)?;
                let hunk = self.find_hunk_in(&raw, path, target_new_start)?;
                let old_ends_nl = self.blob_side_ends_with_newline(DiffSide::Staged, path)?;
                self.reverse_apply_hunk_to_workdir(path, &hunk, old_ends_nl)?;
                self.unstage_hunk(path, target_new_start)?;
                Ok(())
            }
            _ => {
                let raw = self.raw_diff(DiffSide::Unstaged, path)?;
                let hunk = self.find_hunk_in(&raw, path, target_new_start)?;
                let old_ends_nl = self.blob_side_ends_with_newline(DiffSide::Unstaged, path)?;
                self.revert_hunk_in_workdir(path, &hunk, old_ends_nl)
            }
        }
    }

    /// Rebuild the workdir file's content with one hunk's new-side line span
    /// replaced by the hunk's old-side lines (byte-exact), then write it back.
    /// Used for unstaged hunks where the line numbers are authoritative
    /// (the diff IS index→workdir, so `new_start` is a workdir line number).
    fn revert_hunk_in_workdir(
        &self,
        path: &str,
        hunk: &DiffHunk,
        old_ends_nl: bool,
    ) -> Result<(), GitError> {
        let p = self.workdir_path(path);
        let content = std::fs::read(&p)
            .map_err(|e| GitError::ReadFile {
                path: path.to_string(),
                source: e,
            })?;
        ensure_utf8(&content, path)?;
        let new_content = revert_hunk_in_content(&content, hunk, old_ends_nl);
        self.write_workdir(path, &new_content)
    }

    /// Reverse-apply a staged hunk to the workdir by context-matching the
    /// hunk's new-side (index) content within the workdir file and replacing
    /// it with the hunk's old-side (HEAD) content. This is safe when
    /// unstaged changes above or below the staged hunk have shifted line
    /// numbers: the splice targets the actual text, not a line offset.
    fn reverse_apply_hunk_to_workdir(
        &self,
        path: &str,
        hunk: &DiffHunk,
        old_ends_nl: bool,
    ) -> Result<(), GitError> {
        let p = self.workdir_path(path);
        let content = std::fs::read(&p)
            .map_err(|e| GitError::ReadFile {
                path: path.to_string(),
                source: e,
            })?;
        ensure_utf8(&content, path)?;
        let new_content =
            reverse_apply_hunk_in_content(&content, hunk, old_ends_nl).ok_or(GitError::HunkNotFound {
                file: path.to_string(),
                start: hunk.new_start,
            })?;
        self.write_workdir(path, &new_content)
    }

    /// The absolute workdir path for a repo-relative `path`.
    fn workdir_path(&self, path: &str) -> std::path::PathBuf {
        let root = self
            .inner
            .workdir()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        root.join(path)
    }

    /// Write `content` to the workdir file at `path`, creating parent
    /// directories as needed.
    fn write_workdir(&self, path: &str, content: &[u8]) -> Result<(), GitError> {
        let p = self.workdir_path(path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| GitError::WriteFile {
                    path: path.to_string(),
                    source: e,
                })?;
        }
        std::fs::write(&p, content).map_err(|e| GitError::WriteFile {
            path: path.to_string(),
            source: e,
        })
    }

    /// Remove the workdir file at `path`; a missing file is not an error.
    fn remove_workdir_file(&self, path: &str) -> Result<(), GitError> {
        let p = self.workdir_path(path);
        match std::fs::remove_file(&p) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(GitError::WriteFile {
                path: path.to_string(),
                source: e,
            }),
        }
    }

    // ── internals ─────────────────────────────────────────────────────

    /// The HEAD commit's root tree, or `None` on an unborn branch.
    fn head_tree(&self) -> Result<Option<git2::Tree<'_>>, GitError> {
        match self.inner.head() {
            Ok(ref_) => Ok(Some(ref_.peel_to_commit()?.tree()?)),
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// The raw libgit2 `Diff` for one file on one side (pathspec'd to that
    /// literal path).
    fn raw_diff(&self, side: DiffSide, path: &str) -> Result<git2::Diff<'_>, GitError> {
        let mut dopts = git2::DiffOptions::new();
        dopts.pathspec(path)
            .disable_pathspec_match(true);
        match side {
            DiffSide::Staged => {
                let head_tree = self.head_tree()?;
                Ok(self
                    .inner
                    .diff_tree_to_index(head_tree.as_ref(), None, Some(&mut dopts))?)
            }
            DiffSide::Unstaged => Ok(self.inner.diff_index_to_workdir(None, Some(&mut dopts))?),
        }
    }

    /// Apply a single hunk (matched by `new_start`) of `diff` to the index
    /// only. `check` runs a dry run first when true.
    fn apply_hunk_to_index(
        &self,
        diff: &git2::Diff,
        target: u32,
        check: bool,
    ) -> Result<(), GitError> {
        let mut aopts = git2::ApplyOptions::new();
        if check {
            aopts.check(true);
        }
        aopts.hunk_callback(move |hunk| hunk.map(|h| h.new_start() == target).unwrap_or(true));
        self.inner
            .apply(diff, git2::ApplyLocation::Index, Some(&mut aopts))?;
        Ok(())
    }

    /// Reset an index entry to the version of `path` stored in HEAD.
    fn reset_index_entry_to_head(&self, path: &str) -> Result<(), GitError> {
        let head_tree = self.head_tree()?;
        let tree = head_tree.as_ref().ok_or(GitError::NoHead)?;
        let (oid, mode) = find_path_in_tree(&self.inner, Some(tree.id()), path)
            .ok_or_else(|| GitError::IndexEntryNotFound(path.to_string()))?;
        let content: Vec<u8> = {
            let blob = self.inner.find_blob(oid)?;
            blob.content().to_vec()
        };

        let mut index = self.inner.index()?;
        // Copy the existing entry (carrying its path/flags); fall back to a
        // freshly built entry when the path is not yet in the index.
        let base = index
            .get_path(Path::new(path), 0)
            .map(copy_index_entry)
            .unwrap_or_else(|| default_entry(path, mode));
        let mut new_entry = base;
        new_entry.id = oid;
        new_entry.mode = mode;
        index.add_frombuffer(&new_entry, &content)?;
        index.write()?;
        Ok(())
    }

    /// Remove an index entry (dropping a staged addition, or one side of a
    /// rename).
    fn remove_index_entry(&self, path: &str) -> Result<(), GitError> {
        let mut index = self.inner.index()?;
        index.remove_path(Path::new(path))?;
        index.write()?;
        Ok(())
    }
}

/// The canonical (new) path for a rename delta, plus the pre-rename (old)
/// path. `fallback` is the entry path when the new path is unavailable.
fn rename_paths(new: Option<String>, old: Option<String>, fallback: String) -> (String, Option<String>) {
    (new.unwrap_or(fallback), old)
}

/// Rebuild a file's content (bytes) with one hunk's new-side line span
/// replaced by the hunk's old-side lines (context + deletion, in order),
/// byte-exactly. This is the reverse-apply step for unstaging a single
/// hunk. Every old-side line is re-terminated with `\n` except the old
/// file's last line when it lacked a trailing newline (`old_ends_nl`
/// false — libgit2's EOFNL marker, which `extract` does not store as a
/// line).
fn revert_hunk_in_content(
    content: &[u8],
    hunk: &DiffHunk,
    old_ends_nl: bool,
) -> Vec<u8> {
    let lines = split_lines_inclusive(content);
    let start0 = (hunk.new_start as usize).saturating_sub(1); // 0-based first line of the new-side span
    let end0 = (start0 + hunk.new_lines as usize).min(lines.len()); // 0-based exclusive end

    let old: Vec<&DiffLine> = hunk
        .lines
        .iter()
        .filter(|l| l.origin != DiffOrigin::Addition)
        .collect();

    let mut out: Vec<u8> = Vec::new();
    for line in &lines[..start0] {
        out.extend_from_slice(line);
    }
    for (i, l) in old.iter().enumerate() {
        out.extend_from_slice(l.content.as_bytes());
        let is_final_old_line = !old_ends_nl && i + 1 == old.len();
        if !is_final_old_line {
            out.push(b'\n');
        }
    }
    for line in &lines[end0..] {
        out.extend_from_slice(line);
    }
    out
}

/// Reverse-apply a staged hunk to the workdir content by finding the
/// hunk's new-side (index) line block in `content` and replacing it with
/// the hunk's old-side (HEAD) line block. Returns `None` when the
/// new-side text is not found (the workdir has diverged from the index
/// in this hunk's region, so the reverse-apply cannot be performed
/// safely).
///
/// The new-side text is the concatenation of all non-Deletion lines
/// (Context + Addition) in order, each terminated with `\n` except the
/// last new-side line when `new_ends_nl` is false. The old-side text is
/// the concatenation of all non-Addition lines (Context + Deletion) in
/// order, each terminated with `\n` except the last old-side line when
/// `old_ends_nl` is false.
fn reverse_apply_hunk_in_content(
    content: &[u8],
    hunk: &DiffHunk,
    old_ends_nl: bool,
) -> Option<Vec<u8>> {
    // Build the new-side text: all lines that appear in the new (index) file.
    let new_lines: Vec<&DiffLine> = hunk
        .lines
        .iter()
        .filter(|l| l.origin != DiffOrigin::Deletion)
        .collect();
    // The new side's trailing-newline shape cannot be trusted from the
    // diff markers alone (xdiff keys the marker to the last hunk line's
    // prefix, which diverges from the semantic truth when BOTH sides lack
    // the LF). Try the full form (every line newline-terminated) first;
    // fall back to the trimmed form (final line without the newline) --
    // whichever actually matches the workdir bytes is the truth.
    let build_new_side = |trimmed: bool| -> Vec<u8> {
        let mut v = Vec::new();
        for (i, line) in new_lines.iter().enumerate() {
            v.extend_from_slice(line.content.as_bytes());
            let is_final = trimmed && i + 1 == new_lines.len();
            if !is_final {
                v.push(b'\n');
            }
        }
        v
    };
    let mut new_side = build_new_side(false);
    if !content.windows(new_side.len()).any(|w| w == new_side.as_slice()) {
        let trimmed = build_new_side(true);
        if content.windows(trimmed.len()).any(|w| w == trimmed.as_slice()) {
            new_side = trimmed;
        } else {
            return None;
        }
    }

    // A pure-deletion hunk has an empty new side; fall back to line-number
    // splicing (correct when line numbers are not shifted).
    if new_side.is_empty() {
        return Some(revert_hunk_in_content(content, hunk, old_ends_nl));
    }

    // The old side's trailing-newline truth comes from the caller, which
    // reads the actual old-side (HEAD) blob -- the diff markers under-
    // determine it in the both-sides-lack-LF shape.
    let old_lines: Vec<&DiffLine> = hunk
        .lines
        .iter()
        .filter(|l| l.origin != DiffOrigin::Addition)
        .collect();
    let mut old_side: Vec<u8> = Vec::new();
    for (i, l) in old_lines.iter().enumerate() {
        old_side.extend_from_slice(l.content.as_bytes());
        let is_final_old_line = !old_ends_nl && i + 1 == old_lines.len();
        if !is_final_old_line {
            old_side.push(b'\n');
        }
    }

    // Find the new-side text in the workdir content.
    let pos = content
        .windows(new_side.len())
        .position(|w| w == new_side.as_slice())?;

    // Guard against silent line merge: if the old side's final line lacks a
    // trailing newline but the workdir has content immediately after the
    // matched region, splicing would merge two lines (e.g. "endY").
    // `git apply` refuses here; we do too.
    if !old_ends_nl && pos + new_side.len() < content.len() {
        return None;
    }

    // Splice: content[..pos] + old_side + content[pos+new_side.len()..]
    let mut out = Vec::with_capacity(content.len() + old_side.len() - new_side.len());
    out.extend_from_slice(&content[..pos]);
    out.extend_from_slice(&old_side);
    out.extend_from_slice(&content[pos + new_side.len()..]);
    Some(out)
}

/// Split `bytes` into lines, each keeping its trailing `\n` (the final
/// line keeps its absence of one). A byte-exact partition: concatenating
/// the result reproduces `bytes`.
fn split_lines_inclusive(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        match bytes[start..].iter().position(|b| *b == b'\n') {
            Some(i) => {
                out.push(&bytes[start..start + i + 1]);
                start += i + 1;
            }
            None => {
                out.push(&bytes[start..]);
                break;
            }
        }
    }
    out
}

/// Fail with `GitError::NotUtf8` when `content` is not valid UTF-8.
fn ensure_utf8(content: &[u8], path: &str) -> Result<(), GitError> {
    std::str::from_utf8(content)
        .map(|_| ())
        .map_err(|_| GitError::NotUtf8(path.to_string()))
}

/// Copy an index entry by field (`git2::IndexEntry` is not `Clone` but is
/// an owned plain struct, so it moves freely).
fn copy_index_entry(e: git2::IndexEntry) -> git2::IndexEntry {
    git2::IndexEntry {
        ctime: e.ctime,
        mtime: e.mtime,
        dev: e.dev,
        ino: e.ino,
        mode: e.mode,
        uid: e.uid,
        gid: e.gid,
        file_size: e.file_size,
        id: e.id,
        flags: e.flags,
        flags_extended: e.flags_extended,
        path: e.path,
    }
}

/// Walk the tree at `root` (by oid, via `repo`) to `path` and return
/// `(oid, mode)` for the entry, or `None`.
fn find_path_in_tree(
    repo: &git2::Repository,
    root: Option<git2::Oid>,
    path: &str,
) -> Option<(git2::Oid, u32)> {
    let mut cur = root?;
    let mut rest = path;
    loop {
        let tree = repo.find_tree(cur).ok()?;
        let (name, next) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, ""),
        };
        let entry = tree.get_name(name)?;
        let oid = entry.id();
        let mode = entry.filemode(); // i32 in git2 0.21
        if mode == 0o40000 {
            // A directory (tree) entry: descend into it.
            if next.is_empty() {
                return None;
            }
            cur = oid;
            rest = next;
        } else if rest == name {
            return Some((oid, mode as u32));
        } else {
            return None;
        }
    }
}

/// A minimal index entry (path + mode), used when restoring a path that is
/// not currently in the index.
fn default_entry(path: &str, mode: u32) -> git2::IndexEntry {
    let path_bytes = path.as_bytes();
    let (flags, flags_extended) = if path_bytes.len() < 64 {
        ((path_bytes.len() as u16) & 0xff, 0)
    } else {
        (0, path_bytes.len() as u16)
    };
    git2::IndexEntry {
        ctime: git2::IndexTime::new(0, 0),
        mtime: git2::IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode,
        uid: 0,
        gid: 0,
        file_size: 0,
        id: git2::Oid::ZERO_SHA1,
        flags,
        flags_extended,
        path: path_bytes.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::diff::DiffOrigin;
    use crate::model::sections::StatusTree;
    use std::collections::HashMap;
    use std::path::Path;

    /// Run the git CLI in `dir`, with an isolated global config and
    /// committed author/committer identity so the tests are hermetic.
    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn init_repo(dir: &Path) -> GitRepo {
        git(dir, &["init", "-q", "-b", "main"]);
        git(dir, &["config", "user.name", "Test"]);
        git(dir, &["config", "user.email", "test@example.com"]);
        git(dir, &["config", "commit.gpgsign", "false"]);
        GitRepo::discover(dir).expect("discover the repo")
    }

    /// Build a `StatusTree` the same way the store does (status + per-file
    /// diffs).
    fn build_tree(g: &GitRepo) -> StatusTree {
        let status = g.status().unwrap();
        let mut staged = HashMap::new();
        let mut unstaged = HashMap::new();
        for f in &status.files {
            if f.is_staged() && let Ok(d) = g.diff(DiffSide::Staged, &f.path) {
                staged.insert(f.path.clone(), d);
            }
            if f.is_unstaged() && let Ok(d) = g.diff(DiffSide::Unstaged, &f.path) {
                unstaged.insert(f.path.clone(), d);
            }
        }
        StatusTree::build(&status, &staged, &unstaged, None)
    }

    #[test]
    fn status_classification_matches_git_cli() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);

        // Baseline: a.txt, e.txt, and f.txt are committed.
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        std::fs::write(root.join("e.txt"), "e\n").unwrap();
        std::fs::write(root.join("f.txt"), "f\n").unwrap();
        git(root, &["add", "a.txt"]);
        git(root, &["add", "e.txt"]);
        git(root, &["add", "f.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        // a.txt: unstaged modification only
        std::fs::write(root.join("a.txt"), "A\n").unwrap();
        // c.txt: staged addition
        std::fs::write(root.join("c.txt"), "c\n").unwrap();
        git(root, &["add", "c.txt"]);
        // d.txt: untracked
        std::fs::write(root.join("d.txt"), "d\n").unwrap();
        // staged rename: HEAD has e.txt, index has e2.txt
        git(root, &["mv", "e.txt", "e2.txt"]);
        // f.txt: staged AND unstaged
        std::fs::write(root.join("f.txt"), "F1\n").unwrap();
        git(root, &["add", "f.txt"]);
        std::fs::write(root.join("f.txt"), "F2\n").unwrap();

        let status = g.status().unwrap();

        // Cross-check the number of changed entries against the git CLI
        // (rename detection forced on so a rename is one entry, as in the
        // wrapper).
        let porcelain = git(root, &["-c", "status.renames=true", "status", "--porcelain"]);
        let porcelain_lines: Vec<&str> = porcelain.lines().collect();
        assert_eq!(
            porcelain_lines.len(),
            status.files.len(),
            "porcelain:\n{porcelain}\nwrapper:\n{status:#?}"
        );

        let find = |p: &str| -> &FileStatus {
            status
                .files
                .iter()
                .find(|f| f.path == p)
                .unwrap_or_else(|| panic!("missing {p} in {status:#?}"))
        };
        assert_eq!(find("a.txt").unstaged, StatusKind::Modified);
        assert_eq!(find("a.txt").staged, StatusKind::None);
        assert_eq!(find("c.txt").staged, StatusKind::Added);
        assert_eq!(find("c.txt").unstaged, StatusKind::None);
        assert!(find("d.txt").untracked);
        assert_eq!(find("e2.txt").staged, StatusKind::Renamed);
        assert_eq!(find("e2.txt").orig, Some("e.txt".into()));
        assert_eq!(find("f.txt").staged, StatusKind::Modified);
        assert_eq!(find("f.txt").unstaged, StatusKind::Modified);

        assert_eq!(status.staged_count(), 3);   // c, e2, f
        assert_eq!(status.unstaged_count(), 2); // a, f
        assert_eq!(status.untracked_count(), 1); // d
    }

    #[test]
    fn stage_file_then_unstage_matches_cli() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        std::fs::write(root.join("a.txt"), "a\nb\nc\n").unwrap();
        git(root, &["add", "a.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        std::fs::write(root.join("a.txt"), "a\nB\nc\n").unwrap();
        g.stage_file("a.txt").unwrap();

        // wrapper staged diff agrees with `git diff --cached`
        let staged = g.diff(DiffSide::Staged, "a.txt").unwrap();
        assert_eq!(staged.hunks.len(), 1);
        assert!(staged.hunks[0]
            .lines
            .iter()
            .any(|l| l.content == "B" && l.origin == DiffOrigin::Addition));
        assert!(staged.hunks[0]
            .lines
            .iter()
            .any(|l| l.content == "b" && l.origin == DiffOrigin::Deletion));
        let cached = git(root, &["diff", "--cached"]);
        assert!(cached.contains("+B"), "cached:\n{cached}");
        assert!(cached.contains("-b"), "cached:\n{cached}");
        // workdir now equals index: unstaged diff empty
        assert!(git(root, &["diff"]).trim().is_empty());

        // unstage: index reverts to HEAD, workdir keeps the change
        g.unstage_file("a.txt", None).unwrap();
        assert!(
            git(root, &["diff", "--cached"]).trim().is_empty(),
            "after unstage, --cached must be empty"
        );
        assert!(git(root, &["diff"]).contains("+B"));
    }

    #[test]
    fn stage_middle_hunk_stages_only_that_hunk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        let base: Vec<String> = (1..=30).map(|i| format!("line{i:02}")).collect();
        std::fs::write(root.join("big.txt"), base.join("\n") + "\n").unwrap();
        git(root, &["add", "big.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        // Three far-apart edits → three hunks.
        let mut w = base.clone();
        w[1] = "MOD-2".into();
        w[14] = "MOD-15".into();
        w[27] = "MOD-28".into();
        std::fs::write(root.join("big.txt"), w.join("\n") + "\n").unwrap();

        let d = g.diff(DiffSide::Unstaged, "big.txt").unwrap();
        assert_eq!(d.hunks.len(), 3, "hunks:\n{d:#?}");
        let mid = d
            .hunks
            .iter()
            .find(|h| h.lines.iter().any(|l| l.content == "MOD-15"))
            .unwrap();
        g.stage_hunk("big.txt", mid.new_start).unwrap();

        // The index gained exactly the middle hunk.
        let cached = git(root, &["diff", "--cached"]);
        assert!(cached.contains("MOD-15"), "cached:\n{cached}");
        assert!(!cached.contains("MOD-28"), "cached leaked MOD-28:\n{cached}");
        assert!(!cached.contains("MOD-2\n"), "cached leaked MOD-2:\n{cached}");
        let staged = g.diff(DiffSide::Staged, "big.txt").unwrap();
        assert_eq!(staged.hunks.len(), 1, "staged:\n{staged:#?}");
        assert!(staged.hunks[0].lines.iter().any(|l| l.content == "MOD-15"));
        // The workdir still carries all three edits (index-only staging).
        let workdir = git(root, &["diff"]);
        assert!(
            workdir.contains("MOD-2\n") && workdir.contains("MOD-28"),
            "workdir:\n{workdir}"
        );
    }

    #[test]
    fn unstage_middle_hunk_reverts_index_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        let base: Vec<String> = (1..=30).map(|i| format!("line{i:02}")).collect();
        std::fs::write(root.join("big.txt"), base.join("\n") + "\n").unwrap();
        git(root, &["add", "big.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        let mut w = base.clone();
        w[1] = "MOD-2".into();
        w[14] = "MOD-15".into();
        w[27] = "MOD-28".into();
        std::fs::write(root.join("big.txt"), w.join("\n") + "\n").unwrap();
        g.stage_file("big.txt").unwrap(); // stage all three

        let staged = g.diff(DiffSide::Staged, "big.txt").unwrap();
        assert_eq!(staged.hunks.len(), 3);
        let mid = staged
            .hunks
            .iter()
            .find(|h| h.lines.iter().any(|l| l.content == "MOD-15"))
            .unwrap();
        g.unstage_hunk("big.txt", mid.new_start).unwrap();

        // The index lost the middle hunk; the other two remain staged.
        let staged2 = g.diff(DiffSide::Staged, "big.txt").unwrap();
        assert_eq!(staged2.hunks.len(), 2, "staged after unstage:\n{staged2:#?}");
        assert!(!staged2
            .hunks
            .iter()
            .any(|h| h.lines.iter().any(|l| l.content == "MOD-15")));
        let cached = git(root, &["diff", "--cached"]);
        assert!(!cached.contains("MOD-15"), "cached:\n{cached}");
        assert!(cached.contains("MOD-28"), "cached:\n{cached}");
        assert!(cached.contains("MOD-2\n"), "cached must keep hunk1:\n{cached}");
        // The workdir still carries the middle edit.
        assert!(git(root, &["diff"]).contains("MOD-15"));
    }

    #[test]
    fn unstage_hunk_on_no_trailing_newline_file_keeps_index_exact() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        // 30 lines; the last line (line30) has NO trailing newline.
        let base: Vec<String> = (1..=30).map(|i| format!("line{i:02}")).collect();
        let base_text: String = base[..29].join("\n") + "\n" + base[29].as_str();
        std::fs::write(root.join("big.txt"), &base_text).unwrap();
        git(root, &["add", "big.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        // Three far-apart edits; the last touches the file's final line.
        let mut w = base.clone();
        w[1] = "MOD-2".into();
        w[14] = "MOD-15".into();
        w[29] = "MOD-30".into();
        let w_text: String = w[..29].join("\n") + "\n" + w[29].as_str();
        std::fs::write(root.join("big.txt"), &w_text).unwrap();

        let d = g.diff(DiffSide::Unstaged, "big.txt").unwrap();
        assert_eq!(d.hunks.len(), 3, "hunks:\n{d:#?}");
        g.stage_file("big.txt").unwrap(); // stage all three

        // Unstage the hunk that touches the old file's final line.
        let staged = g.diff(DiffSide::Staged, "big.txt").unwrap();
        let last = staged
            .hunks
            .iter()
            .find(|h| h.lines.iter().any(|l| l.content == "MOD-30"))
            .expect("the final-line hunk must be staged");
        // Marker note: xdiff keys the EOFNL marker to the last hunk line's
        // prefix ('+' here -> DeleteEOFNL) even when BOTH sides lack the
        // trailing LF, so the marker-derived flag is not the semantic truth
        // -- the byte-exact assertions below are the real contract.
        assert!(last.old_ends_nl, "DeleteEOFNL marker maps old_ends_nl=true");
        g.unstage_hunk("big.txt", last.new_start).unwrap();

        // The index blob must equal HEAD byte-for-byte except for the two
        // remaining hunks — including the final line's missing newline.
        let staged_blob = git(root, &["show", ":big.txt"]);
        let mut expected = base.clone();
        expected[1] = "MOD-2".into();
        expected[14] = "MOD-15".into();
        let expected_text: String =
            expected[..29].join("\n") + "\n" + expected[29].as_str();
        assert_eq!(
            staged_blob, expected_text,
            "index blob must be byte-exact"
        );

        // `git diff --cached` shows the remaining hunks and nothing else:
        // no MOD-30, and no leaked EOFNL marker text.
        let cached = git(root, &["diff", "--cached"]);
        assert!(cached.contains("MOD-2"), "cached:\n{cached}");
        assert!(cached.contains("MOD-15"), "cached:\n{cached}");
        assert!(!cached.contains("MOD-30"), "cached:\n{cached}");
        assert!(!cached.contains("No newline"), "marker leaked into index:\n{cached}");

        // The wrapper agrees: exactly the two remaining hunks, staged side.
        let staged2 = g.diff(DiffSide::Staged, "big.txt").unwrap();
        assert_eq!(staged2.hunks.len(), 2, "staged:\n{staged2:#?}");
        assert!(!staged2
            .hunks
            .iter()
            .any(|h| h.lines.iter().any(|l| l.content == "MOD-30")));
        // The workdir still carries the final-line edit (now unstaged).
        let workdir = git(root, &["diff"]);
        assert!(workdir.contains("MOD-30"), "workdir:\n{workdir}");
        // MOD-2 / MOD-15 are still staged: they do NOT appear in the
        // index-vs-workdir diff.
        assert!(!workdir.contains("MOD-15"), "workdir:\n{workdir}");
    }

    #[test]
    fn unstage_hunk_refuses_non_utf8_content() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        // A "text" file with an invalid UTF-8 byte (0xFF, no NULs): libgit2
        // still produces text hunks for it.
        std::fs::write(root.join("latin.txt"), b"a\xff\nb\n").unwrap();
        git(root, &["add", "latin.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);
        std::fs::write(root.join("latin.txt"), b"a\xff\nB\n").unwrap();
        g.stage_file("latin.txt").unwrap();

        let staged = g.diff(DiffSide::Staged, "latin.txt").unwrap();
        assert!(
            !staged.hunks.is_empty(),
            "non-UTF-8 text file must produce hunks"
        );
        let err = g
            .unstage_hunk("latin.txt", staged.hunks[0].new_start)
            .unwrap_err();
        assert!(
            matches!(err, GitError::NotUtf8(ref p) if p == "latin.txt"),
            "unexpected error: {err:?}"
        );
        // The index is untouched: the hunk is still staged.
        assert!(git(root, &["diff", "--cached"]).contains("+B"));
    }

    #[test]
    fn stage_deleted_file_stages_the_removal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        std::fs::write(root.join("gone.txt"), "content\n").unwrap();
        git(root, &["add", "gone.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        // Delete from workdir, then stage: the index entry must be removed.
        std::fs::remove_file(root.join("gone.txt")).unwrap();
        g.stage_file("gone.txt").unwrap();

        // `git diff --cached` must show the deletion.
        let cached = git(root, &["diff", "--cached"]);
        assert!(cached.contains("-content"), "cached:\n{cached}");
        // `git status --porcelain` must show it as staged-deleted ("D  ").
        let porcelain = git(root, &["status", "--porcelain"]);
        assert!(
            porcelain.starts_with("D ") || porcelain.starts_with("D\t"),
            "porcelain:\n{porcelain}"
        );
        // The unstaged diff must be empty (workdir matches index: both absent).
        assert!(git(root, &["diff"]).trim().is_empty());
    }

    #[test]
    fn staged_rename_reported_with_orig_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        std::fs::write(root.join("doc.txt"), "hello world\n").unwrap();
        git(root, &["add", "doc.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        git(root, &["mv", "doc.txt", "docs.txt"]);
        let status = g.status().unwrap();
        let entry = status
            .files
            .iter()
            .find(|f| f.path == "docs.txt")
            .expect("renamed entry must be present");
        assert_eq!(entry.staged, StatusKind::Renamed);
        assert_eq!(entry.orig, Some("doc.txt".into()));
        assert!(!status.files.iter().any(|f| f.path == "doc.txt"));
    }

    #[test]
    fn discard_unstaged_file_restores_workdir_to_head() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        std::fs::write(root.join("a.txt"), "keep\n").unwrap();
        git(root, &["add", "a.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        // Unstaged modification only.
        std::fs::write(root.join("a.txt"), "changed\n").unwrap();
        g.discard_unstaged_file("a.txt").unwrap();

        // Workdir restored to HEAD; index unchanged (still matches HEAD).
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "keep\n"
        );
        assert!(
            git(root, &["diff"]).trim().is_empty(),
            "workdir must match HEAD after discard"
        );
        assert!(
            git(root, &["diff", "--cached"]).trim().is_empty(),
            "index must be untouched"
        );
    }

    #[test]
    fn discard_staged_file_clears_index_and_workdir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        std::fs::write(root.join("a.txt"), "keep\n").unwrap();
        git(root, &["add", "a.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        // Staged modification.
        std::fs::write(root.join("a.txt"), "staged\n").unwrap();
        git(root, &["add", "a.txt"]);
        assert!(git(root, &["diff", "--cached"]).contains("+staged"));

        g.discard_staged_file("a.txt", None).unwrap();

        // Both index and workdir restored to HEAD.
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "keep\n"
        );
        assert!(
            git(root, &["diff", "--cached"]).trim().is_empty(),
            "index must match HEAD"
        );
        assert!(
            git(root, &["diff"]).trim().is_empty(),
            "workdir must match HEAD"
        );
    }

    #[test]
    fn discard_staged_addition_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        std::fs::write(root.join("base.txt"), "base\n").unwrap();
        git(root, &["add", "base.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        // Stage a brand-new file.
        std::fs::write(root.join("new.txt"), "added\n").unwrap();
        git(root, &["add", "new.txt"]);
        assert!(git(root, &["diff", "--cached"]).contains("+added"));

        g.discard_staged_file("new.txt", None).unwrap();

        // The index entry is gone and the workdir file is removed.
        assert!(
            git(root, &["diff", "--cached"]).trim().is_empty(),
            "staged addition must be dropped"
        );
        assert!(!root.join("new.txt").exists(), "workdir file must be removed");
    }

    #[test]
    fn discard_unstaged_hunk_reverts_only_that_hunk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        let base: Vec<String> = (1..=30).map(|i| format!("line{i:02}")).collect();
        std::fs::write(root.join("big.txt"), base.join("\n") + "\n").unwrap();
        git(root, &["add", "big.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        // Two far-apart edits → two hunks (both unstaged).
        let mut w = base.clone();
        w[1] = "MOD-2".into();
        w[20] = "MOD-21".into();
        std::fs::write(root.join("big.txt"), w.join("\n") + "\n").unwrap();

        let d = g.diff(DiffSide::Unstaged, "big.txt").unwrap();
        assert_eq!(d.hunks.len(), 2, "hunks:\n{d:#?}");
        let second = d
            .hunks
            .iter()
            .find(|h| h.lines.iter().any(|l| l.content == "MOD-21"))
            .unwrap();
        g.discard_hunk("big.txt", Some(Side::Unstaged), second.new_start)
            .unwrap();

        // Only the target hunk reverted in the workdir; the other survives.
        let after = std::fs::read_to_string(root.join("big.txt")).unwrap();
        assert!(
            after.contains("line21"),
            "the discarded hunk's line must be restored: {after}"
        );
        assert!(
            !after.contains("MOD-21"),
            "the discarded change must be gone: {after}"
        );
        assert!(
            after.contains("MOD-2"),
            "the untouched hunk must survive: {after}"
        );
        // The index is untouched (nothing was staged).
        assert!(
            git(root, &["diff", "--cached"]).trim().is_empty(),
            "index must be untouched"
        );
    }

    #[test]
    fn discard_staged_hunk_reverts_workdir_and_unstages() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        let base: Vec<String> = (1..=30).map(|i| format!("line{i:02}")) .collect();
        std::fs::write(root.join("big.txt"), base.join("\n") + "\n").unwrap();
        git(root, &["add", "big.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        // Two far-apart edits: stage the first, leave the second unstaged.
        let mut w = base.clone();
        w[1] = "STAGED-MOD".into(); // hunk 1: staged
        w[20] = "UNSTAGED-MOD".into(); // hunk 2: unstaged
        std::fs::write(root.join("big.txt"), w.join("\n") + "\n").unwrap();

        // Stage only the first hunk by committing the full workdir, then
        // re-write to get a clean state with staged + unstaged mixed.
        // Simpler: write both mods, stage the file, then add the second mod.
        // Actually the simplest is: write both mods, stage the file (both
        // staged), then add a third change (unstaged). But we want to test
        // the staged hunk path specifically.
        //
        // Cleanest approach: write the first mod, stage it. Write the second
        // mod without staging. Now hunk 1 is staged, hunk 2 is unstaged.
        // But git's diff will show them differently. Let's use a simpler
        // scenario: one staged change and one unstaged change in the same file.
        //
        // Reset: re-initialize.
        // Actually let me just do it in one shot: write both mods, stage the
        // file (both staged), then modify one line back to create an
        // unstaged delta. No, that gets complicated.
        //
        // Simplest: just test the pure staged case first (all changes staged),
        // then verify the workdir + index are correct.
        std::fs::write(root.join("big.txt"), base.join("\n") + "\n").unwrap();
        // Now make one change and stage it.
        let mut staged_only = base.clone();
        staged_only[1] = "STAGED-MOD".into();
        std::fs::write(root.join("big.txt"), staged_only.join("\n") + "\n").unwrap();
        git(root, &["add", "big.txt"]);
        assert!(
            git(root, &["diff", "--cached"]).contains("STAGED-MOD"),
            "staged change present in index"
        );

        // Get the staged diff and find the hunk.
        let d = g.diff(DiffSide::Staged, "big.txt").unwrap();
        assert_eq!(d.hunks.len(), 1, "one staged hunk: {d:#?}");
        let hunk = &d.hunks[0];

        g.discard_hunk("big.txt", Some(Side::Staged), hunk.new_start)
            .unwrap();

        // Workdir must match HEAD (the staged change is reverted in workdir).
        let after = std::fs::read_to_string(root.join("big.txt")).unwrap();
        assert!(
            after.contains("line02"),
            "workdir restored to HEAD: {after}"
        );
        assert!(
            !after.contains("STAGED-MOD"),
            "staged mod must be gone from workdir: {after}"
        );
        // Index must also be clean (the hunk was unstaged).
        assert!(
            git(root, &["diff", "--cached"]).trim().is_empty(),
            "index must match HEAD after staged-hunk discard"
        );
        assert!(
            git(root, &["diff"]).trim().is_empty(),
            "workdir must match HEAD after staged-hunk discard"
        );
    }

    #[test]
    fn discard_staged_hunk_with_mixed_staged_unstaged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        let base: Vec<String> = (1..=30).map(|i| format!("line{i:02}")) .collect();
        std::fs::write(root.join("big.txt"), base.join("\n") + "\n").unwrap();
        git(root, &["add", "big.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        // Hunk 1 (near line 2): STAGED.  Hunk 2 (near line 21): UNSTAGED.
        // Write both changes, stage the file, then add a further unstaged
        // edit to a different region. Actually: write both, stage the file
        // (both staged). Then we need one to be unstaged.
        //
        // The cleanest way: write mod 1, stage it. Write mod 2 (now
        // workdir has both, index has only mod 1).
        let mut w = base.clone();
        w[1] = "STAGED-MOD".into();
        std::fs::write(root.join("big.txt"), w.join("\n") + "\n").unwrap();
        git(root, &["add", "big.txt"]);
        // Now add the unstaged mod.
        let mut w2 = w.clone();
        w2[20] = "UNSTAGED-MOD".into();
        std::fs::write(root.join("big.txt"), w2.join("\n") + "\n").unwrap();

        // Verify: index has STAGED-MOD but not UNSTAGED-MOD.
        let cached = git(root, &["diff", "--cached"]);
        assert!(cached.contains("STAGED-MOD"), "index has staged mod");
        assert!(!cached.contains("UNSTAGED-MOD"), "index must not have unstaged mod");
        let unstaged = git(root, &["diff"]);
        assert!(unstaged.contains("UNSTAGED-MOD"), "workdir has unstaged mod");

        // Now discard the staged hunk.
        let d = g.diff(DiffSide::Staged, "big.txt").unwrap();
        assert_eq!(d.hunks.len(), 1, "one staged hunk: {d:#?}");
        g.discard_hunk("big.txt", Some(Side::Staged), d.hunks[0].new_start)
            .unwrap();

        // The staged change is gone from both index and workdir.
        let cached_after = git(root, &["diff", "--cached"]);
        assert!(!cached_after.contains("STAGED-MOD"), "staged mod unstaged");
        // The unstaged change survives in the workdir.
        let after = std::fs::read_to_string(root.join("big.txt")).unwrap();
        assert!(
            after.contains("UNSTAGED-MOD"),
            "unstaged mod must survive: {after}"
        );
        // Use exact-line match: "UNSTAGED-MOD" contains "STAGED-MOD" as a substring.
        assert!(
            !after.lines().any(|l| l == "STAGED-MOD"),
            "staged mod must be reverted from workdir: {after}"
        );
        // The unstaged diff still shows the surviving change.
        let unstaged_after = git(root, &["diff"]);
        assert!(unstaged_after.contains("UNSTAGED-MOD"), "unstaged mod still in workdir diff");
    }

    #[test]
    fn discard_staged_hunk_with_unstaged_insertion_above() {
        // Repro for the blocking finding: an unstaged insertion above the
        // staged hunk shifts line numbers, so the workdir splice must use
        // context matching, not the index-side line offset.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        let base: Vec<String> = (1..=30).map(|i| format!("line{i:02}")).collect();
        std::fs::write(root.join("big.txt"), base.join("\n") + "\n").unwrap();
        git(root, &["add", "big.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        // Stage a change to line 5 (index now has "STAGED-MOD" at line 5).
        let mut staged = base.clone();
        staged[4] = "STAGED-MOD".into();
        std::fs::write(root.join("big.txt"), staged.join("\n") + "\n").unwrap();
        git(root, &["add", "big.txt"]);

        // Now insert a line at the top of the workdir (unstaged). This shifts
        // all workdir line numbers by +1 relative to the index.
        let mut shifted = vec!["INSERTED".to_string()];
        shifted.extend(staged);
        std::fs::write(root.join("big.txt"), shifted.join("\n") + "\n").unwrap();

        // Verify: the index has the staged change, the workdir has the
        // insertion + the staged change (shifted down by one line).
        let cached = git(root, &["diff", "--cached"]);
        assert!(cached.contains("STAGED-MOD"), "staged mod in index");
        let unstaged = git(root, &["diff"]);
        assert!(unstaged.contains("INSERTED"), "insertion is unstaged");

        // Get the staged hunk's new_start (index-side line number).
        let d = g.diff(DiffSide::Staged, "big.txt").unwrap();
        assert_eq!(d.hunks.len(), 1, "one staged hunk: {d:#?}");
        let hunk_start = d.hunks[0].new_start;
        // In the index, the hunk starts at line 2 (3 context lines before
        // the change at line 5). In the workdir, it starts at line 3
        // (shifted by the insertion). The old code would splice at line 2
        // (wrong); the fix must find the content by context.
        assert_eq!(hunk_start, 2, "index-side hunk start (with context)");

        g.discard_hunk("big.txt", Some(Side::Staged), hunk_start)
            .unwrap();

        // The staged change must be gone from the workdir (reverted to
        // "line05"), but the insertion must survive.
        let after = std::fs::read_to_string(root.join("big.txt")).unwrap();
        // Byte-exact assertion: the workdir must be exactly "INSERTED\n" +
        // all 30 HEAD lines. This discriminates against the old
        // index-side line splice which would lose line01 and duplicate
        // line08.
        let expected = format!("INSERTED\n{}\n", base.join("\n"));
        assert_eq!(
            after, expected,
            "workdir must be byte-exact: INSERTED + all 30 HEAD lines"
        );
        // The index must also be clean (the hunk was unstaged).
        assert!(
            git(root, &["diff", "--cached"]).trim().is_empty(),
            "index must match HEAD after staged-hunk discard"
        );
        // The workdir diff must show only the insertion (not the staged mod).
        let unstaged_after = git(root, &["diff"]);
        assert!(unstaged_after.contains("INSERTED"));
        assert!(!unstaged_after.contains("STAGED-MOD"));
    }

    #[test]
    fn discard_untracked_file_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let _g = init_repo(root);
        std::fs::write(root.join("scratch.txt"), "untracked\n").unwrap();

        _g.discard_untracked_file("scratch.txt").unwrap();

        assert!(!root.join("scratch.txt").exists(), "file must be deleted");
    }

    
#[test]
    fn discard_staged_hunk_at_eof_no_trailing_newline() {
        // Regression test for the phantom trailing newline: when a staged
        // hunk reaches EOF of a file with no trailing newline, the
        // new-side search must NOT append a phantom \\n that isn't in the
        // workdir. Before the fix, `reverse_apply_hunk_in_content` would
        // fail with HunkNotFound because it searched for "a\nb\nB\nend\n"
        // when the workdir actually contains "a\nb\nB\nend".
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        // HEAD: "a\nb\nend" (no trailing newline).
        std::fs::write(root.join("f.txt"), "a\nb\nend").unwrap();
        git(root, &["add", "f.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        // Stage b->B (index: "a\nB\nend", no trailing NL).
        std::fs::write(root.join("f.txt"), "a\nB\nend").unwrap();
        git(root, &["add", "f.txt"]);

        // Workdir == index (no unstaged changes).
        let workdir_before = std::fs::read(root.join("f.txt")).unwrap();
        assert_eq!(workdir_before, b"a\nB\nend");

        // Get the staged hunk.
        let d = g.diff(DiffSide::Staged, "f.txt").unwrap();
        assert_eq!(d.hunks.len(), 1, "one staged hunk: {d:#?}");
        let hunk_start = d.hunks[0].new_start;

        // Discard the staged hunk: must succeed and restore "a\\nb\\nend".
        g.discard_hunk("f.txt", Some(Side::Staged), hunk_start)
            .expect("discard must succeed for EOF-no-NL file");

        let after = std::fs::read(root.join("f.txt")).unwrap();
        assert_eq!(
            after, b"a\nb\nend",
            "workdir must be byte-exact after staged-hunk discard at EOF"
        );
    }

    #[test]
    fn snapshot_real_repo_status_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        std::fs::write(root.join("app.rs"), "fn main() {}\n").unwrap();
        git(root, &["add", "app.rs"]);
        git(root, &["commit", "-q", "-m", "init"]);
        // staged change…
        std::fs::write(root.join("app.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
        g.stage_file("app.rs").unwrap();
        // …and a further unstaged change, plus an untracked file.
        std::fs::write(
            root.join("app.rs"),
            "fn main() {\n    println!(\"hello\");\n    println!(\"world\");\n}\n",
        )
        .unwrap();
        std::fs::write(root.join("scratch.txt"), "untracked\n").unwrap();

        let mut tree = build_tree(&g);
        // Open the first file so its hunk body appears in the snapshot.
        tree.move_down();
        tree.toggle_fold();
        let rows: Vec<String> = tree
            .visible_rows()
            .iter()
            .map(|r| format!("{:?} | {}", r.role, r.text))
            .collect();
        insta::assert_debug_snapshot!(rows);
    }
}

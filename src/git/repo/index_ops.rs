//! Index mutations for `GitRepo` (stage/unstage/discard of files and
//! hunks) — the second `impl GitRepo` block. As a child of the `repo`
//! module it reaches `GitRepo`'s private state with no visibility change;
//! only the cross-sibling private calls carry `pub(super)`.

use std::path::Path;

use crate::git::diff::{DiffHunk, DiffSide, extract};
use crate::git::error::GitError;
use crate::git::status::Side;

use super::GitRepo;
use super::hunk::{
    copy_index_entry,
    default_entry,
    ensure_utf8,
    find_path_in_tree,
    revert_hunk_in_content,
    reverse_apply_hunk_in_content,
};

impl GitRepo {
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
        // The index blob is reverse-applied content, not the workdir file; the
        // workdir still carries the unstaged hunk. Zero the carried stat so git
        // re-hashes and reports the surviving workdir delta (same lie as
        // reset_index_entry_to_head).
        Self::zero_index_entry_stat(&mut new_entry);
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
    pub(super) fn head_tree(&self) -> Result<Option<git2::Tree<'_>>, GitError> {
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

    /// Zero an index entry's stat cache (ctime/mtime/dev/ino/uid/gid/
    /// file_size). Call this whenever an entry's blob is set to content that is
    /// NOT the workdir file — HEAD's blob (reset/unstage) or a reverse-applied
    /// index blob (unstage-hunk). Carrying the workdir stat (copied from a prior
    /// `add_path`) makes `git status` see a matching stat, so git's
    /// racy-timestamp re-hash never fires and a genuinely modified workdir file
    /// is reported clean. `git reset` writes a zeroed stat for exactly this
    /// reason (ctime 0:0, mtime 0:0, ino 0, size 0), forcing a re-hash.
    fn zero_index_entry_stat(e: &mut git2::IndexEntry) {
        e.ctime = git2::IndexTime::new(0, 0);
        e.mtime = git2::IndexTime::new(0, 0);
        e.dev = 0;
        e.ino = 0;
        e.uid = 0;
        e.gid = 0;
        e.file_size = 0;
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
        // The entry now names HEAD's blob, not the workdir file. Zero the stat
        // cache (which `base` may carry from a prior `add_path`) so git must
        // re-hash the workdir file and cannot be fooled into calling it clean.
        Self::zero_index_entry_stat(&mut new_entry);
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

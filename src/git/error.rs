//! The git error type: a `thiserror` enum that wraps libgit2 errors and
//! covers the magit-subset operations (issue 07). Surfaced through the
//! existing app error path (minibuffer messages).

use std::path::PathBuf;

use thiserror::Error;

/// Errors raised by the git wrappers.
#[derive(Debug, Error)]
pub enum GitError {
    /// No git repository was found at (or above) the given path.
    #[error("not a git repository: {path}")]
    NotARepository { path: PathBuf },

    /// The repository has no HEAD commit yet (unborn branch).
    #[error("no HEAD commit (unborn branch)")]
    NoHead,

    /// A raw libgit2 error, passed through with its message.
    #[error("git: {0}")]
    Git(#[from] git2::Error),

    /// The index has no entry for the path (e.g. unstaging a hunk of a
    /// file that is not actually in the index).
    #[error("no index entry for `{0}`")]
    IndexEntryNotFound(String),

    /// No hunk with the given new-side start line exists in the diff
    /// (the target vanished between refresh and staging).
    #[error("hunk not found in `{file}` (new_start {start})")]
    HunkNotFound { file: String, start: u32 },

    /// The file content is not valid UTF-8, so a hunk cannot be
    /// reverse-applied byte-exactly (the diff model carries line text as
    /// strings). Refused rather than silently corrupted.
    #[error("cannot unstage a hunk in `{0}`: content is not valid UTF-8")]
    NotUtf8(String),

    /// Commit identity is unset: `user.name` / `user.email` are not in git
    /// config (tests set them explicitly).
    #[error("no author identity: set user.name and user.email in git config")]
    NoAuthor,

    /// A branch switch was refused because the working tree or index has
    /// uncommitted changes (magit's default: `git checkout` refuses a dirty
    /// tree rather than clobbering it).
    #[error("cannot switch branch: the working tree or index has uncommitted changes")]
    DirtyTree,

    /// The working-directory file needed for a read (e.g. blame line text)
    /// could not be read.
    #[error("cannot read `{path}`: {source}")]
    ReadFile {
        path: String,
        source: std::io::Error,
    },

    /// A working-directory file could not be written or removed (discard
    /// restore/delete, issue 002).
    #[error("cannot update workdir file `{path}`: {source}")]
    WriteFile { path: String, source: std::io::Error },
}

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

    /// Commit identity cannot be resolved the way git resolves it (the
    /// `user.name`/`user.email` config chain → `GIT_AUTHOR_*` /
    /// `GIT_COMMITTER_*` → `EMAIL`/`user@hostname` invention, with
    /// `user.useConfigOnly` honoured). `detail` is self-diagnosing: it
    /// distinguishes `useConfigOnly` forbidding the invented fallback from
    /// a plain "nothing resolves in this environment", quotes git's own
    /// error, and names the config files and env vars consulted.
    #[error("no commit identity: {detail}")]
    NoAuthor { detail: String },

    /// The `git var` subprocess that resolves the commit identity could not
    /// be started (e.g. git is not in this process's `PATH`).
    #[error("could not run `git var` to resolve the commit identity: {0}")]
    IdentityCommand(std::io::Error),

    /// `git var` succeeded but its identity line was unparseable (should be
    /// impossible with a healthy git).
    #[error("unparseable identity from `git var`: {0}")]
    IdentityUnparseable(String),

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

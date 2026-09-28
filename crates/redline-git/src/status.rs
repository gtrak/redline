//! Plain status data + classification, returned by the repo wrapper.
//!
//! These structs carry no git2 types — the repo module converts the
//! libgit2 `Statuses`/`StatusEntry` iteration into these plain forms.

/// Which side of a change a file/hunk section lives on. This drives the
/// dwim staging keys: `s` acts on the unstaged/untracked side, `u` on the
/// staged side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The change is in the index, not yet in HEAD (the "Staged changes"
    /// section).
    Staged,
    /// The change is in the workdir, not yet in the index (the "Unstaged
    /// changes" section).
    Unstaged,
    /// A new, never-tracked file (the "Untracked files" section).
    Untracked,
}

/// The kind of change on one side of a file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusKind {
    /// No change on this side.
    #[default]
    None,
    Added,
    Modified,
    Deleted,
    Renamed,
    Typechange,
}

impl StatusKind {
    /// The single-letter status indicator shown in a file heading.
    pub fn letter(self) -> char {
        match self {
            StatusKind::None => ' ',
            StatusKind::Added => 'A',
            StatusKind::Modified => 'M',
            StatusKind::Deleted => 'D',
            StatusKind::Renamed => 'R',
            StatusKind::Typechange => 'T',
        }
    }
}

/// A changed file, with its staged and unstaged classification. `path` is
/// the index/workdir path (the new side for a rename); `orig` is the
/// pre-rename path when the file was renamed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStatus {
    pub path: String,
    pub orig: Option<String>,
    pub staged: StatusKind,
    pub unstaged: StatusKind,
    pub untracked: bool,
}

impl FileStatus {
    pub fn is_staged(&self) -> bool {
        self.staged != StatusKind::None
    }

    pub fn is_unstaged(&self) -> bool {
        self.unstaged != StatusKind::None
    }
}

/// The current branch (or a detached/unborn state).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchInfo {
    pub name: String,
    pub detached: bool,
    pub unborn: bool,
}

/// An aggregate snapshot of `git status` for the magit status buffer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoStatus {
    /// The current branch, if the repository has one (or a detached/unborn
    /// marker).
    pub branch: Option<BranchInfo>,
    /// Files with any change, sorted by path. git2's iteration order is not
    /// sorted, so the repo wrapper sorts before returning.
    pub files: Vec<FileStatus>,
}

impl RepoStatus {
    pub fn staged_count(&self) -> usize {
        self.files.iter().filter(|f| f.is_staged()).count()
    }

    pub fn unstaged_count(&self) -> usize {
        self.files.iter().filter(|f| f.is_unstaged()).count()
    }

    pub fn untracked_count(&self) -> usize {
        self.files.iter().filter(|f| f.untracked).count()
    }
}

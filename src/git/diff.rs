//! Plain diff data (unified, per-file) + the patch extraction that the
//! repo wrapper runs over a libgit2 `Diff`.
//!
//! The public types (`FileDiff`, `DiffHunk`, `DiffLine`, …) carry no git2
//! types; only `extract` (a `pub(crate)` helper) touches a `git2::Diff`.

use git2::{Diff, DiffFormat, DiffLineType};

/// Which side of a diff: the old/left side and the new/right side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffSide {
    /// `git diff --cached`: HEAD (old) vs index (new).
    Staged,
    /// `git diff`: index (old) vs workdir (new).
    Unstaged,
}

/// The origin of a diff body line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffOrigin {
    Context,
    Addition,
    Deletion,
}

impl DiffOrigin {
    /// The leading character of a unified-diff line.
    pub fn marker(self) -> char {
        match self {
            DiffOrigin::Context => ' ',
            DiffOrigin::Addition => '+',
            DiffOrigin::Deletion => '-',
        }
    }
}

/// One line of a hunk body. `content` is the line text without its
/// trailing newline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub origin: DiffOrigin,
    pub content: String,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
}

/// One hunk: its `@@ … @@` header plus its body lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffHunk {
    /// The `@@ -a,b +c,d @@` header text.
    pub header: String,
    /// 1-based line in the old file where the hunk starts.
    pub old_start: u32,
    pub old_lines: u32,
    /// 1-based line in the new file where the hunk starts (used as the
    /// hunk identity for staging).
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
    /// Whether the old file's last line — when it falls in this hunk —
    /// ends with a trailing newline. libgit2 flags the opposite with a
    /// `ContextEOFNL`/`DeleteEOFNL` marker line; the marker is not a
    /// content line and is not stored in `lines`.
    pub old_ends_nl: bool,
}

/// The unified diff of a single file on one side (staged or unstaged).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    /// True when the change is a binary (no text hunks are extracted).
    pub binary: bool,
    pub insertions: u32,
    pub deletions: u32,
    pub hunks: Vec<DiffHunk>,
}

/// Extract a single file's unified diff (hunks + body lines) from a
/// libgit2 `Diff` that has been constrained to that file (via
/// `DiffOptions::pathspec`). File-header lines are dropped; each hunk is
/// captured with its `@@` header and its context/add/delete body lines.
pub(crate) fn extract(diff: &Diff, path: &str) -> FileDiff {
    let mut file = FileDiff {
        path: path.to_string(),
        binary: false,
        insertions: 0,
        deletions: 0,
        hunks: Vec::new(),
    };

    let _ = diff.print(DiffFormat::Patch, |_delta, hunk, line| {
        match line.origin_value() {
            // `diff --git` / `index` / `---` / `+++` lines: not part of the
            // magit file heading; drop them.
            DiffLineType::FileHeader => {}
            DiffLineType::Binary => file.binary = true,
            DiffLineType::HunkHeader => {
                let h = hunk.expect("a hunk header line must carry its DiffHunk");
                file.hunks.push(DiffHunk {
                    header: strip_newline(line.content()),
                    old_start: h.old_start(),
                    old_lines: h.old_lines(),
                    new_start: h.new_start(),
                    new_lines: h.new_lines(),
                    lines: Vec::new(),
                    old_ends_nl: true,
                });
            }
            DiffLineType::AddEOFNL | DiffLineType::DeleteEOFNL | DiffLineType::ContextEOFNL => {
                // "No newline at end of file" marker: the affected side's
                // last line in this hunk lacks a trailing newline. It is
                // not a content line — record the fact for the old side
                // (the reverse-apply in `revert_hunk_in_content` needs it)
                // and skip the marker itself.
                if matches!(line.origin_value(), DiffLineType::DeleteEOFNL | DiffLineType::ContextEOFNL)
                    && let Some(target) = file.hunks.last_mut()
                {
                    target.old_ends_nl = false;
                }
            }
            _ => {
                let Some(target) = file.hunks.last_mut() else {
                    return true;
                };
                let origin = match line.origin_value() {
                    DiffLineType::Addition => {
                        file.insertions += 1;
                        DiffOrigin::Addition
                    }
                    DiffLineType::Deletion => {
                        file.deletions += 1;
                        DiffOrigin::Deletion
                    }
                    _ => DiffOrigin::Context,
                };
                target.lines.push(DiffLine {
                    origin,
                    content: strip_newline(line.content()),
                    old_lineno: line.old_lineno(),
                    new_lineno: line.new_lineno(),
                });
            }
        }
        true
    });

    file
}

/// `git2` diff `content()` includes the trailing newline; strip a single
/// `\n` so the stored line text is clean for display and snapshotting.
fn strip_newline(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    s.strip_suffix('\n')
        .map(|t| t.to_string())
        .unwrap_or_else(|| s.into_owned())
}

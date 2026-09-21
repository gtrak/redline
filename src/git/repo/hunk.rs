//! Pure hunk byte math and index-entry plumbing for the git repo wrapper:
//! free functions over bytes, diff hunks, and git2 index entries. No
//! `GitRepo` state; `pub(super)` only where the sibling `index_ops`
//! submodule calls them.

use crate::git::diff::{DiffHunk, DiffLine, DiffOrigin};
use crate::git::error::GitError;

/// Rebuild a file's content (bytes) with one hunk's new-side line span
/// replaced by the hunk's old-side lines (context + deletion, in order),
/// byte-exactly. This is the reverse-apply step for unstaging a single
/// hunk. Every old-side line is re-terminated with `\n` except the old
/// file's last line when it lacked a trailing newline (`old_ends_nl`
/// false — libgit2's EOFNL marker, which `extract` does not store as a
/// line).
pub(super) fn revert_hunk_in_content(
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
pub(super) fn reverse_apply_hunk_in_content(
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
pub(super) fn ensure_utf8(content: &[u8], path: &str) -> Result<(), GitError> {
    std::str::from_utf8(content)
        .map(|_| ())
        .map_err(|_| GitError::NotUtf8(path.to_string()))
}

/// Copy an index entry by field (`git2::IndexEntry` is not `Clone` but is
/// an owned plain struct, so it moves freely).
pub(super) fn copy_index_entry(e: git2::IndexEntry) -> git2::IndexEntry {
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
pub(super) fn find_path_in_tree(
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
pub(super) fn default_entry(path: &str, mode: u32) -> git2::IndexEntry {
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

//! The magit section tree: a plain-data, snapshot-testable model of the
//! status buffer.
//!
//! Sections are nested (groups → files → hunks), each carries a fold state
//! and the tree carries a cursor (the section under point). The tree is
//! headless (no git2, no iocraft) and renders to a flat list of display
//! rows that the UI colors. Fold state and the cursor survive refreshes by
//! matching sections on their stable `id`.

use std::collections::HashMap;

use crate::git::diff::{DiffLine, DiffOrigin, FileDiff};
use crate::git::status::{BranchInfo, FileStatus, RepoStatus, Side, StatusKind};

/// The kind of a section.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SectionKind {
    /// The branch header line.
    #[default]
    Header,
    /// A top-level group: "Staged changes" / "Unstaged changes" /
    /// "Untracked files".
    Group,
    /// A file section (in a diff group or the untracked list).
    File,
    /// A hunk section (a child of a file section).
    Hunk,
}

/// The display role of a rendered row; the UI maps this (plus `selected`)
/// to a theme face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowRole {
    Branch,
    Group,
    File,
    HunkHeader,
    DiffContext,
    DiffAdd,
    DiffDelete,
    /// A log entry row (issue 08).
    Commit,
    /// A blame row (issue 08).
    Blame,
    /// A commit-editor line that is a comment (prefilled, `#`-prefixed).
    Comment,
    /// A commit-editor line that is real message text.
    Text,
}

/// One section: a heading plus optional children and body (diff lines).
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// Stable identity (e.g. `"staged:src/a.rs#12"`), used to preserve fold
    /// state across refreshes.
    pub id: String,
    /// The heading line text.
    pub heading: String,
    pub kind: SectionKind,
    /// Which side this section lives on (File/Hunk only).
    pub side: Option<Side>,
    /// The file path (File/Hunk only).
    pub path: Option<String>,
    /// The pre-rename path (File only, when the file was renamed).
    pub orig: Option<String>,
    /// The hunk's new-side start line (Hunk only).
    pub hunk_new_start: Option<u32>,
    /// Whether this section's children/body are hidden.
    pub folded: bool,
    /// Body lines (Hunk sections only: the diff lines).
    pub body: Vec<DiffLine>,
    pub children: Vec<Section>,
}

/// One display row produced by rendering the tree.
#[derive(Clone, Debug, PartialEq)]
pub struct MagitRow {
    pub text: String,
    pub role: RowRole,
    /// True when this row is the section under the cursor.
    pub selected: bool,
}

/// The magit status section tree, with its cursor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatusTree {
    sections: Vec<Section>,
    cursor: Option<String>,
}

/// The cursor's (path, kind, side, orig, hunk_new_start), if any.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CursorTarget {
    pub path: Option<String>,
    pub kind: SectionKind,
    pub side: Option<Side>,
    pub orig: Option<String>,
    pub hunk_new_start: Option<u32>,
}

impl StatusTree {
    /// Build a status section tree from a status snapshot plus per-file
    /// diffs (staged and unstaged). `prev`, when present, supplies the fold
    /// state and cursor to carry over (magit preserves visibility on
    /// refresh).
    pub fn build(
        status: &RepoStatus,
        staged: &HashMap<String, FileDiff>,
        unstaged: &HashMap<String, FileDiff>,
        prev: Option<&StatusTree>,
    ) -> Self {
        let empty: HashMap<String, FileDiff> = HashMap::new();
        let mut sections = vec![
            header_section(status.branch.as_ref()),
            diff_group("staged", "Staged changes", status, Side::Staged, staged),
            diff_group(
                "unstaged",
                "Unstaged changes",
                status,
                Side::Unstaged,
                unstaged,
            ),
            diff_group(
                "untracked",
                "Untracked files",
                status,
                Side::Untracked,
                &empty,
            ),
        ];
        if let Some(prev) = prev {
            for s in sections.iter_mut() {
                set_fold(s, prev);
            }
        }
        let cursor = resolve_cursor(&sections, prev);
        Self { sections, cursor }
    }

    /// The section under the cursor, if any.
    pub fn cursor_section(&self) -> Option<&Section> {
        let cursor = self.cursor.clone()?;
        find_by_id(&self.sections, &cursor)
    }

    /// The cursor's dwim fields (path, kind, side, …), if any.
    pub fn cursor_target(&self) -> Option<CursorTarget> {
        self.cursor_section().map(|s| CursorTarget {
            path: s.path.clone(),
            kind: s.kind,
            side: s.side,
            orig: s.orig.clone(),
            hunk_new_start: s.hunk_new_start,
        })
    }

    /// Move the cursor to the next visible section (wraps nowhere; a no-op
    /// at the last section).
    pub fn move_down(&mut self) {
        let ids = self.visible_ids();
        let cur = self.cursor.clone();
        if let Some(pos) = ids.iter().position(|id| Some(id.as_str()) == cur.as_deref())
            && pos + 1 < ids.len()
        {
            self.cursor = Some(ids[pos + 1].clone());
        } else if !ids.is_empty() {
            self.cursor = Some(ids[0].clone());
        }
    }

    /// Move the cursor to the previous visible section (a no-op at the
    /// first).
    pub fn move_up(&mut self) {
        let ids = self.visible_ids();
        let cur = self.cursor.clone();
        if let Some(pos) = ids.iter().position(|id| Some(id.as_str()) == cur.as_deref())
            && pos > 0
        {
            self.cursor = Some(ids[pos - 1].clone());
        }
    }

    /// Toggle the fold state of the section under the cursor.
    pub fn toggle_fold(&mut self) {
        if let Some(c) = self.cursor.clone()
            && let Some(s) = find_by_id_mut(&mut self.sections, &c)
        {
            s.folded = !s.folded;
        }
    }

    /// The flat list of display rows for the current fold state, in
    /// render order. The cursor's section is marked `selected`.
    pub fn visible_rows(&self) -> Vec<MagitRow> {
        let mut out = Vec::new();
        let cursor = self.cursor.as_deref();
        for s in &self.sections {
            Self::render(s, cursor, 0, &mut out);
        }
        out
    }

    // ── rendering ─────────────────────────────────────────────────────

    fn render(s: &Section, cursor: Option<&str>, depth: usize, out: &mut Vec<MagitRow>) {
        let role = match s.kind {
            SectionKind::Header => RowRole::Branch,
            SectionKind::Group => RowRole::Group,
            SectionKind::File => RowRole::File,
            SectionKind::Hunk => RowRole::HunkHeader,
        };
        let indent = "  ".repeat(depth);
        let selected = cursor == Some(s.id.as_str());
        out.push(MagitRow {
            text: format!("{indent}{}", s.heading),
            role,
            selected,
        });
        if !s.folded {
            for c in &s.children {
                Self::render(c, cursor, depth + 1, out);
            }
            for line in &s.body {
                let role = match line.origin {
                    DiffOrigin::Addition => RowRole::DiffAdd,
                    DiffOrigin::Deletion => RowRole::DiffDelete,
                    DiffOrigin::Context => RowRole::DiffContext,
                };
                out.push(MagitRow {
                    text: format!("{indent}  {}{}", line.origin.marker(), line.content),
                    role,
                    selected: false,
                });
            }
        }
    }

    /// Ids of every section whose heading is rendered (DFS order, skipping
    /// the children of folded sections). The cursor may only rest here.
    fn visible_ids(&self) -> Vec<String> {
        let mut out = Vec::new();
        for s in &self.sections {
            collect_visible_ids(s, &mut out);
        }
        out
    }
}

fn collect_visible_ids(s: &Section, out: &mut Vec<String>) {
    out.push(s.id.clone());
    if !s.folded {
        for c in &s.children {
            collect_visible_ids(c, out);
        }
    }
}

fn set_fold(s: &mut Section, prev: &StatusTree) {
    if let Some(p) = find_by_id(&prev.sections, &s.id) {
        s.folded = p.folded;
    }
    for c in s.children.iter_mut() {
        set_fold(c, prev);
    }
}

fn resolve_cursor(sections: &[Section], prev: Option<&StatusTree>) -> Option<String> {
    // A carried-over cursor is kept only if that section still exists and
    // is visible (all ancestors unfolded).
    if let Some(prev) = prev
        && let Some(c) = &prev.cursor
    {
        let visible = collect_all_visible(sections);
        if visible.iter().any(|id| id == c) {
            return Some(c.clone());
        }
    }
    first_addressable(sections)
}

fn collect_all_visible(sections: &[Section]) -> Vec<String> {
    let mut out = Vec::new();
    for s in sections {
        collect_visible_ids(s, &mut out);
    }
    out
}

/// The first file/hunk section in DFS order (the default cursor position).
fn first_addressable(sections: &[Section]) -> Option<String> {
    for s in sections {
        if matches!(s.kind, SectionKind::File | SectionKind::Hunk) {
            return Some(s.id.clone());
        }
        if let Some(c) = first_addressable(&s.children) {
            return Some(c);
        }
    }
    None
}

fn find_by_id<'a>(sections: &'a [Section], id: &str) -> Option<&'a Section> {
    for s in sections {
        if s.id == id {
            return Some(s);
        }
        if let Some(c) = find_by_id(&s.children, id) {
            return Some(c);
        }
    }
    None
}

fn find_by_id_mut<'a>(sections: &'a mut [Section], id: &str) -> Option<&'a mut Section> {
    for s in sections.iter_mut() {
        if s.id == id {
            return Some(s);
        }
        if let Some(c) = find_by_id_mut(&mut s.children, id) {
            return Some(c);
        }
    }
    None
}

// ── section builders ───────────────────────────────────────────────────

fn header_section(branch: Option<&BranchInfo>) -> Section {
    let heading = match branch {
        Some(b) if b.unborn => "## (no commits)".to_string(),
        Some(b) => format!("## {}", b.name), // detached name already "(short)"
        None => "## (unknown branch)".to_string(),
    };
    Section {
        id: "header".into(),
        heading,
        kind: SectionKind::Header,
        side: None,
        path: None,
        orig: None,
        hunk_new_start: None,
        folded: false,
        body: Vec::new(),
        children: Vec::new(),
    }
}

fn diff_group(
    id: &str,
    title: &str,
    status: &RepoStatus,
    side: Side,
    diffs: &HashMap<String, FileDiff>,
) -> Section {
    let mut children = Vec::new();
    for f in &status.files {
        let on_this_side = match side {
            Side::Staged => f.is_staged(),
            Side::Unstaged => f.is_unstaged(),
            Side::Untracked => f.untracked,
        };
        if !on_this_side {
            continue;
        }
        children.push(file_section(id, side, f, diffs.get(&f.path)));
    }
    Section {
        id: id.into(),
        heading: title.into(),
        kind: SectionKind::Group,
        side: Some(side),
        path: None,
        orig: None,
        hunk_new_start: None,
        folded: false,
        body: Vec::new(),
        children,
    }
}

fn file_section(
    group_id: &str,
    side: Side,
    f: &FileStatus,
    diff: Option<&FileDiff>,
) -> Section {
    let kind = match side {
        Side::Staged => f.staged,
        Side::Unstaged => f.unstaged,
        Side::Untracked => StatusKind::None,
    };
    let letter = if side == Side::Untracked { '?' } else { kind.letter() };
    let mut heading = format!("{letter} {}", f.path);
    if let Some(orig) = &f.orig {
        heading.push_str(&format!(" (was {})", orig));
    }
    if let Some(d) = diff
        && (d.insertions > 0 || d.deletions > 0)
    {
        heading.push_str(&format!(" (+{} -{})", d.insertions, d.deletions));
    }
    let file_id = format!("{group_id}:{}", f.path);
    let mut children = Vec::new();
    if let Some(d) = diff {
        for h in &d.hunks {
            children.push(hunk_section(&file_id, side, &f.path, h));
        }
    }
    Section {
        id: file_id,
        heading,
        kind: SectionKind::File,
        side: Some(side),
        path: Some(f.path.clone()),
        orig: f.orig.clone(),
        hunk_new_start: None,
        folded: true, // default: file sections start collapsed
        body: Vec::new(),
        children,
    }
}

fn hunk_section(file_id: &str, side: Side, path: &str, h: &crate::git::diff::DiffHunk) -> Section {
    let mut body = Vec::new();
    for line in &h.lines {
        body.push(DiffLine::clone(line));
    }
    Section {
        id: format!("{}#{}", file_id, h.new_start),
        heading: h.header.clone(),
        kind: SectionKind::Hunk,
        side: Some(side),
        path: Some(path.to_string()),
        orig: None,
        hunk_new_start: Some(h.new_start),
        folded: false, // default: a hunk's body shows once its file is open
        body,
        children: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::diff::DiffHunk;
    use std::collections::HashMap;

    fn line(origin: DiffOrigin, content: &str) -> DiffLine {
        DiffLine {
            origin,
            content: content.to_string(),
            old_lineno: None,
            new_lineno: None,
        }
    }

    fn hunk(new_start: u32, header: &str, lines: Vec<DiffLine>) -> DiffHunk {
        DiffHunk {
            header: header.to_string(),
            old_start: new_start,
            old_lines: lines
                .iter()
                .filter(|l| l.origin != DiffOrigin::Addition)
                .count() as u32,
            new_start,
            new_lines: lines
                .iter()
                .filter(|l| l.origin != DiffOrigin::Deletion)
                .count() as u32,
            lines,
            old_ends_nl: true,
            new_ends_nl: true,
        }
    }

    fn fdiff(path: &str, ins: u32, del: u32, hunks: Vec<DiffHunk>) -> FileDiff {
        FileDiff {
            path: path.to_string(),
            binary: false,
            insertions: ins,
            deletions: del,
            hunks,
        }
    }

    /// A representative mixed repo state: one staged-only file, one
    /// unstaged-only file, one both, one untracked, and one staged rename.
    fn sample_status() -> RepoStatus {
        RepoStatus {
            branch: Some(BranchInfo {
                name: "main".into(),
                detached: false,
                unborn: false,
            }),
            files: vec![
                FileStatus {
                    path: "src/a.rs".into(),
                    orig: None,
                    staged: StatusKind::Modified,
                    unstaged: StatusKind::None,
                    untracked: false,
                },
                FileStatus {
                    path: "src/b.rs".into(),
                    orig: None,
                    staged: StatusKind::None,
                    unstaged: StatusKind::Modified,
                    untracked: false,
                },
                FileStatus {
                    path: "src/c.rs".into(),
                    orig: None,
                    staged: StatusKind::Modified,
                    unstaged: StatusKind::Modified,
                    untracked: false,
                },
                FileStatus {
                    path: "new.txt".into(),
                    orig: None,
                    staged: StatusKind::None,
                    unstaged: StatusKind::None,
                    untracked: true,
                },
                FileStatus {
                    path: "src/renamed.rs".into(),
                    orig: Some("src/old.rs".into()),
                    staged: StatusKind::Renamed,
                    unstaged: StatusKind::None,
                    untracked: false,
                },
            ],
        }
    }

    fn sample_diffs() -> (HashMap<String, FileDiff>, HashMap<String, FileDiff>) {
        let a = fdiff(
            "src/a.rs",
            1,
            1,
            vec![hunk(
                4,
                "@@ -4,3 +4,3 @@",
                vec![
                    line(DiffOrigin::Context, "fn f() {"),
                    line(DiffOrigin::Deletion, "    old"),
                    line(DiffOrigin::Addition, "    new"),
                    line(DiffOrigin::Context, "}"),
                ],
            )],
        );
        let b = fdiff(
            "src/b.rs",
            1,
            1,
            vec![hunk(
                2,
                "@@ -2,2 +2,2 @@",
                vec![
                    line(DiffOrigin::Context, "ctx"),
                    line(DiffOrigin::Deletion, "gone"),
                    line(DiffOrigin::Addition, "here"),
                ],
            )],
        );
        let c_s = fdiff(
            "src/c.rs",
            1,
            0,
            vec![hunk(
                7,
                "@@ -7,1 +7,2 @@",
                vec![
                    line(DiffOrigin::Context, "base"),
                    line(DiffOrigin::Addition, "staged line"),
                ],
            )],
        );
        let c_u = fdiff(
            "src/c.rs",
            1,
            1,
            vec![hunk(
                8,
                "@@ -8,2 +8,2 @@",
                vec![
                    line(DiffOrigin::Deletion, "index line"),
                    line(DiffOrigin::Addition, "work line"),
                ],
            )],
        );
        let renamed = fdiff(
            "src/renamed.rs",
            1,
            1,
            vec![hunk(
                1,
                "@@ -1 +1 @@",
                vec![
                    line(DiffOrigin::Deletion, "old name"),
                    line(DiffOrigin::Addition, "new name"),
                ],
            )],
        );
        let mut staged = HashMap::new();
        staged.insert("src/a.rs".into(), a);
        staged.insert("src/c.rs".into(), c_s);
        staged.insert("src/renamed.rs".into(), renamed);
        let mut unstaged = HashMap::new();
        unstaged.insert("src/b.rs".into(), b);
        unstaged.insert("src/c.rs".into(), c_u);
        (staged, unstaged)
    }

    fn build() -> StatusTree {
        let status = sample_status();
        let (staged, unstaged) = sample_diffs();
        StatusTree::build(&status, &staged, &unstaged, None)
    }

    /// Rows as `(role | text | selected)` strings, for assertions and
    /// snapshots.
    fn rows_text(tree: &StatusTree) -> Vec<String> {
        tree.visible_rows()
            .iter()
            .map(|r| format!("{:?} | {} | sel={}", r.role, r.text, r.selected))
            .collect()
    }

    #[test]
    fn structure_groups_files_by_side() {
        let tree = build();
        // Top-level: header + three groups.
        assert_eq!(tree.sections.len(), 4);
        let staged = &tree.sections[1];
        assert_eq!(staged.heading, "Staged changes");
        assert_eq!(
            staged.children.iter().map(|c| c.path.clone()).collect::<Vec<_>>(),
            vec![
                Some("src/a.rs".into()),
                Some("src/c.rs".into()),
                Some("src/renamed.rs".into()),
            ]
        );
        let unstaged = &tree.sections[2];
        assert_eq!(
            unstaged.children.iter().map(|c| c.path.clone()).collect::<Vec<_>>(),
            vec![Some("src/b.rs".into()), Some("src/c.rs".into())],
        );
        let untracked = &tree.sections[3];
        assert_eq!(untracked.children.len(), 1);
        assert_eq!(untracked.children[0].path, Some("new.txt".into()));
        // The rename carries its pre-rename path.
        let renamed = staged.children.iter().find(|c| c.path.as_deref() == Some("src/renamed.rs")).unwrap();
        assert_eq!(renamed.orig, Some("src/old.rs".into()));
    }

    #[test]
    fn default_fold_hides_hunks_and_positions_cursor() {
        let tree = build();
        // No hunk headings are visible while files are collapsed.
        assert!(!rows_text(&tree).iter().any(|l| l.starts_with("HunkHeader")));
        // The cursor starts on the first addressable (file) section and its
        // row is marked selected.
        assert_eq!(tree.cursor, Some("staged:src/a.rs".into()));
        let selected: Vec<_> = tree
            .visible_rows()
            .iter()
            .filter(|r| r.selected)
            .map(|r| r.text.clone())
            .collect();
        assert_eq!(selected, vec!["  M src/a.rs (+1 -1)".to_string()]);
    }

    #[test]
    fn toggle_fold_reveals_hunks_and_body() {
        let mut tree = build();
        // The file section starts folded; toggle it open.
        tree.toggle_fold();
        let rows = rows_text(&tree);
        // The a.rs hunk header and its added/deleted lines now render.
        assert!(rows.iter().any(|l| l.starts_with("HunkHeader") && l.contains("@@ -4,3 +4,3 @@")), "\n{rows:?}");
        assert!(rows.iter().any(|l| l.starts_with("DiffAdd") && l.contains("new")), "\n{rows:?}");
        assert!(rows.iter().any(|l| l.starts_with("DiffDelete") && l.contains("old")), "\n{rows:?}");
    }

    #[test]
    fn cursor_movement_respects_fold_and_groups() {
        let mut tree = build();
        // Start on staged:src/a.rs. Move down: next sibling file (c.rs),
        // skipping its (folded) hunks, then the rename, then the group.
        assert_eq!(tree.cursor.as_deref(), Some("staged:src/a.rs"));
        tree.move_down();
        assert_eq!(tree.cursor.as_deref(), Some("staged:src/c.rs"));
        tree.move_down();
        assert_eq!(tree.cursor.as_deref(), Some("staged:src/renamed.rs"));
        tree.move_down();
        assert_eq!(tree.cursor.as_deref(), Some("unstaged")); // group heading
        tree.move_up();
        assert_eq!(tree.cursor.as_deref(), Some("staged:src/renamed.rs"));
    }

    /// Inline hunks (issue 002): unfolding a file reveals its hunk rows
    /// (header + diff body) directly under it; the cursor can move onto the
    /// hunk, which is then addressable (dwim) with its path + new_start.
    #[test]
    fn unfold_file_cursor_lands_on_addressable_hunk() {
        let mut tree = build();
        // Cursor starts on the folded staged:src/a.rs. Unfold it.
        tree.toggle_fold();
        // Move down into the now-visible hunk.
        tree.move_down();
        let target = tree.cursor_target().expect("cursor on a section");
        assert_eq!(target.kind, SectionKind::Hunk, "cursor must be on the hunk");
        assert_eq!(target.path, Some("src/a.rs".into()));
        assert_eq!(target.side, Some(Side::Staged));
        assert_eq!(target.hunk_new_start, Some(4));
        // The hunk's inline diff body lines are present in the rendered rows.
        let rows = tree.visible_rows();
        assert!(
            rows.iter().any(|r| r.role == RowRole::DiffAdd && r.text.contains("new")),
            "inline added line missing: {rows:?}"
        );
        assert!(
            rows.iter().any(|r| r.role == RowRole::DiffDelete && r.text.contains("old")),
            "inline deleted line missing: {rows:?}"
        );
    }

    #[test]
    fn fold_state_survives_refresh() {
        let mut tree = build();
        tree.toggle_fold(); // open staged:src/a.rs
        assert!(!tree.cursor_section().unwrap().folded);
        // Rebuild from the same inputs, carrying over fold + cursor.
        let status = sample_status();
        let (staged, unstaged) = sample_diffs();
        let prev = tree.clone();
        let tree2 = StatusTree::build(&status, &staged, &unstaged, Some(&prev));
        // The previously-open file stays open and the cursor is retained.
        assert_eq!(tree2.cursor, Some("staged:src/a.rs".into()));
        assert!(
            !tree2.cursor_section().unwrap().children.is_empty(),
            "opened file must keep its hunk children"
        );
        assert!(!tree2.cursor_section().unwrap().folded);
    }

    #[test]
    fn snapshot_default_status_tree() {
        let tree = build();
        insta::assert_debug_snapshot!(rows_text(&tree));
    }

    #[test]
    fn snapshot_unfolded_status_tree() {
        let mut tree = build();
        // Open the staged a.rs file so its hunk body is visible.
        tree.toggle_fold();
        insta::assert_debug_snapshot!(rows_text(&tree));
    }
}

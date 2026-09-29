//! The magit section tree: a plain-data, snapshot-testable model of the
//! status buffer.
//!
//! Sections are nested (groups → files → hunks), each carries a fold state
//! and the tree carries a cursor (the section under point). The tree is
//! headless (no git2, no iocraft) and renders to a flat list of display
//! rows that the UI colors. Fold state and the cursor survive refreshes by
//! matching sections on their stable `id`.

use std::collections::HashMap;

use redline_git::diff::{DiffLine, DiffOrigin, FileDiff};
use redline_git::status::{BranchInfo, FileStatus, RepoStatus, Side, StatusKind};

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

impl Section {
    /// Push this section's heading row, then (when unfolded) its children's
    /// rows and its own body lines, into `out` (DFS order).
    fn render(&self, cursor: Option<&str>, depth: usize, out: &mut Vec<MagitRow>) {
        let role = match self.kind {
            SectionKind::Header => RowRole::Branch,
            SectionKind::Group => RowRole::Group,
            SectionKind::File => RowRole::File,
            SectionKind::Hunk => RowRole::HunkHeader,
        };
        let indent = "  ".repeat(depth);
        let selected = cursor == Some(self.id.as_str());
        out.push(MagitRow {
            text: format!("{indent}{}", self.heading),
            role,
            selected,
        });
        if !self.folded {
            for c in &self.children {
                c.render(cursor, depth + 1, out);
            }
            for line in &self.body {
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

    /// Push this section's id and (when unfolded) every visible descendant
    /// id into `out` (DFS order).
    fn collect_visible_ids(&self, out: &mut Vec<String>) {
        out.push(self.id.clone());
        if !self.folded {
            for c in &self.children {
                c.collect_visible_ids(out);
            }
        }
    }

    /// Copy the fold state of the same-id section from `prev` onto this
    /// subtree (magit preserves visibility on refresh).
    fn set_fold(&mut self, prev: &StatusTree) {
        if let Some(p) = find_by_id(&prev.sections, &self.id) {
            self.folded = p.folded;
        }
        for c in self.children.iter_mut() {
            c.set_fold(prev);
        }
    }
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
                s.set_fold(prev);
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

    /// The top-level sections (read-model access: the U-E10 inventory
    /// cross-check scores the tree's OWN headings/bodies through the
    /// shared core — never a copy of the projection's strings).
    #[allow(dead_code)] // test-only accessor (the U-E10 inventory cross-check + magit pins read the tree's own headings/bodies)
    pub fn sections(&self) -> &[Section] {
        &self.sections
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

    /// Move the cursor to the next visible section (magit's `n` /
    /// `magit-section-forward`). Returns `true` when the cursor moved.
    ///
    /// Magit 4.7.1 does not wrap (`lisp/magit-section.el:805-831`): at the
    /// last visible section the cursor stays put and the move reports
    /// `false`, so the store's magit cursor handler can echo
    /// `No next section`. With no cursor (`None`) the cursor stays `None`
    /// and the move reports `false`.
    pub fn move_down(&mut self) -> bool {
        let ids = self.visible_ids();
        let cur = self.cursor.clone();
        let Some(pos) = ids.iter().position(|id| Some(id.as_str()) == cur.as_deref()) else {
            return false; // no cursor: nothing to move from
        };
        if pos + 1 >= ids.len() {
            return false; // last visible section: stay put (no wrap)
        }
        self.cursor = Some(ids[pos + 1].clone());
        true
    }

    /// Move the cursor to the previous visible section (magit's `p` /
    /// `magit-section-backward`). Returns `true` when the cursor moved.
    ///
    /// Magit 4.7.1 does not wrap (`lisp/magit-section.el:805-831`): at the
    /// first visible section the cursor stays put and the move reports
    /// `false`, so the store's magit cursor handler can echo
    /// `No previous section`. With no cursor (`None`) the cursor stays
    /// `None` and the move reports `false`.
    pub fn move_up(&mut self) -> bool {
        let ids = self.visible_ids();
        let cur = self.cursor.clone();
        let Some(pos) = ids.iter().position(|id| Some(id.as_str()) == cur.as_deref()) else {
            return false; // no cursor: nothing to move from
        };
        if pos == 0 {
            return false; // first visible section: stay put (no wrap)
        }
        self.cursor = Some(ids[pos - 1].clone());
        true
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
            s.render(cursor, 0, &mut out);
        }
        out
    }

    /// The U-E10 section-narrowing projection over the current tree:
    /// the flat list of display rows that survive `match_text` (the
    /// narrow query's matcher, supplied by the store — the model stays
    /// matcher-free). A section survives iff ITS heading, one of its own
    /// body lines, or a descendant section matches — so a heading
    /// survives iff a surviving descendant exists (magit's own section
    /// narrowing; PLAN 018 §2.3-2). Two structural guarantees:
    ///
    /// - a surviving hunk renders its FULL body — the diff payload
    ///   (context/add/delete lines) is ONE document; a match is never
    ///   trimmed to its matching lines, and the rows are never re-ranked;
    /// - a surviving child renders even when the canonical tree has its
    ///   section folded: the projection REVEALS the match (a hit hidden
    ///   behind a fold would be a dead end), while the fold state itself
    ///   is untouched — this is a view-time projection, so clearing the
    ///   query restores the previous folds byte-for-byte.
    pub fn narrowed_rows(&self, match_text: &mut impl FnMut(&str) -> bool) -> Vec<MagitRow> {
        let mut out = Vec::new();
        let cursor = self.cursor.as_deref();
        for s in &self.sections {
            render_narrowed(s, cursor, 0, match_text, &mut out);
        }
        out
    }

    /// The ids of the sections whose heading survives the U-E10
    /// projection, in NARROWED RENDER ORDER (revealed DFS: a surviving
    /// child's heading follows its parent's even under a canonical fold —
    /// the projection's on-screen row order). This is both the set of
    /// rows a cursor may rest on while the narrow query is active and the
    /// order `move_down_within` / `move_up_within` step through.
    pub fn narrowed_surviving_ids(&self, match_text: &mut impl FnMut(&str) -> bool) -> Vec<String> {
        let mut out = Vec::new();
        for s in &self.sections {
            collect_surviving_ids(s, match_text, &mut out);
        }
        out
    }

    /// U-E10 cursor invariant: while the narrow query is active the
    /// cursor rests on a SURVIVING section (the selection is clamped
    /// into the narrowed set, never lost — the cursor's identity is the
    /// section it is on). When the current cursor's section does not
    /// survive (or none is set), the cursor moves to the first surviving
    /// addressable (File/Hunk) section in render order, else the first
    /// surviving section. When nothing survives the cursor is left
    /// alone: the narrowed set is empty (no rows to select), and the
    /// query's clear re-derives the full list where the section id is
    /// valid again.
    pub fn clamp_cursor_to_narrowed(&mut self, match_text: &mut impl FnMut(&str) -> bool) {
        let surviving = self.narrowed_surviving_ids(match_text);
        if surviving.is_empty() {
            return;
        }
        if self
            .cursor
            .as_ref()
            .is_some_and(|c| surviving.iter().any(|s| s == c))
        {
            return;
        }
        let target = first_surviving_section(
            &self.sections,
            match_text,
            &|s| matches!(s.kind, SectionKind::File | SectionKind::Hunk),
        )
        .or_else(|| first_surviving_section(&self.sections, match_text, &|_| true));
        if let Some(id) = target {
            self.cursor = Some(id);
        }
    }

    /// Move the cursor to the next section in the U-E10 narrowed render
    /// order (the surviving set, revealed DFS — the on-screen row order
    /// while the query is active; a filtered-out section is never a
    /// landing spot, and a revealed hunk IS). Boundary semantics identical
    /// to `move_down`: no wrap, and the move reports `false` at the edge.
    /// The cursor must already rest on a surviving section (the store's
    /// recompute keeps that invariant); a cursor outside the set reports
    /// `false` without moving.
    pub fn move_down_within(&mut self, match_text: &mut impl FnMut(&str) -> bool) -> bool {
        let ids = self.narrowed_surviving_ids(match_text);
        let cur = self.cursor.clone();
        let Some(pos) = ids.iter().position(|id| Some(id.as_str()) == cur.as_deref()) else {
            return false;
        };
        if pos + 1 >= ids.len() {
            return false;
        }
        self.cursor = Some(ids[pos + 1].clone());
        true
    }

    /// The `move_up` twin over the U-E10 narrowed render order, same
    /// no-wrap boundary semantics as `move_up`.
    pub fn move_up_within(&mut self, match_text: &mut impl FnMut(&str) -> bool) -> bool {
        let ids = self.narrowed_surviving_ids(match_text);
        let cur = self.cursor.clone();
        let Some(pos) = ids.iter().position(|id| Some(id.as_str()) == cur.as_deref()) else {
            return false;
        };
        if pos == 0 {
            return false;
        }
        self.cursor = Some(ids[pos - 1].clone());
        true
    }

    // ── rendering ─────────────────────────────────────────────────────

    /// Ids of every section whose heading is rendered (DFS order, skipping
    /// the children of folded sections). The cursor may only rest here.
    fn visible_ids(&self) -> Vec<String> {
        let mut out = Vec::new();
        for s in &self.sections {
            s.collect_visible_ids(&mut out);
        }
        out
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
        s.collect_visible_ids(&mut out);
    }
    out
}

/// U-E10: whether `s` survives the narrow projection — its heading,
/// one of its own body lines, or a descendant section matches.
fn survives(s: &Section, match_text: &mut impl FnMut(&str) -> bool) -> bool {
    match_text(&s.heading)
        || s.body.iter().any(|l| match_text(&l.content))
        || s.children.iter().any(|c| survives(c, match_text))
}

/// U-E10: push `s`'s heading row, then its surviving children (rendered
/// even under a canonical fold — the projection reveals the match), and
/// — for a surviving hunk — its FULL body (the diff payload is one
/// document).
fn render_narrowed(
    s: &Section,
    cursor: Option<&str>,
    depth: usize,
    match_text: &mut impl FnMut(&str) -> bool,
    out: &mut Vec<MagitRow>,
) {
    if !survives(s, match_text) {
        return;
    }
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
    for c in &s.children {
        render_narrowed(c, cursor, depth + 1, match_text, out);
    }
    if !s.body.is_empty() {
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

/// U-E10: push `s`'s id (when it survives) and its surviving children's
/// ids, in DFS render order.
fn collect_surviving_ids(
    s: &Section,
    match_text: &mut impl FnMut(&str) -> bool,
    out: &mut Vec<String>,
) {
    if !survives(s, match_text) {
        return;
    }
    out.push(s.id.clone());
    for c in &s.children {
        collect_surviving_ids(c, match_text, out);
    }
}

/// The first section in DFS render order that survives `match_text` AND
/// passes `kind_ok` (U-E10's cursor clamp target). `kind_ok` is a `dyn`
/// pointer so the recursion's type stays flat (a generic `impl Fn` would
/// nest one reference per level and hit the recursion limit).
fn first_surviving_section(
    sections: &[Section],
    match_text: &mut impl FnMut(&str) -> bool,
    kind_ok: &dyn Fn(&Section) -> bool,
) -> Option<String> {
    for s in sections {
        if survives(s, match_text) {
            if kind_ok(s) {
                return Some(s.id.clone());
            }
            if let Some(c) = first_surviving_section(&s.children, match_text, &kind_ok) {
                return Some(c);
            }
        }
    }
    None
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

fn hunk_section(file_id: &str, side: Side, path: &str, h: &redline_git::diff::DiffHunk) -> Section {
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
    use redline_git::diff::{DiffHunk, DiffSide};
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

    /// Magit 4.7.1 boundary behavior (`lisp/magit-section.el:805-831`): `n`
    /// and `p` do not wrap. At the last/first visible section the cursor
    /// stays put and the move reports `false`, which the store turns into
    /// the `No next section` / `No previous section` echo-area message.
    #[test]
    fn move_down_at_last_section_stays_put_and_reports() {
        let mut tree = build();
        // The last visible section (all groups/files visible, files folded
        // by default): the untracked group's only file.
        tree.cursor = Some("untracked:new.txt".into());
        assert!(!tree.move_down(), "at the last section: must report no move");
        assert_eq!(
            tree.cursor.as_deref(),
            Some("untracked:new.txt"),
            "no wrap: the cursor must not jump to the first section"
        );
    }

    #[test]
    fn move_up_at_first_section_stays_put_and_reports() {
        let mut tree = build();
        // The first visible section: the branch header.
        tree.cursor = Some("header".into());
        assert!(!tree.move_up(), "at the first section: must report no move");
        assert_eq!(
            tree.cursor.as_deref(),
            Some("header"),
            "no wrap: the cursor must not jump to the last section"
        );
    }

    #[test]
    fn move_down_up_mid_list_move_and_report() {
        let mut tree = build();
        assert!(
            tree.move_down(),
            "mid-list move_down must move and report true"
        );
        assert_eq!(tree.cursor.as_deref(), Some("staged:src/c.rs"));
        assert!(
            tree.move_up(),
            "mid-list move_up must move and report true"
        );
        assert_eq!(tree.cursor.as_deref(), Some("staged:src/a.rs"));
    }

    #[test]
    fn move_without_cursor_stays_none_and_reports_no_move() {
        // An empty repo has no file/hunk sections, so build leaves the
        // cursor at None (the header and empty groups render but are not
        // the default landing spot). With no cursor, both directions leave
        // it None and report `false` (the store echoes the boundary
        // message, since nothing can move).
        let status = RepoStatus {
            branch: None,
            files: Vec::new(),
        };
        let empty: HashMap<String, FileDiff> = HashMap::new();
        let mut tree = StatusTree::build(&status, &empty, &empty, None);
        assert_eq!(tree.cursor, None);
        assert!(!tree.move_down());
        assert!(!tree.move_up());
        assert_eq!(tree.cursor, None);
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

    // U-E10: the section-structural projection. `"staged line"` matches
    // exactly ONE text in the sample fixture — the staged-side hunk body
    // line of src/c.rs — so the survivors are exactly: the "Staged
    // changes" group (via its surviving child), the c.rs file (via its
    // surviving descendant), and that hunk. The c.rs file section starts
    // FOLDED, and the projection reveals the match: the hunk header +
    // the FULL body render while the canonical fold state stays folded.
    #[test]
    fn narrowed_projection_keeps_structure_and_reveals_folded_matches() {
        let mut tree = build();
        // The c.rs file section is folded by default (pre-fix state).
        let c = find_by_id(&tree.sections, "staged:src/c.rs").unwrap();
        assert!(c.folded, "the fixture's c.rs file section starts folded");

        let mut pred = |text: &str| text == "staged line";
        let rows = tree.narrowed_rows(&mut pred);
        let texts: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "Staged changes",
                "  M src/c.rs (+1 -0)",
                "    @@ -7,1 +7,2 @@",
                "       base",
                "      +staged line",
            ],
            "a heading survives iff a surviving descendant exists; the surviving hunk renders its full body: {texts:?}"
        );
        // The surviving set is exactly the match's structural path.
        let ids = tree.narrowed_surviving_ids(&mut pred);
        assert_eq!(
            ids,
            vec!["staged", "staged:src/c.rs", "staged:src/c.rs#7"],
            "only the match's ancestor chain survives: {ids:?}"
        );
        // The projection reveals the match UNDER THE FOLD: the hunk rows
        // are present although the canonical tree keeps the file folded.
        assert!(
            find_by_id(&tree.sections, "staged:src/c.rs").unwrap().folded,
            "the projection must not mutate the canonical fold state"
        );
        // And the un-narrowed render is untouched by the projection.
        let full = tree.visible_rows();
        assert!(
            !full.iter().any(|r| r.text == "    @@ -7,1 +7,2 @@"),
            "the canonical (folded) render shows no hunk rows: {full:?}"
        );
        // The cursor clamps into the surviving set: from a.rs (filtered
        // out) it lands on the first surviving addressable section.
        tree.clamp_cursor_to_narrowed(&mut pred);
        assert_eq!(
            tree.cursor.as_deref(),
            Some("staged:src/c.rs"),
            "clamped onto the first surviving File section"
        );
        // n/p move WITHIN the narrowed render order (never onto a
        // filtered row; the revealed hunk IS a landing spot).
        assert!(tree.move_down_within(&mut pred));
        assert_eq!(tree.cursor.as_deref(), Some("staged:src/c.rs#7"));
        assert!(!tree.move_down_within(&mut pred), "no wrap past the surviving set's last section");
        assert!(tree.move_up_within(&mut pred));
        assert_eq!(tree.cursor.as_deref(), Some("staged:src/c.rs"));
        assert!(tree.move_up_within(&mut pred), "back up to the surviving group heading");
        assert_eq!(tree.cursor.as_deref(), Some("staged"));
        assert!(!tree.move_up_within(&mut pred), "the filtered a.rs/renamed sections are not allowed predecessors");
        // Nothing survives: the cursor is left alone (the empty set has
        // no rows to select; the clear re-derives the full list).
        let mut none_pred = |text: &str| text == "zzz-no-match";
        tree.clamp_cursor_to_narrowed(&mut none_pred);
        assert_eq!(tree.cursor.as_deref(), Some("staged"));
        assert!(tree.narrowed_rows(&mut none_pred).is_empty());
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

    // ── Real-repo snapshot (moved from redline-git's repo/tests.rs, plan 014
    // stage 2): StatusTree lives here (bin's model), so its oracle test
    // lives here too — the crate cannot dev-dep on the bin (bin-only
    // package), and redline-model does not exist yet (stage 3).

    /// The standard fixture identity ("Test"/"test@example.com"); the hermetic
    /// env block lives once in `redline_testutil`.
    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        redline_testutil::git_cli(dir, args, "Test", "test@example.com")
    }

    fn init_repo(dir: &std::path::Path) -> redline_git::GitRepo {
        redline_testutil::git_repo_init(dir, "Test", "test@example.com", true);
        redline_git::GitRepo::discover(dir).expect("discover the repo")
    }

    /// Build a `StatusTree` the same way the store does (status + per-file
    /// diffs).
    fn build_tree(g: &redline_git::GitRepo) -> StatusTree {
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

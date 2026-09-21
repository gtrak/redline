use super::navigation::{tooling_candidate_name, xref_location_candidate};
use super::*;

impl AppStore {
    fn palette_candidates(&self) -> Vec<PickerCandidate> {
        // picker-density: the palette stays display-only on purpose — a
        // row's whole content is a single command identifier (no kind or
        // path context to right-align), so the name-first split would
        // just duplicate the name into two cells.
        self.registry
            .list()
            .map(|c| PickerCandidate {
                name: c.name.to_string(),
                display: c.name.to_string(),
                label: String::new(),
                detail: String::new(),
                docs: c.docs.to_string(),
                category: c.category.to_string(),
            })
            .collect()
    }

    fn find_file_candidates(&self) -> Vec<PickerCandidate> {
        self.project
            .as_ref()
            .and_then(|p| self.files.get(&p.root))
            .map(|list| list.files.iter().map(|f| file_candidate(f)).collect())
            .unwrap_or_default()
    }

    fn recent_file_candidates(&self) -> Vec<PickerCandidate> {
        let Some(project) = self.project.as_ref() else {
            return Vec::new();
        };
        let root = project.root.to_string_lossy().into_owned();
        let mut out = Vec::new();
        for rel in self.project_store.recents.list(&root) {
            // Deleted files drop out of the list.
            if project.root.join(rel).is_file() {
                let label = rel.rsplit('/').next().unwrap_or(rel);
                out.push(PickerCandidate {
                    name: rel.clone(),
                    display: rel.clone(),
                    // picker-density: name-first — the file name left,
                    // the path right-aligned (truncates tail-keeping, so
                    // the name survives long paths). A root-level file
                    // has no directory component, so the name IS the
                    // path: detail stays empty and the row draws the
                    // name once (left-anchored), not twice.
                    label: label.to_string(),
                    detail: if label == rel { String::new() } else { rel.clone() },
                    docs: String::new(),
                    category: "recent".to_string(),
                });
            }
        }
        out
    }

    fn buffer_candidates(&self) -> Vec<PickerCandidate> {
        self.buffers.list().into_iter().map(|(key, _)| {
            // picker-density: name-first — the (marked) buffer name left,
            // the absolute path right-aligned (the project root context
            // display alone doesn't carry for external buffers).
            let detail = self
                .buffers
                .get(key)
                .and_then(|b| b.path.clone())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            PickerCandidate {
                name: key.to_string(),
                display: self.buffer_display(key),
                label: self.buffer_display(key),
                detail,
                docs: String::new(),
                category: "buffer".to_string(),
            }
        }).collect()
    }

    /// Known projects, excluding the current one (projectile default).
    fn project_candidates(&self) -> Vec<PickerCandidate> {
        let current = self.project.as_ref().map(|p| p.root.clone());
        self.project_store
            .registry
            .list()
            .iter()
            .filter(|p| Some(p.root.clone()) != current)
            .map(|p| PickerCandidate {
                name: p.root.to_string_lossy().into_owned(),
                display: p.name.clone(),
                // picker-density: name-first — the project name left,
                // the root path right-aligned (what the preview used to
                // be the only place for).
                label: p.name.clone(),
                detail: p.root.to_string_lossy().into_owned(),
                docs: p.root.to_string_lossy().into_owned(),
                category: "project".to_string(),
            })
            .collect()
    }

    /// Candidates for the annotations picker (015-01): every annotation
    /// record in the notes document, in document order (file, then line —
    /// the notes document's own order, NOT recency). `NotesEntry::Raw`
    /// blocks are verbatim non-annotation content (malformed records,
    /// stray lines — kept verbatim, never interpreted) and are NEVER
    /// candidates: the list is annotations only. Row shape (name-first):
    /// name/label = the annotation text, detail = `path:line` (1-based
    /// line, the location pickers' display convention).
    fn annotations_candidates(&mut self) -> Vec<PickerCandidate> {
        // The notes document (disk or open buffer) is the source of truth
        // for the records, so load/re-parse it before building (plan 005
        // issue 02).
        self.ensure_notes_doc();
        let mut rows: Vec<(String, usize, PickerCandidate)> = self
            .notes_doc
            .entries
            .iter()
            .filter_map(|e| e.as_record())
            .map(|a| {
                let detail = format!("{}:{}", a.path, a.line + 1);
                (
                    a.path.clone(),
                    a.line,
                    PickerCandidate {
                        name: a.text.clone(),
                        display: format!("{}  {}", a.text, detail),
                        // picker-density: name-first — the annotation text
                        // left, the location right-aligned.
                        label: a.text.clone(),
                        detail,
                        docs: String::new(),
                        category: "annotation".to_string(),
                    },
                )
            })
            .collect();
        rows.sort_by(|(p1, l1, _), (p2, l2, _)| p1.cmp(p2).then(l1.cmp(l2)));
        rows.into_iter().map(|(_, _, c)| c).collect()
    }

    /// `C-c n a` (015-01): open the annotations picker — a temporary,
    /// filterable list of every annotation record in the notes document.
    /// RET jumps to the selected annotation's `(path, line)`; editing
    /// needs no command of its own — jump, then `A` on that line
    /// pre-fills the existing note. With zero annotations the picker
    /// opens with an empty list (the count row reads 0/0) rather than
    /// erroring — an empty notes document is a normal state, not a
    /// broken list.
    pub fn open_annotations_picker(&mut self) {
        if self.project.is_none() {
            self.minibuffer_message("annotations: no project");
            return;
        }
        let candidates = self.annotations_candidates();
        self.open_picker(PickerKind::Annotations, "Annotations: ", candidates);
    }

    /// `d` in the annotations picker (015-01): delete the selected
    /// annotation's record and recompute the list on the same query —
    /// the buffer view's `d` reached from the picker (the Stash list's
    /// `x` drop is the sibling precedent).
    pub(super) fn annotations_picker_delete(&mut self) {
        let Some((detail, query)) = self
            .picker
            .as_ref()
            .and_then(|p| {
                (p.kind == PickerKind::Annotations)
                    .then(|| {
                        p.filtered
                            .get(p.selected)
                            .map(|(cand, _)| (cand.detail.clone(), p.query.clone()))
                    })
                    .flatten()
            })
        else {
            return;
        };
        // detail is "path:line" (1-based line number, as displayed).
        if let Some((path, line_str)) = detail.rsplit_once(':')
            && let Ok(line) = line_str.parse::<usize>()
        {
            self.delete_annotation_at_path_line(path, line - 1);
            let candidates = self.annotations_candidates();
            self.set_picker_query(PickerKind::Annotations, query, candidates);
        }
    }

    fn candidates_for(&mut self, kind: PickerKind) -> Vec<PickerCandidate> {
        match kind {
            PickerKind::Palette => self.palette_candidates(),
            PickerKind::FindFile => self.find_file_candidates(),
            PickerKind::RecentFiles => self.recent_file_candidates(),
            PickerKind::Buffers | PickerKind::KillBuffer => self.buffer_candidates(),
            PickerKind::Projects => self.project_candidates(),
            PickerKind::Xref => self.xref_candidates(),
            PickerKind::Impls => self.impls_candidates(),
            PickerKind::Imenu => self.imenu_candidates(),
            PickerKind::Symbols => self.symbol_candidates(),
            PickerKind::Branch => self.branch_candidates(),
            PickerKind::Stash => self.stash_candidates(),
            PickerKind::Annotations => self.annotations_candidates(),
        }
    }

    /// Candidates for the branch picker (`y`, issue 08): local branches with
    /// the current (HEAD) one marked.
    pub(super) fn branch_candidates(&self) -> Vec<PickerCandidate> {
        self.with_git(|g| g.branches())
            .unwrap_or_default()
            .into_iter()
            .map(|b| PickerCandidate {
                name: b.name.clone(),
                display: format!("{}{}", if b.current { "*" } else { " " }, b.name),
                // picker-density: name-first — the branch name left,
                // the HEAD marker right-aligned (display keeps the
                // `*`-prefixed form for matching).
                label: b.name.clone(),
                detail: if b.current { "*".to_string() } else { String::new() },
                docs: if b.current {
                    "current branch".to_string()
                } else {
                    String::new()
                },
                category: "branch".to_string(),
            })
            .collect()
    }

    /// Candidates for the stash list (`z`, issue 08).
    pub(super) fn stash_candidates(&mut self) -> Vec<PickerCandidate> {
        self.with_git_mut(|g| g.stash_list())
            .unwrap_or_default()
            .into_iter()
            .map(|s| PickerCandidate {
                name: s.index.to_string(),
                display: format!("stash@{{{}}} {}", s.index, s.subject),
                // picker-density: name-first — the stash ref left, the
                // subject right-aligned.
                label: format!("stash@{{{}}}", s.index),
                detail: s.subject,
                docs: String::new(),
                category: "stash".to_string(),
            })
            .collect()
    }

    /// Candidates for the Xref picker (definition locations for the
    /// current lookup name — the project index, or the crate index the
    /// picker was opened with, 006-03).
    /// Candidates for the Xref picker (definition locations for the
    /// current lookup name — the project index, or the crate index the
    /// picker was opened with, 006-03), with the source marker per row
    /// (`here`/`other`, jump-ambiguity), and the pending tooling row
    /// (jump-ambiguity) prepended so it stays preselected under query
    /// re-derivation (the initial open and the filtered re-derivation
    /// cannot drift apart).
    pub(super) fn xref_candidates(&mut self) -> Vec<PickerCandidate> {
        let name = self.xref_lookup_name.clone();
        let root = self.xref_crate_root.clone();
        let defs: Vec<crate::nav::index::Location> = match root.as_ref() {
            Some(root) => self
                .crate_index_arc(root)
                .map(|arc| arc.lock().unwrap().definitions_of(&name))
                .unwrap_or_default(),
            None => self.index.definitions_of(&name),
        };
        let mut out: Vec<PickerCandidate> = Vec::new();
        let cur = self.xref_current_file_rel(root.as_ref());
        for d in &defs {
            out.push(xref_location_candidate(d, cur.as_deref()));
        }
        // (jump-ambiguity) the pending tooling row joins the list
        // preselected (row 0) — unless an index candidate already covers
        // the same (file, line), in which case that row IS the top guess
        // (RET lands on the same location through the index path).
        if let Some(tooling) = self.xref_tooling_candidate()
            && !out.iter().any(|c| c.name == tooling.name)
        {
            out.insert(0, tooling);
        }
        out
    }

    /// (jump-ambiguity) The pending tooling row, or `None` when the
    /// picker carries index candidates only. The `name` ("file:line",
    /// 1-based, file in the picker's keying — crate-relative when the
    /// landing is a file of its own crate, project-relative inside the
    /// project, else absolute) doubles as the RET's landing
    /// discriminator (`run_selected` compares the selected row's name
    /// against it); the `tooling` tag keeps the weak resolve visible
    /// ("top of file" reads as a weak answer, not a mystery).
    pub(super) fn xref_tooling_candidate(&self) -> Option<PickerCandidate> {
        let (source, symbol) = self.xref_tooling_pending.as_ref()?;
        let name = tooling_candidate_name(source, &self.xref_crate_root, &self.project);
        Some(PickerCandidate {
            name: name.clone(),
            display: format!("{name}  [tooling] {symbol}"),
            label: symbol.clone(),
            detail: format!("[tooling] {name}"),
            docs: String::new(),
            category: "xref".to_string(),
        })
    }

    /// Candidates for the find-implementations picker (010-04, plan 010
    /// Shape A rung 4): the `impl <Trait> for <Type>` blocks for the
    /// trait keys stored when the picker was opened (`impls_keys` — the
    /// M-. path token and/or the bare identifier, like the name-keyed
    /// M-. index lookup), from the project index or the crate index the
    /// picker was opened with (006-03, like the Xref picker). Deterministic
    /// (key order, then file, impl line); a block hit by both keys is
    /// listed once (the first key's spelling).
    pub(super) fn impls_candidates(&mut self) -> Vec<PickerCandidate> {
        let keys = self.impls_keys.clone();
        let root = self.xref_crate_root.clone();
        let mut out: Vec<PickerCandidate> = Vec::new();
        let mut seen: Vec<(String, usize)> = Vec::new();
        for key in &keys {
            let locs: Vec<crate::nav::index::TraitImplLocation> = match root.as_ref() {
                Some(root) => self
                    .crate_index_arc(root)
                    .map(|arc| arc.lock().unwrap().trait_impl_locations(key))
                    .unwrap_or_default(),
                None => self.index.trait_impl_locations(key),
            };
            for l in locs {
                if seen.iter().any(|(f, li)| f == &l.file && *li == l.impl_line) {
                    continue;
                }
                seen.push((l.file.clone(), l.impl_line));
                out.push(PickerCandidate {
                    name: format!("{}:{}", l.file, l.impl_line + 1),
                    display: format!(
                        "{}:{}  [impl {} for {}]",
                        l.file,
                        l.impl_line + 1,
                        key,
                        l.self_type
                    ),
                    // picker-density: name-first — the impl block left,
                    // its location right-aligned.
                    label: format!("impl {} for {}", key, l.self_type),
                    detail: format!("{}:{}", l.file, l.impl_line + 1),
                    docs: String::new(),
                    category: "impls".to_string(),
                });
            }
        }
        out
    }

    /// Candidates for the Imenu picker (current file's outline — the
    /// project index for project files, the owning crate's index for
    /// external buffers, 006-03). The SAME display derivation as the
    /// initial open (`imenu_candidate` — incl. the Rust impl-parent
    /// grouping), so a query re-derivation cannot drift from the list
    /// the user opened.
    fn imenu_candidates(&mut self) -> Vec<PickerCandidate> {
        let outline = self.current_buffer_outline();
        let tables = self.current_buffer_rust_tables();
        outline
            .iter()
            .map(|s| Self::imenu_candidate(s, &outline, tables.as_ref()))
            .collect()
    }

    /// Watchlist (imenu impl-parent grouping): one imenu candidate for
    /// `s` — name `name:line` (1-based, the picker's match target, never
    /// grouped); display indented by enclosing extent (the existing PART
    /// A rule) PLUS, for Rust files, an impl METHOD (the file's Rung 1
    /// tables record an impl method of exactly this name on exactly this
    /// line — a line holds at most one impl method, so the match is
    /// conclusive) renders under the impl's type: one level deeper than
    /// that type's own indent when the type is in the outline, else one
    /// level deeper than its enclosing-extent depth (an impl of a type
    /// defined elsewhere). Other languages (no tables) stay byte-for-byte
    /// the PART A indent.
    pub(super) fn imenu_candidate(
        s: &crate::nav::index::Symbol,
        outline: &[crate::nav::index::Symbol],
        tables: Option<&redline_syntax::queries::RustTables>,
    ) -> PickerCandidate {
        let depth = Self::imenu_depth(s, outline, tables);
        let indent = "  ".repeat(depth);
        PickerCandidate {
            name: format!("{}:{}", s.name, s.line + 1),
            display: format!("{indent}{}  [{}]", s.name, s.kind.tag()),
            // picker-density: name-first — the (indented) name left, the
            // kind tag right-aligned. The indent travels with the LABEL so
            // the impl-parent grouping/indent renders unchanged; `display`
            // (the match target) stays byte-identical.
            label: format!("{indent}{}", s.name),
            detail: format!("[{}]", s.kind.tag()),
            docs: String::new(),
            category: "imenu".to_string(),
        }
    }

    /// The imenu display depth of `s`: the count of symbols whose extent
    /// strictly contains `s` (the PART A enclosing-extent rule), plus
    /// one level for a Rust impl method grouped under the impl's type
    /// (see `imenu_candidate`).
    fn imenu_depth(
        s: &crate::nav::index::Symbol,
        outline: &[crate::nav::index::Symbol],
        tables: Option<&redline_syntax::queries::RustTables>,
    ) -> usize {
        let enclosing = |t: &crate::nav::index::Symbol| {
            outline
                .iter()
                .filter(|e| {
                    !(**e == *t)
                        && e.line <= t.line
                        && t.end_line <= e.end_line
                        && (e.line < t.line || e.end_line > t.end_line)
                })
                .count()
        };
        let base = enclosing(s);
        let Some(tables) = tables else {
            return base;
        };
        let self_type = tables
            .impls
            .iter()
            .find_map(|(self_type, methods)| {
                methods
                    .iter()
                    .any(|m| m.method == s.name && m.line == s.line)
                    .then_some(self_type.as_str())
            });
        let Some(self_type) = self_type else {
            return base;
        };
        // Under the impl's type: the type's own enclosing-extent depth
        // when it is in this file's outline; a type defined elsewhere has
        // no row to nest under — one level below the method's own depth.
        let parent_depth = match outline.iter().find(|e| e.name == self_type) {
            Some(parent) => enclosing(parent),
            None => base,
        };
        parent_depth + 1
    }

    /// Candidates for the project-wide symbol picker (all index symbols).
    fn symbol_candidates(&self) -> Vec<PickerCandidate> {
        self.index
            .all_locations()
            .into_iter()
            .map(|loc| PickerCandidate {
                name: format!("{}:{}", loc.file, loc.symbol.line + 1),
                display: format!("{}  [{}]  {}", loc.symbol.name, loc.symbol.kind.tag(), loc.file),
                label: loc.symbol.name.clone(),
                detail: format!("[{}] {}", loc.symbol.kind.tag(), loc.file),
                docs: String::new(),
                category: "symbol".to_string(),
            })
            .collect()
    }

    /// Open the `M-x` palette over the command registry.
    pub fn open_palette(&mut self) {
        self.open_picker(PickerKind::Palette, "M-x ", self.palette_candidates());
    }

    /// `C-x C-f` / `C-c p f`: the project file picker.
    pub fn open_find_file(&mut self) {
        if self.project.is_none() {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        }
        if self.ensure_files().is_none() {
            return;
        }
        self.open_picker(PickerKind::FindFile, "Find file: ", self.find_file_candidates());
    }

    /// `C-c p e`: recently visited files of the current project.
    pub fn open_recent_files(&mut self) {
        if self.project.is_none() {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        }
        let candidates = self.recent_file_candidates();
        if candidates.is_empty() {
            self.minibuffer_message("no recently visited files");
            return;
        }
        self.open_picker(PickerKind::RecentFiles, "Recent file: ", candidates);
    }

    /// `C-x b`: switch-buffer picker.
    pub fn open_switch_buffer(&mut self) {
        self.open_picker(PickerKind::Buffers, "Switch buffer: ", self.buffer_candidates());
    }

    /// `C-x k`: kill-buffer picker.
    pub fn open_kill_buffer(&mut self) {
        self.open_picker(PickerKind::KillBuffer, "Kill buffer: ", self.buffer_candidates());
    }

    /// `C-c p p`: known-project switcher (current project excluded).
    pub fn open_switch_project(&mut self) {
        let candidates = self.project_candidates();
        if candidates.is_empty() {
            self.minibuffer_message("no other known projects");
            return;
        }
        self.open_picker(PickerKind::Projects, "Switch project: ", candidates);
    }

    pub(super) fn open_picker(&mut self, kind: PickerKind, prompt: &str, candidates: Vec<PickerCandidate>) {
        // (jump-ambiguity) opening a non-Xref picker closes any pending
        // tooling landing (its row belongs to the Xref picker only; the
        // Xref seams set/clear it explicitly around their own state).
        if !matches!(kind, PickerKind::Xref) {
            self.xref_tooling_pending = None;
        }
        let files = matches!(kind, PickerKind::FindFile | PickerKind::RecentFiles);
        let mut picker = Picker {
            kind,
            prompt: prompt.to_string(),
            query: String::new(),
            selected: 0,
            filtered: Vec::new(),
            preview: String::new(),
        };
        let m = if files { &mut self.file_matcher } else { &mut self.matcher };
        picker.recompute(&candidates, m);
        self.picker = Some(picker);
        self.refresh_preview();
    }

    pub(super) fn picker_query_char(&mut self, c: char) {
        let (kind, query, candidates) = match self.picker.as_ref() {
            Some(p) => {
                let mut q = p.query.clone();
                q.push(c);
                (p.kind, q, self.candidates_for(p.kind))
            }
            None => return,
        };
        self.set_picker_query(kind, query, candidates);
    }

    /// Backspace / C-h: remove the last query character.
    pub(super) fn picker_query_backspace(&mut self) {
        let (kind, query, candidates) = match self.picker.as_ref() {
            Some(p) => (p.kind, p.query.clone(), self.candidates_for(p.kind)),
            None => return,
        };
        let mut query = query;
        query.pop();
        self.set_picker_query(kind, query, candidates);
    }

    fn set_picker_query(&mut self, kind: PickerKind, query: String, candidates: Vec<PickerCandidate>) {
        let files = matches!(kind, PickerKind::FindFile | PickerKind::RecentFiles);
        if let Some(p) = self.picker.as_mut() {
            p.query = query;
            let m = if files { &mut self.file_matcher } else { &mut self.matcher };
            p.recompute(&candidates, m);
            p.selected = p.selected.min(p.filtered.len().saturating_sub(1));
        }
        self.refresh_preview();
    }

    pub fn picker_open(&self) -> bool {
        self.picker.is_some()
    }

    pub fn picker_kind(&self) -> Option<PickerKind> {
        self.picker.as_ref().map(|p| p.kind)
    }

    pub fn picker_prompt(&self) -> &str {
        self.picker.as_ref().map(|p| p.prompt.as_str()).unwrap_or("")
    }

    pub fn picker_query(&self) -> &str {
        self.picker.as_ref().map(|p| p.query.as_str()).unwrap_or("")
    }

    /// The filtered candidates for the current query, best-first.
    pub fn picker_filtered(&self) -> &[(PickerCandidate, u32)] {
        self.picker
            .as_ref()
            .map(|p| p.filtered.as_slice())
            .unwrap_or(&[])
    }

    /// (filtered count, total candidate count).
    pub fn picker_count(&mut self) -> (usize, usize) {
        let total = match self.picker_kind() {
            None | Some(PickerKind::Palette) => self.registry.list().count(),
            Some(PickerKind::FindFile) => self
                .project
                .as_ref()
                .and_then(|p| self.files.get(&p.root))
                .map(|l| l.len())
                .unwrap_or(0),
            Some(PickerKind::RecentFiles) => self
                .project
                .as_ref()
                .map(|p| {
                    let root = p.root.to_string_lossy().into_owned();
                    self.project_store.recents.list(&root).len()
                })
                .unwrap_or(0),
            Some(PickerKind::Buffers | PickerKind::KillBuffer) => self.buffers.len(),
            Some(PickerKind::Projects) => self.project_store.registry.len(),
            // 006-03: the Xref picker's total follows its root — the
            // project index, or the crate index it was opened with — and
            // the pending tooling row (jump-ambiguity). Re-deriving
            // through `xref_candidates` keeps the count in step with the
            // rows (incl. the dedup case: tooling row == an index row).
            Some(PickerKind::Xref) => self.xref_candidates().len(),
            Some(PickerKind::Imenu) => self.current_buffer_outline().len(),
            Some(PickerKind::Impls) => self.impls_candidates().len(),
            Some(PickerKind::Annotations) => self.annotations_candidates().len(),
            Some(PickerKind::Symbols) => self.index.total(),
            Some(PickerKind::Branch) => self
                .with_git(|g| g.branches())
                .map(|b| b.len())
                .unwrap_or(0),
            Some(PickerKind::Stash) => self
                .picker
                .as_ref()
                .map(|p| p.filtered.len())
                .unwrap_or(0),
        };
        let shown = self.picker.as_ref().map(|p| p.filtered.len()).unwrap_or(0);
        (shown, total)
    }

    pub fn picker_selected(&self) -> usize {
        self.picker.as_ref().map(|p| p.selected).unwrap_or(0)
    }

    /// The preview-pane text for the selected candidate.
    pub fn picker_preview(&self) -> &str {
        self.picker
            .as_ref()
            .map(|p| p.preview.as_str())
            .unwrap_or("")
    }

    /// (Re)compute the preview for the picker's selected candidate:
    /// command docs for the palette, a first page of the file for
    /// file pickers, the buffer head for buffer pickers, the root path
    /// for the project switcher.
    fn refresh_preview(&mut self) {
        let choice = self
            .picker
            .as_ref()
            .and_then(|p| {
                p.filtered
                    .get(p.selected)
                    .map(|(c, _)| (p.kind, c.name.clone(), c.docs.clone(), c.detail.clone()))
            });
        let (kind, name, docs, detail) = match choice {
            Some(choice) => choice,
            None => {
                if let Some(p) = self.picker.as_mut() {
                    p.preview = String::new();
                }
                return;
            }
        };
        let preview = match kind {
            PickerKind::Palette | PickerKind::Projects => docs,
            PickerKind::FindFile | PickerKind::RecentFiles => self.file_preview(&name),
            PickerKind::Buffers | PickerKind::KillBuffer => self.buffer_preview(&name),
            // Xref and Symbols: preview the file at the definition location.
            // The name is "file:line" (1-based). Show a window around the
            // definition line, not the file's first page.
            PickerKind::Xref | PickerKind::Symbols | PickerKind::Impls => {
                if let Some((file, line_str)) = name.rsplit_once(':')
                    && let Ok(line) = line_str.parse::<usize>()
                {
                    self.file_preview_at_line(file, line - 1)
                } else {
                    String::new()
                }
            }
            // Imenu: preview the current file (the name is "symbol:line").
            PickerKind::Imenu => self.file_preview_current(),
            // Annotations: preview the file at the annotation's line
            // (like Xref — the detail is "path:line", 1-based).
            PickerKind::Annotations => {
                if let Some((file, line_str)) = detail.rsplit_once(':')
                    && let Ok(line) = line_str.parse::<usize>()
                {
                    self.file_preview_at_line(file, line - 1)
                } else {
                    String::new()
                }
            }
            // Branch / Stash: no preview pane (the candidate display is
            // already self-describing).
            PickerKind::Branch | PickerKind::Stash => String::new(),
        };
        if let Some(p) = self.picker.as_mut() {
            p.preview = preview;
        }
    }

    /// First page of the file as plain text (issue 03 adds highlighting).
    /// Already-open buffers render from memory; others are read with a
    /// byte cap.
    fn file_preview(&self, rel: &str) -> String {
        let Some(project) = self.project.as_ref() else {
            return String::new();
        };
        let abs = project.root.join(rel);
        let key = abs.to_string_lossy().into_owned();
        let text = if let Some(buf) = self.buffers.get(&key) {
            buf.text()
        } else {
            // Bound the read itself (take(N)): a huge file must never be
            // pulled fully into memory on a selection move.
            let mut file = match std::fs::File::open(&abs) {
                Ok(f) => f,
                Err(e) => return format!("(cannot read: {e})"),
            };
            let mut bytes = Vec::new();
            match Read::take(&mut file, PREVIEW_MAX_BYTES as u64).read_to_end(&mut bytes) {
                Ok(_) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(e) => return format!("(cannot read: {e})"),
            }
        };
        text.lines()
            .take(PREVIEW_LINES)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The absolute path of a picker file candidate: crate-relative when
    /// the Xref picker lists an external crate (006-03 — the
    /// `source_root` recorded when the picker opened), project-relative
    /// otherwise. An ABSOLUTE path (the jump-ambiguity tooling row for an
    /// external landing — `tooling_candidate_name` keys registry files by
    /// absolute path when they are outside both the project root and the
    /// picker's crate root) passes through as itself.
    fn picker_file_abs(&self, rel: &str) -> Option<PathBuf> {
        if let Some(root) = self.xref_crate_root.as_ref() {
            return Some(root.join(rel));
        }
        if Path::new(rel).is_absolute() {
            return Some(rel.into());
        }
        self.project.as_ref().map(|p| p.root.join(rel))
    }

    /// Preview of a file centred on a 0-based line: a window of
    /// `PREVIEW_LINES` total lines around `line` (the definition context).
    fn file_preview_at_line(&self, rel: &str, line: usize) -> String {
        let Some(abs) = self.picker_file_abs(rel) else {
            return String::new();
        };
        let key = abs.to_string_lossy().into_owned();
        let text = if let Some(buf) = self.buffers.get(&key) {
            buf.text()
        } else {
            let mut file = match std::fs::File::open(&abs) {
                Ok(f) => f,
                Err(e) => return format!("(cannot read: {e})"),
            };
            let mut bytes = Vec::new();
            match Read::take(&mut file, PREVIEW_MAX_BYTES as u64).read_to_end(&mut bytes) {
                Ok(_) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(e) => return format!("(cannot read: {e})"),
            }
        };
        let lines: Vec<&str> = text.lines().collect();
        if lines.is_empty() {
            return String::new();
        }
        // Window: up to 8 lines before, the rest after (32 total).
        const BEFORE: usize = 8;
        let start = line.saturating_sub(BEFORE);
        let end = (start + PREVIEW_LINES).min(lines.len());
        if start >= end {
            return String::new();
        }
        lines[start..end].join("\n")
    }

    fn buffer_preview(&self, key: &str) -> String {
        self.buffers
            .get(key)
            .map(|b| {
                b.text()
                    .lines()
                    .take(PREVIEW_LINES)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    /// Preview of the current buffer's text (for the Imenu picker).
    fn file_preview_current(&self) -> String {
        self.buffers
            .current_buffer()
            .map(|b| {
                b.text()
                    .lines()
                    .take(PREVIEW_LINES)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    /// Run the candidate the picker has selected (RET in a picker).
    pub fn run_selected(&mut self) {
        let choice = self
            .picker
            .as_ref()
            .and_then(|p| {
                p.filtered
                    .get(p.selected)
                    .map(|(c, _)| (p.kind, c.name.clone(), c.detail.clone()))
            });
        // (jump-ambiguity) the pending tooling landing travels with the
        // selection (cleared below, with the picker — every picker close
        // drops it).
        let tooling = self.xref_tooling_pending.clone();
        self.picker = None;
        self.xref_tooling_pending = None;
        let Some((kind, name, detail)) = choice else {
            self.minibuffer_message("no candidate selected");
            return;
        };
        match kind {
            PickerKind::Palette => {
                let _ = self.dispatch(&name, None);
            }
            PickerKind::FindFile | PickerKind::RecentFiles => self.open_path(&name),
            PickerKind::Buffers => {
                self.buffers.set_current(&name);
                // 006-03b item 1: a switched-to external buffer keeps its
                // owning crate MRU.
                self.bump_current_crate_recency();
                self.normalize_top_view();
            }
            PickerKind::KillBuffer => self.kill_buffer(&name),
            PickerKind::Projects => self.switch_project_root(&name),
            PickerKind::Xref | PickerKind::Symbols | PickerKind::Impls => {
                // name is "file:line" (1-based line number).
                // (jump-ambiguity) the SELECTED row is the pending
                // tooling row (name comparison — in the dedup case the
                // tooling name equals an index row's, so the index path
                // below lands the same location) → land through the
                // stored source (project-relative open, or the external
                // read-only open), with the origin captured when `M-.`
                // was pressed.
                let tooling_name = tooling.as_ref().map(|(src, _)| {
                    tooling_candidate_name(src, &self.xref_crate_root, &self.project)
                });
                if kind == PickerKind::Xref && tooling_name.as_deref() == Some(name.as_str()) {
                    if let Some((source, symbol)) = tooling {
                        self.land_tooling_resolved_source(&source, &symbol);
                    }
                } else if let Some((file, line_str)) = name.rsplit_once(':')
                    && let Ok(line) = line_str.parse::<usize>()
                {
                    // (jump-ambiguity) a tooling picker's INDEX rows carry
                    // the M-. time origin too (the whole path is async —
                    // the point may have moved between the keypress and
                    // this RET); a plain index picker captures now (the
                    // same instant the picker opened, synchronously).
                    let origin = if tooling.is_some() {
                        self.xref_tooling_origin.clone()
                    } else {
                        self.current_jump_entry()
                    };
                    if let Some(root) = self.xref_crate_root.clone() {
                        // 006-03: the candidate is CRATE-relative — open
                        // READ-ONLY via the external path (the landing
                        // stays inside the same source_root, so the crate
                        // cache stays valid and 008-01 semantics hold).
                        if self.open_external_path(&root.join(file)).is_some() {
                            self.set_point_line(line - 1);
                            self.recenter_landing();
                            self.ensure_highlight();
                            self.record_jump(origin, "M-.");
                            self.minibuffer_message(&format!("jumped to {file}:{line}"));
                        } else {
                            self.minibuffer_message(&format!("cannot open {file}"));
                        }
                    } else {
                        self.open_path(file);
                        self.set_point_line(line - 1);
                        self.recenter_landing();
                        self.ensure_highlight();
                        self.record_jump(origin, "M-.");
                        self.minibuffer_message(&format!("jumped to {file}:{line}"));
                    }
                }
            }
            PickerKind::Imenu => {
                // name is "symbol:line" (1-based line number); the file is
                // the current buffer, so just scroll to the line.
                if let Some(line_str) = name.rsplit_once(':').map(|(_, l)| l)
                    && let Ok(line) = line_str.parse::<usize>()
                {
                    let origin = self.current_jump_entry();
                    self.set_point_line(line - 1);
                    self.recenter_landing();
                    self.ensure_highlight();
                    self.record_jump(origin, "M-i");
                }
            }
            // Issue 08: branch picker RET checks out; stash list RET pops.
            PickerKind::Branch => self.checkout_branch(&name),
            PickerKind::Stash => {
                if let Ok(index) = name.parse::<usize>() {
                    self.stash_pop(index);
                } else {
                    self.minibuffer_message("no stash selected");
                }
            }
            // 015-01: RET jumps to the selected annotation's (path, line)
            // — the same project-relative landing sequence as the Xref
            // picker (open, point to the line, recenter, record the jump).
            PickerKind::Annotations => {
                // detail is "path:line" (1-based line number, as shown).
                if let Some((file, line_str)) = detail.rsplit_once(':')
                    && let Ok(line) = line_str.parse::<usize>()
                {
                    let origin = self.current_jump_entry();
                    self.open_path(file);
                    self.set_point_line(line - 1);
                    self.recenter_landing();
                    self.ensure_highlight();
                    self.record_jump(origin, "C-c n a");
                    self.minibuffer_message(&format!("jumped to {file}:{line}"));
                }
            }
        }
    }

    pub fn picker_select_next(&mut self) {
        if let Some(p) = self.picker.as_mut() && !p.filtered.is_empty() {
            p.selected = (p.selected + 1) % p.filtered.len();
        }
        // helm follow-mode: a selection move re-renders the preview of
        // the newly selected candidate.
        self.refresh_preview();
    }

    pub fn picker_select_prev(&mut self) {
        if let Some(p) = self.picker.as_mut() && !p.filtered.is_empty() {
            // Wrap-decrement: prev at index 0 lands on the last candidate.
            p.selected = (p.selected + p.filtered.len() - 1) % p.filtered.len();
        }
        // helm follow-mode: a selection move re-renders the preview of
        // the newly selected candidate.
        self.refresh_preview();
    }
}

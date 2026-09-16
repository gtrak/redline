//! Central app store: the single source of truth the ui layer renders
//! and updates. Holds the view stack, keymap state, pending key
//! sequence, minibuffer message, status line state, quit flag, the
//! picker overlay state, the project layer (current project, known
//! projects, per-project recents, cached file lists), and the
//! open-buffer set.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use nucleo_matcher::{
    Matcher, pattern::{CaseMatching, Normalization, Pattern},
};

use crate::app::command::{CommandRegistry, RegistryError};
use crate::app::config::Config;
use crate::app::keymap::{Key, KeyCode, KeyMap, KeySeq, KeymapEngine, Lookup, parse_sequence};
use crate::model::buffer::{BufferTable, SCRATCH_NAME};
use crate::model::files::FileList;
use crate::model::project::{detect_root, Project, ProjectStore};
use crate::theme::Theme;

/// A view on the stack. The top of the stack is what the main view
/// renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewId {
    /// The main view: shows the current buffer's text.
    Buffer,
    /// The `C-x C-b` list-buffers view.
    BufferList,
}

impl ViewId {
    pub fn name(self) -> &'static str {
        match self {
            ViewId::Buffer => "buffer",
            ViewId::BufferList => "buffer-list",
        }
    }

    fn keymap(self) -> KeyMap {
        match self {
            ViewId::Buffer => {
                let mut km = KeyMap::new();
                km.bind(&[Key::char('q')], "quit").unwrap();
                km.bind(&[Key::alt_char('o')], "open-scratch").unwrap();
                km
                    .bind(&[Key::ctrl_char('x'), Key::char('o')], "open-scratch")
                    .unwrap();
                km
                    .bind(&[Key::ctrl_char('x'), Key::ctrl_char('i')], "insert-demo-text")
                    .unwrap();
                km
            }
            ViewId::BufferList => {
                let mut km = KeyMap::new();
                km.bind(&[Key::char('q')], "close-view").unwrap();
                km.bind(&[Key::enter()], "open-buffer-list-selected").unwrap();
                km.bind(&[Key::down()], "buffer-list-next").unwrap();
                km.bind(&[Key::ctrl_char('n')], "buffer-list-next").unwrap();
                km.bind(&[Key::up()], "buffer-list-prev").unwrap();
                km.bind(&[Key::ctrl_char('p')], "buffer-list-prev").unwrap();
                km
            }
        }
    }
}

/// What a picker's RET does with the selected candidate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickerKind {
    /// `M-x` command palette: dispatch the candidate's command name.
    #[default]
    Palette,
    /// Find file (`C-x C-f` / `C-c p f`): open the selected project file.
    FindFile,
    /// `C-c p e`: open a recently visited file.
    RecentFiles,
    /// `C-x b`: switch to the selected buffer.
    Buffers,
    /// `C-x k`: kill the selected buffer.
    KillBuffer,
    /// `C-c p p`: switch to the selected project, then land in that
    /// project's file picker (projectile's default switch action).
    Projects,
}

/// One candidate in the picker. `name` is the value RET acts on
/// (command name, project-relative path, buffer key, or project root);
/// `display` is what the list renders. Nucleo matches `display`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerCandidate {
    pub name: String,
    pub display: String,
    pub docs: String,
    pub category: String,
}

/// Picker overlay state. The filtered list is recomputed in the store
/// (not the ui) so navigation keys can move through it headlessly.
#[derive(Debug)]
struct Picker {
    kind: PickerKind,
    prompt: String,
    query: String,
    selected: usize,
    /// (candidate, nucleo score) for the current query, best-first.
    filtered: Vec<(PickerCandidate, u32)>,
    /// Preview-pane text for the selected candidate (multi-line).
    preview: String,
}

/// One row of the buffer-list view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferRow {
    pub name: String,
    pub current: bool,
    pub lines: u64,
}

/// Preview size: a "first page" of the file, byte-capped so a huge
/// file never stalls a selection move.
const PREVIEW_LINES: usize = 32;
const PREVIEW_MAX_BYTES: usize = 64 * 1024;

pub struct AppStore {
    pub theme: Theme,
    pub registry: CommandRegistry,
    pub engine: KeymapEngine,
    /// View stack; the top of the stack is what the main view renders.
    pub view_stack: Vec<ViewId>,
    /// Open buffers + the current buffer.
    pub buffers: BufferTable,
    /// Selection cursor of the buffer-list view.
    buffer_list_selected: usize,
    /// The current project (canonical root), when redline runs inside
    /// one (or after a `C-c p p` switch); otherwise `None`.
    pub project: Option<Project>,
    /// Persisted project state (known-project registry + recents)
    /// under the cache dir.
    pub project_store: ProjectStore,
    /// Cached file lists, per project root.
    files: HashMap<PathBuf, FileList>,
    /// Strict prefix of a key sequence pressed so far (shown in the
    /// status line), e.g. `[C-c p]`.
    pub pending: KeySeq,
    pub quit: bool,
    /// Async-activity indicator slots (status line, e.g. `*indexing`).
    pub activity: Vec<String>,
    /// Minibuffer message (the echo area).
    pub message: String,
    picker: Option<Picker>,
    /// Long-lived nucleo matchers: ~135KB of scratch each, built once,
    /// never per keystroke (nucleo skill gotcha #1). The file matcher
    /// gets path bonuses for file-picking.
    matcher: Matcher,
    file_matcher: Matcher,
}

impl AppStore {
    /// A store rooted at the current directory, with persistence under
    /// the production cache dir (`cache_dir()/redline`).
    pub fn new() -> Self {
        let start = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::at(&start, ProjectStore::default_base())
    }

    /// A store rooted at `start` (project detection) with persistence
    /// under `base` (tests inject tempdirs so the real cache path is
    /// never touched).
    pub fn at(start: &Path, base: PathBuf) -> Self {
        let registry = CommandRegistry::seed();

        let mut global = KeyMap::new();
        global.bind(&[Key::ctrl_char('g')], "cancel").unwrap();
        global.bind(&[Key::alt_char('x')], "open-palette").unwrap();
        // C-x C-c (quit): bare C-x stays a prefix (pending), so both the
        // C-x C-c global binding and the view-map C-x o / C-x C-i
        // bindings remain reachable.
        global
            .bind(&[Key::ctrl_char('x'), Key::ctrl_char('c')], "quit")
            .unwrap();
        global.bind(&[Key::alt_char('s')], "cycle-view-next").unwrap();
        global.bind(&[Key::alt_char('p')], "cycle-view-prev").unwrap();
        // Browse layer (issue 02).
        global
            .bind(&[Key::ctrl_char('x'), Key::ctrl_char('f')], "find-file")
            .unwrap();
        global
            .bind(&[Key::ctrl_char('x'), Key::char('b')], "switch-buffer")
            .unwrap();
        global
            .bind(&[Key::ctrl_char('x'), Key::ctrl_char('b')], "list-buffers")
            .unwrap();
        global
            .bind(&[Key::ctrl_char('x'), Key::char('k')], "kill-buffer")
            .unwrap();
        // Projectile prefix (C-c p …): verified projectile-ux keys.
        global
            .bind(
                &[Key::ctrl_char('c'), Key::char('p'), Key::char('f')],
                "find-file",
            )
            .unwrap();
        global
            .bind(
                &[Key::ctrl_char('c'), Key::char('p'), Key::char('p')],
                "switch-project",
            )
            .unwrap();
        global
            .bind(
                &[Key::ctrl_char('c'), Key::char('p'), Key::char('e')],
                "recent-files",
            )
            .unwrap();
        global
            .bind(
                &[Key::ctrl_char('c'), Key::char('p'), Key::char('i')],
                "re-walk",
            )
            .unwrap();

        let view = ViewId::Buffer.keymap();
        let engine = KeymapEngine::new(global, view);

        let mut project_store = ProjectStore::open(base);
        let project = detect_root(start).map(Project::new);
        if let Some(p) = &project {
            // Track the starting project in the known-project registry.
            project_store.registry.upsert(&p.root);
            let _ = project_store.save_registry();
        }

        Self {
            theme: Theme::default(),
            registry,
            engine,
            view_stack: vec![ViewId::Buffer],
            buffers: BufferTable::new(),
            buffer_list_selected: 0,
            project,
            project_store,
            files: HashMap::new(),
            pending: Vec::new(),
            quit: false,
            activity: Vec::new(),
            message: String::new(),
            picker: None,
            matcher: Matcher::new(nucleo_matcher::Config::DEFAULT),
            file_matcher: Matcher::new(nucleo_matcher::Config::DEFAULT.match_paths()),
        }
    }

    /// Apply a config: select the theme and (re)bind key overrides.
    /// Returns an error describing the first unbindable override.
    pub fn apply_config(&mut self, config: &Config) -> Result<(), String> {
        self.theme = Theme::from(config.theme);
        for (command, sequence) in &config.key_bindings {
            let seq = parse_sequence(sequence)
                .map_err(|e| format!("`{command}`: invalid key sequence `{sequence}`: {e}"))?;
            self.engine.global.bind(&seq, command).map_err(|e| {
                format!("`{command}`: cannot bind `{sequence}`: {e}")
            })?;
        }
        Ok(())
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn top_view(&self) -> ViewId {
        *self.view_stack.last().expect("view stack is never empty")
    }

    pub fn view_name(&self) -> &'static str {
        self.top_view().name()
    }

    /// Status-line view name: the current buffer's display name (or
    /// `*list-buffers*` in the buffer-list view).
    pub fn view_name_display(&self) -> String {
        match self.top_view() {
            ViewId::Buffer => self
                .buffers
                .current()
                .map(|key| self.buffer_display(key))
                .unwrap_or_else(|| SCRATCH_NAME.to_string()),
            ViewId::BufferList => "*list-buffers*".to_string(),
        }
    }

    /// Status-line project name (replaces the issue-01 placeholder).
    pub fn project_display(&self) -> &str {
        self.project
            .as_ref()
            .map(|p| p.name.as_str())
            .unwrap_or("no project")
    }

    /// Display name of a buffer: project-relative when the buffer
    /// belongs to the current project, absolute otherwise.
    pub fn buffer_display(&self, key: &str) -> String {
        let Some(buf) = self.buffers.get(key) else {
            return key.to_string();
        };
        match buf.path {
            Some(ref abs) => {
                if let Some(project) = &self.project && let Ok(rel) = abs.strip_prefix(&project.root)
                {
                    return rel.to_string_lossy().into_owned();
                }
                abs.to_string_lossy().into_owned()
            }
            None => SCRATCH_NAME.to_string(),
        }
    }

    pub fn pending_display(&self) -> String {
        self.pending
            .iter()
            .map(|k| k.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn activity_display(&self) -> String {
        self.activity.join(" ")
    }

    /// The current buffer's text (empty when no buffer is current).
    pub fn buffer_text(&self) -> String {
        self.buffers
            .current_buffer()
            .map(|b| b.text.clone())
            .unwrap_or_default()
    }

    /// Rows for the buffer-list view (MRU order).
    pub fn buffer_rows(&self) -> Vec<BufferRow> {
        self.buffers
            .list()
            .into_iter()
            .map(|(key, buf)| BufferRow {
                name: self.buffer_display(key),
                current: self.buffers.current() == Some(key),
                lines: buf.text.lines().count() as u64,
            })
            .collect()
    }

    /// Selection cursor of the buffer-list view.
    pub fn buffer_list_selected(&self) -> usize {
        self.buffer_list_selected
    }

    pub fn push_view(&mut self, view: ViewId) {
        let view_km = view.keymap();
        self.engine = KeymapEngine::new(self.engine.global.clone(), view_km);
        self.view_stack.push(view);
    }

    /// Pop the top view (if a non-root view is on top) and restore the
    /// new top view's keymap.
    pub fn close_view(&mut self) {
        if self.view_stack.len() > 1 {
            self.view_stack.pop();
            let view_km = self.top_view().keymap();
            self.engine = KeymapEngine::new(self.engine.global.clone(), view_km);
            self.buffer_list_selected = 0;
        }
    }

    /// Rotate the view stack by `delta` positions (positive: the top
    /// view moves toward the back; negative: the back moves to the top).
    pub fn cycle_view(&mut self, delta: i64) {
        let n = self.view_stack.len() as i64;
        if n < 2 {
            return;
        }
        let steps = (delta.rem_euclid(n)) as usize;
        if steps > 0 {
            let start = (n - steps as i64) as usize;
            let moved: Vec<ViewId> = self.view_stack.drain(start..).collect();
            self.view_stack.extend(moved);
        }
        // Re-assert the top view's keymap after rotating.
        let view_km = self.top_view().keymap();
        self.engine = KeymapEngine::new(self.engine.global.clone(), view_km);
    }

    /// Insert text into the current buffer (the `insert-demo-text`
    /// demo command); false when no buffer is current.
    pub fn insert_text(&mut self, text: &str) -> bool {
        let Some(key) = self.buffers.current() else {
            return false;
        };
        let key = key.to_string();
        let pos = self
            .buffers
            .get(&key)
            .map(|b| b.text.chars().count())
            .unwrap_or(0);
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.text.insert_str(pos, text);
            true
        } else {
            false
        }
    }

    /// Switch to (creating if needed) the `*scratch*` buffer.
    pub fn open_scratch(&mut self) {
        let key = SCRATCH_NAME.to_string();
        if self.buffers.get(&key).is_none() {
            self.buffers.insert(None, String::new());
        }
        self.buffers.set_current(&key);
        self.minibuffer_message("switched to *scratch*");
    }

    /// Open the project-relative file in a buffer and make it current;
    /// records it in the project's recents (persisted).
    pub fn open_path(&mut self, rel: &str) {
        let Some(project) = self.project.clone() else {
            self.minibuffer_message("no project");
            return;
        };
        let abs = project.root.join(rel);
        let key = abs.to_string_lossy().into_owned();
        if self.buffers.get(&key).is_none() {
            match std::fs::read_to_string(&abs) {
                Ok(text) => {
                    self.buffers.insert(Some(abs.clone()), text);
                }
                Err(e) => {
                    self.minibuffer_message(&format!("cannot open {rel}: {e}"));
                    return;
                }
            }
        }
        self.buffers.set_current(&key);
        self.record_recent(rel);
    }

    /// Record `rel` in the current project's recents and persist.
    fn record_recent(&mut self, rel: &str) {
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let root = project.root.to_string_lossy().into_owned();
        self.project_store.recents.add(&root, rel);
        let _ = self.project_store.save_recents();
    }

    /// Ensure the current project's file list is cached; builds it on
    /// first use (one walk per project — the hot key is the nucleo
    /// filter over the cached list, not the walk).
    fn ensure_files(&mut self) -> Option<()> {
        let project = self.project.clone()?;
        if !self.files.contains_key(&project.root) {
            match FileList::build(&project.root) {
                Ok(list) => {
                    self.files.insert(project.root.clone(), list);
                }
                Err(e) => {
                    self.minibuffer_message(&format!(
                        "walk failed for {}: {e}",
                        project.name
                    ));
                    return None;
                }
            }
        }
        Some(())
    }

    /// `C-c p i`: invalidate the cached file list and re-walk the
    /// project root.
    pub fn re_walk(&mut self) {
        let Some(project) = self.project.clone() else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        match FileList::build(&project.root) {
            Ok(list) => {
                let n = list.len();
                self.files.insert(project.root.clone(), list);
                self.minibuffer_message(&format!(
                    "re-walked {}: {} files",
                    project.name, n
                ));
            }
            Err(e) => self.minibuffer_message(&format!(
                "walk failed for {}: {e}",
                project.name
            )),
        }
    }

    // ── picker: candidate lists ─────────────────────────────────────────

    fn palette_candidates(&self) -> Vec<PickerCandidate> {
        self.registry
            .list()
            .map(|c| PickerCandidate {
                name: c.name.to_string(),
                display: c.name.to_string(),
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
                out.push(PickerCandidate {
                    name: rel.clone(),
                    display: rel.clone(),
                    docs: String::new(),
                    category: "recent".to_string(),
                });
            }
        }
        out
    }

    fn buffer_candidates(&self) -> Vec<PickerCandidate> {
        self.buffers.list().into_iter().map(|(key, _)| PickerCandidate {
            name: key.to_string(),
            display: self.buffer_display(key),
            docs: String::new(),
            category: "buffer".to_string(),
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
                docs: p.root.to_string_lossy().into_owned(),
                category: "project".to_string(),
            })
            .collect()
    }

    fn candidates_for(&self, kind: PickerKind) -> Vec<PickerCandidate> {
        match kind {
            PickerKind::Palette => self.palette_candidates(),
            PickerKind::FindFile => self.find_file_candidates(),
            PickerKind::RecentFiles => self.recent_file_candidates(),
            PickerKind::Buffers | PickerKind::KillBuffer => self.buffer_candidates(),
            PickerKind::Projects => self.project_candidates(),
        }
    }

    // ── picker: open per command ────────────────────────────────────────

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

    fn open_picker(&mut self, kind: PickerKind, prompt: &str, candidates: Vec<PickerCandidate>) {
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

    // ── picker: query editing ───────────────────────────────────────────

    fn picker_query_char(&mut self, c: char) {
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
    fn picker_query_backspace(&mut self) {
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

    // ── picker: selection + preview ─────────────────────────────────────

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
    pub fn picker_count(&self) -> (usize, usize) {
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
                    .map(|(c, _)| (p.kind, c.name.clone(), c.docs.clone()))
            });
        let (kind, name, docs) = match choice {
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
            buf.text.clone()
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

    fn buffer_preview(&self, key: &str) -> String {
        self.buffers
            .get(key)
            .map(|b| {
                b.text
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
                    .map(|(c, _)| (p.kind, c.name.clone()))
            });
        self.picker = None;
        let Some((kind, name)) = choice else {
            self.minibuffer_message("no candidate selected");
            return;
        };
        match kind {
            PickerKind::Palette => {
                let _ = self.dispatch(&name, None);
            }
            PickerKind::FindFile | PickerKind::RecentFiles => self.open_path(&name),
            PickerKind::Buffers => self.buffers.set_current(&name),
            PickerKind::KillBuffer => self.kill_buffer(&name),
            PickerKind::Projects => self.switch_project_root(&name),
        }
    }

    /// Switch the current project to `root` (a known-project registry
    /// entry), persist the registry, and land in the new project's file
    /// picker (projectile's default switch action is find-file).
    pub fn switch_project_root(&mut self, root: &str) {
        let root_path = PathBuf::from(root);
        let project = Project::new(root_path.clone());
        let name = project.name.clone();
        self.project = Some(project);
        self.project_store.registry.upsert(&root_path);
        let _ = self.project_store.save_registry();
        self.ensure_files();
        self.minibuffer_message(&format!("project: {name}"));
        self.open_find_file();
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

    // ── buffers ─────────────────────────────────────────────────────────

    /// Kill the buffer with key `key`; the current buffer falls back to
    /// a fresh `*scratch*` when it was the one killed.
    pub fn kill_buffer(&mut self, key: &str) {
        let display = self.buffer_display(key);
        if !self.buffers.kill(key) {
            self.minibuffer_message(&format!("no buffer: {display}"));
            return;
        }
        if self.buffers.current().is_none() {
            self.open_scratch();
        }
        self.minibuffer_message(&format!("killed {display}"));
    }

    /// Buffer-list view: open the selected buffer and close the list.
    pub fn open_buffer_list_selected(&mut self) {
        let key = match self.buffers.list().get(self.buffer_list_selected) {
            Some((key, _)) => key.to_string(),
            None => return,
        };
        self.buffers.set_current(&key);
        self.close_view();
    }

    pub fn buffer_list_next(&mut self) {
        let n = self.buffers.len();
        if n > 0 {
            self.buffer_list_selected = (self.buffer_list_selected + 1) % n;
        }
    }

    pub fn buffer_list_prev(&mut self) {
        let n = self.buffers.len();
        if n > 0 {
            self.buffer_list_selected = (self.buffer_list_selected + n - 1) % n;
        }
    }

    // ── keys & dispatch (unchanged skeleton from issue 01) ──────────────

    /// Feed one keypress from the terminal. While the picker is open,
    /// printable characters extend the query, Backspace/C-h edit it,
    /// a small set of keys drives the picker (RET runs, C-g cancels,
    /// arrows / C-n / C-p move); everything else goes to the keymap
    /// engine.
    pub fn key_event(&mut self, key: Key) {
        if self.quit {
            return;
        }
        if self.picker.is_some() {
            if let Some(c) = key.char_value() {
                self.picker_query_char(c);
                return;
            }
            // Query editing: Backspace (and C-h, the control-h byte some
            // terminals emit for it) removes the last query character.
            if key.code == KeyCode::Backspace || key == Key::ctrl_char('h') {
                self.picker_query_backspace();
                return;
            }
            if key.code == KeyCode::Enter {
                self.run_selected();
                return;
            }
            if key.code == KeyCode::Down || key == Key::ctrl_char('n') {
                self.picker_select_next();
                return;
            }
            if key.code == KeyCode::Up || key == Key::ctrl_char('p') {
                self.picker_select_prev();
                return;
            }
            // Other keys fall through to the keymap engine; the
            // unbound-key echo is suppressed while the picker is open
            // (see dispatch_key).
        }
        // C-g aborts from any state: it clears the pending sequence and
        // closes the picker before the keymap engine can start one.
        if key == Key::ctrl_char('g') {
            self.cancel();
            return;
        }
        self.dispatch_key(key);
    }

    fn dispatch_key(&mut self, key: Key) {
        let mut seq = self.pending.clone();
        seq.push(key);
        match self.engine.resolve(&seq) {
            Some(Lookup::Command(cmd)) => {
                let cmd = cmd.to_string();
                self.pending.clear();
                let _ = self.dispatch(&cmd, None);
            }
            Some(Lookup::Pending) => {
                self.pending = seq;
            }
            None => {
                self.pending.clear();
                if self.picker.is_none() {
                    self.minibuffer_message(&format!("unbound key: {key}"));
                }
            }
        }
    }

    /// Dispatch-by-name through the store's own registry: clone the
    /// command out (the registry is a field of this store, so the
    /// handler may not borrow it while running), then run it here.
    pub fn dispatch(&mut self, name: &str, arg: Option<String>) -> Result<(), RegistryError> {
        let command = self
            .registry
            .get(name)
            .cloned()
            .ok_or_else(|| RegistryError::UnknownCommand(name.to_string()))?;
        command.run(self, arg);
        Ok(())
    }

    /// C-g semantics: clear a pending sequence, close the picker, and
    /// echo a cancel notice in the minibuffer.
    pub fn cancel(&mut self) {
        let mut did = false;
        if !self.pending.is_empty() {
            self.pending.clear();
            did = true;
        }
        if self.picker.take().is_some() {
            did = true;
        }
        if did {
            self.minibuffer_message("cancel");
        }
    }

    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }

    pub fn minibuffer_message(&mut self, msg: &str) {
        self.message = msg.to_string();
    }
}

fn file_candidate(rel: &str) -> PickerCandidate {
    PickerCandidate {
        name: rel.to_string(),
        display: rel.to_string(),
        docs: String::new(),
        category: "file".to_string(),
    }
}

impl Default for AppStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Picker {
    fn recompute(&mut self, candidates: &[PickerCandidate], matcher: &mut Matcher) {
        let query = self.query.as_str();
        if query.is_empty() {
            self.filtered = candidates
                .iter()
                .map(|c| (c.clone(), u32::MAX))
                .collect();
            return;
        }
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let mut buf = Vec::new();
        let mut scored: Vec<(&PickerCandidate, u32)> = candidates
            .iter()
            .filter_map(|c| {
                let haystack = nucleo_matcher::Utf32Str::new(&c.display, &mut buf);
                pattern
                    .score(haystack, matcher)
                    .map(|score| (c, score))
            })
            .collect();
        scored.sort_by_key(|item| std::cmp::Reverse(item.1));
        self.filtered = scored.into_iter().map(|(c, s)| (c.clone(), s)).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::keymap::parse_key;

    /// A store rooted in `dir` as the project start, with persistence
    /// under a throwaway sibling base (never the project dir itself —
    /// the walk must not see the persistence files — and never the
    /// real cache dir).
    fn store(dir: &std::path::Path) -> AppStore {
        let base = tempfile::tempdir().unwrap();
        AppStore::at(dir, base.path().to_path_buf())
    }

    fn key(s: &str) -> Key {
        parse_key(s).unwrap()
    }

    #[test]
    fn unknown_keys_echo_in_minibuffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.key_event(key("Backspace"));
        assert_eq!(store.message, "unbound key: DEL");
        assert!(store.pending.is_empty());
    }

    #[test]
    fn pending_prefix_shows_and_cancels() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.key_event(key("C-x"));
        assert_eq!(store.pending_display(), "C-x");
        assert!(store.message.is_empty());

        store.key_event(key("C-g"));
        assert!(store.pending.is_empty());
        assert_eq!(store.message, "cancel");
    }

    #[test]
    fn pending_then_full_match_dispatches() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.key_event(key("C-x"));
        assert_eq!(store.pending_display(), "C-x");
        store.key_event(key("C-c"));
        assert!(store.quit);
        assert!(store.pending.is_empty());
    }

    #[test]
    fn unknown_key_after_prefix_cancels_pending() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.key_event(key("C-x"));
        assert_eq!(store.pending_display(), "C-x");
        store.key_event(key("C-z"));
        assert!(store.pending.is_empty());
        assert_eq!(store.message, "unbound key: C-z");
    }

    #[test]
    fn per_view_q_beats_global_but_global_still_reaches() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.key_event(key("q"));
        assert!(s.quit, "view-local `q` must bind quit");

        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.key_event(key("M-x"));
        assert!(s.picker_open());
    }

    #[test]
    fn issue_02_bindings_resolve_including_c_c_p_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        use crate::app::keymap::{Lookup, parse_sequence};
        let expect = |seq: &str, cmd: &str| {
            assert_eq!(
                store.engine.resolve(&parse_sequence(seq).unwrap()),
                Some(Lookup::Command(cmd)),
                "`{seq}` must resolve to `{cmd}`"
            );
        };
        expect("C-x C-f", "find-file");
        expect("C-x b", "switch-buffer");
        expect("C-x C-b", "list-buffers");
        expect("C-x k", "kill-buffer");
        expect("C-c p f", "find-file");
        expect("C-c p p", "switch-project");
        expect("C-c p e", "recent-files");
        expect("C-c p i", "re-walk");
        // The C-c p prefix path stays pending while building.
        assert_eq!(
            store.engine.resolve(&parse_sequence("C-c p").unwrap()),
            Some(Lookup::Pending)
        );
    }

    fn project_with_files(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.join("README.md"), "# readme\n").unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "// lib\n").unwrap();
    }

    #[test]
    fn detect_project_from_start_dir_and_show_name() {
        let dir = tempfile::tempdir().unwrap();
        let name = dir.path().file_name().unwrap().to_string_lossy().into_owned();
        project_with_files(dir.path());
        let store = store(dir.path());
        assert_eq!(store.project_display(), name);
        assert_eq!(store.view_name_display(), "*scratch*");
    }

    #[test]
    fn find_file_picker_opens_filters_and_opens_on_ret() {
        let dir = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap(); // long-lived: persistence assertions
        project_with_files(dir.path());
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());

        store.key_event(key("C-x"));
        store.key_event(key("C-f"));
        assert!(store.picker_open());
        assert_eq!(store.picker_kind(), Some(PickerKind::FindFile));
        assert_eq!(store.picker_prompt(), "Find file: ");
        assert_eq!(store.picker_count(), (4, 4));
        // Preview shows the first page of the selected (first) file:
        // Cargo.toml in the sorted list.
        assert!(
            store.picker_preview().contains("[package]"),
            "preview: {}",
            store.picker_preview()
        );

        // Type "main" → only src/main.rs survives; RET opens it.
        store.key_event(key("m"));
        store.key_event(key("a"));
        store.key_event(key("i"));
        store.key_event(key("n"));
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.display.as_str())
            .collect();
        assert_eq!(names, vec!["src/main.rs"], "{names:?}");
        store.key_event(key("RET"));
        assert!(!store.picker_open());
        assert_eq!(store.view_name_display(), "src/main.rs");
        assert!(store.buffer_text().contains("println!"));

        // The file made it into recents (persisted under the temp base).
        let root = store.project.as_ref().unwrap().root.to_string_lossy().into_owned();
        assert_eq!(store.project_store.recents.list(&root), vec!["src/main.rs"]);
        assert!(store.project_store.recents_path().is_file());
    }

    #[test]
    fn preview_follows_selection_movement() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_find_file();
        // Empty query: candidates are in sorted source order (no nucleo
        // scoring involved), so the preview expectations are exact.
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["Cargo.toml", "README.md", "src/lib.rs", "src/main.rs"],
            "{names:?}"
        );

        // Index 0: Cargo.toml.
        assert!(
            store.picker_preview().contains("[package]"),
            "preview: {}",
            store.picker_preview()
        );
        // Down moves to README.md and the preview must follow.
        store.key_event(key("DOWN"));
        assert_eq!(store.picker_selected(), 1);
        assert!(
            store.picker_preview().contains("# readme"),
            "preview after DOWN: {}",
            store.picker_preview()
        );
        // C-n moves the same way.
        store.key_event(key("C-n"));
        assert_eq!(store.picker_selected(), 2);
        assert!(
            store.picker_preview().contains("// lib"),
            "preview after C-n: {}",
            store.picker_preview()
        );
        // Up moves back to README.md.
        store.key_event(key("UP"));
        assert_eq!(store.picker_selected(), 1);
        assert!(
            store.picker_preview().contains("# readme"),
            "preview after UP: {}",
            store.picker_preview()
        );
        // Down to src/main.rs, then wrap at the end back to Cargo.toml.
        store.key_event(key("DOWN"));
        store.key_event(key("DOWN"));
        store.key_event(key("DOWN"));
        assert_eq!(store.picker_selected(), 0);
        assert!(store.picker_preview().contains("[package]"));
        // C-p at index 0 wrap-decrements to the last candidate.
        store.key_event(key("C-p"));
        assert_eq!(store.picker_selected(), 3);
        assert!(
            store.picker_preview().contains("println!"),
            "preview after wrap: {}",
            store.picker_preview()
        );
    }

    #[test]
    fn file_preview_stays_capped_for_huge_files() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        // Far beyond the 64KB preview cap: ~200 lines of ~1KB each
        // (~200KB total), with a marker past the cap.
        let line = "x".repeat(1000);
        let mut content = String::new();
        for i in 0..200 {
            if i == 100 {
                content.push_str("HUGE-FILE-MARKER\n");
            }
            content.push_str(&line);
            content.push('\n');
        }
        std::fs::write(dir.path().join("big.txt"), &content).unwrap();

        let mut store = store(dir.path());
        store.open_find_file();
        for c in "big".chars() {
            store.key_event(key(&c.to_string()));
        }
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["big.txt"], "{names:?}");

        let preview = store.picker_preview();
        // The first page: exactly 32 lines, all from the file head, and
        // nothing from past the byte cap.
        assert_eq!(preview.lines().count(), 32);
        assert!(preview.starts_with("xxxxxxxxxx"));
        assert!(!preview.contains("HUGE-FILE-MARKER"));
    }

    #[test]
    fn picker_query_backspace_edits_query() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_find_file();

        store.key_event(key("m"));
        store.key_event(key("a"));
        assert_eq!(store.picker_query(), "ma");
        store.key_event(key("Backspace"));
        assert_eq!(store.picker_query(), "m");
        // C-h (control-h) edits the query the same way.
        store.key_event(key("a"));
        store.key_event(key("C-h"));
        assert_eq!(store.picker_query(), "m");

        // Backspace empties the query (all candidates come back)…
        store.key_event(key("Backspace"));
        assert_eq!(store.picker_query(), "");
        // …and a Backspace on the empty query is a no-op (stays open).
        store.key_event(key("Backspace"));
        assert!(store.picker_open());
        assert_eq!(store.picker_query(), "");
    }

    #[test]
    fn unbound_key_echo_suppressed_while_picker_open() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_find_file();
        assert!(store.message.is_empty());
        // PageDown is unbound everywhere; with the picker open it must
        // NOT echo "unbound key".
        store.key_event(key("PGDN"));
        assert!(store.message.is_empty(), "echo leaked: {:?}", store.message);
        assert!(store.picker_open());
    }

    #[test]
    fn switch_buffer_and_kill_buffer() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_path("src/main.rs");
        store.open_path("src/lib.rs");
        assert_eq!(store.buffers.len(), 3); // + scratch

        // C-x b: switch to src/lib.rs.
        store.key_event(key("C-x"));
        store.key_event(key("b"));
        assert_eq!(store.picker_kind(), Some(PickerKind::Buffers));
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.display.as_str())
            .collect();
        assert!(names.contains(&"src/lib.rs"), "{names:?}");
        // Select the lib.rs row (find its index, drive selection there).
        let idx = names.iter().position(|n| *n == "src/lib.rs").unwrap();
        for _ in 0..idx {
            store.picker_select_next();
        }
        store.key_event(key("RET"));
        assert!(!store.picker_open());
        assert_eq!(store.view_name_display(), "src/lib.rs");

        // C-x k: kill it; the killed buffer disappears and current
        // falls back to *scratch*.
        store.key_event(key("C-x"));
        store.key_event(key("k"));
        assert_eq!(store.picker_kind(), Some(PickerKind::KillBuffer));
        let idx = store
            .picker_filtered()
            .iter()
            .position(|(c, _)| c.display == "src/lib.rs")
            .unwrap();
        for _ in 0..idx {
            store.picker_select_next();
        }
        store.key_event(key("RET"));
        assert_eq!(store.buffers.len(), 2); // scratch + main.rs
        assert_eq!(store.view_name_display(), "*scratch*");
        assert!(store.message.contains("killed src/lib.rs"));
    }

    #[test]
    fn list_buffers_view_and_buffer_list_keys() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_path("src/main.rs");

        store.dispatch("list-buffers", None).unwrap();
        assert_eq!(store.top_view(), ViewId::BufferList);
        let rows = store.buffer_rows();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|r| r.name == "src/main.rs" && r.current));
        assert!(rows.iter().any(|r| r.name == "*scratch*" && !r.current));

        // q closes the view; RET on the scratch row switches to it.
        store.key_event(key("q"));
        assert_eq!(store.top_view(), ViewId::Buffer);
        store.dispatch("list-buffers", None).unwrap();
        store.key_event(key("RET")); // row 0 is the MRU buffer (main.rs)
        assert_eq!(store.top_view(), ViewId::Buffer);
        assert_eq!(store.view_name_display(), "src/main.rs");
    }

    #[test]
    fn switch_project_lands_in_new_projects_file_picker() {
        let base = tempfile::tempdir().unwrap();
        // Two sibling projects sharing one persistence base.
        let p1 = base.path().join("alpha");
        let p2 = base.path().join("beta");
        std::fs::create_dir_all(p1.join("src")).unwrap();
        std::fs::create_dir_all(p2.join("src")).unwrap();
        std::fs::write(p1.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(p1.join("src/a.rs"), "a\n").unwrap();
        std::fs::write(p2.join("pyproject.toml"), "[project]\n").unwrap();
        std::fs::write(p2.join("src/b.py"), "b\n").unwrap();

        let mut store = AppStore::at(&p1, base.path().to_path_buf());
        assert_eq!(store.project_display(), "alpha");
        // Register beta in the known-project registry.
        store.project_store.registry.upsert(&p2);
        let _ = store.project_store.save_registry();

        store.key_event(key("C-c"));
        store.key_event(key("p"));
        assert_eq!(store.pending_display(), "C-c p");
        store.key_event(key("p"));
        assert!(store.picker_open());
        assert_eq!(store.picker_kind(), Some(PickerKind::Projects));
        // The current project is excluded from the candidates.
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.display.as_str())
            .collect();
        assert_eq!(names, vec!["beta"], "{names:?}");

        store.key_event(key("RET"));
        assert_eq!(store.project_display(), "beta");
        // Default switch action: land in beta's file picker.
        assert_eq!(store.picker_kind(), Some(PickerKind::FindFile));
        let files: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.display.as_str())
            .collect();
        assert!(files.contains(&"src/b.py"), "{files:?}");
        // The registry now has both, beta most recent.
        let roots: Vec<_> = store
            .project_store
            .registry
            .list()
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(roots, vec!["beta", "alpha"]);
    }

    #[test]
    fn recent_files_persist_across_restart() {
        let base = tempfile::tempdir().unwrap();
        let p = base.path().join("gamma");
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(p.join("src/one.rs"), "one\n").unwrap();
        std::fs::write(p.join("src/two.rs"), "two\n").unwrap();

        {
            let mut store = AppStore::at(&p, base.path().to_path_buf());
            store.open_path("src/one.rs");
            store.open_path("src/two.rs");
            store.open_path("src/one.rs"); // MRU again
        }
        // "Restart": a fresh store on the same base.
        let mut store = AppStore::at(&p, base.path().to_path_buf());
        let root = store.project.as_ref().unwrap().root.to_string_lossy().into_owned();
        assert_eq!(
            store.project_store.recents.list(&root),
            vec!["src/one.rs", "src/two.rs"],
            "recents must survive a restart, MRU first"
        );
        assert_eq!(
            store
                .project_store
                .registry
                .list()
                .iter()
                .map(|pr| pr.name.as_str())
                .collect::<Vec<_>>(),
            vec!["gamma"],
            "registry must survive a restart"
        );

        store.key_event(key("C-c"));
        store.key_event(key("p"));
        store.key_event(key("e"));
        assert_eq!(store.picker_kind(), Some(PickerKind::RecentFiles));
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.display.as_str())
            .collect();
        assert_eq!(names, vec!["src/one.rs", "src/two.rs"]);
    }

    #[test]
    fn re_walk_picks_up_new_files() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_find_file();
        assert_eq!(store.picker_count(), (4, 4));

        // A new file lands on disk; the cache still hides it…
        std::fs::write(dir.path().join("src/new.rs"), "new\n").unwrap();
        store.cancel();
        store.open_find_file();
        assert_eq!(store.picker_count(), (4, 4), "cache until re-walk");

        // …and the re-walk command invalidates it.
        store.dispatch("re-walk", None).unwrap();
        assert!(store.message.contains("5 files"));
        store.open_find_file();
        assert_eq!(store.picker_count(), (5, 5));
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.display.as_str())
            .collect();
        assert!(names.contains(&"src/new.rs"), "{names:?}");
    }

    #[test]
    fn find_file_without_project_explains_itself() {
        let dir = tempfile::tempdir().unwrap(); // no markers → no project
        let mut store = store(dir.path());
        store.key_event(key("C-x"));
        store.key_event(key("C-f"));
        assert!(!store.picker_open());
        assert!(store.message.contains("no project"));
    }

    #[test]
    fn open_path_missing_file_reports_error() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_path("src/nope.rs");
        assert!(store.message.contains("cannot open src/nope.rs"));
        assert_eq!(store.buffers.len(), 1); // still just scratch
    }

    #[test]
    fn palette_navigation_and_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.open_palette();
        assert_eq!(store.picker_count().0, 21);

        // Shipped UI path (M-x, Down, Up): Up must wrap-decrement, not
        // reflect — prev(1) is 0, not 8.
        store.key_event(key("DOWN"));
        assert_eq!(store.picker_selected(), 1);
        store.key_event(key("UP"));
        assert_eq!(store.picker_selected(), 0);

        // Mid-list: Up from an interior index decrements by exactly one.
        store.picker_select_next();
        store.picker_select_next(); // 0 -> 2
        store.picker_select_prev();
        assert_eq!(store.picker_selected(), 1);

        // Wrap at top: Up at index 0 lands on the last candidate.
        store.picker_select_prev(); // 1 -> 0
        store.picker_select_prev();
        assert_eq!(store.picker_selected(), 20);

        // C-p goes through the same wrap-decrement path as Up.
        store.key_event(key("C-p"));
        assert_eq!(store.picker_selected(), 19);

        // RET runs the candidate at the selected index (19: buffer-list-next).
        store.key_event(key("RET"));
        assert!(!store.picker_open());
        assert!(!store.quit);
    }

    #[test]
    fn palette_ret_runs_selected_command() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.open_palette();
        // Seed order: quit is first.
        store.key_event(key("RET"));
        assert!(!store.picker_open());
        assert!(store.quit);
    }

    #[test]
    fn palette_c_g_closes() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.open_palette();
        store.key_event(key("C-g"));
        assert!(!store.picker_open());
        assert_eq!(store.message, "cancel");
    }

    #[test]
    fn palette_query_filters_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.open_palette();
        assert_eq!(store.picker_count().0, 21);

        store.key_event(key("q"));
        store.key_event(key("u"));
        let names: Vec<_> = store
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.display.as_str())
            .collect();
        assert_eq!(names, vec!["quit"], "{names:?}");

        // Backspace now edits the query (carry-over from the issue 01
        // review): "qu" -> "q".
        store.key_event(key("Backspace"));
        assert!(store.picker_open());
        assert_eq!(store.picker_query(), "q");
    }

    #[test]
    fn insert_text_goes_to_current_buffer() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_path("src/main.rs");
        assert!(store.insert_text("demo text\n"));
        assert!(store.buffer_text().contains("demo text"));
    }
}


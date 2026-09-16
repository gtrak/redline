//! Central app store: the single source of truth the ui layer renders
//! and updates. Holds the view stack, keymap state, pending key
//! sequence, minibuffer message, status line state, quit flag, the
//! picker overlay state, the project layer (current project, known
//! projects, per-project recents, cached file lists), the open-buffer
//! set (ropey-backed), the highlight cache, per-buffer scroll state,
//! isearch state, and goto-line state.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nucleo_matcher::{
    Matcher, pattern::{CaseMatching, Normalization, Pattern},
};

use crate::app::command::{CommandRegistry, RegistryError};
use crate::app::config::Config;
use crate::app::events::{ChangeBus, ProjectChange};
use crate::app::keymap::{Key, KeyCode, KeyMap, KeySeq, KeymapEngine, Lookup, parse_sequence};
use crate::app::watcher::{ActiveWatcher, DEFAULT_DEBOUNCE};
use crate::git::diff::{DiffSide, FileDiff};
use crate::git::status::{RepoStatus, Side};
use crate::git::{GitError, GitRepo};
use crate::model::buffer::{load_file, BufferTable, SCRATCH_NAME};
use crate::model::files::FileList;
use crate::model::project::{detect_root, Project, ProjectStore};
use crate::model::sections::{MagitRow, SectionKind, StatusTree};
use crate::syntax::cache::{CacheKey, HighlightCache};
use crate::syntax::highlight::{self, HighlightResult};
use crate::syntax::registry::GrammarRegistry;
use crate::theme::Theme;

/// A view on the stack. The top of the stack is what the main view
/// renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewId {
    /// The main view: shows the current buffer's text.
    Buffer,
    /// The `C-x C-b` list-buffers view.
    BufferList,
    /// The `C-x g` magit status view (issue 07).
    MagitStatus,
}

impl ViewId {
    pub fn name(self) -> &'static str {
        match self {
            ViewId::Buffer => "buffer",
            ViewId::BufferList => "buffer-list",
            ViewId::MagitStatus => "magit-status",
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
                // Motion (issue 03).
                km.bind(&[Key::ctrl_char('n')], "scroll-line-down").unwrap();
                km.bind(&[Key::ctrl_char('p')], "scroll-line-up").unwrap();
                km.bind(&[Key::char('j')], "scroll-line-down").unwrap();
                km.bind(&[Key::char('k')], "scroll-line-up").unwrap();
                km.bind(&[Key::ctrl_char('v')], "scroll-page-down").unwrap();
                km.bind(&[Key::alt_char('v')], "scroll-page-up").unwrap();
                km.bind(&[Key::ctrl_char('d')], "scroll-half-page-down").unwrap();
                km.bind(&[Key::ctrl_char('u')], "scroll-half-page-up").unwrap();
                // `g` = force-reload the current file buffer (issue 04's
                // refresh role; scroll-top is still reachable via M-<).
                km.bind(&[Key::char('g')], "reload-buffer").unwrap();
                km.bind(&[Key::char('G')], "scroll-bottom").unwrap();
                km
                    .bind(&[Key::alt_char('g'), Key::char('g')], "goto-line")
                    .unwrap();
                km
                    .bind(&[Key::alt_char('<')], "scroll-top")
                    .unwrap();
                km
                    .bind(&[Key::alt_char('>')], "scroll-bottom")
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
            ViewId::MagitStatus => {
                // Magit dwim keys (issue 07): the section under the cursor
                // determines what `s`/`u`/`RET` do.
                let mut km = KeyMap::new();
                km.bind(&[Key::char('q')], "close-view").unwrap();
                km.bind(&[Key::char('s')], "magit-stage").unwrap();
                km.bind(&[Key::char('u')], "magit-unstage").unwrap();
                km.bind(&[Key::tab()], "magit-fold").unwrap();
                km.bind(&[Key::enter()], "magit-visit-file").unwrap();
                km.bind(&[Key::char('g')], "magit-refresh").unwrap();
                km.bind(&[Key::char('n')], "magit-next").unwrap();
                km.bind(&[Key::ctrl_char('n')], "magit-next").unwrap();
                km.bind(&[Key::char('p')], "magit-prev").unwrap();
                km.bind(&[Key::ctrl_char('p')], "magit-prev").unwrap();
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

/// One visible line in the file view: the text (without trailing
/// newline) and the highlight spans (byte offsets relative to the
/// line start). Pre-computed by the store; the file view renders it.
#[derive(Clone, Debug, Default)]
pub struct FileViewLine {
    pub text: String,
    pub spans: Vec<crate::syntax::highlight::LineSpan>,
}

/// Direction of an incremental search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsearchDirection {
    Forward,
    Backward,
}

/// Incremental in-buffer search state. The store owns this; the UI
/// only renders it (query, match count, current position).
#[derive(Debug)]
pub struct IsearchState {
    pub active: bool,
    pub query: String,
    pub direction: IsearchDirection,
    /// All match byte offsets in the current buffer (in search order).
    pub matches: Vec<usize>,
    /// Index into `matches` of the current match.
    pub current: usize,
    /// The line to restore to when isearch exits without a confirmed
    /// match (C-g cancel).
    pub pre_search_line: usize,
}

impl Default for IsearchState {
    fn default() -> Self {
        Self {
            active: false,
            query: String::new(),
            direction: IsearchDirection::Forward,
            matches: Vec::new(),
            current: 0,
            pre_search_line: 0,
        }
    }
}

/// Live dirty counts for the status line: staged (index vs HEAD),
/// unstaged (workdir vs index), and untracked file counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirtyCounts {
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
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
    /// Open buffers + the current buffer (ropey-backed).
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
    /// The open git repository (lazily discovered from the project root),
    /// cached so status/staging ops don't re-open it every time.
    git: Option<GitRepo>,
    /// The magit status section tree (issue 07), when built.
    status_tree: Option<StatusTree>,
    /// Live dirty counts for the status line, updated on each magit
    /// refresh (watcher-driven live refresh is issue 04).
    dirty: Option<DirtyCounts>,
    /// Grammar registry (built once at startup; all tree-sitter API
    /// churn is isolated in `src/syntax/registry.rs`).
    pub grammar_registry: GrammarRegistry,
    /// Highlight cache: keyed by (path, mtime, theme); bounded memory.
    pub highlight_cache: HighlightCache,
    /// Per-buffer scroll state: buffer key → top line (the first
    /// visible line). Preserved across view switches.
    scroll: HashMap<String, usize>,
    /// Incremental in-buffer search state (C-s / C-r).
    isearch: IsearchState,
    /// Goto-line mode active (M-g g).
    goto_line_active: bool,
    /// Goto-line digit input buffer.
    goto_line_input: String,
    /// Number of lines visible in the file view (set by the UI on
    /// resize); used for page-scroll and slice math.
    viewport_lines: usize,
    /// Project-change bus: the file watcher publishes here; the UI
    /// (FileView auto-reload) and git status (07's seam) subscribe.
    pub watch_bus: ChangeBus,
    /// The single active file watcher (exactly one per project at a time).
    watcher: Option<ActiveWatcher>,
    /// Live-reload changed files on disk (config default `true`).
    pub auto_reload: bool,
    /// Runtime suspend state for the watcher (`M-x toggle-watcher`);
    /// independent of the `auto_reload` config default.
    watch_suspended: bool,
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
        // Magit status (issue 07).
        global
            .bind(&[Key::ctrl_char('x'), Key::char('g')], "magit-status")
            .unwrap();
        // Isearch (issue 03).
        global.bind(&[Key::ctrl_char('s')], "isearch-forward").unwrap();
        global.bind(&[Key::ctrl_char('r')], "isearch-backward").unwrap();
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
            git: None,
            status_tree: None,
            dirty: None,
            grammar_registry: GrammarRegistry::build(),
            highlight_cache: HighlightCache::new(),
            scroll: HashMap::new(),
            isearch: IsearchState::default(),
            goto_line_active: false,
            goto_line_input: String::new(),
            viewport_lines: 24, // default; the UI updates on resize
            watch_bus: ChangeBus::new(),
            watcher: None,
            auto_reload: true,
            watch_suspended: false,
        }
    }

    /// Apply a config: select the theme and (re)bind key overrides.
    /// Returns an error describing the first unbindable override.
    pub fn apply_config(&mut self, config: &Config) -> Result<(), String> {
        self.theme = Theme::from(config.theme);
        self.auto_reload = config.auto_reload;
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
            ViewId::MagitStatus => "*magit-status*".to_string(),
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
    #[allow(dead_code)] // public API: used by tests and future UI layers
    pub fn buffer_text(&self) -> String {
        self.buffers
            .current_buffer()
            .map(|b| b.text())
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
                lines: buf.line_count() as u64,
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
    /// demo command); false when no buffer is current or not editable.
    pub fn insert_text(&mut self, text: &str) -> bool {
        let Some(key) = self.buffers.current() else {
            return false;
        };
        let key = key.to_string();
        // Check editable before mutating.
        let editable = self.buffers.get(&key).map(|b| b.editable).unwrap_or(false);
        if !editable {
            return false;
        }
        let pos = self
            .buffers
            .get(&key)
            .map(|b| b.rope.len_chars())
            .unwrap_or(0);
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.insert(pos, text);
            // A local edit: the buffer now differs from disk (the
            // light-editing flag, plan decision #6).
            buf.locally_modified = true;
            // Invalidate the highlight cache for this buffer (text changed).
            self.invalidate_highlight_for_key(&key);
            true
        } else {
            false
        }
    }

    /// Invalidate the highlight cache entry for a buffer key (called
    /// after an edit so the next render rebuilds the highlight).
    fn invalidate_highlight_for_key(&mut self, key: &str) {
        let buf = match self.buffers.get(key) {
            Some(b) => b,
            None => return,
        };
        let path = match &buf.path {
            Some(p) => p.clone(),
            None => return,
        };
        let cache_key = CacheKey::new(&path, buf.mtime, self.theme.name());
        // Remove the cache entry so it's rebuilt on next ensure_highlight.
        // (We can't do this cleanly without a remove method on HighlightCache;
        // instead we clear the whole cache — simple and correct.)
        if self.highlight_cache.get(&cache_key).is_some() {
            self.highlight_cache.clear();
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
            match load_file(&abs) {
                Ok((rope, mtime)) => {
                    self.buffers.insert_rope(Some(abs.clone()), rope, mtime, false);
                }
                Err(e) => {
                    self.minibuffer_message(&format!("cannot open {rel}: {e}"));
                    return;
                }
            }
        } else {
            // Re-stat on reopen: reload when mtime changed (spec: "highlight
            // cache invalidates when a file changes on disk (pre-watcher: on
            // reopen)"). Issue-03 buffers are read-only, so this is safe.
            if let Ok(new_mtime) = std::fs::metadata(&abs).and_then(|m| m.modified()) {
                let old_mtime = self
                    .buffers
                    .get(&key)
                    .map(|b| b.mtime)
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                if new_mtime != old_mtime {
                    match load_file(&abs) {
                        Ok((rope, mtime)) => {
                            self.buffers
                                .insert_rope(Some(abs.clone()), rope, mtime, false);
                        }
                        Err(e) => {
                            self.minibuffer_message(&format!("cannot reload {rel}: {e}"));
                        }
                    }
                }
            }
        }
        self.buffers.set_current(&key);
        self.record_recent(rel);
        // Build (or update) the highlight for the new current buffer.
        self.ensure_highlight();
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
        // Invalidate the cached git repo and magit state: they belong to
        // the previous project root and would otherwise be used against
        // the wrong repository after the switch.
        self.git = None;
        self.status_tree = None;
        self.dirty = None;
        // Swap the file watcher: stop the old project's watcher and start
        // one for the new root (exactly one at a time). No-op in plain unit
        // tests (no runtime).
        self.start_watcher();
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

    // ── file view: scroll + isearch + goto-line (issue 03) ────────────

    /// Set the viewport height (in lines); called by the UI on resize.
    pub fn set_viewport_lines(&mut self, n: usize) {
        self.viewport_lines = n.max(1);
    }

    /// The scroll position (top line) for the current buffer.
    pub fn scroll_top(&self) -> usize {
        self.buffers
            .current()
            .and_then(|key| self.scroll.get(key))
            .copied()
            .unwrap_or(0)
    }

    /// Set the scroll position for the current buffer.
    fn set_scroll_top(&mut self, top: usize) {
        let key = self.buffers.current().map(String::from);
        if let Some(key) = key {
            let total = self
                .buffers
                .get(&key)
                .map(|b| b.line_count())
                .unwrap_or(0);
            self.scroll.insert(key, top.min(total.saturating_sub(1)));
        }
    }

    /// Scroll down by one line.
    pub fn scroll_line_down(&mut self) {
        let top = self.scroll_top();
        self.set_scroll_top(top + 1);
    }

    /// Scroll up by one line.
    pub fn scroll_line_up(&mut self) {
        let top = self.scroll_top();
        self.set_scroll_top(top.saturating_sub(1));
    }

    /// Scroll down by one page (viewport height).
    pub fn scroll_page_down(&mut self) {
        let top = self.scroll_top();
        self.set_scroll_top(top + self.viewport_lines);
    }

    /// Scroll up by one page (viewport height).
    pub fn scroll_page_up(&mut self) {
        let top = self.scroll_top();
        self.set_scroll_top(top.saturating_sub(self.viewport_lines));
    }

    /// Scroll down by half a page.
    pub fn scroll_half_page_down(&mut self) {
        let top = self.scroll_top();
        self.set_scroll_top(top + self.viewport_lines / 2);
    }

    /// Scroll up by half a page.
    pub fn scroll_half_page_up(&mut self) {
        let top = self.scroll_top();
        self.set_scroll_top(top.saturating_sub(self.viewport_lines / 2));
    }

    /// Scroll to the top (line 0).
    pub fn scroll_to_top(&mut self) {
        self.set_scroll_top(0);
    }

    /// Scroll to the bottom (last visible line).
    pub fn scroll_to_bottom(&mut self) {
        let total = self
            .buffers
            .current_buffer()
            .map(|b| b.line_count())
            .unwrap_or(0);
        let top = total.saturating_sub(self.viewport_lines);
        self.set_scroll_top(top);
    }

    /// Pre-compute the visible lines for the file view: text from the
    /// rope, spans from the highlight cache (or empty for plain text).
    /// The visible range is `[top_line, top_line + viewport_lines)`.
    pub fn file_view_lines(&self) -> Vec<FileViewLine> {
        let Some(buf) = self.buffers.current_buffer() else {
            return Vec::new();
        };
        let total = buf.line_count();
        if total == 0 {
            return Vec::new();
        }
        let top = self.scroll_top();
        let start = top.min(total.saturating_sub(1));
        let end = (top + self.viewport_lines).min(total);
        if start >= end {
            return Vec::new();
        }

        // Get the highlight result from the cache (if any).
        let highlight: Option<&HighlightResult> = self.buffer_highlight_result();

        let mut out = Vec::with_capacity(end - start);
        for line in start..end {
            let text = buf.line_text(line).unwrap_or_default().to_string();
            let spans = highlight
                .and_then(|h| h.lines.get(line))
                .map(|hl| hl.spans.clone())
                .unwrap_or_default();
            out.push(FileViewLine { text, spans });
        }
        out
    }

    /// (top_line, total_lines, viewport_lines) for the file view.
    pub fn file_view_scroll_info(&self) -> (usize, usize, usize) {
        let total = self
            .buffers
            .current_buffer()
            .map(|b| b.line_count())
            .unwrap_or(0);
        (self.scroll_top(), total, self.viewport_lines)
    }

    /// The highlight result for the current buffer, from the cache.
    /// Returns `None` when the buffer is plain text, big-file, or the
    /// cache has no entry for the current (path, mtime, theme).
    fn buffer_highlight_result(&self) -> Option<&HighlightResult> {
        let key = self.buffers.current()?.to_string();
        let buf = self.buffers.get(&key)?;
        // Big files skip highlighting entirely.
        if buf.is_big() {
            return None;
        }
        let path = buf.path.as_ref()?;
        let lang = self.grammar_registry.language_for(&path.to_string_lossy());
        if lang == crate::syntax::registry::LanguageId::Plain {
            return None;
        }
        let cache_key = CacheKey::new(path, buf.mtime, self.theme.name());
        self.highlight_cache.get(&cache_key)
    }

    /// Ensure the current buffer's highlight is cached; builds it on
    /// a cache miss. Called on buffer open and on theme change.
    pub fn ensure_highlight(&mut self) {
        if let Some(key) = self.buffers.current().map(str::to_string) {
            self.ensure_highlight_for_key(&key);
        }
    }

    /// Build (or reuse) the highlight for the buffer with `key`; a no-op
    /// when the buffer is plain text, big, or already cached for the
    /// current (path, mtime, theme). Used by reloads so a re-read buffer is
    /// re-highlighted immediately (cache invalidation by mtime).
    fn ensure_highlight_for_key(&mut self, key: &str) {
        let (path, mtime, big) = {
            let Some(buf) = self.buffers.get(key) else { return };
            (buf.path.clone(), buf.mtime, buf.is_big())
        };
        let Some(path) = path else { return };
        if big {
            return;
        }
        let path_str = path.to_string_lossy().into_owned();
        let lang = self.grammar_registry.language_for(&path_str);
        if lang == crate::syntax::registry::LanguageId::Plain {
            return;
        }
        let cache_key = CacheKey::new(&path, mtime, self.theme.name());
        if self.highlight_cache.get(&cache_key).is_some() {
            return; // cache hit
        }
        // Cache miss: build the highlight.
        let rope = {
            let buf = self.buffers.get(key).unwrap();
            // Rope clone is O(1) (data sharing).
            buf.rope.clone()
        };
        let config = match self.grammar_registry.config(lang) {
            Some(c) => c,
            None => return,
        };
        match highlight::highlight(&rope, config, lang) {
            Ok(result) => {
                self.highlight_cache.insert(cache_key, result);
            }
            Err(()) => {
                tracing::warn!("highlight failed for {path_str}");
            }
        }
    }

    // ── isearch (C-s / C-r) ────────────────────────────────────────────

    /// Start an incremental search in the given direction.
    /// Records the pre-search line for clean exit (C-g restores it).
    pub fn isearch_start(&mut self, direction: IsearchDirection) {
        if self.isearch.active {
            return; // already active; C-s during isearch is a no-op
        }
        self.isearch = IsearchState {
            active: true,
            query: String::new(),
            direction,
            matches: Vec::new(),
            current: 0,
            pre_search_line: self.scroll_top(),
        };
        self.minibuffer_message("I-search: ");
    }

    /// Append a character to the isearch query and update matches.
    fn isearch_query_char(&mut self, c: char) {
        self.isearch.query.push(c);
        self.isearch_recompute();
    }

    /// Remove the last character from the isearch query.
    fn isearch_backspace(&mut self) {
        self.isearch.query.pop();
        self.isearch_recompute();
    }

    /// Recompute all matches for the current query and jump to the
    /// first match in the search direction.
    fn isearch_recompute(&mut self) {
        let query = self.isearch.query.clone();
        if query.is_empty() {
            self.isearch.matches.clear();
            self.isearch.current = 0;
            self.minibuffer_message("I-search: ");
            return;
        }
        // Get the full buffer text for searching.
        let text = self
            .buffers
            .current_buffer()
            .map(|b| b.text())
            .unwrap_or_default();
        self.isearch.matches = Self::find_all_matches(&text, &query, self.isearch.direction);
        if self.isearch.matches.is_empty() {
            self.isearch.current = 0;
            self.minibuffer_message(&format!("I-search: {query} [no matches]"));
        } else {
            // Jump to the first match in the search direction.
            let start_line = self.scroll_top();
            let start_byte = self
                .buffers
                .current_buffer()
                .and_then(|b| b.try_line_to_byte(start_line))
                .unwrap_or(0);
            // Find the first match at or after start_byte (forward)
            // or at or before start_byte (backward).
            self.isearch.current = match self.isearch.direction {
                IsearchDirection::Forward => self
                    .isearch
                    .matches
                    .iter()
                    .position(|&m| m >= start_byte)
                    .unwrap_or(0),
                IsearchDirection::Backward => {
                    // Find the last match at or before start_byte.
                    self.isearch
                        .matches
                        .iter()
                        .rposition(|&m| m <= start_byte)
                        .unwrap_or(self.isearch.matches.len().saturating_sub(1))
                }
            };
            self.isearch_jump_to_current();
            let count = self.isearch.matches.len();
            let idx = self.isearch.current + 1;
            self.minibuffer_message(&format!("I-search: {query} [{idx}/{count}]"));
        }
    }

    /// Navigate to the next match (n) with wrap-around.
    pub fn isearch_next(&mut self) {
        if !self.isearch.active || self.isearch.matches.is_empty() {
            return;
        }
        let n = self.isearch.matches.len();
        self.isearch.current = (self.isearch.current + 1) % n;
        self.isearch_jump_to_current();
        let count = n;
        let idx = self.isearch.current + 1;
        let query = self.isearch.query.clone();
        self.minibuffer_message(&format!("I-search: {query} [{idx}/{count}]"));
    }

    /// Navigate to the previous match (N) with wrap-around.
    pub fn isearch_prev(&mut self) {
        if !self.isearch.active || self.isearch.matches.is_empty() {
            return;
        }
        let n = self.isearch.matches.len();
        self.isearch.current = (self.isearch.current + n - 1) % n;
        self.isearch_jump_to_current();
        let count = n;
        let idx = self.isearch.current + 1;
        let query = self.isearch.query.clone();
        self.minibuffer_message(&format!("I-search: {query} [{idx}/{count}]"));
    }

    /// Scroll the view to show the current match.
    fn isearch_jump_to_current(&mut self) {
        let Some(&match_byte) = self.isearch.matches.get(self.isearch.current) else {
            return;
        };
        let line = self
            .buffers
            .current_buffer()
            .and_then(|b| b.try_byte_to_line(match_byte))
            .unwrap_or(0);
        self.set_scroll_top(line);
    }

    /// Confirm isearch (RET): keep the current position, deactivate.
    pub fn isearch_confirm(&mut self) {
        if !self.isearch.active {
            return;
        }
        let query = self.isearch.query.clone();
        self.isearch.active = false;
        if query.is_empty() {
            self.minibuffer_message("");
        } else if self.isearch.matches.is_empty() {
            self.minibuffer_message(&format!("I-search: {query} [not found]"));
        } else {
            self.minibuffer_message("");
        }
    }

    /// Cancel isearch (C-g): restore the pre-search position.
    pub fn isearch_cancel(&mut self) {
        if !self.isearch.active {
            return;
        }
        self.isearch.active = false;
        self.isearch.matches.clear();
        self.isearch.query.clear();
        self.set_scroll_top(self.isearch.pre_search_line);
        self.minibuffer_message("cancel");
    }

    /// Whether isearch is currently active.
    #[allow(dead_code)] // public API: used by tests and future UI layers
    pub fn isearch_active(&self) -> bool {
        self.isearch.active
    }

    /// The isearch match count.
    #[allow(dead_code)] // public API: used by tests and future UI layers
    pub fn isearch_match_count(&self) -> usize {
        self.isearch.matches.len()
    }

    /// The current match index (1-based, for display).
    #[allow(dead_code)] // public API: used by tests and future UI layers
    pub fn isearch_match_index(&self) -> usize {
        self.isearch.current + 1
    }

    // ── goto-line (M-g g) ──────────────────────────────────────────────

    /// Start goto-line mode.
    pub fn goto_line_start(&mut self) {
        self.goto_line_active = true;
        self.goto_line_input.clear();
        self.minibuffer_message("Go to line: ");
    }

    /// Whether goto-line mode is active.
    #[allow(dead_code)] // public API: used by tests and future UI layers
    pub fn goto_line_active(&self) -> bool {
        self.goto_line_active
    }

    /// The goto-line input buffer (for display).
    #[allow(dead_code)] // public API: used by tests and future UI layers
    pub fn goto_line_input(&self) -> &str {
        &self.goto_line_input
    }

    /// Append a digit to the goto-line input.
    fn goto_line_digit(&mut self, c: char) {
        self.goto_line_input.push(c);
        self.minibuffer_message(&format!("Go to line: {}", self.goto_line_input));
    }

    /// Backspace in goto-line input.
    fn goto_line_backspace(&mut self) {
        self.goto_line_input.pop();
        self.minibuffer_message(&format!("Go to line: {}", self.goto_line_input));
    }

    /// Confirm goto-line (RET): scroll to the target line.
    pub fn goto_line_confirm(&mut self) {
        if !self.goto_line_active {
            return;
        }
        self.goto_line_active = false;
        let input = self.goto_line_input.clone();
        self.goto_line_input.clear();
        if let Ok(line) = input.parse::<usize>() {
            let total = self
                .buffers
                .current_buffer()
                .map(|b| b.line_count())
                .unwrap_or(0);
            if line < total {
                self.set_scroll_top(line);
                self.minibuffer_message("");
            } else {
                self.minibuffer_message(&format!("line {line} out of range (1-{total})"));
            }
        } else {
            self.minibuffer_message("invalid line number");
        }
    }

    /// Cancel goto-line (C-g).
    pub fn goto_line_cancel(&mut self) {
        if !self.goto_line_active {
            return;
        }
        self.goto_line_active = false;
        self.goto_line_input.clear();
        self.minibuffer_message("cancel");
    }

    /// Find all occurrences of `query` in `text` (case-sensitive, plain
    /// text). Returns byte offsets in search order.
    fn find_all_matches(text: &str, query: &str, direction: IsearchDirection) -> Vec<usize> {
        if query.is_empty() {
            return Vec::new();
        }
        let mut matches = Vec::new();
        let mut search_from = 0;
        while let Some(pos) = text[search_from..].find(query) {
            let match_start = search_from + pos;
            matches.push(match_start);
            // Advance to the next char boundary after the match start
            // (avoids landing mid-UTF-8-char on multibyte matches).
            let mut next = match_start + 1;
            while next < text.len() && !text.is_char_boundary(next) {
                next += 1;
            }
            search_from = next;
        }
        if direction == IsearchDirection::Backward {
            matches.reverse();
        }
        matches
    }

    // ── magit status (issue 07) ─────────────────────────────────────────

    /// Open (or re-focus) the magit status buffer: ensure the repo is
    /// open, refresh the section tree, and push the view if needed.
    pub fn open_magit_status(&mut self) {
        let Some(project) = self.project.clone() else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        if !self.ensure_git(project.root) {
            return;
        }
        if !self.refresh_magit() {
            return;
        }
        if self.top_view() != ViewId::MagitStatus {
            self.push_view(ViewId::MagitStatus);
        }
    }

    /// `g`: manually refresh the status (watcher-driven auto-refresh is
    /// issue 04; the watcher bus will subscribe here).
    pub fn magit_refresh(&mut self) {
        if self.refresh_magit() {
            self.minibuffer_message("status refreshed");
        }
    }

    /// `TAB`: fold/unfold the section under the cursor.
    pub fn magit_toggle_fold(&mut self) {
        if let Some(t) = self.status_tree.as_mut() {
            t.toggle_fold();
        }
    }

    /// `n` / `C-n`: move the cursor to the next visible section.
    pub fn magit_cursor_down(&mut self) {
        if let Some(t) = self.status_tree.as_mut() {
            t.move_down();
        }
    }

    /// `p` / `C-p`: move the cursor to the previous visible section.
    pub fn magit_cursor_up(&mut self) {
        if let Some(t) = self.status_tree.as_mut() {
            t.move_up();
        }
    }

    /// `RET`: visit the file under the cursor through the buffer model
    /// (issue 02) at the file level, then return to the buffer view.
    /// TODO(issue 03): jump to the hunk offset once FileView lands.
    pub fn magit_visit_file(&mut self) {
        let path = match self
            .status_tree
            .as_ref()
            .and_then(|t| t.cursor_target())
            .and_then(|t| t.path)
        {
            Some(p) => p,
            None => {
                self.minibuffer_message("no file under point");
                return;
            }
        };
        self.open_path(&path);
        if self.top_view() == ViewId::MagitStatus {
            self.close_view();
        }
    }

    /// `s`: stage the change at point (file or hunk on the unstaged side,
    /// or an untracked file), then refresh.
    pub fn magit_stage(&mut self) {
        let Some(target) = self.status_tree.as_ref().and_then(|t| t.cursor_target()) else {
            self.minibuffer_message("no section under point");
            return;
        };
        let Some(path) = target.path else {
            self.minibuffer_message("no file under point");
            return;
        };
        let result = match (target.kind, target.side) {
            (SectionKind::File, Some(Side::Unstaged) | Some(Side::Untracked)) => {
                self.with_git(move |g| g.stage_file(&path))
            }
            (SectionKind::Hunk, Some(Side::Unstaged)) => {
                let start = target.hunk_new_start.unwrap_or(0);
                self.with_git(move |g| g.stage_hunk(&path, start))
            }
            _ => {
                self.minibuffer_message("nothing to stage at point");
                return;
            }
        };
        match result {
            Ok(()) => {
                self.refresh_magit();
            }
            Err(e) => self.minibuffer_message(&format!("stage failed: {e}")),
        }
    }

    /// `u`: unstage the change at point (file or hunk on the staged side),
    /// then refresh.
    pub fn magit_unstage(&mut self) {
        let Some(target) = self.status_tree.as_ref().and_then(|t| t.cursor_target()) else {
            self.minibuffer_message("no section under point");
            return;
        };
        let Some(path) = target.path else {
            self.minibuffer_message("no file under point");
            return;
        };
        let result = match (target.kind, target.side) {
            (SectionKind::File, Some(Side::Staged)) => {
                let orig = target.orig.clone();
                self.with_git(move |g| g.unstage_file(&path, orig.as_deref()))
            }
            (SectionKind::Hunk, Some(Side::Staged)) => {
                let start = target.hunk_new_start.unwrap_or(0);
                self.with_git(move |g| g.unstage_hunk(&path, start))
            }
            _ => {
                self.minibuffer_message("nothing to unstage at point");
                return;
            }
        };
        match result {
            Ok(()) => {
                self.refresh_magit();
            }
            Err(e) => self.minibuffer_message(&format!("unstage failed: {e}")),
        }
    }

    /// The magit status rows for the current fold state (empty when the
    /// tree has not been built).
    pub fn magit_rows(&self) -> Vec<MagitRow> {
        self.status_tree
            .as_ref()
            .map(|t| t.visible_rows())
            .unwrap_or_default()
    }

    /// Live dirty counts for the status line (`None` until the first
    /// refresh).
    pub fn dirty_counts(&self) -> Option<DirtyCounts> {
        self.dirty
    }

    /// Run `f` against the cached repo, opening a `NotARepository` error
    /// when none is open.
    fn with_git<R>(&self, f: impl FnOnce(&GitRepo) -> Result<R, GitError>) -> Result<R, GitError> {
        match self.git.as_ref() {
            Some(g) => f(g),
            None => Err(GitError::NotARepository {
                path: self
                    .project
                    .as_ref()
                    .map(|p| p.root.clone())
                    .unwrap_or_else(|| PathBuf::from(".")),
            }),
        }
    }

    /// Open (or keep) the cached git repo at `root`.
    fn ensure_git(&mut self, root: PathBuf) -> bool {
        if self.git.is_none() {
            match GitRepo::discover(&root) {
                Ok(g) => self.git = Some(g),
                Err(e) => {
                    self.minibuffer_message(&e.to_string());
                    return false;
                }
            }
        }
        true
    }

    fn git_status(&self) -> Result<RepoStatus, GitError> {
        match self.git.as_ref() {
            Some(g) => g.status(),
            None => Err(GitError::NotARepository {
                path: self
                    .project
                    .as_ref()
                    .map(|p| p.root.clone())
                    .unwrap_or_else(|| PathBuf::from(".")),
            }),
        }
    }

    /// Fetch per-file diffs for every changed file (staged and unstaged
    /// sides). Untracked files have no diff.
    fn collect_diffs(&self, status: &RepoStatus) -> (HashMap<String, FileDiff>, HashMap<String, FileDiff>) {
        let git = match self.git.as_ref() {
            Some(g) => g,
            None => return (HashMap::new(), HashMap::new()),
        };
        let mut staged = HashMap::new();
        let mut unstaged = HashMap::new();
        for f in &status.files {
            if f.is_staged()
                && let Ok(d) = git.diff(DiffSide::Staged, &f.path)
            {
                staged.insert(f.path.clone(), d);
            }
            if f.is_unstaged()
                && let Ok(d) = git.diff(DiffSide::Unstaged, &f.path)
            {
                unstaged.insert(f.path.clone(), d);
            }
        }
        (staged, unstaged)
    }

    /// Rebuild the status section tree (preserving fold + cursor) and the
    /// dirty counts from a fresh status. Returns false (and sets the
    /// minibuffer) on a git error.
    fn refresh_magit(&mut self) -> bool {
        let status = match self.git_status() {
            Ok(s) => s,
            Err(e) => {
                self.minibuffer_message(&format!("git status failed: {e}"));
                return false;
            }
        };
        let (staged, unstaged) = self.collect_diffs(&status);
        let prev = self.status_tree.clone();
        let tree = StatusTree::build(&status, &staged, &unstaged, prev.as_ref());
        self.dirty = Some(DirtyCounts {
            staged: status.staged_count(),
            unstaged: status.unstaged_count(),
            untracked: status.untracked_count(),
        });
        self.status_tree = Some(tree);
        true
    }

    // ── file watching (issue 04) ───────────────────────────────────────

    /// The project-change bus (subscribers: FileView auto-reload, git status).
    pub fn watch_bus(&self) -> &ChangeBus {
        &self.watch_bus
    }

    /// Whether a file watcher is currently active (0 or 1 — exactly one at a
    /// time). Exposed for test diagnostics (the spec's "handle count" check).
    #[allow(dead_code)]
    pub fn watcher_count(&self) -> usize {
        usize::from(self.watcher.is_some())
    }

    /// The root the active watcher is watching (exactly one at a time).
    /// Exposed for test diagnostics.
    #[allow(dead_code)]
    pub fn watcher_active_root(&self) -> Option<&PathBuf> {
        self.watcher.as_ref().map(|w| &w.root)
    }

    /// Whether the watcher is runtime-suspended (`M-x toggle-watcher`).
    /// Exposed for test diagnostics.
    #[allow(dead_code)]
    pub fn watcher_suspended(&self) -> bool {
        self.watch_suspended
    }

    /// Start (or replace) the debounced watcher for the current project with
    /// the default debounce window. No-op when there's no project, watching is
    /// disabled (`auto_reload`), or it's suspended.
    pub fn start_watcher(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            return;
        };
        self.start_watcher_at(&root, DEFAULT_DEBOUNCE);
    }

    /// Start a watcher for `root` with `debounce`. Stops any current watcher
    /// first (exactly one at a time) and skips the spawn when no runtime is
    /// available (plain unit tests have none; production + tokio tests do).
    pub fn start_watcher_at(&mut self, root: &Path, debounce: Duration) {
        if !self.auto_reload || self.watch_suspended {
            self.stop_watcher();
            return;
        }
        if self.watcher.as_ref().is_some_and(|w| w.root.as_path() == root) {
            return; // already watching this root
        }
        self.stop_watcher();
        if tokio::runtime::Handle::try_current().is_err() {
            return; // no runtime (plain unit test): nothing to spawn into
        }
        match crate::app::watcher::start_watch(root, &self.watch_bus, debounce) {
            Some(w) => {
                self.watcher = Some(w);
                tracing::info!(root = %root.display(), "watcher started");
            }
            None => {
                self.minibuffer_message("file watching unavailable (inotify limit?)");
            }
        }
    }

    /// Stop the current watcher (signal the consumer to tear down the
    /// debouncer + notify + pump threads). Idempotent.
    pub fn stop_watcher(&mut self) {
        if let Some(mut w) = self.watcher.take() {
            w.stop();
        }
    }

    /// `M-x toggle-watcher`: suspend/resume live watching. Suspend stops the
    /// watcher (events are dropped, not buffered); resume starts a fresh one.
    pub fn toggle_watcher(&mut self) {
        self.watch_suspended = !self.watch_suspended;
        if self.watch_suspended {
            self.stop_watcher();
            self.minibuffer_message("file watching suspended (M-x toggle-watcher)");
        } else {
            self.start_watcher();
            self.minibuffer_message("file watching resumed (M-x toggle-watcher)");
        }
    }

    /// Take the active watcher out of the store (brief mutable borrow) so the
    /// caller can tear it down WITHOUT holding the store lock across an
    /// await. The caller owns the returned `ActiveWatcher`. `None` when no
    /// watcher is active.
    pub fn take_watcher(&mut self) -> Option<ActiveWatcher> {
        self.watcher.take()
    }

    /// Apply a project change: auto-reload non-locally-owned buffers whose
    /// path changed (preserving the scroll anchor), set the conflict marker
    /// on locally-owned ones, and refresh git status (07's seam).
    ///
    /// This is the FileView / git-status subscription entry point.
    pub fn apply_project_change(&mut self, change: &ProjectChange) {
        if change.is_empty() {
            return;
        }
        let mut reloaded = 0usize;
        let mut conflicts = 0usize;
        for path in &change.paths {
            let key = path.to_string_lossy().into_owned();
            // Read the buffer's state without holding a borrow across the
            // mutable reload / conflict update below.
            let (matches, locally_owned) = match self.buffers.get(&key) {
                Some(buf) if buf.path.as_ref() == Some(path) => {
                    (true, buf.is_locally_owned())
                }
                _ => (false, false),
            };
            if !matches {
                continue;
            }
            if locally_owned {
                // Conflict: never auto-clobber a locally-owned buffer.
                if let Some(b) = self.buffers.get_mut(&key) {
                    b.changed_on_disk = true;
                }
                conflicts += 1;
            } else if self.reload_buffer(path) {
                reloaded += 1;
            }
        }
        // git status refresh (07's seam): only when a repo is open, so a
        // non-git project never spams a failure message on every change.
        if self.git.is_some() {
            self.refresh_magit();
        }
        if conflicts > 0 && reloaded == 0 {
            // The user should know their edit conflicts with a disk change.
            self.minibuffer_message("changed on disk — press g to reload");
        }
    }

    /// Re-read the file buffer at `path` from disk, preserving the scroll
    /// anchor (same line number if it still exists, else clamp). Returns true
    /// when the buffer existed and was reloaded.
    fn reload_buffer(&mut self, path: &Path) -> bool {
        let key = path.to_string_lossy().into_owned();
        let old_top = self.scroll.get(&key).copied().unwrap_or(0);
        let (rope, mtime) = match load_file(path) {
            Ok(x) => x,
            // File vanished / unreadable: keep the old content; no error
            // spam on every change event.
            Err(_) => return false,
        };
        let new_total = rope.len_lines();
        let new_top = reload_anchor(old_top, new_total);
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope = rope;
            buf.mtime = mtime;
            buf.changed_on_disk = false;
        }
        self.scroll.insert(key.clone(), new_top);
        self.ensure_highlight_for_key(&key);
        true
    }

    /// `g`: force-reload the current file buffer from disk, superseding any
    /// conflict (clears the changed-on-disk marker and the local-modified
    /// flag). The scratch buffer (no path) is handled gracefully.
    pub fn reload_current_buffer(&mut self) {
        let Some(key) = self.buffers.current().map(str::to_string) else {
            self.minibuffer_message("no buffer to reload");
            return;
        };
        let Some(path) = self.buffers.get(&key).and_then(|b| b.path.clone()) else {
            // Scratch (no path): nothing on disk to reload.
            self.minibuffer_message("no file to reload (scratch buffer)");
            return;
        };
        let old_top = self.scroll.get(&key).copied().unwrap_or(0);
        let (rope, mtime) = match load_file(&path) {
            Ok(x) => x,
            Err(e) => {
                self.minibuffer_message(&format!("reload failed: {e}"));
                return;
            }
        };
        let new_total = rope.len_lines();
        let new_top = reload_anchor(old_top, new_total);
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope = rope;
            buf.mtime = mtime;
            buf.changed_on_disk = false;
            buf.locally_modified = false; // force reload supersedes local edits
        }
        self.scroll.insert(key.clone(), new_top);
        self.ensure_highlight_for_key(&key);
        self.minibuffer_message("reloaded");
    }

    /// Mark a buffer as locally modified (the light-editing hook; also used
    /// by tests to exercise the conflict logic before the editing UI lands).
    #[allow(dead_code)]
    pub fn mark_locally_modified(&mut self, key: &str) {
        if let Some(buf) = self.buffers.get_mut(key) {
            buf.locally_modified = true;
        }
    }

    /// Whether the current buffer has an un-reconciled disk change (the
    /// "changed on disk" conflict marker shown in the file view).
    pub fn current_buffer_changed_on_disk(&self) -> bool {
        self.buffers
            .current_buffer()
            .map(|b| b.changed_on_disk)
            .unwrap_or(false)
    }

    // ── keys & dispatch (unchanged skeleton from issue 01) ──────────────

    /// Feed one keypress from the terminal. While the picker is open,
    /// printable characters extend the query, Backspace/C-h edit it,
    /// a small set of keys drives the picker (RET runs, C-g cancels,
    /// arrows / C-n / C-p move); everything else goes to the keymap
    /// engine. While isearch is active, printable characters extend the
    /// query, n/N navigate, RET confirms, C-g cancels. While goto-line
    /// is active, digits build the line number, RET confirms, C-g
    /// cancels.
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
        // Isearch mode: printable chars extend the query, n/N navigate,
        // RET confirms, C-g cancels (restores pre-search position).
        if self.isearch.active {
            if key == Key::ctrl_char('g') {
                self.isearch_cancel();
                return;
            }
            if key.code == KeyCode::Enter {
                self.isearch_confirm();
                return;
            }
            if key == Key::char('n') {
                self.isearch_next();
                return;
            }
            if key == Key::char('N') {
                self.isearch_prev();
                return;
            }
            if let Some(c) = key.char_value() {
                self.isearch_query_char(c);
                return;
            }
            if key.code == KeyCode::Backspace || key == Key::ctrl_char('h') {
                self.isearch_backspace();
                return;
            }
            // Other keys: swallow (don't echo "unbound key" mid-search).
            return;
        }
        // Goto-line mode: digits build the line number, RET confirms,
        // C-g cancels.
        if self.goto_line_active {
            if key == Key::ctrl_char('g') {
                self.goto_line_cancel();
                return;
            }
            if key.code == KeyCode::Enter {
                self.goto_line_confirm();
                return;
            }
            if key.code == KeyCode::Backspace {
                self.goto_line_backspace();
                return;
            }
            if let Some(c) = key.char_value()
                && c.is_ascii_digit()
            {
                self.goto_line_digit(c);
                return;
            }
            // Other keys: swallow.
            return;
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

/// Pure scroll-anchor math for a buffer reload (issue 04).
///
/// `old_top` is the scroll top (first visible line, 0-based) before the
/// reload; `new_total` is the line count after the reload. The reader's line
/// anchor is preserved when it still exists (`old_top < new_total`); when the
/// anchor line has vanished (the file shrank past it), the view clamps to the
/// last line so the reader snaps to the end of the now-shorter file.
///
/// Extracted as a pure function so it is testable without notify / IO.
pub fn reload_anchor(old_top: usize, new_total: usize) -> usize {
    if new_total == 0 {
        return 0;
    }
    old_top.min(new_total - 1)
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
        assert_eq!(store.picker_count().0, 42);

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
        assert_eq!(store.picker_selected(), 41);

        // C-p goes through the same wrap-decrement path as Up.
        store.key_event(key("C-p"));
        assert_eq!(store.picker_selected(), 40);

        // RET runs the candidate at the selected index (40: reload-buffer).
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
        assert_eq!(store.picker_count().0, 42);

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
        // The scratch buffer is editable; file buffers are read-only (issue 03).
        store.open_scratch();
        assert!(store.insert_text("demo text\n"));
        assert!(store.buffer_text().contains("demo text"));
    }

    // ── issue 03 store-level tests (finding 9) ─────────────────────────

    fn store_with_lines(n_lines: usize) -> (AppStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut content = String::new();
        for i in 0..n_lines {
            content.push_str(&format!("line{}\n", i));
        }
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/big.rs"), &content).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/big.rs");
        s.set_viewport_lines(10);
        (s, dir)
    }

    #[test]
    fn scroll_line_down_increments_top() {
        let (mut s, _dir) = store_with_lines(100);
        assert_eq!(s.scroll_top(), 0);
        s.scroll_line_down();
        assert_eq!(s.scroll_top(), 1);
        s.scroll_line_down();
        assert_eq!(s.scroll_top(), 2);
    }

    #[test]
    fn scroll_line_up_saturates_at_zero() {
        let (mut s, _dir) = store_with_lines(100);
        s.scroll_line_up();
        assert_eq!(s.scroll_top(), 0, "cannot scroll above top");
    }

    #[test]
    fn scroll_page_down_advances_by_viewport() {
        let (mut s, _dir) = store_with_lines(100);
        s.scroll_page_down();
        assert_eq!(s.scroll_top(), 10);
        s.scroll_page_down();
        assert_eq!(s.scroll_top(), 20);
    }

    #[test]
    fn scroll_page_up_saturates_at_zero() {
        let (mut s, _dir) = store_with_lines(100);
        s.scroll_page_up();
        assert_eq!(s.scroll_top(), 0);
    }

    #[test]
    fn scroll_half_page_down_advances_by_half_viewport() {
        let (mut s, _dir) = store_with_lines(100);
        s.scroll_half_page_down();
        assert_eq!(s.scroll_top(), 5);
        s.scroll_half_page_down();
        assert_eq!(s.scroll_top(), 10);
    }

    #[test]
    fn scroll_half_page_up_saturates_at_zero() {
        let (mut s, _dir) = store_with_lines(100);
        s.scroll_half_page_up();
        assert_eq!(s.scroll_top(), 0);
    }

    #[test]
    fn scroll_to_bottom_clamps_to_last_visible_line() {
        let (mut s, _dir) = store_with_lines(100);
        let total = s.buffers.current_buffer().unwrap().line_count();
        s.scroll_to_bottom();
        assert_eq!(s.scroll_top(), total - 10, "top = total - viewport");
    }

    #[test]
    fn scroll_top_resets_to_zero() {
        let (mut s, _dir) = store_with_lines(100);
        s.scroll_page_down();
        s.scroll_to_top();
        assert_eq!(s.scroll_top(), 0);
    }

    #[test]
    fn scroll_preserved_across_buffer_switch() {
        let dir = tempfile::tempdir().unwrap();
        let mut c1 = String::new();
        let mut c2 = String::new();
        for i in 0..100 {
            c1.push_str(&format!("a{}\n", i));
            c2.push_str(&format!("b{}\n", i));
        }
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/a.rs"), &c1).unwrap();
        std::fs::write(dir.path().join("src/b.rs"), &c2).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.set_viewport_lines(10);
        s.open_path("src/a.rs");
        s.scroll_page_down();
        assert_eq!(s.scroll_top(), 10);
        s.open_path("src/b.rs");
        assert_eq!(s.scroll_top(), 0);
        s.open_path("src/a.rs");
        assert_eq!(s.scroll_top(), 10, "scroll preserved on return");
    }

    #[test]
    fn isearch_forward_incremental_and_count() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/t.rs"),
            "foo world\nfoo there\nfoo again\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        assert!(s.isearch_active());
        s.isearch_query_char('f');
        assert_eq!(s.isearch_match_count(), 3);
        s.isearch_query_char('o');
        assert_eq!(s.isearch_match_count(), 3);
        s.isearch_query_char('o');
        assert_eq!(s.isearch_match_count(), 3);
    }

    #[test]
    fn isearch_next_prev_wrap() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/t.rs"),
            "aaa\nbbb\naaa\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('a');
        let total = s.isearch_match_count();
        assert!(total >= 3);
        let idx = s.isearch_match_index();
        for _ in 0..total {
            s.isearch_next();
        }
        assert_eq!(s.isearch_match_index(), idx, "next wraps to start");
        s.isearch_prev();
        assert_eq!(s.isearch_match_index(), total, "prev wraps to end");
    }

    #[test]
    fn isearch_cancel_restores_position() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/t.rs"),
            "alpha\nbeta\ngamma\ndelta\nomega\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.set_viewport_lines(10);
        s.scroll_line_down();
        s.scroll_line_down();
        let pre_line = s.scroll_top();
        assert_eq!(pre_line, 2);
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('o'); // only in "omega" (line 4)
        assert_ne!(s.scroll_top(), pre_line, "search must move the view");
        s.isearch_cancel();
        assert!(!s.isearch_active());
        assert_eq!(s.scroll_top(), pre_line, "cancel must restore position");
    }

    #[test]
    fn isearch_multibyte_no_panic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "é\né\né\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('\u{e9}'); // é
        assert_eq!(s.isearch_match_count(), 3);
    }

    #[test]
    fn goto_line_confirm_jumps_to_line() {
        let (mut s, _dir) = store_with_lines(100);
        s.goto_line_start();
        assert!(s.goto_line_active());
        s.goto_line_digit('5');
        s.goto_line_digit('0');
        assert_eq!(s.goto_line_input(), "50");
        s.goto_line_confirm();
        assert!(!s.goto_line_active());
        assert_eq!(s.scroll_top(), 50);
    }

    #[test]
    fn goto_line_out_of_range_rejects() {
        let (mut s, _dir) = store_with_lines(100);
        s.goto_line_start();
        s.goto_line_digit('9');
        s.goto_line_digit('9');
        s.goto_line_digit('9');
        s.goto_line_confirm();
        assert!(!s.goto_line_active());
        assert!(s.message.contains("out of range"), "msg: {}", s.message);
    }

    #[test]
    fn goto_line_cancel() {
        let (mut s, _dir) = store_with_lines(100);
        s.scroll_line_down();
        let pre = s.scroll_top();
        s.goto_line_start();
        s.goto_line_digit('5');
        s.goto_line_cancel();
        assert!(!s.goto_line_active());
        assert_eq!(s.scroll_top(), pre, "cancel must not change scroll");
    }

    #[test]
    fn reopen_invalidates_stale_buffer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "fn old() {}\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        assert!(s.buffer_text().contains("old"));
        // Modify the file on disk; ensure mtime differs.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(dir.path().join("src/t.rs"), "fn new() {}\n").unwrap();
        s.open_path("src/t.rs");
        assert!(
            s.buffer_text().contains("new"),
            "reopen must reload changed file"
        );
    }

    // ── issue 04 store-level tests (no notify needed) ─────────────────

    fn change(paths: Vec<std::path::PathBuf>) -> ProjectChange {
        let n = paths.len();
        ProjectChange {
            seq: 1,
            paths,
            kinds: vec![crate::app::events::ChangeKind::Modify; n],
        }
    }

    #[test]
    fn reload_anchor_preserves_line_when_it_exists() {
        // Gains lines below: the anchor line still exists → keep it.
        assert_eq!(reload_anchor(50, 100), 50);
        // Gains lines above: the anchor line still exists (by number) → keep.
        assert_eq!(reload_anchor(5, 105), 5);
        // Anchor exactly on the last line.
        assert_eq!(reload_anchor(19, 20), 19);
    }

    #[test]
    fn reload_anchor_clamps_when_anchor_vanished() {
        // File shrank past the anchor: clamp to the last line.
        assert_eq!(reload_anchor(50, 20), 19);
        assert_eq!(reload_anchor(100, 1), 0);
    }

    #[test]
    fn reload_anchor_empty_file() {
        assert_eq!(reload_anchor(50, 0), 0);
        assert_eq!(reload_anchor(0, 0), 0);
    }

    #[test]
    fn auto_reload_defaults_on_and_watcher_inactive_until_started() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        assert!(s.auto_reload, "auto_reload must default to on");
        assert!(!s.watcher_suspended());
        assert_eq!(s.watcher_count(), 0, "no watcher until started");
    }

    #[test]
    fn toggle_watcher_flips_suspend_state() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut s = store(dir.path());
        s.toggle_watcher();
        assert!(s.watcher_suspended(), "first toggle suspends");
        s.toggle_watcher();
        assert!(!s.watcher_suspended(), "second toggle resumes");
    }

    #[test]
    fn non_edited_buffer_auto_reloads_on_change() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let path = dir.path().join("src/t.rs");
        std::fs::write(&path, "fn old() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_path("src/t.rs");
        let key = s.buffers.current().unwrap().to_string();
        // A plain read-only file buffer: not locally owned.
        assert!(!s.buffers.get(&key).unwrap().is_locally_owned());

        std::fs::write(&path, "fn fresh() {}\n").unwrap();
        s.apply_project_change(&change(vec![path]));
        assert!(
            s.buffer_text().contains("fresh"),
            "auto-reload must re-read content"
        );
        assert!(
            !s.buffers.get(&key).unwrap().changed_on_disk,
            "no conflict marker for a clean buffer"
        );
    }

    #[test]
    fn conflict_flag_transitions_on_locally_modified_buffer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let path = dir.path().join("src/t.rs");
        std::fs::write(&path, "fn old() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_path("src/t.rs");
        let key = s.buffers.current().unwrap().to_string();
        // Simulate a local edit (the light-editing flag path).
        s.mark_locally_modified(&key);
        assert!(s.buffers.get(&key).unwrap().locally_modified);

        // A disk change arrives while locally owned → NO auto-reload; marker set.
        s.apply_project_change(&change(vec![path.clone()]));
        assert!(
            s.buffers.get(&key).unwrap().changed_on_disk,
            "conflict marker must be set"
        );
        assert!(s.current_buffer_changed_on_disk());
        assert!(
            s.buffer_text().contains("old"),
            "no auto-reload while conflicted"
        );

        // Change the disk, then `g` (force reload) supersedes the conflict.
        std::fs::write(&path, "fn new() {}\n").unwrap();
        s.reload_current_buffer();
        assert!(
            !s.buffers.get(&key).unwrap().changed_on_disk,
            "marker cleared after g"
        );
        assert!(
            !s.buffers.get(&key).unwrap().locally_modified,
            "local flag cleared after g"
        );
        assert!(
            s.buffer_text().contains("new"),
            "g re-read the new content"
        );
    }

    #[test]
    fn reload_preserves_scroll_anchor_when_lines_added() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let path = dir.path().join("src/big.txt");
        let base: String = (0..100)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&path, &base).unwrap();
        let base_dir = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base_dir.path().to_path_buf());
        s.set_viewport_lines(10);
        s.open_path("src/big.txt");
        s.scroll_to_bottom();
        let n_before = s.buffers.current_buffer().unwrap().line_count();
        let top_before = s.scroll_top();
        assert_eq!(top_before, n_before - 10, "scrolled to last visible page");

        // The file grows well past the anchor; the anchor line still exists.
        let grown: String = (0..200)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&path, &grown).unwrap();
        s.apply_project_change(&change(vec![path]));
        assert_eq!(s.scroll_top(), top_before, "anchor line preserved");
        assert!(
            s.buffers.current_buffer().unwrap().line_count() > top_before,
            "file grew past the anchor"
        );
    }

    #[test]
    fn reload_clamps_scroll_anchor_when_file_shrinks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let path = dir.path().join("src/big.txt");
        let base: String = (0..100)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&path, &base).unwrap();
        let base_dir = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base_dir.path().to_path_buf());
        s.set_viewport_lines(10);
        s.open_path("src/big.txt");
        s.scroll_to_bottom();
        let n_before = s.buffers.current_buffer().unwrap().line_count();
        let top_before = s.scroll_top();
        assert_eq!(top_before, n_before - 10, "scrolled to last visible page");

        // The file shrinks to 20 content lines (well below the anchor).
        let shrunken: String = (0..20)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&path, &shrunken).unwrap();
        s.apply_project_change(&change(vec![path]));
        let n_after = s.buffers.current_buffer().unwrap().line_count();
        assert!(
            n_after <= top_before,
            "anchor line must vanish: n_after={n_after}, top_before={top_before}"
        );
        assert_eq!(s.scroll_top(), n_after - 1, "clamped to last line");
    }
}


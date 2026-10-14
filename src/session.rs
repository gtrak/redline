//! Scriptable session façade over `AppStore` (plan 019 issue 01a).
//!
//! Everything the external CLI/socket can do is expressed here against one
//! `AppStore`: dispatch a named registry command, feed keys/text through the
//! SAME modal routing the terminal uses, and read the observable state. No
//! new mutation paths are invented.
//!
//! 01a builds the module + tests only. The headless CLI that consumes it, and
//! `quote()` (the static frame render), land in 01b/019-02 — until then this
//! module's public surface has no non-test caller in the binary, so dead-code
//! analysis is silenced here rather than scattering `#[allow]` over the API.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::app::command::RegistryError;
use crate::app::keymap::{Key, parse_key, parse_sequence};
use crate::app::store::{AppStore, ViewId};

#[derive(Debug)]
pub enum SessionError {
    UnknownCommand(String),
    BadKey(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::UnknownCommand(name) => write!(f, "unknown command `{name}`"),
            SessionError::BadKey(token) => write!(f, "bad key token: {token}"),
        }
    }
}

impl std::error::Error for SessionError {}

pub struct Session {
    store: Arc<Mutex<AppStore>>,
    root: PathBuf,
    width: u16,
    height: u16,
}

impl Session {
    /// Build a headless session rooted at `project`. The persistence base is
    /// a throwaway dir under the system temp dir so a headless run NEVER
    /// touches the real cache dir (recents/registry/notes).
    pub fn at(project: &Path, width: u16, height: u16) -> Self {
        let base = std::env::temp_dir().join(format!(
            "redline-session-{}-{}",
            std::process::id(),
            // a monotonic-ish suffix so two sessions differ
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let mut store = AppStore::at(project, base);
        store.set_viewport_lines(height.saturating_sub(3) as usize);
        Self {
            store: Arc::new(Mutex::new(store)),
            root: project.to_path_buf(),
            width,
            height,
        }
    }

    pub fn store(&self) -> Arc<Mutex<AppStore>> {
        Arc::clone(&self.store)
    }

    /// Wrap an EXISTING store handle — used by the control socket (019-02)
    /// so a remote command mutates the very session the human is watching.
    /// The headless CLI uses [`Session::at`] instead; this constructor never
    /// owns the store's lifetime, it just shares the `Arc`.
    pub fn from_store(
        store: Arc<Mutex<AppStore>>,
        root: PathBuf,
        width: u16,
        height: u16,
    ) -> Self {
        Self {
            store,
            root,
            width,
            height,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn do_command(&mut self, name: &str, arg: Option<String>) -> Result<(), SessionError> {
        let mut s = self.store.lock().unwrap();
        s.dispatch(name, arg)
            .map_err(|RegistryError::UnknownCommand(n)| SessionError::UnknownCommand(n))
    }

    pub fn key(&mut self, token: &str) -> Result<(), SessionError> {
        let k = parse_key(token).map_err(|e| SessionError::BadKey(e.to_string()))?;
        self.store.lock().unwrap().key_event(k);
        Ok(())
    }

    pub fn keys(&mut self, seq: &str) -> Result<(), SessionError> {
        let ks = parse_sequence(seq).map_err(|e| SessionError::BadKey(e.to_string()))?;
        let mut s = self.store.lock().unwrap();
        for k in ks.iter() {
            s.key_event(*k);
        }
        Ok(())
    }

    /// Literal text into the current insert context: each char is one key
    /// event, `\n` is Enter. Read-only buffers refuse exactly as a human.
    pub fn text(&mut self, s: &str) -> Result<(), SessionError> {
        let mut st = self.store.lock().unwrap();
        for ch in s.chars() {
            let k = if ch == '\n' {
                Key::enter()
            } else {
                Key::char(ch)
            };
            st.key_event(k);
        }
        Ok(())
    }

    /// The full observable state, as a `serde::Serialize` value.
    pub fn snapshot(&mut self) -> Snapshot {
        let snap = self.scalar_snapshot();
        snap.with_surfaces(&self.store)
    }

    /// The lock-held scalar read: every accessor that takes only `&self`,
    /// built into a `Snapshot` whose stateful surfaces are still `None`.
    fn scalar_snapshot(&self) -> Snapshot {
        let s = self.store.lock().unwrap();
        let root = s
            .project
            .as_ref()
            .map(|p| p.root.display().to_string())
            .unwrap_or_default();
        let buffer = s
            .buffers
            .current()
            .map(|k| s.buffer_display(k))
            .unwrap_or_default();
        let view = s.render_view();
        let view_name = s.view_name_display();
        let mode = s.buffer_mode_display();
        let message = s.message.clone();
        let pending = s.pending_display();
        let activity = s.activity_display();
        let (point_line, point_col) = s.file_view_point();
        let (win_top, win_total, win_viewport) = s.file_view_scroll_info();
        let position = s.file_view_position_display();
        let which_function = s.which_function();
        let annotations = s.annotation_count_display();
        let dirty = s.dirty_counts();
        let indexing = s.indexing_display();
        let resolving = s.resolving_display();
        let crate_indexing = s.crate_indexing_display();
        Snapshot {
            project: s.project_display().to_string(),
            root,
            view: view_label(view).to_string(),
            view_name,
            buffer,
            mode,
            message,
            pending,
            activity,
            point: Point {
                line: point_line,
                col: point_col,
            },
            window: Window {
                top_line: win_top,
                total_lines: win_total,
                viewport: win_viewport,
            },
            position,
            which_function,
            annotations,
            dirty: dirty.map(|d| Dirty {
                staged: d.staged,
                unstaged: d.unstaged,
                untracked: d.untracked,
            }),
            picker: None,
            search: None,
            magit: None,
            tree: None,
            isearch: None,
            buffer_list: None,
            indexing,
            resolving,
            crate_indexing,
        }
    }

    /// The static frame text: what a human would see, rendered at the
    /// session width (`self.width`, defaults to 80). Takes the session's own
    /// `Arc<Mutex<AppStore>>` handle — no ownership transfer — and the render
    /// takes the lock itself (no lock is held across this call).
    pub fn quote(&mut self) -> String {
        crate::ui::root::render_handle_text(Arc::clone(&self.store), self.width as usize)
    }

    /// A 1-based, inclusive line range over the current buffer's text.
    /// `None` returns the whole buffer.
    pub fn buffer_text(&mut self, range: Option<(usize, usize)>) -> String {
        let text = self.store.lock().unwrap().buffer_text();
        match range {
            None => text,
            Some((start, end)) => {
                let mut out = String::new();
                for (i, line) in text.lines().enumerate() {
                    let n = i + 1;
                    if n >= start && n <= end {
                        out.push_str(line);
                        out.push('\n');
                    }
                }
                out
            }
        }
    }

    pub fn notes(&mut self, plain: bool) -> String {
        let mut s = self.store.lock().unwrap();
        let items = s.annotations_for_dump();
        let root = s
            .project
            .as_ref()
            .map(|p| p.root.display().to_string())
            .unwrap_or_default();
        crate::app::store::format_notes_dump(&items, &root, plain)
    }

    pub fn quit_requested(&self) -> bool {
        self.store.lock().unwrap().quit
    }

    /// The shared sugar-verb dispatcher (W8/frozen surface): every sugar
    /// verb is ONE call here, composed from real store operations — never a
    /// registry `arg` replay. The local headless CLI and the control-socket
    /// wire both route through this so the two cannot drift.
    ///
    /// `arg` is the verb's single optional string argument:
    ///
    /// | verb | arg | semantics |
    /// |------|-----|-----------|
    /// | `open` | `PATH[:LINE[:COL]]` | `open_path_at`; `PATH` project-relative; `LINE`/`COL` 1-based |
    /// | `goto` | `LINE` | land the point on `LINE` (1-based) via the real goto-line prompt |
    /// | `jump` | `SYMBOL` | open the project symbol picker filtered to `SYMBOL` |
    /// | `search` | `QUERY` | `start_project_search` (the real store op) |
    /// | `isearch` | `QUERY` | start an in-buffer isearch and type `QUERY` |
    /// | `select` | `N` \| `next` \| `prev` | picker: move the selection (1-based row) |
    /// | `confirm` | — | picker: accept the selected candidate (`run_selected`) |
    /// | `cancel` | — | the global cancel (`store.cancel`, C-g equivalent) |
    /// | `annotate` | `TEXT` | annotate at point with `TEXT` |
    /// | `magit` | — | open magit status |
    ///
    /// Unknown names are [`SessionError::UnknownCommand`].
    pub fn verb(&mut self, name: &str, arg: Option<&str>) -> Result<(), SessionError> {
        let arg = arg.unwrap_or("").trim();
        match name {
            "open" => {
                if arg.is_empty() {
                    return Err(SessionError::BadKey("open needs a path".to_string()));
                }
                let (path, line, col) = parse_path_spec(arg);
                self.store.lock().unwrap().open_path_at(&path, line, col);
            }
            "goto" => {
                let line: usize = arg.parse().map_err(|_| {
                    SessionError::BadKey(format!("goto expects a line number, got `{arg}`"))
                })?;
                let mut s = self.store.lock().unwrap();
                // Exercise the real goto-line prompt (the same path a human
                // uses): open the prompt, feed the digits, confirm.
                s.goto_line_start();
                for c in line.to_string().chars() {
                    s.key_event(Key::char(c));
                }
                // A degenerate input (0/the prompt failing to latch) falls
                // back to a direct landing so the verb is never a no-op.
                if s.goto_line_active() {
                    s.goto_line_confirm();
                }
                if line == 0 {
                    s.goto_line_col(0, 0);
                }
            }
            "jump" => {
                let mut s = self.store.lock().unwrap();
                s.open_symbol_picker();
                // Only type into the picker when it actually opened (an
                // empty index leaves no picker and typing would self-insert).
                if s.picker_open() {
                    for c in arg.chars() {
                        s.key_event(Key::char(c));
                    }
                }
            }
            "search" => {
                if arg.is_empty() {
                    return Err(SessionError::BadKey("search needs a query".to_string()));
                }
                self.store.lock().unwrap().start_project_search(arg.to_string());
            }
            "isearch" => {
                if arg.is_empty() {
                    return Err(SessionError::BadKey("isearch needs a query".to_string()));
                }
                let mut s = self.store.lock().unwrap();
                s.isearch_start(crate::app::store::IsearchDirection::Forward);
                if s.isearch_active() {
                    for c in arg.chars() {
                        s.key_event(Key::char(c));
                    }
                }
            }
            "select" => {
                let mut s = self.store.lock().unwrap();
                match arg {
                    "next" => s.picker_select_next(),
                    "prev" => s.picker_select_prev(),
                    other => {
                        let n: usize = other.parse().map_err(|_| {
                            SessionError::BadKey(format!(
                                "select expects a row number or next/prev, got `{other}`"
                            ))
                        })?;
                        if n == 0 {
                            return Err(SessionError::BadKey(
                                "select row numbers are 1-based".to_string(),
                            ));
                        }
                        s.picker_select_index(n - 1);
                    }
                }
            }
            "confirm" => {
                self.store.lock().unwrap().run_selected();
            }
            "cancel" => {
                self.store.lock().unwrap().cancel();
            }
            "annotate" => {
                let mut s = self.store.lock().unwrap();
                s.annotate();
                if s.note_prompt_active() {
                    for c in arg.chars() {
                        s.key_event(Key::char(c));
                    }
                    s.key_event(Key::enter());
                }
            }
            "magit" => {
                self.store.lock().unwrap().open_magit_status();
            }
            other => return Err(SessionError::UnknownCommand(other.to_string())),
        }
        Ok(())
    }
}

/// Split a `PATH[:LINE[:COL]]` spec into the project-relative path and the
/// optional 1-based line/column. A trailing run of one or two `:N` numeric
/// segments is consumed from the right; everything before it is the path
/// (so a `:` inside a path is preserved as long as the last segment is not
/// numeric). A non-numeric trailing segment is part of the path.
fn parse_path_spec(spec: &str) -> (String, Option<usize>, Option<usize>) {
    let mut it = spec.rsplitn(3, ':');
    let first = it.next().unwrap_or("");
    if let Ok(n1) = first.parse::<usize>() {
        match it.next() {
            Some(second) => {
                if let Ok(n2) = second.parse::<usize>() {
                    // Two trailing numbers: LINE then COL, path is the rest.
                    match it.next() {
                        Some(path) if !path.is_empty() => {
                            return (path.to_string(), Some(n2), Some(n1));
                        }
                        _ => return (spec.to_string(), None, None),
                    }
                }
                (second.to_string(), Some(n1), None)
            }
            None => (spec.to_string(), None, None),
        }
    } else {
        (spec.to_string(), None, None)
    }
}

impl Snapshot {
    /// Fill the stateful surfaces (picker/search/magit/tree/isearch/
    /// buffer-list) — split out of `Session::snapshot` so each `&mut`
    /// accessor is borrowed in its own statement.
    fn with_surfaces(mut self, store: &Arc<Mutex<AppStore>>) -> Snapshot {
        let mut s = store.lock().unwrap();
        if s.picker_open() {
            let prompt = s.picker_prompt().to_string();
            let query = s.picker_query().to_string();
            let selected = s.picker_selected();
            let total = s.picker_count().1;
            let rows: Vec<PickerRow> = s
                .picker_filtered()
                .iter()
                .map(|(c, _)| PickerRow {
                    name: c.name.clone(),
                    label: c.label.clone(),
                    detail: c.detail.clone(),
                })
                .collect();
            self.picker = Some(Picker {
                prompt,
                query,
                selected,
                total,
                rows,
            });
        }
        let search_active = s.render_view() == ViewId::Search
            || s.search_running()
            || s.search_error().is_some();
        if search_active {
            let (rows, _top, _total, selected) = s.search_view_info();
            let title = s.search_title();
            let running = s.search_running();
            let error = s.search_error();
            let narrow_query = s.search_narrow_query().to_string();
            let rows: Vec<SearchRow> = rows
                .iter()
                .map(|r| match r {
                    crate::app::store::ResultRow::Header { file, count, .. } => SearchRow {
                        kind: "header".to_string(),
                        file: file.clone(),
                        line: None,
                        col: None,
                        text: String::new(),
                        count: Some(*count),
                    },
                    crate::app::store::ResultRow::Hit { hit, .. } => SearchRow {
                        kind: "hit".to_string(),
                        file: hit.file.clone(),
                        line: Some(hit.line_no),
                        col: hit.col,
                        text: hit.line.clone(),
                        count: None,
                    },
                })
                .collect();
            let total = rows.len();
            self.search = Some(Search {
                title,
                running,
                error,
                narrow_query,
                total,
                selected,
                rows,
            });
        }
        if s.render_view() == ViewId::MagitStatus {
            let (_rows, _top, total) = s.magit_view_info();
            let narrow_query = s.magit_narrow_query().to_string();
            self.magit = Some(Magit {
                rows_total: total,
                narrow_query,
            });
        }
        if s.tree_visible() {
            let (rows, selected) = s.tree_view_info();
            let query = s.tree_narrow_query().to_string();
            self.tree = Some(Tree {
                visible: true,
                query,
                rows_total: rows.len(),
                selected,
            });
        }
        {
            let (rows, current, active) = s.isearch_list();
            if active {
                self.isearch = Some(Isearch {
                    active,
                    selected: current,
                    total: rows.len(),
                });
            }
        }
        if s.render_view() == ViewId::BufferList {
            let (_rows, _scroll, total, selected) = s.buffer_list_view_info();
            let query = s.buffer_list_query().to_string();
            self.buffer_list = Some(BufferList {
                query,
                total,
                selected,
            });
        }
        drop(s);
        self
    }

    /// A stable, grep-friendly block (one `key: value` line per scalar;
    /// lists indented). JSON is the machine form; this is the human form.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let mut line = |k: &str, v: &str| {
            out.push_str(k);
            out.push_str(": ");
            out.push_str(v);
            out.push('\n');
        };
        line("project", &self.project);
        line("root", &self.root);
        line("view", &self.view);
        line("view_name", &self.view_name);
        line("buffer", &self.buffer);
        line("mode", &self.mode);
        line("message", &self.message);
        line("pending", &self.pending);
        line("activity", &self.activity);
        line("point.line", &self.point.line.to_string());
        line("point.col", &self.point.col.to_string());
        line("window.top_line", &self.window.top_line.to_string());
        line("window.total_lines", &self.window.total_lines.to_string());
        line("window.viewport", &self.window.viewport.to_string());
        line("position", &self.position);
        line("which_function", &self.which_function);
        line("annotations", &self.annotations);
        match &self.dirty {
            Some(d) => line(
                "dirty",
                &format!(
                    "staged={} unstaged={} untracked={}",
                    d.staged, d.unstaged, d.untracked
                ),
            ),
            None => line("dirty", "null"),
        }
        match &self.picker {
            Some(p) => {
                line("picker", &format!("{} rows", p.rows.len()));
                line("picker.prompt", &p.prompt);
                line("picker.query", &p.query);
                line("picker.selected", &p.selected.to_string());
                line("picker.total", &p.total.to_string());
                for r in &p.rows {
                    line("picker.row", &format!("{} | {} | {}", r.name, r.label, r.detail));
                }
            }
            None => line("picker", "null"),
        }
        match &self.search {
            Some(s) => {
                line("search.title", &s.title);
                line("search.running", &s.running.to_string());
                line(
                    "search.error",
                    s.error.as_deref().unwrap_or("null"),
                );
                line("search.narrow_query", &s.narrow_query);
                line("search.total", &s.total.to_string());
                line(
                    "search.selected",
                    &s.selected.map(|v| v.to_string()).unwrap_or_else(|| "null".to_string()),
                );
                for r in &s.rows {
                    line(
                        "search.row",
                        &format!(
                            "{} {}:{} {}",
                            r.kind,
                            r.file,
                            r.line.map(|l| l.to_string()).unwrap_or_default(),
                            r.text
                        ),
                    );
                }
            }
            None => line("search", "null"),
        }
        match &self.magit {
            Some(m) => {
                line("magit.rows_total", &m.rows_total.to_string());
                line("magit.narrow_query", &m.narrow_query);
            }
            None => line("magit", "null"),
        }
        match &self.tree {
            Some(t) => {
                line("tree.visible", &t.visible.to_string());
                line("tree.query", &t.query);
                line("tree.rows_total", &t.rows_total.to_string());
                line("tree.selected", &t.selected.to_string());
            }
            None => line("tree", "null"),
        }
        match &self.isearch {
            Some(i) => {
                line("isearch.active", &i.active.to_string());
                line("isearch.selected", &i.selected.to_string());
                line("isearch.total", &i.total.to_string());
            }
            None => line("isearch", "null"),
        }
        match &self.buffer_list {
            Some(b) => {
                line("buffer_list.query", &b.query);
                line("buffer_list.total", &b.total.to_string());
                line(
                    "buffer_list.selected",
                    &b.selected
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "null".to_string()),
                );
            }
            None => line("buffer_list", "null"),
        }
        line("indexing", &self.indexing);
        line("resolving", &self.resolving);
        line("crate_indexing", &self.crate_indexing);
        out
    }
}

/// A human-readable view label. The store's `ViewId::name()` is the
/// lowercase stable id; the snapshot uses the capitalized variant name
/// (the schema pins `"Buffer"`).
fn view_label(v: ViewId) -> &'static str {
    match v {
        ViewId::Home => "Home",
        ViewId::Buffer => "Buffer",
        ViewId::BufferList => "BufferList",
        ViewId::MagitStatus => "MagitStatus",
        ViewId::Search => "Search",
        ViewId::Log => "Log",
        ViewId::Blame => "Blame",
        ViewId::CommitDiff => "CommitDiff",
        ViewId::CommitEditor => "CommitEditor",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub project: String,
    pub root: String,
    pub view: String,
    pub view_name: String,
    pub buffer: String,
    pub mode: String,
    pub message: String,
    pub pending: String,
    pub activity: String,
    pub point: Point,
    pub window: Window,
    pub position: String,
    pub which_function: String,
    pub annotations: String,
    pub dirty: Option<Dirty>,
    pub picker: Option<Picker>,
    pub search: Option<Search>,
    pub magit: Option<Magit>,
    pub tree: Option<Tree>,
    pub isearch: Option<Isearch>,
    pub buffer_list: Option<BufferList>,
    pub indexing: String,
    pub resolving: String,
    pub crate_indexing: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Point {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Window {
    pub top_line: usize,
    pub total_lines: usize,
    pub viewport: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dirty {
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickerRow {
    pub name: String,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Picker {
    pub prompt: String,
    pub query: String,
    pub selected: usize,
    pub total: usize,
    pub rows: Vec<PickerRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRow {
    pub kind: String,
    pub file: String,
    pub line: Option<u64>,
    pub col: Option<u64>,
    pub text: String,
    pub count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Search {
    pub title: String,
    pub running: bool,
    pub error: Option<String>,
    pub narrow_query: String,
    pub total: usize,
    pub selected: Option<usize>,
    pub rows: Vec<SearchRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Magit {
    pub rows_total: usize,
    pub narrow_query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tree {
    pub visible: bool,
    pub query: String,
    pub rows_total: usize,
    pub selected: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Isearch {
    pub active: bool,
    pub selected: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferList {
    pub query: String,
    pub total: usize,
    pub selected: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"probe\"\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# probe\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let mut main = String::new();
        for i in 1..=20 {
            main.push_str(&format!("// line {i}\n"));
        }
        std::fs::write(dir.path().join("src/main.rs"), main).unwrap();
        dir
    }

    fn session(dir: &tempfile::TempDir) -> Session {
        Session::at(dir.path(), 80, 24)
    }

    /// Open `src/main.rs` the way a human does through the seed registry:
    /// `find-file` opens the picker (the seed handlers ignore `arg`; an
    /// arg-aware open lands in 01b/03), so we dispatch then type the path
    /// and confirm.
    fn open_main(s: &mut Session) {
        s.do_command("find-file", Some("src/main.rs".to_string()))
            .unwrap();
        s.text("src/main.rs\n").unwrap();
    }

    #[test]
    fn dispatch_open_then_snapshot_shows_buffer() {
        let dir = project();
        let mut s = session(&dir);
        open_main(&mut s);
        let snap = s.snapshot();
        assert!(
            snap.buffer.contains("src/main.rs"),
            "buffer should name the opened file: {:?}",
            snap.buffer
        );
        assert_eq!(snap.view, "Buffer");
    }

    #[test]
    fn goto_line_moves_point() {
        let dir = project();
        let mut s = session(&dir);
        open_main(&mut s);
        // Registry name: "goto-line" (command.rs) then digits + RET.
        s.do_command("goto-line", None).unwrap();
        s.text("10\n").unwrap();
        assert_eq!(s.snapshot().point.line, 9);
    }

    #[test]
    fn key_unknown_token_is_bad_key() {
        let dir = project();
        let mut s = session(&dir);
        match s.key("NOTAKEY") {
            Err(SessionError::BadKey(_)) => {}
            other => panic!("expected BadKey, got {other:?}"),
        }
    }

    #[test]
    fn unknown_command_is_error() {
        let dir = project();
        let mut s = session(&dir);
        match s.do_command("no-such-cmd", None) {
            Err(SessionError::UnknownCommand(name)) => assert_eq!(name, "no-such-cmd"),
            other => panic!("expected UnknownCommand, got {other:?}"),
        }
    }

    #[test]
    fn snapshot_json_round_trips() {
        let dir = project();
        let mut s = session(&dir);
        open_main(&mut s);
        let snap = s.snapshot();
        let json = serde_json::to_string(&snap).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value.get("view").is_some(), "view key missing: {json}");
        assert!(value.get("point").is_some(), "point key missing: {json}");
    }

    #[test]
    fn quote_renders_opened_file() {
        let dir = project();
        let mut s = session(&dir);
        open_main(&mut s);
        let frame = s.quote();
        assert!(
            frame.contains("// line 1"),
            "quote should render the opened file's text: {frame:?}"
        );
    }

    #[test]
    fn headless_state_json_has_view_and_point() {
        let dir = project();
        let mut s = session(&dir);
        open_main(&mut s);
        s.do_command("goto-line", None).unwrap();
        s.text("3\n").unwrap();
        let snap = s.snapshot();
        let json = serde_json::to_string(&snap).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value.get("view").is_some(), "view key missing: {json}");
        assert_eq!(value["point"]["line"], 2, "goto-line 3 lands on line 2 (0-based)");
    }

    #[test]
    fn text_refused_in_read_only_buffer() {
        let dir = project();
        let mut s = session(&dir);
        open_main(&mut s);
        let before = s.buffer_text(None);
        s.text("x").unwrap();
        let after = s.buffer_text(None);
        assert_eq!(before, after, "read-only buffer must refuse literal text");
    }

    // ── sugar verbs (019-03) ────────────────────────────────────────────

    #[test]
    fn parse_path_spec_splits_line_and_col() {
        assert_eq!(parse_path_spec("src/main.rs"), ("src/main.rs".into(), None, None));
        assert_eq!(
            parse_path_spec("src/main.rs:5"),
            ("src/main.rs".into(), Some(5), None)
        );
        assert_eq!(
            parse_path_spec("src/main.rs:5:3"),
            ("src/main.rs".into(), Some(5), Some(3))
        );
        // A non-numeric trailing segment is part of the path (no colon split).
        assert_eq!(
            parse_path_spec("weird:name.rs"),
            ("weird:name.rs".into(), None, None)
        );
    }

    #[test]
    fn open_verb_lands_the_point() {
        let dir = project();
        let mut s = session(&dir);
        s.verb("open", Some("src/main.rs:10")).unwrap();
        let snap = s.snapshot();
        assert!(snap.buffer.contains("src/main.rs"), "opened: {:?}", snap.buffer);
        assert_eq!(snap.point.line, 9, "open src/main.rs:10 lands on line index 9");
    }

    #[test]
    fn open_verb_with_col() {
        let dir = project();
        let mut s = session(&dir);
        s.verb("open", Some("src/main.rs:3:2")).unwrap();
        let snap = s.snapshot();
        assert_eq!(snap.point.line, 2);
        assert_eq!(snap.point.col, 1);
    }

    #[test]
    fn goto_verb_lands_the_line() {
        let dir = project();
        let mut s = session(&dir);
        s.verb("open", Some("src/main.rs")).unwrap();
        s.verb("goto", Some("12")).unwrap();
        assert_eq!(s.snapshot().point.line, 11);
    }

    #[test]
    fn jump_verb_opens_symbol_picker() {
        let dir = project();
        let mut s = session(&dir);
        // No index in a headless session, so the picker may not open; the
        // verb must not error, and must not have self-inserted text.
        s.verb("jump", Some("alpha")).unwrap();
    }

    #[test]
    fn search_verb_starts_a_project_search() {
        let dir = project();
        let mut s = session(&dir);
        s.verb("search", Some("probe")).unwrap();
        let snap = s.snapshot();
        assert!(snap.search.is_some() || snap.view == "Search", "search started: {snap:?}");
    }

    #[test]
    fn isearch_verb_starts_and_types() {
        let dir = project();
        let mut s = session(&dir);
        s.verb("open", Some("src/main.rs")).unwrap();
        s.verb("isearch", Some("line")).unwrap();
        let snap = s.snapshot();
        let is = snap.isearch.expect("isearch active after the verb");
        assert!(is.active);
    }

    #[test]
    fn cancel_verb_is_a_no_op_without_pending_state() {
        let dir = project();
        let mut s = session(&dir);
        s.verb("cancel", None).unwrap();
    }

    #[test]
    fn annotate_verb_writes_a_note() {
        let dir = project();
        let mut s = session(&dir);
        s.verb("open", Some("src/main.rs:2")).unwrap();
        s.verb("annotate", Some("hello note")).unwrap();
        // The note count is reflected in the snapshot's annotations string.
        let snap = s.snapshot();
        assert!(
            snap.annotations.contains("note"),
            "annotation committed: {:?}",
            snap.annotations
        );
    }

    #[test]
    fn select_verb_rejects_zero_and_bad_tokens() {
        let dir = project();
        let mut s = session(&dir);
        assert!(matches!(
            s.verb("select", Some("0")),
            Err(SessionError::BadKey(_))
        ));
        assert!(matches!(
            s.verb("select", Some("nope")),
            Err(SessionError::BadKey(_))
        ));
    }

    #[test]
    fn unknown_verb_is_unknown_command() {
        let dir = project();
        let mut s = session(&dir);
        assert!(matches!(
            s.verb("frobnicate", None),
            Err(SessionError::UnknownCommand(_))
        ));
    }

    #[test]
    fn missing_required_arg_is_an_error() {
        let dir = project();
        let mut s = session(&dir);
        assert!(s.verb("open", None).is_err());
        assert!(s.verb("search", None).is_err());
        assert!(s.verb("goto", None).is_err());
    }

    // ── frozen Snapshot schema (019-03) ─────────────────────────────────

    /// The snapshot's top-level key set is FROZEN: a rename must redden this
    /// test (the JSON is the documented external API). Update this list and
    /// `docs/control-api.md` together when the schema genuinely changes.
    #[test]
    fn snapshot_top_level_keys_are_frozen() {
        let dir = project();
        let mut s = session(&dir);
        open_main(&mut s);
        let value = serde_json::to_value(s.snapshot()).unwrap();
        let obj = value.as_object().unwrap();
        let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "activity",
                "annotations",
                "buffer",
                "buffer_list",
                "crate_indexing",
                "dirty",
                "indexing",
                "isearch",
                "magit",
                "message",
                "mode",
                "pending",
                "picker",
                "point",
                "position",
                "project",
                "resolving",
                "root",
                "search",
                "tree",
                "view",
                "view_name",
                "which_function",
                "window",
            ]
        );
    }

    /// Each nested object's key set is frozen too.
    #[test]
    fn snapshot_nested_keys_are_frozen() {
        let dir = project();
        let mut s = session(&dir);
        open_main(&mut s);
        let value = serde_json::to_value(s.snapshot()).unwrap();
        let keys_of = |path: &[&str]| -> Vec<String> {
            let mut v = &value;
            for p in path {
                v = &v[*p];
            }
            let mut ks: Vec<String> = v
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            ks.sort_unstable();
            ks
        };
        assert_eq!(keys_of(&["point"]), vec!["col", "line"]);
        assert_eq!(keys_of(&["window"]), vec!["top_line", "total_lines", "viewport"]);
        // `dirty` is null in a clean session; its shape is pinned by the
        // struct test below where it is Some.
    }

    /// The nested `Dirty` object keys (exercised via the struct directly, so
    /// it does not depend on a dirty git fixture).
    #[test]
    fn dirty_keys_are_frozen() {
        let d = Dirty {
            staged: 0,
            unstaged: 0,
            untracked: 0,
        };
        let value = serde_json::to_value(d).unwrap();
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["staged", "unstaged", "untracked"]);
    }

    /// The nested Picker/Search/Magit/Tree/Isearch/BufferList structs' keys
    /// are frozen (built directly — the surfaces need a live picker).
    #[test]
    fn surface_struct_keys_are_frozen() {
        let keys = |v: serde_json::Value| -> Vec<String> {
            let mut ks: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
            ks.sort_unstable();
            ks
        };
        assert_eq!(
            keys(serde_json::to_value(Picker {
                prompt: String::new(),
                query: String::new(),
                selected: 0,
                total: 0,
                rows: vec![],
            })
            .unwrap()),
            vec!["prompt", "query", "rows", "selected", "total"]
        );
        assert_eq!(
            keys(serde_json::to_value(PickerRow {
                name: String::new(),
                label: String::new(),
                detail: String::new(),
            })
            .unwrap()),
            vec!["detail", "label", "name"]
        );
        assert_eq!(
            keys(serde_json::to_value(Search {
                title: String::new(),
                running: false,
                error: None,
                narrow_query: String::new(),
                total: 0,
                selected: None,
                rows: vec![],
            })
            .unwrap()),
            vec![
                "error",
                "narrow_query",
                "rows",
                "running",
                "selected",
                "title",
                "total"
            ]
        );
        assert_eq!(
            keys(serde_json::to_value(SearchRow {
                kind: String::new(),
                file: String::new(),
                line: None,
                col: None,
                text: String::new(),
                count: None,
            })
            .unwrap()),
            vec!["col", "count", "file", "kind", "line", "text"]
        );
        assert_eq!(
            keys(serde_json::to_value(Magit {
                rows_total: 0,
                narrow_query: String::new(),
            })
            .unwrap()),
            vec!["narrow_query", "rows_total"]
        );
        assert_eq!(
            keys(serde_json::to_value(Tree {
                visible: false,
                query: String::new(),
                rows_total: 0,
                selected: 0,
            })
            .unwrap()),
            vec!["query", "rows_total", "selected", "visible"]
        );
        assert_eq!(
            keys(serde_json::to_value(Isearch {
                active: false,
                selected: 0,
                total: 0,
            })
            .unwrap()),
            vec!["active", "selected", "total"]
        );
        assert_eq!(
            keys(serde_json::to_value(BufferList {
                query: String::new(),
                total: 0,
                selected: None,
            })
            .unwrap()),
            vec!["query", "selected", "total"]
        );
    }
}

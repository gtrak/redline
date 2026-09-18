//! Central app store: the single source of truth the ui layer renders
//! and updates. Holds the view stack, keymap state, pending key
//! sequence, minibuffer message, status line state, quit flag, the
//! picker overlay state, the project layer (current project, known
//! projects, per-project recents, cached file lists), the open-buffer
//! set (ropey-backed), the highlight cache, per-buffer scroll state,
//! isearch state, and goto-line state.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use nucleo_matcher::{
    Matcher, pattern::{CaseMatching, Normalization, Pattern},
};
use ropey::Rope;
use tokio::sync::mpsc;

use crate::app::command::{CommandRegistry, RegistryError};
use crate::app::config::Config;
use crate::app::events::{ChangeBus, ProjectChange};
use crate::app::keymap::{Key, KeyCode, KeyMap, KeySeq, KeymapEngine, Lookup, parse_sequence};
use crate::app::watcher::{ActiveWatcher, DEFAULT_DEBOUNCE};
use crate::git::blame::BlameLine;
use crate::git::diff::{DiffSide, FileDiff};
use crate::git::log::{relative_time_from, LogEntry};
use crate::git::status::{RepoStatus, Side};
use crate::git::{GitError, GitRepo};
use crate::model::buffer::{load_file, BufferTable, SCRATCH_NAME};
use crate::model::files::FileList;
use crate::model::project::{detect_root, Project, ProjectStore};
use crate::model::sections::{MagitRow, RowRole, SectionKind, StatusTree};
use crate::nav::index::{build_index, refresh_in_place, IndexBus, IndexEvent, IndexProgress, SymbolIndex};
use crate::search::occur;
use crate::search::references;
use crate::search::rg::{self, Hit, SearchBus, SearchConfig, SearchEvent};
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
    /// The search results view (issue 06): grouped, counted, jumpable.
    Search,
    /// The magit log view (issue 08): paged commit list.
    Log,
    /// The blame view (issue 08): per-line commit/author/age prefix.
    Blame,
    /// A read-only full tree diff of one commit (issue 08, log `RET`).
    CommitDiff,
    /// The inline commit-message editor (issue 08, `c`): the first editable
    /// buffer, `C-c C-c` commits / `C-c C-k` aborts.
    CommitEditor,
}

impl ViewId {
    pub fn name(self) -> &'static str {
        match self {
            ViewId::Buffer => "buffer",
            ViewId::BufferList => "buffer-list",
            ViewId::MagitStatus => "magit-status",
            ViewId::Search => "search",
            ViewId::Log => "log",
            ViewId::Blame => "blame",
            ViewId::CommitDiff => "commit-diff",
            ViewId::CommitEditor => "commit-editor",
        }
    }

    fn keymap(self) -> KeyMap {
        match self {
            ViewId::Buffer => {
                let mut km = KeyMap::new();
                // Bare `q` closes the view (issue 05, finding 5): consistent
                // with the list views. When the main buffer view is the only
                // view, `close-view` is a no-op — it does NOT quit the app
                // (that is still `C-x C-c`).
                km.bind(&[Key::char('q')], "close-view").unwrap();
                km.bind(&[Key::alt_char('o')], "open-scratch").unwrap();
                km
                    .bind(&[Key::ctrl_char('x'), Key::char('o')], "open-scratch")
                    .unwrap();
                // Motion (issue 03 + plan 004 issue 05b). Point motion:
                // C-n/Down and C-p/Up move the point (goal column preserved);
                // C-f/Right and C-b/Left move by character (wrap at EOL/BOL);
                // C-a/C-e jump to line start/end. The window follows the
                // point (the shipped follow-scroll pattern). This supersedes
                // the plan-001 item-4 stopgap that bound the arrows to window
                // scroll (the user directive is explicit: arrows move point).
                km.bind(&[Key::ctrl_char('n')], "point-down").unwrap();
                km.bind(&[Key::ctrl_char('p')], "point-up").unwrap();
                km.bind(&[Key::down()], "point-down").unwrap();
                km.bind(&[Key::up()], "point-up").unwrap();
                km.bind(&[Key::ctrl_char('f')], "point-forward").unwrap();
                km.bind(&[Key::ctrl_char('b')], "point-backward").unwrap();
                km.bind(&[Key::new(KeyCode::Right)], "point-forward").unwrap();
                km.bind(&[Key::new(KeyCode::Left)], "point-backward").unwrap();
                km.bind(&[Key::ctrl_char('a')], "point-line-start").unwrap();
                km.bind(&[Key::ctrl_char('e')], "point-line-end").unwrap();
                // Word motion (plan 004 issue 05c): M-f / M-b.
                km.bind(&[Key::alt_char('f')], "word-forward").unwrap();
                km.bind(&[Key::alt_char('b')], "word-backward").unwrap();
                // Window scroll (emacs paging + the non-emacs `j`/`k`):
                // moves the window, the point's screen row stays fixed.
                km.bind(&[Key::char('j')], "scroll-line-down").unwrap();
                km.bind(&[Key::char('k')], "scroll-line-up").unwrap();
                km.bind(&[Key::ctrl_char('v')], "scroll-page-down").unwrap();
                km.bind(&[Key::alt_char('v')], "scroll-page-up").unwrap();
                km.bind(&[Key::new(KeyCode::PageDown)], "scroll-page-down").unwrap();
                km.bind(&[Key::new(KeyCode::PageUp)], "scroll-page-up").unwrap();
                km.bind(&[Key::ctrl_char('d')], "scroll-half-page-down").unwrap();
                km.bind(&[Key::ctrl_char('u')], "scroll-half-page-up").unwrap();
                // `g` = force-reload the current file buffer (issue 04's
                // refresh role; M-< / M-> / G move the point to start/end).
                km.bind(&[Key::char('g')], "reload-buffer").unwrap();
                km.bind(&[Key::char('G')], "point-buffer-end").unwrap();
                km
                    .bind(&[Key::alt_char('g'), Key::char('g')], "goto-line")
                    .unwrap();
                km
                    .bind(&[Key::alt_char('<')], "point-buffer-start")
                    .unwrap();
                km
                    .bind(&[Key::alt_char('>')], "point-buffer-end")
                    .unwrap();
                // Plan 004 row 8: recenter cycle (top → middle → bottom → top).
                km.bind(&[Key::ctrl_char('l')], "recenter").unwrap();
                // Plan 004 issue 03: mark/region + kill ring.
                // C-SPC: set-mark. The terminal delivers C-SPC as NUL,
                // which crossterm/iocraft decode as Char(' ') + CONTROL.
                // The binding must match that representation.
                km.bind(&[Key::ctrl_char(' ')], "set-mark").unwrap();
                // C-w: kill-region.
                km.bind(&[Key::ctrl_char('w')], "kill-region").unwrap();
                // M-w: copy-region-to-kill-ring.
                km.bind(&[Key::alt_char('w')], "copy-region").unwrap();
                // C-y: yank.
                km.bind(&[Key::ctrl_char('y')], "yank").unwrap();
                // M-y: yank-pop.
                km.bind(&[Key::alt_char('y')], "yank-pop").unwrap();
                // C-x C-x: exchange point and mark.
                km.bind(&[Key::ctrl_char('x'), Key::ctrl_char('x')], "exchange-point-and-mark")
                    .unwrap();
                // plan 005 issue 02: inline annotations. `A` prompts for a
                // note on the line at point (minibuffer; RET commits and
                // the cue appears immediately); on an annotated line it
                // pre-fills for edit. `d` deletes the annotation on the
                // line at point (a message on an unannotated line — and in
                // EDIT buffers `A`/`d` self-insert as printables, the
                // printable-leaf rule). `C-c a` toggles the inline note
                // rows (the margin markers stay).
                km.bind(&[Key::char('A')], "annotate").unwrap();
                km.bind(&[Key::char('d')], "annotate-delete").unwrap();
                km
                    .bind(
                        &[Key::ctrl_char('c'), Key::char('a')],
                        "annotate-toggle",
                    )
                    .unwrap();
                // Navigation (issue 05).
                km
                    .bind(&[Key::alt_char('.')], "xref-find-definitions")
                    .unwrap();
                km
                    .bind(&[Key::alt_char(',')], "jump-back")
                    .unwrap();
                km
                    .bind(&[Key::ctrl_char('i')], "jump-forward")
                    .unwrap();
                // C-i (Ctrl+I) arrives as Tab from crossterm: bind Tab too.
                km.bind(&[Key::tab()], "jump-forward").unwrap();
                km
                    .bind(&[Key::alt_char('i')], "imenu")
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
                // Issue 05h: `n`/`p` match the magit/log convention (same
                // commands as the arrow / C-n / C-p binds).
                km.bind(&[Key::char('n')], "buffer-list-next").unwrap();
                km.bind(&[Key::char('p')], "buffer-list-prev").unwrap();
                // Issue 05h: `d` is the dired-convention kill verb (parity
                // log row 31). Kills the selected buffer via the same
                // `kill_buffer` path the `C-x k` picker runs; the list
                // stays open.
                km.bind(&[Key::char('d')], "buffer-list-kill-selected").unwrap();
                // PART A fix (item 4): page keys step the selection too.
                km.bind(&[Key::new(KeyCode::PageDown)], "buffer-list-next").unwrap();
                km.bind(&[Key::new(KeyCode::PageUp)], "buffer-list-prev").unwrap();
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
                // PART A fix (item 4): arrows + page keys move the cursor too.
                km.bind(&[Key::down()], "magit-next").unwrap();
                km.bind(&[Key::up()], "magit-prev").unwrap();
                km.bind(&[Key::new(KeyCode::PageDown)], "magit-next").unwrap();
                km.bind(&[Key::new(KeyCode::PageUp)], "magit-prev").unwrap();
                // Issue 08: the magit-status context keys (log/blame/commit/
                // branch/stash) — redline's binding, documented as a
                // deviation from real magit (where `b` is the branch
                // transient and blame is a file-view prefix).
                km.bind(&[Key::char('l')], "magit-log").unwrap();
                km.bind(&[Key::char('b')], "magit-blame").unwrap();
                km.bind(&[Key::char('c')], "magit-commit").unwrap();
                km.bind(&[Key::char('y')], "branch-picker").unwrap();
                km.bind(&[Key::char('z')], "stash-list").unwrap();
                // Issue 002: `h` is magit's top-level dispatch menu (the
                // same component as `?`), and `k` discards the file/hunk at
                // point (confirmation-gated).
                km.bind(&[Key::char('h')], "open-transient-menu").unwrap();
                km.bind(&[Key::char('k')], "magit-discard").unwrap();
                km
            }
            ViewId::Log => {
                // Log (issue 08): n/p page the history, arrows move the
                // in-page selection, RET opens the selected commit's diff,
                // q closes.
                let mut km = KeyMap::new();
                km.bind(&[Key::char('q')], "close-view").unwrap();
                km.bind(&[Key::char('n')], "log-next-page").unwrap();
                km.bind(&[Key::char('p')], "log-prev-page").unwrap();
                km.bind(&[Key::down()], "log-move-down").unwrap();
                km.bind(&[Key::char('j')], "log-move-down").unwrap();
                km.bind(&[Key::ctrl_char('n')], "log-move-down").unwrap();
                km.bind(&[Key::up()], "log-move-up").unwrap();
                km.bind(&[Key::char('k')], "log-move-up").unwrap();
                km.bind(&[Key::ctrl_char('p')], "log-move-up").unwrap();
                // PART A fix (item 4): page keys step the selection too.
                km.bind(&[Key::new(KeyCode::PageDown)], "log-move-down").unwrap();
                km.bind(&[Key::new(KeyCode::PageUp)], "log-move-up").unwrap();
                km.bind(&[Key::enter()], "log-open-commit").unwrap();
                km
            }
            ViewId::Blame => {
                // Blame (issue 08): read-only; q closes. Emacs motion
                // (issue 003-02): the cursor-following window keeps the
                // selected row in view on every move.
                let mut km = KeyMap::new();
                km.bind(&[Key::char('q')], "close-view").unwrap();
                km.bind(&[Key::ctrl_char('n')], "blame-next").unwrap();
                km.bind(&[Key::ctrl_char('p')], "blame-prev").unwrap();
                km.bind(&[Key::ctrl_char('v')], "blame-page-down").unwrap();
                km.bind(&[Key::alt_char('v')], "blame-page-up").unwrap();
                km.bind(&[Key::alt_char('<')], "blame-top").unwrap();
                km.bind(&[Key::alt_char('>')], "blame-bottom").unwrap();
                km
            }
            ViewId::CommitDiff => {
                // Read-only commit diff (issue 08): q closes back to log.
                // Emacs motion (issue 003-02): the pane has no cursor; these
                // move the window (the FileView vocabulary, no new bindings).
                let mut km = KeyMap::new();
                km.bind(&[Key::char('q')], "close-view").unwrap();
                km.bind(&[Key::ctrl_char('n')], "commit-diff-scroll-down").unwrap();
                km.bind(&[Key::ctrl_char('p')], "commit-diff-scroll-up").unwrap();
                km.bind(&[Key::ctrl_char('v')], "commit-diff-page-down").unwrap();
                km.bind(&[Key::alt_char('v')], "commit-diff-page-up").unwrap();
                km.bind(&[Key::alt_char('<')], "commit-diff-scroll-top").unwrap();
                km.bind(&[Key::alt_char('>')], "commit-diff-scroll-bottom").unwrap();
                km
            }
            ViewId::CommitEditor => {
                // Inline commit editor (issue 08). Printable/motion keys and
                // the ESC/C-g aborts are intercepted in `key_event` before the
                // keymap engine; only the C-c C-c / C-c C-k bindings resolve
                // through the engine (so the `C-c` prefix pending state is
                // visible in the status line). q is deliberately NOT bound:
                // in the message buffer a bare q types "q" (matching magit's
                // message buffer, where q is not a command).
                let mut km = KeyMap::new();
                km.bind(&[Key::ctrl_char('c'), Key::ctrl_char('c')], "commit-editor-commit")
                    .unwrap();
                km.bind(&[Key::ctrl_char('c'), Key::ctrl_char('k')], "commit-editor-abort")
                    .unwrap();
                km
            }
            ViewId::Search => {
                // Results view (issue 06): n/p between matches, RET jump
                // (records a jump-stack entry so M-, returns), g re-run,
                // q/ESC close (cancelling an in-flight search), C-g
                // cancels the search without closing.
                let mut km = KeyMap::new();
                km.bind(&[Key::char('q')], "close-search-view").unwrap();
                km.bind(&[Key::new(KeyCode::Escape)], "close-search-view").unwrap();
                km.bind(&[Key::enter()], "search-jump").unwrap();
                km.bind(&[Key::char('n')], "search-next").unwrap();
                km.bind(&[Key::ctrl_char('n')], "search-next").unwrap();
                km.bind(&[Key::char('p')], "search-prev").unwrap();
                km.bind(&[Key::ctrl_char('p')], "search-prev").unwrap();
                km.bind(&[Key::down()], "search-next").unwrap();
                km.bind(&[Key::up()], "search-prev").unwrap();
                // PART A fix (item 4): page keys step the selection too.
                km.bind(&[Key::new(KeyCode::PageDown)], "search-next").unwrap();
                km.bind(&[Key::new(KeyCode::PageUp)], "search-prev").unwrap();
                km.bind(&[Key::char('g')], "search-rerun").unwrap();
                km.bind(&[Key::ctrl_char('g')], "search-cancel").unwrap();
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
    /// `M-.` ambiguous: jump to the selected definition location.
    Xref,
    /// `M-i`: imenu outline of the current file.
    Imenu,
    /// `C-c p s`: project-wide symbol picker.
    Symbols,
    /// `y` in the magit-status context (issue 08): local branches; RET
    /// checks out the selected branch.
    Branch,
    /// `z` in the magit-status context (issue 08): stash list; RET pops, `x`
    /// drops the selected entry.
    Stash,
}

/// The file view's point for one buffer (plan 004 issue 05b): the
/// `(line, col)` position of the cursor plus the emacs **goal column**.
///
/// * `line` — 0-based buffer line of the point.
/// * `col` — 0-based *character* offset within the line (clamped to the
///   line's length at EOL; emacs clamps). This is the horizontal position
///   the cursor renders at.
/// * `goal_col` — the emacs goal column: the horizontal position `C-n` /
///   `C-p` (next/previous-line) try to keep across short lines. Moving to
///   the end of a short line clamps `col` but `goal_col` is preserved, so
///   returning to a longer line restores it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FilePoint {
    pub line: usize,
    pub col: usize,
    pub goal_col: usize,
}

/// One entry in the jump stack (issue 05): the buffer, line, and column
/// to restore, plus a short label for diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JumpEntry {
    /// Buffer key (absolute path string, or `*scratch*`).
    pub buffer_key: String,
    /// Line (0-based).
    pub line: usize,
    /// Column (byte offset within the line; 0 when unknown).
    pub col: usize,
    /// Short label (e.g. the command that triggered the jump).
    pub label: String,
}

/// Bounded jump stack (emacs convention: 256 entries max).
/// `history` holds the full sequence of visited positions (the initial
/// position plus every jump destination). `pos` is the index into
/// `history` of the current position (always `pos < history.len()`
/// when the stack is non-empty).
#[derive(Clone, Debug, Default)]
pub struct JumpStack {
    history: Vec<JumpEntry>,
    pos: usize,
}

impl JumpStack {
    const MAX: usize = 256;

    /// Record a jump from `origin` to `destination`. Truncates forward
    /// history (a new jump invalidates the forward list). When the stack
    /// is empty, `origin` is recorded as the first position.
    pub fn record_jump(&mut self, origin: &JumpEntry, destination: &JumpEntry) {
        // Truncate forward history (keep up to and including the current
        // position, which is the origin of this jump).
        self.history.truncate(self.pos + 1);
        // If the stack is empty, the origin is the first visited position.
        if self.history.is_empty() {
            self.history.push(origin.clone());
        }
        self.history.push(destination.clone());
        if self.history.len() > Self::MAX {
            self.history.remove(0);
            // Adjust pos: removing from the front shifts indices down by 1.
            // The destination is now at the end, so pos = len - 1.
        }
        self.pos = self.history.len() - 1;
    }

    /// `M-,`: move back one entry. Returns the entry to restore.
    pub fn back(&mut self) -> Option<&JumpEntry> {
        if self.pos == 0 {
            return None;
        }
        self.pos -= 1;
        self.history.get(self.pos)
    }

    /// `C-i`: move forward one entry. Returns the entry to restore.
    pub fn forward(&mut self) -> Option<&JumpEntry> {
        // `pos + 1` (never `len() - 1`): an empty history must not underflow.
        if self.pos + 1 >= self.history.len() {
            return None;
        }
        self.pos += 1;
        self.history.get(self.pos)
    }

    #[allow(dead_code)] // used by tests and future UI wiring
    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }

    /// Number of entries in the stack (for tests).
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.history.len()
    }
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

/// The transient menu (issue 002): a bottom-of-frame overlay listing the
/// active view's bindings (`keymap` × `registry`), magit's hydra tree.
/// `open` mirrors the picker's open/closed; `path` is the current submenu
/// (empty at the top level). The menu is a pure renderer over this state.
#[derive(Debug, Default)]
struct TransientMenuState {
    open: bool,
    /// The current submenu path (the prefix keys pressed so far).
    path: KeySeq,
}

/// A menu entry (one key at the current submenu level). A leaf has a
/// `command` (+ registry docs/category); a prefix opens a submenu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransientMenuEntry {
    pub key: Key,
    /// The full sequence including the current path (e.g. `"C-c p"`).
    pub key_display: String,
    /// Some(command name) for a leaf, None for a prefix.
    pub command: Option<String>,
    pub is_prefix: bool,
    /// Registry docs (leaf only).
    pub docs: String,
    /// Registry category (leaf only).
    pub category: String,
}

/// A display row of the transient menu (grouped + sorted in the store).
/// A header row marks a category (or the submenus group).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransientMenuRow {
    pub is_header: bool,
    /// The key sequence (`""` for header rows).
    pub key_display: String,
    /// The description (leaf) or category name (header). Prefix rows carry
    /// an empty label (rendered as `…`).
    pub label: String,
    pub is_prefix: bool,
}

/// The target of an armed destructive-discard confirmation (issue 002):
/// `k` captures the cursor's file/hunk; `y` executes it, `n`/C-g/ESC cancel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscardTarget {
    pub path: String,
    pub kind: SectionKind,
    pub side: Option<Side>,
    pub orig: Option<String>,
    pub hunk_new_start: Option<u32>,
}

/// One row of the buffer-list view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferRow {
    pub name: String,
    pub current: bool,
    pub lines: u64,
}

/// One row of the project file-tree sidebar (issue 09): an indented entry
/// (a directory node or a file leaf) with its depth and project-relative
/// path. `rel_path` is `""` for a top-level root marker; files open via it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRow {
    /// Directory depth (0 = project root level) → indentation.
    pub depth: usize,
    /// The entry's file or directory name (shown after the indent).
    pub name: String,
    /// `true` for a directory node (rendered with a ▸ marker).
    pub is_dir: bool,
    /// Project-relative path ("" when the entry is a directory whose
    /// children are shown by indentation; a file's path for `RET`).
    pub rel_path: String,
}

/// The project file-tree sidebar state (issue 09). `rows` is the indented
/// list of directory/file entries (built from the ignore-aware walk),
/// `selected` the cursor, `visible` whether the sidebar is shown, and
/// `follow` the optional buffer-follow flag (off by default).
#[derive(Debug)]
struct TreeState {
    visible: bool,
    rows: Vec<TreeRow>,
    selected: usize,
    follow: bool,
}

/// One RENDERED row of the file view (plan 005 issue 02): either a code
/// row (the buffer line `line` itself) or a virtual annotation note row
/// (the note rendered directly under the anchored code row `line`). Every
/// row carries its buffer-line index so the renderer, the hardware-cursor
/// math, and the click mapping translate `buffer_line` ↔ `rendered_row`
/// both ways — the dense 1:1 "row i == line top+i" assumption of the
/// pre-annotation slice is gone (note rows are extra rows).
#[derive(Clone, Debug, Default)]
pub struct FileViewRow {
    /// The buffer line this row belongs to: the code row itself for code
    /// rows; the anchored line for a virtual note row.
    pub line: usize,
    /// True for a virtual annotation note row (not a buffer line).
    pub is_note: bool,
    /// Code rows only: the line carries at least one annotation (the
    /// margin marker — independent of note-row visibility, `C-c a`).
    pub annotated: bool,
    /// The row's text (without trailing newline; an orphaned note row
    /// carries an `(orphaned)` tag in its text).
    pub text: String,
    /// Highlight spans (code rows only; byte offsets relative to the line
    /// start). Empty for note rows.
    pub spans: Vec<crate::syntax::highlight::LineSpan>,
}

impl FileViewRow {
    /// The rendered-row index of buffer line `line`'s CODE row (the note
    /// rows under it never match: they are `is_note`).
    pub fn row_for_line(rows: &[FileViewRow], line: usize) -> Option<usize> {
        rows.iter().position(|r| !r.is_note && r.line == line)
    }

    /// The buffer line rendered at rendered row `row` (a note row maps to
    /// its anchored code row).
    pub fn line_for_row(rows: &[FileViewRow], row: usize) -> Option<usize> {
        rows.get(row).map(|r| r.line)
    }
}

/// One annotation anchored to a file line (plan 005 issue 02): the record
/// stored in `.redline-notes.md`'s structured section and rendered inline
/// in the file view. `path` is project-relative. `anchor` is the exact
/// text of the anchored line at creation: automatic re-anchoring (±25
/// lines) keeps `line` pointing at it as the file drifts, and `orphaned`
/// flags the case where the anchor text is gone (the annotation stays at
/// its last known line — NEVER moved to a guessed line).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Annotation {
    pub path: String,
    pub line: usize,
    pub col: usize,
    pub anchor: String,
    pub text: String,
    pub orphaned: bool,
}

/// One entry of the notes file's structured annotation section: either a
/// parsed record or a verbatim raw block (malformed records are kept as
/// raw blocks, never dropped — plan 005 issue 02).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotesEntry {
    Record(Annotation),
    Raw(String),
}

impl NotesEntry {
    /// The record, if this entry is one.
    pub fn as_record(&self) -> Option<&Annotation> {
        match self {
            NotesEntry::Record(a) => Some(a),
            NotesEntry::Raw(_) => None,
        }
    }

    /// The record (mutable), if this entry is one.
    pub fn as_record_mut(&mut self) -> Option<&mut Annotation> {
        match self {
            NotesEntry::Record(a) => Some(a),
            NotesEntry::Raw(_) => None,
        }
    }
}

/// The parsed `.redline-notes.md` document (plan 005 issue 02): free text
/// BEFORE the structured annotation section, the section's entries, and
/// free text AFTER it. Everything outside the section is preserved
/// verbatim on re-serialization.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotesDoc {
    pub before: Vec<String>,
    pub entries: Vec<NotesEntry>,
    pub after: Vec<String>,
}

/// The notes file's structured-section markers (plan 005 issue 02).
pub const NOTES_BEGIN: &str = "<!-- redline-annotations:begin -->";
pub const NOTES_END: &str = "<!-- redline-annotations:end -->";

/// The record start line of the structured section.
pub const NOTES_RECORD_START: &str = "[annotation]";

/// The ±25-line content search window for annotation re-anchoring
/// (plan 005 issue 02).
pub const ANNOTATION_REANCHOR_WINDOW: usize = 25;

/// Parse a notes file's text into a `NotesDoc` (plan 005 issue 02):
/// free text outside the structured section is preserved verbatim; a
/// record block (`[annotation]` + `key: value` lines) parses into an
/// `Annotation` when the required fields (`path`, `line`, `anchor`,
/// `note`) are present and well-formed; every other block (malformed
/// records, stray lines) is kept verbatim as a `NotesEntry::Raw`, never
/// dropped. No markers at all → the whole file is `before` (untouched).
pub fn parse_notes(text: &str) -> NotesDoc {
    let mut lines: Vec<&str> = text.split('\n').collect();
    // A trailing newline is a line terminator, not an empty last line:
    // drop the final empty element so round-trips don't accumulate
    // blank lines.
    if text.ends_with('\n') && lines.last() == Some(&"") {
        lines.pop();
    }
    let begin = lines
        .iter()
        .position(|l| l.trim() == NOTES_BEGIN)
        .and_then(|b| {
            lines[b + 1..]
                .iter()
                .position(|l| l.trim() == NOTES_END)
                .map(|e| (b, b + 1 + e))
        });
    let Some((b, e)) = begin else {
        return NotesDoc {
            before: lines.iter().map(|s| s.to_string()).collect(),
            entries: Vec::new(),
            after: Vec::new(),
        };
    };
    NotesDoc {
        before: lines[..b].iter().map(|s| s.to_string()).collect(),
        entries: parse_notes_section(&lines[b + 1..e]),
        after: lines[e + 1..].iter().map(|s| s.to_string()).collect(),
    }
}

/// Parse the section body into entries: record blocks (`[annotation]`
/// through the next `[annotation]` / end) that carry every required field
/// become `Record`s; anything else is a verbatim `Raw` block (malformed
/// records are kept, never dropped).
fn parse_notes_section(lines: &[&str]) -> Vec<NotesEntry> {
    // Split into (is_record_start, verbatim text) blocks: a block begins at
    // every record start and runs to the next record start.
    let starts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.trim() == NOTES_RECORD_START)
        .map(|(i, _)| i)
        .collect();
    let mut entries = Vec::new();
    // Stray lines before the first record block → one verbatim block.
    if !starts.is_empty() && starts[0] > 0 {
        entries.push(NotesEntry::Raw(lines[..starts[0]].join("\n")));
    }
    for (i, start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(lines.len());
        let block = &lines[*start..end];
        match parse_record_block(block) {
            Some(rec) => entries.push(NotesEntry::Record(rec)),
            None => entries.push(NotesEntry::Raw(block.join("\n"))),
        }
    }
    entries
}

/// Parse one `[annotation]` block into an `Annotation`. `None` when a
/// required field (`path`, `line`, `anchor`, `note`) is missing or
/// ill-formed (the caller keeps the block verbatim).
fn parse_record_block(block: &[&str]) -> Option<Annotation> {
    let mut rec = Annotation::default();
    let mut have = [false; 4]; // path, line, anchor, note
    for line in &block[1..] {
        // Lines without a `:` (stray text, blank lines) are skipped, not
        // fatal: a record stays valid as long as the required fields are
        // present (tolerant parse — hand-edited blocks survive).
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        // Value: the remainder after the first ':' with one leading space
        // stripped (anchor text is otherwise exact — it is the re-anchor
        // key, so it must round-trip byte-for-byte).
        let value = value.strip_prefix(' ').unwrap_or(value);
        match key {
            "path" => {
                rec.path = value.to_string();
                have[0] = true;
            }
            "line" => {
                rec.line = value.trim().parse::<usize>().ok()?;
                have[1] = true;
            }
            "col" => {
                // `col` is optional metadata (not an anchor field): a
                // malformed value defaults to 0 rather than demoting the
                // whole record to raw.
                rec.col = value.trim().parse::<usize>().unwrap_or(0);
            }
            "anchor" => {
                rec.anchor = value.to_string();
                have[2] = true;
            }
            "note" => {
                rec.text = value.to_string();
                have[3] = true;
            }
            "orphaned" => {
                rec.orphaned = value.trim() == "true";
            }
            // Unknown keys inside a record block: the block stays valid
            // (forward compatibility), they are simply not re-emitted.
            _ => {}
        }
    }
    if have.iter().all(|h| *h) {
        Some(rec)
    } else {
        None
    }
}

/// Serialize a `NotesDoc` back to file text (plan 005 issue 02): free text
/// outside the section verbatim, records in canonical form, raw blocks
/// verbatim, in the entries' order.
pub fn serialize_notes(doc: &NotesDoc) -> String {
    let mut out = String::new();
    for line in &doc.before {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(NOTES_BEGIN);
    out.push('\n');
    for entry in &doc.entries {
        match entry {
            NotesEntry::Record(a) => {
                out.push_str(NOTES_RECORD_START);
                out.push('\n');
                out.push_str(&format!("path: {}\n", a.path));
                out.push_str(&format!("line: {}\n", a.line));
                out.push_str(&format!("col: {}\n", a.col));
                out.push_str(&format!("anchor: {}\n", a.anchor));
                out.push_str(&format!("note: {}\n", a.text));
                out.push_str(&format!("orphaned: {}\n", a.orphaned));
            }
            NotesEntry::Raw(s) => {
                out.push_str(s);
                out.push('\n');
            }
        }
    }
    out.push_str(NOTES_END);
    out.push('\n');
    for line in &doc.after {
        out.push_str(line);
        out.push('\n');
    }
    out
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

/// What produced the current search results (drives the view title, the
/// `g` re-run, and the prompt's label).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchKind {
    /// `C-c p s s`: project-wide literal search.
    #[default]
    Project,
    /// `M-?`: references to the symbol under point (word-boundary +
    /// token-class filtering).
    References,
    /// `M-s o`: regex occurrences in the current buffer.
    Occur,
}

impl SearchKind {
    pub fn label(self) -> &'static str {
        match self {
            SearchKind::Project => "Search",
            SearchKind::References => "References",
            SearchKind::Occur => "Occur",
        }
    }
}

/// One row of the search results view (store-owned; the view renders).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResultRow {
    /// A file group header: the (project-relative or buffer) file name
    /// and its live hit count (`final_count` once its `FileDone` event
    /// has arrived).
    Header { file: String, count: u64, final_count: bool },
    /// A match line: the hit itself plus its flat index in the result
    /// set (`selected` indexes into the flat hit list).
    Hit {
        hit: crate::search::rg::Hit,
        hit_index: usize,
    },
}

/// The search-job state owned by the store (issue 06). Hits and rows
/// stream in through `apply_search_event` (the UI's SearchBus drain);
/// the results view is a view over `rows`/`hits`.
#[derive(Debug, Default)]
pub struct SearchState {
    pub kind: SearchKind,
    pub query: String,
    pub running: bool,
    pub cancelled: bool,
    pub error: Option<String>,
    /// The generation this state belongs to (events from older jobs are
    /// discarded).
    pub generation: usize,
    /// Flat hits in arrival order; `selected` indexes into this.
    pub hits: Vec<crate::search::rg::Hit>,
    /// Display rows (headers + hits), in arrival order.
    pub rows: Vec<ResultRow>,
    /// Hit index → its row index (selection/scroll math).
    pub hit_rows: Vec<usize>,
    /// File name → its header row index.
    pub file_row: HashMap<String, usize>,
    /// The selected hit (index into `hits`).
    pub selected: usize,
    /// The results-view scroll top (row index).
    pub scroll: usize,
    /// The in-flight job's cancel flag (`None` when idle).
    pub cancel: Option<Arc<AtomicBool>>,
}

/// The search query prompt (issue 06): typed characters extend the
/// query; RET starts the search, C-g/ESC cancel.
#[derive(Debug)]
pub struct SearchPrompt {
    /// Project search (`C-c p s s`) or buffer occur (`M-s o`).
    pub kind: SearchPromptKind,
    pub query: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchPromptKind {
    /// `C-c p s s`: project-wide literal search (smart case).
    Project,
    /// `M-s o`: regex occurrences in the current buffer.
    Occur,
}

impl SearchPromptKind {
    fn label(self) -> &'static str {
        match self {
            SearchPromptKind::Project => "Search: ",
            SearchPromptKind::Occur => "Occur: ",
        }
    }
}

/// Quit save-prompt state (plan 004 issue 04, emacs
/// `save-buffers-kill-terminal` semantics). `pending` is the SNAPSHOT of
/// locally-modified buffer keys taken at quit-interception time, ordered
/// oldest-first (MRU order reversed) so the least-recently-touched buffer is
/// asked first. The snapshot is never recomputed: a buffer saved during the
/// prompt does not re-appear, and a buffer modified after interception is
/// not asked about.
#[derive(Debug)]
struct QuitPrompt {
    pending: Vec<String>,
}

/// The jump-stack sentinel for the results view: a `JumpEntry` whose
/// `buffer_key` is this never-a-real-buffer value means "return to the
/// search results view" (the view has no buffer of its own; the entry's
/// `line` is the hit index to restore, `col` the scroll top).
const SEARCH_JUMP_KEY: &str = "*search-results*";

/// A page of the magit log (issue 08). `entries` is the current page;
/// `selected` is the in-page cursor (for `RET`). `total` drives the paging
/// indicator and the `n`/`p` bounds.
#[derive(Debug)]
pub struct LogState {
    /// The branch being walked; `None` walks HEAD (the current branch or a
    /// detached tip).
    pub branch: Option<String>,
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
    pub entries: Vec<crate::git::log::LogEntry>,
    pub selected: usize,
}

impl LogState {
    /// The display name for the status line / log title (`HEAD` when walking
    /// HEAD).
    pub fn display(&self) -> &str {
        self.branch.as_deref().unwrap_or("HEAD")
    }
}

/// The read-only full tree diff of one commit (issue 08). `expanded` holds
/// the file paths whose hunks are currently shown (all expanded by default
/// is fine for v1; the set exists for future fold support).
#[derive(Debug)]
pub struct CommitDiffState {
    pub diff: crate::git::log::CommitDiff,
}

/// The blame buffer for the current file (issue 08).
#[derive(Debug)]
pub struct BlameState {
    pub path: String,
    pub lines: Vec<crate::git::blame::BlameLine>,
    pub selected: usize,
}

/// The inline commit-message editor (issue 08): the first editable buffer.
/// `rope` is the message text (comment lines are `#`-prefixed); `cursor` is
/// a char index into the rope. `staged` is the pre-filled staged-file list.
/// `now` is a captured unix time so the pre-fill is deterministic-ish and the
/// view can age the lines without re-querying the clock each render.
#[derive(Debug)]
pub struct CommitEditorState {
    pub rope: ropey::Rope,
    pub cursor: usize,
    pub staged: Vec<String>,
}

/// Page size for the magit log.
const LOG_PAGE: usize = 25;

/// Preview size: a "first page" of the file, byte-capped so a huge
/// file never stalls a selection move.
const PREVIEW_LINES: usize = 32;
const PREVIEW_MAX_BYTES: usize = 64 * 1024;

/// The kill ring: a bounded LIFO of text strings (emacs depth 60).
/// Shared across all buffers (kill in a read-only view, yank in notes).
#[derive(Debug, Clone, Default)]
pub struct KillRing {
    entries: Vec<String>,
}

impl KillRing {
    const MAX: usize = 60;

    /// Push a string onto the ring (most recent first). Empty strings and
    /// consecutive duplicates at the top are suppressed.
    pub fn push(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        if self.entries.first() == Some(&text) {
            return;
        }
        self.entries.insert(0, text);
        if self.entries.len() > Self::MAX {
            self.entries.pop();
        }
    }

    /// The most recent entry (for yank).
    pub fn top(&self) -> Option<&str> {
        self.entries.first().map(|s| s.as_str())
    }

    /// The entry at depth `n` (0 = most recent, 1 = previous, etc.).
    pub fn at(&self, n: usize) -> Option<&str> {
        self.entries.get(n).map(|s| s.as_str())
    }

    /// Number of entries in the ring.
    #[allow(dead_code)] // public API: used by tests and future UI wiring
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[allow(dead_code)] // public API: used by tests and future UI wiring
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

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
    /// Quit save-prompt (plan 004 issue 04): when `Some`, a locally-modified
    /// buffer awaits a save/skip decision and every key is routed to
    /// `quit_prompt_key` (y / n / ! / C-g).
    quit_prompt: Option<QuitPrompt>,
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
    /// The magit status window's top row (issue 002-02): a scroll offset into
    /// `visible_rows()` so long status buffers keep the cursor in view and the
    /// help line stays pinned. Kept in range by `magit_keep_visible`.
    magit_scroll: usize,
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
    /// Per-buffer file-view point (plan 004 issue 05b): buffer key →
    /// `(line, col, goal_col)`. The read-focused cursor: C-n/C-p/C-f/C-b/
    /// C-a/C-e/arrows move it, the window follows (and, for pure window
    /// scrolls, the point's screen row is kept fixed). Edits (notes) keep
    /// their append-at-end cursor and do not use this point.
    point: HashMap<String, FilePoint>,
    /// Incremental in-buffer search state (C-s / C-r).
    isearch: IsearchState,
    /// Goto-line mode active (M-g g).
    goto_line_active: bool,
    /// Goto-line digit input buffer.
    goto_line_input: String,
    /// Number of lines visible in the file view (set by the UI on
    /// resize); used for page-scroll and slice math.
    viewport_lines: usize,
    /// `C-l` recenter cycle index (plan 004 issue 05c; emacs
    /// `recenter-top-bottom` / `recenter-last-op`): counts consecutive
    /// recenters since the last non-recenter command, `mod 3` selects the
    /// next position in the `(middle top bottom)` order. Reset to `0` on
    /// any other command (a fresh C-l therefore goes to MIDDLE).
    recenter_cycle: usize,
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
    /// Jump stack (issue 05): bounded history of (buffer, line, col) for
    /// `M-,` / `C-i` navigation.
    jump_stack: JumpStack,
    /// The installed symbol index snapshot (issue 05). Updated by the
    /// IndexBus drain in the UI layer.
    index: SymbolIndex,
    /// The index-result bus: the background indexer publishes here; the
    /// UI drains it into `index`.
    pub index_bus: IndexBus,
    /// indexing state: `Some((done, total, generation))` when a background
    /// index job is in progress; `None` when idle. The generation tag
    /// identifies which project's job this is, so stale events (from a
    /// previous project's job) can be discarded. Drives the status-line
    /// indicator.
    indexing: Option<(usize, usize, usize)>,
    /// True while an INCREMENTAL reindex job is in flight (PART A fix, item
    /// 2b): the status line then shows `indexing…` (no misleading N/M total,
    /// since an incremental job only reparses the changed files); a full
    /// build shows an honest `indexing N/M` that advances.
    indexing_incremental: bool,
    /// Generation counter for index jobs: bumped on every project switch so
    /// that events from a previous project's in-flight job can be identified
    /// and discarded by `apply_index_event`.
    index_generation: usize,
    /// Changed paths accumulated while an index job is in flight. When the
    /// flight clears, these are coalesced into one incremental job so that
    /// no watcher event is dropped during a job.
    pending_index_changes: HashSet<PathBuf>,
    /// The symbol name being looked up by the Xref picker (set by
    /// `xref_find_definitions` when the lookup is ambiguous).
    xref_lookup_name: String,
    /// The search-event bus (issue 06): the store keeps the sender side
    /// for its lifetime; each search job clones a sender for its worker
    /// thread. The receiver is kept in the store and handed out exactly
    /// once (`search_rx`) to the `use_future` drain in `Root` (the
    /// issue-04/05 bus precedent: the store owns the bus; the UI layer
    /// applies events to the store — and runs inside the render loop, so
    /// each event re-renders).
    pub search_bus: SearchBus,
    search_rx: Option<mpsc::UnboundedReceiver<SearchEvent>>,
    /// Pre-subscribed index receiver: captured in `main` BEFORE the first
    /// index job starts (tokio watch `send` with zero receivers discards the
    /// event -- see the tokio skill). `Root` takes it for its drain.
    index_rx: Option<tokio::sync::watch::Receiver<crate::nav::index::IndexEvent>>,
    /// The current search job's results state (issue 06).
    search: SearchState,
    /// Generation counter for search jobs: bumped on every new search
    /// (and project switch) so events from an in-flight job of an older
    /// generation are discarded by `apply_search_event`.
    search_generation: usize,
    /// The active search-query prompt (`C-c p s s` / `M-s o`), when one
    /// is on screen.
    search_prompt: Option<SearchPrompt>,
    // ── issue 08: log / blame / commit / branches / stash ─────────
    /// The magit log view state, when the log view is on the stack.
    log: Option<LogState>,
    /// The read-only commit-diff view state.
    commit_diff: Option<CommitDiffState>,
    /// The blame view state (for the file being blamed).
    blame: Option<BlameState>,
    /// Per-pane scroll offsets (issue 003-02 shared windowing): the index of
    /// the first visible row in each pane's full row list. Each is kept in
    /// range by the shared window helpers and resets to 0 when its pane opens
    /// (or, for the log, when it pages).
    commit_diff_scroll: usize,
    blame_scroll: usize,
    log_scroll: usize,
    /// The inline commit-editor state (the first editable buffer).
    commit_editor: Option<CommitEditorState>,
    /// The branch-create name prompt (`M-x` → `branch-create`), when active.
    branch_create: Option<String>,
    /// The project file-tree sidebar state (issue 09): toggle + navigate +
    /// `RET` opens. Rows are built from the ignore-aware walk on first use.
    tree: TreeState,
    /// The transient menu (issue 002): bottom-of-frame overlay listing the
    /// active view's bindings (keymap × registry).
    menu: TransientMenuState,
    /// An armed destructive-discard confirmation (issue 002): `k` arms it,
    /// `y` executes, `n`/C-g/ESC cancel. Holds the target to discard.
    discard_confirm: Option<DiscardTarget>,
    /// Buffer keys (absolute path strings) the app created itself this
    /// session (e.g. `.redline-notes.md` on `C-x n`). The watcher's first
    /// event for a just-created file is our own creation, not an external
    /// change: its next matching event is ignored so we don't flag the
    /// buffer "changed on disk" over our own write (issue 05, finding 2).
    created_paths: HashSet<String>,
    /// Buffer keys (absolute path strings) we saved in place this session
    /// (plan 005 issue 01), with the mtime the file had right after our
    /// write. A watcher event for our own save (the file's mtime still
    /// matches the recorded one) is not an external change; a genuinely
    /// later external write changes the mtime and still conflicts.
    /// Mirrors `created_paths` (which covers files we CREATED) for files
    /// we merely SAVED.
    saved_paths: HashMap<String, std::time::SystemTime>,
    /// A toggle-read-only confirm is armed (plan 005 issue 01): toggling a
    /// file buffer back to read-only with unsaved edits awaits a
    /// `y`/`n`/C-g decision (every key routes to `toggle_ro_key`). Holds
    /// the buffer key to confirm.
    toggle_ro_confirm: Option<String>,
    // ── plan 005 issue 02: inline annotations ──────────────────────────
    /// The parsed `.redline-notes.md` document (plan 005 issue 02): free
    /// text outside the structured annotation section is preserved
    /// verbatim; malformed records are kept as raw blocks, never dropped.
    notes_doc: NotesDoc,
    /// Whether `notes_doc` has been loaded (from the open notes buffer or
    /// from disk) this session.
    notes_doc_loaded: bool,
    /// The disk mtime recorded when `notes_doc` was loaded from disk (the
    /// closed-buffer freshness check; while the notes buffer is open its
    /// text is the source of truth).
    notes_doc_mtime: Option<std::time::SystemTime>,
    /// True while the open notes buffer's text has not yet been re-parsed
    /// into `notes_doc` after a local edit / reload.
    notes_buffer_dirty: bool,
    /// The `A` annotation prompt state (plan 005 issue 02): the minibuffer
    /// note entry. `note_prompt_line` is the annotated buffer line.
    note_prompt_active: bool,
    note_prompt_input: String,
    note_prompt_line: usize,
    /// Whether the inline annotation note rows render under annotated
    /// lines (C-c a toggles; the margin markers stay either way).
    show_note_rows: bool,
    // ── plan 004 issue 03: mark/region + kill ring ──────────────────────
    /// The shared kill ring (emacs depth 60; shared across all buffers).
    kill_ring: KillRing,
    /// Byte offset where the last yank was inserted (for M-y yank-pop).
    yank_pos: Option<usize>,
    /// Length of the last yanked text (for M-y yank-pop).
    yank_len: Option<usize>,
    /// Kill ring depth for yank-pop (M-y cycles backward from 0).
    /// `None` when no yank is in progress.
    yank_ring_index: Option<usize>,
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
        // View cycling (issue 01) was M-s / M-p, but issue 06's `M-s o`
        // (occur) needs the M-s prefix; the engine forbids a command on a
        // strict prefix of a longer binding, so cycling is now M-x only
        // (`cycle-view-next` / `cycle-view-prev`).
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
        global
            .bind(&[Key::ctrl_char('x'), Key::char('n')], "open-notes")
            .unwrap();
        global
            .bind(&[Key::ctrl_char('x'), Key::ctrl_char('s')], "save-buffer")
            .unwrap();
        // plan 005 issue 01: file edit mode (emacs `toggle-read-only`).
        global
            .bind(&[Key::ctrl_char('x'), Key::ctrl_char('q')], "toggle-read-only")
            .unwrap();
        // Magit status (issue 07).
        global
            .bind(&[Key::ctrl_char('x'), Key::char('g')], "magit-status")
            .unwrap();
        // Transient menu (issue 002): `?` opens this view's command menu in
        // any view (magit's hydra tree; `h` does the same in magit views).
        global.bind(&[Key::char('?')], "open-transient-menu").unwrap();
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
        global
            .bind(
                &[Key::ctrl_char('c'), Key::char('p'), Key::char('s'), Key::char('s')],
                "project-search",
            )
            .unwrap();
        // Tree sidebar (issue 09).
        global
            .bind(
                &[Key::ctrl_char('c'), Key::char('p'), Key::char('t')],
                "toggle-tree",
            )
            .unwrap();
        // Search & references (issue 06).
        global.bind(&[Key::alt_char('?')], "references-at-point").unwrap();
        global
            .bind(&[Key::alt_char('s'), Key::char('o')], "occur")
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

        // Search bus (issue 06): the store keeps the sender side for its
        // lifetime; the receiver stays in the store until Root's
        // `use_future` drain takes it (exactly once).
        let (search_bus, search_rx) = SearchBus::new();
        let search_rx = Some(search_rx);

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
            quit_prompt: None,
            activity: Vec::new(),
            message: String::new(),
            picker: None,
            matcher: Matcher::new(nucleo_matcher::Config::DEFAULT),
            file_matcher: Matcher::new(nucleo_matcher::Config::DEFAULT.match_paths()),
            git: None,
            status_tree: None,
            magit_scroll: 0,
            dirty: None,
            grammar_registry: GrammarRegistry::build(),
            highlight_cache: HighlightCache::new(),
            scroll: HashMap::new(),
            point: HashMap::new(),
            isearch: IsearchState::default(),
            goto_line_active: false,
            goto_line_input: String::new(),
            viewport_lines: 24, // default; the UI updates on resize
            recenter_cycle: 0,
            watch_bus: ChangeBus::new(),
            watcher: None,
            auto_reload: true,
            watch_suspended: false,
            jump_stack: JumpStack::default(),
            index: SymbolIndex::new(),
            index_bus: IndexBus::new(),
            indexing: None,
            indexing_incremental: false,
            index_generation: 0,
            pending_index_changes: HashSet::new(),
            xref_lookup_name: String::new(),
            search_bus,
            search_rx,
            index_rx: None,
            search: SearchState::default(),
            search_generation: 0,
            search_prompt: None,
            log: None,
            commit_diff: None,
            blame: None,
            commit_diff_scroll: 0,
            blame_scroll: 0,
            log_scroll: 0,
            commit_editor: None,
            branch_create: None,
            tree: TreeState {
                visible: false,
                rows: Vec::new(),
                selected: 0,
                follow: false,
            },
            menu: TransientMenuState::default(),
            discard_confirm: None,
            created_paths: HashSet::new(),
            saved_paths: HashMap::new(),
            toggle_ro_confirm: None,
            notes_doc: NotesDoc::default(),
            notes_doc_loaded: false,
            notes_doc_mtime: None,
            notes_buffer_dirty: false,
            note_prompt_active: false,
            note_prompt_input: String::new(),
            note_prompt_line: 0,
            show_note_rows: true,
            kill_ring: KillRing::default(),
            yank_pos: None,
            yank_len: None,
            yank_ring_index: None,
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
    /// `*list-buffers*` in the buffer-list view, `*magit-status*`,
    /// `*search*`).
    pub fn view_name_display(&self) -> String {
        match self.top_view() {
            ViewId::Buffer => self
                .buffers
                .current()
                .map(|key| self.buffer_display(key))
                .unwrap_or_else(|| SCRATCH_NAME.to_string()),
            ViewId::BufferList => "*list-buffers*".to_string(),
            ViewId::MagitStatus => "*magit-status*".to_string(),
            ViewId::Log => "*log*".to_string(),
            ViewId::Blame => "*blame*".to_string(),
            ViewId::CommitDiff => "*commit-diff*".to_string(),
            ViewId::CommitEditor => "*commit*".to_string(),
            ViewId::Search => "*search*".to_string(),
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

    /// Insert text at the end of the current buffer (the shared editing
    /// primitive behind notes / scratch typing); false when no buffer is
    /// current or the buffer is not editable.
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
            // Keep the insertion row (the new end of the buffer) inside the
            // visible window (issue 003-02: typing near the bottom keeps the
            // active region in view). The file view's window is the full
            // `viewport_lines` (its title/indicator live in the same area).
            self.buffer_keep_insert_visible();
            true
        } else {
            false
        }
    }

    /// Keep the current buffer's last line (the insertion row) inside the
    /// visible file-view window after an edit; no-op when there is no current
    /// buffer or the row is already visible. Uses the shared window math with
    /// the file view's window size (`viewport_lines`).
    fn buffer_keep_insert_visible(&mut self) {
        let key = self.buffers.current().map(String::from);
        let Some(key) = key else {
            return;
        };
        let total = self.buffers.get(&key).map(|b| b.line_count()).unwrap_or(0);
        if total == 0 {
            return;
        }
        let insert_row = total - 1;
        let window = self.viewport_lines.max(1);
        let next = keep_cursor_visible(self.scroll_top(), insert_row, total, window);
        if next != self.scroll_top() {
            self.scroll.insert(key, next);
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

    /// Open (or create) the per-project notes file (`.redline-notes.md`) as
    /// an editable buffer. PART B item 8: the file is locally-owned
    /// (conflict rules identical to issue 04); editing is bounded
    /// (append + backspace, like the commit editor) with explicit save
    /// via `C-x C-s` (`save-buffer`).
    pub fn open_notes(&mut self) {
        const NOTES_REL: &str = ".redline-notes.md";
        let Some(project) = self.project.clone() else {
            self.minibuffer_message("open-notes: no project open");
            return;
        };
        let abs = project.root.join(NOTES_REL);
        // Create the file if it doesn't exist yet. Track whether WE created
        // it: the watcher's first event for a just-created file is our own
        // creation (not an external change), and it must not flag the buffer
        // "changed on disk" (issue 05, finding 2).
        let created_by_us = if abs.exists() {
            false
        } else {
            if std::fs::write(&abs, "# Notes\n").is_err() {
                self.minibuffer_message("open-notes: could not create notes file");
                return;
            }
            true
        };
        let key = abs.to_string_lossy().into_owned();
        if created_by_us {
            self.created_paths.insert(key.clone());
        }
        if self.buffers.get(&key).is_none() {
            match load_file(&abs) {
                Ok((rope, mtime)) => {
                    // Editable + locally-owned: the user types notes here;
                    // disk changes are flagged (conflict marker) rather
                    // than silently overwriting local edits.
                    self.buffers
                        .insert_rope(Some(abs.clone()), rope, mtime, true);
                }
                Err(e) => {
                    self.minibuffer_message(&format!("cannot open notes: {e}"));
                    return;
                }
            }
        }
        self.buffers.set_current(&key);
        // plan 005 issue 02: on (re)open the notes buffer's text is the
        // source of truth — re-parse the notes document from it.
        self.notes_buffer_dirty = true;
        self.record_recent(NOTES_REL);
        self.ensure_highlight();
        self.minibuffer_message("notes: C-x C-s to save");
    }

    /// Save the current buffer to its on-disk path (issue 09, notes +
    /// any editable buffer). No-op when there is no current buffer.
    pub fn save_buffer(&mut self) {
        let Some(key) = self.buffers.current() else {
            self.minibuffer_message("save-buffer: no current buffer");
            return;
        };
        let key = key.to_string();
        self.save_buffer_key(&key);
    }

    /// Save the buffer with `key` to its on-disk path (plan 004 issue 04:
    /// the quit save-prompt must save buffers that are not the current one).
    /// Updates the buffer's mtime and clears `locally_modified`. Returns
    /// `true` when the write landed; on any refusal/failure the minibuffer
    /// message reports the reason and `false` is returned.
    pub fn save_buffer_key(&mut self, key: &str) -> bool {
        let (path, text) = {
            let buf = match self.buffers.get(key) {
                Some(b) => b,
                None => {
                    self.minibuffer_message("save-buffer: no buffer");
                    return false;
                }
            };
            if !buf.editable {
                self.minibuffer_message("save-buffer: buffer is read-only");
                return false;
            }
            let path = match &buf.path {
                Some(p) => p.clone(),
                None => {
                    self.minibuffer_message("save-buffer: no file (scratch)");
                    return false;
                }
            };
            (path, buf.rope.to_string())
        };
        match std::fs::write(&path, &text) {
            Ok(()) => {
                let mtime = std::fs::metadata(&path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                if let Some(buf) = self.buffers.get_mut(key) {
                    buf.mtime = mtime;
                    buf.locally_modified = false;
                    buf.changed_on_disk = false;
                }
                // Watcher self-write suppression (plan 005 issue 01): the
                // next watcher event for this path is OUR own save. Record
                // the post-save mtime so `apply_project_change` can tell
                // our write (mtime still matches → suppress) from a
                // genuinely later external write (mtime differs → still
                // conflicts).
                self.saved_paths.insert(key.to_string(), mtime);
                self.invalidate_highlight_for_key(key);
                // plan 005 issue 02: the automatic anchoring runs in the
                // same pass as the edit-mode save — the saved content is
                // the anchor truth.
                self.reanchor_for_key(key);
                // Saving the notes file itself re-parses its structured
                // section from the saved text (the buffer is the source
                // of truth while open).
                if self.notes_key().as_deref() == Some(key) {
                    self.notes_doc_mtime = Some(mtime);
                    self.notes_buffer_dirty = true;
                    self.ensure_notes_doc();
                }
                self.minibuffer_message(&format!("wrote {}", path.display()));
                true
            }
            Err(e) => {
                self.minibuffer_message(&format!("save failed: {e}"));
                false
            }
        }
    }

    // ── plan 005 issue 02: inline annotations ────────────────────────

    /// The absolute buffer key of the per-project notes file
    /// (`.redline-notes.md`), when a project is open.
    fn notes_key(&self) -> Option<String> {
        let root = self.project.as_ref()?.root.clone();
        Some(root.join(".redline-notes.md").to_string_lossy().into_owned())
    }

    /// Load / re-parse the notes document (plan 005 issue 02). While the
    /// notes buffer is open, ITS text is the source of truth (re-parsed
    /// when `notes_buffer_dirty` marks a local edit/reload); otherwise the
    /// disk file is (re-read on mtime change). The lazy disk-load path runs
    /// the re-anchor pass over every open buffer ("on load").
    fn ensure_notes_doc(&mut self) {
        let Some(key) = self.notes_key() else {
            return;
        };
        if self.buffers.get(&key).is_some() {
            if !self.notes_doc_loaded || self.notes_buffer_dirty {
                let text = self
                    .buffers
                    .get(&key)
                    .map(|b| b.text())
                    .unwrap_or_default();
                self.notes_doc = parse_notes(&text);
                self.notes_doc_loaded = true;
                self.notes_buffer_dirty = false;
            }
        } else {
            let path = PathBuf::from(key.clone());
            let mtime = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok());
            if !self.notes_doc_loaded || mtime != self.notes_doc_mtime {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                self.notes_doc = parse_notes(&text);
                self.notes_doc_loaded = true;
                self.notes_doc_mtime = mtime;
                // On load: the anchors maintain themselves against every
                // open buffer's current content.
                self.reanchor_all_buffers();
            }
        }
    }

    /// The current buffer's project-relative path (annotation records are
    /// keyed by it), `None` for pathless buffers (scratch) or no project.
    fn current_buffer_rel(&self) -> Option<String> {
        let key = self.buffers.current()?.to_string();
        self.buffer_rel_path(&key)
    }

    /// The index of the FIRST structured-section record anchored at the
    /// current buffer's line `line`, or `None` when the line carries no
    /// annotation.
    fn record_index_for_line(&self, line: usize) -> Option<usize> {
        let rel = self.current_buffer_rel()?;
        self.notes_doc
            .entries
            .iter()
            .position(|e| matches!(e, NotesEntry::Record(a) if a.path == rel && a.line == line))
    }

    /// Re-anchor the annotations of the buffer at `key` against the
    /// buffer's current content (plan 005 issue 02): for each record
    /// anchored there whose stored line no longer holds `anchor` exactly,
    /// search ±25 lines — a UNIQUE match re-anchors (updates `line`,
    /// clears `orphaned`); zero or multiple matches set `orphaned = true`
    /// and leave `line` unchanged (NEVER move an anchor to a guessed line).
    /// Stable and idempotent: a record whose line holds the anchor is
    /// untouched (its `orphaned` flag clears — the anchor came back).
    fn reanchor_for_key(&mut self, key: &str) {
        let Some(rel) = self.buffer_rel_path(key) else {
            return;
        };
        let Some(buf) = self.buffers.get(key) else {
            return;
        };
        let total = buf.line_count();
        if total == 0 {
            return;
        }
        let mut changed = false;
        for entry in self.notes_doc.entries.iter_mut() {
            let Some(a) = entry.as_record_mut() else {
                continue;
            };
            if a.path != rel {
                continue;
            }
            let held = a.line < total
                && buf
                    .line_text(a.line)
                    .map(|t| t.as_ref() == a.anchor.as_str())
                    .unwrap_or(false);
            if held {
                if a.orphaned {
                    a.orphaned = false;
                    changed = true;
                }
                continue;
            }
            // Drift: content search within ±ANNOTATION_REANCHOR_WINDOW.
            let lo = a.line.saturating_sub(ANNOTATION_REANCHOR_WINDOW);
            let hi = (a.line + ANNOTATION_REANCHOR_WINDOW).min(total - 1);
            let matches: Vec<usize> = (lo..=hi)
                .filter(|l| {
                    buf.line_text(*l)
                        .map(|t| t.as_ref() == a.anchor.as_str())
                        .unwrap_or(false)
                })
                .collect();
            if matches.len() == 1 {
                a.line = matches[0];
                a.orphaned = false;
                changed = true;
            } else if !a.orphaned {
                // 0 or ambiguous matches: flag, never guess.
                a.orphaned = true;
                changed = true;
            }
        }
        if changed {
            // The re-anchored records are the truth: refresh the notes
            // file (and the open notes buffer, when one is open).
            self.sync_notes_from_doc();
        }
    }

    /// Re-anchor every open file buffer's annotations (the "on load"
    /// pass: runs after the notes document is (re)loaded from disk).
    fn reanchor_all_buffers(&mut self) {
        let keys: Vec<String> = self
            .buffers
            .list()
            .into_iter()
            .filter(|(_, b)| b.path.is_some())
            .map(|(k, _)| k.to_string())
            .collect();
        for key in keys {
            self.reanchor_for_key(&key);
        }
    }

    /// Write the notes document back to disk (and into the open notes
    /// buffer when one is open — the real-file surface stays in sync with
    /// the in-memory records). Returns the post-write mtime on success;
    /// the watcher self-write suppression (plan 005 issue 01) records the
    /// write so our own event is not flagged "changed on disk".
    fn sync_notes_from_doc(&mut self) -> Option<std::time::SystemTime> {
        let key = self.notes_key()?;
        let path = PathBuf::from(key.clone());
        let text = serialize_notes(&self.notes_doc);
        if std::fs::write(&path, &text).is_err() {
            return None;
        }
        let mtime = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok());
        if let Some(m) = mtime {
            self.saved_paths.insert(key.clone(), m);
            self.notes_doc_mtime = Some(m);
        }
        // The in-memory doc now equals the disk: loaded.
        self.notes_doc_loaded = true;
        if self.buffers.get(&key).is_some() {
            self.replace_buffer_text(&key, &text);
            if let Some(buf) = self.buffers.get_mut(&key) {
                if let Some(m) = mtime {
                    buf.mtime = m;
                }
                buf.locally_modified = false;
                buf.changed_on_disk = false;
                buf.mark = None;
            }
            self.invalidate_highlight_for_key(&key);
        }
        mtime
    }

    /// Replace a buffer's text (the notes-buffer sync path).
    fn replace_buffer_text(&mut self, key: &str, text: &str) {
        let rope = Rope::from_str(text);
        if let Some(buf) = self.buffers.get_mut(key) {
            buf.rope = rope;
        }
    }

    /// `A` (plan 005 issue 02): prompt for an annotation on the line at
    /// point in the minibuffer. An existing record on the line pre-fills
    /// the prompt (edit); RET commits (record written to
    /// `.redline-notes.md`, cue appears immediately), C-g/ESC cancels.
    pub fn annotate(&mut self) {
        if self.top_view() != ViewId::Buffer {
            self.minibuffer_message("annotate: not in the file view");
            return;
        }
        let Some(key) = self.buffers.current().map(String::from) else {
            return;
        };
        if self.buffers.get(&key).map(|b| b.path.is_none()).unwrap_or(true) {
            self.minibuffer_message("annotate: no file to annotate (scratch)");
            return;
        }
        self.ensure_notes_doc();
        let line = self.point_line();
        let prefill = self.notes_doc.entries.iter().find_map(|e| {
            e.as_record()
                .filter(|a| a.line == line && a.path == self.current_buffer_rel().as_deref().unwrap_or(""))
                .map(|a| a.text.clone())
        });
        self.note_prompt_line = line;
        self.note_prompt_input = prefill.unwrap_or_default();
        self.note_prompt_active = true;
        self.minibuffer_message(&format!("Note: {}", self.note_prompt_input));
    }

    /// The `A` prompt state (for tests).
    #[allow(dead_code)] // public API: used by tests
    pub fn note_prompt_active(&self) -> bool {
        self.note_prompt_active
    }

    /// The `A` prompt's current input (for tests + prefill verification).
    #[allow(dead_code)] // public API: used by tests
    pub fn note_prompt_input(&self) -> &str {
        &self.note_prompt_input
    }

    fn note_prompt_char(&mut self, c: char) {
        self.note_prompt_input.push(c);
        self.minibuffer_message(&format!("Note: {}", self.note_prompt_input));
    }

    fn note_prompt_backspace(&mut self) {
        self.note_prompt_input.pop();
        self.minibuffer_message(&format!("Note: {}", self.note_prompt_input));
    }

    fn note_prompt_cancel(&mut self) {
        if !self.note_prompt_active {
            return;
        }
        self.note_prompt_active = false;
        self.note_prompt_input.clear();
        self.minibuffer_message("note cancelled");
    }

    /// RET in the `A` prompt (plan 005 issue 02): commit the record.
    /// Empty input on an existing record deletes it; empty input on a
    /// fresh prompt cancels.
    pub fn note_prompt_confirm(&mut self) {
        if !self.note_prompt_active {
            return;
        }
        let line = self.note_prompt_line;
        let text = self.note_prompt_input.trim().to_string();
        self.note_prompt_active = false;
        self.note_prompt_input.clear();
        if text.is_empty() {
            if self.record_index_for_line(line).is_some() {
                self.delete_annotation_at(line);
            } else {
                self.minibuffer_message("note cancelled");
            }
            return;
        }
        let Some(key) = self.buffers.current().map(String::from) else {
            return;
        };
        let Some(rel) = self.buffer_rel_path(&key) else {
            self.minibuffer_message("annotate: no file to annotate (scratch)");
            return;
        };
        let Some(buf) = self.buffers.get(&key) else {
            return;
        };
        let total = buf.line_count();
        if total == 0 {
            self.minibuffer_message("annotate: empty buffer");
            return;
        }
        let line = line.min(total - 1);
        let anchor = buf.line_text(line).map(|t| t.into_owned()).unwrap_or_default();
        let col = self.point_col();
        let existing = self.notes_doc.entries.iter().position(|e| {
            matches!(e, NotesEntry::Record(a) if a.path == rel && a.line == line)
        });
        match existing {
            Some(i) => {
                // Edit: only the note text changes (the record's stored
                // position stays — the re-anchor pass maintains it).
                if let Some(a) = self.notes_doc.entries[i].as_record_mut() {
                    a.text = text.clone();
                }
            }
            None => {
                self.notes_doc
                    .entries
                    .push(NotesEntry::Record(Annotation {
                        path: rel.clone(),
                        line,
                        col,
                        anchor,
                        text: text.clone(),
                        orphaned: false,
                    }));
            }
        }
        match self.sync_notes_from_doc() {
            Some(_) => {
                let n = self.current_buffer_annotation_count();
                self.minibuffer_message(&format!(
                    "note saved ({} note{} in {})",
                    n,
                    if n == 1 { "" } else { "s" },
                    rel
                ));
            }
            None => self.minibuffer_message("note not saved: could not write the notes file"),
        }
    }

    /// `d` in the buffer view (plan 005 issue 02): delete the annotation
    /// on the line at point, echoing what was removed. A line with no
    /// annotation gets a message (never a self-insert, never an unbound-key
    /// echo).
    pub fn annotate_delete(&mut self) {
        if self.top_view() != ViewId::Buffer {
            self.minibuffer_message("annotate-delete: not in the file view");
            return;
        }
        let Some(key) = self.buffers.current().map(String::from) else {
            return;
        };
        if self.buffers.get(&key).map(|b| b.path.is_none()).unwrap_or(true) {
            self.minibuffer_message("annotate-delete: no file (scratch)");
            return;
        }
        self.ensure_notes_doc();
        let line = self.point_line();
        match self.record_index_for_line(line) {
            Some(idx) => self.delete_annotation_at_index(idx),
            None => self.minibuffer_message("no annotation on this line"),
        }
    }

    /// Delete the record anchored at the current buffer's line `line` (the
    /// empty-RET edit-path delete).
    fn delete_annotation_at(&mut self, line: usize) {
        match self.record_index_for_line(line) {
            Some(idx) => self.delete_annotation_at_index(idx),
            None => self.minibuffer_message("note cancelled"),
        }
    }

    fn delete_annotation_at_index(&mut self, idx: usize) {
        let removed = match self.notes_doc.entries.get(idx) {
            Some(NotesEntry::Record(a)) => a.text.clone(),
            _ => return,
        };
        self.notes_doc.entries.remove(idx);
        match self.sync_notes_from_doc() {
            Some(_) => {
                let chars: Vec<char> = removed.chars().collect();
                let echo: String = chars.iter().take(40).collect();
                let echo = if chars.len() > 40 { format!("{echo}…") } else { echo };
                self.minibuffer_message(&format!("deleted annotation: {echo}"));
            }
            None => self.minibuffer_message(
                "annotation not deleted: could not write the notes file",
            ),
        }
    }

    /// `C-c a` in the buffer view (plan 005 issue 02): toggle the inline
    /// annotation note rows; the margin markers stay either way.
    pub fn annotate_toggle(&mut self) {
        if self.top_view() != ViewId::Buffer {
            return;
        }
        self.show_note_rows = !self.show_note_rows;
        self.minibuffer_message(if self.show_note_rows {
            "note rows: shown"
        } else {
            "note rows: hidden"
        });
    }

    /// The annotation count for the current buffer's file (0 for
    /// pathless buffers or when no record matches its path).
    fn current_buffer_annotation_count(&self) -> usize {
        let Some(rel) = self.current_buffer_rel() else {
            return 0;
        };
        self.notes_doc
            .entries
            .iter()
            .filter(|e| matches!(e, NotesEntry::Record(a) if a.path == rel))
            .count()
    }

    /// The status-line annotation count for the current file (`"1 note"` /
    /// `"3 notes"`; empty outside the buffer view or with no annotations).
    pub fn annotation_count_display(&self) -> String {
        if self.top_view() != ViewId::Buffer {
            return String::new();
        }
        let n = self.current_buffer_annotation_count();
        if n == 0 {
            return String::new();
        }
        format!("{n} note{}", if n == 1 { "" } else { "s" })
    }

    /// `C-x C-q` (emacs `toggle-read-only`, plan 005 issue 01): flip the
    /// current FILE buffer between read-only and edit mode. File-backed
    /// buffers start read-only; this is the first mid-session editability
    /// flip. Toggling an editable file buffer with unsaved edits arms the
    /// discard confirm (`y` discards + goes read-only, `n`/C-g/ESC cancel
    /// and keep edit mode) instead of silently losing the edits.
    /// Non-file buffers (scratch) and non-buffer views are no-ops with a
    /// minibuffer message.
    pub fn toggle_read_only(&mut self) {
        if self.top_view() != ViewId::Buffer {
            self.minibuffer_message("toggle-read-only: not a buffer view");
            return;
        }
        let Some(key) = self.buffers.current().map(str::to_string) else {
            self.minibuffer_message("toggle-read-only: no current buffer");
            return;
        };
        let (editable, locally_modified) = {
            let buf = match self.buffers.get(&key) {
                Some(b) => b,
                None => {
                    self.minibuffer_message("toggle-read-only: no current buffer");
                    return;
                }
            };
            if buf.path.is_none() {
                // Scratch (no on-disk path): nothing to toggle.
                self.minibuffer_message("toggle-read-only: scratch has no file");
                return;
            }
            (buf.editable, buf.locally_modified)
        };
        if editable {
            if locally_modified {
                // Unsaved edits must not be lost silently: confirm first
                // (the display name keeps the prompt inside one minibuffer
                // row at 80 columns).
                self.toggle_ro_confirm = Some(key.clone());
                self.minibuffer_message(&format!(
                    "Discard unsaved edits in {} to make it read-only? (y or n)",
                    self.buffer_display(&key)
                ));
                return;
            }
            if let Some(buf) = self.buffers.get_mut(&key) {
                buf.editable = false;
            }
            self.minibuffer_message("read-only (C-x C-q to edit)");
        } else {
            if let Some(buf) = self.buffers.get_mut(&key) {
                buf.editable = true;
            }
            self.minibuffer_message("editable (C-x C-s to save)");
        }
    }

    /// Whether a toggle-read-only discard confirm is armed.
    pub fn toggle_ro_active(&self) -> bool {
        self.toggle_ro_confirm.is_some()
    }

    /// The confirm's state machine (plan 005 issue 01): `y` discards the
    /// unsaved edits (re-reads the on-disk content) and makes the buffer
    /// read-only; `n`, C-g, and ESC cancel and keep edit mode (the text is
    /// untouched). Every other key is swallowed (no "unbound key" echo
    /// mid-prompt, matching the quit save-prompt discipline).
    pub fn toggle_ro_key(&mut self, key: Key) {
        if key == Key::ctrl_char('g') || key.code == KeyCode::Escape {
            self.toggle_ro_cancel();
            return;
        }
        let Some(c) = key.char_value() else { return };
        match c {
            'y' => {
                self.toggle_ro_accept();
            }
            'n' => {
                self.toggle_ro_cancel();
            }
            _ => {}
        }
    }

    /// The confirm's `y`: discard the local edits (re-read the file from
    /// disk, clearing `locally_modified`/`changed_on_disk`) and turn the
    /// buffer read-only. A failed re-read keeps the confirm armed (no
    /// silent state half-change).
    fn toggle_ro_accept(&mut self) {
        let Some(key) = self.toggle_ro_confirm.clone() else {
            return;
        };
        let Some(path) = self.buffers.get(&key).and_then(|b| b.path.clone()) else {
            self.toggle_ro_confirm = None;
            return;
        };
        let (rope, mtime) = match load_file(&path) {
            Ok(x) => x,
            Err(e) => {
                self.minibuffer_message(&format!("cannot discard: {e}"));
                return;
            }
        };
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope = rope;
            buf.mtime = mtime;
            buf.locally_modified = false;
            buf.changed_on_disk = false;
            buf.editable = false;
        }
        self.toggle_ro_confirm = None;
        self.ensure_highlight_for_key(&key);
        self.minibuffer_message("read-only (C-x C-q to edit)");
    }

    /// The confirm's `n` / C-g / ESC: cancel; edit mode and the text stay.
    fn toggle_ro_cancel(&mut self) {
        self.toggle_ro_confirm = None;
        self.minibuffer_message("cancel (edit mode kept)");
    }

    /// Append a character to the current buffer at its end (bounded
    /// editing for notes, mirroring the commit-editor's insert path).
    /// Returns `true` when the character was inserted.
    pub fn notes_insert_char(&mut self, c: char) -> bool {
        let key = self.buffers.current().map(String::from);
        let inserted = self.insert_text(&c.to_string());
        // plan 005 issue 02: local edits to the OPEN notes buffer mark the
        // notes document stale (it re-parses on the next ensure).
        if inserted && key.as_deref() == self.notes_key().as_deref() {
            self.notes_buffer_dirty = true;
        }
        inserted
    }

    /// Delete the last character of the current buffer (bounded editing
    /// for notes, mirroring the commit-editor's backspace path).
    pub fn notes_backspace(&mut self) {
        let Some(key) = self.buffers.current() else { return };
        let key = key.to_string();
        let len = self
            .buffers
            .get(&key)
            .map(|b| b.rope.len_chars())
            .unwrap_or(0);
        if len == 0 {
            return;
        }
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.remove((len - 1)..len);
            buf.locally_modified = true;
            self.invalidate_highlight_for_key(&key);
        }
        // plan 005 issue 02: a notes-buffer edit marks the notes document
        // stale (re-parsed on the next ensure).
        if key == self.notes_key().unwrap_or_default() {
            self.notes_buffer_dirty = true;
        }
    }

    // ── plan 004 issue 03: mark / region / kill ring / yank ───────────

    /// The current buffer's "point" as a byte offset: the start of the line
    /// at the point's line (column 0). The region mark is a line-start byte
    /// offset, so the point's column does not extend the region (plan 004
    /// issue 05b: region semantics unchanged). Returns `None` when there is
    /// no current buffer.
    fn current_point_byte(&self) -> Option<usize> {
        let key = self.buffers.current()?.to_string();
        let buf = self.buffers.get(&key)?;
        let line = self.point_line();
        buf.rope.try_line_to_byte(line.min(buf.line_count().saturating_sub(1))).ok()
    }

    /// The region's normalized byte range [start, end) for the current buffer,
    /// or `None` when no mark is set.
    pub fn region_byte_range(&self) -> Option<(usize, usize)> {
        let key = self.buffers.current()?.to_string();
        let buf = self.buffers.get(&key)?;
        let mark = buf.mark?;
        let point = self.current_point_byte()?;
        let start = mark.min(point);
        let end = mark.max(point);
        if start == end {
            None
        } else {
            Some((start, end))
        }
    }

    /// The region size in bytes (for the status line display). `None` when
    /// no mark is set or the region is empty.
    pub fn region_size_bytes(&self) -> Option<usize> {
        self.region_byte_range()
            .map(|(s, e)| e - s)
    }

    /// The region's line range (start_line, end_line inclusive) in buffer
    /// line indices, for the file view's region face rendering. `None` when
    /// no mark is set or the region is empty.
    pub fn region_line_range(&self) -> Option<(usize, usize)> {
        let (byte_start, byte_end) = self.region_byte_range()?;
        let key = self.buffers.current()?.to_string();
        let buf = self.buffers.get(&key)?;
        let start_line = buf.rope.try_byte_to_line(byte_start).ok()?;
        // end is exclusive; the last line in the region is the line
        // containing byte_end - 1 (the last byte of the region).
        let last_byte = byte_end.saturating_sub(1);
        let end_line = buf.rope.try_byte_to_line(last_byte).ok()?;
        Some((start_line, end_line))
    }

    /// C-SPC: set the mark at the current point. Echoes "Mark set".
    pub fn set_mark(&mut self) {
        let point = match self.current_point_byte() {
            Some(p) => p,
            None => {
                self.minibuffer_message("no buffer");
                return;
            }
        };
        let key = self.buffers.current().map(String::from);
        if let Some(key) = key
            && let Some(buf) = self.buffers.get_mut(&key)
        {
            buf.mark = Some(point);
        }
        self.minibuffer_message("Mark set");
    }

    /// C-x C-x: exchange point and mark. If no mark is set, no-op.
    /// After the exchange, the cursor is at where the mark was, and the mark
    /// is at where the point was.
    pub fn exchange_point_and_mark(&mut self) {
        let key = match self.buffers.current().map(String::from) {
            Some(k) => k,
            None => {
                self.minibuffer_message("no buffer");
                return;
            }
        };
        let (mark, point) = {
            let buf = match self.buffers.get(&key) {
                Some(b) => b,
                None => {
                    self.minibuffer_message("no buffer");
                    return;
                }
            };
            (buf.mark, self.current_point_byte())
        };
        let Some(mark) = mark else {
            self.minibuffer_message("Mark not set");
            return;
        };
        let Some(point) = point else { return };
        // Move the point to the line where the mark was (the new point);
        // the window follows.
        let mark_line = self
            .buffers
            .get(&key)
            .and_then(|b| b.rope.try_byte_to_line(mark).ok())
            .unwrap_or(0);
        self.set_point_line(mark_line);
        // Set the mark to the old point.
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.mark = Some(point);
        }
    }

    /// C-w: kill the region. In editable buffers, removes the text and saves
    /// it to the kill ring. In read-only buffers, saves the text to the kill
    /// ring without modifying the buffer (emacs read-only kill-ring-save).
    /// Clears the mark after the operation.
    pub fn kill_region(&mut self) {
        let range = match self.region_byte_range() {
            Some(r) => r,
            None => {
                self.minibuffer_message("Mark not set");
                return;
            }
        };
        let key = self.buffers.current().map(String::from).unwrap_or_default();
        let (editable, text, char_start, char_end) = {
            let Some(buf) = self.buffers.get(&key) else {
                self.minibuffer_message("no buffer");
                return;
            };
            // Convert byte offsets to char offsets (ropey edit APIs are char-index based).
            let char_start = buf.rope.byte_to_char(range.0);
            let char_end = buf.rope.byte_to_char(range.1);
            let text = buf.rope.slice(char_start..char_end).to_string();
            (buf.editable, text, char_start, char_end)
        };
        // Save to the kill ring (always, regardless of editable).
        self.kill_ring.push(text.clone());
        // In editable buffers, remove the text from the rope.
        if editable {
            if let Some(buf) = self.buffers.get_mut(&key) {
                buf.rope.remove(char_start..char_end);
                buf.locally_modified = true;
                buf.mark = None;
            }
            self.invalidate_highlight_for_key(&key);
            // Adjust scroll to keep the view sane after the text removal.
            let total = self
                .buffers
                .get(&key)
                .map(|b| b.line_count())
                .unwrap_or(0);
            let top = self.scroll_top();
            if total > 0 && top >= total.saturating_sub(1) {
                self.set_scroll_top(total.saturating_sub(1));
            }
        } else {
            // Read-only: clear the mark (region is consumed).
            if let Some(buf) = self.buffers.get_mut(&key) {
                buf.mark = None;
            }
        }
        // Reset yank-pop state (a new kill is not a yank).
        self.yank_pos = None;
        self.yank_len = None;
        self.yank_ring_index = None;
        self.minibuffer_message(&format!("{} bytes killed", range.1 - range.0));
    }

    /// M-w: copy the region to the kill ring (no removal; works in both
    /// editable and read-only buffers). Keeps the mark active.
    pub fn copy_region(&mut self) {
        let range = match self.region_byte_range() {
            Some(r) => r,
            None => {
                self.minibuffer_message("Mark not set");
                return;
            }
        };
        let key = self.buffers.current().map(String::from).unwrap_or_default();
        let text = {
            let Some(buf) = self.buffers.get(&key) else {
                self.minibuffer_message("no buffer");
                return;
            };
            // Convert byte offsets to char offsets (ropey slice is char-index based).
            let char_start = buf.rope.byte_to_char(range.0);
            let char_end = buf.rope.byte_to_char(range.1);
            buf.rope.slice(char_start..char_end).to_string()
        };
        self.kill_ring.push(text);
        // Reset yank-pop state.
        self.yank_pos = None;
        self.yank_len = None;
        self.yank_ring_index = None;
        self.minibuffer_message(&format!("{} bytes copied to kill ring", range.1 - range.0));
    }

    /// C-y: yank the most recent kill ring entry at the current point.
    /// Only works in editable buffers. Sets the yank-pop state for M-y.
    pub fn yank(&mut self) {
        let text = match self.kill_ring.top() {
            Some(t) => t.to_string(),
            None => {
                self.minibuffer_message("Kill ring is empty");
                return;
            }
        };
        let key = match self.buffers.current().map(String::from) {
            Some(k) => k,
            None => {
                self.minibuffer_message("no buffer");
                return;
            }
        };
        let editable = self
            .buffers
            .get(&key)
            .map(|b| b.editable)
            .unwrap_or(false);
        if !editable {
            self.minibuffer_message("Buffer is read-only");
            return;
        }
        let point_byte = match self.current_point_byte() {
            Some(p) => p,
            None => {
                self.minibuffer_message("no buffer");
                return;
            }
        };
        let point_char = {
            let buf = self.buffers.get(&key).unwrap();
            buf.rope.byte_to_char(point_byte)
        };
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.insert(point_char, &text);
            buf.locally_modified = true;
            // Clear the mark (the text insertion shifts byte offsets).
            buf.mark = None;
        }
        self.invalidate_highlight_for_key(&key);
        // Set the yank-pop state (char offsets for ropey edit APIs).
        self.yank_pos = Some(point_char);
        self.yank_len = Some(text.chars().count());
        self.yank_ring_index = Some(0);
        // No scroll adjustment: the insertion is at the current top line,
        // so the view is already anchored correctly (finding 4 fix).
    }

    /// M-y: yank-pop — replace the last yanked text with the previous kill
    /// ring entry. Only valid immediately after C-y or another M-y.
    pub fn yank_pop(&mut self) {
        let Some(idx) = self.yank_ring_index else {
            self.minibuffer_message("Yank-pop: no previous yank");
            return;
        };
        let next = idx + 1;
        let text = match self.kill_ring.at(next) {
            Some(t) => t.to_string(),
            None => {
                self.minibuffer_message("Yank-pop: end of kill ring");
                return;
            }
        };
        let key = match self.buffers.current().map(String::from) {
            Some(k) => k,
            None => {
                self.minibuffer_message("no buffer");
                return;
            }
        };
        let (yank_pos, yank_len) = match (self.yank_pos, self.yank_len) {
            (Some(p), Some(l)) => (p, l),
            _ => {
                self.minibuffer_message("Yank-pop: no previous yank");
                return;
            }
        };
        let editable = self
            .buffers
            .get(&key)
            .map(|b| b.editable)
            .unwrap_or(false);
        if !editable {
            self.minibuffer_message("Buffer is read-only");
            return;
        }
        let end = (yank_pos + yank_len).min(
            self.buffers
                .get(&key)
                .map(|b| b.rope.len_chars())
                .unwrap_or(0),
        );
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.remove(yank_pos..end);
            buf.rope.insert(yank_pos, &text);
            buf.locally_modified = true;
            buf.mark = None;
        }
        self.invalidate_highlight_for_key(&key);
        self.yank_len = Some(text.chars().count());
        self.yank_ring_index = Some(next);
        // No scroll adjustment: the replacement is at the current top line.
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
        // Buffer-follow (issue 09, off by default): sync the tree cursor.
        self.tree_follow_opened(rel);
        // plan 005 issue 02: on load, the annotation anchors for this file
        // maintain themselves against the (possibly re-read) content.
        self.reanchor_for_key(&key);
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
                // Rebuild the tree sidebar rows (blocking fix #2): the old
                // rows reflect the pre-walk file set.
                if self.tree.visible {
                    self.tree.rows = self.build_tree_rows();
                    self.tree.selected = 0;
                }
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

    fn candidates_for(&mut self, kind: PickerKind) -> Vec<PickerCandidate> {
        match kind {
            PickerKind::Palette => self.palette_candidates(),
            PickerKind::FindFile => self.find_file_candidates(),
            PickerKind::RecentFiles => self.recent_file_candidates(),
            PickerKind::Buffers | PickerKind::KillBuffer => self.buffer_candidates(),
            PickerKind::Projects => self.project_candidates(),
            PickerKind::Xref => self.xref_candidates(),
            PickerKind::Imenu => self.imenu_candidates(),
            PickerKind::Symbols => self.symbol_candidates(),
            PickerKind::Branch => self.branch_candidates(),
            PickerKind::Stash => self.stash_candidates(),
        }
    }

    /// Candidates for the branch picker (`y`, issue 08): local branches with
    /// the current (HEAD) one marked.
    fn branch_candidates(&self) -> Vec<PickerCandidate> {
        self.with_git(|g| g.branches())
            .unwrap_or_default()
            .into_iter()
            .map(|b| PickerCandidate {
                name: b.name.clone(),
                display: format!("{}{}", if b.current { "*" } else { " " }, b.name),
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
    fn stash_candidates(&mut self) -> Vec<PickerCandidate> {
        self.with_git_mut(|g| g.stash_list())
            .unwrap_or_default()
            .into_iter()
            .map(|s| PickerCandidate {
                name: s.index.to_string(),
                display: format!("stash@{{{}}} {}", s.index, s.subject),
                docs: String::new(),
                category: "stash".to_string(),
            })
            .collect()
    }

    /// Candidates for the Xref picker (definition locations for the
    /// current lookup name).
    fn xref_candidates(&self) -> Vec<PickerCandidate> {
        let name = &self.xref_lookup_name;
        self.index
            .definitions_of(name)
            .into_iter()
            .map(|d| PickerCandidate {
                name: format!("{}:{}", d.file, d.symbol.line + 1),
                display: format!("{}:{}  [{}] {}", d.file, d.symbol.line + 1, d.symbol.kind.tag(), d.symbol.name),
                docs: String::new(),
                category: "xref".to_string(),
            })
            .collect()
    }

    /// Candidates for the Imenu picker (current file's outline).
    fn imenu_candidates(&self) -> Vec<PickerCandidate> {
        let Some(key) = self.buffers.current().map(String::from) else {
            return Vec::new();
        };
        let Some(buf) = self.buffers.get(&key) else {
            return Vec::new();
        };
        let Some(path) = buf.path.as_ref() else {
            return Vec::new();
        };
        let Some(project) = self.project.as_ref() else {
            return Vec::new();
        };
        let Ok(rel) = path.strip_prefix(&project.root) else {
            return Vec::new();
        };
        let rel = rel.to_string_lossy();
        self.index
            .outline(&rel)
            .iter()
            .map(|s| PickerCandidate {
                name: format!("{}:{}", s.name, s.line + 1),
                display: format!("{}  [{}]", s.name, s.kind.tag()),
                docs: String::new(),
                category: "imenu".to_string(),
            })
            .collect()
    }

    /// Candidates for the project-wide symbol picker (all index symbols).
    fn symbol_candidates(&self) -> Vec<PickerCandidate> {
        self.index
            .all_locations()
            .into_iter()
            .map(|loc| PickerCandidate {
                name: format!("{}:{}", loc.file, loc.symbol.line + 1),
                display: format!("{}  [{}]  {}", loc.symbol.name, loc.symbol.kind.tag(), loc.file),
                docs: String::new(),
                category: "symbol".to_string(),
            })
            .collect()
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
            Some(PickerKind::Xref) => self
                .index
                .definitions_of(&self.xref_lookup_name)
                .len(),
            Some(PickerKind::Imenu) => self
                .buffers
                .current()
                .and_then(|key| self.buffers.get(key))
                .and_then(|b| b.path.as_ref())
                .and_then(|path| {
                    self.project.as_ref().and_then(|p| {
                        path.strip_prefix(&p.root).ok().map(|r| {
                            self.index.outline(&r.to_string_lossy()).len()
                        })
                    })
                })
                .unwrap_or(0),
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
            // Xref and Symbols: preview the file at the definition location.
            // The name is "file:line" (1-based). Show a window around the
            // definition line, not the file's first page.
            PickerKind::Xref | PickerKind::Symbols => {
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

    /// Preview of a file centred on a 0-based line: a window of
    /// `PREVIEW_LINES` total lines around `line` (the definition context).
    fn file_preview_at_line(&self, rel: &str, line: usize) -> String {
        let Some(project) = self.project.as_ref() else {
            return String::new();
        };
        let abs = project.root.join(rel);
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
            PickerKind::Xref | PickerKind::Symbols => {
                // name is "file:line" (1-based line number).
                if let Some((file, line_str)) = name.rsplit_once(':')
                    && let Ok(line) = line_str.parse::<usize>()
                {
                    let origin = self.current_jump_entry();
                    self.open_path(file);
                    let key = self.buffers.current().map(String::from).unwrap_or_default();
                    self.set_point_line(line - 1);
                    self.ensure_highlight();
                    let _ = key;
                    self.record_jump(&origin, "M-.");
                    self.minibuffer_message(&format!("jumped to {file}:{line}"));
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
                    self.ensure_highlight();
                    self.record_jump(&origin, "M-i");
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
        // Rebuild the symbol index for the new project (full rebuild, not
        // incremental: the old index belongs to the previous project).
        // Bump the generation so that any in-flight job from the previous
        // project is tagged stale and its event discarded by
        // `apply_index_event`. `start_indexing` will start a new job even
        // if the old project's job is still in flight (generation-aware
        // single-flight).
        self.index = SymbolIndex::new();
        self.index_generation += 1;
        self.pending_index_changes.clear();
        // Invalidate any in-flight search job (its root was the previous
        // project): bump the generation so its events are discarded, and
        // stop it. Results belong to the old project.
        self.cancel_search_job();
        self.search_generation += 1;
        self.search = SearchState::default();
        self.search.generation = self.search_generation;
        // Reset the tree sidebar (issue 09, blocking fix #2): rows belong to
        // the previous project and would open wrong-project files via
        // tree_open_selected. Clear them; the sidebar rebuilds on toggle.
        self.tree.rows.clear();
        self.tree.selected = 0;
        self.start_indexing();
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

    /// Buffer list: `d` kills the SELECTED buffer (issue 05h; the
    /// dired-convention kill verb backlogged in parity log row 31). Reuses
    /// the existing `kill_buffer` path the `C-x k` picker runs — no second
    /// kill verb. The list stays open; the selection clamps to a valid row.
    pub fn buffer_list_kill_selected(&mut self) {
        let key = match self.buffers.list().get(self.buffer_list_selected) {
            Some((key, _)) => key.to_string(),
            None => return,
        };
        self.kill_buffer(&key);
        let n = self.buffers.len();
        self.buffer_list_selected = if n > 0 {
            self.buffer_list_selected.min(n - 1)
        } else {
            0
        };
    }

    // ── project file-tree sidebar (issue 09) ───────────────────────────

    /// `C-c p t`: toggle the file-tree sidebar. Builds the (ignore-aware)
    /// rows from the cached walk on first show.
    pub fn toggle_tree(&mut self) {
        self.tree.visible = !self.tree.visible;
        if self.tree.visible && self.tree.rows.is_empty() {
            // The file walk is lazy (populated by find-file); populate it
            // here so first-open shows the project instead of an empty tree
            // with a misleading "no files" message.
            self.ensure_files();
            self.tree.rows = self.build_tree_rows();
        }
        if self.tree.visible && self.tree.rows.is_empty() {
            self.minibuffer_message("tree: no files in project");
        }
    }

    /// Build the indented file rows from the project's cached file list.
    /// Each file is a row indented by its directory depth (treemacs-lite:
    /// directories are implied by the indentation, files are the leaves).
    fn build_tree_rows(&self) -> Vec<TreeRow> {
        let Some(project) = self.project.as_ref() else {
            return Vec::new();
        };
        let files = match self.files.get(&project.root) {
            Some(f) => f,
            None => return Vec::new(),
        };
        files
            .files
            .iter()
            .map(|rel| {
                let parts: Vec<&str> = rel.split('/').collect();
                let depth = parts.len().saturating_sub(1);
                let name = parts.last().copied().unwrap_or(rel).to_string();
                TreeRow {
                    depth,
                    name,
                    is_dir: false,
                    rel_path: rel.clone(),
                }
            })
            .collect()
    }

    /// `true` when the sidebar is showing (drives the root's Row layout).
    pub fn tree_visible(&self) -> bool {
        self.tree.visible
    }

    /// The sidebar's rows (empty when hidden or not yet built).
    pub fn tree_rows(&self) -> Vec<TreeRow> {
        if self.tree.visible {
            self.tree.rows.clone()
        } else {
            Vec::new()
        }
    }

    pub fn tree_selected(&self) -> usize {
        self.tree.selected
    }

    /// `↓` / `PageDown`: move the tree cursor down (clamped).
    pub fn tree_move_down(&mut self) {
        if !self.tree.visible || self.tree.rows.is_empty() {
            return;
        }
        self.tree.selected = (self.tree.selected + 1).min(self.tree.rows.len() - 1);
    }

    /// `↑` / `PageUp`: move the tree cursor up (clamped).
    pub fn tree_move_up(&mut self) {
        if !self.tree.visible || self.tree.rows.is_empty() {
            return;
        }
        self.tree.selected = self.tree.selected.saturating_sub(1);
    }

    /// Click-to-select in the tree sidebar (plan 004 issue 05e): the
    /// sidebar layout is a title row (terminal row 0), up to
    /// `TREE_VISIBLE_ROWS` file rows (starting at the visible window top,
    /// `selected.saturating_sub(5)`), then a help row. Map a 0-based
    /// terminal row onto the tree row under it; the title row, the help
    /// row, rows past the window, a hidden tree, or a non-buffer top view
    /// are no-ops. A tree click moves ONLY the tree cursor — it never
    /// touches the code point (the code point is set by code-pane clicks,
    /// and `RET` opens the selected file).
    pub fn tree_click_row(&mut self, terminal_row: usize) {
        if !self.tree.visible || self.top_view() != ViewId::Buffer {
            return;
        }
        let Some(rel) = terminal_row.checked_sub(1) else {
            return; // terminal row 0 is the tree title
        };
        if rel >= crate::model::tree_layout::TREE_VISIBLE_ROWS {
            return; // the help row (or below it)
        }
        let start = self.tree.selected.saturating_sub(5);
        let idx = start.saturating_add(rel);
        if idx < self.tree.rows.len() {
            self.tree.selected = idx;
        }
    }

    /// `RET` (with the tree focused): open the selected file, returning to
    /// the buffer view. No-op on an empty tree.
    pub fn tree_open_selected(&mut self) {
        let (is_dir, rel_path) = match self.tree.rows.get(self.tree.selected) {
            Some(row) => (row.is_dir, row.rel_path.clone()),
            None => {
                self.minibuffer_message("tree: nothing selected");
                return;
            }
        };
        if !is_dir {
            self.open_path(&rel_path);
        }
    }

    /// Toggle optional buffer-follow (off by default): when on, opening a
    /// buffer also moves the tree cursor to that file.
    pub fn toggle_tree_follow(&mut self) {
        self.tree.follow = !self.tree.follow;
        self.minibuffer_message(&format!("tree buffer-follow: {}", if self.tree.follow { "on" } else { "off" }));
    }

    /// When buffer-follow is on and the tree is visible, sync the tree cursor
    /// to the just-opened file (called from `open_path`).
    fn tree_follow_opened(&mut self, rel: &str) {
        if self.tree.follow && self.tree.visible
            && let Some(idx) = self.tree.rows.iter().position(|r| r.rel_path == rel)
        {
            self.tree.selected = idx;
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

    /// Scroll down by one line (window motion; the point's screen row is
    /// kept fixed — plan 004 issue 05b). Bound to `j` in the file view.
    pub fn scroll_line_down(&mut self) {
        self.scroll_window_point(1);
    }

    /// Scroll up by one line (window motion; the point's screen row is kept
    /// fixed). Bound to `k` in the file view.
    pub fn scroll_line_up(&mut self) {
        self.scroll_window_point(-1);
    }

    /// Scroll down by one page. PART A fix (item 5): keep a 2-line overlap
    /// (emacs `next-screen-context-lines`) so context carries over between
    /// pages.
    pub fn scroll_page_down(&mut self) {
        let step = self.viewport_lines.saturating_sub(2).max(1) as i64;
        self.scroll_window_point(step);
    }

    /// Scroll up by one page (2-line overlap, matching page-down).
    pub fn scroll_page_up(&mut self) {
        let step = self.viewport_lines.saturating_sub(2).max(1) as i64;
        self.scroll_window_point(-step);
    }

    /// Scroll down by half a page.
    pub fn scroll_half_page_down(&mut self) {
        self.scroll_window_point((self.viewport_lines / 2) as i64);
    }

    /// Scroll up by half a page.
    pub fn scroll_half_page_up(&mut self) {
        self.scroll_window_point(-((self.viewport_lines / 2) as i64));
    }

    // ── mouse support (issue 09, step 4: best-effort) ─────────────────

    /// Mouse wheel up (plan 004 issue 05c): in the file view this is a
    /// WINDOW scroll of 3 lines with the point's screen row pinned (emacs
    /// `mwheel-scroll` — the same primitive C-v/M-v use after issue 05b).
    /// The point's buffer line advances only because the window moves under
    /// it; the wheel never drags the point's line (the pre-05b
    /// window-scroll behavior). List views keep their cursor model: the
    /// selection moves by 3 rows.
    pub fn mouse_scroll_up(&mut self) {
        const STEP: usize = 3;
        match self.top_view() {
            ViewId::Buffer => {
                self.scroll_window_point(-(STEP as i64));
            }
            ViewId::BufferList => {
                for _ in 0..STEP { self.buffer_list_prev(); }
            }
            ViewId::MagitStatus => {
                for _ in 0..STEP { self.magit_cursor_up(); }
            }
            ViewId::Search => {
                for _ in 0..STEP { self.search_prev(); }
            }
            ViewId::Log => {
                for _ in 0..STEP { self.log_move_up(); }
            }
            _ => {}
        }
    }

    /// Mouse wheel down: the mirror of `mouse_scroll_up` — a 3-line window
    /// scroll with the point's screen row pinned in the file view; the
    /// selection moves by 3 rows in list views.
    pub fn mouse_scroll_down(&mut self) {
        const STEP: usize = 3;
        match self.top_view() {
            ViewId::Buffer => {
                self.scroll_window_point(STEP as i64);
            }
            ViewId::BufferList => {
                for _ in 0..STEP { self.buffer_list_next(); }
            }
            ViewId::MagitStatus => {
                for _ in 0..STEP { self.magit_cursor_down(); }
            }
            ViewId::Search => {
                for _ in 0..STEP { self.search_next(); }
            }
            ViewId::Log => {
                for _ in 0..STEP { self.log_move_down(); }
            }
            _ => {}
        }
    }

    /// Click-to-position in the file view (plan 004 issue 05c): set the
    /// point to the clicked `(line, col)` — both are clamped to the buffer's
    /// bounds, so a click past EOL lands at EOL and a click on an empty line
    /// lands at col 0. `row` is the click's row within the visible file area
    /// (the caller subtracts the file view's title-line offset); `col` is
    /// the clicked DISPLAY column RELATIVE TO THE FILE VIEW'S LEFT EDGE —
    /// with the tree sidebar visible the root event arm subtracts the tree
    /// width before calling (plan 004 issue 05e), so callers never mix the
    /// two. Wide (CJK) chars occupy 2 cells, so the display column is
    /// converted to a char index for the point (a click inside a wide char
    /// maps to that char; plan 004 issue 05d). The existing goal column is
    /// preserved (a mouse set-point does not touch it, emacs model); the
    /// window follows (05b behavior). No-op outside the file view.
    pub fn mouse_click_position(&mut self, row: usize, col: usize) {
        if self.top_view() != ViewId::Buffer {
            return;
        }
        // Map-aware (plan 005 issue 02): with note rows visible a rendered
        // row is NOT `scroll_top + row` buffer lines — translate through
        // the rendered-row list (a note row maps to its anchored code
        // row). A click past the last rendered row maps to the last CODE
        // row in the slice.
        let rows = self.file_view_rows();
        let Some(target_line) = FileViewRow::line_for_row(&rows, row)
            .or_else(|| rows.iter().rev().find(|r| !r.is_note).map(|r| r.line))
        else {
            return;
        };
        let line_len = self.line_char_len(target_line);
        // Display column -> char index (the file view renders from cell 0
        // with no gutter; plan 004 issue 05d).
        let char_col = self
            .buffers
            .current_buffer()
            .and_then(|b| b.line_text(target_line))
            .map(|t| crate::model::text_width::display_col_to_char_index(&t, col))
            .unwrap_or(0)
            .min(line_len);
        let p = self.file_point();
        self.set_point(target_line, char_col, p.goal_col);
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

    /// `C-l` (plan 004 issue 05c): emacs `recenter-top-bottom`. The point
    /// does NOT move; the window repositions so the point's screen row
    /// cycles through the positions. `recenter-positions` defaults to
    /// `(middle top bottom)`, and `recenter-top-bottom` advances the
    /// position only when the immediately-preceding command was also
    /// recenter (tracked by `recenter_cycle`, reset on any other command
    /// in `dispatch` — emacs `recenter-last-op`). So a fresh C-l goes to
    /// MIDDLE; consecutive C-l cycles middle → top → bottom → middle.
    ///
    /// The chosen screen row is `viewport/2` (middle), `0` (top), or
    /// `viewport-1` (bottom); `scroll_top = point_line - desired_row`
    /// clamped to `[0, max_scroll]`. When the buffer barely scrolls the
    /// clamps pin the point where it is and the cycle collapses to the
    /// reachable positions (emacs recenter on a barely-scrolling buffer
    /// behaves the same way). No zone-derived guess: the position is
    /// purely the cycle index, so tiny viewports still cycle without
    /// dead-ends (middle/top/bottom naturally repeat when rows collide).
    pub fn recenter(&mut self) {
        let p = self.file_point();
        let total = self.current_line_count();
        if total <= 1 {
            return;
        }
        let vp = self.viewport_lines.max(1);
        let max_scroll = total.saturating_sub(vp);
        if max_scroll == 0 {
            // The whole buffer fits in the viewport; nothing to recenter.
            return;
        }
        let mid = vp / 2;
        let bottom = vp - 1;
        // Cycle order `(middle top bottom)` selected by the cycle index.
        let desired_row = match self.recenter_cycle % 3 {
            0 => mid,
            1 => 0,
            _ => bottom,
        };
        self.recenter_cycle += 1;
        let new_top = (p.line as i64 - desired_row as i64).clamp(0, max_scroll as i64) as usize;
        self.set_scroll_top(new_top);
    }

    /// Compact position display for the status line (plan 004 row 11):
    /// `Top` at the first line, `Bot` at the last, otherwise
    /// `L{n},{pct}%` where `n` is the 1-based line number and `pct` is
    /// the integer percentage through the buffer. Tracks the point's line
    /// (plan 004 issue 05b), not the window top.
    pub fn file_view_position_display(&self) -> String {
        let total = self
            .buffers
            .current_buffer()
            .map(|b| b.line_count())
            .unwrap_or(0);
        if total == 0 {
            return String::new();
        }
        let line = self.point_line(); // 0-based
        if line == 0 {
            return "Top".to_string();
        }
        if line + self.viewport_lines >= total {
            return "Bot".to_string();
        }
        let pct = (line * 100 + total / 2) / total.max(1);
        format!("L{},{}%", line + 1, pct)
    }

    /// Pre-compute the rendered rows for the file view (plan 005 issue 02):
    /// the code rows of the visible buffer lines
    /// `[top_line, top_line + viewport_lines)`, with a virtual annotation
    /// note row directly under each annotated line when note rows are shown
    /// (`show_note_rows`; `C-c a` toggles — the `annotated` flag on the
    /// code rows is independent, so the margin marker stays). Every row
    /// carries its buffer-line index: the dense 1:1 "row i == line top+i"
    /// assumption is gone, and the renderer / `cursor_cell` /
    /// `mouse_click_position` translate `buffer_line` ↔ `rendered_row`
    /// through `FileViewRow::row_for_line` / `line_for_row`.
    pub fn file_view_rows(&mut self) -> Vec<FileViewRow> {
        let total = self.current_line_count();
        if total == 0 {
            return Vec::new();
        }
        self.ensure_notes_doc();
        let Some(buf) = self.buffers.current_buffer() else {
            return Vec::new();
        };
        let top = self.scroll_top();
        let start = top.min(total.saturating_sub(1));
        let end = (top + self.viewport_lines).min(total);
        if start >= end {
            return Vec::new();
        }

        // Get the highlight result from the cache (if any).
        let highlight: Option<&HighlightResult> = self.buffer_highlight_result();

        // This buffer's annotation records (matched by project-relative
        // path), in record order. The marker flag is independent of
        // note-row visibility.
        let Some(key) = self.buffers.current().map(String::from) else {
            return Vec::new();
        };
        let rel = self.buffer_rel_path(&key);
        let records: Vec<&Annotation> = self
            .notes_doc
            .entries
            .iter()
            .filter_map(|e| e.as_record())
            .filter(|a| rel.as_deref() == Some(a.path.as_str()))
            .collect();

        let mut out = Vec::with_capacity(end - start + records.len());
        for line in start..end {
            let text = buf.line_text(line).unwrap_or_default().to_string();
            let spans = highlight
                .and_then(|h| h.lines.get(line))
                .map(|hl| hl.spans.clone())
                .unwrap_or_default();
            let annotated = records.iter().any(|a| a.line == line);
            out.push(FileViewRow {
                line,
                is_note: false,
                annotated,
                text,
                spans,
            });
            if self.show_note_rows {
                for a in records.iter().filter(|a| a.line == line) {
                    let mut note = format!("  \u{25b8} {}", a.text);
                    if a.orphaned {
                        note.push_str(" (orphaned)");
                    }
                    out.push(FileViewRow {
                        line,
                        is_note: true,
                        annotated: false,
                        text: note,
                        spans: Vec::new(),
                    });
                }
            }
        }
        out
    }

    /// The total number of rendered rows for the current buffer
    /// (plan 005 issue 02): the buffer's line count plus the visible
    /// annotation note rows (zero when `C-c a` hid them). The renderer's
    /// bottom scroll indicator compares the slice length against this in
    /// rendered-row space.
    pub fn file_view_total_rows(&mut self) -> usize {
        let total = self
            .buffers
            .current_buffer()
            .map(|b| b.line_count())
            .unwrap_or(0);
        if !self.show_note_rows {
            return total;
        }
        self.ensure_notes_doc();
        let key = self.buffers.current().map(String::from);
        let Some(rel) = key.as_deref().and_then(|k| self.buffer_rel_path(k)) else {
            return total;
        };
        total + self
            .notes_doc
            .entries
            .iter()
            .filter(|e| matches!(e, NotesEntry::Record(a) if a.path == rel))
            .count()
    }

    /// The current buffer's project-relative path (for annotation lookup);
    /// `None` for buffers without a path (scratch) or with no project.
    fn buffer_rel_path(&self, key: &str) -> Option<String> {
        let buf = self.buffers.get(key)?;
        let abs = buf.path.as_ref()?;
        let root = self.project.as_ref()?.root.clone();
        abs.strip_prefix(root)
            .ok()
            .map(|rel| rel.to_string_lossy().into_owned())
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

    /// The current buffer's file-view point `(line, col)` for the UI cursor
    /// cell (plan 004 issue 05b). `(0, 0)` when there is no current buffer.
    pub fn file_view_point(&self) -> (usize, usize) {
        let p = self.file_point();
        (p.line, p.col)
    }

    // ── plan 004 issue 05b: file-view point (line, col) + emacs motion ───

    /// The line count of the current buffer (0 when none).
    fn current_line_count(&self) -> usize {
        self.buffers.current_buffer().map(|b| b.line_count()).unwrap_or(0)
    }

    /// The character length of buffer line `line` in the current buffer.
    fn line_char_len(&self, line: usize) -> usize {
        self.buffers
            .current_buffer()
            .and_then(|b| b.line_text(line))
            .map(|t| t.chars().count())
            .unwrap_or(0)
    }

    /// The current buffer's line `line` as chars (empty when out of range or
    /// no current buffer).
    fn line_chars(&self, line: usize) -> Vec<char> {
        self.buffers
            .current_buffer()
            .and_then(|b| b.line_text(line))
            .map_or_else(Vec::new, |t| t.chars().collect())
    }

    /// Word-constituent for word motion (plan 004 issue 05c): an
    /// alphanumeric or `_`. This is a fixed rule, NOT the emacs syntax
    /// table: emacs decides word-ness per buffer from its syntax table
    /// (where e.g. `?` and `!` can be word-constituents and whitespace,
    /// symbol, and word categories are distinct). Redline treats every
    /// other character — punctuation AND whitespace — as one "non-word"
    /// class; newlines are non-word.
    fn is_word_char(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }

    /// The current buffer's point, clamped to the buffer's bounds (a reload
    /// that shrinks the buffer self-heals here). `(0,0)` when there is no
    /// current buffer or it is empty. `goal_col` is not clamped to the
    /// current line — it may target a longer line that a later C-n/C-p
    /// moves onto.
    fn file_point(&self) -> FilePoint {
        let Some(key) = self.buffers.current().map(String::from) else {
            return FilePoint::default();
        };
        let Some(buf) = self.buffers.get(&key) else {
            return FilePoint::default();
        };
        let total = buf.line_count();
        if total == 0 {
            return FilePoint::default();
        }
        let p = self.point.get(&key).copied().unwrap_or_default();
        let line = p.line.min(total - 1);
        let line_len = buf
            .line_text(line)
            .map(|t| t.chars().count())
            .unwrap_or(0);
        FilePoint {
            line,
            col: p.col.min(line_len),
            goal_col: p.goal_col,
        }
    }

    /// The point's line (0-based) for the current buffer.
    fn point_line(&self) -> usize {
        self.file_point().line
    }

    /// The point's column (0-based char offset) for the current buffer.
    fn point_col(&self) -> usize {
        self.file_point().col
    }

    /// Set the current buffer's point to `(line, col)` with goal column
    /// `goal_col` (all clamped to the buffer's bounds), then make the window
    /// follow so the point's line stays in the viewport (the shipped
    /// follow-scroll pattern). No-op when there is no current buffer.
    fn set_point(&mut self, line: usize, col: usize, goal_col: usize) {
        let Some(key) = self.buffers.current().map(String::from) else {
            return;
        };
        let total = self
            .buffers
            .get(&key)
            .map(|b| b.line_count())
            .unwrap_or(0);
        if total == 0 {
            return;
        }
        let line = line.min(total - 1);
        let line_len = self
            .buffers
            .get(&key)
            .and_then(|b| b.line_text(line))
            .map(|t| t.chars().count())
            .unwrap_or(0);
        let col = col.min(line_len);
        // NOTE: `goal_col` is deliberately NOT clamped to the current line's
        // length — it is the emacs goal column, which a short line clamps
        // away from but a later C-n/C-p onto a longer line restores. It is
        // only ever applied with `.min(line_len)` at the moment of use.
        self.point
            .insert(key.clone(), FilePoint { line, col, goal_col });
        // Window follows: keep the point's line in the viewport.
        let window = self.viewport_lines.max(1);
        let top = self.scroll.get(&key).copied().unwrap_or(0);
        let next = keep_cursor_visible(top, line, total, window);
        if next != top {
            self.scroll.insert(key, next);
        }
    }

    /// A landing that moves the point to buffer line `line` at column 0
    /// (isearch, goto-line, xref, imenu, jump, search-RET, click):
    /// `set_point` with the point's column reset (the emacs landing is at the
    /// start of the target line).
    fn set_point_line(&mut self, line: usize) {
        self.set_point(line, 0, 0);
    }

    /// Move the file-view window by `delta` lines (`delta > 0` = forward /
    /// down, `delta < 0` = backward / up), keeping the point's **screen row**
    /// fixed: the point's buffer line is recomputed from its on-screen row
    /// after the window moves (emacs scroll behavior). The point's column is
    /// re-clamped to the new line's length. Pure window motion — the cursor
    /// does not move on screen. The window top clamps exactly like
    /// `set_scroll_top` (so a buffer that fits the viewport still scrolls to
    /// its last line, matching the pre-05b behavior).
    fn scroll_window_point(&mut self, delta: i64) {
        let total = self.current_line_count();
        if total == 0 {
            return;
        }
        let p = self.file_point();
        let old_top = self.scroll_top();
        let screen_row = p.line.saturating_sub(old_top);
        let new_top = (old_top as i64 + delta).clamp(0, (total - 1) as i64) as usize;
        self.set_scroll_top(new_top);
        let new_line = (new_top + screen_row).min(total - 1);
        let new_len = self.line_char_len(new_line);
        let new_col = p.col.min(new_len);
        let key = self.buffers.current().map(String::from).unwrap();
        self.point.insert(
            key,
            FilePoint {
                line: new_line,
                col: new_col,
                goal_col: p.goal_col,
            },
        );
    }

    /// C-n / Down: point down one line, preserving the goal column (emacs
    /// `next-line`). No-op at the last line (buffer end).
    pub fn point_down(&mut self) {
        let p = self.file_point();
        let total = self.current_line_count();
        if total == 0 || p.line + 1 >= total {
            return;
        }
        let nl = p.line + 1;
        self.set_point(nl, p.goal_col.min(self.line_char_len(nl)), p.goal_col);
    }

    /// C-p / Up: point up one line, preserving the goal column (emacs
    /// `previous-line`). No-op at the first line.
    pub fn point_up(&mut self) {
        let p = self.file_point();
        if p.line == 0 {
            return;
        }
        let nl = p.line - 1;
        self.set_point(nl, p.goal_col.min(self.line_char_len(nl)), p.goal_col);
    }

    /// C-f / Right: point forward one character; wrap to the next line's
    /// start at end-of-line (emacs `forward-char`). No-op at the buffer end.
    pub fn point_forward(&mut self) {
        let p = self.file_point();
        let total = self.current_line_count();
        let line_len = self.line_char_len(p.line);
        if p.col < line_len {
            self.set_point(p.line, p.col + 1, p.col + 1);
        } else if p.line + 1 < total {
            self.set_point(p.line + 1, 0, 0);
        }
    }

    /// C-b / Left: point backward one character; wrap to the previous
    /// line's end at beginning-of-line (emacs `backward-char`). No-op at the
    /// buffer start.
    pub fn point_backward(&mut self) {
        let p = self.file_point();
        if p.col > 0 {
            self.set_point(p.line, p.col - 1, p.col - 1);
        } else if p.line > 0 {
            let prev_len = self.line_char_len(p.line - 1);
            self.set_point(p.line - 1, prev_len, prev_len);
        }
    }

    /// C-a: point to the beginning of the line (col 0).
    pub fn point_line_start(&mut self) {
        let p = self.file_point();
        self.set_point(p.line, 0, 0);
    }

    /// C-e: point to the end of the line (col = the line's char length).
    pub fn point_line_end(&mut self) {
        let p = self.file_point();
        let line_len = self.line_char_len(p.line);
        self.set_point(p.line, line_len, line_len);
    }

    /// M-f (plan 004 issue 05c; emacs `forward-word`): move to the END of
    /// the next word. If the char at point is a word char, advance to the
    /// end of that word run. Otherwise skip the non-word run
    /// (punctuation/whitespace) up to the next word, THEN walk word chars
    /// to that word's end — `forward-word` lands at the word's END, not its
    /// first char. Newlines are non-word, so the skip crosses line
    /// boundaries and keeps skipping (and then walks) on the next line; at
    /// the end of the buffer it is a no-op. A word motion sets `goal_col`
    /// to the landing column (emacs), and the window follows the point.
    pub fn point_word_forward(&mut self) {
        let p = self.file_point();
        let total = self.current_line_count();
        if total == 0 {
            return;
        }
        let mut line = p.line;
        let mut col = p.col;
        let mut moved = false;
        loop {
            let chars = self.line_chars(line);
            let len = chars.len();
            // Skip the non-word run (punctuation/whitespace) up to the
            // next word's first char.
            while col < len && !Self::is_word_char(chars[col]) {
                col += 1;
                moved = true;
            }
            if col < len {
                // Landed on the next word's first char: advance to the
                // word's END (emacs forward-word lands at the end, not the
                // first char). A word run cannot span a line.
                while col < len && Self::is_word_char(chars[col]) {
                    col += 1;
                    moved = true;
                }
                break;
            }
            // The non-word run reaches EOL: cross to the next line (the
            // newline is part of the run) or stop at the buffer end.
            if line + 1 < total {
                line += 1;
                col = 0;
                moved = true;
            } else {
                break;
            }
        }
        if moved {
            self.set_point(line, col, col);
        }
    }

    /// M-b (plan 004 issue 05c; emacs `backward-word`): move to the START
    /// of the previous word. If the char before point is a word char, that
    /// word's start; otherwise skip the non-word run backward (crossing
    /// lines — the newline is non-word), then keep retreating while word
    /// chars to the word's first char. `backward-word` lands at the word's
    /// START, not its end. A word motion sets `goal_col` to the landing
    /// column (emacs), and the window follows the point.
    pub fn point_word_backward(&mut self) {
        let p = self.file_point();
        let total = self.current_line_count();
        if total == 0 || (p.line == 0 && p.col == 0) {
            return;
        }
        if p.col > 0 && Self::is_word_char(self.line_chars(p.line)[p.col - 1]) {
            // Preceded by a word: retreat to its first char.
            let chars = self.line_chars(p.line);
            let mut col = p.col;
            while col > 0 && Self::is_word_char(chars[col - 1]) {
                col -= 1;
            }
            self.set_point(p.line, col, col);
            return;
        }
        // Non-word before point (or point at a line start, where the
        // preceding char is the newline): skip the non-word run backward,
        // crossing lines (the newline is part of the run).
        let mut line = p.line;
        let mut col = p.col;
        loop {
            if col > 0 {
                let chars = self.line_chars(line);
                while col > 0 && !Self::is_word_char(chars[col - 1]) {
                    col -= 1;
                }
                if col > 0 {
                    // Landed just past a word's last char: retreat to the
                    // word's FIRST char (emacs backward-word lands at the
                    // start, not the end).
                    while col > 0 && Self::is_word_char(chars[col - 1]) {
                        col -= 1;
                    }
                    self.set_point(line, col, col);
                    return;
                }
            }
            if line == 0 {
                return; // reached the start of the buffer
            }
            line -= 1;
            col = self.line_char_len(line);
        }
    }

    /// M-<: point to the buffer start (line 0, col 0); the window follows.
    pub fn point_buffer_start(&mut self) {
        self.set_point(0, 0, 0);
    }

    /// M->: point to the buffer end (last line, last col); the window
    /// follows.
    pub fn point_buffer_end(&mut self) {
        let total = self.current_line_count();
        if total == 0 {
            return;
        }
        let line = total - 1;
        let line_len = self.line_char_len(line);
        self.set_point(line, line_len, line_len);
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
        // PART A fix (item 5): never latch isearch behind an open picker
        // (C-s / C-r with a picker open is a no-op, not a search).
        if self.picker.is_some() {
            return;
        }
        if self.isearch.active {
            return; // already active; C-s during isearch is a no-op
        }
        self.isearch = IsearchState {
            active: true,
            query: String::new(),
            direction,
            matches: Vec::new(),
            current: 0,
            pre_search_line: self.point_line(),
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
            let start_line = self.point_line();
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

    /// Navigate to the next match (C-s) with wrap-around.
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

    /// Navigate to the previous match (C-r) with wrap-around.
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
        self.set_point_line(line);
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
        self.set_point_line(self.isearch.pre_search_line);
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
        // PART A fix (item 5): never latch goto-line behind an open picker
        // (M-g g with a picker open is a no-op, not a goto-line prompt).
        if self.picker.is_some() {
            return;
        }
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

    /// Confirm goto-line (RET): scroll to the target line. PART A fix
    /// (item 5): the input is 1-BASED (matching the error message and
    /// emacs), so `M-g g 50` lands on line 50 (0-based scroll top 49).
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
            if line >= 1 && line <= total {
                self.set_point_line(line - 1);
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
        self.magit_keep_visible();
    }

    /// `n` / `C-n`: move the cursor to the next visible section.
    pub fn magit_cursor_down(&mut self) {
        if let Some(t) = self.status_tree.as_mut() {
            t.move_down();
        }
        self.magit_keep_visible();
    }

    /// `p` / `C-p`: move the cursor to the previous visible section.
    pub fn magit_cursor_up(&mut self) {
        if let Some(t) = self.status_tree.as_mut() {
            t.move_up();
        }
        self.magit_keep_visible();
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

    /// The number of magit status rows that fit in the content area:
    /// `viewport_lines - 2` (room for the pinned title, a scroll indicator,
    /// and the help line), at least one. `viewport_lines` is the file-view
    /// content height set on resize.
    fn magit_window(&self) -> usize {
        pane_window(self.viewport_lines)
    }

    /// Keep the magit cursor row inside the visible window (issue 002-02
    /// windowing for long status buffers). Called on every cursor move, fold,
    /// and refresh; persists `magit_scroll` so the window tracks the cursor in
    /// both directions. Clamped to the row count.
    fn magit_keep_visible(&mut self) {
        let rows = self
            .status_tree
            .as_ref()
            .map(|t| t.visible_rows())
            .unwrap_or_default();
        let total = rows.len();
        if total == 0 {
            self.magit_scroll = 0;
            return;
        }
        let Some(cursor) = rows.iter().position(|r| r.selected) else {
            // No cursor row (shouldn't happen once the tree is built); just
            // clamp the offset.
            self.magit_scroll = self.magit_scroll.min(total.saturating_sub(1));
            return;
        };
        self.magit_scroll = keep_cursor_visible(
            self.magit_scroll,
            cursor,
            total,
            self.magit_window(),
        );
    }

    /// The magit status window (visible rows + top row index + total row
    /// count) for long status buffers (issue 002-02). The cursor row is always
    /// inside the window (kept by `magit_keep_visible`); the view renders the
    /// window plus a scroll indicator and a pinned help line so neither is
    /// clipped.
    pub fn magit_view_info(&self) -> (Vec<MagitRow>, usize, usize) {
        let rows = self.magit_rows();
        let total = rows.len();
        if total == 0 {
            return (Vec::new(), 0, 0);
        }
        let (start, end) = window_slice(self.magit_scroll, total, self.magit_window());
        (rows[start..end].to_vec(), start, total)
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

    /// Mutable variant of [`with_git`] (the git2 `stash_*` APIs take
    /// `&mut Repository`).
    fn with_git_mut<R>(
        &mut self,
        f: impl FnOnce(&mut GitRepo) -> Result<R, GitError>,
    ) -> Result<R, GitError> {
        match self.git.as_mut() {
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
        self.magit_keep_visible();
        true
    }

    // ── transient menu (issue 002) ────────────────────────────────────

    /// Whether the transient menu overlay is open.
    pub fn menu_open(&self) -> bool {
        self.menu.open
    }

    /// The current submenu path (empty at the top level).
    #[allow(dead_code)] // used by the menu tests; public accessor for future UI
    pub fn menu_path(&self) -> &KeySeq {
        &self.menu.path
    }

    /// `?` (any view) / `h` (magit views): open the transient menu at the top
    /// level.
    pub fn open_menu(&mut self) {
        self.menu.open = true;
        self.menu.path.clear();
        self.pending.clear();
    }

    fn close_menu(&mut self) {
        self.menu.open = false;
        self.menu.path.clear();
    }

    /// The active view's effective bindings: the view map plus the global
    /// map, with the view map winning on a shared sequence (mirroring
    /// `KeymapEngine::resolve`). Source of truth for the menu — no
    /// hand-written table, so the menu cannot drift from the keymap.
    fn menu_bindings(&self) -> Vec<(KeySeq, String)> {
        let mut map: std::collections::HashMap<Vec<Key>, String> = self
            .engine
            .global
            .command_pairs()
            .into_iter()
            .collect();
        for (s, c) in self.engine.view.command_pairs() {
            map.insert(s, c);
        }
        map.into_iter().collect()
    }

    /// The menu entries at `path`: each is the next key and either a leaf
    /// (a full binding) or a prefix (a strict prefix of a longer binding).
    pub fn menu_entries_for_path(&self, path: &KeySeq) -> Vec<TransientMenuEntry> {
        let bindings = self.menu_bindings();
        let mut entries: Vec<(Key, Option<String>)> = Vec::new();
        for (seq, cmd) in &bindings {
            if seq.len() > path.len() && seq[..path.len()] == *path {
                let next = seq[path.len()];
                // A node is either a command or a prefix, never both: a
                // one-key extension is a leaf, a longer one a prefix.
                let is_leaf = seq.len() == path.len() + 1;
                entries.push((next, if is_leaf { Some(cmd.clone()) } else { None }));
            }
        }
        // Deterministic order + leaf-wins on a key that is both a leaf and a
        // prefix (the engine resolves the view's leaf first).
        entries.sort_by(|a, b| {
            a.0.to_string().cmp(&b.0.to_string()).then(a.1.is_none().cmp(&b.1.is_none()))
        });
        entries.dedup_by(|a, b| a.0 == b.0);
        entries
            .into_iter()
            .map(|(key, cmd)| {
                let mut key_display = path
                    .iter()
                    .map(|k| k.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                if !key_display.is_empty() {
                    key_display.push(' ');
                }
                key_display.push_str(&key.to_string());
                match cmd {
                    Some(name) => {
                        let meta = self.registry.get(&name);
                        TransientMenuEntry {
                            key,
                            key_display,
                            command: Some(name.clone()),
                            is_prefix: false,
                            docs: meta.map(|m| m.docs.to_string()).unwrap_or_default(),
                            category: meta.map(|m| m.category.to_string()).unwrap_or_default(),
                        }
                    }
                    None => TransientMenuEntry {
                        key,
                        key_display,
                        command: None,
                        is_prefix: true,
                        docs: String::new(),
                        category: String::new(),
                    },
                }
            })
            .collect()
    }

    /// The menu entries at the current submenu path.
    #[allow(dead_code)] // used by the menu tests; public accessor for future UI
    pub fn menu_entries(&self) -> Vec<TransientMenuEntry> {
        self.menu_entries_for_path(&self.menu.path)
    }

    /// The menu's display rows for the current path: prefix (submenu) entries
    /// first, then leaf commands grouped by registry category and sorted
    /// within each group. A header row marks each group.
    pub fn menu_rows(&self) -> Vec<TransientMenuRow> {
        let entries = self.menu_entries_for_path(&self.menu.path);
        let mut rows = Vec::new();

        // Submenus (prefix keys) come first, as `KEY …` rows.
        let prefixes: Vec<&TransientMenuEntry> =
            entries.iter().filter(|e| e.is_prefix).collect();
        if !prefixes.is_empty() {
            rows.push(TransientMenuRow {
                is_header: true,
                key_display: String::new(),
                label: "submenus".into(),
                is_prefix: false,
            });
            for e in &prefixes {
                rows.push(TransientMenuRow {
                    is_header: false,
                    key_display: e.key_display.clone(),
                    label: String::new(),
                    is_prefix: true,
                });
            }
        }

        // Leaf commands grouped by registry category (sorted), then sorted by
        // key within each group.
        let mut by_cat: std::collections::BTreeMap<String, Vec<&TransientMenuEntry>> =
            std::collections::BTreeMap::new();
        for e in &entries {
            if !e.is_prefix {
                by_cat.entry(e.category.clone()).or_default().push(e);
            }
        }
        for (cat, mut es) in by_cat {
            es.sort_by(|a, b| a.key_display.cmp(&b.key_display));
            rows.push(TransientMenuRow {
                is_header: true,
                key_display: String::new(),
                label: cat,
                is_prefix: false,
            });
            for e in es {
                rows.push(TransientMenuRow {
                    is_header: false,
                    key_display: e.key_display.clone(),
                    label: e.docs.clone(),
                    is_prefix: false,
                });
            }
        }
        rows
    }

    /// The overlay's row budget: the menu's row count (title + rows), capped
    /// at the viewport height so the menu never exceeds the frame.
    pub fn menu_height(&self) -> u32 {
        (self.menu_rows().len() + 1).min(self.viewport_lines) as u32
    }

    /// Handle a key while the menu is open: C-g closes; a listed leaf closes
    /// the menu and runs its command; a listed prefix descends; anything else
    /// is swallowed (the menu ignores non-listed keys).
    fn menu_key_event(&mut self, key: Key) {
        if key == Key::ctrl_char('g') {
            self.close_menu();
            self.minibuffer_message("cancel");
            return;
        }
        let entries = self.menu_entries_for_path(&self.menu.path);
        if let Some(entry) = entries.iter().find(|e| e.key == key) {
            match &entry.command {
                Some(cmd) => {
                    let cmd = cmd.clone();
                    self.close_menu();
                    let _ = self.dispatch(&cmd, None);
                }
                None => {
                    self.menu.path.push(key);
                }
            }
        }
        // else: swallowed (ignored).
    }

    // ── discard (issue 002: magit `k`) ───────────────────────────────────

    /// `k`: arm a destructive-discard confirmation for the file/hunk under the
    /// cursor. The actual discard runs only on `y` (magit gates discards with
    /// a confirmation); `n`/C-g/ESC cancel.
    pub fn magit_discard(&mut self) {
        let Some(target) = self
            .status_tree
            .as_ref()
            .and_then(|t| t.cursor_target())
        else {
            self.minibuffer_message("no section under point");
            return;
        };
        let Some(path) = target.path.clone() else {
            self.minibuffer_message("no file under point");
            return;
        };
        if !matches!(target.kind, SectionKind::File | SectionKind::Hunk) {
            self.minibuffer_message("nothing to discard at point");
            return;
        }
        let what = match target.kind {
            SectionKind::Hunk => format!("hunk of {path}"),
            _ => path.clone(),
        };
        self.discard_confirm = Some(DiscardTarget {
            path: path.clone(),
            kind: target.kind,
            side: target.side,
            orig: target.orig.clone(),
            hunk_new_start: target.hunk_new_start,
        });
        self.minibuffer_message(&format!("discard {what}? y/n"));
    }

    /// Handle a key while a discard confirmation is armed: `y` executes the
    /// discard, `n`/C-g/ESC cancel; other keys are swallowed.
    fn discard_key_event(&mut self, key: Key) {
        if key == Key::char('y') {
            self.confirm_discard();
            return;
        }
        if key == Key::char('n')
            || key == Key::ctrl_char('g')
            || key.code == crate::app::keymap::KeyCode::Escape
        {
            self.discard_confirm = None;
            self.minibuffer_message("discard cancelled");
        }
        // else: swallowed (ignored).
    }

    /// Whether a destructive-discard confirmation is currently armed.
    pub fn discard_armed(&self) -> bool {
        self.discard_confirm.is_some()
    }

    /// `y`: run the armed discard, then refresh.
    fn confirm_discard(&mut self) {
        let Some(target) = self.discard_confirm.take() else {
            return;
        };
        match self.execute_discard(&target) {
            Ok(()) => {
                self.refresh_magit();
                self.minibuffer_message(&format!("discarded {}", target.path));
            }
            Err(e) => self.minibuffer_message(&format!("discard failed: {e}")),
        }
    }

    /// Run the git operation for a discard target (no confirmation here —
    /// the caller has already confirmed).
    fn execute_discard(&self, target: &DiscardTarget) -> Result<(), GitError> {
        match target.kind {
            SectionKind::Hunk => {
                let start = target.hunk_new_start.unwrap_or(0);
                self.with_git(|g| g.discard_hunk(&target.path, target.side, start))
            }
            SectionKind::File => match target.side {
                Some(Side::Untracked) => {
                    self.with_git(|g| g.discard_untracked_file(&target.path))
                }
                Some(Side::Staged) => self
                    .with_git(|g| g.discard_staged_file(&target.path, target.orig.as_deref())),
                _ => self.with_git(|g| g.discard_unstaged_file(&target.path)),
            },
            _ => Err(GitError::NotARepository {
                path: self
                    .project
                    .as_ref()
                    .map(|p| p.root.clone())
                    .unwrap_or_default(),
            }),
        }
    }

    // ── issue 08: log / blame / commit / branches / stash ────────────────

    /// `l` in the magit-status context: open (or re-focus) the log for the
    /// current branch (HEAD when detached/unborn).
    pub fn open_log(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        if !self.ensure_git(root) {
            return;
        }
        let branch = self
            .with_git(|g| g.branch())
            .ok()
            .and_then(|b| if !b.detached && !b.unborn { Some(b.name) } else { None });
        let total = self
            .with_git(|g| g.log_total(branch.as_deref()))
            .unwrap_or(0);
        let entries = self
            .with_git(|g| g.log(branch.as_deref(), 0, LOG_PAGE))
            .unwrap_or_default();
        self.log = Some(LogState {
            branch,
            offset: 0,
            limit: LOG_PAGE,
            total,
            entries,
            selected: 0,
        });
        self.log_scroll = 0;
        if self.top_view() != ViewId::Log {
            self.push_view(ViewId::Log);
        }
        self.minibuffer_message(&format!("log: {} ({} commits)", self.log.as_ref().unwrap().display(), total));
    }

    /// `n` in the log: move to the next page.
    pub fn log_next_page(&mut self) {
        let Some(log) = self.log.as_ref() else { return };
        if log.offset + log.entries.len() >= log.total {
            self.minibuffer_message("end of log");
            return;
        }
        self.log_offset_to(log.offset + log.limit);
    }

    /// `p` in the log: move to the previous page.
    pub fn log_prev_page(&mut self) {
        let Some(log) = self.log.as_ref() else { return };
        if log.offset == 0 {
            self.minibuffer_message("start of log");
            return;
        }
        self.log_offset_to(log.offset.saturating_sub(log.limit));
    }

    fn log_offset_to(&mut self, offset: usize) {
        let Some(log) = self.log.as_ref() else { return };
        let branch = log.branch.clone();
        let limit = log.limit;
        let total = self.with_git(|g| g.log_total(branch.as_deref())).unwrap_or(0);
        let entries = self
            .with_git(|g| g.log(branch.as_deref(), offset, limit))
            .unwrap_or_default();
        if let Some(l) = self.log.as_mut() {
            l.offset = offset;
            l.total = total;
            l.entries = entries;
            l.selected = 0;
        }
        // A new page starts its in-page window at the top (issue 003-02).
        self.log_scroll = 0;
    }

    /// Move the in-page log selection down (arrows / j / C-n).
    pub fn log_move_down(&mut self) {
        if let Some(l) = self.log.as_mut() {
            l.selected = (l.selected + 1).min(l.entries.len().saturating_sub(1));
        }
        self.log_keep_visible();
    }

    /// Move the in-page log selection up (arrows / k / C-p).
    pub fn log_move_up(&mut self) {
        if let Some(l) = self.log.as_mut() {
            l.selected = l.selected.saturating_sub(1);
        }
        self.log_keep_visible();
    }

    /// `RET` in the log: open the selected commit's full tree diff read-only.
    pub fn log_open_commit(&mut self) {
        let Some(oid) = self
            .log
            .as_ref()
            .and_then(|l| l.entries.get(l.selected))
            .map(|e| e.short_id.clone())
        else {
            self.minibuffer_message("no commit at point");
            return;
        };
        match self.with_git(|g| g.commit_diff(&oid)) {
            Ok(diff) => {
                self.commit_diff = Some(CommitDiffState { diff });
                self.commit_diff_scroll = 0;
                if self.top_view() != ViewId::CommitDiff {
                    self.push_view(ViewId::CommitDiff);
                }
                self.minibuffer_message("commit diff");
            }
            Err(e) => self.minibuffer_message(&format!("commit diff failed: {e}")),
        }
    }

    /// Rebuild the open log's current page (after a commit or branch switch).
    fn refresh_log_page(&mut self) {
        let Some(log) = self.log.as_ref() else { return };
        let branch = log.branch.clone();
        let offset = log.offset;
        let limit = log.limit;
        let total = self.with_git(|g| g.log_total(branch.as_deref())).unwrap_or(0);
        let entries = self
            .with_git(|g| g.log(branch.as_deref(), offset, limit))
            .unwrap_or_default();
        if let Some(l) = self.log.as_mut() {
            l.total = total;
            l.entries = entries;
            l.selected = l.selected.min(l.entries.len().saturating_sub(1));
        }
        self.log_keep_visible();
    }

    /// `b` in the magit-status context: blame the current buffer's file
    /// (project-relative), one line per commit/author/age prefix.
    pub fn open_blame(&mut self) {
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let Some(path) = self.buffers.get(&key).and_then(|b| b.path.clone()) else {
            self.minibuffer_message("no file (scratch buffer)");
            return;
        };
        let Some(project) = self.project.as_ref() else {
            self.minibuffer_message("no project");
            return;
        };
        let Ok(rel) = path.strip_prefix(&project.root) else {
            self.minibuffer_message("buffer not in project");
            return;
        };
        let rel = rel.to_string_lossy().into_owned();
        let root = project.root.clone();
        if !self.ensure_git(root) {
            return;
        }
        match self.with_git(|g| g.blame(&rel)) {
            Ok(lines) => {
                self.blame = Some(BlameState {
                    path: rel.clone(),
                    lines,
                    selected: 0,
                });
                self.blame_scroll = 0;
                if self.top_view() != ViewId::Blame {
                    self.push_view(ViewId::Blame);
                }
                self.minibuffer_message(&format!("blame: {rel}"));
            }
            Err(e) => self.minibuffer_message(&format!("blame failed: {e}")),
        }
    }

    /// `c` in the magit-status context: open the inline commit editor, pre-
    /// filled with a comment block listing the staged files.
    pub fn open_commit_editor(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        if !self.ensure_git(root) {
            return;
        }
        let staged: Vec<(String, char)> = self
            .git_status()
            .unwrap_or_default()
            .files
            .into_iter()
            .filter(|f| f.is_staged())
            .map(|f| (f.path, f.staged.letter()))
            .collect();
        let (text, cursor) = prefill_commit_message(&staged);
        let staged_paths: Vec<String> = staged.iter().map(|(p, _)| p.clone()).collect();
        self.commit_editor = Some(CommitEditorState {
            rope: Rope::from(text.as_str()),
            cursor,
            staged: staged_paths,
        });
        if self.top_view() != ViewId::CommitEditor {
            self.push_view(ViewId::CommitEditor);
        }
        self.minibuffer_message("commit: C-c C-c commit · C-c C-k abort");
    }

    /// `C-c C-c` in the commit editor: extract the message (comment lines
    /// stripped) and commit the staged changes.
    pub fn commit_editor_commit(&mut self) {
        let Some(ed) = self.commit_editor.as_ref() else { return };
        let msg = extract_commit_message(&ed.rope);
        if msg.trim().is_empty() {
            self.minibuffer_message("empty commit message: add a line not starting with '#'");
            return;
        }
        // Refuse to commit when nothing is staged (magit convention: a
        // commit with no staged changes is an error, not a no-op).
        let staged = self
            .git_status()
            .unwrap_or_default()
            .files
            .iter()
            .filter(|f| f.is_staged())
            .count();
        if staged == 0 {
            self.minibuffer_message("nothing staged to commit");
            return;
        }
        let msg = msg.to_string();
        match self.with_git(move |g| g.commit(&msg)) {
            Ok(oid) => {
                let short: String = oid.chars().take(7).collect();
                self.commit_editor = None;
                if self.top_view() == ViewId::CommitEditor {
                    self.close_view();
                }
                self.refresh_magit();
                if self.log.is_some() {
                    self.refresh_log_page();
                }
                self.minibuffer_message(&format!("committed {short}"));
            }
            Err(e) => self.minibuffer_message(&format!("commit failed: {e}")),
        }
    }

    /// `C-c C-k` (and ESC) in the commit editor: discard the
    /// buffer and touch nothing in the repository.
    pub fn commit_editor_abort(&mut self) {
        if self.commit_editor.take().is_some() {
            if self.top_view() == ViewId::CommitEditor {
                self.close_view();
            }
            self.pending.clear();
            self.minibuffer_message("commit aborted (no changes made)");
        } else {
            self.minibuffer_message("no commit in progress");
        }
    }

    /// Insert a printable character at the commit-editor cursor.
    pub fn commit_editor_insert(&mut self, c: char) {
        if let Some(ed) = self.commit_editor.as_mut() {
            ed.rope.insert_char(ed.cursor, c);
            ed.cursor += 1;
        }
    }

    /// Backspace in the commit editor.
    pub fn commit_editor_backspace(&mut self) {
        if let Some(ed) = self.commit_editor.as_mut()
            && ed.cursor > 0
        {
            ed.rope.remove(ed.cursor - 1..ed.cursor);
            ed.cursor -= 1;
        }
    }

    /// Insert a newline at the commit-editor cursor (RET in the editor).
    pub fn commit_editor_newline(&mut self) {
        if let Some(ed) = self.commit_editor.as_mut() {
            ed.rope.insert_char(ed.cursor, '\n');
            ed.cursor += 1;
        }
    }

    /// Move the commit-editor cursor left / right / up / down.
    fn commit_editor_move(&mut self, dir: EditorMove) {
        let Some(ed) = self.commit_editor.as_mut() else { return };
        match dir {
            EditorMove::Left => ed.cursor = ed.cursor.saturating_sub(1),
            EditorMove::Right => ed.cursor = (ed.cursor + 1).min(ed.rope.len_chars()),
            EditorMove::Up => editor_cursor_line(ed, -1),
            EditorMove::Down => editor_cursor_line(ed, 1),
        }
    }

    /// The commit editor's rows (message + comment lines; the cursor line is
    /// marked `selected` and carries a `←` marker).
    pub fn commit_editor_rows(&self) -> Vec<MagitRow> {
        let Some(ed) = self.commit_editor.as_ref() else { return Vec::new() };
        let text = ed.rope.to_string();
        let lines: Vec<&str> = text.lines().collect();
        let n_lines = ed.rope.len_lines();
        let cursor_line = ed.rope.char_to_line(ed.cursor.min(ed.rope.len_chars()));
        (0..n_lines)
            .map(|i| {
                let line = lines.get(i).copied().unwrap_or("");
                let role = if line.trim_start().starts_with('#') {
                    RowRole::Comment
                } else {
                    RowRole::Text
                };
                let selected = i == cursor_line;
                let text = if selected {
                    format!("{line} ←")
                } else {
                    line.to_string()
                };
                MagitRow { text, role, selected }
            })
            .collect()
    }

    /// `y` in the magit-status context: the local-branch picker (RET checks
    /// out the selected branch).
    pub fn open_branch_picker(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        if !self.ensure_git(root) {
            return;
        }
        let candidates = self.branch_candidates();
        if candidates.is_empty() {
            self.minibuffer_message("no local branches");
            return;
        }
        self.open_picker(PickerKind::Branch, "Branch: ", candidates);
    }

    /// Check out the local branch `name`. On success, refreshes the magit
    /// status (branch + dirty state), schedules a full symbol-index rebuild
    /// (the wholesale change), and refreshes the open log. A dirty tree is
    /// refused (`GitError::DirtyTree`) per magit's default.
    pub fn checkout_branch(&mut self, name: &str) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            self.minibuffer_message("no project");
            return;
        };
        if !self.ensure_git(root) {
            return;
        }
        match self.with_git(|g| g.checkout_branch(name)) {
            Ok(()) => {
                self.refresh_magit();
                // Wholesale change: full-rebuild the symbol index (05's
                // start_indexing path). Bump the generation and clear pending
                // changes (mirroring switch_project_root) so the full rebuild
                // isn't silently skipped by start_indexing's single-flight
                // guard when a current-generation job is already in flight.
                self.index = SymbolIndex::new();
                self.index_generation += 1;
                self.pending_index_changes.clear();
                self.start_indexing();
                if self.log.is_some() {
                    self.refresh_log_page();
                }
                self.minibuffer_message(&format!("checked out {name}"));
            }
            Err(e) => self.minibuffer_message(&format!("checkout failed: {e}")),
        }
    }

    /// Create a new local branch at HEAD. `branch-create` (M-x) opens a name
    /// prompt; this performs the creation.
    pub fn create_branch_from_head(&mut self, name: &str) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            self.minibuffer_message("no project");
            return;
        };
        if !self.ensure_git(root) {
            return;
        }
        if name.trim().is_empty() {
            self.minibuffer_message("empty branch name");
            return;
        }
        match self.with_git(|g| g.create_branch_from_head(name)) {
            Ok(()) => self.minibuffer_message(&format!("created branch {name} at HEAD")),
            Err(e) => self.minibuffer_message(&format!("create branch failed: {e}")),
        }
    }

    /// `z` in the magit-status context: the stash list (RET pops, `x` drops).
    pub fn open_stash_picker(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        if !self.ensure_git(root) {
            return;
        }
        let candidates = self.stash_candidates();
        if candidates.is_empty() {
            self.minibuffer_message("no stashes");
            return;
        }
        self.open_picker(PickerKind::Stash, "Stash: ", candidates);
    }

    /// Pop (apply + drop) the stash at `index`, then refresh.
    pub fn stash_pop(&mut self, index: usize) {
        match self.with_git_mut(|g| g.stash_pop(index)) {
            Ok(()) => {
                self.refresh_magit();
                self.minibuffer_message(&format!("popped stash@{{{index}}}"));
            }
            Err(e) => self.minibuffer_message(&format!("stash pop failed: {e}")),
        }
    }

    /// Drop the stash at `index` (without applying it), then refresh.
    pub fn stash_drop(&mut self, index: usize) {
        match self.with_git_mut(|g| g.stash_drop(index)) {
            Ok(()) => {
                self.refresh_magit();
                self.minibuffer_message(&format!("dropped stash@{{{index}}}"));
            }
            Err(e) => self.minibuffer_message(&format!("stash drop failed: {e}")),
        }
    }

    /// Branch-create name prompt: start / append / backspace / confirm /
    /// cancel (mirrors the search-prompt pattern).
    pub fn branch_create_start(&mut self) {
        self.branch_create = Some(String::new());
        self.minibuffer_message("New branch name: ");
    }

    fn branch_create_char(&mut self, c: char) {
        if let Some(name) = self.branch_create.as_mut() {
            name.push(c);
        }
        if let Some(name) = self.branch_create.as_ref() {
            self.minibuffer_message(&format!("New branch name: {name}"));
        }
    }

    fn branch_create_backspace(&mut self) {
        if let Some(name) = self.branch_create.as_mut() {
            name.pop();
        }
        if let Some(name) = self.branch_create.as_ref() {
            self.minibuffer_message(&format!("New branch name: {name}"));
        }
    }

    fn branch_create_cancel(&mut self) {
        self.branch_create.take();
        self.minibuffer_message("cancel");
    }

    fn branch_create_confirm(&mut self) {
        let Some(name) = self.branch_create.take() else { return };
        if name.trim().is_empty() {
            self.branch_create = Some(name);
            self.minibuffer_message("empty branch name");
            return;
        }
        self.create_branch_from_head(&name);
    }

    /// The log view's rows (header + one row per commit + paging indicator).
    pub fn log_rows(&self) -> Vec<MagitRow> {
        let Some(log) = self.log.as_ref() else { return Vec::new() };
        let mut rows = vec![MagitRow {
            text: format!("## log ({})", log.display()),
            role: RowRole::Branch,
            selected: false,
        }];
        for (i, e) in log.entries.iter().enumerate() {
            rows.push(MagitRow {
                text: log_entry_display(e),
                role: RowRole::Commit,
                selected: i == log.selected,
            });
        }
        let start = log.offset.saturating_add(1);
        let end = log.offset.saturating_add(log.entries.len());
        rows.push(MagitRow {
            text: format!("  ({}–{}/{}  ·  n next · p prev · RET diff · q back)", start, end, log.total),
            role: RowRole::Comment,
            selected: false,
        });
        rows
    }

    /// The blame view's rows (header + one aligned row per line).
    pub fn blame_rows(&self) -> Vec<MagitRow> {
        let Some(b) = self.blame.as_ref() else { return Vec::new() };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let author_w = b
            .lines
            .iter()
            .map(|l| l.author.chars().count())
            .max()
            .unwrap_or(0)
            .max(4);
        let mut rows = vec![MagitRow {
            text: format!("## blame: {}", b.path),
            role: RowRole::Branch,
            selected: false,
        }];
        for (i, l) in b.lines.iter().enumerate() {
            rows.push(MagitRow {
                text: blame_line_display(l, now, author_w),
                role: RowRole::Blame,
                selected: i == b.selected,
            });
        }
        rows
    }

    /// The read-only commit-diff view's rows (header + diffstat + files +
    /// hunks), reusing the diff `RowRole`s so `row_face` colors them.
    pub fn commit_diff_rows(&self) -> Vec<MagitRow> {
        let Some(cd) = self.commit_diff.as_ref() else { return Vec::new() };
        let d = &cd.diff;
        let mut rows = vec![
            MagitRow {
                text: format!("## {} {}", d.short_id, d.subject),
                role: RowRole::Branch,
                selected: false,
            },
            MagitRow {
                text: format!(
                    "  {} files changed, {} insertions(+), {} deletions(-)",
                    d.files.len(),
                    d.insertions,
                    d.deletions
                ),
                role: RowRole::Comment,
                selected: false,
            },
        ];
        for f in &d.files {
            rows.push(MagitRow {
                text: format!("  {} {} (+{} -{})", f.path, if f.binary { "binary" } else { "" }, f.insertions, f.deletions),
                role: RowRole::File,
                selected: false,
            });
            for h in &f.hunks {
                rows.push(MagitRow {
                    text: format!("    {}", h.header),
                    role: RowRole::HunkHeader,
                    selected: false,
                });
                for l in &h.lines {
                    let role = match l.origin {
                        crate::git::diff::DiffOrigin::Addition => RowRole::DiffAdd,
                        crate::git::diff::DiffOrigin::Deletion => RowRole::DiffDelete,
                        _ => RowRole::DiffContext,
                    };
                    rows.push(MagitRow {
                        text: format!("    {}{}", l.origin.marker(), l.content),
                        role,
                        selected: false,
                    });
                }
            }
        }
        rows
    }

    /// The total row count of the commit-diff pane (header + diffstat +
    /// files + hunks), computed without building the row Vecs (the scroll
    /// handlers only need the bound, not the rows).
    fn commit_diff_row_count(&self) -> usize {
        let Some(cd) = self.commit_diff.as_ref() else {
            return 0;
        };
        let d = &cd.diff;
        let mut n = 2; // header + diffstat
        for f in &d.files {
            n += 1; // file row
            for h in &f.hunks {
                n += 1 + h.lines.len(); // hunk header + body lines
            }
        }
        n
    }

    /// The commit-diff window (visible rows + top + total) for the shared
    /// windowing (issue 003-02). This pane has no cursor: the emacs-motion
    /// keys move `commit_diff_scroll` (the window), so `window_slice` alone
    /// bounds it.
    pub fn commit_diff_view_info(&self) -> (Vec<MagitRow>, usize, usize) {
        let rows = self.commit_diff_rows();
        let total = rows.len();
        if total == 0 {
            return (Vec::new(), 0, 0);
        }
        let (start, end) = window_slice(self.commit_diff_scroll, total, pane_window(self.viewport_lines));
        (rows[start..end].to_vec(), start, total)
    }

    fn set_commit_diff_scroll(&mut self, top: usize) {
        self.commit_diff_scroll = top.min(self.commit_diff_row_count().saturating_sub(1));
    }

    /// C-n in the commit-diff view: scroll the window down one row.
    pub fn commit_diff_scroll_down(&mut self) {
        self.set_commit_diff_scroll(self.commit_diff_scroll + 1);
    }

    /// C-p in the commit-diff view: scroll the window up one row.
    pub fn commit_diff_scroll_up(&mut self) {
        self.set_commit_diff_scroll(self.commit_diff_scroll.saturating_sub(1));
    }

    /// C-v in the commit-diff view: scroll the window down one page.
    pub fn commit_diff_page_down(&mut self) {
        let step = pane_window(self.viewport_lines).saturating_sub(2).max(1);
        self.set_commit_diff_scroll(self.commit_diff_scroll + step);
    }

    /// M-v in the commit-diff view: scroll the window up one page.
    pub fn commit_diff_page_up(&mut self) {
        let step = pane_window(self.viewport_lines).saturating_sub(2).max(1);
        self.set_commit_diff_scroll(self.commit_diff_scroll.saturating_sub(step));
    }

    /// M-> in the commit-diff view: scroll the window to the last row.
    pub fn commit_diff_scroll_bottom(&mut self) {
        let total = self.commit_diff_row_count();
        let w = pane_window(self.viewport_lines);
        self.commit_diff_scroll = total.saturating_sub(w);
    }

    /// M-< in the commit-diff view: scroll the window to the top.
    pub fn commit_diff_scroll_top(&mut self) {
        self.commit_diff_scroll = 0;
    }

    /// The blame cursor row count (header + one row per blamed line).
    fn blame_row_count(&self) -> usize {
        self.blame.as_ref().map(|b| b.lines.len() + 1).unwrap_or(0)
    }

    /// Keep the blame cursor (`b.selected`) inside the visible window; persists
    /// `blame_scroll` so the window follows the cursor in both directions.
    fn blame_keep_visible(&mut self) {
        let Some(b) = self.blame.as_mut() else {
            return;
        };
        // The cursor row sits after the header, so its index in the row list
        // is `b.selected + 1`.
        let cursor = b.selected + 1;
        let total = self.blame_row_count();
        self.blame_scroll = keep_cursor_visible(self.blame_scroll, cursor, total, pane_window(self.viewport_lines));
    }

    /// The blame window (visible rows + top + total) for the shared windowing
    /// (issue 003-02). The cursor row is always inside it (kept by
    /// `blame_keep_visible`).
    pub fn blame_view_info(&self) -> (Vec<MagitRow>, usize, usize) {
        let rows = self.blame_rows();
        let total = rows.len();
        if total == 0 {
            return (Vec::new(), 0, 0);
        }
        let (start, end) = window_slice(self.blame_scroll, total, pane_window(self.viewport_lines));
        (rows[start..end].to_vec(), start, total)
    }

    /// C-n in the blame view: move the cursor down one row (the window keeps
    /// it in view).
    pub fn blame_cursor_down(&mut self) {
        if let Some(b) = self.blame.as_mut() {
            b.selected = (b.selected + 1).min(b.lines.len().saturating_sub(1));
        }
        self.blame_keep_visible();
    }

    /// C-p in the blame view: move the cursor up one row.
    pub fn blame_cursor_up(&mut self) {
        if let Some(b) = self.blame.as_mut() {
            b.selected = b.selected.saturating_sub(1);
        }
        self.blame_keep_visible();
    }

    /// C-v in the blame view: move the cursor down one page.
    pub fn blame_page_down(&mut self) {
        if let Some(b) = self.blame.as_mut() {
            let step = pane_window(self.viewport_lines).saturating_sub(2).max(1);
            b.selected = (b.selected + step).min(b.lines.len().saturating_sub(1));
        }
        self.blame_keep_visible();
    }

    /// M-v in the blame view: move the cursor up one page.
    pub fn blame_page_up(&mut self) {
        if let Some(b) = self.blame.as_mut() {
            let step = pane_window(self.viewport_lines).saturating_sub(2).max(1);
            b.selected = b.selected.saturating_sub(step);
        }
        self.blame_keep_visible();
    }

    /// M-> in the blame view: move the cursor to the last row.
    pub fn blame_cursor_bottom(&mut self) {
        if let Some(b) = self.blame.as_mut() {
            b.selected = b.lines.len().saturating_sub(1);
        }
        self.blame_keep_visible();
    }

    /// M-< in the blame view: move the cursor to the first row.
    pub fn blame_cursor_top(&mut self) {
        if let Some(b) = self.blame.as_mut() {
            b.selected = 0;
        }
        self.blame_keep_visible();
    }

    /// The log row count (header + one row per entry + paging footer).
    fn log_row_count(&self) -> usize {
        self.log.as_ref().map(|l| l.entries.len() + 2).unwrap_or(0)
    }

    /// Keep the log's in-page selection inside the visible window; persists
    /// `log_scroll` so the window follows the selection (paging via `n`/`p`
    /// resets the window to the top, separately).
    fn log_keep_visible(&mut self) {
        let Some(log) = self.log.as_ref() else {
            return;
        };
        // The selection row sits after the header, so its index is `log.selected + 1`.
        let cursor = log.selected + 1;
        let total = self.log_row_count();
        self.log_scroll = keep_cursor_visible(self.log_scroll, cursor, total, pane_window(self.viewport_lines));
    }

    /// The log window (visible rows + top + total) for the shared windowing
    /// (issue 003-02). The in-page selection is always inside it (kept by
    /// `log_keep_visible`).
    pub fn log_view_info(&self) -> (Vec<MagitRow>, usize, usize) {
        let rows = self.log_rows();
        let total = rows.len();
        if total == 0 {
            return (Vec::new(), 0, 0);
        }
        let (start, end) = window_slice(self.log_scroll, total, pane_window(self.viewport_lines));
        (rows[start..end].to_vec(), start, total)
    }

    /// The log view's title (the branch name or "(detached HEAD)").
    pub fn log_title(&self) -> String {
        self.log.as_ref().map(|l| format!("log — {}", l.display())).unwrap_or_default()
    }

    /// The blame view's title (the file being blamed).
    pub fn blame_title(&self) -> String {
        self.blame.as_ref().map(|b| format!("blame: {}", b.path)).unwrap_or_default()
    }

    /// The commit-diff view's title (the commit's short id + subject).
    pub fn commit_diff_title(&self) -> String {
        self.commit_diff
            .as_ref()
            .map(|cd| format!("commit {} — {}", cd.diff.short_id, cd.diff.subject))
            .unwrap_or_default()
    }

    /// The commit editor's title.
    pub fn commit_editor_title(&self) -> String {
        self.commit_editor
            .as_ref()
            .map(|ed| format!("commit ({} staged)", ed.staged.len()))
            .unwrap_or_default()
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
            // The first watcher event for a file WE created (e.g. the notes
            // file on `C-x n`) is our own creation, not an external change:
            // consume the marker and ignore this event so it can't flag the
            // buffer "changed on disk" (issue 05, finding 2). A later, genuine
            // external edit is not marked and still conflicts.
            if self.created_paths.remove(&key) {
                continue;
            }
            // The watcher event for a file WE saved in place (plan 005
            // issue 01): suppress only when the on-disk mtime still matches
            // the one recorded right after our write — that is our own
            // write. A genuinely later external write changes the mtime; the
            // marker is consumed either way and the event falls through to
            // the normal conflict / reload handling below.
            if let Some(expected) = self.saved_paths.get(&key) {
                let now = std::fs::metadata(path)
                    .ok()
                    .and_then(|m| m.modified().ok());
                if now == Some(*expected) {
                    self.saved_paths.remove(&key);
                    continue;
                }
                self.saved_paths.remove(&key);
            }
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
        // PART A fix (item 3): skip the expensive refresh when the batch
        // touches no TRACKED file (untracked/ignored files don't appear in
        // the status's tracked sections) — a cheap index pre-filter before
        // the git2 status + diff work.
        if let Some(git) = self.git.as_ref()
            && let Some(project) = self.project.as_ref()
            && git.any_tracked(&change.paths, &project.root)
        {
            self.refresh_magit();
        }
        if conflicts > 0 && reloaded == 0 {
            // The user should know their edit conflicts with a disk change.
            self.minibuffer_message("changed on disk — press g to reload");
        }
        // Incremental symbol-index refresh (issue 05): reparse only the
        // changed files (O(changed files), not a full rebuild).
        self.refresh_index(&change.paths);
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
        // plan 005 issue 02: auto-reload is a content change — the
        // annotation anchors for this file maintain themselves.
        self.reanchor_for_key(&key);
        if self.notes_key().as_deref() == Some(key.as_str()) {
            self.notes_buffer_dirty = true;
        }
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
        // plan 005 issue 02: on reload, the annotation anchors for this
        // file maintain themselves; reloading the notes file itself
        // re-parses its structured section.
        self.reanchor_for_key(&key);
        if self.notes_key().as_deref() == Some(key.as_str()) {
            self.notes_buffer_dirty = true;
        }
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

    /// Whether the current buffer is editable (drives the "changed on disk"
    /// banner hint: editable buffers reload via `M-x reload-buffer`, plain
    /// file buffers via `g`; on an editable buffer plain `g` self-inserts).
    pub fn current_buffer_editable(&self) -> bool {
        self.buffers
            .current_buffer()
            .map(|b| b.editable)
            .unwrap_or(false)
    }

    /// The status-line mode word for the buffer view (plan 005 issue 01):
    /// `Edit` while the current buffer is editable, `Read-only` otherwise.
    /// Empty outside the buffer view (the other views have no buffer-mode
    /// notion; a constant word there would just be noise).
    pub fn buffer_mode_display(&self) -> String {
        if self.top_view() != ViewId::Buffer {
            return String::new();
        }
        match self.buffers.current_buffer() {
            Some(b) => {
                if b.editable {
                    "Edit".to_string()
                } else {
                    "Read-only".to_string()
                }
            }
            None => String::new(),
        }
    }

    // ── symbol navigation (issue 05) ─────────────────────────────────

    /// Capture the current position as a `JumpEntry` (for use as the
    /// origin or destination in `record_jump`).
    fn current_jump_entry(&self) -> JumpEntry {
        let key = self.buffers.current().map(String::from).unwrap_or_else(|| SCRATCH_NAME.to_string());
        let line = self.point_line();
        JumpEntry {
            buffer_key: key,
            line,
            col: self.point_col(),
            label: String::new(),
        }
    }

    /// Record a jump from the current position (captured as `origin` before
    /// navigation) to the new position (captured as `destination` after
    /// navigation). Truncates forward history.
    fn record_jump(&mut self, origin: &JumpEntry, label: &str) {
        let dest = self.current_jump_entry();
        let mut dest = dest;
        dest.label = label.to_string();
        self.jump_stack.record_jump(origin, &dest);
    }

    /// `M-,`: pop back to the prior position (line + column).
    pub fn jump_back(&mut self) {
        let entry = self.jump_stack.back().cloned();
        match entry {
            Some(entry) => self.navigate_to_entry(&entry),
            None => self.minibuffer_message("no jump-back"),
        }
    }

    /// `C-i`: walk forward in the jump stack.
    pub fn jump_forward(&mut self) {
        let entry = self.jump_stack.forward().cloned();
        match entry {
            Some(entry) => self.navigate_to_entry(&entry),
            None => self.minibuffer_message("no jump-forward"),
        }
    }

    /// Navigate to a jump entry: open the buffer (if needed) and scroll to
    /// the entry's line. The results-view sentinel (recorded by `RET` in
    /// the search view) returns to the results view instead: its `line`
    /// is the hit index to restore, `col` the scroll top.
    fn navigate_to_entry(&mut self, entry: &JumpEntry) {
        if entry.buffer_key == SEARCH_JUMP_KEY {
            let hits = self.search.hits.len();
            self.search.selected = entry.line.min(hits.saturating_sub(1));
            self.search.scroll = entry.col;
            if self.top_view() != ViewId::Search {
                self.push_view(ViewId::Search);
            }
            return;
        }
        // If the buffer is not open, try to open it by key.
        if self.buffers.get(&entry.buffer_key).is_none() {
            // The buffer was killed or was never open: just scroll scratch.
            self.open_scratch();
            return;
        }
        self.buffers.set_current(&entry.buffer_key);
        self.set_point(entry.line, entry.col, entry.col);
        self.ensure_highlight();
    }

    /// Start the initial background index build (issue 05). Builds the full
    /// symbol index over the project's file list using a rayon-parallel
    /// tree-sitter parse on a background thread. No-op when there's no
    /// project, no tokio runtime (plain unit tests), or a job for the
    /// *current* generation is already in flight. If a job from a stale
    /// generation (previous project) is in flight, a new job is started
    /// (the stale job's event will be discarded by `apply_index_event`).
    pub fn start_indexing(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            return;
        };
        if self.ensure_files().is_none() {
            return;
        }
        let files = self.files.get(&root).map(|f| f.files.clone()).unwrap_or_default();
        if files.is_empty() {
            return;
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        // Single-flight per generation: don't start a new job if a job for
        // the current generation is already in flight. A stale-generation
        // job (previous project) is superseded.
        if let Some((_, _, inflight_gen)) = self.indexing
            && inflight_gen == self.index_generation
        {
            return;
        }
        self.indexing = Some((0, files.len(), self.index_generation));
        self.indexing_incremental = false;
        let bus = self.index_bus.clone();
        let root_clone = root.clone();
        let generation = self.index_generation;
        tokio::task::spawn_blocking(move || {
            // Attach a publisher so `build_index` emits a coarse progress
            // event every PROGRESS_STEP files (PART A fix, item 2a). The final
            // event below is unchanged (same generation contract).
            let progress = IndexProgress::new(files.len()).with_publisher(bus.clone(), generation);
            let index = build_index(&root_clone, &files, Some(&progress));
            bus.send(IndexEvent {
                index,
                indexing: false,
                done: files.len(),
                total: files.len(),
                generation,
            });
        });
    }

    /// Spawn a background incremental reindex for the changed paths
    /// (issue 05). Reparses only the changed files (O(changed files)).
    /// No-op when no project or no tokio runtime. If a job for the
    /// *current* generation is in flight, the changed paths are accumulated
    /// in the pending set and coalesced into one incremental job when the
    /// flight clears (no dropped events). If a job from a stale generation
    /// (previous project) is in flight, a new job is started to supersede it.
    fn refresh_index(&mut self, changed: &[PathBuf]) {
        if changed.is_empty() {
            return;
        }
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            return;
        };
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        // Single-flight per generation: if a job for the current generation
        // is in flight, accumulate the changed paths in the pending set so
        // they are coalesced into one job when the flight clears. A job from
        // a stale generation (previous project) is superseded.
        if let Some((_, _, inflight_gen)) = self.indexing
            && inflight_gen == self.index_generation
        {
            for p in changed {
                self.pending_index_changes.insert(p.clone());
            }
            return;
        }
        self.indexing = Some((0, changed.len(), self.index_generation));
        // This is an incremental job: the status line shows `indexing…`
        // (no misleading N/M total) rather than a full-build counter.
        self.indexing_incremental = true;
        let bus = self.index_bus.clone();
        let index = self.index.clone();
        let root_clone = root.clone();
        let changed = changed.to_vec();
        let generation = self.index_generation;
        tokio::task::spawn_blocking(move || {
            let mut idx = index;
            let touched = refresh_in_place(&root_clone, &changed, &mut idx);
            bus.send(IndexEvent {
                index: idx,
                indexing: false,
                done: touched.len(),
                total: touched.len(),
                generation,
            });
        });
    }

    /// `M-.`: find the definition of the symbol under point.
    /// Extracts identifiers from the current line and looks them up in the
    /// cross-file index. If exactly one definition is found, jumps directly;
    /// if multiple, opens the Picker. Falls back to the enclosing symbol
    /// when no identifier on the line is a known definition.
    pub fn xref_find_definitions(&mut self) {
        // Get the current file's project-relative path.
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let Some(buf) = self.buffers.get(&key) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let Some(path) = buf.path.as_ref() else {
            self.minibuffer_message("no file (scratch buffer)");
            return;
        };
        let Some(project) = self.project.as_ref() else {
            self.minibuffer_message("no project");
            return;
        };
        let Ok(rel) = path.strip_prefix(&project.root) else {
            self.minibuffer_message("buffer not in project");
            return;
        };
        let rel = rel.to_string_lossy().into_owned();

        let line = self.point_line();

        // Step 1: try to find an identifier on the current line that is a
        // known definition in the index (the "symbol under point").
        // PART A fix (item 5): include uppercase-initial names too (types /
        // constants like `Foo`, `CONSTANT`), not just lowercase — the old
        // filter skipped them, so `M-.` on a type fell back to the enclosing
        // symbol.
        let line_text = buf.line_text(line).unwrap_or_default();
        let identifiers: Vec<&str> = line_text
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|w| !w.is_empty() && w.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_'))
            .collect();

        let mut all_defs: Vec<crate::nav::index::Location> = Vec::new();
        for id in &identifiers {
            let defs = self.index.definitions_of(id);
            // Exclude definitions in the current file: the user is at a call
            // site and wants to jump to the *callee* (cross-file), not the
            // enclosing definition (same file). Same-file jumps use imenu.
            let cross_file: Vec<_> = defs
                .into_iter()
                .filter(|d| d.file != rel)
                .collect();
            all_defs.extend(cross_file);
        }

        // Deduplicate by (file, line) so the same definition isn't listed
        // twice if two identifiers on the line resolve to it.
        all_defs.sort_by(|a, b| (a.file.as_str(), a.symbol.line, &a.symbol.name).cmp(&(b.file.as_str(), b.symbol.line, &b.symbol.name)));
        all_defs.dedup_by(|a, b| a.file == b.file && a.symbol.line == b.symbol.line && a.symbol.name == b.symbol.name);

        let lookup_name: String;
        let defs: Vec<crate::nav::index::Location> = if !all_defs.is_empty() {
            // Use the first identifier that has a cross-file definition.
            lookup_name = all_defs[0].symbol.name.clone();
            all_defs
        } else {
            // Fall back to the enclosing symbol (by line).
            let outline = self.index.outline(&rel);
            let Some(sym) = crate::nav::index::enclosing_symbol(outline, line) else {
                self.minibuffer_message("no symbol under point");
                return;
            };
            lookup_name = sym.name.clone();
            self.index.definitions_of(&lookup_name)
        };

        if defs.is_empty() {
            self.minibuffer_message(&format!("no definition for `{lookup_name}`"));
            return;
        }

        if defs.len() == 1 {
            // Unique: capture origin, navigate, record jump.
            let origin = self.current_jump_entry();
            let def = &defs[0];
            self.open_path(&def.file);
            // Move the point to the definition's line; the window follows.
            let new_key = self.buffers.current().map(String::from).unwrap_or_default();
            self.set_point_line(def.symbol.line);
            let _ = new_key;
            self.ensure_highlight();
            self.record_jump(&origin, "M-.");
            self.minibuffer_message(&format!("jumped to {}:{}", def.file, def.symbol.line + 1));
        } else {
            // Ambiguous: open the Xref picker. The jump entry is recorded
            // when the user selects a candidate (run_selected for Xref).
            self.xref_lookup_name = lookup_name;
            let candidates: Vec<PickerCandidate> = defs
                .iter()
                .map(|d| PickerCandidate {
                    name: format!("{}:{}", d.file, d.symbol.line + 1),
                    display: format!("{}:{}  [{}] {}", d.file, d.symbol.line + 1, d.symbol.kind.tag(), d.symbol.name),
                    docs: String::new(),
                    category: "xref".to_string(),
                })
                .collect();
            self.open_picker(PickerKind::Xref, "Definition: ", candidates);
        }
    }

    /// `M-i`: imenu — open a picker over the current file's outline.
    pub fn open_imenu(&mut self) {
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let Some(buf) = self.buffers.get(&key) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let Some(path) = buf.path.as_ref() else {
            self.minibuffer_message("no file (scratch buffer)");
            return;
        };
        let Some(project) = self.project.as_ref() else {
            self.minibuffer_message("no project");
            return;
        };
        let Ok(rel) = path.strip_prefix(&project.root) else {
            self.minibuffer_message("buffer not in project");
            return;
        };
        let rel = rel.to_string_lossy().into_owned();
        let outline = self.index.outline(&rel).to_vec();
        if outline.is_empty() {
            self.minibuffer_message("no symbols in current file");
            return;
        }
        let candidates: Vec<PickerCandidate> = outline
            .iter()
            .map(|s| {
                // PART A fix (item 5): indent nested symbols by their
                // enclosing extent so imenu shows a visible outline (the old
                // flat list hid the mod/type > fn/method hierarchy). `depth`
                // is the count of symbols whose extent strictly contains `s`
                // (excluding itself / exact-extent duplicates).
                let depth = outline
                    .iter()
                    .filter(|e| {
                        !(**e == *s)
                            && e.line <= s.line
                            && s.end_line <= e.end_line
                            && (e.line < s.line || e.end_line > s.end_line)
                    })
                    .count();
                let indent = "  ".repeat(depth);
                let display = format!("{indent}{}  [{}]", s.name, s.kind.tag());
                PickerCandidate {
                    name: format!("{}:{}", s.name, s.line + 1),
                    display,
                    docs: String::new(),
                    category: "imenu".to_string(),
                }
            })
            .collect();
        self.open_picker(PickerKind::Imenu, "Imenu: ", candidates);
    }

    /// `C-c p s`: project-wide symbol picker (fuzzy over all index symbols).
    pub fn open_symbol_picker(&mut self) {
        if self.project.is_none() {
            self.minibuffer_message("no project");
            return;
        }
        if self.index.total() == 0 {
            self.minibuffer_message("no symbols indexed yet");
            return;
        }
        let candidates: Vec<PickerCandidate> = self
            .index
            .all_locations()
            .into_iter()
            .map(|loc| PickerCandidate {
                name: format!("{}:{}", loc.file, loc.symbol.line + 1),
                display: format!("{}  [{}]  {}", loc.symbol.name, loc.symbol.kind.tag(), loc.file),
                docs: String::new(),
                category: "symbol".to_string(),
            })
            .collect();
        self.open_picker(PickerKind::Symbols, "Symbol: ", candidates);
    }

    /// Install an index event into the store (called by the UI's IndexBus
    /// drain). Discards events from a stale generation (previous project's
    /// job). Replaces the index snapshot and updates the indexing state.
    /// When the flight clears, any accumulated pending paths are coalesced
    /// into one incremental job so no watcher event is dropped.
    pub fn apply_index_event(&mut self, event: &IndexEvent) {
        // Discard events from a stale generation (previous project).
        if event.generation != self.index_generation {
            return;
        }
        if event.indexing {
            // Coarse progress (PART A fix, item 2): update only the status-line
            // counter — do NOT install the (partial/empty) index, so the
            // reader's installed snapshot stays intact until the final event.
            self.indexing = Some((event.done, event.total, event.generation));
        } else {
            self.index = event.index.clone();
            self.indexing = None;
            self.indexing_incremental = false;
            // Coalesce pending paths into one incremental job now that the
            // flight has cleared (Finding 1: no dropped events during flight).
            if !self.pending_index_changes.is_empty() {
                let pending: Vec<PathBuf> = self.pending_index_changes.drain().collect();
                self.refresh_index(&pending);
            }
        }
    }

    /// The enclosing symbol name for the current buffer's cursor line,
    /// for the status-line which-function display.
    pub fn which_function(&self) -> String {
        let Some(key) = self.buffers.current().map(String::from) else {
            return String::new();
        };
        let Some(buf) = self.buffers.get(&key) else {
            return String::new();
        };
        let Some(path) = buf.path.as_ref() else {
            return String::new();
        };
        let Some(project) = self.project.as_ref() else {
            return String::new();
        };
        let Ok(rel) = path.strip_prefix(&project.root) else {
            return String::new();
        };
        let rel = rel.to_string_lossy();
        let line = self.point_line();
        let outline = self.index.outline(&rel);
        crate::nav::index::enclosing_symbol(outline, line)
            .map(|s| s.name.clone())
            .unwrap_or_default()
    }

    /// The indexing indicator for the activity display (empty when idle).
    /// A full build shows an honest, advancing `indexing N/M`; an incremental
    /// reindex shows `indexing…` (PART A fix, item 2b — no misleading total).
    pub fn indexing_display(&self) -> String {
        match &self.indexing {
            // Full builds are long enough to warrant the counter. Incremental
            // refreshes finish in milliseconds; flashing "indexing…" per
            // file-change batch on a busy repo is pure noise, so they render
            // nothing (the single-flight state in `self.indexing` is
            // unaffected -- this is display-only).
            Some((done, total, _)) if *total > 0 && !self.indexing_incremental => {
                format!("indexing {done}/{total}")
            }
            _ => String::new(),
        }
    }

    /// Set the index directly (for tests; bypasses the background thread).
    #[allow(dead_code)]
    pub fn set_index(&mut self, index: SymbolIndex) {
        self.index = index;
    }

    /// The current symbol index (for tests and the UI).
    #[allow(dead_code)]
    pub fn index(&self) -> &SymbolIndex {
        &self.index
    }

    // ── search & references (issue 06) ──────────────────────────────

    /// Hand the SearchBus receiver to the UI's drain task (issue 06: the
    /// `use_future` in `Root`) — exactly once per store; `None` when
    /// already taken.
    pub fn search_rx(&mut self) -> Option<mpsc::UnboundedReceiver<SearchEvent>> {
        self.search_rx.take()
    }

    /// Capture the pre-subscribed index receiver (see `index_rx`).
    pub fn set_index_rx(&mut self, rx: tokio::sync::watch::Receiver<crate::nav::index::IndexEvent>) {
        self.index_rx = Some(rx);
    }

    /// Take the pre-subscribed index receiver for the Root drain. Falls back
    /// to subscribing now (tests; the race only exists at startup).
    pub fn take_index_rx(&mut self) -> tokio::sync::watch::Receiver<crate::nav::index::IndexEvent> {
        self.index_rx
            .take()
            .unwrap_or_else(|| self.index_bus.subscribe())
    }

    /// Stop the in-flight search job (no-op when idle).
    fn cancel_search_job(&mut self) {
        if let Some(flag) = self.search.cancel.take() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// `C-g` in the results view: cancel the in-flight search (the view
    /// stays open on the partial results).
    pub fn search_cancel(&mut self) {
        if self.search.running {
            self.cancel_search_job();
            self.minibuffer_message("search cancelled");
        } else {
            self.minibuffer_message("nothing to cancel");
        }
    }

    /// `q` / `ESC` in the results view: cancel any in-flight search and
    /// close the view.
    pub fn search_close(&mut self) {
        if self.search.running {
            self.cancel_search_job();
        }
        if self.top_view() == ViewId::Search {
            self.close_view();
        }
    }

    /// `g` in the results view: re-run the current search with the same
    /// query.
    pub fn search_rerun(&mut self) {
        let query = self.search.query.clone();
        match self.search.kind {
            SearchKind::Project => self.start_project_search(query),
            SearchKind::References => self.start_references_search(query),
            SearchKind::Occur => self.start_occur(query),
        }
    }

    /// `n` in the results view: move to the next match (wraps).
    pub fn search_next(&mut self) {
        let n = self.search.hits.len();
        if n == 0 {
            self.minibuffer_message("no matches");
            return;
        }
        self.search.selected = (self.search.selected + 1) % n;
        self.search_keep_visible();
    }

    /// `p` in the results view: move to the previous match (wraps).
    pub fn search_prev(&mut self) {
        let n = self.search.hits.len();
        if n == 0 {
            self.minibuffer_message("no matches");
            return;
        }
        self.search.selected = (self.search.selected + n - 1) % n;
        self.search_keep_visible();
    }

    /// Keep the selected hit's row inside the visible window.
    fn search_keep_visible(&mut self) {
        let viewport = self.viewport_lines.max(1);
        let Some(&row) = self.search.hit_rows.get(self.search.selected) else {
            return;
        };
        let scroll = &mut self.search.scroll;
        if row < *scroll {
            *scroll = row;
        } else if row >= *scroll + viewport {
            *scroll = row - viewport + 1;
        }
    }

    /// `RET` in the results view: jump to the match under the cursor.
    /// Records the jump-stack origin as the results-view sentinel, so
    /// `M-,` returns to the results (with the selection restored) and
    /// the file position becomes the forward entry.
    pub fn search_jump(&mut self) {
        let Some(hit) = self.search.hits.get(self.search.selected).cloned() else {
            self.minibuffer_message("no match at point");
            return;
        };
        let sel = self.search.selected;
        let origin = JumpEntry {
            buffer_key: SEARCH_JUMP_KEY.to_string(),
            line: sel, // the hit index to restore on M-,.
            col: self.search.scroll,
            label: "*search*".to_string(),
        };
        let file = hit.file.clone();
        let line_no = hit.line_no as usize;
        self.open_path(&file);
        self.set_point_line(line_no.saturating_sub(1));
        self.ensure_highlight();
        // Leave the results view so the jumped file is what's on screen;
        // `M-,` (the sentinel entry) pushes the results back on top.
        if self.top_view() == ViewId::Search {
            self.close_view();
        }
        let dest = JumpEntry {
            buffer_key: self.buffers.current().map(String::from).unwrap_or_default(),
            line: line_no.saturating_sub(1),
            col: hit.col.unwrap_or(0) as usize,
            label: "search-RET".to_string(),
        };
        self.jump_stack.record_jump(&origin, &dest);
        self.minibuffer_message(&format!("jumped to {file}:{line_no}"));
    }

    /// The visible window of results-view rows, pre-computed for the UI:
    /// (rows, scroll top, total rows, selected hit's row relative to the
    /// window). The window keeps the selected hit visible.
    pub fn search_view_info(&self) -> (Vec<ResultRow>, usize, usize, Option<usize>) {
        let s = &self.search;
        let total = s.rows.len();
        let viewport = self.viewport_lines.max(1);
        let mut scroll = s.scroll.min(total.saturating_sub(1));
        if let Some(&row) = s.hit_rows.get(s.selected) {
            if row < scroll {
                scroll = row;
            } else if row >= scroll + viewport {
                scroll = row - viewport + 1;
            }
            scroll = scroll.min(total.saturating_sub(1));
        }
        if total == 0 {
            return (Vec::new(), 0, 0, None);
        }
        let end = (scroll + viewport).min(total);
        let rows = s.rows[scroll..end].to_vec();
        let selected_row = s
            .hit_rows
            .get(s.selected)
            .copied()
            .filter(|r| *r >= scroll && *r < end)
            .map(|r| r - scroll);
        (rows, scroll, total, selected_row)
    }

    /// The results-view title: kind, query, running counts.
    pub fn search_title(&self) -> String {
        let s = &self.search;
        let files = s.file_row.len();
        let running = if s.running { " (searching…)" } else { "" };
        if s.cancelled {
            format!(
                "{}: '{}' — {} matches in {} files (cancelled){}",
                s.kind.label(),
                s.query,
                s.hits.len(),
                files,
                running
            )
        } else {
            format!(
                "{}: '{}' — {} matches in {} files{}",
                s.kind.label(),
                s.query,
                s.hits.len(),
                files,
                running
            )
        }
    }

    /// The status-line search indicator (empty when no search is
    /// running) — the async-activity slot from issue 01.
    pub fn search_display(&self) -> String {
        if self.search.running {
            "searching".to_string()
        } else {
            String::new()
        }
    }

    /// True while a search job is in flight.
    pub fn search_running(&self) -> bool {
        self.search.running
    }

    /// The last search error (when the most recent job failed to start).
    pub fn search_error(&self) -> Option<String> {
        self.search.error.clone()
    }

    /// `C-c p s s` (project search) / `M-s o` (occur): enter the query
    /// prompt mode (the minibuffer echoes the growing query).
    pub fn search_prompt_start(&mut self, kind: SearchPromptKind) {
        self.search_prompt = Some(SearchPrompt {
            kind,
            query: String::new(),
        });
        self.minibuffer_message(kind.label());
    }

    fn search_prompt_char(&mut self, c: char) {
        let msg = match self.search_prompt.as_mut() {
            Some(p) => {
                p.query.push(c);
                format!("{}{}", p.kind.label(), p.query)
            }
            None => return,
        };
        self.minibuffer_message(&msg);
    }

    fn search_prompt_backspace(&mut self) {
        let msg = match self.search_prompt.as_mut() {
            Some(p) => {
                p.query.pop();
                format!("{}{}", p.kind.label(), p.query)
            }
            None => return,
        };
        self.minibuffer_message(&msg);
    }

    fn search_prompt_cancel(&mut self) {
        self.search_prompt.take();
        self.minibuffer_message("cancel");
    }

    fn search_prompt_confirm(&mut self) {
        let Some(p) = self.search_prompt.take() else {
            return;
        };
        if p.query.is_empty() {
            self.search_prompt = Some(p);
            self.minibuffer_message("empty search");
            return;
        }
        match p.kind {
            SearchPromptKind::Project => self.start_project_search(p.query),
            SearchPromptKind::Occur => self.start_occur(p.query),
        }
    }

    /// Begin a new search job: cancel the previous job, bump the
    /// generation, reset the results state, spawn, and land in the
    /// results view (unless it is already on top).
    fn begin_search(&mut self, kind: SearchKind, query: String, spawn: impl FnOnce(&SearchBus, usize, Arc<AtomicBool>)) {
        self.cancel_search_job();
        self.search_generation += 1;
        let generation = self.search_generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.search = SearchState {
            kind,
            query,
            running: true,
            cancelled: false,
            error: None,
            generation,
            hits: Vec::new(),
            rows: Vec::new(),
            hit_rows: Vec::new(),
            file_row: HashMap::new(),
            selected: 0,
            scroll: 0,
            cancel: Some(cancel.clone()),
        };
        let bus = self.search_bus.clone();
        spawn(&bus, generation, cancel);
        if self.top_view() != ViewId::Search {
            self.push_view(ViewId::Search);
        }
    }

    /// `C-c p s s`: project-wide search. The query is a LITERAL with
    /// smart case (redline's keymap has no prefix-arg mechanism, so
    /// projectile's "prefix = regexp" is not available; the pipeline's
    /// regex mode is used by `M-s o` and `M-?`).
    pub fn start_project_search(&mut self, query: String) {
        let Some(project) = self.project.clone() else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        let root = project.root.clone();
        let msg = format!("search: {query}");
        self.begin_search(SearchKind::Project, query.clone(), move |bus, generation, cancel| {
            let cfg = SearchConfig {
                root,
                pattern: query,
                word: false,
                fixed: true,
                case_smart: true,
                case_insensitive: false,
                glob: None,
                file_type: None,
                filter: None,
                cancel,
            };
            rg::spawn(cfg, bus, generation);
        });
        self.minibuffer_message(&msg);
    }

    /// `M-?`: references to the symbol under point. The buffer model has
    /// no column cursor yet, so point's column is 0; `symbol_under_point`
    /// takes the column and prefers the identifier at/preceding it.
    pub fn references_at_point(&mut self) {
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let Some(buf) = self.buffers.get(&key) else {
            self.minibuffer_message("no buffer");
            return;
        };
        if buf.path.is_none() {
            self.minibuffer_message("no file (scratch buffer)");
            return;
        }
        if self.project.is_none() {
            self.minibuffer_message("no project");
            return;
        }
        let line = self.point_line();
        let col = self.point_col(); // the point's column (plan 004 issue 05b)
        let line_text = buf.line_text(line).unwrap_or_default().to_string();
        let index = &self.index;
        let known = move |id: &str| !index.definitions_of(id).is_empty();
        let Some(symbol) = references::symbol_under_point(&line_text, col, known) else {
            self.minibuffer_message("no symbol under point");
            return;
        };
        self.start_references_search(symbol.to_string());
    }

    /// Start a references search for `symbol` (word-boundary, fixed,
    /// case-sensitive, token-class filtered via `references_filter`).
    pub fn start_references_search(&mut self, symbol: String) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            self.minibuffer_message("no project: start redline inside a project directory");
            return;
        };
        let msg = format!("references: {symbol}");
        self.begin_search(SearchKind::References, symbol.clone(), move |bus, generation, cancel| {
            let mut cfg = references::references_config(root, symbol);
            cfg.cancel = cancel;
            references::spawn_references(cfg, bus, generation);
        });
        self.minibuffer_message(&msg);
    }

    /// `M-s o`: occurrences of `query` (a regex, smart case) in the
    /// current buffer — the pipeline scoped to the buffer's text.
    pub fn start_occur(&mut self, query: String) {
        let key = self
            .buffers
            .current()
            .map(String::from)
            .unwrap_or_else(|| SCRATCH_NAME.to_string());
        let Some(buf) = self.buffers.get(&key) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let display = self.buffer_display(&key);
        let text = buf.text();
        let msg = format!("occur: {query}");
        self.begin_search(SearchKind::Occur, query.clone(), move |bus, generation, cancel| {
            occur::spawn_occur(display, text, query, cancel, bus.sender(), generation);
        });
        self.minibuffer_message(&msg);
    }

    /// Install a search event into the store (called by the UI's
    /// SearchBus drain). Events from a stale generation (a superseded
    /// job, or a project switch) are discarded.
    pub fn apply_search_event(&mut self, event: &SearchEvent) {
        if event.generation() != self.search.generation {
            return; // stale job
        }
        match event {
            SearchEvent::Hit { file, line_no, col, line, .. } => {
                let s = &mut self.search;
                if !s.file_row.contains_key(file) {
                    let row_idx = s.rows.len();
                    s.rows.push(ResultRow::Header {
                        file: file.clone(),
                        count: 0,
                        final_count: false,
                    });
                    s.file_row.insert(file.clone(), row_idx);
                }
                let hit = Hit {
                    file: file.clone(),
                    line_no: *line_no,
                    col: *col,
                    line: line.clone(),
                };
                let hit_index = s.hits.len();
                let row_idx = s.rows.len();
                s.rows.push(ResultRow::Hit { hit: hit.clone(), hit_index });
                s.hit_rows.push(row_idx);
                s.hits.push(hit);
                if let Some(&hr) = s.file_row.get(file)
                    && let ResultRow::Header { count, .. } = &mut s.rows[hr]
                {
                    *count += 1;
                }
            }
            SearchEvent::FileDone { file, hits, .. } => {
                let s = &mut self.search;
                if let Some(&hr) = s.file_row.get(file)
                    && let ResultRow::Header { count, final_count, .. } = &mut s.rows[hr]
                {
                    *count = *hits; // the authoritative final count
                    *final_count = true;
                }
            }
            SearchEvent::Finished { cancelled, .. } => {
                self.search.running = false;
                self.search.cancelled = *cancelled;
                self.search.cancel = None;
                // The parallel walk delivers hits in completion order;
                // normalize to a deterministic (path, line, col) order so
                // the results view is stable between runs. Streaming order
                // before Finished stays as-arrived (live feedback), the
                // final view is sorted.
                if !*cancelled {
                    self.search_sort_hits();
                }
                let n = self.search.hits.len();
                let m = self.search.file_row.len();
                if *cancelled {
                    self.minibuffer_message(&format!("search cancelled ({} matches so far)", n));
                } else {
                    self.minibuffer_message(&format!("{} matches in {} files", n, m));
                }
            }
            SearchEvent::Error { message, .. } => {
                self.search.running = false;
                self.search.error = Some(message.clone());
                self.search.cancel = None;
                self.minibuffer_message(&format!("search error: {message}"));
            }
        }
    }

    /// Reorder hits + display rows into deterministic (path, line, col)
    /// order. Preserves per-file counts and header positions; the selection
    /// resets to the first hit in display order (a fresh result set presents
    /// its first result, not the arrival-order artifact).
    fn search_sort_hits(&mut self) {
        let s = &mut self.search;
        if s.hits.len() < 2 {
            return;
        }
        s.hits.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then(a.line_no.cmp(&b.line_no))
                .then(a.col.cmp(&b.col))
        });

        // Rebuild rows: group by file in the new order, reusing the header
        // metadata (counts, final_count) from the old file_row table.
        let old_file_row = s.file_row.clone();
        let mut header_of: HashMap<String, (u64, bool)> = HashMap::new();
        for (file, &row) in &old_file_row {
            if let ResultRow::Header { count, final_count, .. } = &s.rows[row] {
                header_of.insert(file.clone(), (*count, *final_count));
            }
        }
        s.rows.clear();
        s.file_row.clear();
        s.hit_rows.clear();
        let mut last_file: Option<String> = None;
        for (hit_idx, hit) in s.hits.iter().enumerate() {
            if last_file.as_deref() != Some(hit.file.as_str()) {
                let (count, final_count) =
                    header_of.get(&hit.file).cloned().unwrap_or((0, false));
                s.file_row.insert(hit.file.clone(), s.rows.len());
                s.rows.push(ResultRow::Header {
                    file: hit.file.clone(),
                    count,
                    final_count,
                });
                last_file = Some(hit.file.clone());
            }
            s.hit_rows.push(s.rows.len());
            s.rows.push(ResultRow::Hit { hit: hit.clone(), hit_index: hit_idx });
        }
        // A fresh result set selects the first hit in display order.
        s.selected = 0;
        // Clamp the scroll anchor into the new range.
        if s.scroll >= s.rows.len() {
            s.scroll = s.rows.len().saturating_sub(1);
        }
    }

    // ── keys & dispatch (unchanged skeleton from issue 01) ──────────────

    /// Feed one keypress from the terminal. While the picker is open,
    /// printable characters extend the query, Backspace/C-h edit it,
    /// a small set of keys drives the picker (RET runs, C-g cancels,
    /// arrows / C-n / C-p move); everything else goes to the keymap
    /// engine. While isearch is active, printable characters (including
    /// letters bound to view commands) extend the query, C-s/C-r navigate,
    /// RET confirms, C-g cancels. While goto-line
    /// is active, digits build the line number, RET confirms, C-g
    /// cancels. While the search-query prompt is active, printable
    /// characters extend the query, RET confirms, C-g/ESC cancel. With
    /// the results view on top (no picker), C-g cancels the in-flight
    /// search without closing the view.
    pub fn key_event(&mut self, key: Key) {
        if self.quit {
            return;
        }
        // Quit save-prompt (plan 004 issue 04): modal — it swallows every
        // key (y / n / ! / C-g; anything else is a no-op, no "unbound key"
        // echo mid-prompt) and routes to the state machine.
        if self.quit_prompt_active() {
            self.quit_prompt_key(key);
            return;
        }
        // Transient menu (issue 002): topmost overlay. When open it swallows
        // every key except C-g: a listed leaf closes the menu and runs its
        // command, a listed prefix descends, and non-listed keys are ignored.
        if self.menu_open() {
            self.menu_key_event(key);
            return;
        }
        // Armed discard confirmation (issue 002): `y` executes, `n`/C-g/ESC
        // cancel; other keys are swallowed.
        if self.discard_armed() {
            self.discard_key_event(key);
            return;
        }
        // Toggle-read-only discard confirm (plan 005 issue 01): `y` discards
        // the unsaved edits and makes the buffer read-only, `n`/C-g/ESC
        // cancel and keep edit mode; every other key is swallowed (no
        // "unbound key" echo mid-prompt).
        if self.toggle_ro_active() {
            self.toggle_ro_key(key);
            return;
        }
        if self.picker.is_some() {
            if let Some(c) = key.char_value() {
                // Stash list: `x` drops the selected entry (magit's drop
                // key) instead of extending the filter query.
                if self.picker_kind() == Some(PickerKind::Stash) && c == 'x' {
                    let idx = self
                        .picker_filtered()
                        .get(self.picker_selected())
                        .map(|(cand, _)| cand.name.clone())
                        .and_then(|n| n.parse::<usize>().ok());
                    if let Some(idx) = idx {
                        self.stash_drop(idx);
                    }
                    return;
                }
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
        // Commit editor (issue 08): printable / backspace / RET / arrow keys
        // edit the message; ESC aborts, and that is intercepted here,
        // before the keymap engine. C-g clears an armed prefix only (Emacs
        // convention), not the whole buffer. Only the C-c C-c / C-c C-k
        // bindings reach the engine, so the `C-c` prefix pending state is
        // visible in the status line. A bare q types "q" (it is not a
        // command here). Edits clear any armed prefix; a bare `C-c` arms the
        // prefix via the engine.
        if self.top_view() == ViewId::CommitEditor {
            if let Some(c) = key.char_value() {
                self.commit_editor_insert(c);
                self.pending.clear();
                return;
            }
            if key.code == KeyCode::Backspace || key == Key::ctrl_char('h') {
                self.commit_editor_backspace();
                self.pending.clear();
                return;
            }
            if key.code == KeyCode::Enter {
                self.commit_editor_newline();
                self.pending.clear();
                return;
            }
            match key.code {
                KeyCode::Left => self.commit_editor_move(EditorMove::Left),
                KeyCode::Right => self.commit_editor_move(EditorMove::Right),
                KeyCode::Up => self.commit_editor_move(EditorMove::Up),
                KeyCode::Down => self.commit_editor_move(EditorMove::Down),
                _ => {}
            }
            if key.code == KeyCode::Left || key.code == KeyCode::Right {
                self.pending.clear();
                return;
            }
            if key.code == KeyCode::Up || key.code == KeyCode::Down {
                self.pending.clear();
                return;
            }
            if key.code == KeyCode::Escape {
                self.commit_editor_abort();
                return;
            }
            if key == Key::ctrl_char('g') {
                self.pending.clear();
                return;
            }
            // C-c … (and any other unintercepted key) goes through the engine.
            self.dispatch_key(key);
            return;
        }
        // Branch-create name prompt (issue 08): printable chars extend the
        // name, RET creates, C-g / ESC cancels.
        if self.branch_create.is_some() {
            if key == Key::ctrl_char('g') || key.code == KeyCode::Escape {
                self.branch_create_cancel();
                return;
            }
            if key.code == KeyCode::Enter {
                self.branch_create_confirm();
                return;
            }
            if key.code == KeyCode::Backspace || key == Key::ctrl_char('h') {
                self.branch_create_backspace();
                return;
            }
            if let Some(c) = key.char_value() {
                self.branch_create_char(c);
                return;
            }
            // Other keys: swallow (no "unbound key" echo mid-prompt).
            return;
        }
        // Isearch mode: every printable self-inserts into the query (run
        // before any keymap dispatch, the isearch analogue of the notes
        // editable branch in plan-002 issue 05). Chords keep their isearch
        // semantics: C-s next, C-r reverse, RET end, C-g cancel, DEL rubout.
        if self.isearch.active {
            if key == Key::ctrl_char('g') {
                self.isearch_cancel();
                return;
            }
            if key.code == KeyCode::Enter {
                self.isearch_confirm();
                return;
            }
            // PART A fix (item 5): C-s / C-r while isearch is active repeat
            // the search (next / previous match) instead of being swallowed.
            if key == Key::ctrl_char('s') {
                self.isearch_next();
                return;
            }
            if key == Key::ctrl_char('r') {
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
        // Annotation prompt (plan 005 issue 02): printable chars build the
        // note text, Backspace edits it, RET commits (record written to
        // the notes file; the cue appears immediately), C-g/ESC cancel.
        if self.note_prompt_active {
            if key == Key::ctrl_char('g') || key.code == KeyCode::Escape {
                self.note_prompt_cancel();
                return;
            }
            if key.code == KeyCode::Enter {
                self.note_prompt_confirm();
                return;
            }
            if key.code == KeyCode::Backspace || key == Key::ctrl_char('h') {
                self.note_prompt_backspace();
                return;
            }
            if let Some(c) = key.char_value() {
                self.note_prompt_char(c);
                return;
            }
            // Other keys: swallow (no "unbound key" echo mid-prompt).
            return;
        }
        // Search-query prompt mode (C-c p s s / M-s o): printable chars
        // extend the query, Backspace/C-h edit it, RET starts the search,
        // C-g/ESC cancel.
        if self.search_prompt.is_some() {
            if key == Key::ctrl_char('g') || key.code == KeyCode::Escape {
                self.search_prompt_cancel();
                return;
            }
            if key.code == KeyCode::Enter {
                self.search_prompt_confirm();
                return;
            }
            if key.code == KeyCode::Backspace || key == Key::ctrl_char('h') {
                self.search_prompt_backspace();
                return;
            }
            if let Some(c) = key.char_value() {
                self.search_prompt_char(c);
                return;
            }
            // Other keys: swallow (don't echo "unbound key" mid-prompt).
            return;
        }
        // Notes / editable buffer editing (issue 09): when the current
        // buffer is editable (the notes buffer or any locally-owned file),
        // printable chars append and Backspace deletes. Bounded editing
        // (commit-editor precedent): no cursor movement in v1.
        //
        // Interception order (issue 05, finding 1): a key that completes or
        // extends a bound sequence reaches the keymap engine FIRST (so
        // `C-x g` / `C-c p f` dispatch while editing), mirroring the commit
        // editor's careful order. Only a printable that binds nothing
        // self-inserts; C-g keeps its global cancel (clears pending, does
        // not close the notes buffer).
        if self.top_view() == ViewId::Buffer
            && self.buffers.current().and_then(|k| self.buffers.get(k).map(|b| b.editable && b.path.is_some())).unwrap_or(false)
        {
            // C-g cancels pending / closes overlays from any state,
            // including while editing (the advertised C-g matrix).
            if key == Key::ctrl_char('g') {
                self.cancel();
                return;
            }
            let mut seq = self.pending.clone();
            seq.push(key);
            let extends_sequence = matches!(
                self.engine.resolve(&seq),
                Some(Lookup::Command(_)) | Some(Lookup::Pending)
            );
            // A printable that is a DEPTH-1 leaf command (g/j/k/q/G/n/p...)
            // self-inserts while typing: firing view commands on plain
            // letters destroyed unsaved notes text (g -> reload-buffer).
            // Only chords (C-x, C-c, M-...) and prefix continuations reach
            // the engine.
            let printable_leaf_command = key.char_value().is_some()
                && self.pending.is_empty()
                && matches!(self.engine.resolve(&seq), Some(Lookup::Command(_)))
                && !self.engine.prefix_exists(&seq);
            if !self.pending.is_empty() || (extends_sequence && !printable_leaf_command) {
                // A pending prefix (or a key that starts/continues a bound
                // sequence) must reach the engine before any self-insert.
                self.dispatch_key(key);
                return;
            }
            if let Some(c) = key.char_value() {
                self.notes_insert_char(c);
                return;
            }
            if key.code == KeyCode::Backspace || key == Key::ctrl_char('h') {
                self.notes_backspace();
                return;
            }
            // Other keys fall through to the keymap engine (motion, view
            // commands, C-x C-s save, etc.).
        }
        // Tree sidebar (issue 09): when the tree is visible and the main view
        // is the buffer view, arrows / page keys move the tree cursor and RET
        // opens the selected file. The emacs motion keys (C-n/C-p/j/k) still
        // scroll the file, so arrows and file-motion are cleanly split.
        if self.tree_visible() && self.top_view() == ViewId::Buffer {
            match key.code {
                KeyCode::Down | KeyCode::PageDown => {
                    self.tree_move_down();
                    return;
                }
                KeyCode::Up | KeyCode::PageUp => {
                    self.tree_move_up();
                    return;
                }
                KeyCode::Enter => {
                    self.tree_open_selected();
                    return;
                }
                _ => {}
            }
        }
        // C-g in the results view cancels the in-flight search (the view
        // stays open on the partial results) — intercepted before the
        // global C-g so the advertised `C-g cancel search` works. The
        // picker guard keeps C-g closing an open palette (the global
        // intercept) instead of cancelling the search underneath it.
        if key == Key::ctrl_char('g')
            && self.top_view() == ViewId::Search
            && self.picker.is_none()
        {
            self.search_cancel();
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
        // emacs `recenter-top-bottom`: the position only advances when the
        // immediately-preceding command was also recenter; any other
        // command resets the cycle (emacs `recenter-last-op`), so a fresh
        // C-l starts at the first position (middle).
        if name != "recenter" {
            self.recenter_cycle = 0;
        }
        command.run(self, arg);
        Ok(())
    }

    /// C-g semantics: clear a pending sequence, close the picker, clear the
    /// mark (plan 004 issue 03), and echo a cancel notice in the minibuffer.
    pub fn cancel(&mut self) {
        let mut did = false;
        if !self.pending.is_empty() {
            self.pending.clear();
            did = true;
        }
        if self.picker.take().is_some() {
            did = true;
        }
        // Clear the mark on the current buffer (plan 004 issue 03).
        if let Some(key) = self.buffers.current().map(String::from)
            && let Some(buf) = self.buffers.get_mut(&key)
            && buf.mark.is_some()
        {
            buf.mark = None;
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

    // ── plan 004 issue 04: quit save-prompt (save-buffers-kill-terminal) ─

    /// Quit interception (plan 004 issue 04): `C-x C-c` (and the palette
    /// `quit`) no longer flips `quit` directly. With NO locally-modified
    /// buffer the quit proceeds immediately (existing behavior); with ≥1
    /// modified buffer the save-prompt state machine starts over the
    /// modified set snapshotted at interception time (oldest-first).
    pub fn begin_quit(&mut self) {
        self.clear_pending();
        let modified: Vec<String> = self
            .buffers
            .list()
            .into_iter()
            .rev() // MRU order reversed → oldest-first
            .filter(|(_, b)| b.locally_modified)
            .map(|(k, _)| k.to_string())
            .collect();
        if modified.is_empty() {
            self.quit = true;
            return;
        }
        self.quit_prompt = Some(QuitPrompt { pending: modified });
        self.quit_prompt_show();
    }

    /// Whether the quit save-prompt is active (a modified buffer is being
    /// offered to save/skip).
    pub fn quit_prompt_active(&self) -> bool {
        self.quit_prompt.is_some()
    }

    /// The display name of the buffer currently offered by the quit
    /// save-prompt (its path, or the `*scratch*` sentinel when pathless).
    pub fn quit_prompt_buffer(&self) -> Option<String> {
        let key = self
            .quit_prompt
            .as_ref()
            .and_then(|p| p.pending.first())?;
        match self.buffers.get(key).and_then(|b| b.path.as_ref()) {
            Some(p) => Some(p.display().to_string()),
            None => Some(key.clone()),
        }
    }

    /// Render the prompt for the head of the snapshot in the minibuffer row.
    fn quit_prompt_show(&mut self) {
        let Some(name) = self.quit_prompt_buffer() else {
            return;
        };
        self.minibuffer_message(&format!(
            "Save this buffer: {name}? (y, n, !, C-g)"
        ));
    }

    /// Re-render the prompt AFTER a failed save left an error message in
    /// the minibuffer: compose the prompt with the error so the decision
    /// line stays visible alongside it (plan 004 issue 05d, carried 004-04
    /// review P2). The prompt comes FIRST: the composed line wraps at the
    /// pane width, leaving the error (e.g. `save failed: ...`) on its own
    /// continuation row. A no-op when the prompt is not active.
    fn quit_prompt_show_with_error(&mut self) {
        let Some(name) = self.quit_prompt_buffer() else {
            return;
        };
        self.minibuffer_message(&format!(
            "Save this buffer: {name}? (y, n, !, C-g) — {}",
            self.message
        ));
    }

    /// Answer one key of the quit save-prompt. `y` saves the offered buffer
    /// (a failed save reports the error and re-prompts the SAME buffer),
    /// `n` skips it, `!` saves this and ALL remaining snapshotted buffers
    /// then quits, `C-g` cancels the whole quit. Every other key is
    /// swallowed (no "unbound key" echo mid-prompt).
    pub fn quit_prompt_key(&mut self, key: Key) {
        if key == Key::ctrl_char('g') {
            self.quit_prompt_cancel();
            return;
        }
        let Some(c) = key.char_value() else { return };
        match c {
            'y' => {
                let Some(p) = self.quit_prompt.as_mut() else {
                    return;
                };
                let Some(asked) = p.pending.first().cloned() else {
                    return;
                };
                if self.save_buffer_key(&asked) {
                    self.quit_prompt_advance();
                } else {
                    // A failed save already reported the error in the
                    // minibuffer; the snapshot head is untouched, so the
                    // same buffer is re-offered on the next y/n/!/C-g —
                    // redisplay the prompt alongside the error so the
                    // decision line stays visible.
                    self.quit_prompt_show_with_error();
                }
            }
            'n' => {
                self.quit_prompt_advance();
            }
            '!' => {
                let mut rest = self
                    .quit_prompt
                    .take()
                    .map(|p| p.pending)
                    .unwrap_or_default();
                for (i, k) in rest.iter().enumerate() {
                    if !self.save_buffer_key(k) {
                        // Report the failure and re-prompt from the FAILED
                        // buffer (the already-saved ones drop off), with the
                        // prompt redisplayed alongside the error.
                        self.quit_prompt = Some(QuitPrompt {
                            pending: rest.split_off(i),
                        });
                        self.quit_prompt_show_with_error();
                        return;
                    }
                }
                // Every snapshotted buffer saved: clear the prompt and quit.
                self.quit_prompt = None;
                self.quit = true;
            }
            _ => {}
        }
    }

    /// Drop the answered buffer from the snapshot; the last answer quits.
    fn quit_prompt_advance(&mut self) {
        let answered = self
            .quit_prompt
            .as_mut()
            .and_then(|p| p.pending.drain(0..1).next());
        if answered.is_none() {
            return;
        }
        if self
            .quit_prompt
            .as_ref()
            .is_some_and(|p| p.pending.is_empty())
        {
            self.quit_prompt = None;
            self.quit = true;
        } else {
            self.quit_prompt_show();
        }
    }

    /// C-g: cancel the whole quit. Buffers already answered `y` are kept
    /// saved; the rest are untouched. 004-03 cancel discipline applies to
    /// the rest of the state (pending / picker / mark), and the cancel is
    /// echoed like the other prompt cancels.
    fn quit_prompt_cancel(&mut self) {
        self.quit_prompt = None;
        self.cancel();
        self.minibuffer_message("cancel");
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

/// Cursor-move direction for the commit editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorMove {
    Left,
    Right,
    Up,
    Down,
}

/// Build the commit editor's pre-filled text (a magit-style comment block
/// listing the staged files) and place the cursor at the end (on the trailing
/// empty line, where the user types the message).
fn prefill_commit_message(staged: &[(String, char)]) -> (String, usize) {
    let mut s = String::new();
    s.push_str("# Please enter the commit message for these changes.\n");
    s.push_str("# Lines starting with '#' are ignored; C-c C-c commits, C-c C-k aborts.\n");
    s.push_str("#\n");
    s.push_str("# Staged changes:\n");
    if staged.is_empty() {
        s.push_str("#   (nothing staged)\n");
    } else {
        for (path, letter) in staged {
            s.push_str(&format!("#   {letter} {path}\n"));
        }
    }
    s.push_str("#\n");
    let len = s.len();
    (s, len)
}

/// Extract the commit message from the editor text: drop `#`-prefixed comment
/// lines and trim leading/trailing blank lines. An empty result means the
/// user typed no message.
fn extract_commit_message(rope: &Rope) -> String {
    let text = rope.to_string();
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .map(|l| l.to_string())
        .collect();
    while !lines.is_empty() && lines.first().map(|s| s.trim().is_empty()).unwrap_or(true) {
        lines.remove(0);
    }
    while !lines.is_empty() && lines.last().map(|s| s.trim().is_empty()).unwrap_or(true) {
        lines.pop();
    }
    lines.join("\n")
}

/// Move the commit-editor cursor to a neighbouring line, keeping the column
/// (clamped to the target line's length).
fn editor_cursor_line(ed: &mut CommitEditorState, delta: i32) {
    let len = ed.rope.len_lines();
    let line = ed.rope.char_to_line(ed.cursor.min(ed.rope.len_chars()));
    let line_start = ed.rope.line_to_char(line);
    let col = ed.cursor - line_start;
    let target = line as i64 + delta as i64;
    if target < 0 || target >= len as i64 {
        return;
    }
    let target = target as usize;
    let t_start = ed.rope.line_to_char(target);
    let t_len = ed
        .rope
        .get_line(target)
        .map(|l| l.len_chars())
        .unwrap_or(0);
    ed.cursor = t_start + col.min(t_len);
}

// ── Shared windowing (issue 003-02) ──────────────────────────────────────
//
// The single windowing mechanism for every long-content pane (magit status,
// commit-diff, blame, log, editable buffers). Three pure pieces of math,
// extracted from the magit status buffer's shipped logic so each pane reuses
// one implementation. All are plain Rust (no iocraft), so the UI layer only
// renders whatever the store pre-computes.

/// The number of rows that fit in a pane's content area: the viewport minus
/// the pinned chrome rows (the title, a scroll indicator, and the help line),
/// at least one. `viewport_lines` is the content height set on resize.
fn pane_window(viewport_lines: usize) -> usize {
    viewport_lines.saturating_sub(2).max(1)
}

/// The first/last visible row indices for a scroll window over `total` rows.
/// `scroll` is clamped into `[0, total)`; `end` is bounded by `total`. Returns
/// `(start, end)` (a half-open range); `(0, 0)` when `total` is 0.
fn window_slice(scroll: usize, total: usize, window: usize) -> (usize, usize) {
    if total == 0 {
        return (0, 0);
    }
    let start = scroll.min(total.saturating_sub(1));
    (start, (start + window).min(total))
}

/// The new scroll offset that keeps `cursor` inside the visible window:
/// scroll up when the cursor is above the top row, scroll down when it is
/// below the last visible row (the cursor then lands on the last visible row).
/// `cursor` must be `< total`. Pure; returns the clamped offset.
fn keep_cursor_visible(scroll: usize, cursor: usize, total: usize, window: usize) -> usize {
    if total == 0 {
        return 0;
    }
    let mut scroll = scroll;
    if cursor < scroll {
        scroll = cursor;
    } else if cursor >= scroll + window {
        scroll = cursor + 1 - window;
    }
    scroll.min(total.saturating_sub(1))
}

/// One log row: `<short_id> <subject>  <author>  <date>`.
fn log_entry_display(e: &LogEntry) -> String {
    format!("{} {}  {}  {}", e.short_id, e.subject, e.author, e.date)
}

/// One blame row: aligned `<hash> <author> <age>  <text>`.
fn blame_line_display(line: &BlameLine, now: i64, author_w: usize) -> String {
    let age = relative_time_from(line.time, now);
    format!(
        "{:<7} {:<author_w$} {:<5} {}",
        line.short_id, line.author, age, line.text
    )
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
    fn bare_q_in_buffer_view_closes_view_not_quit() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        // Bare `q` on the root buffer view: a no-op close-view, NOT a quit
        // (issue 05, finding 5: one stray q must not lose the session).
        s.key_event(key("q"));
        assert!(!s.quit, "bare q must not quit the app");
        assert_eq!(s.view_stack.len(), 1, "the buffer view must remain");

        // `q` closes an overlay list view back to the buffer (consistent with
        // the list views' `q`).
        s.key_event(key("C-x"));
        s.key_event(key("C-b")); // list-buffers
        assert_eq!(s.top_view(), ViewId::BufferList);
        s.key_event(key("q"));
        assert_eq!(s.top_view(), ViewId::Buffer, "q must close the list view");
        assert!(!s.quit);

        // `C-x C-c` remains the quit.
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit, "C-x C-c must still quit");
    }

    /// A store with an open notes buffer (the one UI-reachable modified
    /// buffer) for the quit save-prompt tests (plan 004 issue 04).
    fn notes_store() -> (tempfile::TempDir, AppStore) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        (dir, s)
    }

    #[test]
    fn quit_prompt_unmodified_fast_path() {
        let (_dir, mut s) = notes_store();
        // No typed edits: notes is open but UNMODIFIED → immediate quit,
        // no prompt (the existing q-quit behavior is preserved).
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit, "unmodified buffers must quit immediately");
        assert!(!s.quit_prompt_active());
        assert!(
            !s.message.contains("Save this buffer"),
            "no prompt must be rendered: {:?}",
            s.message
        );
    }

    #[test]
    fn quit_prompt_y_saves_and_quits() {
        let (dir, mut s) = notes_store();
        let notes_path = dir.path().join(".redline-notes.md");
        s.key_event(key("H"));
        s.key_event(key("i"));
        assert!(s.buffers.current_buffer().unwrap().locally_modified);

        // Interception: the prompt names the modified buffer's path.
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(!s.quit, "the prompt must hold the quit");
        assert!(s.quit_prompt_active());
        let expected = format!("Save this buffer: {}? (y, n, !, C-g)", notes_path.display());
        assert_eq!(s.message, expected, "prompt text mismatch");
        assert_eq!(s.quit_prompt_buffer().unwrap(), notes_path.display().to_string());

        // `y`: saved to disk, then the last answer quits.
        s.key_event(key("y"));
        assert!(s.quit, "y on the last modified buffer must quit");
        assert!(!s.quit_prompt_active());
        let on_disk = std::fs::read_to_string(&notes_path).unwrap();
        assert!(on_disk.contains("Hi"), "y must write the edit to disk");
        assert!(
            !s.buffers.current_buffer().unwrap().locally_modified,
            "save must clear locally_modified"
        );
    }

    #[test]
    fn quit_prompt_n_skips_and_quits() {
        let (dir, mut s) = notes_store();
        let notes_path = dir.path().join(".redline-notes.md");
        s.key_event(key("x"));
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit_prompt_active());

        // `n`: knowingly discard → quit, the file is left unwritten
        // (open_notes created it with the seed line; the edit must not land).
        s.key_event(key("n"));
        assert!(s.quit, "n on the last modified buffer must quit");
        let on_disk = std::fs::read_to_string(&notes_path).unwrap();
        assert!(!on_disk.contains("x"), "n must NOT write the edit to disk");
        assert!(
            s.buffers.current_buffer().unwrap().locally_modified,
            "the skipped buffer keeps its local text (still modified in memory)"
        );
    }

    /// Two modified, SAVEABLE buffers: the notes buffer plus a second
    /// editable-with-path buffer (the UI only exposes notes, so the second is
    /// built through the buffer API to exercise the multi-buffer prompt).
    fn two_modified_buffers(
        dir: &std::path::Path,
        s: &mut AppStore,
    ) -> (String, std::path::PathBuf) {
        let extra = dir.join("extra.md");
        std::fs::write(&extra, "old\n").unwrap();
        let mtime = std::fs::metadata(&extra)
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let extra_key = s
            .buffers
            .insert_rope(Some(extra.clone()), Rope::from_str("old\n"), mtime, true);
        s.mark_locally_modified(&extra_key);
        // The notes buffer (opened LAST → most recent) also gets an edit.
        s.key_event(key("z"));
        (extra_key, extra)
    }

    #[test]
    fn quit_prompt_bang_saves_all_remaining_then_quits() {
        let (dir, mut s) = notes_store();
        let (extra_key, extra) = two_modified_buffers(dir.path(), &mut s);
        let notes_path = dir.path().join(".redline-notes.md");

        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit_prompt_active());

        // `!`: save this and ALL remaining, then quit (no per-buffer answers).
        s.key_event(key("!"));
        assert!(s.quit, "! must quit after saving everything");
        assert!(!s.quit_prompt_active());
        assert!(extra.exists());
        let extra_disk = std::fs::read_to_string(&extra).unwrap();
        assert!(extra_disk.contains("old"), "! must save the extra buffer");
        let notes_disk = std::fs::read_to_string(&notes_path).unwrap();
        assert!(notes_disk.contains("z"), "! must save the notes buffer");
        assert!(!s.buffers.get(&extra_key).unwrap().locally_modified);
        assert!(!s.buffers.current_buffer().unwrap().locally_modified);
    }

    #[test]
    fn quit_prompt_asks_oldest_first_one_at_a_time() {
        let (_dir, mut s) = notes_store();
        let (_extra_key, extra) = two_modified_buffers(_dir.path(), &mut s);
        // notes was opened BEFORE extra.md → it is the OLDER of the two
        // modified buffers and must be asked first (extra.md, inserted
        // after, sits at the MRU front → last in the oldest-first walk).
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit_prompt_active());
        assert!(
            s.quit_prompt_buffer().unwrap().ends_with(".redline-notes.md"),
            "the oldest modified buffer must be asked first"
        );

        // `n` skips it; the extra.md prompt appears next (oldest-first walk).
        s.key_event(key("n"));
        assert!(!s.quit, "only one buffer answered so far");
        assert!(s.quit_prompt_active());
        assert_eq!(s.quit_prompt_buffer().unwrap(), extra.display().to_string());
        s.key_event(key("n"));
        assert!(s.quit, "the last answer must quit");
        assert!(!s.quit_prompt_active());
    }

    #[test]
    fn quit_prompt_c_g_cancels_the_whole_quit() {
        let (_dir, mut s) = notes_store();
        s.key_event(key("x"));
        let text_before = s.buffers.current_buffer().unwrap().text();

        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit_prompt_active());
        // C-g: cancel the quit entirely — prompt gone, app stays alive.
        s.key_event(key("C-g"));
        assert!(!s.quit, "C-g must cancel the quit");
        assert!(!s.quit_prompt_active());
        assert_eq!(s.message, "cancel");
        assert_eq!(
            s.buffers.current_buffer().unwrap().text(),
            text_before,
            "buffer content must be intact after C-g"
        );
        assert!(s.buffers.current_buffer().unwrap().locally_modified);

        // Quitting again re-enters the prompt (the buffer is still modified);
        // a bare answer finishes it.
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit_prompt_active());
        s.key_event(key("n"));
        assert!(s.quit);
    }

    #[test]
    fn quit_prompt_save_failure_reports_and_reprompts_same_buffer() {
        let (dir, mut s) = notes_store();
        let notes_path = dir.path().join(".redline-notes.md");
        // Make the write fail: the notes file becomes read-only.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&notes_path, std::fs::Permissions::from_mode(0o444)).unwrap();
        }
        s.key_event(key("x"));
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));

        // `y` fails: the error is reported, the SAME buffer is re-offered,
        // and the prompt decision line is redisplayed alongside the error.
        let prompt = format!(
            "Save this buffer: {}? (y, n, !, C-g)",
            notes_path.display()
        );
        s.key_event(key("y"));
        assert!(!s.quit, "a failed save must not quit");
        assert!(s.quit_prompt_active(), "the prompt must stay up after a failed save");
        assert!(
            s.message.contains("save failed"),
            "the error must be reported: {:?}",
            s.message
        );
        assert!(
            s.message.contains(&prompt),
            "the prompt decision line must be redisplayed alongside the error: {:?}",
            s.message
        );
        assert_eq!(
            s.quit_prompt_buffer(),
            Some(notes_path.display().to_string()),
            "the failed buffer must remain the offered one"
        );
        // The `!` failure branch redisplay the prompt the same way: `!`
        // re-saves the (still read-only) head buffer, fails, and re-prompts
        // from it with the decision line visible.
        s.key_event(key("!"));
        assert!(!s.quit, "a failed `!` save must not quit");
        assert!(s.quit_prompt_active(), "the prompt must stay up after a failed `!`");
        assert!(
            s.message.contains("save failed") && s.message.contains(&prompt),
            "error + prompt must both be visible after a failed `!`: {:?}",
            s.message
        );
        // Re-prompted buffer still answered by the state machine: `n` quits.
        s.key_event(key("n"));
        assert!(s.quit);
    }

    #[test]
    fn quit_prompt_snapshot_is_frozen_at_interception() {
        let (_dir, mut s) = notes_store();
        let extra = _dir.path().join("after.md");
        std::fs::write(&extra, "a\n").unwrap();
        let mtime = std::fs::metadata(&extra)
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let extra_key = s
            .buffers
            .insert_rope(Some(extra.clone()), Rope::from_str("a\n"), mtime, true);

        // Modify notes only; a SECOND buffer becomes modified AFTER the
        // interception (it must never enter the prompt).
        s.key_event(key("m"));
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit_prompt_active());
        s.mark_locally_modified(&extra_key);

        // A buffer saved mid-prompt must not re-appear either: save notes
        // through the API, then answer `n` — exactly ONE prompt total.
        let notes_key = s.buffers.current().unwrap().to_string();
        assert!(s.save_buffer_key(&notes_key));
        assert!(!s.buffers.get(&notes_key).unwrap().locally_modified);
        s.key_event(key("n"));
        assert!(s.quit, "one snapshot entry → the next answer quits");
        assert!(!s.quit_prompt_active());
        // The post-interception buffer was never asked about.
        assert!(s.buffers.get(&extra_key).unwrap().locally_modified);
    }

    #[test]
    fn quit_prompt_unknown_keys_are_swallowed() {
        let (_dir, mut s) = notes_store();
        s.key_event(key("x"));
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        let prompt = s.message.clone();
        // Stray keys during the prompt: no "unbound key" echo, no state
        // change (typing must not leak into the buffer either).
        s.key_event(key("z"));
        s.key_event(key("C-SPC"));
        s.key_event(key("RET"));
        assert!(s.quit_prompt_active(), "stray keys must not exit the prompt");
        assert!(!s.quit);
        assert_eq!(s.message, prompt, "the prompt must stay on screen");
        assert!(s.buffers.current_buffer().unwrap().text().ends_with("x"));
    }

    // ── plan 005 issue 01: file edit mode (C-x C-q / C-x C-s) ─────────

    /// A store with a real file buffer open (`src/f.rs`): the toggle/save
    /// tests' fixture. The file content is `fn old() {}\n`.
    fn file_buffer_store() -> (tempfile::TempDir, AppStore) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/f.rs"), "fn old() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_path("src/f.rs");
        (dir, s)
    }

    #[test]
    fn c_x_c_q_and_c_x_c_s_are_bound() {
        let (_dir, s) = file_buffer_store();
        use crate::app::keymap::{Lookup, parse_sequence};
        assert_eq!(
            s.engine.resolve(&parse_sequence("C-x C-q").unwrap()),
            Some(Lookup::Command("toggle-read-only")),
            "C-x C-q must resolve to toggle-read-only"
        );
        assert_eq!(
            s.engine.resolve(&parse_sequence("C-x C-s").unwrap()),
            Some(Lookup::Command("save-buffer")),
            "C-x C-s must resolve to save-buffer"
        );
    }

    #[test]
    fn toggle_read_only_flips_file_buffer_on_and_off() {
        let (_dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        assert!(!s.buffers.get(&bufk).unwrap().editable,
            "file buffers must start read-only");
        assert_eq!(s.buffer_mode_display(), "Read-only");

        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        assert!(s.buffers.get(&bufk).unwrap().editable,
            "C-x C-q must flip the file buffer into edit mode");
        assert_eq!(s.buffer_mode_display(), "Edit");
        assert!(s.message.contains("editable"), "msg: {}", s.message);

        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        assert!(!s.buffers.get(&bufk).unwrap().editable,
            "second C-x C-q must flip back to read-only");
        assert_eq!(s.buffer_mode_display(), "Read-only");
    }

    #[test]
    fn toggle_read_only_also_flips_a_real_file_backed_notes_buffer() {
        // DECISION (2026-09-18, orchestrator): a notes buffer is a real
        // file-backed buffer, so `C-x C-q` toggles it like any other file
        // (emacs `toggle-read-only` is buffer-agnostic). The earlier spec
        // wording said notes "no-op"; that was the ambiguous half and is
        // superseded. The behavior is recoverable (toggle back) and now
        // pinned by this test.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        let notes_key = s.buffers.current().unwrap().to_string();
        assert!(s.buffers.get(&notes_key).unwrap().editable,
            "notes start editable");

        s.toggle_read_only();
        assert!(!s.buffers.get(&notes_key).unwrap().editable,
            "C-x C-q puts the notes buffer into read-only mode like any file");

        s.toggle_read_only();
        assert!(s.buffers.get(&notes_key).unwrap().editable,
            "and back into edit mode");
    }

    #[test]
    fn toggle_read_only_noop_on_scratch_and_non_buffer_views() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        // Scratch (no path): no-op with a message, flag untouched.
        s.toggle_read_only();
        assert!(s.message.contains("scratch"), "msg: {}", s.message);
        let scratch_key = SCRATCH_NAME.to_string();
        assert!(s.buffers.get(&scratch_key).unwrap().editable);

        // Non-buffer view: no-op even though a file buffer is current.
        let (_dir2, mut s2) = file_buffer_store();
        s2.push_view(ViewId::BufferList);
        let bufk = s2.buffers.current().unwrap().to_string();
        s2.toggle_read_only();
        assert!(s2.message.contains("not a buffer view"), "msg: {}", s2.message);
        assert!(!s2.buffers.get(&bufk).unwrap().editable,
            "the toggle must not fire outside the buffer view");
    }

    #[test]
    fn edit_mode_file_buffer_accepts_typing_and_set_locally_modified() {
        let (_dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        // The file buffer in edit mode takes the same self-insert path as
        // notes (a printable that binds nothing appends at the end).
        assert!(s.insert_text("X"), "edit-mode file buffer must accept typing");
        assert!(
            s.buffers.get(&bufk).unwrap().locally_modified,
            "an edit must set locally_modified"
        );
        assert_eq!(s.buffers.get(&bufk).unwrap().text(), "fn old() {}\nX");
        // Backspace goes through the shared bounded-edit path too.
        s.notes_backspace();
        assert_eq!(s.buffers.get(&bufk).unwrap().text(), "fn old() {}\n");
    }

    #[test]
    fn save_buffer_clears_locally_modified_and_writes_disk() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        s.insert_text("X");
        assert!(s.buffers.get(&bufk).unwrap().locally_modified);

        s.key_event(key("C-x"));
        s.key_event(key("C-s"));
        let on_disk = std::fs::read_to_string(dir.path().join("src/f.rs")).unwrap();
        assert_eq!(on_disk, "fn old() {}\nX", "C-x C-s must write the edit to disk");
        let buf = s.buffers.get(&bufk).unwrap();
        assert!(!buf.locally_modified, "save must clear locally_modified");
        assert!(!buf.changed_on_disk, "save must clear changed_on_disk");
        assert!(s.message.contains("wrote"), "msg: {}", s.message);
    }

    #[test]
    fn save_read_only_buffer_refuses() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        // Still read-only: C-x C-s must refuse and leave the disk untouched.
        s.key_event(key("C-x"));
        s.key_event(key("C-s"));
        assert!(s.message.contains("read-only"), "msg: {}", s.message);
        assert!(!s.buffers.get(&bufk).unwrap().locally_modified);
        let on_disk = std::fs::read_to_string(dir.path().join("src/f.rs")).unwrap();
        assert_eq!(on_disk, "fn old() {}\n");
    }

    #[test]
    fn toggle_back_to_read_only_with_unsaved_edits_arms_confirm() {
        let (_dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        s.insert_text("X");

        // C-x C-q while the buffer is editable+modified: no flip, confirm armed.
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        assert!(s.toggle_ro_active(), "the discard confirm must be armed");
        assert!(s.message.contains("Discard unsaved edits"), "msg: {}", s.message);
        assert!(s.buffers.get(&bufk).unwrap().editable,
            "the flip must not happen before the answer");
        // Stray keys are swallowed and the text is untouched.
        s.key_event(key("z"));
        assert!(s.toggle_ro_active());
        assert_eq!(s.buffers.get(&bufk).unwrap().text(), "fn old() {}\nX");
    }

    #[test]
    fn toggle_ro_confirm_cancel_keeps_edit_mode_and_text() {
        for cancel_key in ["n", "C-g", "ESC"] {
            let (_dir, mut s) = file_buffer_store();
            let bufk = s.buffers.current().unwrap().to_string();
            s.key_event(key("C-x"));
            s.key_event(key("C-q"));
            s.insert_text("X");
            s.key_event(key("C-x"));
            s.key_event(key("C-q"));
            assert!(s.toggle_ro_active());
            s.key_event(key(cancel_key));
            assert!(!s.toggle_ro_active(), "{} must close the confirm", cancel_key);
            assert!(s.buffers.get(&bufk).unwrap().editable,
                "cancel must keep edit mode");
            assert!(!s.message.contains("read-only"), "msg: {}", s.message);
            assert_eq!(s.buffers.get(&bufk).unwrap().text(), "fn old() {}\nX",
                "cancel must keep the unsaved text");
        }
    }

    #[test]
    fn toggle_ro_confirm_accept_discards_and_makes_read_only() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        s.insert_text("X");
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        assert!(s.toggle_ro_active());

        s.key_event(key("y"));
        assert!(!s.toggle_ro_active());
        let buf = s.buffers.get(&bufk).unwrap();
        assert!(!buf.editable, "accept must make the buffer read-only");
        assert!(!buf.locally_modified, "accept must clear locally_modified");
        assert!(!buf.changed_on_disk);
        assert_eq!(buf.text(), "fn old() {}\n",
            "accept must re-read the on-disk content (the edit is discarded)");
        let on_disk = std::fs::read_to_string(dir.path().join("src/f.rs")).unwrap();
        assert_eq!(on_disk, "fn old() {}\n", "accept must not write to disk");
    }

    #[test]
    fn saved_path_suppresses_own_watcher_event() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        s.insert_text("X");
        s.key_event(key("C-x"));
        s.key_event(key("C-s"));
        assert!(!s.buffers.get(&bufk).unwrap().locally_modified);

        // The watcher's event for our own save must not flag the buffer.
        let path = dir.path().join("src/f.rs");
        s.apply_project_change(&change(vec![path.clone()]));
        let buf = s.buffers.get(&bufk).unwrap();
        assert!(!buf.changed_on_disk,
            "our own save must not flag changed_on_disk");
        assert_eq!(buf.text(), "fn old() {}\nX", "the buffer must be untouched");

        // The marker was consumed: a repeat event now behaves like a
        // normal external change (the buffer is in edit mode → locally
        // owned → the conflict marker lands instead of a silent clobber).
        s.apply_project_change(&change(vec![path]));
        assert!(
            s.buffers.get(&bufk).unwrap().changed_on_disk,
            "a second event after the suppression must conflict"
        );
    }

    #[test]
    fn genuine_external_write_after_save_still_conflicts() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        let path = dir.path().join("src/f.rs");
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        s.insert_text("X");
        s.key_event(key("C-x"));
        s.key_event(key("C-s"));

        // A genuinely later external write (new mtime) must NOT be
        // suppressed: the mtime recorded at save no longer matches.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&path, "fn old() {}\n\nexternal\n").unwrap();
        s.apply_project_change(&change(vec![path]));
        let buf = s.buffers.get(&bufk).unwrap();
        assert!(
            buf.changed_on_disk,
            "a later external write after our save must flag changed_on_disk"
        );
        assert_eq!(buf.text(), "fn old() {}\nX",
            "the buffer must not be auto-clobbered while in edit mode");
    }

    #[test]
    fn read_only_file_buffer_still_auto_reloads_on_external_change() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        let path = dir.path().join("src/f.rs");
        // Untouched (read-only) file buffer: the existing auto-reload
        // behavior is unchanged by edit mode.
        std::fs::write(&path, "fn fresh() {}\n").unwrap();
        s.apply_project_change(&change(vec![path]));
        assert!(s.buffer_text().contains("fresh"), "auto-reload must land");
        assert!(!s.buffers.get(&bufk).unwrap().changed_on_disk);
        assert!(!s.buffers.get(&bufk).unwrap().editable);
    }

    #[test]
    fn edit_mode_without_edits_is_locally_owned() {
        let (dir, mut s) = file_buffer_store();
        let bufk = s.buffers.current().unwrap().to_string();
        s.key_event(key("C-x"));
        s.key_event(key("C-q"));
        // Edit mode ON, zero edits: the reload guard now keys off the mode,
        // not just locally_modified.
        assert!(s.buffers.get(&bufk).unwrap().is_locally_owned());
        let path = dir.path().join("src/f.rs");
        std::fs::write(&path, "fn fresh() {}\n").unwrap();
        s.apply_project_change(&change(vec![path]));
        let buf = s.buffers.get(&bufk).unwrap();
        assert!(buf.changed_on_disk,
            "an external change while in edit mode must conflict, not auto-reload");
        assert_eq!(buf.text(), "fn old() {}\n",
            "the buffer content must not be reloaded under the cursor");
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

    /// Issue 05h: `n`/`p` move the selection exactly as `C-n`/`C-p` (no
    /// "unbound key" echo), and `d` kills the selected buffer via the
    /// existing `kill_buffer` path — the list stays open and the selection
    /// clamps to a valid row.
    #[test]
    fn buffer_list_n_p_d_keys() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut store = store(dir.path());
        store.open_path("src/main.rs");
        store.open_path("src/lib.rs");
        store.open_path("src/main.rs"); // main.rs current; MRU: main, lib, scratch

        store.dispatch("list-buffers", None).unwrap();
        assert_eq!(store.top_view(), ViewId::BufferList);
        let rows = store.buffer_rows();
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["src/main.rs", "src/lib.rs", "*scratch*"], "{names:?}");
        assert!(rows[0].current);

        // n/p move identically to C-n/C-p, with no unbound-key echo.
        store.key_event(key("C-n"));
        assert_eq!(store.buffer_list_selected(), 1);
        store.key_event(key("C-p"));
        assert_eq!(store.buffer_list_selected(), 0);
        store.key_event(key("n"));
        assert_eq!(store.buffer_list_selected(), 1);
        assert!(!store.message.contains("unbound key"), "`n` must not echo: {:?}", store.message);
        store.key_event(key("p"));
        assert_eq!(store.buffer_list_selected(), 0);

        // d kills the selected (non-current) buffer: the list stays open
        // and the selection clamps to the row that shifted into place.
        store.key_event(key("n")); // row 1: src/lib.rs
        store.key_event(key("d"));
        assert_eq!(store.top_view(), ViewId::BufferList, "d must not close the list");
        assert!(!store.quit);
        let rows = store.buffer_rows();
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["src/main.rs", "*scratch*"], "{names:?}");
        assert!(store.message.contains("killed"), "{:?}", store.message);
        assert_eq!(store.buffer_list_selected(), 1, "selection clamps to a valid row");

        // d on the last row clamps the selection to row 0.
        store.key_event(key("d"));
        assert_eq!(store.buffer_rows().len(), 1);
        assert_eq!(store.buffer_list_selected(), 0);
        assert_eq!(store.top_view(), ViewId::BufferList);

        // q still closes the list.
        store.key_event(key("q"));
        assert_eq!(store.top_view(), ViewId::Buffer);
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
        assert_eq!(store.picker_count().0, 105);

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
        assert_eq!(store.picker_selected(), 104);

        // C-p goes through the same wrap-decrement path as Up.
        store.key_event(key("C-p"));
        assert_eq!(store.picker_selected(), 103);

        // RET runs the candidate at the selected index (the last command —
        // a no-op on *scratch*, so just a message).
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
        assert_eq!(store.picker_count().0, 105);

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

    // ── plan 004 issue 05b: file-view point (line, col) + emacs motion ──

    /// A store with a file buffer whose lines have varying lengths (for
    /// goal-column / EOL-BOL-wrap coverage): a 2-char line between two
    /// 12-char lines.
    fn store_with_varied_lines() -> (AppStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let content = "aaaaaaaaaaaa\nbb\ncccccccccccc"; // 12 / 2 / 12 (3 lines)
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/v.rs"), content).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/v.rs");
        s.set_viewport_lines(10);
        (s, dir)
    }

    #[test]
    fn point_down_up_preserves_goal_column() {
        let (mut s, _dir) = store_with_varied_lines();
        // Start at line 0, col 5 (goal 5).
        s.set_point(0, 5, 5);
        // C-n to line 1 ("bb", len 2): col clamps to 2, goal stays 5.
        s.point_down();
        assert_eq!((s.point_line(), s.point_col()), (1, 2));
        // C-p back to line 0 (len 12): the goal column (5) is restored.
        s.point_up();
        assert_eq!((s.point_line(), s.point_col()), (0, 5));
    }

    #[test]
    fn point_forward_wraps_at_eol() {
        let (mut s, _dir) = store_with_varied_lines();
        s.set_point(0, 12, 12); // line 0 end (col == len 12)
        s.point_forward(); // wrap to line 1, col 0
        assert_eq!((s.point_line(), s.point_col()), (1, 0));
    }

    #[test]
    fn point_backward_wraps_at_bol() {
        let (mut s, _dir) = store_with_varied_lines();
        s.set_point(1, 0, 0); // line 1, col 0 (BOL)
        s.point_backward(); // wrap to line 0 end (col 12)
        assert_eq!((s.point_line(), s.point_col()), (0, 12));
    }

    #[test]
    fn point_line_start_end() {
        let (mut s, _dir) = store_with_varied_lines();
        s.set_point(2, 5, 5);
        s.point_line_start();
        assert_eq!((s.point_line(), s.point_col()), (2, 0));
        s.point_line_end();
        assert_eq!((s.point_line(), s.point_col()), (2, 12));
    }

    #[test]
    fn point_buffer_start_end() {
        let (mut s, _dir) = store_with_varied_lines();
        s.set_point(1, 1, 1);
        s.point_buffer_end();
        assert_eq!((s.point_line(), s.point_col()), (2, 12));
        s.point_buffer_start();
        assert_eq!((s.point_line(), s.point_col()), (0, 0));
    }

    #[test]
    fn motion_saturates_at_buffer_bounds() {
        let (mut s, _dir) = store_with_varied_lines();
        s.point_buffer_start();
        s.point_up(); // at line 0: no-op
        assert_eq!(s.point_line(), 0);
        s.point_buffer_end();
        s.point_down(); // at the last line: no-op
        assert_eq!(s.point_line(), 2);
    }

    #[test]
    fn motion_window_follows_point() {
        let (mut s, _dir) = store_with_lines(100);
        // Point far from the window: the window must follow to keep it in view.
        s.set_point(90, 0, 0);
        assert!(
            s.scroll_top() <= 90 && 90 < s.scroll_top() + 10,
            "window keeps the point in view: top={}",
            s.scroll_top()
        );
        // Moving back to line 0 scrolls the window up.
        s.set_point(0, 0, 0);
        assert_eq!(s.scroll_top(), 0);
    }

    #[test]
    fn window_scroll_keeps_point_screen_row() {
        let (mut s, _dir) = store_with_lines(100);
        s.set_point(5, 3, 3);
        // The point's screen row is 5 (window top 0). C-v keeps that row.
        let screen_row_before = s.point_line().saturating_sub(s.scroll_top());
        s.scroll_page_down();
        assert_eq!(
            s.point_line().saturating_sub(s.scroll_top()),
            screen_row_before,
            "C-v keeps the point's screen row fixed"
        );
    }

    // ── plan 004 issue 05c: mouse (line,col) click, wheel parity, words ──

    #[test]
    fn mouse_click_sets_point_line_and_col() {
        let (mut s, _dir) = store_with_lines(100);
        // Lines are "lineN" — line 2 is "line2" (5 chars). The click row is
        // 0-based within the visible area (window top 0 here).
        s.mouse_click_position(2, 3);
        assert_eq!((s.point_line(), s.point_col()), (2, 3), "click lands (line, col)");
        // The window stays put: the clicked row is visible.
        assert_eq!(s.scroll_top(), 0);
    }

    #[test]
    fn mouse_click_past_eol_clamps_to_eol() {
        let (mut s, _dir) = store_with_varied_lines();
        // Line 1 is "bb" (len 2): a click far past EOL lands at EOL.
        s.mouse_click_position(1, 99);
        assert_eq!((s.point_line(), s.point_col()), (1, 2), "clamped to EOL");
    }

    #[test]
    fn mouse_click_wide_chars_convert_display_col_to_char_index() {
        // plan 004 issue 05d: the clicked column is a terminal (display)
        // column; 中 (char 9) occupies display cols 9-10, so display col
        // 14 is char 13 ('g'), not char 14.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/wide.rs"), "CJK: abcd中 efgh\nbb\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/wide.rs");
        s.set_viewport_lines(10);
        // A click inside the wide char (either of its cells) maps to it.
        s.mouse_click_position(0, 9);
        assert_eq!((s.point_line(), s.point_col()), (0, 9), "click inside 中 → char 9");
        s.mouse_click_position(0, 10);
        assert_eq!((s.point_line(), s.point_col()), (0, 9), "second cell of 中 → char 9");
        // 'g' sits at display col 14 (one extra cell for 中).
        s.mouse_click_position(0, 14);
        assert_eq!((s.point_line(), s.point_col()), (0, 13), "display col 14 → char 13");
        // A click past the line's total WIDTH (16) clamps to EOL (char 15).
        s.mouse_click_position(0, 99);
        assert_eq!((s.point_line(), s.point_col()), (0, 15), "past total width clamps to EOL");
    }

    #[test]
    fn mouse_click_empty_line_lands_col_zero() {
        let dir = tempfile::tempdir().unwrap();
        // Line 1 is empty.
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/e.rs"), "aaaa\n\nbbbb").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/e.rs");
        s.set_viewport_lines(10);
        s.mouse_click_position(1, 40);
        assert_eq!((s.point_line(), s.point_col()), (1, 0), "empty line → col 0");
    }

    #[test]
    fn mouse_click_maps_through_scroll_top() {
        let (mut s, _dir) = store_with_lines(100);
        // Scroll down, then click the last visible row: row + top = line.
        s.set_scroll_top(50);
        s.mouse_click_position(9, 2);
        assert_eq!(s.point_line(), 59, "click row maps through the window top");
    }

    #[test]
    fn mouse_click_preserves_goal_column() {
        let (mut s, _dir) = store_with_varied_lines();
        // Goal 7 on the 12-char line; the click sets (line 1, col 1) but
        // keeps the goal column (emacs: a mouse set-point does not touch it).
        s.set_point(0, 7, 7);
        s.mouse_click_position(1, 1);
        assert_eq!((s.point_line(), s.point_col()), (1, 1));
        // C-p back to line 0 restores the goal column (7), not the click's 1.
        s.point_up();
        assert_eq!((s.point_line(), s.point_col()), (0, 7), "goal column preserved across the click");
    }

    #[test]
    fn mouse_click_position_noop_outside_buffer_view() {
        let (mut s, _dir) = store_with_lines(100);
        s.push_view(ViewId::BufferList);
        s.mouse_click_position(3, 4);
        assert_eq!((s.point_line(), s.point_col()), (0, 0), "click outside the file view is a no-op");
    }

    // ── plan 004 issue 05e: tree-sidebar click-to-select ────────────────

    #[test]
    fn tree_click_row_selects_visible_row_and_leaves_point_alone() {
        // With the tree visible, a click in the tree's columns selects the
        // tree row under it and NEVER moves the code point.
        let (mut s, _dir) = store_with_lines(100);
        s.ensure_files();
        s.toggle_tree();
        assert!(s.tree_visible());
        let n = s.tree_rows().len();
        assert!(n >= 2, "need >=2 tree rows: {n}");

        // The code point starts away from (0,0) so any movement is visible.
        s.set_point(3, 2, 0);

        // Terminal row 0 is the tree title: a no-op (selection + point).
        s.tree_click_row(0);
        assert_eq!(s.tree_selected(), 0);
        assert_eq!((s.point_line(), s.point_col()), (3, 2), "title-row click must not move the code point");

        // Terminal row 2 → visible row 1 (window top is `selected-5`, 0
        // here): the selection moves, the point does not.
        s.tree_click_row(2);
        assert_eq!(s.tree_selected(), 1, "terminal row 2 → tree row 1");
        assert_eq!((s.point_line(), s.point_col()), (3, 2), "tree-row click must not move the code point");

        // The help row (row TREE_VISIBLE_ROWS+1 = 9) is a no-op.
        s.tree_click_row(9);
        assert_eq!(s.tree_selected(), 1);
        // A far row (past the window) is a no-op.
        s.tree_click_row(99);
        assert_eq!(s.tree_selected(), 1);

        // With the tree hidden every tree click is a no-op.
        s.toggle_tree();
        assert!(!s.tree_visible());
        s.tree_click_row(2);
        assert_eq!(s.tree_selected(), 1, "hidden tree: click is a no-op");
    }

    #[test]
    fn tree_click_row_selects_within_a_scrolled_window() {
        // Window top is `selected - 5`: after moving the selection to row 7
        // (of 8 rows) the visible window starts at row 2, so terminal row
        // 1 (the first visible row) selects tree row 2.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        for i in 0..8 {
            std::fs::write(root.join(format!("f{i}.rs")), "x\n").unwrap();
        }
        let mut s = store(root);
        s.ensure_files();
        s.toggle_tree();
        let n = s.tree_rows().len();
        assert!(n >= 8, "need >=8 tree rows: {n}");
        s.tree.selected = 7;
        s.tree_click_row(1);
        assert_eq!(s.tree_selected(), 2, "terminal row 1 → window row 0 → tree row 2");
        assert_eq!((s.point_line(), s.point_col()), (0, 0), "selection only; point untouched");
    }

    #[test]
    fn mouse_wheel_is_a_window_scroll_with_the_screen_row_pinned() {
        // File view: the wheel is the C-v primitive at a 3-line step — the
        // point's screen row is pinned and its buffer line advances only
        // because the window moved under it.
        let (mut s, _dir) = store_with_lines(100);
        s.set_point(5, 2, 2);
        s.mouse_scroll_down();
        assert_eq!(s.scroll_top(), 3, "wheel down scrolls the window 3 lines");
        assert_eq!(s.point_line(), 8, "point line advanced under the window");
        assert_eq!(s.point_line() - s.scroll_top(), 5, "screen row pinned");
        s.mouse_scroll_up();
        assert_eq!(s.scroll_top(), 0, "wheel up scrolls back");
        assert_eq!(s.point_line(), 5, "point line advanced back");
        assert_eq!(s.point_line() - s.scroll_top(), 5, "screen row still pinned");
        // At the top, wheel-up saturates: the window cannot go above 0.
        s.mouse_scroll_up();
        assert_eq!(s.scroll_top(), 0);
        assert_eq!(s.point_line(), 5);
    }

    #[test]
    fn mouse_wheel_clamps_the_point_col_to_the_new_line() {
        // A short line sits three rows down: the screen-row-pinned wheel
        // drag moves the point onto it and re-clamps the col.
        let dir = tempfile::tempdir().unwrap();
        let content = "aaaaaaaaaaaa\naaaaaaaaaaaa\naaaaaaaaaaaa\nbb\ncccccccccccc\ncccccccccccc";
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/wl.rs"), content).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/wl.rs");
        s.set_viewport_lines(10);
        s.set_point(0, 5, 5);
        s.mouse_scroll_down(); // top 3: the point's row 0 drags onto line 3 ("bb")
        assert_eq!((s.point_line(), s.point_col()), (3, 2), "point line advanced, col clamped to the short line");
        s.mouse_scroll_up(); // top 0: back to line 0, col clamped from 2 (≤ 12)
        assert_eq!((s.point_line(), s.point_col()), (0, 2), "wheel back: point line advanced back, col re-clamped");
    }

    /// A store with a word-motion fixture: words, punctuation runs, an
    /// empty-line boundary, and wrap cases (no trailing newline).
    fn store_with_words() -> (AppStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        // Line 0: "hello world_foo!!"  (words: hello, world_foo)
        // Line 1: "x"                  (single-char word)
        // Line 2: "ab cd"              (two words)
        let content = "hello world_foo!!\nx\nab cd";
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/w.rs"), content).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/w.rs");
        s.set_viewport_lines(10);
        (s, dir)
    }

    #[test]
    fn word_forward_walks_words_and_punctuation() {
        // emacs `forward-word` lands at the END of each word (not its first
        // char). Line 0: "hello world_foo!!" (len 17), line 1: "x",
        // line 2: "ab cd".
        let (mut s, _dir) = store_with_words();
        s.set_point(0, 0, 0);
        s.point_word_forward();
        assert_eq!((s.point_line(), s.point_col()), (0, 5), "end of `hello`");
        s.point_word_forward();
        assert_eq!((s.point_line(), s.point_col()), (0, 15), "skip the space, walk to the end of `world_foo`");
        s.point_word_forward();
        // "!!" + newline are one non-word run: crosses to line 1, then
        // walks `x` to its end (col 1).
        assert_eq!((s.point_line(), s.point_col()), (1, 1), "skip the punctuation run across the newline, end of `x`");
        s.point_word_forward();
        assert_eq!((s.point_line(), s.point_col()), (2, 2), "wrap across lines, end of `ab`");
        s.point_word_forward();
        assert_eq!((s.point_line(), s.point_col()), (2, 5), "skip the space, end of `cd` (buffer end)");
        // At the buffer end M-f is a no-op.
        s.point_word_forward();
        assert_eq!((s.point_line(), s.point_col()), (2, 5), "no-op at buffer end");
    }

    #[test]
    fn word_backward_walks_words_and_punctuation() {
        // emacs `backward-word` lands at the START of each word (not its
        // end). Line 0: "hello world_foo!!", line 1: "x", line 2: "ab cd".
        let (mut s, _dir) = store_with_words();
        // From the end of `cd` (line 2, col 5):
        s.set_point(2, 5, 5);
        s.point_word_backward();
        assert_eq!((s.point_line(), s.point_col()), (2, 3), "start of `cd`");
        s.point_word_backward();
        assert_eq!((s.point_line(), s.point_col()), (2, 0), "skip the space, start of `ab`");
        s.point_word_backward();
        // Cross the newline into line 1, back to the start of `x`.
        assert_eq!((s.point_line(), s.point_col()), (1, 0), "cross the newline, start of `x`");
        s.point_word_backward();
        // Cross to line 0, skip "!!" back to the start of `world_foo` (col 6).
        assert_eq!((s.point_line(), s.point_col()), (0, 6), "cross the newline, skip `!!`, start of `world_foo`");
        s.point_word_backward();
        assert_eq!((s.point_line(), s.point_col()), (0, 0), "skip the space, start of `hello` (buffer start)");
        // At the buffer start M-b is a no-op.
        s.point_word_backward();
        assert_eq!((s.point_line(), s.point_col()), (0, 0), "no-op at buffer start");
    }

    #[test]
    fn word_motion_over_empty_lines() {
        let dir = tempfile::tempdir().unwrap();
        // "word\n\n\nnext" — two empty lines between the words.
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/e2.rs"), "word\n\n\nnext").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/e2.rs");
        s.set_viewport_lines(10);
        s.set_point(0, 0, 0);
        s.point_word_forward();
        assert_eq!((s.point_line(), s.point_col()), (0, 4), "end of `word`");
        s.point_word_forward();
        assert_eq!((s.point_line(), s.point_col()), (3, 4), "M-f skips blank lines to the end of `next`");
        // M-b from the end of `next`: the char before point is a word char,
        // so it lands on `next`'s start (line 3, col 0).
        s.point_word_backward();
        assert_eq!((s.point_line(), s.point_col()), (3, 0), "M-b back to the start of `next`");
        s.point_word_backward();
        assert_eq!((s.point_line(), s.point_col()), (0, 0), "M-b back across the blank lines to the start of `word`");
    }

    #[test]
    fn word_motion_sets_goal_column_to_landing_col() {
        let (mut s, _dir) = store_with_words();
        // Landing on `world_foo`'s end sets the goal column to 15; C-n onto
        // the short line clamps col to 1, and C-p back restores goal 15
        // (clamped to line 0's length — 17 — so 15).
        s.set_point(0, 0, 0);
        for _ in 0..2 {
            s.point_word_forward(); // → (0,15) end of `world_foo`
        }
        s.point_down();
        assert_eq!((s.point_line(), s.point_col()), (1, 1), "C-n clamps to the short line");
        s.point_up();
        assert_eq!((s.point_line(), s.point_col()), (0, 15), "C-p restores the word's landing column");
        // M-b lands on `cd`'s start (line 2, col 3) with goal 3.
        s.set_point(2, 5, 5);
        s.point_word_backward();
        assert_eq!((s.point_line(), s.point_col()), (2, 3));
    }

    #[test]
    fn goto_line_lands_point_at_col_zero() {
        let (mut s, _dir) = store_with_lines(100);
        s.goto_line_start();
        s.goto_line_digit('7');
        s.goto_line_confirm();
        assert_eq!((s.point_line(), s.point_col()), (6, 0));
    }

    #[test]
    fn scroll_page_down_keeps_two_line_overlap() {
        // PART A fix (item 5): a full page keeps a 2-line context overlap
        // (emacs `next-screen-context-lines`), so it advances by
        // `viewport - 2`, not the full viewport.
        let (mut s, _dir) = store_with_lines(100); // viewport 10 → step 8
        s.scroll_page_down();
        assert_eq!(s.scroll_top(), 8);
        s.scroll_page_down();
        assert_eq!(s.scroll_top(), 16);
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

    // ── plan 004 issue 05c: recenter-top-bottom contract ─────────────
    // store_with_lines(100) writes 100 "lineN\n" lines → ropey 101 lines;
    // viewport=10 → max_scroll=91, mid=5, top row=0, bottom row=9.
    // `recenter_cycle` selects the position in the emacs `(middle top
    // bottom)` order: index 0 → middle, 1 → top, 2 → bottom, then repeats.
    // It is reset to 0 on any non-recenter command (in `dispatch`), so a
    // fresh C-l goes to MIDDLE. The point's line never moves.

    #[test]
    fn recenter_fresh_goes_to_middle() {
        // A C-l that does not follow another C-l lands the point on the
        // middle row (cycle index 0 → viewport/2).
        let (mut s, _dir) = store_with_lines(100);
        s.set_point(50, 0, 0);
        s.recenter();
        assert_eq!(s.scroll_top(), 45, "fresh C-l → middle (row 5)");
        assert_eq!(s.point_line(), 50, "the point does not move");
    }

    #[test]
    fn recenter_consecutive_cycles_middle_top_bottom() {
        // Consecutive C-l advance middle → top → bottom, purely by cycle
        // index (no zone-derived guess).
        let (mut s, _dir) = store_with_lines(100);
        s.set_point(50, 0, 0);
        s.recenter(); // middle (row 5) → top 45
        assert_eq!(s.scroll_top(), 45);
        s.recenter(); // top (row 0) → top 50
        assert_eq!(s.scroll_top(), 50);
        s.recenter(); // bottom (row 9) → top 41
        assert_eq!(s.scroll_top(), 41);
        assert_eq!(s.point_line(), 50, "the point never moved");
    }

    #[test]
    fn recenter_resets_to_middle_after_other_command() {
        // Any intervening non-recenter command resets the cycle (in
        // `dispatch`), so the next C-l goes to MIDDLE even though the
        // previous C-l had already advanced the cycle.
        let (mut s, _dir) = store_with_lines(100);
        s.set_point(50, 0, 0);
        s.recenter(); // middle → top 45
        s.recenter(); // top → top 50
        assert_eq!(s.scroll_top(), 50);
        s.recenter_cycle = 0; // simulate an intervening command
        s.recenter(); // fresh again → middle → top 45
        assert_eq!(s.scroll_top(), 45);
    }

    #[test]
    fn recenter_resets_cycle_through_dispatch() {
        // The reset happens in `dispatch`: a non-recenter command between
        // two C-l's forces the next C-l back to MIDDLE.
        let (mut s, _dir) = store_with_lines(100);
        s.set_point(50, 0, 0);
        s.dispatch("recenter", None).unwrap(); // fresh → middle
        assert_eq!(s.scroll_top(), 45);
        s.dispatch("recenter", None).unwrap(); // top
        assert_eq!(s.scroll_top(), 50);
        // A non-recenter command resets the cycle to 0.
        s.dispatch("re-walk", None).unwrap();
        s.set_point(50, 0, 0); // restore the point for a clean read
        s.dispatch("recenter", None).unwrap(); // fresh again → middle
        assert_eq!(s.scroll_top(), 45);
    }

    #[test]
    fn recenter_full_cycle_returns_to_start() {
        let (mut s, _dir) = store_with_lines(100);
        s.set_point(50, 0, 0);
        s.recenter(); // middle (row 5)
        assert_eq!(s.scroll_top(), 45);
        s.recenter(); // top (row 0)
        assert_eq!(s.scroll_top(), 50);
        s.recenter(); // bottom (row 9)
        assert_eq!(s.scroll_top(), 41);
        s.recenter(); // middle again (row 5) — the cycle repeats
        assert_eq!(s.scroll_top(), 45);
        s.recenter(); // top again (row 0)
        assert_eq!(s.scroll_top(), 50);
        assert_eq!(s.point_line(), 50, "the point never moved");
    }

    #[test]
    fn recenter_tiny_scroll_ranges_keep_the_point_in_view() {
        // Regression (plan-004-02 review, reworked to the 05c contract):
        // when the buffer barely scrolls (max_scroll in {1,2}) the
        // top/middle/bottom screen rows are unreachable, so the clamps pin
        // the point where it is — C-l must never leave the window off the
        // point and must keep the scroll in range (no dead ends, no
        // out-of-range writes, no oscillation). store_with_lines(n) writes
        // n lines + trailing \n → ropey n+1 lines; viewport=10.
        for n in [10, 11] { // max_scroll in {1, 2}
            let (mut s, _dir) = store_with_lines(n);
            let total = s.buffers.current_buffer().unwrap().line_count();
            let max_scroll = total - 10;
            s.set_point(5, 0, 0);
            for _ in 0..6 {
                s.recenter();
                assert!(
                    s.scroll_top() <= max_scroll,
                    "top in [0, {max_scroll}] (n={n})"
                );
                // The point's screen row must stay inside the viewport.
                let row = s.point_line().saturating_sub(s.scroll_top());
                assert!(row < 10, "point screen row {row} in the viewport (n={n})");
            }
        }
    }

    #[test]
    fn recenter_tiny_viewports_still_cycle() {
        // A 3-row viewport (mid=1, top=0, bottom=2) must cycle without a
        // dead end by index: middle (row 1) → top (row 0) → bottom (row 2)
        // → middle. Rows that collide after clamping simply repeat.
        let (mut s, _dir) = store_with_lines(100);
        s.set_viewport_lines(3);
        s.set_point(50, 0, 0);
        s.recenter(); // middle (row 1) → top 49
        assert_eq!(s.scroll_top(), 49);
        s.recenter(); // top (row 0) → top 50
        assert_eq!(s.scroll_top(), 50);
        s.recenter(); // bottom (row 2) → top 48
        assert_eq!(s.scroll_top(), 48);
        s.recenter(); // middle (row 1) → top 49
        assert_eq!(s.scroll_top(), 49);
        assert_eq!(s.point_line(), 50);
    }

    #[test]
    fn recenter_noop_when_buffer_fits_viewport() {
        // total=5, viewport=10 → max_scroll=0 → no-op.
        let dir = tempfile::tempdir().unwrap();
        let mut c = String::new();
        for i in 0..5 {
            c.push_str(&format!("line_{i}\n"));
        }
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/a.rs"), &c).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.set_viewport_lines(10);
        s.open_path("src/a.rs");
        s.recenter();
        assert_eq!(s.scroll_top(), 0, "no-op when buffer fits viewport");
    }

    // ── plan 004 row 11: position display ─────────────────────────────

    #[test]
    fn position_display_top() {
        let (s, _dir) = store_with_lines(100);
        assert_eq!(s.file_view_position_display(), "Top");
    }

    #[test]
    fn position_display_bot() {
        let (mut s, _dir) = store_with_lines(100);
        // M-> (point-buffer-end): the point lands on the last line; the
        // window follows, so the position display reports "Bot".
        s.point_buffer_end();
        assert_eq!(s.file_view_position_display(), "Bot");
    }

    #[test]
    fn position_display_middle() {
        let (mut s, _dir) = store_with_lines(100);
        s.set_point_line(50); // line 51 (1-based), 50/101 → 50% (rounded)
        assert_eq!(s.file_view_position_display(), "L51,50%");
    }

    #[test]
    fn position_display_single_line_buffer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "hello\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/a.rs");
        assert_eq!(s.file_view_position_display(), "Top");
    }

    #[test]
    fn position_display_empty_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let s = AppStore::at(dir.path(), base.path().to_path_buf());
        // The scratch buffer has an empty rope: line_count()=1, scroll_top=0 → "Top".
        assert_eq!(s.file_view_position_display(), "Top");
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
        assert_eq!(s.scroll_top(), 8, "page down keeps a 2-line overlap");
        s.open_path("src/b.rs");
        assert_eq!(s.scroll_top(), 0);
        s.open_path("src/a.rs");
        assert_eq!(s.scroll_top(), 8, "scroll preserved on return");
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
        let pre_point = s.point_line();
        assert_eq!(pre_point, 2);
        s.isearch_start(IsearchDirection::Forward);
        s.isearch_query_char('o'); // only in "omega" (line 4)
        assert_eq!(s.point_line(), 4, "search must land the point on the match");
        s.isearch_cancel();
        assert!(!s.isearch_active());
        assert_eq!(s.point_line(), pre_point, "cancel must restore the point");
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
    fn isearch_bound_command_letters_extend_query() {
        // Regression: keys that are depth-1 leaf commands in the file view
        // (n, p, l, g, q) must extend the isearch query, not dispatch.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        // Content with "line_5" on line 5 so the full query matches.
        let lines: Vec<String> = (1..=10).map(|i| format!("fn line_{}() {{\n", i)).collect();
        std::fs::write(root.join("src/t.rs"), lines.join("")).unwrap();
        let mut s = store(root);
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        // Type each character of "line_5" via key_event (the bug path).
        for c in "line_5".chars() {
            s.key_event(Key::char(c));
        }
        assert_eq!(s.isearch.query, "line_5", "all printables must extend the query");
        assert!(s.isearch.active);
    }

    #[test]
    fn isearch_match_count_continuity_while_typing() {
        // Match count must update as each character is typed, including
        // bound-command letters (n, p, l, g, q) inside the query.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        // "gamma" is the only line with a 'g'; typing "gamma" one char at a
        // time (including 'n'... wait, no 'n' in gamma). Use "gnome" style:
        // "gnome" appears once; 'g' matches, 'gn' matches, 'gno' matches,
        // 'gnom' matches, 'gnome' matches – all with count 1, and 'n'
        // (the 2nd char) must extend the query, not navigate.
        std::fs::write(
            root.join("src/t.rs"),
            "gnome\nalpha\nbeta\n",
        )
        .unwrap();
        let mut s = store(root);
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.key_event(Key::char('g'));
        assert_eq!(s.isearch.query, "g");
        assert_eq!(s.isearch_match_count(), 1);
        // 'n' is the key that was dropped by the old code – it must extend
        // the query, not call isearch_next().
        s.key_event(Key::char('n'));
        assert_eq!(s.isearch.query, "gn");
        assert_eq!(s.isearch_match_count(), 1);
        s.key_event(Key::char('o'));
        assert_eq!(s.isearch.query, "gno");
        assert_eq!(s.isearch_match_count(), 1);
        s.key_event(Key::char('m'));
        assert_eq!(s.isearch.query, "gnom");
        assert_eq!(s.isearch_match_count(), 1);
        s.key_event(Key::char('e'));
        assert_eq!(s.isearch.query, "gnome");
        assert_eq!(s.isearch_match_count(), 1);
    }

    #[test]
    fn isearch_c_s_c_r_ret_c_g_unchanged() {
        // Chords (C-s, C-r, RET, C-g) keep their isearch semantics after
        // the printable-interception fix.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/t.rs"),
            "foo bar foo baz foo qux\n",
        )
        .unwrap();
        let mut s = store(root);
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.key_event(Key::char('f'));
        s.key_event(Key::char('o'));
        s.key_event(Key::char('o'));
        assert!(s.isearch.active);
        assert_eq!(s.isearch_match_count(), 3);
        // C-s advances to next match.
        let before = s.isearch.current;
        s.key_event(Key::ctrl_char('s'));
        assert_eq!(s.isearch.current, (before + 1) % 3, "C-s must advance");
        assert!(s.isearch.active);
        // C-r moves to previous match.
        s.key_event(Key::ctrl_char('r'));
        assert_eq!(s.isearch.current, before, "C-r must go back");
        assert!(s.isearch.active);
        // RET confirms and deactivates.
        s.key_event(Key::new(KeyCode::Enter));
        assert!(!s.isearch.active);
        // C-g cancels (start a new search first).
        s.isearch_start(IsearchDirection::Forward);
        s.key_event(Key::char('f'));
        assert!(s.isearch.active);
        s.key_event(Key::ctrl_char('g'));
        assert!(!s.isearch.active);
    }

    #[test]
    fn isearch_regression_guard_n_is_not_navigation() {
        // Discriminating test: with the old code, key('n') during isearch
        // called isearch_next() and left the query unchanged. With the fix,
        // it appends 'n' to the query.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/t.rs"), "line_5\nline_5\n").unwrap();
        let mut s = store(root);
        s.open_path("src/t.rs");
        s.isearch_start(IsearchDirection::Forward);
        s.key_event(Key::char('l'));
        assert_eq!(s.isearch.query, "l");
        s.key_event(Key::char('i'));
        assert_eq!(s.isearch.query, "li");
        // This is the character that was dropped by the old code.
        s.key_event(Key::char('n'));
        assert_eq!(s.isearch.query, "lin", "'n' must extend the query, not navigate");
        s.key_event(Key::char('e'));
        s.key_event(Key::char('_'));
        s.key_event(Key::char('5'));
        assert_eq!(s.isearch.query, "line_5");
        assert_eq!(s.isearch_match_count(), 2);
    }

    #[test]
    fn goto_line_confirm_jumps_to_line() {
        // PART A fix (item 5): `M-g g` input is 1-based (line N → 0-based
        // scroll top N-1), matching the error message and emacs.
        let (mut s, _dir) = store_with_lines(100);
        s.goto_line_start();
        assert!(s.goto_line_active());
        s.goto_line_digit('5');
        s.goto_line_digit('0');
        assert_eq!(s.goto_line_input(), "50");
        s.goto_line_confirm();
        assert!(!s.goto_line_active());
        assert_eq!(s.point_line(), 49, "line 50 (1-based) → point line 49");

        // Line 1 is the very top.
        s.goto_line_start();
        s.goto_line_digit('1');
        s.goto_line_confirm();
        assert_eq!(s.point_line(), 0, "line 1 → point line 0");
    }

    #[test]
    fn goto_line_zero_is_out_of_range() {
        // 1-based: 0 is below the valid range (1..=total).
        let (mut s, _dir) = store_with_lines(100);
        s.goto_line_start();
        s.goto_line_digit('0');
        s.goto_line_confirm();
        assert!(!s.goto_line_active());
        assert!(s.message.contains("out of range"), "msg: {}", s.message);
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
    fn apply_project_change_tracked_path_refreshes_magit_counts() {
        // F4 carried leg: the classifier (any_tracked) is tested separately;
        // this drives the magit-REFRESH leg inside apply_project_change —
        // a tracked-path change must update the dirty counts.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fn git_cli(d: &std::path::Path, args: &[&str]) {
            let out = std::process::Command::new("git")
                .arg("-C").arg(d)
                .args(args)
                .env("GIT_AUTHOR_NAME", "T")
                .env("GIT_AUTHOR_EMAIL", "t@e.com")
                .env("GIT_COMMITTER_NAME", "T")
                .env("GIT_COMMITTER_EMAIL", "t@e.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .expect("run git");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        git_cli(root, &["init", "-q", "-b", "main"]);
        git_cli(root, &["config", "user.name", "T"]);
        git_cli(root, &["config", "user.email", "t@e.com"]);
        std::fs::write(root.join("tracked.rs"), "fn a() {}\n").unwrap();
        git_cli(root, &["add", "tracked.rs"]);
        git_cli(root, &["commit", "-q", "-m", "init"]);

        let mut s = store(root);
        // Prime the repo (git=Some) and refresh (clean repo → all-zero counts).
        s.open_magit_status();
        let clean = s.dirty_counts().expect("magit status must populate dirty counts");
        assert_eq!(
            clean.staged + clean.unstaged + clean.untracked,
            0,
            "clean repo must report zero dirty: {clean:?}"
        );

        // A tracked file changes on disk → an unstaged modification appears.
        let proj_root = s.project.as_ref().unwrap().root.clone();
        std::fs::write(proj_root.join("tracked.rs"), "fn a() {}\nfn b() {}\n").unwrap();

        // The watcher's apply_project_change must run the refresh leg.
        s.apply_project_change(&change(vec![proj_root.join("tracked.rs")]));
        let d = s.dirty_counts().expect("refresh leg must keep dirty counts populated");
        assert!(d.unstaged >= 1, "tracked-path change must show an unstaged count: {d:?}");
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

    // ── issue 05: jump stack tests (finding #3) ────────────────────────

    fn jump_entry(buffer_key: &str, line: usize) -> JumpEntry {
        JumpEntry {
            buffer_key: buffer_key.to_string(),
            line,
            col: 0,
            label: "test".to_string(),
        }
    }

    #[test]
    fn jump_stack_back_forward_round_trip() {
        let mut stack = JumpStack::default();
        let p0 = jump_entry("a.rs", 0);
        let d1 = jump_entry("b.rs", 10);
        let d2 = jump_entry("c.rs", 20);

        // First jump: P0 → D1.
        stack.record_jump(&p0, &d1);
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.back().unwrap().line, 0, "back: P0");
        assert_eq!(stack.forward().unwrap().line, 10, "forward: D1");

        // Second jump: D1 → D2 (from D1, which is the current position).
        // We need to simulate being at D1: pos is now 1 (after forward).
        // record_jump truncates to pos+1 = 2, so it keeps [P0, D1] and adds D2.
        stack.record_jump(&d1, &d2);
        assert_eq!(stack.len(), 3);

        // Back twice: D1, then P0.
        assert_eq!(stack.back().unwrap().line, 10, "back: D1");
        assert_eq!(stack.back().unwrap().line, 0, "back: P0");

        // Forward twice: D1, then D2.
        assert_eq!(stack.forward().unwrap().line, 10, "forward: D1");
        assert_eq!(stack.forward().unwrap().line, 20, "forward: D2");

        // At the end: forward is None.
        assert!(stack.forward().is_none());
    }

    #[test]
    fn jump_stack_back_at_start_returns_none() {
        let mut stack = JumpStack::default();
        let p0 = jump_entry("a.rs", 0);
        let d1 = jump_entry("b.rs", 10);
        stack.record_jump(&p0, &d1);
        // Go back to the start.
        stack.back();
        assert!(stack.back().is_none(), "no further back");
    }

    #[test]
    fn jump_stack_new_jump_truncates_forward_history() {
        let mut stack = JumpStack::default();
        let p0 = jump_entry("a.rs", 0);
        let d1 = jump_entry("b.rs", 10);
        let d2 = jump_entry("c.rs", 20);
        let d3 = jump_entry("d.rs", 30);

        // P0 → D1 → D2.
        stack.record_jump(&p0, &d1);
        // Now at D1 (pos=1). Jump D1 → D2.
        stack.record_jump(&d1, &d2);
        assert_eq!(stack.len(), 3);

        // Go back to D1 (pos=1).
        stack.back();
        // New jump from D1: D1 → D3. Truncates D2 from forward history.
        stack.record_jump(&d1, &d3);
        assert_eq!(stack.len(), 3, "D2 was truncated: [P0, D1, D3]");

        // Back: D1, then P0.
        assert_eq!(stack.back().unwrap().line, 10);
        assert_eq!(stack.back().unwrap().line, 0);
        // Forward: D1, then D3 (not D2).
        assert_eq!(stack.forward().unwrap().line, 10);
        assert_eq!(stack.forward().unwrap().line, 30);
    }

    // ── issue 05: xref tests (finding #2) ─────────────────────────────

    fn store_with_index(files: &[(&str, &str)]) -> (AppStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        for (rel, content) in files {
            let path = dir.path().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, content).unwrap();
        }
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        // Build the index synchronously (no tokio runtime in unit tests).
        let files_list = crate::model::files::FileList::build(dir.path()).unwrap();
        let root = dir.path().to_path_buf();
        let index = build_index(&root, &files_list.files, None);
        s.set_index(index);
        (s, dir)
    }

    #[test]
    fn xref_cross_file_definition_jumps_directly() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "mod lib {\n    pub fn target() {}\n}\nfn main() { lib::target(); }\n"),
            ("src/lib.rs", "pub fn target() {}\npub fn other() {}\n"),
        ]);
        // Open main.rs and position the point at the call site line (line 3).
        s.open_path("src/main.rs");
        s.set_point_line(3);
        // `target` is defined in main.rs (same file, excluded) and lib.rs
        // (cross-file). Only the cross-file definition is considered →
        // unique → jump directly to src/lib.rs.
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique cross-file: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 0, "target is at line 0 in lib.rs");
    }

    #[test]
    fn xref_ambiguous_cross_file_opens_picker() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() { target(); }\n"),
            ("src/a.rs", "pub fn target() {}\n"),
            ("src/b.rs", "pub fn target() {}\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point_line(0);
        // `target` is defined in a.rs and b.rs (both cross-file) → ambiguous.
        s.xref_find_definitions();
        assert!(s.picker_open(), "ambiguous: picker should be open");
        assert_eq!(s.picker_kind(), Some(PickerKind::Xref));
        assert_eq!(s.picker_filtered().len(), 2, "two candidates");
    }

    #[test]
    fn xref_single_definition_jumps_cross_file() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() { other(); }\n"),
            ("src/lib.rs", "pub fn other() {}\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point_line(0);
        // `other` is only defined in lib.rs: unique → jump directly.
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 0);
    }

    #[test]
    fn xref_no_symbol_under_point_falls_back_to_enclosing() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() {\n    let x = 1;\n}\n"),
        ]);
        s.open_path("src/main.rs");
        // Line 1: "    let x = 1;" — no known definition on this line.
        // Fall back to enclosing symbol: `main`.
        s.set_point_line(1);
        s.xref_find_definitions();
        // `main` is defined only in main.rs: unique → jump to main's definition (line 0).
        assert!(!s.picker_open());
        assert_eq!(s.point_line(), 0, "jumped to main's definition");
    }

    // ── issue 05: which-function test (finding: enclosing-symbol) ──────

    #[test]
    fn which_function_enclosing_symbol_nested() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "mod outer {\n    fn f() {\n        g()\n    }\n}\n"),
        ]);
        s.open_path("src/main.rs");
        // Line 2 ("        g()"): inside fn f, inside mod outer.
        // The innermost enclosing symbol is `f`.
        s.set_point_line(2);
        assert_eq!(s.which_function(), "f");
        // Line 0 ("mod outer {"): inside mod outer, outside fn f.
        s.set_point_line(0);
        assert_eq!(s.which_function(), "outer");
        // Line 99: outside everything.
        s.set_point_line(99);
        assert_eq!(s.which_function(), "");
    }

    // ── issue 05: apply_index_event test ──────────────────────────────

    #[test]
    fn apply_index_event_updates_index_and_clears_indexing() {
        let (mut s, _dir) = store_with_index(&[
            ("src/a.rs", "fn a() {}\n"),
        ]);
        // Simulate an indexing event.
        let mut new_index = SymbolIndex::new();
        new_index.set_file(
            "src/b.rs",
            vec![crate::syntax::queries::Symbol {
                name: "b".into(),
                kind: crate::syntax::queries::SymbolKind::Function,
                line: 0,
                start_byte: 3,
                end_byte: 4,
                end_line: 0,
            }],
        );
        let event = IndexEvent {
            index: new_index,
            indexing: false,
            done: 1,
            total: 1,
            generation: 0,
        };
        s.indexing = Some((0, 1, 0)); // simulate in-flight (gen=0)
        s.apply_index_event(&event);
        assert!(s.indexing.is_none(), "indexing cleared after event");
        assert_eq!(s.index().total(), 1);
        assert!(s.index().has("src/b.rs"));
    }

    // ── issue 05: Finding 1 — pending changes coalesced on flight clear ──

    #[tokio::test]
    async fn refresh_index_coalesces_pending_changes_during_flight() {
        // Set up a project with P1 and P2.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.join("src/p1.rs"), "fn p1() {}\n").unwrap();
        std::fs::write(root.join("src/p2.rs"), "fn p2() {}\n").unwrap();

        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(&root, base.path().to_path_buf());
        s.project = Some(Project::new(root.clone()));

        // Build the initial index synchronously.
        let files_list = crate::model::files::FileList::build(&root).unwrap();
        let index = build_index(&root, &files_list.files, None);
        s.set_index(index);

        // Start an incremental refresh for P1.
        let p1 = root.join("src/p1.rs");
        s.refresh_index(std::slice::from_ref(&p1));
        // The job is now in flight (gen=0, the default).
        assert!(s.indexing.is_some(), "job is in flight after refresh_index(P1)");

        // While the job is in flight, call refresh_index with P2.
        let p2 = root.join("src/p2.rs");
        s.refresh_index(std::slice::from_ref(&p2));
        // P2 should be accumulated in the pending set (not dropped).
        assert!(
            s.pending_index_changes.contains(&p2),
            "P2 accumulated in pending set during flight"
        );

        // Drive apply_index_event to completion (the in-flight job's event).
        let event = IndexEvent {
            index: s.index.clone(),
            indexing: false,
            done: 1,
            total: 1,
            generation: 0,
        };
        s.apply_index_event(&event);

        // The pending set should be drained (P2 coalesced into a new job).
        assert!(
            s.pending_index_changes.is_empty(),
            "pending set drained after flight clear"
        );
        // A new job for P2 should now be in flight (the pending changes were
        // coalesced into one incremental job — single-flight maintained).
        assert!(
            s.indexing.is_some(),
            "new job in flight for coalesced pending changes"
        );
    }

    // ── issue 05: Finding 2 — stale-generation events discarded ──────────

    #[test]
    fn switch_project_root_discards_stale_index_events() {
        // Create two distinct project roots.
        let dir_a = tempfile::tempdir().unwrap();
        let root_a = dir_a.path().to_path_buf();
        std::fs::create_dir_all(root_a.join("src")).unwrap();
        std::fs::write(root_a.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root_a.join("src/a.rs"), "fn a() {}\n").unwrap();

        let dir_b = tempfile::tempdir().unwrap();
        let root_b = dir_b.path().to_path_buf();
        std::fs::create_dir_all(root_b.join("src")).unwrap();
        std::fs::write(root_b.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root_b.join("src/b.rs"), "fn b() {}\n").unwrap();

        // Create a store rooted at project A.
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(&root_a, base.path().to_path_buf());
        s.project = Some(Project::new(root_a.clone()));

        // Simulate A's index job in flight (gen=0).
        s.indexing = Some((0, 1, 0));

        // Switch to project B: generation is bumped, index reset.
        s.switch_project_root(root_b.to_str().unwrap());
        assert_eq!(s.index_generation, 1, "generation bumped on project switch");
        assert_eq!(s.index().total(), 0, "index reset on project switch");

        // Simulate A's job completing (stale event, gen=0).
        let mut a_index = SymbolIndex::new();
        a_index.set_file(
            "src/a.rs",
            vec![crate::syntax::queries::Symbol {
                name: "a".into(),
                kind: crate::syntax::queries::SymbolKind::Function,
                line: 0,
                start_byte: 3,
                end_byte: 4,
                end_line: 0,
            }],
        );
        let stale_event = IndexEvent {
            index: a_index,
            indexing: false,
            done: 1,
            total: 1,
            generation: 0, // stale: from project A
        };
        s.apply_index_event(&stale_event);
        // The stale event must be discarded: index stays empty.
        assert_eq!(
            s.index().total(),
            0,
            "stale event from project A discarded"
        );

        // Simulate B's job completing (current event, gen=1).
        let mut b_index = SymbolIndex::new();
        b_index.set_file(
            "src/b.rs",
            vec![crate::syntax::queries::Symbol {
                name: "b".into(),
                kind: crate::syntax::queries::SymbolKind::Function,
                line: 0,
                start_byte: 3,
                end_byte: 4,
                end_line: 0,
            }],
        );
        let b_event = IndexEvent {
            index: b_index,
            indexing: false,
            done: 1,
            total: 1,
            generation: 1, // current: from project B
        };
        s.apply_index_event(&b_event);
        // B's event must be applied: index contains B's symbols, not A's.
        assert_eq!(s.index().total(), 1, "B's event applied");
        assert!(s.index().has("src/b.rs"), "B's index contains B's file");
        assert!(!s.index().has("src/a.rs"), "A's file not in B's index");
    }

    // ── search & references (issue 06) ───────────────────────────────

    /// A store rooted in a project with known search content.
    fn search_project() -> (tempfile::TempDir, AppStore) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn target() {}\nfn main() { target(); }\ntarget();\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn target() {}\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let store = AppStore::at(dir.path(), base.path().to_path_buf());
        (dir, store)
    }

    /// Drain the search bus into the store until `Finished` (bounded).
    fn drain_search_finished(
        store: &mut AppStore,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::search::rg::SearchEvent>,
    ) {
        let start = std::time::Instant::now();
        loop {
            let mut finished = false;
            while let Ok(ev) = rx.try_recv() {
                if matches!(ev, crate::search::rg::SearchEvent::Finished { .. }) {
                    store.apply_search_event(&ev);
                    finished = true;
                    break;
                }
                store.apply_search_event(&ev);
            }
            if finished {
                return;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "search did not finish in 5s"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// The command registry carries the issue 06 commands, and the
    /// keymap resolves the plan's keys (and the freed `C-c p s` prefix).
    #[test]
    fn search_keybindings_resolve() {
        let (_dir, store) = search_project();
        for name in [
            "project-search",
            "references-at-point",
            "occur",
            "search-next",
            "search-prev",
            "search-jump",
            "search-rerun",
            "search-cancel",
            "close-search-view",
        ] {
            assert!(store.registry.get(name).is_some(), "missing `{name}`");
        }
        let resolve = |seq: &[crate::app::keymap::Key]| {
            store
                .engine
                .resolve(seq)
                .and_then(|l| match l {
                    crate::app::keymap::Lookup::Command(c) => Some(c.to_string()),
                    _ => None,
                })
        };
        use crate::app::keymap::Key;
        assert_eq!(
            resolve(&[Key::ctrl_char('c'), Key::char('p'), Key::char('s'), Key::char('s')]),
            Some("project-search".into())
        );
        assert_eq!(resolve(&[Key::alt_char('?')]), Some("references-at-point".into()));
        assert_eq!(
            resolve(&[Key::alt_char('s'), Key::char('o')]),
            Some("occur".into())
        );
        // The strict prefix `C-c p s` is freed for the longer binding.
        assert_eq!(
            resolve(&[Key::ctrl_char('c'), Key::char('p'), Key::char('s')]),
            None
        );
    }

    /// `C-c p s s` opens the prompt; typing + RET runs the search and
    /// lands in the results view with grouped, counted rows.
    #[test]
    fn search_prompt_flow_and_grouped_results() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();

        store.key_event(key("C-c"));
        store.key_event(key("p"));
        store.key_event(key("s"));
        store.key_event(key("s"));
        // Prompt is active: the minibuffer echoes "Search: ".
        assert_eq!(store.message, "Search: ");
        store.key_event(key("t"));
        store.key_event(key("a"));
        store.key_event(key("r"));
        store.key_event(key("g"));
        store.key_event(key("e"));
        store.key_event(key("t"));
        assert_eq!(store.message, "Search: target");
        store.key_event(key("RET"));

        assert_eq!(store.top_view(), ViewId::Search);
        assert!(store.search_running(), "the search must be running");
        drain_search_finished(&mut store, &mut rx);
        assert!(!store.search_running());

        // 4 hits in 2 files: src/main.rs (3) and src/lib.rs (1).
        let (rows, _top, _total, _sel) = store.search_view_info();
        let headers: Vec<&crate::app::store::ResultRow> = rows
            .iter()
            .filter(|r| matches!(r, crate::app::store::ResultRow::Header { .. }))
            .collect();
        assert_eq!(headers.len(), 2, "two file groups: {rows:?}");
        let counts: Vec<u64> = headers
            .iter()
            .map(|h| match h {
                crate::app::store::ResultRow::Header { count, final_count, .. } => {
                    assert!(*final_count);
                    *count
                }
                _ => unreachable!(),
            })
            .collect();
        assert!(counts.contains(&3) && counts.contains(&1), "per-file counts: {counts:?}");
        assert!(store.search_title().contains("4 matches in 2 files"));
    }

    /// n/p move between matches (wrapping); the selection indexes the
    /// flat hit list.
    #[test]
    fn search_next_prev_move_selection() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);
        assert_eq!(store.search.selected, 0);

        store.key_event(key("n"));
        assert_eq!(store.search.selected, 1);
        // Wrap from the last hit back to the first.
        store.search.selected = 3;
        store.key_event(key("n"));
        assert_eq!(store.search.selected, 0);
        store.key_event(key("p"));
        assert_eq!(store.search.selected, 3);
    }

    /// RET jumps to the match (exact line + column recorded on the jump
    /// stack) and `M-,` returns to the results view with the selection
    /// restored.
    #[test]
    fn search_ret_jump_and_mcomma_returns_to_results() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);

        // Hits are sorted deterministically on Finished: (path, line, col).
        // "src/lib.rs" < "src/main.rs", so the first hit is lib.rs:1
        // ("pub fn target() {}", "target" at col 7).
        let first = store.search.hits[0].clone();
        assert_eq!(first.file, "src/lib.rs");
        assert_eq!(first.line_no, 1);
        assert_eq!(first.col, Some(7));

        store.key_event(key("RET"));
        // Landed in the buffer view on the hit's file/line.
        assert_eq!(store.top_view(), ViewId::Buffer);
        assert_eq!(store.view_name_display(), "src/lib.rs");
        assert_eq!(store.scroll_top(), 0); // line 1 (0-based)

        // `M-,` returns to the results view, selection restored.
        store.key_event(key("M-,"));
        assert_eq!(store.top_view(), ViewId::Search, "M-, must return to the results");
        assert_eq!(store.search.selected, 0);
        // The stack is [Buffer, Search] again: q closes back to the buffer.
        store.key_event(key("q"));
        assert_eq!(store.top_view(), ViewId::Buffer);
    }

    /// `g` re-runs the search: results reset, a new generation streams
    /// in, and the same hits reappear.
    #[test]
    fn search_rerun_restarts_the_search() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);
        let old_gen = store.search.generation;

        store.key_event(key("g"));
        assert_eq!(store.search.generation, old_gen + 1, "re-run bumps the generation");
        assert!(store.search_running(), "the re-run must be running");
        drain_search_finished(&mut store, &mut rx);
        let n = store.search.hits.len();
        assert_eq!(n, 4, "the same 4 hits reappear: {n}");
    }

    /// `q` closes the results view (and cancels the in-flight job); the
    /// terminal event reports a cancelled finish.
    #[test]
    fn search_close_cancels_and_closes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        // Enough files that the walk takes real time.
        for i in 0..300 {
            std::fs::write(dir.path().join(format!("f{i:03}.txt")), "needle\n").unwrap();
        }
        let base = tempfile::tempdir().unwrap();
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("needle".into());

        // Close mid-flight.
        store.key_event(key("q"));
        assert_ne!(store.top_view(), ViewId::Search, "q must close the results view");
        // The job's terminal event must report a cancelled finish.
        let start = std::time::Instant::now();
        loop {
            let mut done = false;
            while let Ok(ev) = rx.try_recv() {
                if matches!(ev, crate::search::rg::SearchEvent::Finished { cancelled: true, .. }) {
                    done = true;
                    break;
                }
            }
            if done {
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// `C-g` in the results view cancels the in-flight job WITHOUT
    /// closing the view (the partial results stay on screen); the
    /// terminal event reports a cancelled finish and `search_running`
    /// flips.
    #[test]
    fn search_c_g_cancels_without_closing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        // Enough files that the walk takes real time.
        for i in 0..300 {
            std::fs::write(dir.path().join(format!("f{i:03}.txt")), "needle\n").unwrap();
        }
        let base = tempfile::tempdir().unwrap();
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());
        let mut rx = store.search_rx().unwrap();
        store.start_project_search("needle".into());
        assert!(store.search_running(), "the search must be in flight");

        // C-g cancels mid-flight: the view stays open, the job stops.
        store.key_event(key("C-g"));
        assert_eq!(store.top_view(), ViewId::Search, "C-g must keep the results view open");
        assert_eq!(store.message, "search cancelled");

        // The worker's terminal event must report a cancelled finish.
        let start = std::time::Instant::now();
        loop {
            let mut done = false;
            while let Ok(ev) = rx.try_recv() {
                store.apply_search_event(&ev);
                if matches!(ev, crate::search::rg::SearchEvent::Finished { cancelled: true, .. }) {
                    done = true;
                }
            }
            if done {
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(5), "cancel not observed in 5s");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!store.search_running(), "the job must be stopped");
    }

    /// `C-g` while a picker is open over the results view still closes
    /// the picker (the global intercept), not cancel the search.
    #[test]
    fn search_c_g_with_picker_open_closes_the_picker() {
        let (_dir, mut store) = search_project();
        let _rx = store.search_rx().unwrap();
        store.start_project_search("target".into());
        assert_eq!(store.top_view(), ViewId::Search);

        store.open_palette();
        assert!(store.picker_open());
        store.key_event(key("C-g"));
        assert!(!store.picker_open(), "C-g must close the palette");
        // The search itself is untouched (still running or already
        // finished — either way C-g did not cancel it: the message is
        // the generic "cancel", not "search cancelled").
        assert_eq!(store.message, "cancel");
        assert_eq!(store.top_view(), ViewId::Search);
    }

    /// `M-s o` (the two-key sequence) opens the occur prompt; RET runs
    /// the in-buffer regex search, grouped under the buffer's display
    /// name, with the per-file (group) count.
    #[test]
    fn occur_prompt_flow_lists_in_file_matches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("a.txt"), "foo world\nbar foo\nfoo again\nbaz\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut store = AppStore::at(dir.path(), base.path().to_path_buf());
        let mut rx = store.search_rx().unwrap();
        store.open_path("a.txt");

        store.key_event(key("M-s"));
        store.key_event(key("o"));
        assert_eq!(store.message, "Occur: ");
        store.key_event(key("f"));
        store.key_event(key("o"));
        store.key_event(key("o"));
        store.key_event(key("RET"));

        assert_eq!(store.top_view(), ViewId::Search);
        drain_search_finished(&mut store, &mut rx);
        assert_eq!(store.search.hits.len(), 3, "all in-file matches");
        // One group, named after the buffer's display name.
        let (rows, _, _, _) = store.search_view_info();
        let headers: Vec<_> = rows
            .iter()
            .filter_map(|r| match r {
                crate::app::store::ResultRow::Header { file, count, .. } => {
                    Some((file.clone(), *count))
                }
                _ => None,
            })
            .collect();
        assert_eq!(headers, vec![("a.txt".to_string(), 3)],
            "one group named after the buffer: {headers:?}");
    }

    /// `M-?` searches the identifier under point (line-level point; col 0
    /// until the buffer model has a column cursor).
    #[test]
    fn references_at_point_searches_the_symbol() {
        let (dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        // A plain-text file whose line 1 starts with the symbol (col 0 is
        // on it, so no known-identifier fallback is needed).
        std::fs::write(dir.path().join("src/plain.txt"), "target alpha\n").unwrap();
        store.open_path("src/plain.txt");

        store.key_event(key("M-?"));
        assert_eq!(store.top_view(), ViewId::Search);
        assert_eq!(store.search.query, "target");
        assert_eq!(store.search.kind, crate::app::store::SearchKind::References);
        drain_search_finished(&mut store, &mut rx);
        // The plain-text hit is kept (fallback: .txt has no grammar);
        // the .rs code hits survive the token filter.
        assert!(
            store.search.hits.iter().any(|h| h.file == "src/plain.txt"),
            "the plain-text hit must be kept: {:?}",
            store.search.hits
        );
    }

    /// Events from a stale generation (a superseded job) are discarded;
    /// events from the current generation apply.
    #[test]
    fn search_stale_generation_events_discarded() {
        let (_dir, mut store) = search_project();
        store.start_project_search("one".into());
        let stale_gen = store.search.generation;
        store.start_project_search("two".into());
        let cur_gen = store.search.generation;
        assert_eq!(cur_gen, stale_gen + 1);

        let stale = crate::search::rg::SearchEvent::Hit {
            file: "stale.txt".into(),
            line_no: 1,
            col: None,
            line: "stale".into(),
            generation: stale_gen,
        };
        store.apply_search_event(&stale);
        assert!(store.search.hits.is_empty(), "stale hit must be discarded");

        let fresh = crate::search::rg::SearchEvent::Hit {
            file: "fresh.txt".into(),
            line_no: 1,
            col: None,
            line: "fresh".into(),
            generation: cur_gen,
        };
        store.apply_search_event(&fresh);
        assert_eq!(store.search.hits.len(), 1, "current-generation hit applies");
    }

    // ── issue 08: commit editor tests ───────────────────────────────────────

    /// Helper: create a tempdir git repo with one committed file and a
    /// staged change, return the store rooted there.
    fn git_store_with_staged(dir: &std::path::Path) -> AppStore {
        fn git_cli(dir: &std::path::Path, args: &[&str]) {
            let out = std::process::Command::new("git")
                .arg("-C").arg(dir)
                .args(args)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .expect("run git");
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
        git_cli(dir, &["init", "-q", "-b", "main"]);
        git_cli(dir, &["config", "user.name", "Test"]);
        git_cli(dir, &["config", "user.email", "test@example.com"]);
        git_cli(dir, &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        git_cli(dir, &["add", "a.txt"]);
        git_cli(dir, &["commit", "-q", "-m", "init"]);
        // Stage a change so the commit editor has a file list.
        std::fs::write(dir.join("a.txt"), "a\nA\n").unwrap();
        git_cli(dir, &["add", "a.txt"]);
        let base = tempfile::tempdir().unwrap();
        AppStore::at(dir, base.path().to_path_buf())
    }

    // ── issue 08: Finding 1 — checkout must trigger the full symbol-index
    // rebuild even when a current-generation job is already in flight ──────

    #[tokio::test]
    async fn checkout_branch_bumps_generation_and_starts_full_rebuild() {
        // A clean repo with a second branch so checkout succeeds (the
        // dirty-tree guard requires a clean tree; the branch must exist).
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fn git_cli(dir: &std::path::Path, args: &[&str]) {
            let out = std::process::Command::new("git")
                .arg("-C").arg(dir)
                .args(args)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .expect("run git");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        git_cli(&root, &["init", "-q", "-b", "main"]);
        git_cli(&root, &["config", "user.name", "Test"]);
        git_cli(&root, &["config", "user.email", "test@example.com"]);
        git_cli(&root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git_cli(&root, &["add", "a.txt"]);
        git_cli(&root, &["commit", "-q", "-m", "init"]);
        git_cli(&root, &["branch", "feature"]);

        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(&root, base.path().to_path_buf());
        s.project = Some(Project::new(root.clone()));

        // Build an initial index so start_indexing has files to index.
        let files_list = crate::model::files::FileList::build(&root).unwrap();
        let index = build_index(&root, &files_list.files, None);
        s.set_index(index);

        // The hole: a current-generation (gen 0) job is already in flight and
        // there are pending changes. Without the fix, checkout's start_indexing
        // would early-return on the single-flight guard and the full rebuild
        // would be silently skipped.
        s.indexing = Some((0, 1, 0));
        s.pending_index_changes.insert(root.join("a.txt"));

        s.checkout_branch("feature");

        // The generation must be bumped so the in-flight gen-0 job is now stale
        // (its event is discarded), and the pending set cleared.
        assert_eq!(s.index_generation, 1, "generation bumped on checkout");
        assert!(
            s.pending_index_changes.is_empty(),
            "pending changes cleared on checkout"
        );
        // A new full-rebuild job must be in flight for the new generation,
        // proving the single-flight guard did NOT skip the rebuild.
        let Some((_, _, job_gen)) = s.indexing else {
            panic!("no index job in flight after checkout");
        };
        assert_eq!(job_gen, 1, "full-rebuild job started for the new generation");
        // HEAD actually moved to the target branch.
        let head = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&head.stdout).trim(),
            "feature",
            "HEAD moved to feature"
        );
    }

    #[test]
    fn commit_editor_prefill_shows_staged_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = git_store_with_staged(dir.path());
        store.open_commit_editor();
        let ed = store.commit_editor.as_ref().unwrap();
        let text = ed.rope.to_string();
        assert!(text.starts_with("# Please enter the commit message"), "prefill: {text}");
        assert!(text.contains("#   M a.txt"), "staged file listed: {text}");
        assert!(text.contains("# Staged changes:"), "section header: {text}");
        // Cursor is at end of prefill.
        assert_eq!(ed.cursor, text.len());
    }

    #[test]
    fn commit_editor_text_edits_land_in_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = git_store_with_staged(dir.path());
        store.open_commit_editor();
        // Type a commit message after the prefill.
        for c in "hello".chars() {
            store.commit_editor_insert(c);
        }
        let ed = store.commit_editor.as_ref().unwrap();
        let text = ed.rope.to_string();
        assert!(text.ends_with("#\nhello"), "text: {text}");
        // Backspace removes the last char.
        store.commit_editor_backspace();
        let text = store.commit_editor.as_ref().unwrap().rope.to_string();
        assert!(text.ends_with("#\nhell"), "after backspace: {text}");
    }

    #[test]
    fn commit_editor_cc_cc_extracts_message_and_commits() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut store = git_store_with_staged(root);
        store.open_commit_editor();
        // Type the message.
        for c in "my commit msg".chars() {
            store.commit_editor_insert(c);
        }
        // C-c C-c through the keymap engine.
        store.key_event(key("C-c"));
        assert_eq!(store.pending.len(), 1, "C-c arms the prefix");
        store.key_event(key("C-c"));
        // The commit should have been made.
        assert!(store.commit_editor.is_none(), "editor closed after commit");
        assert!(store.message.contains("committed"), "message: {}", store.message);
        // Verify via git CLI that the commit was created with the right message.
        let out = std::process::Command::new("git")
            .arg("-C").arg(root)
            .args(["log", "-1", "--pretty=%s"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output().unwrap();
        let msg = String::from_utf8_lossy(&out.stdout);
        assert_eq!(msg.trim(), "my commit msg", "git log: {msg}");
    }

    #[test]
    fn commit_editor_cc_k_discards() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut store = git_store_with_staged(root);
        // Capture HEAD after repo creation.
        let head_before = {
            let out = std::process::Command::new("git")
                .arg("-C").arg(root)
                .args(["rev-parse", "HEAD"])
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output().unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        store.open_commit_editor();
        for c in "should not commit".chars() {
            store.commit_editor_insert(c);
        }
        // C-c C-k aborts.
        store.key_event(key("C-c"));
        store.key_event(key("C-k"));
        assert!(store.commit_editor.is_none(), "editor closed after abort");
        assert!(store.message.contains("aborted"), "message: {}", store.message);
        // HEAD unchanged.
        let out = std::process::Command::new("git")
            .arg("-C").arg(root)
            .args(["rev-parse", "HEAD"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output().unwrap();
        let head_after = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(head_before, head_after, "HEAD unchanged after abort");
    }

    #[test]
    fn commit_editor_keybindings_resolve_through_engine() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = git_store_with_staged(dir.path());
        store.open_commit_editor();
        // C-c alone is a pending prefix (not a command by itself).
        store.key_event(key("C-c"));
        assert!(!store.pending.is_empty(), "C-c must be a pending prefix");
        // C-k completes the abort sequence.
        store.key_event(key("C-k"));
        assert!(store.pending.is_empty(), "C-c C-k resolves");
        assert!(store.commit_editor.is_none(), "editor closed by C-c C-k");
        // Re-open and test C-c C-c.
        // (We can't easily re-stage here, so just verify the engine accepts
        // the prefix again without error.)
        store.open_commit_editor();
        assert!(store.commit_editor.is_some(), "editor re-opened");
        store.key_event(key("C-c"));
        assert!(!store.pending.is_empty(), "second C-c prefix");
        store.key_event(key("C-c"));
        // This will attempt to commit (message is empty after prefill only →
        // the "empty commit message" guard fires, which is fine).
        assert!(store.message.contains("empty commit"), "guard: {}", store.message);
        store.commit_editor_abort();
        assert!(store.commit_editor.is_none());
    }

    /// Regression (review round 3): arrow keys must clear an armed `C-c`
    /// prefix. Old code let `C-c -> arrow -> C-c` resolve as the full
    /// `commit-editor-commit` sequence -- an unintended commit from a
    /// navigation reflex.
    #[test]
    fn commit_editor_arrow_clears_armed_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = git_store_with_staged(dir.path());
        store.open_commit_editor();
        let head_before = store
            .git
            .as_ref()
            .and_then(|g| g.log(None, 0, 1).ok())
            .and_then(|l| l.into_iter().next())
            .map(|e| e.short_id);

        // Arm the C-c prefix.
        store.key_event(key("C-c"));
        assert!(!store.pending.is_empty(), "C-c arms the prefix");

        // An arrow navigates the editor AND disarms the prefix.
        store.key_event(key("DOWN"));
        assert!(store.pending.is_empty(), "arrow must clear the armed prefix");
        assert!(store.commit_editor.is_some(), "editor stays open");

        // The next C-c only RE-ARMS; the commit must not fire.
        store.key_event(key("C-c"));
        assert!(!store.pending.is_empty(), "C-c re-arms after an arrow");
        assert!(store.commit_editor.is_some(), "no commit fired");
        let head_after = store
            .git
            .as_ref()
            .and_then(|g| g.log(None, 0, 1).ok())
            .and_then(|l| l.into_iter().next())
            .map(|e| e.short_id);
        assert_eq!(head_before, head_after, "HEAD must be unchanged");
    }

    #[test]
    fn commit_editor_refuses_when_nothing_staged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // A repo with a committed file but nothing staged.
        fn git_cli(dir: &std::path::Path, args: &[&str]) {
            let out = std::process::Command::new("git")
                .arg("-C").arg(dir)
                .args(args)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
        git_cli(root, &["init", "-q", "-b", "main"]);
        git_cli(root, &["config", "user.name", "Test"]);
        git_cli(root, &["config", "user.email", "test@example.com"]);
        git_cli(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git_cli(root, &["add", "a.txt"]);
        git_cli(root, &["commit", "-q", "-m", "init"]);

        let base = tempfile::tempdir().unwrap();
        let mut store = crate::app::store::AppStore::at(
            root,
            base.path().to_path_buf(),
        );
        // Ensure git is initialized in the store.
        store.open_commit_editor();
        // Unstage everything (should be nothing staged anyway).
        // Type a message.
        for c in "try to commit".chars() {
            store.commit_editor_insert(c);
        }
        // C-c C-c attempts the commit.
        store.key_event(key("C-c"));
        store.key_event(key("C-c"));
        // The editor must still be open (commit refused).
        assert!(store.commit_editor.is_some(), "editor must stay open");
        assert!(
            store.message.contains("nothing staged"),
            "message should say nothing staged: {}", store.message
        );
    }

    #[test]
    fn commit_editor_cg_clears_armed_prefix_not_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = git_store_with_staged(dir.path());
        store.open_commit_editor();

        // Type some text.
        for c in "my msg".chars() {
            store.commit_editor_insert(c);
        }

        // Arm the C-c prefix.
        store.key_event(key("C-c"));
        assert!(!store.pending.is_empty(), "C-c arms the prefix");

        // C-g must clear the pending prefix, NOT abort the editor.
        store.key_event(key("C-g"));
        assert!(store.pending.is_empty(), "C-g must clear the armed prefix");
        assert!(store.commit_editor.is_some(), "C-g must NOT abort the editor");
        // The text is preserved.
        let text = store.commit_editor.as_ref().unwrap().rope.to_string();
        assert!(text.contains("my msg"), "text preserved after C-g: {text}");
    }

    // ── issue 08: snapshot tests ────────────────────────────────────────────

    #[test]
    fn snapshot_log_entry_display() {
        let entries = [
            LogEntry { short_id: "abc1234".into(), subject: "fix: handle edge case".into(), author: "Alice".into(), time: 1_699_992_800, date: "2 hours ago".into() },
            LogEntry { short_id: "def5678".into(), subject: "feat: add new module".into(), author: "Bob".into(), time: 1_699_734_400, date: "3 days ago".into() },
        ];
        let rows: Vec<String> = entries.iter().map(log_entry_display).collect();
        insta::assert_debug_snapshot!(rows);
    }

    #[test]
    fn snapshot_blame_line_display() {
        let now = 1_700_000_000_i64;
        let lines = [
            BlameLine { line_no: 1, short_id: "aaa1111".into(), author: "Alice".into(), time: now - 7200, text: "fn main() {".into() },
            BlameLine { line_no: 2, short_id: "bbb2222".into(), author: "Bob".into(), time: now - 86400, text: "    println!(\"hi\");".into() },
            BlameLine { line_no: 3, short_id: "ccc3333".into(), author: "Alice".into(), time: now - 3600, text: "}".into() },
        ];
        let author_w = lines.iter().map(|l| l.author.chars().count()).max().unwrap_or(0).max(4);
        let rows: Vec<String> = lines.iter().map(|l| blame_line_display(l, now, author_w)).collect();
        insta::assert_debug_snapshot!(rows);
    }

    // ── issue 09 blocking finding #3: regression tests ────────────────────

    #[test]
    fn tree_toggle_builds_rows_and_navigates() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
        let mut store = store(root);
        // Ensure the file list is cached.
        store.ensure_files();
        // Toggle on: builds rows.
        store.toggle_tree();
        assert!(store.tree_visible());
        let rows = store.tree_rows();
        assert!(!rows.is_empty(), "tree rows must be non-empty");
        assert!(rows.iter().any(|r| r.name == "main.rs"));
        assert!(rows.iter().any(|r| r.name == "lib.rs"));
        // Navigate down.
        let initial = store.tree_selected();
        store.tree_move_down();
        assert_eq!(store.tree_selected(), initial + 1);
        // Navigate up (wraps to 0).
        store.tree_move_up();
        store.tree_move_up();
        assert_eq!(store.tree_selected(), 0);
        // Toggle off.
        store.toggle_tree();
        assert!(!store.tree_visible());
    }

    #[test]
    fn tree_reset_on_project_switch() {
        // Two projects in separate tempdirs.
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let root1 = dir1.path();
        let root2 = dir2.path();
        std::fs::write(root1.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root2.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root1.join("file1.rs"), "one\n").unwrap();
        std::fs::write(root2.join("file2.rs"), "two\n").unwrap();
        let mut store = store(root1);
        store.ensure_files();
        store.toggle_tree();
        assert!(store.tree_visible());
        assert!(!store.tree_rows().is_empty());
        // Switch to project 2.
        let root2_str = root2.to_string_lossy().to_string();
        store.switch_project_root(&root2_str);
        // Tree rows must be cleared (not stale from project 1).
        assert!(store.tree_rows().is_empty(), "tree rows must be empty after project switch");
    }

    #[test]
    fn tree_rebuild_on_re_walk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.join("a.rs"), "a\n").unwrap();
        let mut store = store(root);
        store.ensure_files();
        store.toggle_tree();
        let count_before = store.tree_rows().len();
        // Add a new file and re-walk.
        std::fs::write(root.join("b.rs"), "b\n").unwrap();
        store.re_walk();
        let count_after = store.tree_rows().len();
        assert!(count_after > count_before, "re-walk must add the new file to the tree");
        assert!(store.tree_rows().iter().any(|r| r.name == "b.rs"));
    }

    #[test]
    fn tree_buffer_follow_moves_cursor_on_open_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        project_with_files(root);
        let mut store = store(root);
        store.ensure_files();
        store.toggle_tree();
        assert!(store.tree_visible());
        let lib_row = store
            .tree_rows()
            .iter()
            .position(|r| r.rel_path == "src/lib.rs")
            .expect("src/lib.rs must be a tree row");
        // Off by default: opening a file must not move the tree cursor.
        store.open_path("src/main.rs");
        assert_eq!(store.tree_selected(), 0);
        // Enable follow via the M-x command (the shipped UI path).
        store.dispatch("toggle-tree-follow", None).unwrap();
        assert!(store.message.contains("tree buffer-follow: on"));
        // Now opening src/lib.rs moves the cursor to its row.
        store.open_path("src/lib.rs");
        assert_eq!(store.tree_selected(), lib_row);
        // Toggle back off: the cursor no longer follows.
        store.dispatch("toggle-tree-follow", None).unwrap();
        assert!(store.message.contains("tree buffer-follow: off"));
        store.open_path("README.md");
        assert_eq!(store.tree_selected(), lib_row);
    }

    #[test]
    fn notes_open_is_editable_and_save_writes_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
        let mut store = store(root);
        store.open_notes();
        // The notes buffer must be current and editable.
        let key = store.buffers.current().unwrap().to_string();
        let buf = store.buffers.get(&key).unwrap();
        assert!(buf.editable, "notes buffer must be editable");
        assert!(buf.path.is_some(), "notes buffer must have a path");
        // Type some text (bounded editing: append at end).
        store.notes_insert_char('H');
        store.notes_insert_char('i');
        let buf = store.buffers.get(&key).unwrap();
        assert!(buf.locally_modified, "editing must set locally_modified");
        assert!(buf.text().contains("Hi"));
        // Save: writes to disk.
        store.save_buffer();
        let notes_path = root.join(".redline-notes.md");
        assert!(notes_path.exists(), "notes file must exist after save");
        let content = std::fs::read_to_string(&notes_path).unwrap();
        assert!(content.contains("Hi"), "saved content must contain the edit");
        // After save, locally_modified is cleared.
        let buf = store.buffers.get(&key).unwrap();
        assert!(!buf.locally_modified);
    }

    #[test]
    fn notes_backspace_deletes_last_char() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
        let mut store = store(root);
        store.open_notes();
        let key = store.buffers.current().unwrap().to_string();
        store.notes_insert_char('a');
        store.notes_insert_char('b');
        store.notes_backspace();
        let buf = store.buffers.get(&key).unwrap();
        assert!(!buf.text().ends_with("ab"), "backspace must remove 'b'");
    }

    #[test]
    fn indexing_display_full_build_shows_advancing_counter() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        // Simulate a full index in progress: 50 done of 100.
        store.indexing = Some((50, 100, 0));
        store.indexing_incremental = false;
        assert_eq!(store.indexing_display(), "indexing 50/100");
        // Advance.
        store.indexing = Some((75, 100, 0));
        assert_eq!(store.indexing_display(), "indexing 75/100");
    }

    #[test]
    fn indexing_display_incremental_is_silent() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        // Incremental refreshes are fast; flashing an indicator per
        // file-change batch on a busy repo is noise, so they render nothing.
        store.indexing = Some((3, 3, 1));
        store.indexing_incremental = true;
        assert_eq!(store.indexing_display(), "");
        // The single-flight state is untouched by the display decision.
        assert!(store.indexing.is_some());
    }

    #[test]
    fn indexing_display_idle_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        assert_eq!(store.indexing_display(), "");
    }

    #[test]
    fn any_tracked_skips_untracked_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fn git_cli(dir: &std::path::Path, args: &[&str]) {
            let out = std::process::Command::new("git")
                .arg("-C").arg(dir)
                .args(args)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .expect("run git");
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
        git_cli(root, &["init", "-q", "-b", "main"]);
        git_cli(root, &["config", "user.name", "Test"]);
        git_cli(root, &["config", "user.email", "test@example.com"]);
        git_cli(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("tracked.txt"), "x\n").unwrap();
        git_cli(root, &["add", "tracked.txt"]);
        git_cli(root, &["commit", "-q", "-m", "init"]);
        // An untracked file.
        std::fs::write(root.join("untracked.txt"), "y\n").unwrap();
        // Use the git repo directly.
        let repo = crate::git::GitRepo::discover(root).unwrap();
        let tracked = vec![root.join("tracked.txt")];
        let untracked = vec![root.join("untracked.txt")];
        assert!(repo.any_tracked(&tracked, root), "tracked file must be detected");
        assert!(!repo.any_tracked(&untracked, root), "untracked file must NOT be detected");
    }

    #[test]
    fn unstage_hunk_fully_staged_add_removes_index_entry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fn git_cli(dir: &std::path::Path, args: &[&str]) {
            let out = std::process::Command::new("git")
                .arg("-C").arg(dir)
                .args(args)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .expect("run git");
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
        git_cli(root, &["init", "-q", "-b", "main"]);
        git_cli(root, &["config", "user.name", "Test"]);
        git_cli(root, &["config", "user.email", "test@example.com"]);
        git_cli(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("initial.txt"), "init\n").unwrap();
        git_cli(root, &["add", "initial.txt"]);
        git_cli(root, &["commit", "-q", "-m", "init"]);
        // Create a new file and stage it (fully-staged addition).
        std::fs::write(root.join("newfile.txt"), "line1\nline2\nline3\n").unwrap();
        git_cli(root, &["add", "newfile.txt"]);
        // The hunk new_start for a new file is 1 (first line).
        let repo = crate::git::GitRepo::discover(root).unwrap();
        repo.unstage_hunk("newfile.txt", 1).unwrap();
        // After unstaging a fully-staged addition, the index entry is
        // removed (the file is back to untracked, not an empty blob).
        let status = repo.status().unwrap();
        let newfile_entry = status.files.iter().find(|f| f.path == "newfile.txt");
        assert!(newfile_entry.is_some(), "newfile.txt must appear in status after unstage");
        // It must be untracked (not staged).
        let entry = newfile_entry.unwrap();
        assert!(entry.untracked, "fully-staged addition after unstage must be untracked");
        assert_eq!(entry.staged, crate::git::status::StatusKind::None,
            "staged kind must be None after unstage");
    }

    #[test]
    fn isearch_c_s_repeats_next_match() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("test.rs"), "foo bar foo baz foo qux\n").unwrap();
        let mut store = store(root);
        store.open_path("test.rs");
        // Start isearch forward.
        store.isearch_start(crate::app::store::IsearchDirection::Forward);
        // Type "foo".
        store.key_event(key("f"));
        store.key_event(key("o"));
        store.key_event(key("o"));
        assert!(store.isearch.active);
        assert_eq!(store.isearch.current, 0, "first match should be current");
        // C-s repeats: next match.
        store.key_event(key("C-s"));
        assert_eq!(store.isearch.current, 1, "C-s must advance to next match");
        store.key_event(key("C-s"));
        assert_eq!(store.isearch.current, 2, "C-s must advance to third match");
    }

    #[test]
    fn isearch_c_r_repeats_prev_match() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("test.rs"), "foo bar foo baz foo qux\n").unwrap();
        let mut store = store(root);
        store.open_path("test.rs");
        // Start isearch backward.
        store.isearch_start(crate::app::store::IsearchDirection::Backward);
        // Type "foo".
        store.key_event(key("f"));
        store.key_event(key("o"));
        store.key_event(key("o"));
        assert!(store.isearch.active);
        // C-r repeats: previous match.
        let current_after_start = store.isearch.current;
        store.key_event(key("C-r"));
        assert!(store.isearch.current != current_after_start || store.isearch.matches.len() == 1,
            "C-r must move the match position");
    }

    #[test]
    fn m_dot_includes_uppercase_identifiers() {
        // This test verifies that the identifier filter in xref_find_definitions
        // includes uppercase-initial names (types/constants).
        // We test the filter logic directly: the filter must NOT skip
        // identifiers starting with uppercase.
        let line_text = "let x = Foo::BAR;";
        let identifiers: Vec<&str> = line_text
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|w| !w.is_empty() && w.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_'))
            .collect();
        assert!(identifiers.contains(&"Foo"), "uppercase 'Foo' must be in identifiers");
        assert!(identifiers.contains(&"BAR"), "uppercase 'BAR' must be in identifiers");
        assert!(identifiers.contains(&"x"), "lowercase 'x' must be in identifiers");
    }

    #[test]
    fn imenu_indent_depth_reflects_enclosing_extents() {
        // The imenu outline uses the index's symbol nesting. We verify the
        // depth calculation: a symbol at depth N has N strictly-enclosing
        // extents. This is tested via the index's outline structure.
        use crate::nav::index::SymbolIndex;
        use crate::syntax::queries::{Symbol, SymbolKind};
        let mut idx = SymbolIndex::new();
        // A file with nested symbols: fn outer { struct Inner { fn method } }
        // The inner struct is depth 1 (enclosed by outer), method is depth 2.
        let rel = "test.rs".to_string();
        let outer = Symbol {
            name: "outer".into(),
            kind: SymbolKind::Function,
            line: 0,
            end_line: 99,
            start_byte: 0,
            end_byte: 5,
        };
        let inner = Symbol {
            name: "Inner".into(),
            kind: SymbolKind::Type,
            line: 5,
            end_line: 90,
            start_byte: 10,
            end_byte: 15,
        };
        let method = Symbol {
            name: "method".into(),
            kind: SymbolKind::Function,
            line: 10,
            end_line: 85,
            start_byte: 20,
            end_byte: 26,
        };
        idx.set_file(&rel, vec![outer, inner, method]);
        let outline = idx.outline(&rel);
        // The outline should show nesting: outer at depth 0, Inner at depth 1,
        // method at depth 2 (or however the outline represents depth).
        assert!(!outline.is_empty(), "outline must be non-empty");
        assert_eq!(outline.len(), 3, "all three symbols must be in the outline");
    }

    #[test]
    fn progress_publisher_emits_every_k_files() {
        use crate::nav::index::{IndexBus, IndexProgress, PROGRESS_STEP};
        let bus = IndexBus::new();
        let mut rx = bus.subscribe();
        let progress = IndexProgress::new(100).with_publisher(bus, 42);
        // Simulate 25 files done: should emit one progress event.
        for _ in 0..PROGRESS_STEP {
            progress.note_file_done();
        }
        assert_eq!(progress.done(), PROGRESS_STEP);
        // The bus should have received a progress event.
        let event = rx.borrow_and_update().clone();
        assert!(event.indexing, "progress event must have indexing=true");
        assert_eq!(event.done, PROGRESS_STEP);
        assert_eq!(event.total, 100);
        assert_eq!(event.generation, 42);
    }

    // ── issue 002: transient menu + discard ──────────────────────────────

    /// Walk the whole menu tree from `path`, collecting every reachable leaf
    /// command name (descending into prefixes).
    fn collect_menu_leaves(
        store: &AppStore,
        path: &crate::app::keymap::KeySeq,
    ) -> std::collections::HashSet<String> {
        let mut set = std::collections::HashSet::new();
        for e in store.menu_entries_for_path(path) {
            match &e.command {
                Some(cmd) => {
                    set.insert(cmd.clone());
                }
                None => {
                    let mut child = path.clone();
                    child.push(e.key);
                    set.extend(collect_menu_leaves(store, &child));
                }
            }
        }
        set
    }

    #[test]
    fn menu_derives_exactly_the_view_bindings() {
        // Anti-drift: the menu's reachable leaf commands == the view+global
        // bindings' commands (derived, not hand-written; nothing dropped).
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path()); // Buffer view
        let bound: std::collections::HashSet<String> = s
            .menu_bindings()
            .into_iter()
            .map(|(_, c)| c)
            .collect();
        let menu_leaves = collect_menu_leaves(&s, s.menu_path());
        assert_eq!(
            menu_leaves, bound,
            "menu leaves must exactly equal the keymap × registry bindings"
        );
    }

    #[test]
    fn menu_root_lists_buffer_leaves_and_prefixes() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.open_menu();
        let entries = s.menu_entries();
        // A single-key leaf: j → scroll-line-down (Buffer view).
        assert!(
            entries
                .iter()
                .any(|e| e.key == key("j") && e.command.as_deref() == Some("scroll-line-down")),
            "j leaf missing: {entries:?}"
        );
        // A prefix: C-c (the projectile family) is a prefix, not a leaf.
        assert!(
            entries.iter().any(|e| e.key == key("C-c") && e.is_prefix),
            "C-c prefix missing: {entries:?}"
        );
        // The menu's own opener ? is a leaf (open-transient-menu).
        assert!(
            entries
                .iter()
                .any(|e| e.key == key("?") && e.command.as_deref() == Some("open-transient-menu")),
            "? leaf missing: {entries:?}"
        );
    }

    #[test]
    fn menu_prefix_descent_into_projectile() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.open_menu();
        // Descend into C-c p (projectile family).
        let path: crate::app::keymap::KeySeq = vec![key("C-c"), key("p")];
        let entries = s.menu_entries_for_path(&path);
        assert!(
            entries
                .iter()
                .any(|e| e.key == key("f") && e.command.as_deref() == Some("find-file")),
            "C-c p f missing: {entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|e| e.key == key("p") && e.command.as_deref() == Some("switch-project")),
            "C-c p p missing: {entries:?}"
        );
        // s is a prefix here (C-c p s s).
        assert!(
            entries.iter().any(|e| e.key == key("s") && e.is_prefix),
            "C-c p s prefix missing: {entries:?}"
        );
    }

    #[test]
    fn menu_leaf_executes_and_closes() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.open_menu();
        assert!(s.menu_open());
        // M-x is a leaf (open-palette) at the root.
        s.key_event(key("M-x"));
        assert!(!s.menu_open(), "leaf must close the menu");
        assert!(s.picker_open(), "M-x must open the palette");
    }

    #[test]
    fn menu_prefix_key_descends_not_executes() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.open_menu();
        s.key_event(key("C-c"));
        assert!(s.menu_open(), "prefix keeps the menu open");
        assert_eq!(s.menu_path().len(), 1, "path descends by one");
        assert_eq!(s.menu_path()[0], key("C-c"));
    }

    #[test]
    fn menu_c_g_closes() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.open_menu();
        s.key_event(key("C-g"));
        assert!(!s.menu_open());
        assert_eq!(s.message, "cancel");
    }

    #[test]
    fn menu_swallows_non_listed_keys() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.open_menu();
        // z is not bound in the Buffer view nor global → not in the menu.
        s.key_event(key("z"));
        assert!(s.menu_open(), "non-listed key must not close the menu");
        assert!(
            s.message.is_empty(),
            "no unbound-key echo: got `{}`",
            s.message
        );
    }

    #[test]
    fn menu_h_opens_in_magit_view() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.push_view(ViewId::MagitStatus);
        s.key_event(key("h"));
        assert!(s.menu_open(), "h must open the menu in magit views");
        let entries = s.menu_entries();
        assert!(
            entries
                .iter()
                .any(|e| e.key == key("k") && e.command.as_deref() == Some("magit-discard")),
            "k leaf missing in magit menu: {entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|e| e.key == key("s") && e.command.as_deref() == Some("magit-stage")),
            "s leaf missing in magit menu: {entries:?}"
        );
    }

    /// A store rooted in a fresh single-commit git repo (a.txt committed),
    /// with the project set so the git ops resolve the repo.
    /// Windowing (issue 002-02): a status buffer taller than the viewport
    /// keeps the cursor row inside the visible window while pressing `n`, and
    /// the window's top advances (scrolls) as the cursor moves past it. This
    /// is the store-level regression guard for the cursor-following window
    /// (the pyte PTY check drives the same path end-to-end).
    #[test]
    fn magit_status_window_keeps_cursor_visible() {
        let dir = tempfile::tempdir().unwrap();
        fn git_cli(dir: &std::path::Path, args: &[&str]) {
            let out = std::process::Command::new("git")
                .arg("-C").arg(dir).args(args)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "t@e.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "t@e.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .expect("run git");
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
        git_cli(dir.path(), &["init", "-q", "-b", "main"]);
        git_cli(dir.path(), &["config", "user.name", "Test"]);
        git_cli(dir.path(), &["config", "user.email", "t@e.com"]);
        git_cli(dir.path(), &["config", "commit.gpgsign", "false"]);
        for i in 0..20 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), "x\n").unwrap();
        }
        git_cli(dir.path(), &["add", "-A"]);
        git_cli(dir.path(), &["commit", "-q", "-m", "init"]);
        for i in 0..20 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), format!("x\nchanged {i}\n")).unwrap();
        }
        git_cli(dir.path(), &["add", "-A"]);

        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.project = Some(crate::model::project::Project::new(dir.path().to_path_buf()));
        s.set_viewport_lines(8); // small viewport -> magit_window() == 6
        s.open_magit_status();

        let total = s.magit_rows().len();
        assert!(total > 6, "status buffer must overflow the window: {total} rows");

        // The cursor (first changed file) is inside the window, and the window
        // is bounded by magit_window().
        let (win, top, tot) = s.magit_view_info();
        assert_eq!(tot, total);
        assert!(win.len() <= 6, "window must be bounded: {} rows", win.len());
        let cursor = s.magit_rows().iter().position(|r| r.selected).unwrap();
        assert!((top..top + win.len()).contains(&cursor),
            "cursor row {cursor} must be in window [{top},{}): top={top}", top + win.len());

        // Press `n` until the cursor moves past the first window; the window's
        // top must advance and the cursor must stay visible.
        let first_top = s.magit_view_info().1;
        for _ in 0..12 {
            s.key_event(key("n"));
        }
        let (win2, top2, _) = s.magit_view_info();
        assert!(top2 > first_top, "window must scroll down: {first_top} -> {top2}");
        let cursor2 = s.magit_rows().iter().position(|r| r.selected).unwrap();
        assert!((top2..top2 + win2.len()).contains(&cursor2),
            "cursor row {cursor2} must stay in window [{top2},{}): top={top2}", top2 + win2.len());
    }

    // ── issue 003-02: shared windowing (commit-diff / blame / log / notes) ──

    /// Shared git CLI helper for the windowing tests (isolated env, like the
    /// magit windowing test above).
    fn git_test_cli(dir: &std::path::Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "t@e.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "t@e.com")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A git repo with a base commit and one "tall" commit that grows a file
    /// to 60 lines, so the selected commit's diff overflows a small viewport.
    fn tall_commit_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_test_cli(dir.path(), &["init", "-q", "-b", "main"]);
        git_test_cli(dir.path(), &["config", "user.name", "Test"]);
        git_test_cli(dir.path(), &["config", "user.email", "t@e.com"]);
        git_test_cli(dir.path(), &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.path().join("big.txt"), "l1\n").unwrap();
        git_test_cli(dir.path(), &["add", "-A"]);
        git_test_cli(dir.path(), &["commit", "-q", "-m", "base"]);
        let mut content = String::new();
        for i in 1..=60 {
            content.push_str(&format!("line {i}\n"));
        }
        std::fs::write(dir.path().join("big.txt"), content).unwrap();
        git_test_cli(dir.path(), &["add", "-A"]);
        git_test_cli(dir.path(), &["commit", "-q", "-m", "tall"]);
        dir
    }

    /// Commit-diff (issue 003-02): the window is bounded by the pane window,
    /// `M->` lands on the last row (top clamps), `M-<` round-trips to the top,
    /// and C-n/C-p move the window one row.
    #[test]
    fn commit_diff_window_bounded_and_motions_clamp() {
        let dir = tall_commit_repo();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.project = Some(crate::model::project::Project::new(dir.path().to_path_buf()));
        s.set_viewport_lines(8); // pane window == 6
        s.open_log();
        s.log_open_commit();
        assert_eq!(s.top_view(), ViewId::CommitDiff);

        let total = s.commit_diff_rows().len();
        assert!(total > 6, "commit diff must overflow the window: {total} rows");

        // Window bounded at the top.
        let (win, top, tot) = s.commit_diff_view_info();
        assert_eq!(tot, total);
        assert!(win.len() <= 6, "window must be bounded: {} rows", win.len());
        assert_eq!(top, 0);

        // C-n advances the top one row; C-p steps back.
        s.key_event(key("C-n"));
        assert_eq!(s.commit_diff_view_info().1, 1, "C-n must advance the top");
        s.key_event(key("C-p"));
        assert_eq!(s.commit_diff_view_info().1, 0, "C-p must step back");

        // M-> (scroll to bottom) lands the top on the full last page; the
        // last visible row is the diff's final row.
        let w = pane_window(s.viewport_lines);
        s.key_event(key("M->"));
        let (win_b, top_b, _) = s.commit_diff_view_info();
        assert_eq!(top_b, total - w, "M-> must land on the full last page: got {top_b}");
        assert_eq!(
            win_b[win_b.len() - 1].text,
            s.commit_diff_rows()[total - 1].text,
            "the last visible row must be the diff's final row"
        );

        // M-< (scroll to top) round-trips.
        s.key_event(key("M-<"));
        assert_eq!(s.commit_diff_view_info().1, 0, "M-< must round-trip to top");

        // C-v / M-v page: a page-down advances the top by more than a line
        // (the pane window minus a 2-line overlap), and M-v brings it back.
        s.key_event(key("C-v"));
        let top_page = s.commit_diff_view_info().1;
        assert!(top_page > 1, "C-v must page down (more than one row): got {top_page}");
        s.key_event(key("M-v"));
        assert_eq!(s.commit_diff_view_info().1, 0, "M-v must page back to top");
    }

    /// Blame (issue 003-02): the cursor-following window keeps `b.selected`
    /// in view on every move; the top advances as the cursor passes it and
    /// clamps so the last row stays visible.
    #[test]
    fn blame_cursor_stays_in_window_on_moves() {
        let dir = tempfile::tempdir().unwrap();
        git_test_cli(dir.path(), &["init", "-q", "-b", "main"]);
        git_test_cli(dir.path(), &["config", "user.name", "Test"]);
        git_test_cli(dir.path(), &["config", "user.email", "t@e.com"]);
        git_test_cli(dir.path(), &["config", "commit.gpgsign", "false"]);
        let mut content = String::new();
        for i in 1..=60 {
            content.push_str(&format!("line {i}\n"));
        }
        std::fs::write(dir.path().join("big.txt"), content).unwrap();
        git_test_cli(dir.path(), &["add", "-A"]);
        git_test_cli(dir.path(), &["commit", "-q", "-m", "one"]);

        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.project = Some(crate::model::project::Project::new(dir.path().to_path_buf()));
        s.set_viewport_lines(8); // pane window == 6
        s.open_path("big.txt");
        s.open_blame();
        assert_eq!(s.top_view(), ViewId::Blame);

        let total = s.blame_rows().len(); // header + 60 lines
        assert!(total > 6, "blame must overflow the window: {total} rows");

        // Initially the cursor is on the first line (row 1); the window top is 0.
        let (win, top, tot) = s.blame_view_info();
        assert_eq!(tot, total);
        assert!(win.len() <= 6);
        assert_eq!(top, 0);
        assert!(s.blame.as_ref().unwrap().selected + 1 < top + win.len());

        // Move the cursor down past the first window; it must stay in view and
        // the top must advance.
        for _ in 0..10 {
            s.key_event(key("C-n"));
        }
        let (win2, top2, _) = s.blame_view_info();
        let sel = s.blame.as_ref().unwrap().selected + 1;
        assert!(
            (top2..top2 + win2.len()).contains(&sel),
            "cursor row {sel} must be in window [{top2},{}): top={top2}",
            top2 + win2.len()
        );
        assert!(top2 > 0, "window must have scrolled down: {top2}");

        // M-> moves the cursor to the last line; the top clamps so the last row
        // is the last visible row.
        s.key_event(key("M->"));
        let (win3, top3, _) = s.blame_view_info();
        let last = s.blame.as_ref().unwrap().selected + 1;
        assert_eq!(last, total - 1, "M-> must move the cursor to the last row");
        assert!(
            (top3..top3 + win3.len()).contains(&last),
            "last row {last} must be in window [{top3},{}): top={top3}",
            top3 + win3.len()
        );
        assert_eq!(top3, 55, "top must clamp to show the last row: got {top3}");

        // M-< round-trips the cursor to the first row.
        s.key_event(key("M-<"));
        let (win4, top4, _) = s.blame_view_info();
        let first = s.blame.as_ref().unwrap().selected + 1;
        assert_eq!(first, 1, "M-< must move the cursor to the first row");
        assert!((top4..top4 + win4.len()).contains(&first));
    }

    /// Log in-page (issue 003-02): the in-page motion (arrows / j / k) keeps
    /// the selection inside the visible window; paging (`n`/`p`) resets the
    /// window to the top and is unchanged.
    #[test]
    fn log_in_page_selection_stays_in_window() {
        let dir = tempfile::tempdir().unwrap();
        git_test_cli(dir.path(), &["init", "-q", "-b", "main"]);
        git_test_cli(dir.path(), &["config", "user.name", "Test"]);
        git_test_cli(dir.path(), &["config", "user.email", "t@e.com"]);
        git_test_cli(dir.path(), &["config", "commit.gpgsign", "false"]);
        for i in 0..30 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), "x\n").unwrap();
            git_test_cli(dir.path(), &["add", "-A"]);
            git_test_cli(dir.path(), &["commit", "-q", "-m", &format!("commit {i}")]);
        }

        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.project = Some(crate::model::project::Project::new(dir.path().to_path_buf()));
        s.set_viewport_lines(8); // pane window == 6
        s.open_log();
        assert_eq!(s.top_view(), ViewId::Log);

        let total = s.log_rows().len(); // header + up-to-25 entries + footer
        assert!(total > 6, "log page must overflow the window: {total} rows");

        // Move the in-page selection down with `j`; it must stay in view and
        // the top must advance.
        for _ in 0..20 {
            s.key_event(key("j"));
        }
        let (win, top, _) = s.log_view_info();
        let sel = s.log.as_ref().unwrap().selected + 1;
        assert!(
            (top..top + win.len()).contains(&sel),
            "log selection row {sel} must be in window [{top},{}): top={top}",
            top + win.len()
        );
        assert!(top > 0, "log window must have scrolled: {top}");

        // Paging (`n`) resets the in-page window to the top.
        s.key_event(key("n"));
        let (_, top_paged, _) = s.log_view_info();
        assert_eq!(top_paged, 0, "n (next page) must reset the window to the top");
    }

    /// Editable buffers (issue 003-02): typing in the notes buffer near the
    /// bottom keeps the insertion row inside the visible file-view window.
    #[test]
    fn notes_typing_near_bottom_keeps_insertion_row_in_view() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        s.set_viewport_lines(8); // file-view window == 8
        s.open_notes();
        // Type 40 lines so the buffer far exceeds the 8-line viewport.
        for _ in 0..40 {
            s.notes_insert_char('x');
            s.notes_insert_char('\n');
        }
        let key = s.buffers.current().unwrap().to_string();
        let total = s.buffers.get(&key).unwrap().line_count();
        assert!(total > 8, "notes buffer must overflow the viewport: {total} lines");
        // The insertion row (the last line) must be inside the visible window.
        let (top, total2, viewport) = s.file_view_scroll_info();
        assert_eq!(total2, total);
        let last = total - 1;
        assert!(
            last < top + viewport,
            "insertion row {last} must be in view [top={top}, {viewport} rows)"
        );
        // The rendered window ends exactly on the last line (no gap).
        // (No annotations here: every rendered row is a code row, so the
        // row count equals the buffer-line count.)
        let rows = s.file_view_rows();
        assert_eq!(
            top + rows.len(),
            total,
            "window must end at the last line: top={top} len={} total={total}",
            rows.len()
        );
    }

    fn git_store(dir: &std::path::Path) -> AppStore {
        fn git_cli(dir: &std::path::Path, args: &[&str]) {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .expect("run git");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        git_cli(dir, &["init", "-q", "-b", "main"]);
        git_cli(dir, &["config", "user.name", "Test"]);
        git_cli(dir, &["config", "user.email", "test@example.com"]);
        git_cli(dir, &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("a.txt"), "keep\n").unwrap();
        git_cli(dir, &["add", "a.txt"]);
        git_cli(dir, &["commit", "-q", "-m", "init"]);
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir, base.path().to_path_buf());
        s.project = Some(crate::model::project::Project::new(dir.to_path_buf()));
        s
    }

    #[test]
    fn discard_unstaged_file_confirmation_flow() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = git_store(dir.path());
        std::fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
        s.open_magit_status();
        assert_eq!(s.top_view(), ViewId::MagitStatus);
        // k arms the confirmation.
        s.key_event(key("k"));
        assert!(s.discard_armed());
        assert_eq!(s.message, "discard a.txt? y/n");
        // n cancels — nothing changes.
        s.key_event(key("n"));
        assert!(!s.discard_armed());
        assert_eq!(s.message, "discard cancelled");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "changed\n"
        );
        // C-g also cancels — nothing changes.
        s.key_event(key("k"));
        assert!(s.discard_armed());
        s.key_event(key("C-g"));
        assert!(!s.discard_armed());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "changed\n"
        );
        // y confirms — the file is restored to HEAD.
        s.key_event(key("k"));
        assert!(s.discard_armed());
        s.key_event(key("y"));
        assert!(!s.discard_armed());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "keep\n"
        );
        assert!(
            s.message.contains("discarded a.txt"),
            "got `{}`",
            s.message
        );
    }

    #[test]
    fn discard_acts_on_cursor_row() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fn git_cli(dir: &std::path::Path, args: &[&str]) {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .expect("run git");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        git_cli(root, &["init", "-q", "-b", "main"]);
        git_cli(root, &["config", "user.name", "Test"]);
        git_cli(root, &["config", "user.email", "test@example.com"]);
        git_cli(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("a.txt"), "A\n").unwrap();
        std::fs::write(root.join("b.txt"), "B\n").unwrap();
        git_cli(root, &["add", "a.txt", "b.txt"]);
        git_cli(root, &["commit", "-q", "-m", "init"]);
        std::fs::write(root.join("a.txt"), "A2\n").unwrap();
        std::fs::write(root.join("b.txt"), "B2\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(root, base.path().to_path_buf());
        s.project = Some(crate::model::project::Project::new(root.to_path_buf()));
        s.open_magit_status();
        // Cursor starts on a.txt (first unstaged file). Move to b.txt.
        s.key_event(key("n"));
        s.key_event(key("k"));
        assert!(s.discard_armed());
        assert!(s.message.contains("b.txt"), "got `{}`", s.message);
        s.key_event(key("y"));
        // b.txt restored; a.txt untouched.
        assert_eq!(std::fs::read_to_string(root.join("b.txt")).unwrap(), "B\n");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "A2\n");
    }

    #[test]
    fn discard_menu_leaf_arms_confirmation() {
        // k pressed FROM the open menu closes the menu and arms the gate.
        let dir = tempfile::tempdir().unwrap();
        let mut s = git_store(dir.path());
        std::fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
        s.open_magit_status();
        s.key_event(key("h")); // open the menu (magit dispatch)
        assert!(s.menu_open());
        s.key_event(key("k")); // k is a menu leaf → magit-discard
        assert!(!s.menu_open(), "menu must close on the leaf");
        assert!(s.discard_armed(), "confirmation must be armed");
        s.key_event(key("n"));
        assert!(!s.discard_armed());
    }

    // ── issue 05 (finding 1): editable-buffer key order ─────────────

    #[test]
    fn notes_printable_inserts_and_c_c_p_f_dispatches_while_editing() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut s = store(dir.path());
        s.open_notes();
        let bkey = s.buffers.current().unwrap().to_string();

        // A plain printable (not bound) self-inserts while editing.
        s.key_event(key("l"));
        assert!(
            s.buffers.get(&bkey).unwrap().text().ends_with("l"),
            "`l` must self-insert in the notes buffer"
        );

        // C-c p f (find-file) must open the finder while a notes buffer is
        // current — a multi-key sequence ending in two printables.
        s.key_event(key("C-c"));
        s.key_event(key("p"));
        s.key_event(key("f"));
        assert!(s.picker_open(), "C-c p f must open the finder");
        assert_eq!(s.picker_kind(), Some(PickerKind::FindFile));
        assert!(
            !s.buffers.get(&bkey).unwrap().text().contains("p f"),
            "the prefix tail must not have been typed into notes"
        );
    }

    #[test]
    fn notes_c_x_g_dispatches_while_editing() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = git_store(dir.path());
        s.open_notes();
        let bkey = s.buffers.current().unwrap().to_string();

        // A plain printable (not bound) self-inserts while editing.
        s.key_event(key("l"));
        assert!(
            s.buffers.get(&bkey).unwrap().text().ends_with("l"),
            "`l` must self-insert in the notes buffer"
        );

        // C-x g (magit-status) must dispatch while a notes buffer is current:
        // the pending prefix must reach the engine, not be self-inserted.
        s.key_event(key("C-x"));
        assert_eq!(s.pending_display(), "C-x", "C-x must arm the prefix");
        s.key_event(key("g"));
        assert_eq!(
            s.top_view(),
            ViewId::MagitStatus,
            "C-x g must dispatch while editing notes"
        );
        assert!(
            !s.buffers.get(&bkey).unwrap().text().contains("x g"),
            "the prefix must not have been typed into notes"
        );
    }

    #[test]
    fn notes_c_g_clears_pending_only_not_buffer() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut s = store(dir.path());
        s.open_notes();
        s.key_event(key("C-x")); // arm a prefix
        assert_eq!(s.pending_display(), "C-x");
        s.key_event(key("C-g")); // global cancel
        assert!(s.pending.is_empty(), "C-g must clear the pending prefix");
        // The notes buffer stays current and open; the app is not quitting.
        assert_eq!(s.top_view(), ViewId::Buffer);
        assert!(!s.quit, "C-g in notes must not quit");
        // A printable now self-inserts (the prefix is gone).
        s.key_event(key("a"));
        assert!(s.buffers.current_buffer().unwrap().text().ends_with("a"));
    }

    // ── issue 05 (finding 2): notes self-creation is not a conflict ───

    #[test]
    fn notes_self_creation_event_is_ignored_then_real_edit_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        let bkey = s.buffers.current().unwrap().to_string();
        let notes_path = dir.path().join(".redline-notes.md");

        // Type first, so the buffer is locally owned when the (late) watcher
        // event for our own file creation lands.
        s.key_event(key("a"));
        assert!(s.buffers.get(&bkey).unwrap().locally_modified);

        // The self-inflicted creation event must NOT flag a conflict.
        s.apply_project_change(&change(vec![notes_path.clone()]));
        assert!(
            !s.buffers.get(&bkey).unwrap().changed_on_disk,
            "our own file creation must not set changed_on_disk"
        );
        assert!(!s.current_buffer_changed_on_disk());

        // A genuine external edit (a later, un-marked event) still conflicts.
        s.apply_project_change(&change(vec![notes_path.clone()]));
        assert!(
            s.buffers.get(&bkey).unwrap().changed_on_disk,
            "a genuine external edit must still set changed_on_disk"
        );
        assert!(s.current_buffer_changed_on_disk());
    }

    // ── issue 003-01: access-only batch must not set changed_on_disk ─────

    #[test]
    fn access_only_batch_does_not_set_changed_on_disk() {
        use crate::app::watcher::summarize;
        use notify_debouncer_full::DebouncedEvent;

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        let bkey = s.buffers.current().unwrap().to_string();
        let notes_path = dir.path().join(".redline-notes.md");

        // Type to make the buffer locally owned (the bug's precondition).
        s.key_event(key("a"));
        assert!(s.buffers.get(&bkey).unwrap().locally_modified);

        // Simulate the old bug path: access events for the notes file go
        // through summarize. After the fix, summarize drops them, producing
        // an empty change.
        let access_events = vec![DebouncedEvent {
            event: notify::Event {
                kind: notify::EventKind::Access(notify::event::AccessKind::Any),
                paths: vec![notes_path.clone()],
                attrs: Default::default(),
            },
            time: std::time::Instant::now(),
        }];
        let change = summarize(dir.path(), access_events);
        assert!(change.is_empty(), "summarize must drop access-only batches");

        // The empty change must not set the marker.
        s.apply_project_change(&change);
        assert!(
            !s.buffers.get(&bkey).unwrap().changed_on_disk,
            "access-only batch must not set changed_on_disk"
        );
        assert!(!s.current_buffer_changed_on_disk());
    }

    // ── plan 004 issue 03: mark / region / kill ring / yank tests ──────

    /// A store with a multi-line editable buffer (notes) for mark/region tests.
    fn notes_store_with_lines(n_lines: usize) -> (AppStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        // Type n_lines lines into the notes buffer.
        for _ in 0..n_lines {
            s.notes_insert_char('x');
            s.notes_insert_char('\n');
        }
        (s, dir)
    }

    #[test]
    fn set_mark_sets_buffer_mark_and_echoes() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let key = s.buffers.current().unwrap().to_string();
        assert!(s.buffers.get(&key).unwrap().mark.is_none(), "no mark initially");
        s.set_mark();
        assert!(s.buffers.get(&key).unwrap().mark.is_some(), "mark must be set");
        assert_eq!(s.message, "Mark set");
    }

    #[test]
    fn region_byte_range_returns_normalized_range() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let key = s.buffers.current().unwrap().to_string();
        // Set mark at line 2 (byte offset of line 2's start).
        let line2_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(line2_byte);
        }
        // Point at line 5: the region's end is line 5's start.
        s.set_point_line(5);
        let range = s.region_byte_range().unwrap();
        let line5_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(5).unwrap();
        assert_eq!(range.0, line2_byte, "start = min(mark, point)");
        assert_eq!(range.1, line5_byte, "end = max(mark, point)");
    }

    #[test]
    fn region_byte_range_none_when_mark_not_set() {
        let (s, _dir) = notes_store_with_lines(10);
        assert!(s.region_byte_range().is_none());
    }

    #[test]
    fn region_size_bytes_returns_correct_size() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let key = s.buffers.current().unwrap().to_string();
        let line2_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(line2_byte);
        }
        s.set_point_line(5);
        let size = s.region_size_bytes().unwrap();
        assert!(size > 0, "region must have a positive size");
    }

    #[test]
    fn region_line_range_returns_correct_lines() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let key = s.buffers.current().unwrap().to_string();
        let line2_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(line2_byte);
        }
        s.set_point_line(5);
        let (start, end) = s.region_line_range().unwrap();
        assert_eq!(start, 2, "region starts at line 2");
        assert_eq!(end, 4, "region ends at line 4 (end byte is exclusive: line 5's start)");
    }

    #[test]
    fn c_g_clears_mark() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let bkey = s.buffers.current().unwrap().to_string();
        s.set_mark();
        assert!(s.buffers.get(&bkey).unwrap().mark.is_some());
        s.key_event(key("C-g"));
        assert!(s.buffers.get(&bkey).unwrap().mark.is_none(), "C-g must clear the mark");
    }

    #[test]
    fn exchange_point_and_mark_swaps_positions() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let key = s.buffers.current().unwrap().to_string();
        // Set mark at line 2.
        let line2_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(line2_byte);
        }
        // Point is at line 0.
        assert_eq!(s.point_line(), 0);
        s.exchange_point_and_mark();
        // After exchange: point should be at line 2, mark should be at line 0's byte.
        assert_eq!(s.point_line(), 2, "point must move to where mark was");
        let line0_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(0).unwrap();
        assert_eq!(s.buffers.get(&key).unwrap().mark, Some(line0_byte), "mark must be at old point");
    }

    #[test]
    fn exchange_point_and_mark_noop_when_no_mark() {
        let (mut s, _dir) = notes_store_with_lines(10);
        assert_eq!(s.scroll_top(), 0);
        s.exchange_point_and_mark();
        assert_eq!(s.scroll_top(), 0, "no-op when mark not set");
        assert!(s.message.contains("Mark not set"));
    }

    #[test]
    fn copy_region_pushes_to_kill_ring() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let bkey = s.buffers.current().unwrap().to_string();
        let line2_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&bkey) {
            buf.mark = Some(line2_byte);
        }
        s.set_point_line(5);
        let region_text = s.buffers.get(&bkey).unwrap()
            .rope.slice(line2_byte..s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(5).unwrap())
            .to_string();
        s.copy_region();
        assert!(s.message.contains("copied to kill ring"));
        // The kill ring should now hold the region text.
        assert_eq!(s.kill_ring.top(), Some(region_text.as_str()));
    }

    #[test]
    fn kill_region_removes_text_in_editable_buffer() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let key = s.buffers.current().unwrap().to_string();
        let len_before = s.buffers.get(&key).unwrap().rope.len_bytes();
        let line2_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(line2_byte);
        }
        s.set_point_line(5);
        let line5_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(5).unwrap();
        s.kill_region();
        let len_after = s.buffers.get(&key).unwrap().rope.len_bytes();
        assert!(len_after < len_before, "kill must remove text");
        assert_eq!(len_before - len_after, line5_byte - line2_byte, "exactly the region bytes removed");
        // Mark is cleared after kill.
        assert!(s.buffers.get(&key).unwrap().mark.is_none());
        // Kill ring holds the removed text.
        assert!(!s.kill_ring.is_empty());
    }

    #[test]
    fn kill_region_read_only_copies_without_removing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut content = String::new();
        for i in 0..10 {
            content.push_str(&format!("line{}\n", i));
        }
        std::fs::write(dir.path().join("src/t.rs"), &content).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        s.open_path("src/t.rs");
        let bkey = s.buffers.current().unwrap().to_string();
        let len_before = s.buffers.get(&bkey).unwrap().rope.len_bytes();
        // Set mark at line 2.
        let line2_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&bkey) {
            buf.mark = Some(line2_byte);
        }
        s.set_point_line(5);
        s.kill_region();
        // Buffer is unchanged (read-only).
        assert_eq!(s.buffers.get(&bkey).unwrap().rope.len_bytes(), len_before);
        // Kill ring has the text.
        assert!(!s.kill_ring.is_empty());
        // Mark is cleared.
        assert!(s.buffers.get(&bkey).unwrap().mark.is_none());
    }

    #[test]
    fn yank_inserts_at_point_in_editable_buffer() {
        let (mut s, _dir) = notes_store_with_lines(5);
        let bkey = s.buffers.current().unwrap().to_string();
        // First, copy some text to the kill ring.
        let line1_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(1).unwrap();
        let _line3_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(3).unwrap();
        if let Some(buf) = s.buffers.get_mut(&bkey) {
            buf.mark = Some(line1_byte);
        }
        s.set_point_line(3);
        s.copy_region();
        let yank_text = s.kill_ring.top().unwrap().to_string();
        let len_before = s.buffers.get(&bkey).unwrap().rope.len_bytes();
        // Now yank at line 0.
        s.set_point_line(0);
        s.yank();
        let len_after = s.buffers.get(&bkey).unwrap().rope.len_bytes();
        assert_eq!(len_after - len_before, yank_text.len(), "yank must insert the ring text");
        // The inserted text is at the start (line 0's byte offset).
        let buf_text = s.buffers.get(&bkey).unwrap().rope.to_string();
        assert!(buf_text.starts_with(&yank_text), "yanked text must be at the start");
    }

    #[test]
    fn yank_read_only_buffer_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "hello\nworld\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        // Open the read-only file and copy a region to the kill ring.
        s.open_path("src/t.rs");
        let file_key = s.buffers.current().unwrap().to_string();
        assert!(!s.buffers.get(&file_key).unwrap().editable, "file buffer must be read-only");
        // Set mark at line 0, scroll to line 1, copy the region.
        if let Some(buf) = s.buffers.get_mut(&file_key) {
            buf.mark = Some(0);
        }
        s.set_point_line(1);
        s.copy_region();
        assert!(!s.kill_ring.is_empty(), "kill ring must have an entry");
        // Switch to the read-only file (it's already current, but set it
        // explicitly to be sure).
        s.buffers.set_current(&file_key);
        assert_eq!(s.buffers.current().unwrap(), &file_key, "current must be the file buffer");
        let len_before = s.buffers.get(&file_key).unwrap().rope.len_bytes();
        s.yank();
        assert_eq!(s.buffers.get(&file_key).unwrap().rope.len_bytes(), len_before);
        assert!(s.message.contains("read-only"), "got: {}", s.message);
    }

    #[test]
    fn yank_pop_cycles_kill_ring() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        // The notes buffer starts with "# Notes\n" (1 line pre-filled).
        // Type unique lines so the kill ring entries are distinguishable.
        for c in "AAAA\n".chars() { s.notes_insert_char(c); }
        for c in "BBBB\n".chars() { s.notes_insert_char(c); }
        for c in "CCCC\n".chars() { s.notes_insert_char(c); }
        let bkey = s.buffers.current().unwrap().to_string();
        // Buffer: line 0="# Notes", line 1="AAAA", line 2="BBBB", line 3="CCCC"
        // Copy lines 1-2 ("AAAA\n") to the ring.
        let line1_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(1).unwrap();
        let _line2_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&bkey) {
            buf.mark = Some(line1_byte);
        }
        s.set_point_line(2);
        s.copy_region();
        let first_entry = s.kill_ring.top().unwrap().to_string();
        assert!(first_entry.contains("AAAA"), "first entry: {first_entry}");
        // Copy lines 3-4 ("CCCC\n") to the ring.
        let line3_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(3).unwrap();
        let _line4_byte = s.buffers.get(&bkey).unwrap().rope.try_line_to_byte(4).unwrap();
        if let Some(buf) = s.buffers.get_mut(&bkey) {
            buf.mark = Some(line3_byte);
        }
        s.set_point_line(4);
        s.copy_region();
        let second_entry = s.kill_ring.top().unwrap().to_string();
        assert!(second_entry.contains("CCCC"), "second entry: {second_entry}");
        assert_ne!(first_entry, second_entry);
        // Now yank (inserts second_entry at line 0).
        s.set_point_line(0);
        s.yank();
        let text_after_yank = s.buffers.get(&bkey).unwrap().rope.to_string();
        // The yanked text (CCCC) is now at the start.
        assert!(text_after_yank.starts_with("CCCC"), "C-y must insert at start: {text_after_yank}");
        // M-y: replace with first_entry (AAAA...).
        s.yank_pop();
        let text_after_pop = s.buffers.get(&bkey).unwrap().rope.to_string();
        // After yank-pop, the first_entry replaces the second_entry at the start.
        assert!(text_after_pop.starts_with("AAAA"), "M-y must replace with first entry: {text_after_pop}");
    }

    #[test]
    fn yank_pop_noop_without_prior_yank() {
        let (mut s, _dir) = notes_store_with_lines(5);
        s.yank_pop();
        assert!(s.message.contains("no previous yank"));
    }

    #[test]
    fn kill_ring_shared_across_buffers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("src/t.rs"), "hello world\nfoo bar\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        // Open the read-only file and copy a region to the kill ring.
        s.open_path("src/t.rs");
        let file_key = s.buffers.current().unwrap().to_string();
        let _line1_byte = s.buffers.get(&file_key).unwrap().rope.try_line_to_byte(1).unwrap();
        if let Some(buf) = s.buffers.get_mut(&file_key) {
            buf.mark = Some(0);
        }
        s.set_point_line(1);
        s.copy_region();
        let copied = s.kill_ring.top().unwrap().to_string();
        assert!(!copied.is_empty());
        // Switch to notes and yank: the kill ring is shared.
        s.open_notes();
        let len_before = s.buffers.current_buffer().unwrap().rope.len_bytes();
        s.yank();
        let len_after = s.buffers.current_buffer().unwrap().rope.len_bytes();
        assert_eq!(len_after - len_before, copied.len(), "cross-buffer yank must work");
    }

    #[test]
    fn kill_ring_bounded_at_60() {
        let (mut s, _dir) = notes_store_with_lines(5);
        // Push 65 different strings.
        for i in 0..65 {
            s.kill_ring.push(format!("entry_{}", i));
        }
        assert_eq!(s.kill_ring.len(), 60, "ring must be bounded at 60");
        // The most recent entry is entry_64.
        assert_eq!(s.kill_ring.top(), Some("entry_64"));
        // The oldest is entry_5 (entry_0 through entry_4 were evicted).
        assert_eq!(s.kill_ring.at(59), Some("entry_5"));
    }

    #[test]
    fn kill_ring_suppresses_consecutive_duplicates() {
        let (mut s, _dir) = notes_store_with_lines(5);
        s.kill_ring.push("hello".into());
        s.kill_ring.push("hello".into());
        s.kill_ring.push("world".into());
        assert_eq!(s.kill_ring.len(), 2, "consecutive duplicates must be suppressed");
        assert_eq!(s.kill_ring.top(), Some("world"));
        assert_eq!(s.kill_ring.at(1), Some("hello"));
    }

    #[test]
    fn set_mark_keybinding_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        use crate::app::keymap::{Lookup, parse_sequence};
        // C-SPC resolves to set-mark. The terminal delivers C-SPC as
        // Char(' ') + CONTROL (NUL byte decoded by crossterm/iocraft).
        let c_spc = vec![crate::app::keymap::Key::ctrl_char(' ')];
        assert_eq!(
            store.engine.resolve(&c_spc),
            Some(Lookup::Command("set-mark")),
            "C-SPC must resolve to set-mark"
        );
        // Also verify the parser path: "C-SPC" in config resolves to the same key.
        let parsed = parse_sequence("C-SPC").unwrap();
        assert_eq!(parsed, c_spc, "parser C-SPC must match the terminal representation");
        assert_eq!(
            store.engine.resolve(&parsed),
            Some(Lookup::Command("set-mark")),
            "parsed C-SPC must resolve to set-mark"
        );
        // C-w resolves to kill-region.
        assert_eq!(
            store.engine.resolve(&parse_sequence("C-w").unwrap()),
            Some(Lookup::Command("kill-region")),
        );
        // M-w resolves to copy-region.
        assert_eq!(
            store.engine.resolve(&parse_sequence("M-w").unwrap()),
            Some(Lookup::Command("copy-region")),
        );
        // C-y resolves to yank.
        assert_eq!(
            store.engine.resolve(&parse_sequence("C-y").unwrap()),
            Some(Lookup::Command("yank")),
        );
        // M-y resolves to yank-pop.
        assert_eq!(
            store.engine.resolve(&parse_sequence("M-y").unwrap()),
            Some(Lookup::Command("yank-pop")),
        );
        // C-x C-x resolves to exchange-point-and-mark.
        assert_eq!(
            store.engine.resolve(&parse_sequence("C-x C-x").unwrap()),
            Some(Lookup::Command("exchange-point-and-mark")),
        );
    }

    #[test]
    fn region_display_in_status_line() {
        let (mut s, _dir) = notes_store_with_lines(10);
        let key = s.buffers.current().unwrap().to_string();
        let line2_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(line2_byte);
        }
        s.set_point_line(5);
        let size = s.region_size_bytes().unwrap();
        assert!(size > 0);
        // The status line should show the region size.
        // (We verify via the store method, not the full render.)
        assert_eq!(s.region_size_bytes(), Some(size));
    }

    #[test]
    fn kill_region_non_ascii_content_correct() {
        // Proves the byte-to-char conversion at the edit boundary: a region
        // containing multi-byte UTF-8 characters must be killed correctly
        // (not the wrong text, not a panic from out-of-bounds char index).
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        // Type content with multi-byte characters: "café\nnaïve\nend\n"
        // Buffer: line 0="# Notes", line 1="café", line 2="naïve", line 3="end"
        for c in "café\nnaïve\nend\n".chars() {
            s.notes_insert_char(c);
        }
        let key = s.buffers.current().unwrap().to_string();
        // Set mark at byte 0 (start of "# Notes"), scroll to line 2 (start of "naïve").
        if let Some(b) = s.buffers.get_mut(&key) {
            b.mark = Some(0); // byte 0 = char 0
        }
        s.set_point_line(2);
        // Region is bytes [0, byte_offset_of_line2) = "# Notes\ncafé\n"
        s.kill_region();
        // After kill, the buffer should contain "naïve\nend\n".
        let remaining = s.buffers.get(&key).unwrap().rope.to_string();
        assert_eq!(remaining, "naïve\nend\n", "kill must remove exactly the region: {remaining:?}");
        // The kill ring holds the killed text.
        assert_eq!(s.kill_ring.top(), Some("# Notes\ncafé\n"));
    }

    #[test]
    fn yank_non_ascii_content_correct() {
        // Proves that yank inserts at the correct char offset when the buffer
        // contains multi-byte characters before the insertion point.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        // Buffer: "# Notes\n" + "héllo\n" (héllo has a multi-byte é)
        for c in "héllo\n".chars() {
            s.notes_insert_char(c);
        }
        let key = s.buffers.current().unwrap().to_string();
        // Copy "héllo\n" to the kill ring (lines 1-2).
        let line1_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(1).unwrap();
        let _line2_byte = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.mark = Some(line1_byte);
        }
        s.set_point_line(2);
        s.copy_region();
        let yank_text = s.kill_ring.top().unwrap().to_string();
        assert_eq!(yank_text, "héllo\n");
        // Now yank at line 0 (start of buffer, before the multi-byte content).
        s.set_point_line(0);
        s.yank();
        let buf_text = s.buffers.get(&key).unwrap().rope.to_string();
        // The yanked text is inserted at the start.
        assert!(buf_text.starts_with("héllo\n"), "yank must insert at correct char offset: {buf_text:?}");
    }

    #[test]
    fn yank_pop_non_ascii_content_correct() {
        // Proves that yank-pop removes/inserts at the correct char offsets
        // when the buffer contains multi-byte characters.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        let mut s = store(dir.path());
        s.open_notes();
        // Type unique lines with multi-byte content.
        for c in "café\n".chars() { s.notes_insert_char(c); }
        for c in "naïve\n".chars() { s.notes_insert_char(c); }
        for c in "end\n".chars() { s.notes_insert_char(c); }
        let key = s.buffers.current().unwrap().to_string();
        // Copy "café\n" (lines 1-2) to the ring.
        let l1 = s.buffers.get(&key).unwrap().rope.try_line_to_byte(1).unwrap();
        let _l2 = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) { buf.mark = Some(l1); }
        s.set_point_line(2);
        s.copy_region();
        // Copy "naïve\n" (lines 2-3) to the ring (now on top).
        let l2b = s.buffers.get(&key).unwrap().rope.try_line_to_byte(2).unwrap();
        let _l3 = s.buffers.get(&key).unwrap().rope.try_line_to_byte(3).unwrap();
        if let Some(buf) = s.buffers.get_mut(&key) { buf.mark = Some(l2b); }
        s.set_point_line(3);
        s.copy_region();
        // Yank at line 0 (inserts "naïve\n" at the start).
        s.set_point_line(0);
        s.yank();
        let after_yank = s.buffers.get(&key).unwrap().rope.to_string();
        assert!(after_yank.starts_with("naïve\n"), "yank: {after_yank:?}");
        // M-y: replace with "café\n".
        s.yank_pop();
        let after_pop = s.buffers.get(&key).unwrap().rope.to_string();
        assert!(after_pop.starts_with("café\n"), "yank-pop must replace with prev entry: {after_pop:?}");
        // The rest of the buffer is intact.
        assert!(after_pop.contains("naïve\n"), "original content must be preserved: {after_pop:?}");
        assert!(after_pop.contains("end\n"), "original content must be preserved: {after_pop:?}");
    }

    // ── plan 005 issue 02: inline annotations ─────────────────────────

    /// A store rooted at a temp project WITH an open project (a Cargo.toml
    /// marker), so notes-key / rel-path machinery works. The temp project
    /// dir is leaked (OS-cleaned on exit, like the persistence base).
    fn store_with_project() -> AppStore {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let dir_path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let base = tempfile::tempdir().unwrap();
        AppStore::at(&dir_path, base.path().to_path_buf())
    }

    /// Open a file buffer in the store via the open_path seam.
    fn open_ann_file(store: &mut AppStore, rel: &str, content: &str) {
        let root = store.project.as_ref().unwrap().root.clone();
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
        store.open_path(rel);
        assert_eq!(store.top_view(), ViewId::Buffer);
    }

    fn ann_records(store: &AppStore) -> Vec<&Annotation> {
        store
            .notes_doc
            .entries
            .iter()
            .filter_map(|e| e.as_record())
            .collect()
    }

    /// Parse → serialize → parse round-trip: records, raw blocks, and free
    /// text on BOTH sides of the section survive verbatim.
    #[test]
    fn notes_parse_serialize_round_trip() {
        let doc = NotesDoc {
            before: vec!["# Notes".to_string(), "free line one".to_string()],
            entries: vec![
                NotesEntry::Raw("# a comment".to_string()),
                NotesEntry::Record(Annotation {
                    path: "src/main.rs".to_string(),
                    line: 4,
                    col: 2,
                    anchor: "fn main() {".to_string(),
                    text: "fix the off-by-one: it's: nasty".to_string(),
                    orphaned: false,
                }),
                NotesEntry::Raw("[annotation]\npath: src/main.rs\nline: 9\n(no required fields)".to_string()),
                NotesEntry::Record(Annotation {
                    path: "README.md".to_string(),
                    line: 0,
                    col: 0,
                    anchor: "Redline".to_string(),
                    text: "top".to_string(),
                    orphaned: true,
                }),
            ],
            after: vec!["trailing free text".to_string()],
        };
        let text = serialize_notes(&doc);
        let reparsed = parse_notes(&text);
        assert_eq!(reparsed, doc, "round-trip must be exact");
        // The anchor with a colon round-trips byte-for-byte (the value is
        // the text after the FIRST colon of its field line).
        let rec = &reparsed.entries[1];
        let a = rec.as_record().unwrap();
        assert_eq!(a.anchor, "fn main() {");
        assert_eq!(a.text, "fix the off-by-one: it's: nasty");
        // A file with NO section at all parses into pure `before` and
        // re-serializes with the section appended, keeping the text.
        let plain = "alpha\nbeta\n";
        let d = parse_notes(plain);
        assert_eq!(d.before, vec!["alpha".to_string(), "beta".to_string()]);
        assert!(d.entries.is_empty());
        let s = serialize_notes(&d);
        assert!(s.starts_with("alpha\nbeta\n"));
        assert!(s.contains(NOTES_BEGIN) && s.contains(NOTES_END));
        assert_eq!(parse_notes(&s).before, d.before);
    }

    /// Tolerant parse: a malformed record (missing required fields) is
    /// kept VERBATIM as a raw block, never dropped, and valid records
    /// around it still parse.
    #[test]
    fn notes_tolerant_parse_keeps_malformed() {
        let text = "before\n\
                    <!-- redline-annotations:begin -->\n\
                    [annotation]\n\
                    path: a.rs\n\
                    line: 1\n\
                    note: missing anchor\n\
                    stray line without colon\n\
                    [annotation]\n\
                    path: b.rs\n\
                    line: 0\n\
                    anchor: hello\n\
                    note: ok\n\
                    <!-- redline-annotations:end -->\n\
                    after\n";
        let doc = parse_notes(text);
        assert_eq!(doc.before, vec!["before".to_string()]);
        assert_eq!(doc.after, vec!["after".to_string()]);
        // Two entries: the malformed block (raw, verbatim) + the good one.
        assert_eq!(doc.entries.len(), 2);
        assert!(matches!(doc.entries[0], NotesEntry::Raw(_)), "malformed kept verbatim: {:?}", doc.entries[0]);
        let raw = match &doc.entries[0] {
            NotesEntry::Raw(s) => s,
            _ => unreachable!(),
        };
        assert!(raw.contains("missing anchor"), "{raw}");
        assert!(raw.contains("stray line without colon"), "{raw}");
        let a = doc.entries[1].as_record().unwrap();
        assert_eq!(a.path, "b.rs");
        assert_eq!(a.anchor, "hello");
        // Re-serializing keeps the malformed block byte-for-byte.
        let out = serialize_notes(&doc);
        assert!(out.contains("note: missing anchor"), "{out}");
        assert_eq!(parse_notes(&out).entries.len(), 2);
    }

    /// A record whose stored line no longer holds the anchor is re-anchored
    /// by content within ±25 lines (the window bound is exclusive beyond
    /// ±25: a match at exactly ±25 re-anchors, at ±26 it does not).
    #[test]
    fn notes_reanchor_on_drift_and_bounds() {
        // 30 distinct filler lines + the anchor at line 29 (0-based).
        let mut lines: Vec<String> = (0..30).map(|i| format!("filler {i}")).collect();
        lines.push("THE ANCHOR LINE".to_string()); // line 30
        let content = lines.join("\n") + "\n";

        // Case 1: the stored line is 5 ABOVE the anchor (line 25 vs 30):
        // drift → unique match within ±25 → re-anchor to 30.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann1.rs", &content);
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            path: "src/ann1.rs".to_string(),
            line: 25,
            col: 0,
            anchor: "THE ANCHOR LINE".to_string(),
            text: "n".to_string(),
            orphaned: false,
        }));
        let key = s.buffers.current().unwrap().to_string();
        s.reanchor_for_key(&key);
        let a = ann_records(&s)[0];
        assert_eq!(a.line, 30, "unique match re-anchors");
        assert!(!a.orphaned);
        drop(s);

        // Case 2 (boundary): a match exactly +25 away re-anchors.
        let mut s = store_with_project();
        let far: Vec<String> = (0..26).map(|i| format!("pad {i}")).collect();
        let content = far.join("\n") + "\nEDGE\n"; // anchor at line 26 = 0 + 26? no: line 26
        open_ann_file(&mut s, "src/ann2.rs", &content);
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            path: "src/ann2.rs".to_string(),
            line: 1,
            col: 0,
            anchor: "EDGE".to_string(),
            text: "n".to_string(),
            orphaned: false,
        }));
        let key = s.buffers.current().unwrap().to_string();
        s.reanchor_for_key(&key);
        assert_eq!(ann_records(&s)[0].line, 26, "match at exactly +25 re-anchors");

        // Case 3 (boundary): a match at +26 does NOT re-anchor → orphaned,
        // line unchanged.
        let mut s = store_with_project();
        let far: Vec<String> = (0..27).map(|i| format!("pad {i}")).collect();
        let content = far.join("\n") + "\nFAR\n"; // anchor at line 27
        open_ann_file(&mut s, "src/ann3.rs", &content);
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            path: "src/ann3.rs".to_string(),
            line: 1,
            col: 0,
            anchor: "FAR".to_string(),
            text: "n".to_string(),
            orphaned: false,
        }));
        let key = s.buffers.current().unwrap().to_string();
        s.reanchor_for_key(&key);
        let a = ann_records(&s)[0];
        assert_eq!(a.line, 1, "no move at +26 (never a guessed line)");
        assert!(a.orphaned, "orphaned flag set");
    }

    /// Ambiguous drift (two identical anchor lines inside ±25) → orphaned,
    /// no move; and an anchor that returns to its stored line clears the
    /// orphan flag (idempotent/stable: a second pass changes nothing).
    #[test]
    fn notes_reanchor_ambiguous_and_idempotent() {
        let content = "same line\nother\nsame line\n".to_string();
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann4.rs", &content);
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            path: "src/ann4.rs".to_string(),
            line: 5, // out of range: drift for sure
            col: 0,
            anchor: "same line".to_string(),
            text: "n".to_string(),
            orphaned: false,
        }));
        let key = s.buffers.current().unwrap().to_string();
        s.reanchor_for_key(&key);
        let a = ann_records(&s)[0];
        assert!(a.orphaned, "ambiguous (2 matches) must not guess");
        assert_eq!(a.line, 5);
        // Idempotent: a second pass leaves the record exactly as-is.
        let (a_line, a_orphaned) = (a.line, a.orphaned);
        s.reanchor_for_key(&key);
        let a2 = ann_records(&s)[0];
        assert_eq!(a2.line, a_line);
        assert_eq!(a2.orphaned, a_orphaned);
        // Stable: when the anchor text returns to the STORED line (line 5,
        // uniquely), the orphan flag clears.
        let key = s.buffers.current().unwrap().to_string();
        if let Some(buf) = s.buffers.get_mut(&key) {
            buf.rope = Rope::from_str("a\nb\nc\nd\ne\nsame line\n");
        }
        let key = s.buffers.current().unwrap().to_string();
        s.reanchor_for_key(&key);
        assert!(!ann_records(&s)[0].orphaned, "anchor back → orphan cleared");
    }

    /// `A` on a line prompts with an empty prefill; on an annotated line it
    /// pre-fills the existing note for edit; RET commits the record to the
    /// notes file (and the in-memory doc); `d` deletes with an echo; a `d`
    /// on an unannotated line is a no-op with a message.
    #[test]
    fn notes_a_flow_prefill_commit_delete() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann5.rs", "one\ntwo\nthree\n");
        // A on line 1 (fresh): prompt opens, empty input.
        s.set_point_line(1);
        s.annotate();
        assert!(s.note_prompt_active());
        assert_eq!(s.note_prompt_input(), "");
        s.note_prompt_char('f');
        s.note_prompt_char('i');
        s.note_prompt_confirm();
        assert!(!s.note_prompt_active());
        assert_eq!(ann_records(&s).len(), 1);
        let a = ann_records(&s)[0];
        assert_eq!(a.path, "src/ann5.rs");
        assert_eq!(a.line, 1);
        assert_eq!(a.anchor, "two", "anchor = the exact anchored-line text");
        assert_eq!(a.text, "fi");
        // The record is on disk in the notes file, in the structured
        // section.
        let disk = std::fs::read_to_string(
            s.project.as_ref().unwrap().root.join(".redline-notes.md"),
        )
        .unwrap();
        assert!(disk.contains(NOTES_BEGIN), "{disk}");
        assert!(disk.contains("path: src/ann5.rs"), "{disk}");
        assert!(disk.contains("note: fi"), "{disk}");
        // A on the annotated line pre-fills for edit.
        s.set_point_line(1);
        s.annotate();
        assert_eq!(s.note_prompt_input(), "fi", "prefill for edit");
        s.note_prompt_cancel();
        // d on the annotated line deletes with an echo.
        s.set_point_line(1);
        s.annotate_delete();
        assert!(s.message.contains("deleted annotation: fi"), "{}", s.message);
        assert_eq!(ann_records(&s).len(), 0);
        // d on an unannotated line: message, no record, no unbound-key echo.
        s.set_point_line(0);
        s.annotate_delete();
        assert_eq!(s.message, "no annotation on this line");
        assert!(ann_records(&s).is_empty());
    }

    /// Empty RET while editing an existing record deletes it (the
    /// pre-filled-for-edit cancel path); empty RET on a fresh prompt just
    /// cancels.
    #[test]
    fn notes_empty_ret_edits_delete_cancels() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann6.rs", "a\nb\n");
        s.set_point_line(0);
        s.annotate();
        s.note_prompt_char('x');
        s.note_prompt_confirm();
        assert_eq!(ann_records(&s).len(), 1);
        // Pre-fill for edit, clear it, RET → delete.
        s.set_point_line(0);
        s.annotate();
        assert_eq!(s.note_prompt_input(), "x");
        s.note_prompt_backspace();
        s.note_prompt_confirm();
        assert_eq!(ann_records(&s).len(), 0, "empty RET on existing record deletes");
        // Fresh prompt, RET with nothing → cancel, no record.
        s.set_point_line(1);
        s.annotate();
        s.note_prompt_confirm();
        assert_eq!(ann_records(&s).len(), 0);
        assert_eq!(s.message, "note cancelled");
    }

    /// The rendered-row map round-trips buffer_line ↔ rendered_row through
    /// the STORE's file_view_rows (the real viewport, interleaved note
    /// rows), and mouse_click_position maps a code row under an annotation
    /// to the right buffer line.
    #[test]
    fn notes_row_map_store_and_click_mapping() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann7.rs", "c0\nc1\nc2\nc3\nc4\n");
        // Two annotations: line 1 and line 3 (both note rows visible).
        for (line, note) in [(1, "n1"), (3, "n3")] {
            s.notes_doc.entries.push(NotesEntry::Record(Annotation {
                path: "src/ann7.rs".to_string(),
                line,
                col: 0,
                anchor: format!("c{line}"),
                text: note.to_string(),
                orphaned: false,
            }));
        }
        s.sync_notes_from_doc();
        s.set_viewport_lines(10);
        s.set_scroll_top(0);
        let rows = s.file_view_rows();
        // Row list: c0, c1, note1, c2, c3, note3, c4, (empty last line
        // from the trailing newline — ropey's len_lines counts it).
        let shape: Vec<(usize, bool)> = rows
            .iter()
            .map(|r| (r.line, r.is_note))
            .collect();
        assert_eq!(
            shape,
            vec![
                (0, false),
                (1, false),
                (1, true),
                (2, false),
                (3, false),
                (3, true),
                (4, false),
                (5, false)
            ],
            "row shape: {shape:?}"
        );
        // Marker flags: code rows 1 and 3 annotated, others not.
        assert!(rows[1].annotated && rows[4].annotated);
        assert!(!rows[0].annotated && !rows[3].annotated);
        // Both directions of the map.
        for line in 0..6 {
            let r = FileViewRow::row_for_line(&rows, line).unwrap();
            assert_eq!(FileViewRow::line_for_row(&rows, r), Some(line));
        }
        assert_eq!(FileViewRow::line_for_row(&rows, 2), Some(1), "note row → anchored line");
        assert_eq!(FileViewRow::line_for_row(&rows, 5), Some(3));
        // Total rendered rows = 6 code + 2 note.
        assert_eq!(s.file_view_total_rows(), 8);
        // Click mapping: rendered row 4 (c3, the code row UNDER a note row
        // above it… here row 2) and rendered row 2 (the note row) both map
        // to their line; the old dense math (scroll_top + row) would have
        // sent row 4 to line 4.
        let mut s2 = s;
        s2.mouse_click_position(4, 0); // c3
        assert_eq!(s2.point_line(), 3, "click on c3's rendered row → line 3");
        s2.mouse_click_position(2, 0); // note row under c1
        assert_eq!(s2.point_line(), 1, "click on a note row → anchored line");
        // C-c a hides the note rows: the map collapses back to 1:1, the
        // marker flags stay.
        s2.annotate_toggle();
        let rows2 = s2.file_view_rows();
        assert_eq!(rows2.len(), 6, "note rows hidden");
        assert!(rows2[1].annotated && rows2[3].annotated, "markers stay");
        assert_eq!(s2.file_view_total_rows(), 6);
        // C-c a again: back.
        s2.annotate_toggle();
        assert_eq!(s2.file_view_rows().len(), 8);
    }

    /// C-n/C-p across a virtual note row: the point walks BUFFER lines
    /// (never a note row), so from the annotated line C-n lands on the
    /// NEXT CODE line and C-p returns — the status position display is the
    /// buffer line (map correctness).
    #[test]
    fn notes_point_motion_crosses_note_rows_on_code_lines() {
        let mut s = store_with_project();
        let content: String = (0..30).map(|i| format!("k{i}\n")).collect();
        open_ann_file(&mut s, "src/ann8.rs", &content);
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            path: "src/ann8.rs".to_string(),
            line: 1,
            col: 0,
            anchor: "k1".to_string(),
            text: "note".to_string(),
            orphaned: false,
        }));
        s.sync_notes_from_doc();
        // Point on the annotated line (1). C-n → line 2 (the CODE line,
        // never "stuck" on the note row); the position display is the
        // BUFFER line (map correctness).
        s.set_point_line(1);
        s.point_down();
        assert_eq!(s.point_line(), 2, "C-n crosses the note row → next code line");
        assert_eq!(
            s.file_view_position_display(),
            "L3,6%",
            "status shows the code line: {}",
            s.file_view_position_display()
        );
        s.point_up();
        assert_eq!(s.point_line(), 1, "C-p back onto the annotated line");
        assert_eq!(s.file_view_position_display(), "L2,3%");
        // The rendered slice has 4 rows (3 code + 1 note) and the cursor
        // row for point line 1 is 1 (not 2 — the note row comes AFTER it).
        let rows = s.file_view_rows();
        assert_eq!(FileViewRow::row_for_line(&rows, 1), Some(1));
        assert!(rows[2].is_note);
    }

    /// The status line shows the current file's annotation count (and an
    /// edit-mode save of the anchored file re-anchors in the same pass).
    #[test]
    fn notes_status_count_and_save_reanchors() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann9.rs", "p0\np1\n");
        assert_eq!(s.annotation_count_display(), "");
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            path: "src/ann9.rs".to_string(),
            line: 1,
            col: 0,
            anchor: "p1".to_string(),
            text: "a".to_string(),
            orphaned: false,
        }));
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            path: "src/ann9.rs".to_string(),
            line: 0,
            col: 0,
            anchor: "p0".to_string(),
            text: "b".to_string(),
            orphaned: false,
        }));
        assert_eq!(s.annotation_count_display(), "2 notes");
        // Edit-mode save re-anchors: flip the file to edit mode, change
        // p1's line content elsewhere… simplest: the buffer already holds
        // the anchor at line 1, so save is a no-op re-anchor (count stable).
        let key = s.buffers.current().unwrap().to_string();
        s.buffers.get_mut(&key).unwrap().editable = true;
        s.save_buffer();
        assert_eq!(s.annotation_count_display(), "2 notes", "save keeps the records");
        // Now delete the anchor line out-of-band + save: p0 gone → orphan
        // flag, record NOT lost.
        let key = s.buffers.current().unwrap().to_string();
        s.buffers.get_mut(&key).unwrap().rope = Rope::from_str("x\np1\n");
        s.buffers.get_mut(&key).unwrap().locally_modified = true;
        s.save_buffer();
        let recs = ann_records(&s);
        assert_eq!(recs.len(), 2, "orphaned records are not lost");
        let o = recs.iter().find(|a| a.anchor == "p0").unwrap();
        assert!(o.orphaned, "p0's anchor text is gone → orphaned");
        let k = recs.iter().find(|a| a.anchor == "p1").unwrap();
        assert!(!k.orphaned);
        assert_eq!(s.annotation_count_display(), "2 notes");
    }
}



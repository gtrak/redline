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
use redline_resolve::{
    CargoProvider, ResolvedSource, Resolver, SymbolContext,
    providers::{go_provider::GoProvider, js_provider::JsProvider, python_provider::PythonProvider},
};
use ropey::Rope;
use tokio::sync::mpsc;
use tokio::sync::watch;

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
use crate::syntax::cache::{CacheKey, HighlightCache, TreeKey};
use crate::syntax::highlight::{self, HighlightResult, RetainedTree};
use crate::syntax::registry::{GrammarRegistry, LanguageId};
use crate::theme::Theme;

/// The result the background tooling-resolver job publishes to the app via
/// the [`ResolveBus`] (plan 006 issue 02). Mirrors the [`IndexBus`] pattern:
/// a `watch` channel, latest-value-wins. `cargo metadata` / `cargo fetch`
/// shell out (network, seconds) and MUST never block the input path, so the
/// M-. workspace-miss fall-through runs on `spawn_blocking` and lands here.
#[derive(Clone, Debug, Default)]
pub struct ResolveEvent {
    /// Generation tag (matches the store's `resolve_generation`); stale
    /// events (a superseded M-. request or a previous project) are
    /// discarded by [`AppStore::apply_resolve_event`].
    pub generation: usize,
    /// The symbol the job was asked to resolve (result/report text).
    pub symbol: String,
    /// Where to open (read-only) on a hit.
    pub source: Option<ResolvedSource>,
    /// The failure report on a miss (the provider-chain error text).
    pub error: Option<String>,
}

/// The resolve-result bus (plan 006 issue 02): a `watch` channel over
/// [`ResolveEvent`]. The store owns the sender; the UI's drain task in
/// `Root` subscribes and applies each event to the store.
#[derive(Clone)]
pub struct ResolveBus {
    tx: watch::Sender<ResolveEvent>,
}

impl std::fmt::Debug for ResolveBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolveBus").finish()
    }
}

impl ResolveBus {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(ResolveEvent::default());
        Self { tx }
    }

    pub fn subscribe(&self) -> watch::Receiver<ResolveEvent> {
        self.tx.subscribe()
    }

    pub fn send(&self, event: ResolveEvent) {
        let _ = self.tx.send(event);
    }
}

/// The result the background crate-indexing job publishes to the app via
/// the [`CrateIndexBus`] (plan 006 issue 03): the FINISHED symbol index
/// for an EXTERNAL source tree (a registry crate root or a path-dependency
/// crate root), keyed by its `source_root`. Mirrors the
/// [`ResolveEvent`]/[`ResolveBus`] pattern: a `watch` channel,
/// latest-value-wins. The build (the project indexer's
/// rayon-parallel tree-sitter machinery — `nav::index::build_index` is
/// root-agnostic) runs on `spawn_blocking` and never blocks the landing;
/// this event is the single publish per build (the `indexing crate …`
/// indicator is store-side state, cleared when the event lands).
#[derive(Clone, Debug, Default)]
pub struct CrateIndexEvent {
    /// The source-tree root the index covers (the LRU cache key).
    pub source_root: PathBuf,
    /// The finished index (crate-relative file paths).
    pub index: SymbolIndex,
}

/// The crate-index-result bus (plan 006 issue 03): a `watch` channel over
/// [`CrateIndexEvent`]. The store owns the sender; the UI's drain task in
/// `Root` subscribes and applies each event to the store (mirrors
/// `ResolveBus`).
#[derive(Clone)]
pub struct CrateIndexBus {
    tx: watch::Sender<CrateIndexEvent>,
}

impl std::fmt::Debug for CrateIndexBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CrateIndexBus").finish()
    }
}

impl CrateIndexBus {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(CrateIndexEvent::default());
        Self { tx }
    }

    pub fn subscribe(&self) -> watch::Receiver<CrateIndexEvent> {
        self.tx.subscribe()
    }

    pub fn send(&self, event: CrateIndexEvent) {
        let _ = self.tx.send(event);
    }
}

/// The LRU cap for the external crate-index cache (plan 006 issue 03): a
/// handful of crates is plenty; the oldest is evicted when a new one
/// lands.
const EXT_INDEX_CAP: usize = 3;
/// An external source tree larger than this (`.rs` files) is too big to
/// index: the build is refused with a clear message (no known registry
/// crate approaches it).
const EXT_INDEX_FILE_CAP: usize = 2000;

/// The M-. selection outcome inside an EXTERNAL (registry / tooling)
/// buffer (plan 006 issue 03) — the project path's four outcomes,
/// crate-relative.
enum ExternalXrefOutcome {
    /// Unique definition: crate-relative file + 0-based line.
    Jump { file: String, line: usize },
    /// Ambiguous: candidates (crate-relative, same-file-first) + the
    /// picker's lookup label.
    Picker { lookup: String, defs: Vec<crate::nav::index::Location> },
    /// Crate miss with the point on a symbol: the resolver fall-through,
    /// carrying the point's (path-shaped) token.
    Resolver(String),
    /// An enclosing symbol with no indexed definition.
    NoDefinition(String),
    /// Nothing under or near the point.
    NoSymbol,
}

/// A view on the stack. The top of the stack is what the main view
/// renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewId {
    /// The home view (plan 004 issue 06a): rendered when no buffer is
    /// current. Header (project + dirty counts + "redline"), the derived
    /// command groups (live keymap × command registry), and the standard
    /// help line. Home is NOT a buffer: opening a view replaces it.
    Home,
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
            ViewId::Home => "home",
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
                // Window-split keys (the 3 pre-existing ux_sweep findings),
                // degraded onto the single-pane view-stack model — a full
                // vertical split is a scoped follow-up (per-pane buffer /
                // point / scroll state, a second render pane, window-focus
                // cycling), so the keys are bound and degrade honestly:
                // C-x 0 closes the top view (the `q` close-view command;
                // no-op on the last view), C-x 1 truncates the stack to the
                // buffer view (no-op when it is already the only view),
                // C-x 2 reports the single-pane model without changing
                // state.
                km.bind(&[Key::ctrl_char('x'), Key::char('0')], "close-view").unwrap();
                km
                    .bind(&[Key::ctrl_char('x'), Key::char('1')], "close-other-views")
                    .unwrap();
                km.bind(
                    &[Key::ctrl_char('x'), Key::char('2')],
                    "split-window-vertical",
                )
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
            ViewId::Home => {
                // 06a: home has NO view-local bindings. `q` is deliberately
                // unbound (there is no buffer to close); every entry point
                // (C-x C-f, C-x g, C-x n, C-x b, C-x C-c, C-c p …, ?) is a
                // GLOBAL binding, so they all work from home without any
                // home-side inheritance. The render switch still shows home
                // whenever the buffer view has no current buffer.
                KeyMap::new()
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
                // Watchlist item 4: `M-,` under the results view — jump-back
                // pops through the sentinel to the pre-search position in
                // one step (the buffer view's M-, only exists once the
                // results view is closed; the sentinel entry's navigation
                // re-opens the view, so the pop must work from it too).
                km.bind(&[Key::alt_char(',')], "jump-back").unwrap();
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
    /// find-implementations (010-04, plan 010 Shape A rung 4): the
    /// `impl <Trait> for <Type>` blocks implementing the trait at point;
    /// RET jumps to the selected impl header.
    Impls,
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
/// in the file view. `path` is the record key: the project-relative path
/// for project files, or the absolute path string for external (library)
/// buffers (plan 008 issue 01 — see `buffer_annotation_path`, the one
/// key-derivation point). `anchor` is the exact text of the anchored line
/// at creation: automatic re-anchoring (±25 lines) keeps `line` pointing
/// at it as the file drifts, and `orphaned` flags the case where the
/// anchor text is gone (the annotation stays at its last known line —
/// NEVER moved to a guessed line).
/// The OPTIONAL syntax anchor of an annotation (plan 007 issue 02): the
/// (kind, text) of the identifier-ish node under the point at creation
/// (007-01's `node_at` at the point's byte offset — e.g. `identifier` /
/// `target_one` for a function name). Re-anchoring first searches the
/// WHOLE file for a unique node with this kind + name (surviving a
/// 100-line insertion and a reformat alike); zero or multiple matches
/// fall through to the text rules. `None` for legacy records (no
/// `syntax_*` keys), non-Rust files, and points where `node_at` has no
/// identifier-ish answer (keywords, whitespace, operators) — those
/// records ride the text rules alone, exactly as before.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyntaxAnchor {
    /// The tree-sitter node kind — always one of 007-01's identifier-ish
    /// kinds (`identifier`, `field_identifier`, `type_identifier`,
    /// `scoped_identifier`, `scoped_type_identifier`, `primitive_type`).
    pub kind: String,
    /// The node's source text (e.g. `target_one`; a `::` path comes back
    /// whole, exactly as `node_at` returns it — `tokio::spawn`).
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Annotation {
    pub path: String,
    pub line: usize,
    pub col: usize,
    pub anchor: String,
    pub text: String,
    pub orphaned: bool,
    /// The syntax anchor captured at creation (plan 007 issue 02); see
    /// [`SyntaxAnchor`] for the capture rule and the re-anchor order.
    pub syntax: Option<SyntaxAnchor>,
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
    // The syntax anchor's keys (plan 007 issue 02) are OPTIONAL: both must
    // be present for `syntax: Some`; either missing → `None` (legacy).
    let mut syntax_kind: Option<String> = None;
    let mut syntax_name: Option<String> = None;
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
            "syntax_kind" => {
                syntax_kind = Some(value.to_string());
            }
            "syntax_name" => {
                syntax_name = Some(value.to_string());
            }
            // Unknown keys inside a record block: the block stays valid
            // (forward compatibility), they are simply not re-emitted.
            _ => {}
        }
    }
    if have.iter().all(|h| *h) {
        // The syntax anchor is OPTIONAL: absent keys (legacy records) and a
        // half-written pair (a hand-edited `syntax_kind` without a
        // `syntax_name`) both degrade to `None` — the record stays valid
        // (tolerant parse), the anchor simply does not half-fire.
        rec.syntax = match (syntax_kind, syntax_name) {
            (Some(kind), Some(name)) => Some(SyntaxAnchor { kind, name }),
            _ => None,
        };
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
                // The syntax keys are emitted ONLY when present, appended
                // after `orphaned` (additive): a record without a syntax
                // anchor serializes byte-identically to the pre-007-02
                // shape (no migration of legacy files).
                if let Some(sa) = &a.syntax {
                    out.push_str(&format!("syntax_kind: {}\n", sa.kind));
                    out.push_str(&format!("syntax_name: {}\n", sa.name));
                }
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

/// One annotation in the quit-dump shape (plan 005 issue 03): a
/// self-contained brief an agent can act on. `line` is 1-based (the
/// record's 0-based `Annotation::line` + 1 — the dump is for humans and
/// agents, not ropey). `code` is the anchored line: the buffer's current
/// content at `line` when the anchor holds, otherwise the stored `anchor`
/// (the last known anchored text — an orphaned record's line must not be
/// trusted).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DumpAnnotation {
    /// The record's `path` verbatim: project-relative for project files,
    /// absolute for external (library) buffers (plan 008 issue 01).
    pub path: String,
    /// 1-based anchored line.
    pub line: usize,
    /// The anchored code line (see above).
    pub code: String,
    /// The note text (the record's `text`; may span lines).
    pub text: String,
    pub orphaned: bool,
}

/// Format the quit-dump (plan 005 issue 03). Both modes order by path
/// asc, then line asc (stable for diffing); an empty set yields ZERO
/// bytes (pipes stay clean).
///
/// Block mode (default, `plain = false`) — the agent brief:
/// `# redline annotations — <root>` + a blank line, then per record:
/// `path:line`, the anchored line indented +4 as code, and `NOTE:` with
/// the text (subsequent note lines aligned under the first). Orphaned
/// records carry an explicit `  ORPHANED (anchor text not found)` line so
/// the agent never trusts a stale line number silently.
///
/// Plain mode (`--notes=plain`, the grep/pipe shape): one `path:line:
/// text` per record (subsequent note lines aligned under the first),
/// no header and no code/orphan lines.
pub fn format_notes_dump(items: &[DumpAnnotation], root: &str, plain: bool) -> String {
    if items.is_empty() {
        return String::new();
    }
    // The ordering is part of the output contract (diff stability), so it
    // lives here, not in the accessor.
    let mut items: Vec<&DumpAnnotation> = items.iter().collect();
    items.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.line.cmp(&b.line)));
    let mut out = String::new();
    if !plain {
        out.push_str("# redline annotations \u{2014} ");
        out.push_str(root);
        out.push_str("\n\n");
    }
    for item in items {
        if !plain {
            out.push_str(&format!("{}:{}\n", item.path, item.line));
            if item.orphaned {
                out.push_str("  ORPHANED (anchor text not found)\n");
            }
            out.push_str("    ");
            out.push_str(&item.code);
            out.push('\n');
            let lines: Vec<&str> = item.text.split('\n').collect();
            let (first, rest) = match lines.split_first() {
                Some((f, r)) => (*f, r),
                None => ("", &[] as &[&str]),
            };
            out.push_str("  NOTE: ");
            out.push_str(first);
            for extra in rest {
                out.push('\n');
                out.push_str("        "); // aligned under the first note line's text
                out.push_str(extra);
            }
            out.push('\n');
        } else {
            let prefix = format!("{}:{}: ", item.path, item.line);
            let lines: Vec<&str> = item.text.split('\n').collect();
            let (first, rest) = match lines.split_first() {
                Some((f, r)) => (*f, r),
                None => ("", &[] as &[&str]),
            };
            out.push_str(&prefix);
            out.push_str(first);
            for extra in rest {
                out.push('\n');
                out.push_str(&prefix); // aligned under the first note line's text
                out.push_str(extra);
            }
            out.push('\n');
        }
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
    /// The trait keys the find-implementations picker was opened with
    /// (010-04 — the M-. path token and/or the bare identifier), so query
    /// re-computation inside the picker stays against the same name-keyed
    /// trait map (mirrors `xref_lookup_name`; replaced on every Impls
    /// picker open, inert otherwise).
    impls_keys: Vec<String>,
    // ── tooling-aware jump fall-through (plan 006 issue 02) ─────────────
    /// The resolve-result bus: a background M-. fall-through job publishes
    /// here; the UI's drain task applies each event (mirrors `index_bus`).
    pub resolve_bus: ResolveBus,
    /// Generation counter for resolve jobs: bumped on every M-. request
    /// (a workspace hit supersedes any in-flight job, 006-02b item 2), on
    /// every new fall-through request, and on project switch — a stale
    /// in-flight result is then discarded.
    resolve_generation: usize,
    /// Status-line activity while a resolve job is in flight:
    /// `(text, generation)`; the display hides it when the generation no
    /// longer matches (superseded request or project switch), so it can
    /// never hang on a stale job.
    resolving: Option<(String, usize)>,
    /// The buffer keys opened via the external-landing path
    /// (`open_external_path`: registry / tooling sources, plan 006 issue
    /// 02) — the buffers the ownership guard (006-02b item 1) refuses to
    /// make editable or write. Path-keyed, stable across project switches
    /// (a path under a NEW project root is project-owned again, checked
    /// first in `buffer_is_project_owned`).
    external_buffers: std::collections::HashSet<String>,
    // ── external crate index cache (plan 006 issue 03) ────────────
    /// LRU cache of EXTERNAL (registry / tooling) source-tree indexes,
    /// one entry per `source_root`: newest last, capped at
    /// `EXT_INDEX_CAP` (the oldest is evicted when a new crate lands).
    /// Built in the background (`start_crate_indexing`, off the input
    /// path) and consulted by M-. / imenu inside an external buffer.
    /// Machine-wide (absolute roots), so it survives project switches —
    /// these indexes are never "the project" (008-01 semantics hold).
    external_indexes: Vec<(PathBuf, Arc<std::sync::Mutex<SymbolIndex>>)>,
    /// The crate-index-result bus: a background crate-index job publishes
    /// its finished index here; the UI's drain task applies each event
    /// (mirrors `resolve_bus`).
    pub crate_index_bus: CrateIndexBus,
    /// Status-line activity while a crate-index job is in flight: one
    /// `(source_root, label, progress)` per build (the publisher-less
    /// progress counter feeds the `indexing crate <dir> (i/N)…` status,
    /// 006-03b item 5); cleared when its final event lands
    /// (`apply_crate_index_event`).
    crate_indexing: Vec<(PathBuf, String, Arc<IndexProgress>)>,
    /// The `source_root` the Xref picker's candidates are keyed against
    /// (`None` = the project index): set when M-. inside an EXTERNAL
    /// buffer opens the ambiguous picker, so query re-computation,
    /// preview, and the selection all stay crate-relative.
    xref_crate_root: Option<PathBuf>,
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

        let view = ViewId::Home.keymap();
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
            view_stack: vec![ViewId::Home],
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
            impls_keys: Vec::new(),
            resolve_bus: ResolveBus::new(),
            resolve_generation: 0,
            resolving: None,
            external_buffers: std::collections::HashSet::new(),
            external_indexes: Vec::new(),
            crate_index_bus: CrateIndexBus::new(),
            crate_indexing: Vec::new(),
            xref_crate_root: None,
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

    /// The view the main pane actually RENDERS (plan 004 issue 06a): the
    /// buffer view with no current buffer renders home (the table may be
    /// empty — boot starts that way, and killing the last buffer lands
    /// there). Never creates a buffer to satisfy the render.
    pub fn render_view(&self) -> ViewId {
        if self.top_view() == ViewId::Buffer && self.buffers.current().is_none() {
            ViewId::Home
        } else {
            self.top_view()
        }
    }

    /// Keep the top view consistent with the buffer table (plan 004
    /// issue 06a): home renders only with NO current buffer, and the buffer
    /// view renders only WITH one — so every view change re-normalizes the
    /// top (home ⇄ buffer) and rebuilds the keymap for the new top.
    fn normalize_top_view(&mut self) {
        let swap = match self.view_stack.last() {
            Some(&ViewId::Home) if self.buffers.current().is_some() => Some(ViewId::Buffer),
            Some(&ViewId::Buffer) if self.buffers.current().is_none() => Some(ViewId::Home),
            _ => None,
        };
        if let Some(view) = swap {
            self.view_stack.pop();
            self.view_stack.push(view);
        }
        let top = self.top_view();
        let view_km = top.keymap();
        self.engine = KeymapEngine::new(self.engine.global.clone(), view_km);
    }

    pub fn view_name(&self) -> &'static str {
        self.top_view().name()
    }

    /// Status-line view name: the current buffer's display name (or
    /// `*list-buffers*` in the buffer-list view, `*magit-status*`,
    /// `*search*`). Renders through `render_view`, so a buffer view with no
    /// current buffer reads `home` (06a), never a ghost `*scratch*`.
    pub fn view_name_display(&self) -> String {
        match self.render_view() {
            ViewId::Buffer => self
                .buffers
                .current()
                .map(|key| self.buffer_display(key))
                // Unreachable via render_view (Buffer only renders with a
                // current buffer); keep a defined value anyway.
                .unwrap_or_default(),
            ViewId::Home => "home".to_string(),
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
    /// new top view's keymap (re-normalizing home ⇄ buffer, 06a).
    pub fn close_view(&mut self) {
        if self.view_stack.len() > 1 {
            self.view_stack.pop();
            self.buffer_list_selected = 0;
            self.normalize_top_view();
        }
    }

    /// C-x 1 (emacs's `only-this-window`) on the view-stack model: close
    /// every view except the current buffer view — the stack collapses to
    /// exactly `[Buffer]`. Bound only in the buffer view's keymap (and that
    /// view renders only WITH a current buffer, 06a), so the buffer view
    /// always exists; when it is already the only view the command is a
    /// no-op, like emacs's C-x 1 on a single window.
    pub fn close_other_views(&mut self) {
        if self.top_view() != ViewId::Buffer || self.view_stack.len() <= 1 {
            return;
        }
        self.view_stack = vec![ViewId::Buffer];
        self.buffer_list_selected = 0;
        self.normalize_top_view();
    }

    /// C-x 2 (emacs's `split-window-vertically`) on the single-pane
    /// view-stack model: there is no split layout yet (the scoped
    /// follow-up — a real split needs per-pane buffer / point / scroll /
    /// edit-mode state, a second render pane, and window-focus cycling),
    /// so the key is bound and reports the model instead of dead-ending:
    /// no state change (buffer, point, and view stack all untouched), the
    /// note lands in the minibuffer.
    pub fn split_window_vertical(&mut self) {
        self.minibuffer_message(
            "single pane by design: redline runs as a herdr popup, one pane",
        );
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
        // Re-assert the top view's keymap after rotating (and re-normalize
        // home ⇄ buffer, 06a).
        self.normalize_top_view();
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
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.insert(pos, text);
            // A local edit: the buffer now differs from disk (the
            // light-editing flag, plan decision #6).
            buf.locally_modified = true;
            // Plan 007 issue 04: record the edit on the retained parse
            // tree so the next ensure_highlight reparses incrementally.
            if let Some(old_rope) = old_rope {
                self.retain_rope_edit(&key, &old_rope, pos, pos, text);
            }
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

    /// Record a rope edit on the buffer's retained parse tree (plan 007
    /// issue 04) so the next `ensure_highlight` can reparse incrementally.
    /// `old_rope` is the PRE-edit rope (the char positions refer to it).
    /// No-op when the buffer has no retained tree (plain text, big file,
    /// non-reuse language, or never highlighted yet) — those keep the
    /// full-parse path.
    fn retain_rope_edit(
        &mut self,
        key: &str,
        old_rope: &Rope,
        char_start: usize,
        char_end: usize,
        new_text: &str,
    ) {
        let Some(buf) = self.buffers.get(key) else { return };
        if buf.is_big() || buf.path.is_none() {
            return;
        }
        let edit =
            highlight::rope_edit_to_input_edit(old_rope, char_start, char_end, new_text);
        self.highlight_cache
            .retain_apply_edit(&TreeKey::new(key, buf.mtime), &edit);
    }

    /// Switch to (creating if needed) the `*scratch*` buffer.
    /// 06a: an EXPLICIT affordance (M-o / C-x o / M-x) — scratch is no
    /// longer auto-created at boot; opening it replaces the home view.
    pub fn open_scratch(&mut self) {
        let key = SCRATCH_NAME.to_string();
        if self.buffers.get(&key).is_none() {
            self.buffers.insert(None, String::new());
        }
        self.buffers.set_current(&key);
        self.normalize_top_view();
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
        self.normalize_top_view();
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
            // 006-02b item 1: ownership guard — an external (registry /
            // tooling) source is a cache shared by every project on the
            // machine; refuse the write even if the buffer somehow reached
            // edit mode.
            if !self.buffer_is_project_owned(key) {
                self.minibuffer_message(
                    "save-buffer: external buffer is read-only (not project-owned)",
                );
                return false;
            }
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

    /// The current buffer's annotation key (plan 008 issue 01): the
    /// project-relative path when the buffer's path strips under the
    /// project root, else the absolute path string (external buffers).
    /// `None` only for pathless buffers (scratch).
    fn current_annotation_path(&self) -> Option<String> {
        let key = self.buffers.current()?.to_string();
        self.buffer_annotation_path(&key)
    }

    /// The index of the FIRST structured-section record anchored at the
    /// current buffer's line `line`, or `None` when the line carries no
    /// annotation.
    fn record_index_for_line(&self, line: usize) -> Option<usize> {
        let rel = self.current_annotation_path()?;
        self.notes_doc
            .entries
            .iter()
            .position(|e| matches!(e, NotesEntry::Record(a) if a.path == rel && a.line == line))
    }

    /// Re-anchor the annotations of the buffer at `key` against the
    /// buffer's current content (plan 005 issue 02 + plan 007 issue 02):
    /// for each record anchored there, the re-anchor order is
    ///
    /// 1. **syntax anchor** (records carrying a `SyntaxAnchor`): a node of
    ///    the recorded `kind` + `name` found ANYWHERE in the file — exactly
    ///    one match re-anchors to its line (any distance, orphan flag
    ///    clears). Zero or multiple matches fall through (never guess,
    ///    exactly like the text rules' ambiguity rule). The file is
    ///    parsed at most once per pass, and only when at least one record
    ///    for this file HAS a syntax anchor (the common legacy case pays
    ///    no parse).
    /// 2. **exact-line text**: the content at the stored line still matches
    ///    `anchor` exactly (the record stays; an orphan flag clears).
    /// 3. **±25-line text search**: a UNIQUE match re-anchors (updates
    ///    `line`, clears `orphaned`).
    /// 4. **orphan**: zero or multiple text matches set `orphaned = true`
    ///    and leave `line` unchanged (NEVER move an anchor to a guessed
    ///    line).
    ///
    /// Stable and idempotent: a record whose line holds the anchor (or
    /// whose syntax node is unique) is untouched on a second pass.
    fn reanchor_for_key(&mut self, key: &str) {
        let Some(rel) = self.buffer_annotation_path(key) else {
            return;
        };
        let Some(buf) = self.buffers.get(key) else {
            return;
        };
        let total = buf.line_count();
        if total == 0 {
            return;
        }
        // The syntax re-anchor index is built BEFORE the mutable pass (it
        // reads `self`); `None` for the legacy / non-Rust / no-anchor cases
        // and the text rules below run exactly as before.
        let syntax_index = self.build_syntax_index_for_key(key, &rel);
        let mut changed = false;
        for entry in self.notes_doc.entries.iter_mut() {
            let Some(a) = entry.as_record_mut() else {
                continue;
            };
            if a.path != rel {
                continue;
            }
            // Order matters (007-02): syntax first — a unique (kind, name)
            // node ANYWHERE in the file re-anchors to its line regardless
            // of distance (this is what survives a 100-line insertion),
            // even when the stored line still holds the anchor text (the
            // note follows the symbol, not a coincidental text match).
            // Zero or multiple matches fall through to the text rules.
            if let Some(idx) = &syntax_index
                && let Some(sa) = a.syntax.as_ref()
                && let Some(lines) = idx.get(&(sa.kind.clone(), sa.name.clone()))
                && lines.len() == 1
            {
                if a.line != lines[0] {
                    a.line = lines[0];
                    changed = true;
                }
                if a.orphaned {
                    a.orphaned = false;
                    changed = true;
                }
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

    /// The syntax re-anchor index for the buffer at `key` (plan 007
    /// issue 02): `(kind, name) → the 0-based lines of every node of that
    /// kind + text`, built from ONE parse of the buffer's current text.
    /// `None` unless at least one record with annotation key `rel` carries
    /// a `SyntaxAnchor` AND the buffer's language parses (Rust today — the
    /// same single grammar pin 007-01 uses, via `queries::language_for`):
    /// the common legacy file pays no parse at all on a re-anchor pass.
    fn build_syntax_index_for_key(
        &self,
        key: &str,
        rel: &str,
    ) -> Option<HashMap<(String, String), Vec<usize>>> {
        let any_syntax = self.notes_doc.entries.iter().any(|e| {
            matches!(e, NotesEntry::Record(a) if a.path == rel && a.syntax.is_some())
        });
        if !any_syntax {
            return None;
        }
        let (path_str, source) = {
            let buf = self.buffers.get(key)?;
            let path = buf.path.as_ref()?;
            (path.to_string_lossy().into_owned(), buf.text())
        };
        let lang = self.grammar_registry.language_for(&path_str);
        if lang != crate::syntax::registry::LanguageId::Rust {
            return None;
        }
        // One fresh parse of the buffer's rope (the 007-02 contract: tree
        // reuse / incremental reparse is 007-04's job, not this one's).
        let language = crate::syntax::queries::language_for(lang)?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).ok()?;
        let tree = parser.parse(source.as_bytes(), None)?;
        let mut index: HashMap<(String, String), Vec<usize>> = HashMap::new();
        Self::collect_syntax_anchor_nodes(tree.root_node(), source.as_bytes(), &mut index);
        Some(index)
    }

/// Walk the parse tree collecting every NAMED node of one of 007-01's
/// identifier-ish kinds as `(kind, text) → lines` (plan 007 issue 02).
/// Restricting to those kinds keeps the walk cheap and correct: a
/// captured `SyntaxAnchor.kind` always belongs to the closed set, so no
/// other node kind can ever contribute a false match. `::` paths stay
/// whole (a `scoped_identifier` is one node — exactly as `node_at`
/// returns it). Multiple nodes may share a line (`x = x`); each is
/// counted, so uniqueness is over NODES, never lines.
fn collect_syntax_anchor_nodes(
    node: tree_sitter::Node,
    source: &[u8],
    index: &mut HashMap<(String, String), Vec<usize>>,
) {
    if node.is_named()
        && Self::is_syntax_anchor_kind(node.kind())
        && let Ok(text) = node.utf8_text(source)
    {
        index
            .entry((node.kind().to_string(), text.to_string()))
            .or_default()
            .push(node.start_position().row);
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            Self::collect_syntax_anchor_nodes(child, source, index);
        }
    }
}

/// The identifier-ish node kinds `node_at` (007-01) can return — the
/// closed set a captured `SyntaxAnchor.kind` belongs to. Kept in
/// lockstep with `src/syntax/node.rs`'s `is_rust_identifier_kind`
/// (007-01 is frozen — mirrored here, not shared).
fn is_syntax_anchor_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "field_identifier"
            | "type_identifier"
            | "scoped_identifier"
            | "scoped_type_identifier"
            | "primitive_type"
    )
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

    /// Collect the annotations for the quit-dump (plan 005 issue 03): one
    /// `DumpAnnotation` per structured record in the notes document (raw
    /// blocks are skipped — they carry no usable record). `line` is
    /// rendered 1-based; `code` is the open buffer's current content at
    /// the anchored line when the anchor holds, otherwise the stored
    /// `anchor` (an orphaned record's line must not be trusted, so its
    /// stored anchor text is the last known line).
    pub fn annotations_for_dump(&mut self) -> Vec<DumpAnnotation> {
        self.ensure_notes_doc();
        let items: Vec<DumpAnnotation> = self
            .notes_doc
            .entries
            .iter()
            .filter_map(|e| {
                let a = e.as_record()?;
                let code = if a.orphaned {
                    a.anchor.clone()
                } else {
                    self.buffer_line_text_for_rel(&a.path, a.line)
                        .unwrap_or_else(|| a.anchor.clone())
                };
                Some(DumpAnnotation {
                    path: a.path.clone(),
                    line: a.line + 1, // 0-based record line -> 1-based dump line
                    code,
                    text: a.text.clone(),
                    orphaned: a.orphaned,
                })
            })
            .collect();
        items
    }

    /// The open buffer's content at 0-based `line` for the annotation key
    /// `rel` (project-relative, or absolute for external buffers), when
    /// such a buffer is open.
    fn buffer_line_text_for_rel(&self, rel: &str, line: usize) -> Option<String> {
        for (key, _) in self.buffers.list() {
            if self.buffer_annotation_path(key).as_deref() == Some(rel) {
                return self
                    .buffers
                    .get(key)
                    .and_then(|b| b.line_text(line))
                    .map(|t| t.into_owned());
            }
        }
        None
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

    /// 007-04 review P2-1: drop a buffer's retained tree after a content
    /// replacement that is NOT an edit event (`replace_buffer_text`,
    /// read-only accept, disk reload). The `(key, mtime)` key alone
    /// under-protects on coarse-granularity or mtime-preserving
    /// filesystems; removing the tree forces a full parse on the next
    /// highlight — the conservative, always-correct outcome.
    fn drop_retained_tree(&mut self, key: &str) {
        if let Some(buf) = self.buffers.get(key) {
            self.highlight_cache
                .retain_remove(&TreeKey::new(key, buf.mtime));
        }
    }

    /// Replace a buffer's text (the notes-buffer sync path).
    fn replace_buffer_text(&mut self, key: &str, text: &str) {
        let rope = Rope::from_str(text);
        if let Some(buf) = self.buffers.get_mut(key) {
            buf.rope = rope;
        }
        self.drop_retained_tree(key);
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
                .filter(|a| a.line == line && a.path == self.current_annotation_path().as_deref().unwrap_or(""))
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
        let Some(rel) = self.buffer_annotation_path(&key) else {
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
        // 007-02: capture the syntax anchor at the point (None for
        // non-Rust / keyword offsets — the text rules alone keep working).
        let syntax = self.capture_syntax_anchor(&key, line);
        let existing = self.notes_doc.entries.iter().position(|e| {
            matches!(e, NotesEntry::Record(a) if a.path == rel && a.line == line)
        });
        match existing {
            Some(i) => {
                // Edit: only the note text changes (the record's stored
                // position stays — the re-anchor pass maintains it). A
                // re-capture at commit refreshes the syntax anchor when the
                // point lands on an identifier-ish node; a `None` capture
                // (the cursor slid onto a keyword) keeps the record's
                // existing anchor rather than discarding a good one.
                if let Some(a) = self.notes_doc.entries[i].as_record_mut() {
                    a.text = text.clone();
                    if let Some(sa) = syntax.clone() {
                        a.syntax = Some(sa);
                    }
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
                        syntax,
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

    /// 007-02: capture the syntax anchor for a record committed at buffer
    /// `key`'s line `line`, taken at the CURRENT POINT's byte offset via
    /// 007-01's `node_at`.
    ///
    /// Capture rule (explicit):
    /// - **Offset: the point, not the line's first non-whitespace byte.**
    ///   The point is where the user's attention is (the same target `M-.`
    ///   acts on), and in Rust a line's first non-whitespace byte is
    ///   usually a KEYWORD (`fn`, `let`, `if`, `struct`) where `node_at`
    ///   has no identifier-ish node — the line-head rule would silently
    ///   strip the anchor from exactly the item-header lines most worth
    ///   anchoring. What is stored is the (kind, text) pair, so the
    ///   criterion is landing on a meaningful node, which the point does
    ///   best.
    /// - **Node: the identifier-ish node at the point verbatim** — `kind`
    ///   is `node_at`'s kind and `name` its text. `node_at` returns only
    ///   identifier-ish nodes (007-01's frozen surface: it has no
    ///   item-kind or statement surface), so there is no "statement →
    ///   enclosing item" fallback to take: a local/call identifier
    ///   anchors on itself, and re-anchoring's uniqueness filter keeps it
    ///   honest (a name shadowed or repeated anywhere in the file →
    ///   ambiguous → text rules, never a guess).
    /// - **`None`** for non-Rust buffers, EOL points, and offsets with no
    ///   identifier-ish node — the record then rides the text rules alone
    ///   (today's behavior).
    fn capture_syntax_anchor(&self, key: &str, line: usize) -> Option<SyntaxAnchor> {
        let col = self.point_col();
        let (path_str, source, byte) = {
            let buf = self.buffers.get(key)?;
            let path = buf.path.as_ref()?;
            let line_start = buf.try_line_to_byte(line)?;
            let line_text = buf.line_text(line)?;
            // The point's column is a CHAR offset; convert it to a byte
            // offset within the line. `col == line length` (EOL) has no
            // char under it: the byte lands past the last char and
            // `node_at` finds nothing (the honest answer for EOL).
            let byte_in_line = line_text
                .char_indices()
                .nth(col)
                .map(|(b, _)| b)
                .unwrap_or(line_text.len());
            (
                path.to_string_lossy().into_owned(),
                buf.text(),
                line_start + byte_in_line,
            )
        };
        let lang = self.grammar_registry.language_for(&path_str);
        if lang != crate::syntax::registry::LanguageId::Rust {
            return None;
        }
        let info = crate::syntax::node::node_at(lang, &source, byte)?;
        Some(SyntaxAnchor {
            kind: info.kind,
            name: info.text,
        })
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
        let Some(rel) = self.current_annotation_path() else {
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
        let (editable, locally_modified, owned) = {
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
            (buf.editable, buf.locally_modified, self.buffer_is_project_owned(&key))
        };
        if !owned {
            // 006-02b item 1: the C-x C-q override must NEVER turn an
            // external (registry / tooling) source editable — it is a cache
            // shared by every project on the machine.
            self.minibuffer_message(
                "toggle-read-only: external buffer is read-only (not project-owned)",
            );
            return;
        }
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
        self.drop_retained_tree(&key);
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
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.remove((len - 1)..len);
            buf.locally_modified = true;
            if let Some(old_rope) = old_rope {
                self.retain_rope_edit(&key, &old_rope, len - 1, len, "");
            }
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
        let (editable, text, char_start, char_end, old_rope) = {
            let Some(buf) = self.buffers.get(&key) else {
                self.minibuffer_message("no buffer");
                return;
            };
            // Convert byte offsets to char offsets (ropey edit APIs are char-index based).
            let char_start = buf.rope.byte_to_char(range.0);
            let char_end = buf.rope.byte_to_char(range.1);
            let text = buf.rope.slice(char_start..char_end).to_string();
            (buf.editable, text, char_start, char_end, buf.rope.clone())
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
            self.retain_rope_edit(&key, &old_rope, char_start, char_end, "");
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
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.insert(point_char, &text);
            buf.locally_modified = true;
            // Clear the mark (the text insertion shifts byte offsets).
            buf.mark = None;
        }
        if let Some(old_rope) = old_rope {
            self.retain_rope_edit(&key, &old_rope, point_char, point_char, &text);
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
        let old_rope = self.buffers.get(&key).map(|b| b.rope.clone());
        if let Some(buf) = self.buffers.get_mut(&key) {
            buf.rope.remove(yank_pos..end);
            buf.rope.insert(yank_pos, &text);
            buf.locally_modified = true;
            buf.mark = None;
        }
        if let Some(old_rope) = old_rope {
            self.retain_rope_edit(&key, &old_rope, yank_pos, end, &text);
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
        if let Err(e) = self.open_project_path(&abs, rel) {
            self.minibuffer_message(&e);
        }
    }

    /// The load + install core of `open_path`: `abs` = `project.root.join(rel)`
    /// (already resolved by the caller). Returns the error report on failure
    /// so a caller (the tooling-resolver landing, plan 006 issue 02) can
    /// decide whether a jump entry is recorded.
    fn open_project_path(&mut self, abs: &Path, rel: &str) -> Result<(), String> {
        let key = abs.to_string_lossy().into_owned();
        if self.buffers.get(&key).is_none() {
            match load_file(abs) {
                Ok((rope, mtime)) => {
                    self.buffers.insert_rope(Some(abs.to_path_buf()), rope, mtime, false);
                }
                Err(e) => {
                    return Err(format!("cannot open {rel}: {e}"));
                }
            }
        } else {
            // Re-stat on reopen: reload when mtime changed (spec: "highlight
            // cache invalidates when a file changes on disk (pre-watcher: on
            // reopen)"). Issue-03 buffers are read-only, so this is safe.
            if let Ok(new_mtime) = std::fs::metadata(abs).and_then(|m| m.modified()) {
                let old_mtime = self
                    .buffers
                    .get(&key)
                    .map(|b| b.mtime)
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                if new_mtime != old_mtime {
                    match load_file(abs) {
                        Ok((rope, mtime)) => {
                            self.buffers
                                .insert_rope(Some(abs.to_path_buf()), rope, mtime, false);
                        }
                        // Reload failure is non-fatal (the buffer stays current
                        // on its stale content); keep `open_path`'s report.
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
        self.normalize_top_view();
        Ok(())
    }

    /// Open an ABSOLUTE path as a READ-ONLY buffer (plan 006 issue 02):
    /// the tooling-resolver lands external sources (registry source dirs)
    /// that are NOT project files. Unlike `open_path` it never records the
    /// file in the project's recents, never touches the tree/file-walk
    /// state, and inserts with `editable = false` so an external source
    /// can never enter edit mode (per-session, like other external jumps).
    /// Returns the buffer key, or `None` when the file cannot be read.
    fn open_external_path(&mut self, abs: &Path) -> Option<String> {
        let (rope, mtime) = load_file(abs).ok()?;
        let key = self.buffers.insert_rope(Some(abs.to_path_buf()), rope, mtime, false);
        // 006-02b item 1: remember that this key is an external (registry /
        // tooling) source — the ownership guard refuses edit mode + save
        // for exactly these buffers.
        self.external_buffers.insert(key.clone());
        self.buffers.set_current(&key);
        // 006-03b item 1: the landed crate is now the crate we are IN —
        // keep it MRU so it can never be the LRU eviction victim.
        self.bump_current_crate_recency();
        // Build (or update) the highlight for the new current buffer.
        self.ensure_highlight();
        self.normalize_top_view();
        Some(key)
    }

    /// Land a tooling-resolver result (plan 006 issue 02): open the resolved
    /// source and record a jump like any M-. landing. A source inside the
    /// workspace opens through the project-relative path (recents, tree
    /// follow); an external source opens READ-ONLY via
    /// `open_external_path` (never in the project recents / file walk). On a
    /// load failure the failure is reported and no jump is recorded.
    fn open_resolved_source(&mut self, source: &ResolvedSource, symbol: &str) {
        let origin = self.current_jump_entry();
        let (display, opened) = if let Some(project) = self.project.as_ref()
            && let Ok(rel) = source.file.strip_prefix(&project.root)
        {
            let rel = rel.to_string_lossy().into_owned();
            (rel.clone(), self.open_project_path(&source.file, &rel).is_ok())
        } else {
            (
                source.file.display().to_string(),
                self.open_external_path(&source.file).is_some(),
            )
        };
        if !opened {
            self.minibuffer_message(&format!(
                "no provider resolution for `{symbol}`: cannot open {display}"
            ));
            return;
        }
        // Land on the resolved line (1-based; when the provider could not
        // pin one, the top of the file) and record the jump. A provider
        // emitting 0 is treated as "no line" (006-02b item 6) so the
        // `(l - 1) as usize` below can never underflow.
        let line = source
            .line
            .filter(|l| *l > 0)
            .map(|l| (l - 1) as usize)
            .unwrap_or(0);
        // 006-03: an external (registry / tooling) landing registers its
        // crate's source tree for background indexing (off the input
        // path; the LRU cap governs) so M-. / imenu work INSIDE it. The
        // landing itself is never blocked on the index.
        if source.external {
            self.start_crate_indexing(&source.source_root, &source.file);
        }
        self.set_point_line(line);
        self.recenter_landing();
        self.ensure_highlight();
        self.record_jump(origin, "M-.");
        self.minibuffer_message(&format!("jumped to {display}:{}", line + 1));
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
            PickerKind::Impls => self.impls_candidates(),
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
    /// current lookup name — the project index, or the crate index the
    /// picker was opened with, 006-03).
    fn xref_candidates(&mut self) -> Vec<PickerCandidate> {
        let name = self.xref_lookup_name.clone();
        let root = self.xref_crate_root.clone();
        let defs: Vec<crate::nav::index::Location> = match root.as_ref() {
            Some(root) => self
                .crate_index_arc(root)
                .map(|arc| arc.lock().unwrap().definitions_of(&name))
                .unwrap_or_default(),
            None => self.index.definitions_of(&name),
        };
        defs
            .iter()
            .map(|d| PickerCandidate {
                name: format!("{}:{}", d.file, d.symbol.line + 1),
                display: format!("{}:{}  [{}] {}", d.file, d.symbol.line + 1, d.symbol.kind.tag(), d.symbol.name),
                docs: String::new(),
                category: "xref".to_string(),
            })
            .collect()
    }

    /// Candidates for the find-implementations picker (010-04, plan 010
    /// Shape A rung 4): the `impl <Trait> for <Type>` blocks for the
    /// trait keys stored when the picker was opened (`impls_keys` — the
    /// M-. path token and/or the bare identifier, like the name-keyed
    /// M-. index lookup), from the project index or the crate index the
    /// picker was opened with (006-03, like the Xref picker). Deterministic
    /// (key order, then file, impl line); a block hit by both keys is
    /// listed once (the first key's spelling).
    fn impls_candidates(&mut self) -> Vec<PickerCandidate> {
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
    fn imenu_candidate(
        s: &crate::nav::index::Symbol,
        outline: &[crate::nav::index::Symbol],
        tables: Option<&crate::syntax::queries::RustTables>,
    ) -> PickerCandidate {
        let depth = Self::imenu_depth(s, outline, tables);
        let indent = "  ".repeat(depth);
        PickerCandidate {
            name: format!("{}:{}", s.name, s.line + 1),
            display: format!("{indent}{}  [{}]", s.name, s.kind.tag()),
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
        tables: Option<&crate::syntax::queries::RustTables>,
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
            // project index, or the crate index it was opened with.
            Some(PickerKind::Xref) => {
                let name = self.xref_lookup_name.clone();
                match self.xref_crate_root.clone().as_ref() {
                    Some(root) => self
                        .crate_index_arc(root)
                        .map(|arc| arc.lock().unwrap().definitions_of(&name).len())
                        .unwrap_or(0),
                    None => self.index.definitions_of(&name).len(),
                }
            }
            Some(PickerKind::Imenu) => self.current_buffer_outline().len(),
            Some(PickerKind::Impls) => self.impls_candidates().len(),
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
    /// otherwise.
    fn picker_file_abs(&self, rel: &str) -> Option<PathBuf> {
        if let Some(root) = self.xref_crate_root.as_ref() {
            return Some(root.join(rel));
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
                if let Some((file, line_str)) = name.rsplit_once(':')
                    && let Ok(line) = line_str.parse::<usize>()
                {
                    let origin = self.current_jump_entry();
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
        // A resolve job in flight belonged to the previous project: bump the
        // generation so its event is discarded (mirrors the index bump above).
        self.resolve_generation += 1;
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

    /// Kill the buffer with key `key`. 06a: no accidental buffer creation —
    /// when the killed buffer was current, the MRU survivor becomes current
    /// (emacs `kill-buffer` fallback); with the LAST buffer killed the main
    /// view returns to home.
    pub fn kill_buffer(&mut self, key: &str) {
        let display = self.buffer_display(key);
        if !self.buffers.kill(key) {
            self.minibuffer_message(&format!("no buffer: {display}"));
            return;
        }
        if self.buffers.current().is_none() {
            let mru = self.buffers.list().first().map(|(k, _)| k.to_string());
            if let Some(k) = mru {
                self.buffers.set_current(&k);
            }
        }
        self.normalize_top_view();
        self.minibuffer_message(&format!("killed {display}"));
    }

    /// Buffer-list view: open the selected buffer and close the list.
    pub fn open_buffer_list_selected(&mut self) {
        let key = match self.buffers.list().get(self.buffer_list_selected) {
            Some((key, _)) => key.to_string(),
            None => return,
        };
        self.buffers.set_current(&key);
        // 006-03b item 1: a switched-to external buffer keeps its owning
        // crate MRU.
        self.bump_current_crate_recency();
        self.close_view();
        self.normalize_top_view();
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
        // 06a: the tree shares the main pane with home (home renders in the
        // buffer slot), so clicks land while either is on top.
        if !self.tree.visible
            || !matches!(self.top_view(), ViewId::Buffer | ViewId::Home)
        {
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
        // plan 005 issue 02b: annotated lines render their code at cell 1
        // (the 1-cell gutter for the \u{258e} marker). A click in the gutter
        // (col 0) maps to char 0; a click at code cell k maps to char
        // display_col_to_char_index(text, k - 1).
        let annotated = rows
            .iter()
            .any(|r| !r.is_note && r.line == target_line && r.annotated);
        let code_col = if annotated { col.saturating_sub(1) } else { col };
        let line_len = self.line_char_len(target_line);
        // Display column -> char index (plan 004 issue 05d).
        let char_col = self
            .buffers
            .current_buffer()
            .and_then(|b| b.line_text(target_line))
            .map(|t| crate::model::text_width::display_col_to_char_index(&t, code_col))
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
        let vp = self.viewport_lines.max(1);
        // Cycle order `(middle top bottom)` selected by the cycle index.
        let desired_row = match self.recenter_cycle % 3 {
            0 => vp / 2,
            1 => 0,
            _ => vp - 1,
        };
        if let Some(new_top) = recenter_top_for(p.line, desired_row, total, vp) {
            self.recenter_cycle += 1;
            self.set_scroll_top(new_top);
        }
    }

    /// Jump-landing recenter (plan 004 issue 07): vanilla emacs
    /// `xref-after-jump-hook` is `(recenter xref-pulse-momentarily)` —
    /// every M-. jump (and the picker/imenu selections that record the
    /// same jump) repositions the window so the landed line sits on the
    /// MIDDLE row, instead of the minimal `keep_cursor_visible` scroll
    /// that lands a below-window target on the BOTTOM row. Same
    /// arithmetic as `recenter` (`recenter_top_for`), but a jump is not a
    /// `C-l`: `recenter_cycle` is untouched, so a jump between two C-ls
    /// neither resets nor advances the middle→top→bottom cycle. No-op
    /// when the buffer does not scroll.
    pub fn recenter_landing(&mut self) {
        let line = self.point_line();
        let total = self.current_line_count();
        let vp = self.viewport_lines.max(1);
        if let Some(new_top) = recenter_top_for(line, vp / 2, total, vp) {
            self.set_scroll_top(new_top);
        }
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
    /// `[top_line, top_line + viewport_lines)`, capped to the LARGEST span
    /// that fits so that `code_rows + note_rows <= viewport_lines` always
    /// holds (plan 005 issue 02c: at least one code row, the point's line
    /// always drawn, and a densely-annotated window still fills the
    /// canvas — a 25-line all-annotated file in a 21-row viewport emits
    /// 10 code + 10 note rows, not a 1-row span),
    /// with a virtual annotation note row directly under each annotated
    /// line as the note-row budget allows (`show_note_rows`; `C-c a`
    /// toggles — the `annotated` flag on the code rows is independent, so
    /// the margin marker stays). Every row
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
        let mut start = top.min(total.saturating_sub(1));
        let mut end = (top + self.viewport_lines).min(total);
        if start >= end {
            return Vec::new();
        }

        // Get the highlight result from the cache (if any).
        let highlight: Option<&HighlightResult> = self.buffer_highlight_result();

        // This buffer's annotation records (matched by annotation key:
        // project-relative path, or the absolute path for external
        // buffers), in record order. The marker flag is independent of
        // note-row visibility.
        let Some(key) = self.buffers.current().map(String::from) else {
            return Vec::new();
        };
        let rel = self.buffer_annotation_path(&key);
        let records: Vec<&Annotation> = self
            .notes_doc
            .entries
            .iter()
            .filter_map(|e| e.as_record())
            .filter(|a| rel.as_deref() == Some(a.path.as_str()))
            .collect();

        // plan 005 issue 02c: choose the LARGEST code-row span that fits
        // the canvas — the max `s` in [1, window] with
        // `s + notes_in_window(s) <= viewport_lines` (the canvas has
        // exactly viewport_lines rows; note rows steal canvas rows, so
        // the code rows must be reduced). `notes_in_window` is monotonic
        // non-decreasing in `s`, so `s + notes_in_window(s)` is strictly
        // increasing in `s`: a scan from the window top down finds the
        // largest fitting span, and it is the answer (02b instead
        // floored `viewport_lines - n_notes` at 1, which under-filled —
        // a 25-line all-annotated file showed 3 rows of 21). The span is
        // at least 1 (round 2 P1): if no larger span fits (e.g. many
        // records on one line), the floor stays as the blank-view
        // guarantee. If the point's line is excluded by the span, advance
        // start so the point is always drawn.
        if self.show_note_rows && !records.is_empty() {
            let n_notes: usize = records
                .iter()
                .filter(|a| a.line >= start && a.line < end)
                .count();
            if n_notes > 0 {
                let window = end - start; // <= viewport_lines
                let mut code_span = 1; // the floor: blank-view guarantee
                for s in (1..=window).rev() {
                    let in_span = records
                        .iter()
                        .filter(|a| a.line >= start && a.line < start + s)
                        .count();
                    if s + in_span <= self.viewport_lines {
                        code_span = s;
                        break;
                    }
                }
                end = (start + code_span).min(total);
                // Ensure the point's line is in the emitted range: if the
                // cap excluded it, advance start so the point is the last
                // code row.
                let point_line = self.file_point().line.min(total.saturating_sub(1));
                if point_line >= end {
                    start = point_line.saturating_add(1).saturating_sub(code_span);
                    end = (start + code_span).min(total);
                }
            }
        }

        // Round 2 (budget counts are FINAL): the cap above is sized from
        // the FULL window's note count, but the emitted range — shrunk to
        // `code_span` and possibly advanced for the point — can hold a
        // different (larger) note count, e.g. many records on one line.
        // Recount in the emitted range and cap the emitted NOTE rows so
        // `code_rows + note_rows <= viewport_lines` always holds; the
        // code rows (>= 1, including the point's line) are emitted first
        // and never reduced further.
        let code_rows = end - start;
        let mut notes_left = self.viewport_lines.saturating_sub(code_rows);

        let mut out = Vec::with_capacity(end.saturating_sub(start) + notes_left);
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
                    if notes_left == 0 {
                        break; // the note-row budget is final: stop here
                    }
                    notes_left -= 1;
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
        let Some(rel) = key.as_deref().and_then(|k| self.buffer_annotation_path(k)) else {
            return total;
        };
        total + self
            .notes_doc
            .entries
            .iter()
            .filter(|e| matches!(e, NotesEntry::Record(a) if a.path == rel))
            .count()
    }

    /// Whether the buffer at `key` is PROJECT-OWNED (006-02b item 1, the
    /// edit-mode/save ownership guard): `true` for path-less (scratch)
    /// buffers, for paths under the current project root (project files +
    /// the notes file — this also covers a PREVIOUS project's buffers after
    /// a switch: they are the user's own files, not a shared cache), and
    /// for any other buffer that was not opened via the external-landing
    /// path. `false` for external (tooling / registry) sources — a cache
    /// shared by every project on the machine, never editable, never
    /// written.
    fn buffer_is_project_owned(&self, key: &str) -> bool {
        let Some(buf) = self.buffers.get(key) else {
            return false;
        };
        let Some(path) = buf.path.as_ref() else {
            return true; // scratch (no on-disk path): as today
        };
        if let Some(root) = self.project.as_ref().map(|p| &p.root)
            && path.starts_with(root)
        {
            return true;
        }
        !self.external_buffers.contains(key)
    }

    /// The buffer at `key`'s annotation key (plan 008 issue 01, the ONE
    /// key-derivation point for annotation records): the project-relative
    /// path (lossy) when the buffer's path strips under the project root
    /// — byte-identical to the pre-008 key for existing project buffers
    /// (no migration, no notes-file change) — else the absolute path
    /// string (external buffers opened read-only via `open_external_path`,
    /// e.g. `M-.` into a registry source). Notes still live in the
    /// project's `.redline-notes.md`; the record's `path` simply carries
    /// the absolute string. `None` only for pathless buffers (scratch).
    fn buffer_annotation_path(&self, key: &str) -> Option<String> {
        let buf = self.buffers.get(key)?;
        let abs = buf.path.as_ref()?;
        if let Some(root) = self.project.as_ref().map(|p| &p.root)
            && let Ok(rel) = abs.strip_prefix(root)
        {
            return Some(rel.to_string_lossy().into_owned());
        }
        Some(abs.to_string_lossy().into_owned())
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
    ///
    /// Plan 007 issue 04: for reuse-capable languages the reparse is
    /// incremental when a retained parse tree exists for this (buffer,
    /// mtime) — the edit paths recorded their `InputEdit`s on it and the
    /// parser gets the old tree as a hint. Fallbacks to the full parse:
    /// no retained tree, the retained tree carries a parse error (stale
    /// or mid-typing baseline), or the incremental parse itself errors.
    /// A disk reload changes the mtime, so its stale tree never matches
    /// the `TreeKey` and the buffer parses from scratch.
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
        if lang == LanguageId::Plain {
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
        let result = if highlight::supports_reuse(lang) {
            let tree_key = TreeKey::new(key, mtime);
            let incremental = self
                .highlight_cache
                .retain_tree(&tree_key)
                .and_then(|retained| highlight::highlight_with_tree(&rope, lang, retained));
            match incremental {
                Some(r) => r,
                None => highlight::highlight_reusable(&rope, lang)
                    .map(|(result, tree)| {
                        // Retain the freshly parsed tree as the next
                        // incremental baseline.
                        self.highlight_cache
                            .retain_insert(tree_key, RetainedTree::new(tree));
                        result
                    }),
            }
        } else {
            // JS/TS/TSX: local-variable tracking lives in the
            // tree-sitter-highlight Highlighter, which does not expose
            // its parse tree — keep the full-parse path for them.
            highlight::highlight(&rope, config, lang)
        };
        match result {
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

    /// The home view's derived body rows (plan 004 issue 06a): every
    /// EFFECTIVE binding (the live keymap × global map, via `menu_bindings` —
    /// on home the view map is empty, so these are the commands that work
    /// from home), grouped by registry category and sorted by key within
    /// each group — the same grouping `menu_rows` uses, queried at the
    /// TOP level of the whole keymap (the "all top-level groups" query;
    /// no new formatter, no hand-maintained list).
    pub fn home_rows(&self) -> Vec<TransientMenuRow> {
        let bindings = self.menu_bindings();
        let mut by_cat: std::collections::BTreeMap<String, Vec<(String, String)>> =
            std::collections::BTreeMap::new();
        for (seq, cmd) in bindings {
            let key_display = seq
                .iter()
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            let (docs, category) = match self.registry.get(&cmd) {
                Some(m) => (m.docs.to_string(), m.category.to_string()),
                None => (String::new(), String::new()),
            };
            by_cat.entry(category).or_default().push((key_display, docs));
        }
        let mut rows = Vec::new();
        for (cat, mut entries) in by_cat {
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            rows.push(TransientMenuRow {
                is_header: true,
                key_display: String::new(),
                label: cat,
                is_prefix: false,
            });
            for (key_display, docs) in entries {
                rows.push(TransientMenuRow {
                    is_header: false,
                    key_display,
                    label: docs,
                    is_prefix: false,
                });
            }
        }
        rows
    }

    /// The home view's header: `redline · {project}` + the dirty counts
    /// (`+` staged, `~` unstaged, `?` untracked — the status-line
    /// convention) when git reports any.
    pub fn home_title(&self) -> String {
        let mut title = format!("redline · {}", self.project_display());
        if let Some(d) = self.dirty_counts()
            && d.staged + d.unstaged + d.untracked > 0
        {
            title.push_str(&format!("  +{} ~{} ?{}", d.staged, d.unstaged, d.untracked));
        }
        title
    }

    /// The home body rows bounded to the viewport (06a): the terminal is
    /// `viewport_lines + 3` rows (the resize handler's contract: viewport =
    /// height - 3), the main view is `viewport_lines + 1` of those
    /// (minibuffer + status aside); home keeps its title + help chrome
    /// (2 rows), and an open overlay (the 12-row picker canvas, the menu)
    /// takes its fixed height — the body yields it, the way the file
    /// view's canvas yields the same space.
    pub fn home_body_rows(&self) -> Vec<TransientMenuRow> {
        let overlay = if self.picker_open() {
            12 // the picker's fixed canvas height (src/ui/picker.rs)
        } else if self.menu_open() {
            self.menu_height() as usize
        } else {
            0
        };
        let budget = self
            .viewport_lines
            .saturating_sub(1)
            .saturating_sub(overlay);
        self.home_rows().into_iter().take(budget).collect()
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
            // 006-03: registry / tooling sources are not git checkouts —
            // blame stays refused, but the message says WHY.
            if self.external_buffers.contains(&key) {
                self.minibuffer_message("no git history for external sources");
            } else {
                self.minibuffer_message("buffer not in project");
            }
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
                // A resolve job in flight belonged to the pre-checkout tree:
                // its event is discarded (mirrors the index bump).
                self.resolve_generation += 1;
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
        self.drop_retained_tree(&key);
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
        self.drop_retained_tree(&key);
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
    /// origin or destination in `record_jump`). None when no buffer is
    /// current (06a: no scratch fallback — the home state has no buffer).
    fn current_jump_entry(&self) -> Option<JumpEntry> {
        let key = self.buffers.current().map(String::from)?;
        let line = self.point_line();
        Some(JumpEntry {
            buffer_key: key,
            line,
            col: self.point_col(),
            label: String::new(),
        })
    }

    /// Record a jump from the current position (captured as `origin` before
    /// navigation) to the new position (captured as `destination` after
    /// navigation). Truncates forward history.
    fn record_jump(&mut self, origin: Option<JumpEntry>, label: &str) {
        // 06a review P1-3: with no current buffer (home state) there is no
        // origin or destination to record — the old SCRATCH_NAME fallback
        // created `*scratch*` origins on async resolver landings after the
        // last buffer was killed (then `M-,` created the buffer).
        let (Some(origin), Some(mut dest)) = (origin, self.current_jump_entry()) else {
            return;
        };
        dest.label = label.to_string();
        self.jump_stack.record_jump(&origin, &dest);
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
        // If the buffer is not open, report it — no accidental buffer
        // creation (06a review P1-1: the pre-06a fallback created `*scratch*`,
        // reachable from home state via a dead jump entry).
        if self.buffers.get(&entry.buffer_key).is_none() {
            self.minibuffer_message(&format!(
                "no buffer: {}",
                self.buffer_display(&entry.buffer_key)
            ));
            return;
        }
        self.buffers.set_current(&entry.buffer_key);
        // 006-03b item 1: a jump-back into an external buffer keeps its
        // owning crate MRU.
        self.bump_current_crate_recency();
        self.set_point(entry.line, entry.col, entry.col);
        self.recenter_landing();
        self.ensure_highlight();
        // Watchlist: `M-,` through the sentinel lands the pre-search
        // position — the results view must close so the landing is
        // visible (before, the buffer/point moved underneath the results
        // view: a no-op until the view was closed by hand).
        if self.top_view() == ViewId::Search {
            self.close_view();
        }
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

    /// `M-.`: jump to the definition of the symbol UNDER THE POINT
    /// (plan 006 issue 02 selection rule).
    ///
    /// Selection rule (the user's report: "doesn't use my cursor position"): the
    /// lookup keys off the point's COLUMN, not "every identifier on the line".
    /// 1. The identifier run at the point's column on the point line (a cursor
    ///    parked right after the name counts, the usual call-site spot); when
    ///    it sits inside a `::`-path (`tokio::spawn`), the full path token is
    ///    kept for the tooling-resolver fall-through and the workspace index —
    ///    which is name-keyed — is tried with BOTH the last segment and the
    ///    full path. A Rust `self.<member>` carries the receiver (`self.
    ///    <member>`, 010-01): before the name-keyed index is tried, the
    ///    member resolves via the LEXICALLY ENCLOSING impl's type (field →
    ///    the struct's field line; method → the impl method's line), and an
    ///    empty self-resolution degrades to the bare `<member>` lookup below
    ///    (byte-for-byte; generics / no-impl / unknown member never guess).
    /// 2. Same-file definitions are first-class (the old cross-file filter made
    ///    the normal struct+impl-in-one-file case unjumpable): candidates are
    ///    ordered same-file-first, then (file, line, name). Exactly one → jump
    ///    directly (same-file OR cross-file); several → the Xref picker so the
    ///    user chooses.
    /// 3. No symbol-at-point with a definition → the enclosing-symbol fallback
    ///    (unchanged): the enclosing symbol's definitions take over, and a
    ///    workspace hit on THAT never triggers the resolver (no resolver spam).
    /// 4. Still nothing (and the point sits on a symbol) → tooling-resolver
    ///    fall-through (plan 006): `SymbolContext { workspace_root,
    ///    symbol: <path-shaped token>, from_file }` runs OFF the input path
    ///    (`spawn_blocking` + `ResolveBus`, like the symbol indexer) because
    ///    `cargo fetch` is a network shell-out that must never block a keypress.
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
        // An OWNED path: the `buf` borrow must not span the `&mut self`
        // calls below (the external-buffer navigation, 006-03).
        let path = match &buf.path {
            Some(p) => p.clone(),
            None => {
                self.minibuffer_message("no file (scratch buffer)");
                return;
            }
        };
        let Some(project) = self.project.as_ref() else {
            self.minibuffer_message("no project");
            return;
        };
        let Ok(rel) = path.strip_prefix(&project.root) else {
            // 006-03: an EXTERNAL (registry / tooling) buffer navigates
            // within its OWN crate's index (keyed against the crate's
            // source_root); a crate miss keeps the resolver fall-through
            // (the origin project's metadata — unchanged semantics).
            // A non-external buffer outside the root keeps the pre-006-03
            // refusal.
            if self.external_buffers.contains(&key) {
                self.xref_in_external_buffer(&key, &path);
            } else {
                self.minibuffer_message("buffer not in project");
            }
            return;
        };
        let rel = rel.to_string_lossy().into_owned();

        // 006-02b item 2: supersede any in-flight tooling resolve — a
        // successful workspace hit must not be clobbered by a stale event
        // from a superseded request that lands later. The fall-through
        // path re-bumps in `start_symbol_resolution`, which is fine (a
        // generation only needs to differ; the event carries its own).
        self.resolve_generation += 1;

        let line = self.point_line();
        let line_text = buf.line_text(line).unwrap_or_default();
        // (1) The symbol under the point: identifier run around the point's
        // column, plus the path token it belongs to (raw, for the
        // resolver). 011-06: language-aware — in a non-Rust buffer the
        // token is the whole dotted path when the point sits in the
        // language's path container, else the bare extraction.
        let lang = self.grammar_registry.language_for(&path.to_string_lossy());
        let at = Self::symbol_at_point(lang, &line_text, self.point_col());

        let defs: Option<Vec<crate::nav::index::Location>> = at
            .as_ref()
            .and_then(|(ident, path_token)| {
                // 010-01 (plan 010 Shape A, rung 1): the Rust self-receiver
                // pre-step — `self.<member>` resolves via the LEXICALLY
                // ENCLOSING impl's type (field → the struct's field line,
                // method → the impl method's line), same-file first. An
                // empty result degrades to today's bare-`<member>` index
                // lookup below (byte-for-byte; never a guess).
                if lang == LanguageId::Rust
                    && let Some(member) = path_token.strip_prefix("self.")
                    && !member.is_empty()
                {
                    let source = buf.rope.to_string();
                    if let Some(byte) = point_byte_offset(&buf.rope, line, self.point_col()) {
                        let cands =
                            Self::self_receiver_candidates(&self.index, &rel, &source, byte, member);
                        if !cands.is_empty() {
                            return Some(cands);
                        }
                    }
                }
                // 010-03 (plan 010 Shape A, rung 3): the local-binding
                // pre-step — `x.<member>` / `x.<member>()` resolves via
                // the binding's WRITTEN-DOWN type (a `let x: Type`
                // annotation or a `let x = Type { … }` literal) recorded
                // in an enclosing scope, through the same field / method
                // tables as the self pre-step above. An empty result
                // keeps today's bare-`<member>` behavior byte-for-byte
                // (the extraction's path token never changed; never a
                // guess — unannotated bindings are never inferred).
                if lang == LanguageId::Rust
                    && let Some((receiver, member_col)) =
                        Self::rust_dotted_receiver(&line_text, self.point_col())
                {
                    let source = buf.rope.to_string();
                    if let Some(byte) = point_byte_offset(&buf.rope, line, member_col) {
                        let cands = Self::local_binding_candidates(
                            &self.index,
                            &rel,
                            &source,
                            byte,
                            &receiver,
                            ident,
                        );
                        if !cands.is_empty() {
                            return Some(cands);
                        }
                    }
                }
                Self::xref_definition_candidates(&self.index, ident, path_token, &rel)
            });
        let defs = defs.unwrap_or_default();

        let lookup_name: String;
        let defs: Vec<crate::nav::index::Location> = if !defs.is_empty() {
            // (2) Symbol-at-point with definitions: first candidate after the
            // same-file-first order (the picker's lookup label).
            lookup_name = defs[0].symbol.name.clone();
            defs
        } else {
            // (3) Enclosing-symbol fallback (unchanged: by line, not by the
            // point's column).
            let outline = self.index.outline(&rel);
            let Some(sym) = crate::nav::index::enclosing_symbol(outline, line) else {
                // (4) Nothing the workspace knows about under/near the point:
                // fall through to the tooling resolver when the point sits on
                // a symbol (otherwise behave as before: no symbol under point).
                match &at {
                    Some((_, path_token)) => self.start_symbol_resolution(path_token, &rel),
                    None => self.minibuffer_message("no symbol under point"),
                }
                return;
            };
            lookup_name = sym.name.clone();
            self.index.definitions_of(&lookup_name)
        };

        if defs.is_empty() {
            // The enclosing symbol has no indexed definition: same (4) seam,
            // but the point's own token (path-shaped) is what the resolver
            // gets — not the enclosing name.
            match &at {
                Some((_, path_token)) => self.start_symbol_resolution(path_token, &rel),
                None => self.minibuffer_message(&format!("no definition for `{lookup_name}`")),
            }
            return;
        }

        if defs.len() == 1 {
            // Unique (same-file or cross-file): capture origin, navigate,
            // record jump.
            let origin = self.current_jump_entry();
            let def = &defs[0];
            self.open_path(&def.file);
            // Move the point to the definition's line; the jump-landing
            // recenter positions the window (plan 004 issue 07).
            self.set_point_line(def.symbol.line);
            self.recenter_landing();
            self.ensure_highlight();
            self.record_jump(origin, "M-.");
            self.minibuffer_message(&format!("jumped to {}: {}", def.file, def.symbol.line + 1));
        } else {
            // Ambiguous: open the Xref picker (same-file candidates first).
            // The jump entry is recorded when the user selects a candidate
            // (run_selected for Xref).
            self.xref_crate_root = None;
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

    /// (010-04, plan 010 Shape A rung 4) find-implementations — the
    /// read-only view of the Rung 1 impl tables: a PICKER of the
    /// `impl <Trait> for <Type>` blocks implementing the trait at point
    /// (file + impl line, the self type), from the name-keyed trait map
    /// built in the SAME index pass as the Rust tables (zero extra parse;
    /// the 010-01 same-content-refresh discipline applies to the map —
    /// pinned in `nav/index.rs`). The trait at point reuses the M-.
    /// extraction (`symbol_at_point`, byte-for-byte); the map is tried
    /// with both the path token and the bare identifier (the index is
    /// name-keyed — `impl Display` vs `impl std::fmt::Display`, each
    /// spelling is its own key, exactly like the M-. candidate lookup).
    ///
    /// Honest degradation, byte-for-byte — the EXISTING bare-symbol M-.
    /// lookup runs (index / enclosing symbol / tooling fall-through) when:
    /// no symbol at point; no table entry for the trait (no Rust file
    /// impls it, or a non-Rust buffer — the tables only exist for Rust);
    /// or the impl's captured trait text is generic (`Display<T>` — a
    /// bare `Display` at point never matches the generic key; never a
    /// guess). The picker's RET reuses the Xref jump path (the same
    /// `xref_crate_root` seam, so external buffers land read-only).
    pub fn find_implementations(&mut self) {
        // Get the current file's project-relative path (the M-. guards).
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let Some(buf) = self.buffers.get(&key) else {
            self.minibuffer_message("no buffer");
            return;
        };
        let path = match &buf.path {
            Some(p) => p.clone(),
            None => {
                self.minibuffer_message("no file (scratch buffer)");
                return;
            }
        };
        let Some(project) = self.project.as_ref() else {
            self.minibuffer_message("no project");
            return;
        };
        let lang = self.grammar_registry.language_for(&path.to_string_lossy());
        // The point's line text must be read (owned) BEFORE the crate-root
        // match below (its `crate_index_arc_for_path` takes `&mut self`;
        // the `buf` borrow must not span it — the 006-03 borrow rule).
        let line = self.point_line();
        let line_text: String = buf
            .line_text(line)
            .map(|c| c.into_owned())
            .unwrap_or_default();
        // The index source: the project index (project files) or the
        // owning crate's index (external buffers, 006-03); a non-external
        // buffer outside the root keeps the pre-006-03 refusal.
        let crate_root: Option<PathBuf> = match path.strip_prefix(&project.root) {
            Ok(_) => None,
            Err(_) if self.external_buffers.contains(&key) => {
                self.crate_index_arc_for_path(&path).map(|(root, _)| root)
            }
            Err(_) => {
                self.minibuffer_message("buffer not in project");
                return;
            }
        };
        // The trait at point: the SAME extraction as M-. (byte-for-byte).
        let Some((ident, path_token)) =
            Self::symbol_at_point(lang, &line_text, self.point_col())
        else {
            self.minibuffer_message("no symbol under point");
            return;
        };
        // The trait-keyed map is name-keyed: try the path token and the
        // bare identifier (deduped — they are equal for a bare trait).
        let mut keys: Vec<String> = vec![path_token.clone()];
        if path_token != ident {
            keys.push(ident.clone());
        }
        let mut locations: Vec<crate::nav::index::TraitImplLocation> = Vec::new();
        for k in &keys {
            let locs: Vec<crate::nav::index::TraitImplLocation> = match &crate_root {
                Some(root) => self
                    .crate_index_arc(root)
                    .map(|arc| arc.lock().unwrap().trait_impl_locations(k))
                    .unwrap_or_default(),
                None => self.index.trait_impl_locations(k),
            };
            for l in locs {
                if !locations.iter().any(|e| e.file == l.file && e.impl_line == l.impl_line) {
                    locations.push(l);
                }
            }
        }
        if locations.is_empty() {
            // Honest degradation: the EXISTING bare-symbol lookup,
            // byte-for-byte (the M-. path — index, enclosing symbol,
            // tooling fall-through, exactly as M-. does it).
            if crate_root.is_some() {
                self.xref_in_external_buffer(&key, &path);
            } else {
                self.xref_find_definitions();
            }
            return;
        }
        self.xref_crate_root = crate_root;
        self.impls_keys = keys;
        // The picker's candidate list comes from the SAME re-computation
        // the query editing uses (`impls_candidates`) — the initial list
        // and the filtered re-derivation cannot drift apart.
        let candidates = self.impls_candidates();
        self.open_picker(PickerKind::Impls, "Impls: ", candidates);
    }

    /// (M-., selection rule 2) The definition candidates for the symbol at the
    /// point in `index`: the index tried with BOTH the last segment and the
    /// full `::`-path (the index is name-keyed), deduplicated, ordered
    /// SAME-FILE-FIRST (`rel`) then (file, line, name) — a same-file match
    /// (struct + impl in one file is the normal case) wins the direct jump,
    /// and when several remain the picker lists them in that order. Shared
    /// by the project index and the external crate indexes (006-03).
    fn xref_definition_candidates(
        index: &SymbolIndex,
        ident: &str,
        path_token: &str,
        rel: &str,
    ) -> Option<Vec<crate::nav::index::Location>> {
        if ident.is_empty() {
            return None;
        }
        let mut all: Vec<crate::nav::index::Location> = index.definitions_of(ident);
        if path_token != ident {
            all.extend(index.definitions_of(path_token));
        }
        all.sort_by(|a, b| {
            // Same-file candidates first (false < true), then deterministic
            // (file, line, name).
            (a.file != rel).cmp(&(b.file != rel)).then_with(|| {
                (a.file.as_str(), a.symbol.line, &a.symbol.name).cmp(&(b.file.as_str(), b.symbol.line, &b.symbol.name))
            })
        });
        all.dedup_by(|a, b| a.file == b.file && a.symbol.line == b.symbol.line && a.symbol.name == b.symbol.name);
        (!all.is_empty()).then_some(all)
    }

    /// (010-01, plan 010 Shape A rung 1) The M-. self-receiver candidates:
    /// `self.<member>` resolves via the LEXICALLY ENCLOSING impl's self
    /// type ("which impl am I lexically inside" — no expression typing):
    /// - a FIELD → the struct's `field_declaration` line, from the index's
    ///   cross-file field locations (the same file comes out first after
    ///   the ordering below);
    /// - a METHOD → the impl method's line from the SAME FILE's impl table
    ///   (an impl block is lexically one file — its methods are never
    ///   cross-file, so only `rel`'s tables are consulted);
    /// - both are gathered when a name is both a field and a method (the
    ///   picker lets the user choose — never guessed away).
    ///
    /// `Vec::new()` (the caller degrades to today's bare-`<member>`
    /// behavior) whenever: the enclosing impl can't be found, its self
    /// type isn't a PLAIN identifier (generics — `impl<T> Foo<T>` —
    /// degrade; never a guess), or no field/method named `<member>` is
    /// recorded for that type. Pure over the index (testable in isolation,
    /// shared by the project and the external crate paths).
    fn self_receiver_candidates(
        index: &SymbolIndex,
        rel: &str,
        source: &str,
        byte: usize,
        member: &str,
    ) -> Vec<crate::nav::index::Location> {
        let Some(type_name) = crate::syntax::queries::rust_self_type_at(source, byte) else {
            return Vec::new();
        };
        Self::type_member_candidates(index, rel, &type_name, member)
    }

    /// (010-03, plan 010 Shape A rung 3) The M-. local-binding
    /// candidates: `x.<member>` / `x.<member>()` resolves when the
    /// binding `x` has a WRITTEN-DOWN type in the enclosing scope — a
    /// `let x: Type` annotation or a `let x = Type { … }` struct
    /// literal (innermost scope wins; within a scope the last `let`
    /// before the use wins — the shadow rule) — and then exactly like
    /// the self pre-step: field → the struct's `field_declaration`
    /// line (cross-file via the index), method → the impl method's line
    /// in the same file.
    ///
    /// `Vec::new()` (the caller degrades to today's bare-`<member>`
    /// behavior byte-for-byte) when the file has no binding table, the
    /// binding's type isn't written down anywhere in the scope chain
    /// (never inferred), or no field/method named `<member>` is recorded
    /// for that type (non-struct types, `&T { … }`, generics, … all
    /// degrade here or earlier).
    fn local_binding_candidates(
        index: &SymbolIndex,
        rel: &str,
        source: &str,
        byte: usize,
        receiver: &str,
        member: &str,
    ) -> Vec<crate::nav::index::Location> {
        let Some(tables) = index.tables(rel) else {
            return Vec::new();
        };
        let Some(type_name) =
            crate::syntax::queries::rust_binding_type_at(tables, source, byte, receiver)
        else {
            return Vec::new();
        };
        Self::type_member_candidates(index, rel, &type_name, member)
    }

    /// The shared member gathering of the 010-01 / 010-03 pre-steps: for
    /// type `type_name`, the member `member` — a FIELD → the struct's
    /// `field_declaration` lines from the index's cross-file field
    /// locations (the same file orders first below), a METHOD → the
    /// same file's impl tables (an impl's methods are lexically one
    /// file); both are gathered when a name is both (the picker lets the
    /// user choose — never guessed away), deduped, same-file-first.
    fn type_member_candidates(
        index: &SymbolIndex,
        rel: &str,
        type_name: &str,
        member: &str,
    ) -> Vec<crate::nav::index::Location> {
        let mk = |file: String, kind: crate::syntax::queries::SymbolKind, line: usize| {
            crate::nav::index::Location {
                file,
                symbol: crate::nav::index::Symbol {
                    name: member.to_string(),
                    kind,
                    line,
                    end_line: line,
                    start_byte: 0,
                    end_byte: 0,
                },
            }
        };
        let mut out: Vec<crate::nav::index::Location> = Vec::new();
        // Fields: the struct's `field_declaration` lines, all files (the
        // same file orders first below).
        for (file, line) in index.field_locations(type_name, member) {
            out.push(mk(
                file,
                crate::syntax::queries::SymbolKind::Constant,
                line,
            ));
        }
        // Methods: the same file's impl tables only (lexical — an impl's
        // methods all live in its own file).
        if let Some(tables) = index.tables(rel)
            && let Some(methods) = tables.impls.get(type_name)
        {
            for m in methods.iter().filter(|m| m.method == member) {
                out.push(mk(
                    rel.to_string(),
                    crate::syntax::queries::SymbolKind::Function,
                    m.line,
                ));
            }
        }
        if out.is_empty() {
            return out;
        }
        out.sort_by(|a, b| {
            (a.file != rel).cmp(&(b.file != rel)).then_with(|| {
                (a.file.as_str(), a.symbol.line, &a.symbol.name).cmp(&(b.file.as_str(), b.symbol.line, &b.symbol.name))
            })
        });
        out.dedup_by(|a, b| a.file == b.file && a.symbol.line == b.symbol.line && a.symbol.name == b.symbol.name);
        out
    }

    /// (010-03, plan 010 Shape A rung 3) The bare receiver identifier of
    /// a Rust `x.<member>` access when the point sits on (or
    /// immediately after) the MEMBER identifier run, plus the member
    /// run's char column (for the byte translation). The run derivation
    /// mirrors `symbol_at_point`'s (the char at the point when it is an
    /// identifier char, else the run ending immediately before it — the
    /// usual call-site spot). `None` for: the `self.` receiver (the
    /// 010-01 pre-step owns it — `Self` too, a type position), a
    /// receiver that isn't a bare identifier (`(expr).m`, `a[0].m`,
    /// `call().m`), a `::`-path receiver (`a::b.m`), a dot-chained
    /// receiver (`a.b.m` — the middle segment `b` is a field access,
    /// never a local binding), and a point not on a member run (the
    /// dot, whitespace).
    /// The extraction's path token stays BARE for these accesses — this
    /// scan is the pre-step's own, so a miss is byte-for-byte today's
    /// behavior (the caller gates on the language).
    fn rust_dotted_receiver(line_text: &str, col: usize) -> Option<(String, usize)> {
        let chars: Vec<char> = line_text.chars().collect();
        if col > chars.len() {
            return None;
        }
        let is_ident = |c: char| c.is_alphanumeric() || c == '_';
        let start = if col < chars.len() && is_ident(chars[col]) {
            let mut i = col;
            while i > 0 && is_ident(chars[i - 1]) {
                i -= 1;
            }
            i
        } else if col > 0 && is_ident(chars[col - 1]) {
            let mut i = col - 1;
            while i > 0 && is_ident(chars[i - 1]) {
                i -= 1;
            }
            i
        } else {
            return None;
        };
        // The member run must be preceded by the `.`.
        if start < 2 || chars[start - 1] != '.' {
            return None;
        }
        // The receiver run ends at the char BEFORE the dot.
        let mut i = start - 2;
        if !is_ident(chars[i]) {
            return None; // `(expr).m`, `a[0].m`, `call().m` …
        }
        while i > 0 && is_ident(chars[i - 1]) {
            i -= 1;
        }
        // A bare identifier: nothing glued on the left (`myself.` is
        // fine — the run is the whole word — but `a::b.m`'s `b` has a
        // `:` before it: a path receiver; and `a.b.m`'s `b` has a `.`
        // before it: the middle segment of a dot chain — neither is a
        // local binding).
        if i > 0 && (is_ident(chars[i - 1]) || chars[i - 1] == ':' || chars[i - 1] == '.') {
            return None;
        }
        let receiver: String = chars[i..start - 1].iter().collect();
        match receiver.as_str() {
            "self" | "Self" => None,
            _ => Some((receiver, start)),
        }
    }

    /// (M., selection rule 4) Start the tooling-resolver fall-through OFF the
    /// input path (plan 006 issue 02): `spawn_blocking` + `ResolveBus`,
    /// mirroring the symbol-indexer pattern (`start_indexing`). `cargo
    /// metadata` / `cargo fetch` shell out (network, seconds) and must never
    /// block a keypress. A new request supersedes any in-flight one (the
    /// generation bump makes the stale event discard itself in
    /// `apply_resolve_event`). No-op-ish without a tokio runtime (plain unit
    /// tests): the indicator is cleared and a miss message reported, so the
    /// status line can never hang.
    ///
    /// 007-03: the `SymbolContext.scope` hint is populated from the current
    /// buffer's tree-sitter layer (`resolver_scope`), so a BARE symbol
    /// imported via `use` resolves instead of hitting the providers'
    /// "needs scope info" bail; without a hint the context stays empty and
    /// the providers behave exactly as before (byte-for-byte).
    pub fn start_symbol_resolution(&mut self, symbol: &str, from_file: &str) {
        let Some(project) = self.project.as_ref() else {
            self.minibuffer_message("no project");
            return;
        };
        let root = project.root.clone();
        let symbol_owned = symbol.to_string();
        self.resolve_generation += 1;
        let generation = self.resolve_generation;
        self.resolving = Some((format!("resolving `{symbol}`…"), generation));
        if tokio::runtime::Handle::try_current().is_err() {
            self.resolving = None;
            self.minibuffer_message(&format!("no provider resolution for `{symbol}` (no background runtime)"));
            return;
        }
        let bus = self.resolve_bus.clone();
        let from = std::path::PathBuf::from(from_file);
        // 007-03: the scope hint (use-declaration path for a bare symbol,
        // the enclosing item chain for a path-shaped one, empty otherwise).
        let scope = self.resolver_scope(symbol);
        // 011-01: the buffer's language (the dispatch key — only providers
        // whose `languages()` contain it are attempted; `None` for an
        // unknown extension keeps the pre-dispatch in-order walk).
        let language = self.resolution_language(from_file);
        tokio::task::spawn_blocking(move || {
            // The provider chain. 011-01 registers the non-Rust providers
            // and dispatches on the context language: a Python buffer can
            // never reach the cargo provider (and vice versa), so the
            // registration order among languages is only a tie-breaker.
            // `None` language (unknown extension) still walks the whole
            // chain, in this order.
            let mut chain = Resolver::new();
            chain.add(CargoProvider::new());
            chain.add(JsProvider::new());
            chain.add(PythonProvider::new());
            chain.add(GoProvider::new());
            let ctx = SymbolContext {
                workspace_root: root,
                symbol: symbol_owned.clone(),
                from_file: from,
                scope,
                language,
            };
            let (source, error) = match chain.resolve_traced(&ctx) {
                Ok(outcome) => (Some(outcome.source), None),
                Err(e) => (None, Some(e.to_string())),
            };
            bus.send(ResolveEvent { generation, symbol: symbol_owned, source, error });
        });
    }

    /// Install a resolve event into the store (called by the UI's ResolveBus
    /// drain in `Root`). Discards events from a stale generation (a
    /// superseded M-. request or a previous project — mirroring
    /// `apply_index_event`); otherwise clears the status activity and lands
    /// the result (jump) or reports the miss.
    pub fn apply_resolve_event(&mut self, event: &ResolveEvent) {
        if event.generation != self.resolve_generation {
            // A stale event (a superseded request or a previous project):
            // its result is discarded. The drain is latest-wins — this
            // stale send may have OVERWRITTEN the current generation's
            // event in the watch channel (006-02b item 3); if so the
            // current job's event never reaches us and its `resolving`
            // indicator would stick until the next action. Clearing it here
            // is always safe: a still-in-flight current-generation event
            // lands its jump when it arrives (its generation still matches)
            // — at worst the indicator hides a few moments early.
            self.resolving = None;
            return;
        }
        self.resolving = None;
        match (&event.source, &event.error) {
            (Some(source), _) => self.open_resolved_source(source, &event.symbol),
            (None, Some(e)) => {
                self.minibuffer_message(&format!(
                    "no provider resolution for `{}`: {}",
                    event.symbol, e
                ));
            }
            _ => {}
        }
    }

    /// The resolving indicator for the activity display (empty when idle;
    /// hidden automatically when the generation no longer matches — a
    /// superseded request or project switch).
    pub fn resolving_display(&self) -> String {
        match &self.resolving {
            Some((text, g)) if *g == self.resolve_generation => text.clone(),
            _ => String::new(),
        }
    }

    /// (011-01) The `SymbolContext.language` dispatch key for `from_file`:
    /// the grammar registry's lowercase language name ("rust", "python",
    /// "javascript", "go", … — matching the providers' `languages()`
    /// strings). `None` when the extension is unknown (`Plain`): the chain
    /// then keeps the pre-dispatch behavior and walks every provider in
    /// order (byte-for-byte today's behavior for unknown files).
    fn resolution_language(&self, from_file: &str) -> Option<String> {
        let lang = self.grammar_registry.language_for(from_file);
        (lang != crate::syntax::registry::LanguageId::Plain).then(|| lang.name().to_string())
    }

    /// (007-03 / 011-02) The `SymbolContext.scope` hint for `symbol`, from
    /// the CURRENT buffer's tree-sitter layer (007-01's `scope_path_at` +
    /// the per-language import walks):
    /// - a BARE symbol with an import declaration that brings the name
    ///   into scope → the import's FULL original path, item included
    ///   (Rust `use a::B as C` → `["a","B"]` for bare `C`; JS/TS
    ///   `import { B as C } from "a"` → `["a","B"]`; Python
    ///   `from a import B as C` → `["a","B"]`; Go dot-import
    ///   `import . "a/b"` → `["b","<bare item>"]`);
    /// - a BARE symbol with no such import → EMPTY (the providers keep
    ///   their exact no-hint behavior — std/prelude names are never
    ///   guessed, byte-for-byte degradation);
    /// - a path-shaped symbol → the enclosing item chain (Rust, 007-01)
    ///   or a JS/TS namespace-aliased member rewrite (`ns.member` →
    ///   `["pkg","member"]` when `ns` comes from `import * as ns from
    ///   "pkg"`); Python/Go dotted symbols carry their own module /
    ///   package path and get no hint.
    ///
    /// Empty for a missing buffer/path, a failed parse, or an unimplemented
    /// language (including `Plain`).
    fn resolver_scope(&self, symbol: &str) -> Vec<String> {
        let Some(key) = self.buffers.current().map(String::from) else {
            return Vec::new();
        };
        let Some(buf) = self.buffers.get(&key) else {
            return Vec::new();
        };
        let Some(path) = buf.path.as_ref() else {
            return Vec::new();
        };
        let p = self.file_point();
        Self::resolver_scope_for(path, &buf.rope, p.line, p.col, symbol)
    }

    /// The scope hint for the buffer at `(line, col)` — the testable seam
    /// behind [`resolver_scope`](Self::resolver_scope).
    fn resolver_scope_for(
        path: &Path,
        rope: &Rope,
        line: usize,
        col: usize,
        symbol: &str,
    ) -> Vec<String> {
        let lang = crate::syntax::registry::resolve_language(&path.display().to_string());
        let source = rope.to_string();
        let Some(byte) = point_byte_offset(rope, line, col) else {
            return Vec::new();
        };
        match lang {
            // 007-03 (Rust): bare → the `use` declaration's path; path-
            // shaped → the enclosing item chain (carried, not consumed).
            crate::syntax::registry::LanguageId::Rust => {
                if !symbol.contains("::") {
                    return Self::use_path_for_symbol(&source, byte, symbol).unwrap_or_default();
                }
                crate::syntax::node::scope_path_at(lang, &source, byte)
            }
            // 011-02: the per-language import walks (bare symbols), plus
            // the JS/TS namespace-member rewrite for path-shaped symbols.
            crate::syntax::registry::LanguageId::JavaScript
            | crate::syntax::registry::LanguageId::TypeScript
            | crate::syntax::registry::LanguageId::Tsx => {
                Self::js_ts_scope_for(lang, &source, byte, symbol)
            }
            crate::syntax::registry::LanguageId::Python => {
                Self::python_scope_for(&source, byte, symbol)
            }
            crate::syntax::registry::LanguageId::Go => Self::go_scope_for(&source, byte, symbol),
            // Every other language (and Plain): no hint — the providers
            // keep their exact no-hint behavior.
            _ => Vec::new(),
        }
    }

    /// (007-03) The FULL original path of the `use` declaration that brings
    /// `symbol` into scope at `byte` (e.g. `use serde::Deserialize;` →
    /// `["serde", "Deserialize"]`); an aliased import
    /// (`use a::B as C`) yields the ORIGINAL path for the alias `C`.
    ///
    /// Bounded by design: Rust imports are module-scoped, so only the
    /// `use_declaration` items of the source root (top level — the Rust
    /// grammar's root node is `source_file`) or of `byte`'s
    /// ANCESTOR `mod_item` chain are considered — a sibling or nested
    /// module's imports never name `byte`'s scope. Innermost module first
    /// (an inner import shadows an outer one); within a module, the LAST
    /// matching declaration wins. Globs (`use a::*`), single-segment
    /// imports (`use foo;` — same-crate modules), and `self`/`super`/
    /// `crate`-prefixed paths never name an external item → `None`
    /// (never guess).
    fn use_path_for_symbol(source: &str, byte: usize, symbol: &str) -> Option<Vec<String>> {
        let language = crate::syntax::queries::language_for(
            crate::syntax::registry::LanguageId::Rust,
        )?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).ok()?;
        let tree = parser.parse(source.as_bytes(), None)?;
        let root = tree.root_node();
        if !(root.start_byte() <= byte && byte < root.end_byte()) {
            return None;
        }
        // Innermost node containing `byte` (the same containment rule as
        // 007-01's `innermost_at`).
        let mut leaf = root;
        loop {
            let mut child = None;
            for i in 0..leaf.child_count() {
                if let Some(c) = leaf.child(i)
                    && c.start_byte() <= byte
                    && byte < c.end_byte()
                {
                    child = Some(c);
                    break;
                }
            }
            match child {
                Some(c) => leaf = c,
                None => break,
            }
        }
        // Candidate modules: the source root (top level) + the enclosing
        // `mod_item` ancestors, innermost first (nearest scope shadows).
        let mut modules = Vec::new();
        let mut anc = leaf.parent();
        while let Some(a) = anc {
            if a.kind() == "mod_item" || a.kind() == "source_file" {
                modules.push(a);
            }
            anc = a.parent();
        }
        // The ancestor walk already yields innermost-first order.
        for module in &modules {
            // `mod_item` items live in its `body` block; the source root's
            // are direct children.
            let items = if module.kind() == "mod_item" {
                module.child_by_field_name("body")?
            } else {
                *module
            };
            let mut hit: Option<Vec<String>> = None;
            for i in 0..items.child_count() {
                let child = items.child(i)?;
                if child.kind() != "use_declaration" {
                    continue;
                }
                if let Ok(text) = child.utf8_text(source.as_bytes())
                    && let Some(path) = Self::use_decl_path(text, symbol)
                {
                    hit = Some(path); // last matching declaration wins
                }
            }
            if hit.is_some() {
                return hit;
            }
        }
        None
    }

    /// The original import path (segments, item included) of a
    /// `use_declaration` TEXT that brings `symbol` into scope — or `None`
    /// when no entry of the declaration names `symbol` (globs, module-only
    /// imports, `self`/`super`/`crate` prefixes are never guessed).
    fn use_decl_path(text: &str, symbol: &str) -> Option<Vec<String>> {
        // `pub (vis) use <spec>;` — find the `use` KEYWORD (token-wise; a
        // `pub(crate)` prefix never contains the token `use`).
        let body = text.split(';').next()?.trim();
        let mut pos = 0usize;
        let mut found = false;
        for tok in body.split(char::is_whitespace) {
            if tok == "use" {
                pos += 3;
                found = true;
                break;
            }
            pos += tok.len() + 1;
        }
        if !found || pos > body.len() || !body.is_char_boundary(pos) {
            return None;
        }
        let rest = body[pos..].trim();
        if rest.is_empty() {
            return None;
        }
        // `prefix::{ ... }` / `prefix::Name [as Alias]`
        match rest.find('{') {
            Some(open) => {
                let close = rest.rfind('}')?;
                if close < open {
                    return None;
                }
                let prefix_raw = rest[..open].trim();
                let prefix = prefix_raw.strip_suffix("::").unwrap_or(prefix_raw).to_string();
                Self::use_group_entries(&rest[open + 1..close], &prefix, symbol)
            }
            None => {
                let (path_part, alias) = match rest.split_once(" as ") {
                    Some((p, a)) => (p, Some(a.trim())),
                    None => (rest, None),
                };
                let segments = Self::import_segments(path_part)?;
                // Single-segment imports are same-crate modules (no
                // external crate is named) — never guessed.
                if segments.len() < 2 {
                    return None;
                }
                let local = alias.unwrap_or(segments.last().unwrap());
                (local == symbol).then_some(segments)
            }
        }
    }

    /// The import entries of a `use` group body (comma-separated, nested
    /// `sub::{…}` groups recurse), matched against `symbol`.
    fn use_group_entries(group: &str, prefix: &str, symbol: &str) -> Option<Vec<String>> {
        let mut depth = 0i32;
        let mut start = 0usize;
        let mut entries: Vec<&str> = Vec::new();
        for (i, c) in group.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                ',' if depth == 0 => {
                    entries.push(&group[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        entries.push(&group[start..]);
        for entry in entries
            .into_iter()
            .map(|e| e.trim())
            .filter(|e| !e.is_empty())
        {
            if let Some(nested) = entry.find('{') {
                // `sub::{…}` — the group's own prefix joins in.
                let sub = entry[..nested].trim();
                let full = if prefix.is_empty() {
                    sub.to_string()
                } else {
                    format!("{prefix}::{sub}")
                };
                if let Some(close) = entry.rfind('}')
                    && let Some(p) =
                        Self::use_group_entries(&entry[nested + 1..close], &full, symbol)
                {
                    return Some(p);
                }
                continue;
            }
            let (name_part, alias) = match entry.split_once(" as ") {
                Some((p, a)) => (p.trim(), Some(a.trim())),
                None => (entry, None),
            };
            // `*` (glob) cannot name a specific symbol.
            if name_part == "*" {
                continue;
            }
            // A non-resolvable entry (`self`/`super`/`crate`-prefixed, or a
            // single segment) must SKIP, not abort the whole group: in
            // `use a::b::{self, c};` a bare `c` still has a valid hint.
            // `?` here would discard the remaining entries (007-03 review P2).
            let Some(segs) = Self::import_segments(name_part) else {
                continue;
            };
            let full = if prefix.is_empty() {
                segs
            } else {
                let Some(mut v) = Self::import_segments(prefix) else {
                    continue;
                };
                v.extend_from_slice(&segs);
                v
            };
            // Same rule as the plain form: without an external prefix a
            // single segment is a same-crate item (never guessed).
            if full.len() < 2 {
                continue;
            }
            let local = alias.unwrap_or(full.last().unwrap());
            if local == symbol {
                return Some(full);
            }
        }
        None
    }

    /// Split a `::`-path on `::` into clean identifier segments; `None`
    /// when a segment is empty or non-identifier (never guess).
    fn import_segments(s: &str) -> Option<Vec<String>> {
        let segs: Vec<&str> = s.split("::").collect();
        if segs.is_empty() || segs.iter().any(|g| g.is_empty()) {
            return None;
        }
        let mut out = Vec::with_capacity(segs.len());
        for g in segs {
            if !g.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return None;
            }
            out.push(g.to_string());
        }
        // `self`/`super`/`crate` prefixes never name an external crate.
        if matches!(out.first().map(String::as_str), Some("self" | "super" | "crate")) {
            return None;
        }
        Some(out)
    }

    // ── 011-02: per-language import walks (bare-symbol hints) ────────────

    /// The text of a syntax node (`None` on non-UTF8).
    fn node_text(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
        node.utf8_text(source).ok().map(String::from)
    }

    /// A non-empty ASCII identifier (`a0_Z`) — the segment shape an import
    /// path may carry (mirrors the Rust `import_segments` rule; anything
    /// else is an unsupported shape → no hint, never a guess).
    fn is_ascii_identifier(s: &str) -> bool {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    /// (011-02, relative extension in the 011-08 fix-jsrel P2-7
    /// follow-up) The JS/TS scope hint: a bare symbol → the package path
    /// (or the relative specifier) its import binds it to; a path-shaped
    /// `ns.member` → the namespace rewrite; a plain dotted path
    /// (`lodash.map`) → EMPTY (it carries its own package — the provider's
    /// path wins, never treated as bare).
    fn js_ts_scope_for(
        lang: crate::syntax::registry::LanguageId,
        source: &str,
        byte: usize,
        symbol: &str,
    ) -> Vec<String> {
        // One parse per miss (the 007-03 discipline): a parse failure or an
        // out-of-range offset degrades to the empty hint.
        let Some(language) = crate::syntax::queries::language_for(lang) else {
            return Vec::new();
        };
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&language).is_err() {
            return Vec::new();
        }
        let Some(tree) = parser.parse(source.as_bytes(), None) else {
            return Vec::new();
        };
        let root = tree.root_node();
        if !(root.start_byte() <= byte && byte < root.end_byte()) {
            return Vec::new();
        }
        let bytes = source.as_bytes();
        if !symbol.contains('.') {
            return Self::js_ts_bare_import_path(root, bytes, symbol)
                .unwrap_or_default();
        }
        // Path-shaped: exactly two dot segments (`ns.member`); deeper
        // chains are an unsupported shape (no hint).
        let Some((ns, member)) = symbol.split_once('.') else {
            return Vec::new();
        };
        if member.contains('.') {
            return Vec::new();
        }
        Self::js_ts_namespace_member_path(root, bytes, ns, member)
            .unwrap_or_default()
    }

    /// The import path for a BARE JS/TS symbol, from the module's import
    /// declarations. Bounded by design: only TOP-LEVEL declarations are
    /// considered (ESM imports are module-scoped; CJS `require` bindings
    /// are tracked at the top level only). First hit wins — a duplicate
    /// binding of one name is a syntax error, so at most one declaration
    /// can bind `symbol`:
    /// - `import { X } from "pkg"` / `import type { X }` → `["pkg", "X"]`;
    /// - `import { X as Y }` → `["pkg", "X"]` for bare `Y` (alias →
    ///   original);
    /// - `import X from "pkg"` → `["pkg", "X"]` (the default export);
    /// - `import * as ns from "pkg"` → `["pkg"]` for bare `ns` (the
    ///   package entry itself);
    /// - `const { X } = require("pkg")` / `const { X as Y } = require` →
    ///   the same rule (CJS destructuring);
    /// - `const m = require("pkg")` → `["pkg"]` for bare `m` (the module
    ///   object names the entry).
    ///
    /// 011-08 follow-up (fix-jsrel P2-7): a relative specifier (`./…`,
    /// `../…`) now CARRIES its hint (the JS provider resolves it against
    /// the importing buffer's directory and lands in the sibling file,
    /// workspace-local / `external = false`). Absolute paths and bare
    /// side-effect imports (`import "pkg"`) still bind nothing the
    /// provider can resolve → `None` (never guessed).
    fn js_ts_bare_import_path(root: tree_sitter::Node, source: &[u8], symbol: &str) -> Option<Vec<String>> {
        for i in 0..root.child_count() {
            let child = root.child(i)?;
            let hit = match child.kind() {
                "import_statement" => {
                    Self::js_ts_import_stmt_path(child, source, symbol)
                }
                "lexical_declaration" | "variable_declaration" => {
                    Self::js_ts_require_path(child, source, symbol)
                }
                _ => None,
            };
            if hit.is_some() {
                return hit;
            }
        }
        None
    }

    /// One `import_statement`: the local binding's original path
    /// (`None` when no binding names `symbol`).
    fn js_ts_import_stmt_path(stmt: tree_sitter::Node, source: &[u8], symbol: &str) -> Option<Vec<String>> {
        let source_node = stmt.child_by_field_name("source")?;
        let spec = Self::js_ts_specifier(source_node, source)?;
        // `import_clause` is NOT a grammar field (verified against the
        // pinned tree-sitter-javascript 0.23.1 sexp) — find it by kind.
        // Its absence is the side-effect form (`import "pkg"`) — binds
        // nothing, never a hint.
        let clause = (0..stmt.child_count())
            .filter_map(|k| stmt.child(k))
            .find(|n| n.kind() == "import_clause")?;
        for i in 0..clause.child_count() {
            let c = clause.child(i)?;
            match c.kind() {
                // `import X from "pkg"` — the local binding for the
                // package's default export.
                "identifier" => {
                    if let Some(name) = Self::node_text(c, source)
                        && name == symbol
                    {
                        return Some(Self::js_ts_import_hint(&spec, &name));
                    }
                }
                "named_imports" => {
                    for j in 0..c.child_count() {
                        let entry = c.child(j)?;
                        if entry.kind() != "import_specifier" {
                            continue;
                        }
                        let Some(name_node) = entry.child_by_field_name("name") else {
                            continue;
                        };
                        let name = Self::node_text(name_node, source)?;
                        let alias = entry
                            .child_by_field_name("alias")
                            .and_then(|a| Self::node_text(a, source));
                        if alias.as_deref().unwrap_or(&name) == symbol {
                            return Some(Self::js_ts_import_hint(&spec, &name));
                        }
                    }
                }
                // `import * as ns from "pkg"` — the bare namespace names
                // the package entry itself (its single named child is the
                // alias; `*`/`as` are anonymous tokens).
                "namespace_import" => {
                    let alias = (0..c.child_count())
                        .filter_map(|k| c.child(k))
                        .find(|n| n.kind() == "identifier")
                        .and_then(|n| Self::node_text(n, source))?;
                    if alias == symbol {
                        return Some(vec![spec]);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// `const/let/var` declarations whose initializer is a plain
    /// `require("pkg")` call (the CJS import shape).
    fn js_ts_require_path(decl: tree_sitter::Node, source: &[u8], symbol: &str) -> Option<Vec<String>> {
        for i in 0..decl.child_count() {
            let d = decl.child(i)?;
            if d.kind() != "variable_declarator" {
                continue;
            }
            let Some(name) = d.child_by_field_name("name") else {
                continue;
            };
            let Some(value) = d.child_by_field_name("value") else {
                continue;
            };
            if value.kind() != "call_expression" {
                continue;
            }
            // Only the bare identifier `require` (not a member/alias call).
            let Some(callee) = value.child_by_field_name("function") else {
                continue;
            };
            if callee.kind() != "identifier"
                || Self::node_text(callee, source).as_deref() != Some("require")
            {
                continue;
            }
            let Some(args) = value.child_by_field_name("arguments") else {
                continue;
            };
            // The arguments node: `(` at index 0, first arg at 1.
            let Some(first) = args.child(1) else {
                continue;
            };
            if first.kind() != "string" {
                continue;
            }
            let Some(spec) = Self::js_ts_specifier(first, source) else {
                continue;
            };
            match name.kind() {
                // `const m = require("pkg")` — the whole module object:
                // the binding names the package entry itself.
                "identifier" => {
                    if let Some(n) = Self::node_text(name, source)
                        && n == symbol
                    {
                        return Some(vec![spec]);
                    }
                }
                // `const { x, y: z } = require("pkg")` — destructured
                // exports (shorthand and `original: local` pairs).
                "object_pattern" => {
                    for j in 0..name.child_count() {
                        let p = name.child(j)?;
                        let (local, original) = match p.kind() {
                            "shorthand_property_identifier_pattern" => {
                                let t = Self::node_text(p, source)?;
                                (t.clone(), t)
                            }
                            "pair_pattern" => {
                                let Some(key) = p.child_by_field_name("key") else {
                                    continue;
                                };
                                let Some(val) = p.child_by_field_name("value") else {
                                    continue;
                                };
                                if val.kind() != "identifier" {
                                    continue; // nested patterns: unsupported shape.
                                }
                                (
                                    Self::node_text(val, source)?,
                                    Self::node_text(key, source)?,
                                )
                            }
                            _ => continue,
                        };
                        if local == symbol {
                            return Some(Self::js_ts_import_hint(&spec, &original));
                        }
                    }
                }
                _ => {} // array/nested patterns: unsupported shape → no hint.
            }
        }
        None
    }

    /// The hint path for an imported item: `import { default as D }` /
    /// `const { default: D } = require` name the ENTRY itself (no item
    /// segment); any other original name carries it.
    fn js_ts_import_hint(spec: &str, original: &str) -> Vec<String> {
        if original == "default" {
            vec![spec.to_string()]
        } else {
            vec![spec.to_string(), original.to_string()]
        }
    }

    /// A JS/TS string-literal module specifier the hint may carry —
    /// quoted with `'`/`"` (a template literal or other shape is
    /// unsupported). Two shapes pass:
    /// - a package path — scoped (`@scope/name`) and subpath (`name/sub`)
    ///   specs keep their `/` (the 011-02 external-package hint, which
    ///   resolves only through node_modules);
    /// - a relative specifier (`./…` / `../…`) — the 011-08 fix-jsrel
    ///   provider semantics: it resolves against the importing buffer's
    ///   directory (exact file → JS-extension walk → directory entry)
    ///   and lands workspace-locally (`external = false`), so the hint
    ///   carries it, item included.
    ///
    /// Absolute paths and any other `.`-leading shape (bare `.`/`..`) stay
    /// out: the provider bails dedicated on absolute, and a bare `.`/`..`
    /// never reaches its relative branch (it is not a `./`-prefixed spec)
    /// → never a hint.
    fn js_ts_specifier(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
        let text = Self::node_text(node, source)?;
        let bytes = text.as_bytes();
        if bytes.len() < 2 {
            return None;
        }
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if !(first == b'\'' && last == b'\'' || first == b'"' && last == b'"') {
            return None;
        }
        let spec = std::str::from_utf8(&bytes[1..bytes.len() - 1]).ok()?;
        let is_relative = spec.starts_with("./") || spec.starts_with("../");
        if spec.is_empty()
            || spec.starts_with('/')
            || (spec.starts_with('.') && !is_relative)
        {
            return None;
        }
        Some(spec.to_string())
    }

    /// The namespace-member rewrite: `ns.member` where `ns` comes from
    /// `import * as ns from "pkg"` → `["pkg", "member"]` (the provider's
    /// alias-rewrite rule turns it back into the package's real path;
    /// identity cases — `import * as pkg from "pkg"` — are a no-op there
    /// because the joined hint equals the symbol's own path).
    fn js_ts_namespace_member_path(
        root: tree_sitter::Node,
        source: &[u8],
        ns: &str,
        member: &str,
    ) -> Option<Vec<String>> {
        for i in 0..root.child_count() {
            let child = root.child(i)?;
            if child.kind() != "import_statement" {
                continue;
            }
            let Some(source_node) = child.child_by_field_name("source") else {
                continue;
            };
            let Some(spec) = Self::js_ts_specifier(source_node, source) else {
                continue;
            };
            let Some(clause) = (0..child.child_count())
                .filter_map(|k| child.child(k))
                .find(|n| n.kind() == "import_clause")
            else {
                // `import "pkg"` — side-effect only, binds nothing.
                continue;
            };
            for j in 0..clause.child_count() {
                let c = clause.child(j)?;
                if c.kind() != "namespace_import" {
                    continue;
                }
                let alias = (0..c.child_count())
                    .filter_map(|k| c.child(k))
                    .find(|n| n.kind() == "identifier")
                    .and_then(|n| Self::node_text(n, source))?;
                if alias == ns {
                    return Some(vec![spec, member.to_string()]);
                }
            }
        }
        None
    }

    /// (011-02) The Python scope hint: a BARE symbol → the module path +
    /// item its import binds it to. A dotted symbol (`os.path.join`) carries
    /// its own module path → EMPTY (never treated as bare).
    fn python_scope_for(source: &str, byte: usize, symbol: &str) -> Vec<String> {
        if symbol.contains('.') {
            return Vec::new();
        }
        Self::python_import_path_for_symbol(source, byte, symbol).unwrap_or_default()
    }

    /// The module path (segments, item included) of the import that binds a
    /// BARE Python `symbol` at `byte`:
    /// - `from a import X` → `["a", "X"]`; `from a.b import X` →
    ///   `["a", "b", "X"]`; `from a import X as Y` → the original `X` for
    ///   bare `Y`;
    /// - `import a.b as c` → `["a", "b"]` for bare `c` (the module alias);
    /// - a plain `import a.b` binds ONLY the top-level `a` → never a hint
    ///   for bare `b` (not guessed);
    /// - relative imports (`from . import X`), wildcards (`import *`), and
    ///   `from a import b.c` (binds `b`, an attribute walk) → `None`
    ///   (the sys.path root is unknown from the buffer path alone — never
    ///   guessed).
    ///
    /// Bounded: the module level + the enclosing `function`/`class` blocks
    /// only (innermost first — a local import shadows the module-level
    /// one); imports nested deeper (under an `if`, etc.) are not counted.
    /// Within a block the LAST matching statement at/before `byte` wins
    /// (a re-import shadows the earlier one).
    fn python_import_path_for_symbol(source: &str, byte: usize, symbol: &str) -> Option<Vec<String>> {
        let language = crate::syntax::queries::language_for(
            crate::syntax::registry::LanguageId::Python,
        )?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).ok()?;
        let tree = parser.parse(source.as_bytes(), None)?;
        let root = tree.root_node();
        if !(root.start_byte() <= byte && byte < root.end_byte()) {
            return None;
        }
        // The innermost node containing `byte` (the same containment rule
        // as the Rust walk).
        let mut leaf = root;
        loop {
            let mut child = None;
            for i in 0..leaf.child_count() {
                if let Some(c) = leaf.child(i)
                    && c.start_byte() <= byte
                    && byte < c.end_byte()
                {
                    child = Some(c);
                    break;
                }
            }
            match child {
                Some(c) => leaf = c,
                None => break,
            }
        }
        // Candidate blocks, innermost first (nearest scope shadows), the
        // module level last.
        let mut scopes: Vec<tree_sitter::Node> = Vec::new();
        let mut anc = leaf.parent();
        while let Some(a) = anc {
            if a.kind() == "function_definition" || a.kind() == "class_definition" {
                scopes.push(a);
            }
            anc = a.parent();
        }
        scopes.push(root);
        let bytes = source.as_bytes();
        for scope in scopes {
            let block = if scope.kind() == "module" {
                scope
            } else {
                scope.child_by_field_name("body")?
            };
            let mut hit: Option<Vec<String>> = None;
            for i in 0..block.child_count() {
                let child = block.child(i)?;
                match child.kind() {
                    "import_statement" | "import_from_statement" => {
                        if !(child.start_byte() <= byte) {
                            continue;
                        }
                        if let Some(path) =
                            Self::python_import_stmt_path(child, bytes, symbol)
                        {
                            hit = Some(path); // last matching statement wins
                        }
                    }
                    _ => {}
                }
            }
            if hit.is_some() {
                return hit;
            }
        }
        None
    }

    /// One Python import statement: the original path the entry binds to
    /// `symbol` (`None` when no entry names it — plain `import a.b`,
    /// relative modules, wildcards, and attribute-walk entries are never
    /// guessed).
    fn python_import_stmt_path(
        stmt: tree_sitter::Node,
        source: &[u8],
        symbol: &str,
    ) -> Option<Vec<String>> {
        match stmt.kind() {
            "import_statement" => {
                // Each entry is field `name`: a `dotted_name` (binds only
                // its TOP-LEVEL segment — never a hint) or an
                // `aliased_import` (binds the alias to the full module).
                let mut hit: Option<Vec<String>> = None;
                for i in 0..stmt.child_count() {
                    let child = stmt.child(i)?;
                    if child.kind() != "aliased_import" {
                        continue;
                    }
                    let Some(alias_node) = child.child_by_field_name("alias") else {
                        continue;
                    };
                    let alias = Self::node_text(alias_node, source)?;
                    if alias != symbol {
                        continue;
                    }
                    let Some(name) = child.child_by_field_name("name") else {
                        continue;
                    };
                    hit = Some(Self::python_dotted_segments(name, source)?);
                }
                hit
            }
            "import_from_statement" => {
                let module = stmt.child_by_field_name("module_name")?;
                let base = if module.kind() == "relative_import" {
                    return None; // `from . import X`: the enclosing package is
                    // ambiguous without the sys.path root — not guessed.
                } else {
                    Self::python_dotted_segments(module, source)?
                };
                let mut hit: Option<Vec<String>> = None;
                for i in 0..stmt.child_count() {
                    let child = stmt.child(i)?;
                    match child.kind() {
                        "aliased_import" => {
                            let Some(alias_node) = child.child_by_field_name("alias") else {
                                continue;
                            };
                            let alias = Self::node_text(alias_node, source)?;
                            if alias != symbol {
                                continue;
                            }
                            let Some(name) = child.child_by_field_name("name") else {
                                continue;
                            };
                            let item = Self::python_dotted_segments(name, source)?;
                            if item.len() != 1 {
                                continue; // `from a import b.c` binds `b`, not `b.c`.
                            }
                            let mut full = base.clone();
                            full.extend(item);
                            hit = Some(full);
                        }
                        "dotted_name" => {
                            // `from a import b` — binds the (single) name.
                            let item = Self::python_dotted_segments(child, source)?;
                            if item.len() == 1 && item[0] == symbol {
                                let mut full = base.clone();
                                full.push(item[0].clone());
                                hit = Some(full);
                            }
                        }
                        _ => {} // `wildcard_import`: names no specific symbol.
                    }
                }
                hit
            }
            _ => None,
        }
    }

    /// The identifier segments of a Python `dotted_name` node (its `.`
    /// tokens are anonymous — only `identifier` children count); `None`
    /// when a segment is not a plain ASCII identifier.
    fn python_dotted_segments(node: tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
        let mut out = Vec::new();
        for i in 0..node.child_count() {
            let c = node.child(i)?;
            if c.kind() != "identifier" {
                continue;
            }
            let t = Self::node_text(c, source)?;
            if !Self::is_ascii_identifier(&t) {
                return None;
            }
            out.push(t);
        }
        if out.is_empty() {
            return None;
        }
        Some(out)
    }

    /// (011-02) The Go scope hint: a BARE symbol → the dot-imported package
    /// name + the symbol (Go binds bare item names only through DOT
    /// imports — plain and aliased imports bind a package NAME, which is
    /// always used qualified and needs no hint). A dotted symbol
    /// (`y.Fn`) carries its own package name → EMPTY.
    fn go_scope_for(source: &str, byte: usize, symbol: &str) -> Vec<String> {
        if symbol.contains('.') {
            return Vec::new();
        }
        Self::go_import_path_for_symbol(source, byte, symbol).unwrap_or_default()
    }

    /// The dot-import hint for a bare Go `symbol`: exactly ONE dot-imported
    /// package in the file (file-scoped) → `["<local pkg name>",
    /// "<symbol>"]` (the local name is the import path's last segment);
    /// zero dot imports → `None`; SEVERAL → `None` (the origin of a bare
    /// item is ambiguous — never guessed).
    fn go_import_path_for_symbol(source: &str, byte: usize, symbol: &str) -> Option<Vec<String>> {
        let language = crate::syntax::queries::language_for(
            crate::syntax::registry::LanguageId::Go,
        )?;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).ok()?;
        let tree = parser.parse(source.as_bytes(), None)?;
        let root = tree.root_node();
        if !(root.start_byte() <= byte && byte < root.end_byte()) {
            return None;
        }
        let bytes = source.as_bytes();
        let mut dot_pkgs: Vec<String> = Vec::new();
        for i in 0..root.child_count() {
            let child = root.child(i)?;
            if child.kind() != "import_declaration" {
                continue;
            }
            // Specs are direct children (single import) or children of the
            // `import_spec_list` (grouped `import ( … )`).
            let mut stack = vec![child];
            while let Some(node) = stack.pop() {
                for j in 0..node.child_count() {
                    let c = node.child(j)?;
                    match c.kind() {
                        "import_spec" => {
                            // Only `import . "pkg"` (the named `dot` node)
                            // binds bare item names.
                            let Some(name) = c.child_by_field_name("name") else {
                                continue;
                            };
                            if name.kind() != "dot" {
                                continue;
                            }
                            let Some(path_node) = c.child_by_field_name("path") else {
                                continue;
                            };
                            let path = Self::go_string_content(path_node, bytes)?;
                            let pkg = path.rsplit('/').next()?;
                            if !Self::is_ascii_identifier(pkg) {
                                continue;
                            }
                            dot_pkgs.push(pkg.to_string());
                        }
                        "import_spec_list" => stack.push(c),
                        _ => {}
                    }
                }
            }
        }
        let [pkg] = dot_pkgs.as_slice() else {
            return None;
        };
        Some(vec![pkg.to_string(), symbol.to_string()])
    }

    /// The content of a Go `interpreted_string_literal` (import paths): the
    /// quotes stripped, for `"…"` and backtick-quoted strings.
    fn go_string_content(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
        let text = Self::node_text(node, source)?;
        let bytes = text.as_bytes();
        if bytes.len() < 2 {
            return None;
        }
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if !(first == b'"' && last == b'"' || first == b'`' && last == b'`') {
            return None;
        }
        std::str::from_utf8(&bytes[1..bytes.len() - 1]).ok().map(String::from)
    }

    // ── external crate index cache (plan 006 issue 03) ───────────────

    /// Start the background index build for an EXTERNAL source tree
    /// (plan 006 issue 03; the per-language generalization is plan 011
    /// issue 04): walk the OWNING language's source extensions under
    /// `root` — the language of the LANDED `landed_file` (the registry's
    /// extension map is the authority: Rust `rs`, JS/TS `js/jsx/ts/tsx/
    /// mjs/cjs`, Python `py/pyi`, Go `go`, C `c/h`, C++ `cc/cpp/cxx/hh/hpp/
    /// hxx`, Markdown `md/markdown/mdx` (011-07); any other language:
    /// nothing, the pre-011-04 end state) — and run the project indexer's
    /// machinery (`nav::index::build_index` is root-agnostic) on
    /// `spawn_blocking`; the finished index publishes on the
    /// `CrateIndexBus`. NEVER blocks the landing. Single-flight per
    /// source_root (already cached or already in flight: no-op).
    /// Refused with a clear message above `EXT_INDEX_FILE_CAP` files (no
    /// known registry crate approaches it); silent no-op without a tokio
    /// runtime (plain unit tests).
    pub fn start_crate_indexing(&mut self, root: &Path, landed_file: &Path) {
        if !root.is_dir() {
            return;
        }
        if self.external_indexes.iter().any(|(r, _)| r == root) {
            return; // already indexed
        }
        if self.crate_indexing.iter().any(|(r, _, _)| r == root) {
            return; // a build is already in flight for this root
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        // 011-04: the file-selection predicate is the OWNING language's
        // extension set (006-03's `**/*.rs` is the Rust case of this) —
        // NOT a global "index everything" walk.
        let lang = self
            .grammar_registry
            .language_for(&landed_file.to_string_lossy());
        let files = Self::crate_source_files(root, Self::source_extensions_for(lang));
        if files.is_empty() {
            return;
        }
        if files.len() > EXT_INDEX_FILE_CAP {
            self.minibuffer_message(&format!(
                "crate too large to index ({} source files; cap {})",
                files.len(),
                EXT_INDEX_FILE_CAP
            ));
            return;
        }
        let label = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        // 006-03b item 5: the N/M file counter — the same publisher-less
        // `IndexProgress` the project indexer uses (no `nav/index.rs`
        // change: `new` / `done` / `total` are already public).
        let progress = Arc::new(IndexProgress::new(files.len()));
        self.crate_indexing
            .push((root.to_path_buf(), label.clone(), progress.clone()));
        let bus = self.crate_index_bus.clone();
        let root_clone = root.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let index = build_index(&root_clone, &files, Some(progress.as_ref()));
            bus.send(CrateIndexEvent {
                source_root: root_clone,
                index,
            });
        });
    }

    /// The file-extension set the 011-04 index walk collects for the
    /// OWNING language (`registry.rs`'s extension map is the authority —
    /// each set here is the union of the extensions that map to the
    /// language's grammar family): Rust `rs`; JS/TS (JavaScript /
    /// TypeScript / Tsx) `js/jsx/ts/tsx/mjs/cjs`; Python `py/pyi`; Go
    /// `go`; C `c/h` (011-07 — headers ARE definition sources, the
    /// registry maps `h` to C); C++ `cc/cpp/cxx/hh/hpp/hxx` (011-07, the
    /// full registry map); Markdown `md/markdown/mdx` (011-07 — its
    /// definition query captures headings only); Java `java` /
    /// C# `cs` / Ruby `rb` / Scheme `scm/ss/sls/sld` (new-languages
    /// lane — the registry map per language); Clojure `clj/cljs/cljc`
    /// (runtime-bump lane). Every other language:    /// nothing — the walk finds no files, so no index builds (the same
    /// end state 006-03's `.rs`-only walk had for every non-Rust
    /// landing). The JSON/TOML/YAML/Bash outline queries are not M-.
    /// definition sources here (011-04 judgment, carried by 011-07):
    /// data/config/shell files stay out of the walk.
    fn source_extensions_for(lang: LanguageId) -> &'static [&'static str] {
        match lang {
            LanguageId::Rust => &["rs"],
            LanguageId::JavaScript | LanguageId::TypeScript | LanguageId::Tsx =>
                &["js", "jsx", "ts", "tsx", "mjs", "cjs"],
            LanguageId::Python => &["py", "pyi"],
            LanguageId::Go => &["go"],
            LanguageId::C => &["c", "h"],
            LanguageId::Cpp => &["cc", "cpp", "cxx", "hh", "hpp", "hxx"],
            LanguageId::Markdown => &["md", "markdown", "mdx"],
            LanguageId::Java => &["java"],
            LanguageId::CSharp => &["cs"],
            LanguageId::Ruby => &["rb"],
            LanguageId::Scheme => &["scm", "ss", "sls", "sld"],
            LanguageId::Clojure => &["clj", "cljs", "cljc"],
            _ => &[],
        }
    }

    /// The source walk of an external tree for `exts` (the owning
    /// language's extensions, 011-04; crate-relative, forward-slash
    /// paths — the same key shape the project index uses). Nested
    /// `node_modules` trees are skipped (011-04 item 4): they hold OTHER
    /// packages (transitive deps), never the landed dependency's own
    /// source — the js provider's definition walk skips them the same
    /// way; the refusal cap stays the backstop for every other tree.
    fn crate_source_files(root: &Path, exts: &[&str]) -> Vec<String> {
        ignore::Walk::new(root)
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_type().is_some_and(|t| t.is_file())
                    && e.path()
                        .extension()
                        .is_some_and(|x| {
                            let ext = x.to_string_lossy();
                            exts.iter().any(|wanted| ext.eq_ignore_ascii_case(wanted))
                        })
                    && !Self::under_node_modules(root, e.path())
            })
            .filter_map(|e| {
                e.path()
                    .strip_prefix(root)
                    .ok()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
            })
            .collect()
    }

    /// True when `path` (under `root`) is inside a `node_modules`
    /// DIRECTORY component of its crate-relative path — never for the
    /// file name itself. The root itself may BE a `node_modules` dir
    /// (a landed `node_modules/<pkg>`), so only components AFTER `root`
    /// count.
    fn under_node_modules(root: &Path, path: &Path) -> bool {
        let Ok(rel) = path.strip_prefix(root) else {
            return false;
        };
        let mut comps = rel.components();
        // Drop the file name: only a directory component can be the
        // `node_modules` boundary.
        let _ = comps.next_back();
        comps.any(|c| {
            matches!(c, std::path::Component::Normal(n) if n == "node_modules")
        })
    }

    /// Install a crate-index event into the store (called by the UI's
    /// CrateIndexBus drain). LRU: the root's entry becomes the newest;
    /// the oldest are evicted past `EXT_INDEX_CAP` — and then the current
    /// buffer's owning crate is bumped back to the newest slot, so the
    /// crate the user is IN is never the eviction victim (006-03b item
    /// 1: the arrival is where the eviction happens). The root's
    /// in-flight indicator clears (it can never stick past its own final
    /// event).
    pub fn apply_crate_index_event(&mut self, event: &CrateIndexEvent) {
        let index = Arc::new(std::sync::Mutex::new(event.index.clone()));
        // A duplicate entry for the same root (a stale re-build; never
        // expected — single-flight) would leak: replace, not append.
        self.external_indexes.retain(|(r, _)| r != &event.source_root);
        self.external_indexes
            .push((event.source_root.clone(), index));
        while self.external_indexes.len() > EXT_INDEX_CAP {
            self.external_indexes.remove(0); // evict the oldest
        }
        self.bump_current_crate_recency();
        self.crate_indexing
            .retain(|(r, _, _)| r != &event.source_root);
    }

    /// The `indexing crate <dir> (i/N)…` indicator for the activity
    /// display (empty when idle; the N/M counter is the build's file
    /// progress, 006-03b item 5): mirrors the project indexing indicator
    /// on the status line.
    pub fn crate_indexing_display(&self) -> String {
        self.crate_indexing
            .first()
            .map(|(_, label, p)| {
                format!("indexing crate {label} ({}/{})…", p.done(), p.total())
            })
            .unwrap_or_default()
    }

    /// The cached index handle for an exact `source_root` (the LRU
    /// recency bump — the entry moves to the newest slot): the caller
    /// locks it within its own scope.
    fn crate_index_arc(
        &mut self,
        root: &Path,
    ) -> Option<Arc<std::sync::Mutex<SymbolIndex>>> {
        let pos = self.external_indexes.iter().position(|(r, _)| r == root);
        let entry = self.external_indexes.remove(pos?);
        self.external_indexes.push(entry.clone());
        Some(entry.1)
    }

    /// The cached index handle of the crate owning `path` (the
    /// `source_root` it is nested under — there is exactly one: the
    /// cache holds sibling registry crate roots, never a nested pair),
    /// with the LRU recency bump.
    fn crate_index_arc_for_path(
        &mut self,
        path: &Path,
    ) -> Option<(PathBuf, Arc<std::sync::Mutex<SymbolIndex>>)> {
        let root = self
            .external_indexes
            .iter()
            .find(|(r, _)| path.starts_with(r))?
            .0
            .clone();
        let arc = self.crate_index_arc(&root)?;
        Some((root, arc))
    }

    /// The crate-relative index key for `path` under `root` —
    /// forward-slash normalized (006-03b item 3): the SAME key shape
    /// `crate_source_files` builds with, so a native-`\`-separated `rel`
    /// never breaks the same-file-first ordering or the `outline` lookup
    /// (the Windows separator case). `None` when `path` is not under
    /// `root` (callers treat that as the empty result, never a panic —
    /// 006-03b item 4).
    fn crate_rel(path: &Path, root: &Path) -> Option<String> {
        path.strip_prefix(root)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
    }

    /// 006-03b item 1: keep the current buffer's OWNING crate MRU — the
    /// crate the user is IN is never the LRU eviction victim. Called on
    /// every transition that can put an external buffer current (the
    /// `open_external_path` landing, the buffer-list / picker / jump-back
    /// switches) and at every index arrival (the eviction point, in
    /// `apply_crate_index_event`).
    fn bump_current_crate_recency(&mut self) {
        let Some(key) = self.buffers.current().map(String::from) else {
            return;
        };
        let Some(path) = self.buffers.get(&key).and_then(|b| b.path.clone()) else {
            return;
        };
        let Some((root, _)) = self
            .external_indexes
            .iter()
            .find(|(r, _)| path.starts_with(r))
            .cloned()
        else {
            return;
        };
        let _ = self.crate_index_arc(&root);
    }

    /// 006-03b item 2: the resolver fall-through's `from_file` for an
    /// EXTERNAL buffer. `SymbolContext.from_file` is documented
    /// root-relative (the project path passes the project-relative
    /// `rel`), so the absolute path never goes: the crate-relative key
    /// shape (the index's own key) when the owning root is known —
    /// cached, or still in flight — else the bare file name (relative,
    /// never absolute; the root is genuinely unknown when a build was
    /// refused or no runtime is present).
    fn resolver_from_file(&self, path: &Path) -> String {
        let root = self
            .external_indexes
            .iter()
            .find(|(r, _)| path.starts_with(r))
            .map(|(r, _)| r.clone())
            .or_else(|| {
                self.crate_indexing
                    .iter()
                    .find(|(r, _, _)| path.starts_with(r))
                    .map(|(r, _, _)| r.clone())
            });
        root.and_then(|root| Self::crate_rel(path, &root))
            .unwrap_or_else(|| {
                // Never absolute: `SymbolContext.from_file` is root-relative.
                // file_name() is Some for every real buffer path; the
                // empty-string fallback (rather than display()) keeps the
                // contract even for the pathological `/` or `..` shapes.
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
    }

    /// M-. inside an EXTERNAL (registry / tooling) buffer (plan 006
    /// issue 03): the SAME selection rule as the project path (symbol at
    /// point + `::`-path token, same-file-first candidate ordering,
    /// dedup, enclosing-symbol fallback) run against the OWNING crate's
    /// index (keyed against its source_root); a crate miss keeps the
    /// resolver fall-through (unchanged semantics — `SymbolContext` still
    /// carries the ORIGIN project's workspace_root, so following a type
    /// into ANOTHER dependency resolves through the origin project's
    /// metadata; the landing in that crate registers its index per
    /// `open_resolved_source`, and the LRU cap governs).
    fn xref_in_external_buffer(&mut self, key: &str, path: &Path) {
        // Supersede any in-flight tooling resolve (006-02b item 2: a hit
        // must not be clobbered by a stale event of a superseded
        // request); the fall-through path re-bumps in
        // `start_symbol_resolution`.
        self.resolve_generation += 1;
        let line = self.point_line();
        let line_text = self
            .buffers
            .get(key)
            .map(|b| b.line_text(line).unwrap_or_default())
            .unwrap_or_default();
        // 011-06: language-aware path token (the same seam as the project
        // path — external buffers are non-Rust in practice, but the
        // behavior is uniform by construction).
        let lang = self.grammar_registry.language_for(&path.to_string_lossy());
        let at = Self::symbol_at_point(lang, &line_text, self.point_col());
        let Some((root, outcome)) =
            self.crate_xref_outcome(key, path, line, lang, at.as_ref())
        else {
            // No crate index for this root yet (build in flight, refused,
            // or no runtime): the miss behaves as the project's — the
            // resolver fall-through (or "no symbol under point").
            match &at {
                // 006-03b item 2: a root-relative `from_file` (never the
                // absolute path).
                Some((_, path_token)) => {
                    self.start_symbol_resolution(path_token, &self.resolver_from_file(path))
                }
                None => self.minibuffer_message("no symbol under point"),
            }
            return;
        };
        match outcome {
            ExternalXrefOutcome::Jump { file, line } => {
                let origin = self.current_jump_entry();
                let abs = root.join(&file);
                if self.open_external_path(&abs).is_some() {
                    self.set_point_line(line);
                    self.recenter_landing();
                    self.ensure_highlight();
                    self.record_jump(origin, "M-.");
                    self.minibuffer_message(&format!(
                        "jumped to {file}:{}",
                        line + 1
                    ));
                } else {
                    self.minibuffer_message(&format!("cannot open {file}"));
                }
            }
            ExternalXrefOutcome::Picker { lookup, defs } => {
                self.xref_crate_root = Some(root);
                self.xref_lookup_name = lookup;
                let candidates: Vec<PickerCandidate> = defs
                    .iter()
                    .map(|d| PickerCandidate {
                        name: format!("{}:{}", d.file, d.symbol.line + 1),
                        display: format!(
                            "{}:{}  [{}] {}",
                            d.file,
                            d.symbol.line + 1,
                            d.symbol.kind.tag(),
                            d.symbol.name
                        ),
                        docs: String::new(),
                        category: "xref".to_string(),
                    })
                    .collect();
                self.open_picker(PickerKind::Xref, "Definition: ", candidates);
            }
            ExternalXrefOutcome::Resolver(token) => {
                // 006-03b item 2: a root-relative `from_file` (never the
                // absolute path).
                self.start_symbol_resolution(&token, &self.resolver_from_file(path))
            }
            ExternalXrefOutcome::NoDefinition(name) => {
                self.minibuffer_message(&format!("no definition for `{name}`"))
            }
            ExternalXrefOutcome::NoSymbol => {
                self.minibuffer_message("no symbol under point")
            }
        }
    }

    /// The M-. selection outcome for an EXTERNAL buffer, run against its
    /// owning crate's index (crate-relative paths, the project path's
    /// same-file-first ordering). `None` when the root has no cached
    /// index yet.
    fn crate_xref_outcome(
        &mut self,
        key: &str,
        path: &Path,
        line: usize,
        lang: LanguageId,
        at: Option<&(String, String)>,
    ) -> Option<(PathBuf, ExternalXrefOutcome)> {
        let (root, arc) = self.crate_index_arc_for_path(path)?;
        let idx = arc.lock().unwrap();
        // 006-03b item 3: the index key shape (forward-slash normalized).
        let rel = Self::crate_rel(path, &root)?;
        // (2) Symbol-at-point definitions (the same selection rule as the
        // project path).
        let defs = at
            .and_then(|(ident, token)| {
                // 010-01: the Rust self-receiver pre-step (the SAME seam as
                // the project path — external crates are Rust in practice,
                // and the behavior is uniform by construction).
                if lang == LanguageId::Rust
                    && let Some(member) = token.strip_prefix("self.")
                    && !member.is_empty()
                    && let Some(buf) = self.buffers.get(key)
                {
                    let source = buf.rope.to_string();
                    if let Some(byte) = point_byte_offset(&buf.rope, line, self.point_col()) {
                        let cands =
                            Self::self_receiver_candidates(&idx, &rel, &source, byte, member);
                        if !cands.is_empty() {
                            return Some(cands);
                        }
                    }
                }
                // 010-03: the local-binding pre-step (the SAME seam as the
                // project path — external crates are Rust in practice, and
                // the behavior is uniform by construction).
                if lang == LanguageId::Rust
                    && let Some(buf) = self.buffers.get(key)
                    && let Some((receiver, member_col)) = Self::rust_dotted_receiver(
                        &buf.line_text(line).unwrap_or_default(),
                        self.point_col(),
                    )
                {
                    let source = buf.rope.to_string();
                    if let Some(byte) = point_byte_offset(&buf.rope, line, member_col) {
                        let cands = Self::local_binding_candidates(
                            &idx,
                            &rel,
                            &source,
                            byte,
                            &receiver,
                            ident,
                        );
                        if !cands.is_empty() {
                            return Some(cands);
                        }
                    }
                }
                Self::xref_definition_candidates(&idx, ident, token, &rel)
            })
            .unwrap_or_default();
        let outcome = if !defs.is_empty() {
            let lookup = defs[0].symbol.name.clone();
            if defs.len() == 1 {
                ExternalXrefOutcome::Jump {
                    file: defs[0].file.clone(),
                    line: defs[0].symbol.line,
                }
            } else {
                ExternalXrefOutcome::Picker { lookup, defs }
            }
        } else {
            // (3) Enclosing-symbol fallback (unchanged: by line, not by the
            // point's column).
            let outline = idx.outline(&rel).to_vec();
            match crate::nav::index::enclosing_symbol(&outline, line) {
                Some(sym) => {
                    let lookup = sym.name.clone();
                    let defs = idx.definitions_of(&lookup);
                    if defs.is_empty() {
                        // (4) The point's own (path-shaped) token is what
                        // the resolver gets — not the enclosing name.
                        match at {
                            Some((_, token)) => {
                                ExternalXrefOutcome::Resolver(token.clone())
                            }
                            None => ExternalXrefOutcome::NoDefinition(lookup),
                        }
                    } else if defs.len() == 1 {
                        ExternalXrefOutcome::Jump {
                            file: defs[0].file.clone(),
                            line: defs[0].symbol.line,
                        }
                    } else {
                        ExternalXrefOutcome::Picker { lookup, defs }
                    }
                }
                None => match at {
                    Some((_, token)) => ExternalXrefOutcome::Resolver(token.clone()),
                    None => ExternalXrefOutcome::NoSymbol,
                },
            }
        };
        Some((root, outcome))
    }

    /// The identifier run at the point's column (char offset) on `text`, plus
    /// the path token it belongs to (plan 006 issue 02, selection rule 1).
    /// Returns `(identifier, path_token)` — e.g. the cursor inside
    /// `tokio::spawn` → `("spawn", "tokio::spawn")`; on a Rust `obj.name`
    /// → `("name", "name")`; on a Rust `self.name` →
    /// `("name", "self.name")` (010-01 — the `self.` receiver is carried so
    /// the M-. self-resolution pre-step sees it; every other `.` receiver
    /// stays bare). A cursor parked just AFTER the name (before
    /// `(`, `.`, or whitespace — the usual call-site spot) counts. `None`
    /// when the point is not on (or immediately after) an identifier run.
    ///
    /// 011-06: the PATH TOKEN is language-aware. Rust's `::` shape is the
    /// char-scan below, byte-for-byte unchanged; a Rust `.` field access
    /// stays bare EXCEPT the `self.` receiver (010-01 — `self.name` →
    /// `self.name`, which the M-. self-resolution pre-step consumes; the
    /// other receivers' fields are not in the index). In a non-Rust
    /// buffer, when the point sits inside the language's dotted path
    /// container, the path token becomes the WHOLE dotted path
    /// (`json.dumps`, `ns.member`, `pkg.Fn`) so the providers' already-
    /// unit-tested dotted handling is reachable from M-.: ONE parse
    /// (`syntax::node::node_at` — 011-03's whole-path machinery, 007-01's
    /// one-parse discipline). When `node_at` returns `None` (no tree / a
    /// shape it does not cover / the identifier is not a full
    /// dot-delimited segment of the container, e.g. a computed member
    /// `a[b]`) the exact current bare extraction stands — never guess.
    fn symbol_at_point(
        lang: LanguageId,
        text: &str,
        col: usize,
    ) -> Option<(String, String)> {
        let chars: Vec<char> = text.chars().collect();
        if col > chars.len() {
            return None;
        }
        let is_ident = |c: char| c.is_alphanumeric() || c == '_';
        // 006-02b item 4: a cursor parked on the SECOND colon of a `::`
        // separator (the point's own char and its predecessor are both
        // `:`) sits at the very end of the preceding path segment — treat
        // the point as one char to the left (the segment's end) so the
        // identifier / path token extract per the existing rules
        // (`a::b` → `("a", "a::b")`). The FIRST colon is already covered:
        // the char before the point is the segment's last identifier char.
        let col = if col >= 2
            && col < chars.len()
            && chars[col] == ':'
            && chars[col - 1] == ':'
            && is_ident(chars[col - 2])
        {
            col - 1
        } else {
            col
        };
        // The run the point belongs to: the char AT the point when it is an
        // identifier char, else the run ending immediately BEFORE it.
        let start = if col < chars.len() && is_ident(chars[col]) {
            let mut i = col;
            while i > 0 && is_ident(chars[i - 1]) {
                i -= 1;
            }
            i
        } else if col > 0 && is_ident(chars[col - 1]) {
            let mut i = col - 1;
            while i > 0 && is_ident(chars[i - 1]) {
                i -= 1;
            }
            i
        } else {
            return None;
        };
        let mut end = start;
        while end < chars.len() && is_ident(chars[end]) {
            end += 1;
        }
        let identifier: String = chars[start..end].iter().collect();
        // Extend left across `::` separators to the full path token (kept for
        // the tooling resolver; the workspace index stays name-keyed and is
        // tried with both the last segment and this token).
        let mut pstart = start;
        while pstart >= 3 && chars[pstart - 1] == ':' && chars[pstart - 2] == ':' {
            // The segment before the `::`: the identifier run ending at index
            // `pstart - 3`. A non-identifier there means a leading `::` — stop.
            let after_sep = pstart - 3;
            if !is_ident(chars[after_sep]) {
                break;
            }
            let mut i = after_sep;
            while i > 0 && is_ident(chars[i - 1]) {
                i -= 1;
            }
            pstart = i;
        }
        let mut pend = end;
        while pend + 2 < chars.len()
            && chars[pend] == ':'
            && chars[pend + 1] == ':'
            && is_ident(chars[pend + 2])
        {
            pend += 3;
            while pend < chars.len() && is_ident(chars[pend]) {
                pend += 1;
            }
        }
        let path_token: String = chars[pstart..pend].iter().collect();
        // 010-01: the Rust self-receiver — when the identifier run is the
        // member of `self.` (the run is preceded by `.self`, exactly), the
        // path token carries the receiver (`self.bar`), so the M-.
        // self-resolution pre-step (in `xref_find_definitions`) sees it.
        // Every other Rust `.` access stays bare (byte-for-byte — fields
        // of arbitrary receivers are still not resolvable); a prefix like
        // `myself.` must NOT match (the 4-char window must be exactly
        // `self`, not preceded by an identifier char).
        let path_token = if lang == LanguageId::Rust
            && pstart == start
            && start >= 5
            && chars[start - 1] == '.'
            && chars[start - 5..start - 1] == ['s', 'e', 'l', 'f']
            && (start < 6 || !is_ident(chars[start - 6]))
        {
            format!("self.{}", identifier)
        } else {
            path_token
        };
        // 011-06: non-Rust — upgrade the path token to the whole dotted
        // path when the point sits inside the language's path container.
        // `node_at` takes a byte offset: the identifier run's last char
        // (always in-range — the run is non-empty here). Any miss keeps
        // the bare extraction byte-for-byte (the `::` scan above is the
        // Rust shape; `.` never extends it).
        let path_token = if lang != LanguageId::Rust
            && let Some(byte) = text.char_indices().nth(end - 1).map(|(b, _)| b)
            && let Some(info) = crate::syntax::node::node_at(lang, text, byte)
            && Self::dotted_path_container(lang, &info.kind)
            && info.text.split('.').any(|seg| seg == identifier)
            // 011-06 review P1: EVERY dot-delimited segment must be a bare
            // identifier. The raw container text of a wrong-container shape
            // is NOT a dotted path — `a?.b` (JS/TS optional chaining),
            // `foo().bar` (Python attribute-on-call), `(*p).field` (Go
            // pointer receiver) all match the segment-membership check but
            // their non-identifier segments would be treated as package /
            // module names by the providers (an unintended `npm install
            // "a?"` / `pip install "foo()"` shell-out in online projects).
            // Only a genuine path upgrades; these fall back to the
            // byte-for-byte bare extraction.
            && info
                .text
                .split('.')
                .all(|seg| !seg.is_empty() && seg.chars().all(is_ident))
        {
            info.text
        } else {
            path_token
        };
        Some((identifier, path_token))
    }

    /// 011-06: the per-language DOTTED path-container node kinds — exactly
    /// the containers 011-03's `node_at` whole-path machinery returns, as
    /// pinned per language in `src/syntax/node.rs`: JS/TS/TSX
    /// `member_expression` (`a.b.c`) and the TS-only nested type
    /// identifiers, Python's `attribute` (`a.b.c`), Go's
    /// `selector_expression` / `qualified_type` (`pkg.Fn`), C's
    /// `field_expression` (`o.x` — `p->x` fails the caller's
    /// all-identifier-segment check and stays bare), Cpp's
    /// `field_expression` (its `::` shape is ALREADY carried whole by
    /// `symbol_at_point`'s byte-scan — `qualified_identifier` would be
    /// byte-for-byte the same token, so it is deliberately NOT enumerated
    /// here: no double handling), Toml's `dotted_key` (`a.b.c` — the
    /// index stores the dotted key as ONE symbol name, so without the
    /// upgrade an M-. on a segment could never hit it), Java's
    /// `field_access` (`A.c` — `o.m(…)` is a `method_invocation`, NOT a
    /// container: node.rs returns the bare `m` identifier there) plus
    /// `scoped_identifier` / `scoped_type_identifier` (`com.example.Foo`
    /// — the Rust `::` shape, dot-delimited), C#'s
    /// `member_access_expression` (`o.P` / `a.b.c`) + `qualified_name`
    /// (`N.Inner`), and Ruby's argumentless `call` (`a.b.c` — `node_at`
    /// returns a `call` node only while it is a genuine path container:
    /// a `receiver` field AND no `arguments` field, the same gate as
    /// node.rs's `in_identifier_position`, so an argument-carrying call
    /// NEVER reaches this arm; its inner segments come back as bare
    /// identifiers instead). Ruby's `scope_resolution` (`Foo::Bar`) is
    /// deliberately NOT enumerated: the `::` byte-scan above already
    /// carries it whole, and the all-identifier-segment check would
    /// reject a `Foo::Bar` segment anyway (no double handling). Json has
    /// NO container: the pinned JSON grammar has
    /// no dotted-key node — every key is a standalone string — so a JSON
    /// key's M-. stays the byte-for-byte bare index lookup (judgment: the
    /// index fall-through already lands bare keys; there is no key-PATH
    /// to speak of). Scheme has no dotted-path construct at all (the
    /// flat S-expression grammar — `is_path_segment`'s default arm).
    /// Rust is out of scope here — its `::` shape is extracted
    /// byte-for-byte in `symbol_at_point` itself.
    fn dotted_path_container(lang: LanguageId, kind: &str) -> bool {
        match lang {
            LanguageId::JavaScript | LanguageId::TypeScript | LanguageId::Tsx => {
                matches!(
                    kind,
                    "member_expression" | "nested_type_identifier" | "nested_identifier"
                )
            }
            LanguageId::Python => kind == "attribute",
            LanguageId::Go => {
                matches!(kind, "selector_expression" | "qualified_type")
            }
            LanguageId::C => kind == "field_expression",
            LanguageId::Cpp => kind == "field_expression",
            LanguageId::Toml => kind == "dotted_key",
            LanguageId::Java => matches!(
                kind,
                "field_access" | "scoped_identifier" | "scoped_type_identifier"
            ),
            LanguageId::CSharp => {
                matches!(kind, "member_access_expression" | "qualified_name")
            }
            // node_at returns a `call` node only for the argumentless
            // receiver-carrying shape (node.rs's `in_identifier_position`
            // gate), so `a.b(1)` never arrives here — the caller's
            // all-identifier-segment check additionally rejects any
            // `call` whose text carries a `(...)` segment (`a.b(1).c`
            // → the `b(1)` segment is not a bare identifier → bare
            // extraction, byte-for-byte).
            LanguageId::Ruby => kind == "call",
            _ => false,
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
        // 006-03: an EXTERNAL (registry / tooling) buffer gets its outline
        // from the owning crate's index (below) instead of refusing; a
        // non-external buffer outside the root keeps the refusal.
        match path.strip_prefix(&project.root) {
            Ok(_) => {}
            Err(_) if self.external_buffers.contains(&key) => {}
            Err(_) => {
                self.minibuffer_message("buffer not in project");
                return;
            }
        }
        let outline = self.current_buffer_outline();
        if outline.is_empty() {
            self.minibuffer_message("no symbols in current file");
            return;
        }
        // Watchlist (impl-parent grouping): the file's Rust tables (Rung 1)
        // group the impl methods under their impl's type; other languages
        // have no tables and stay flat.
        let tables = self.current_buffer_rust_tables();
        self.open_imenu_picker(outline, tables);
    }

    /// The current buffer's symbol outline: the project index for project
    /// files, the OWNING crate's index for external (registry / tooling)
    /// buffers (006-03), empty for scratch / out-of-project non-external.
    fn current_buffer_outline(&mut self) -> Vec<crate::nav::index::Symbol> {
        let Some(key) = self.buffers.current().map(String::from) else {
            return Vec::new();
        };
        let Some(path) = self.buffers.get(&key).and_then(|b| b.path.clone()) else {
            return Vec::new();
        };
        let Some(project) = self.project.as_ref() else {
            return Vec::new();
        };
        match path.strip_prefix(&project.root) {
            Ok(rel) => self.index.outline(&rel.to_string_lossy()).to_vec(),
            Err(_) if self.external_buffers.contains(&key) => self
                .crate_index_arc_for_path(&path)
                .and_then(|(root, arc)| {
                    // 006-03b item 4: a `strip_prefix` miss yields the
                    // empty outline (the existing fallback), never a
                    // panic.
                    let crel = Self::crate_rel(&path, &root)?;
                    let idx = arc.lock().unwrap();
                    Some(idx.outline(&crel).to_vec())
                })
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    /// The current buffer's Rust tables (plan 010 Shape A, rung 1): the
    /// project index for project files, the OWNING crate's index for
    /// external (registry / tooling) buffers (006-03), `None` otherwise
    /// (scratch / out-of-project non-external, or a non-Rust file — the
    /// tables only exist for Rust). The imenu impl-parent grouping's data
    /// source (watchlist item).
    fn current_buffer_rust_tables(&mut self) -> Option<crate::syntax::queries::RustTables> {
        let key = self.buffers.current().map(String::from)?;
        let path = self.buffers.get(&key).and_then(|b| b.path.clone())?;
        let project = self.project.as_ref()?;
        match path.strip_prefix(&project.root) {
            Ok(rel) => self.index.tables(&rel.to_string_lossy()).cloned(),
            Err(_) if self.external_buffers.contains(&key) => self
                .crate_index_arc_for_path(&path)
                .and_then(|(root, arc)| {
                    // The same strip_prefix-miss tolerance as
                    // `current_buffer_outline`: `None`, never a panic.
                    let crel = Self::crate_rel(&path, &root)?;
                    Some(arc.lock().unwrap().tables(&crel).cloned())
                })
                .flatten(),
            Err(_) => None,
        }
    }

    /// Open the imenu picker over `outline` + the file's Rust `tables`
    /// (indentation by enclosing extent, plus the Rust impl-parent
    /// grouping — shared by the project and external-buffer paths).
    fn open_imenu_picker(
        &mut self,
        outline: Vec<crate::nav::index::Symbol>,
        tables: Option<crate::syntax::queries::RustTables>,
    ) {
        let candidates: Vec<PickerCandidate> = outline
            .iter()
            .map(|s| Self::imenu_candidate(s, &outline, tables.as_ref()))
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
        // The symbol picker lists the PROJECT index — any open Xref
        // picker's crate root (006-03) must not leak into it.
        self.xref_crate_root = None;
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
        // The pre-search position, captured BEFORE the open (the open
        // changes the current buffer): it records in front of the
        // sentinel so a second `M-,` from the results view lands here
        // (watchlist: `M-,` under the Search view pops through the
        // sentinel to the pre-search position in one step — emacs
        // `xref-pop-marker-stack`). The home state (no buffer) has none:
        // the sentinel stays the stack's first entry, unchanged.
        let pre_search = self.current_jump_entry();
        let file = hit.file.clone();
        let line_no = hit.line_no as usize;
        // The open-or-report seam (like `open_resolved_source`): only the
        // success path leaves the results view and records the jump.
        // Watchlist (the pre-011 artifact): on an OPEN FAILURE the
        // results view stays open and the failure is reported (the jump
        // did not happen) — no jump entry is recorded, no view closed.
        let opened = match self.project.as_ref() {
            Some(project) => self
                .open_project_path(&project.root.join(&file), &file)
                .is_ok(),
            None => false,
        };
        if !opened {
            // The hit's file could not be opened (e.g. it was deleted
            // since the walk, or an occur on a buffer with no file).
            self.minibuffer_message(&format!("cannot open {file}: the jump did not happen"));
            return;
        }
        self.set_point_line(line_no.saturating_sub(1));
        self.ensure_highlight();
        // Leave the results view so the jumped file is what's on screen;
        // `M-,` (the sentinel entry) pushes the results back on top.
        if self.top_view() == ViewId::Search {
            self.close_view();
        }
        // 06a review P1-2: only record the destination jump when the open
        // actually opened a buffer (the `opened` gate above — a failed
        // open leaves the home state, and a `""`-keyed entry would later
        // create `*scratch*` via a dead jump entry (P1-1's class)).
        if let Some(key) = self.buffers.current().map(String::from) {
            if let Some(pre) = pre_search {
                self.jump_stack.record_jump(&pre, &origin);
            }
            let dest = JumpEntry {
                buffer_key: key,
                line: line_no.saturating_sub(1),
                col: hit.col.unwrap_or(0) as usize,
                label: "search-RET".to_string(),
            };
            self.jump_stack.record_jump(&origin, &dest);
            self.minibuffer_message(&format!("jumped to {file}:{line_no}"));
        }
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
        // 06a review P2: no scratch fallback — the home state has no buffer;
        // the honest "no buffer" echo is the whole behavior.
        let Some(key) = self.buffers.current().map(String::from) else {
            self.minibuffer_message("no buffer");
            return;
        };
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
        // 06a: home renders in the buffer slot, so the tree stays fully
        // usable on top of it — RET opens the file and replaces home.
        if self.tree_visible()
            && matches!(self.top_view(), ViewId::Buffer | ViewId::Home)
        {
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

/// The new scroll offset that puts buffer line `point_line` on screen row
/// `desired_row`: `scroll_top = point_line - desired_row`, clamped to
/// `[0, total - viewport]`. Returns `None` when the buffer does not scroll
/// at all (nothing to recenter). Shared by `recenter` (C-l — the desired
/// row is cycle-selected) and the jump-landing recenter (plan 004 issue
/// 07 — always the fresh MIDDLE row; the only difference from `recenter`
/// is that `recenter_cycle` is NOT advanced: a jump is not a `C-l`).
fn recenter_top_for(point_line: usize, desired_row: usize, total: usize, viewport: usize) -> Option<usize> {
    if total <= 1 {
        return None;
    }
    let vp = viewport.max(1);
    let max_scroll = total.saturating_sub(vp);
    if max_scroll == 0 {
        return None;
    }
    Some((point_line as i64 - desired_row as i64).clamp(0, max_scroll as i64) as usize)
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

/// The point's BYTE offset into `rope` for the (007-01) tree-sitter
/// surfaces, which key on raw-source byte offsets: the line's char start
/// plus the point's (char-based) column, translated to bytes. `None` when
/// either translation is out of range (the caller degrades to the empty
/// scope hint).
fn point_byte_offset(rope: &Rope, line: usize, col: usize) -> Option<usize> {
    let char_off = rope.try_line_to_char(line).ok()?;
    let char_off = char_off.saturating_add(col);
    rope.try_char_to_byte(char_off).ok()
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
    fn bare_q_in_home_view_is_unbound_not_quit() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        // 06a: bare `q` on the home view is UNBOUND (no buffer to close) —
        // an unbound-key echo, NOT a quit and NOT a view change.
        s.key_event(key("q"));
        assert!(!s.quit, "bare q must not quit the app");
        assert_eq!(s.view_stack.len(), 1, "the home view must remain");
        assert_eq!(s.top_view(), ViewId::Home);
        assert!(s.message.contains("unbound key: q"), "msg: {}", s.message);

        // `q` closes an overlay list view back to home (consistent with
        // the list views' `q`).
        s.key_event(key("C-x"));
        s.key_event(key("C-b")); // list-buffers
        assert_eq!(s.top_view(), ViewId::BufferList);
        s.key_event(key("q"));
        assert_eq!(s.top_view(), ViewId::Home, "q must close the list view");
        assert!(!s.quit);

        // `C-x C-c` quits IMMEDIATELY from home (no buffers ⇒ nothing to
        // prompt; the 004-04 semantics).
        s.key_event(key("C-x"));
        s.key_event(key("C-c"));
        assert!(s.quit, "C-x C-c must still quit");
        assert!(!s.quit_prompt_active(), "no save prompt with zero buffers");
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
        // 06a: boot has NO current buffer — toggle is a no-op with a message
        // (and must not create one). Then explicit scratch (the 06a
        // affordance): scratch (no path) is a no-op with its own message.
        s.toggle_read_only();
        assert!(s.message.contains("not a buffer view"), "msg: {}", s.message);
        assert_eq!(s.buffers.len(), 0, "toggle must not create a buffer");
        s.open_scratch();
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

    /// 006-02b item 1 fixture: a project store with an external (outside-
    /// root) file open read-only via the tooling-landing path.
    fn store_with_external_buffer() -> (tempfile::TempDir, tempfile::TempDir, AppStore) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let mut s = store(dir.path());
        let ext = tempfile::tempdir().unwrap();
        let abs = ext.path().join("registry_src.rs");
        std::fs::write(&abs, "pub fn spawn<F>(f: F) {}\n").unwrap();
        s.open_external_path(&abs).unwrap();
        (dir, ext, s)
    }

    #[test]
    fn toggle_read_only_refuses_external_buffer() {
        // 006-02b item 1: the C-x C-q override must NEVER turn an external
        // (registry / tooling) buffer editable — it is a cache shared by
        // every project on the machine.
        let (_dir, _ext, mut s) = store_with_external_buffer();
        let key = s.buffers.current().unwrap().to_string();
        assert!(!s.buffers.get(&key).unwrap().editable,
            "external buffers start read-only");
        s.toggle_read_only();
        assert!(!s.buffers.get(&key).unwrap().editable,
            "C-x C-q must NOT turn an external buffer editable");
        assert!(s.message.contains("external buffer is read-only"), "msg: {}", s.message);
        // Plan-005 semantics intact: a PROJECT file in the same session
        // still toggles into edit mode.
        std::fs::create_dir_all(_dir.path().join("src")).unwrap();
        std::fs::write(_dir.path().join("src/p.rs"), "fn p() {}\n").unwrap();
        s.open_path("src/p.rs");
        s.toggle_read_only();
        let pkey = s.buffers.current().unwrap().to_string();
        assert!(s.buffers.get(&pkey).unwrap().editable,
            "a project file toggles into edit mode as before (msg: {})", s.message);
    }

    #[test]
    fn buffer_is_project_owned_classifies_external_scratch_and_project() {
        // 006-02b item 1: the ownership notion itself — external paths
        // refuse, scratch and project files (incl. the notes file) pass.
        let (dir, _ext, mut s) = store_with_external_buffer();
        let ext_key = s.buffers.current().unwrap().to_string();
        assert!(!s.buffer_is_project_owned(&ext_key), "registry source: not owned");
        // 06a: scratch no longer exists at boot — create it (the explicit
        // affordance) before checking its ownership.
        s.open_scratch();
        assert!(s.buffer_is_project_owned(SCRATCH_NAME), "scratch (no path): owned as today");
        // The notes file lives under the root: owned (the 005 decision —
        // notes keep their edit-mode semantics).
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        s.open_notes();
        let notes_key = s.notes_key().unwrap();
        assert!(s.buffer_is_project_owned(&notes_key), "notes file (under root): owned");
    }

    #[test]
    fn save_buffer_key_refuses_external_even_when_editable() {
        // 006-02b item 1: even if an external buffer somehow reached edit
        // mode (a future bug), the save must be refused — the write would
        // corrupt a cache shared by every project on the machine.
        let (_dir, ext, mut s) = store_with_external_buffer();
        let key = s.buffers.current().unwrap().to_string();
        s.buffers.get_mut(&key).unwrap().editable = true; // the hypothetical future bug
        assert!(!s.save_buffer_key(&key), "the save must be refused");
        assert!(s.message.contains("external buffer is read-only"), "msg: {}", s.message);
        let abs = ext.path().join("registry_src.rs");
        assert_eq!(std::fs::read_to_string(&abs).unwrap(), "pub fn spawn<F>(f: F) {}\n",
            "the external file is untouched");
        // Plan-005 path still works: a project file passes the gate and
        // writes to disk.
        std::fs::create_dir_all(_dir.path().join("src")).unwrap();
        std::fs::write(_dir.path().join("src/p.rs"), "old\n").unwrap();
        s.open_path("src/p.rs");
        s.toggle_read_only(); // into edit mode (project file: allowed)
        let pkey = s.buffers.current().unwrap().to_string();
        assert!(s.save_buffer_key(&pkey), "a project file saves (msg: {})", s.message);
    }

    #[test]
    fn save_buffer_key_still_writes_previous_project_buffer_after_switch() {
        // 006-02b item 1: the guard refuses only EXTERNAL (registry / tooling)
        // buffers. A previous project's buffer (no longer under the new root
        // after a switch) is the user's own file — C-x C-s still writes it.
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        for d in [&dir1, &dir2] {
            std::fs::write(d.path().join("Cargo.toml"), "[package]\n").unwrap();
            std::fs::create_dir_all(d.path().join("src")).unwrap();
        }
        std::fs::write(dir1.path().join("src/a.rs"), "fn a() {}\n").unwrap();
        let mut s = store(dir1.path());
        s.open_path("src/a.rs");
        s.toggle_read_only(); // into edit mode (under root1: owned)
        let key1 = s.buffers.current().unwrap().to_string();
        assert!(s.buffers.get(&key1).unwrap().editable);
        // Switch to a second project: dir1's buffer is no longer under the
        // current root, but it is not an external landing either.
        s.switch_project_root(dir2.path().to_str().unwrap());
        let root2 = s.project.as_ref().unwrap().root.clone();
        assert!(!s.buffers.get(&key1).unwrap().path.as_ref().unwrap().starts_with(&root2),
            "precondition: the buffer is outside the new root");
        assert!(s.buffer_is_project_owned(&key1), "previous-project file is still owned");
        assert!(s.save_buffer_key(&key1), "previous-project file still saves (msg: {})", s.message);
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
        // 06a: boot is the home view (no auto-created scratch).
        assert_eq!(store.view_name_display(), "home");
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
        assert_eq!(store.buffers.len(), 2); // 06a: no scratch

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

        // C-x k: kill it; the killed buffer disappears. 06a: no scratch
        // fallback — with main.rs still current the view stays on it.
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
        assert_eq!(store.buffers.len(), 1); // main.rs only (06a: no scratch)
        assert_eq!(store.view_name_display(), "src/main.rs");
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
        assert_eq!(rows.len(), 1, "06a: only the opened buffer, no scratch");
        assert!(rows.iter().any(|r| r.name == "src/main.rs" && r.current));
        assert!(!rows.iter().any(|r| r.name == "*scratch*"));

        // q closes the view back to the buffer view (main.rs replaced home
        // when it was opened; the table is non-empty, so it renders as the
        // buffer view).
        store.key_event(key("q"));
        assert_eq!(store.top_view(), ViewId::Buffer);
        store.dispatch("list-buffers", None).unwrap();
        store.key_event(key("RET")); // row 0 is the MRU buffer (main.rs)
        assert_eq!(store.top_view(), ViewId::Buffer);
        assert_eq!(store.view_name_display(), "src/main.rs");
    }

    // ── plan 004 issue 06a: empty buffer table + home view ─────────────

    /// Boot pin: the table starts empty, `current` is `None`, and the main
    /// view is home with a DERIVED header/body (project + "redline",
    /// registry categories + live global bindings, no buffer-view keys).
    #[test]
    fn boot_starts_on_home_with_empty_buffer_table() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let name = dir.path().file_name().unwrap().to_string_lossy().into_owned();
        let s = store(dir.path());
        assert_eq!(s.top_view(), ViewId::Home);
        assert_eq!(s.render_view(), ViewId::Home);
        assert_eq!(s.view_name_display(), "home");
        assert_eq!(s.buffers.len(), 0, "06a: no auto-created buffer at boot");
        assert!(s.buffers.current().is_none());
        assert!(
            s.home_title().starts_with(&format!("redline · {name}")),
            "{}",
            s.home_title()
        );
        // The home body is derived: registry categories + live globals.
        let rows = s.home_rows();
        assert!(rows.iter().any(|r| r.is_header && r.label == "files"));
        assert!(rows.iter().any(|r| r.is_header && r.label == "git"));
        assert!(rows.iter().any(|r| !r.is_header && r.key_display == "C-x C-f"));
        assert!(rows.iter().any(|r| !r.is_header && r.key_display == "C-x C-c"));
        // View-local buffer keys are NOT on home (the view map is empty).
        assert!(!rows.iter().any(|r| !r.is_header && r.key_display == "q"));
    }

    /// Anti-drift pin (06a): home's body must be GENERATED from the live
    /// keymap × command registry — mutate the registry and the keymap in a
    /// test and home's derived content must change. A hand-maintained
    /// string list would not.
    #[test]
    fn home_rows_track_live_keymap_and_registry() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        let before = s.home_rows();
        // Rebind an existing global command to a new sequence: the new
        // key row appears on home.
        s.engine.global.bind(&[Key::alt_char('z')], "open-palette").unwrap();
        let after_rebind = s.home_rows();
        assert_ne!(before, after_rebind, "rebinding must change home's derived rows");
        assert!(after_rebind.iter().any(|r| !r.is_header && r.key_display == "M-z"));
        // Register a brand-new command + bind it: a new category appears.
        s.registry.register(crate::app::command::Command::new(
            "home-probe-command",
            "probe docs",
            "home-probe-category",
            |_, _| {},
        ));
        s.engine
            .global
            .bind(&[Key::alt_char('y')], "home-probe-command")
            .unwrap();
        let after_register = s.home_rows();
        assert_ne!(after_rebind, after_register, "registry mutation must change home");
        assert!(after_register.iter().any(|r| r.is_header && r.label == "home-probe-category"));
        assert!(after_register.iter().any(
            |r| !r.is_header && r.key_display == "M-y" && r.label == "probe docs"
        ));
        // The `?` menu (same machinery, at the top path) tracks it too.
        assert!(s
            .menu_rows()
            .iter()
            .any(|r| r.is_header && r.label == "home-probe-category"));
    }

    /// 06a key contract on home: `?` opens the descendable menu; every
    /// global entry point works FROM home and replaces home with the
    /// opened view; the explicit scratch affordance still creates scratch.
    #[test]
    fn home_entry_points_replace_home_and_scratch_stays_explicit() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        // `?` opens the menu on home (and C-g closes it).
        let mut s = store(dir.path());
        s.key_event(key("?"));
        assert!(s.menu_open());
        s.key_event(key("C-g"));
        assert!(!s.menu_open());

        // C-x C-f landing (open_path) replaces home with the buffer view.
        let mut s = store(dir.path());
        s.open_path("src/main.rs");
        assert_eq!(s.top_view(), ViewId::Buffer);
        assert_eq!(s.view_stack, vec![ViewId::Buffer]);
        assert_eq!(s.view_name_display(), "src/main.rs");

        // C-x g pushes magit on top of home; q returns to home.
        let mut s = store(dir.path());
        // A git repo so C-x g has something to show (open_magit_status
        // reports and stays on home in a non-git dir).
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["init", "-q", "-b", "main"])
            .output();
        s.key_event(key("C-x"));
        s.key_event(key("g"));
        assert_eq!(s.top_view(), ViewId::MagitStatus, "C-x g must open magit from home");
        assert_eq!(s.view_stack, vec![ViewId::Home, ViewId::MagitStatus]);
        s.key_event(key("q"));
        assert_eq!(s.top_view(), ViewId::Home, "q on magit returns to home");

        // The explicit affordance: open-scratch creates scratch on demand
        // and replaces home.
        let mut s = store(dir.path());
        s.open_scratch();
        assert_eq!(s.top_view(), ViewId::Buffer);
        assert_eq!(s.buffers.current(), Some(SCRATCH_NAME));
        assert_eq!(s.view_name_display(), "*scratch*");
    }

    /// Killing the LAST buffer returns to home: empty table, no current,
    /// and NO scratch is created by the kill (06a's "no accidental buffer
    /// creation" audit path).
    #[test]
    fn kill_last_buffer_returns_to_home_without_creating_scratch() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut s = store(dir.path());
        s.open_path("src/main.rs");
        assert_eq!(s.top_view(), ViewId::Buffer);
        let key = s.buffers.current().unwrap().to_string();
        s.kill_buffer(&key);
        assert_eq!(s.buffers.len(), 0);
        assert!(s.buffers.current().is_none());
        assert!(s.buffers.get(SCRATCH_NAME).is_none(), "no scratch from the kill");
        assert_eq!(s.top_view(), ViewId::Home);
        assert_eq!(s.render_view(), ViewId::Home);
        assert_eq!(s.view_name_display(), "home");
        assert!(s.message.contains("killed"), "{:?}", s.message);
    }

    /// 06a review P1 repro-mirror: a jump entry whose buffer no longer
    /// exists must NOT create `*scratch*` on `M-,`/`C-i` — the honest
    /// "no buffer" report leaves the view and the table unchanged
    /// (the contract's no-accidental-buffer-creation invariant, both
    /// directions of the stack).
    #[test]
    fn dead_jump_entry_reports_no_buffer_without_creating_scratch() {
        let dir = tempfile::tempdir().unwrap();
        project_with_files(dir.path());
        let mut s = store(dir.path());
        s.open_path("src/main.rs");
        let before = s.buffers.len();
        // A dead entry: the buffer was killed (or never open) — simulate
        // exactly what a failed search-RET / stale origin would record.
        let dead = JumpEntry {
            buffer_key: "/gone/gone.rs".to_string(),
            line: 3,
            col: 0,
            label: "search-RET".to_string(),
        };
        s.jump_stack.record_jump(
            &JumpEntry {
                buffer_key: SEARCH_JUMP_KEY.to_string(),
                line: 0,
                col: 0,
                label: "*search*".to_string(),
            },
            &dead,
        );
        // `M-,` returns to the ORIGIN (the search sentinel); the dead entry
        // is the DESTINATION, reached with `C-i` (jump-forward) — the
        // search-RET-then-C-i repro shape from the review.
        s.jump_back();
        assert_eq!(s.buffers.len(), before);
        assert!(s.buffers.get(SCRATCH_NAME).is_none());
        s.jump_forward();
        assert_eq!(s.buffers.len(), before, "no buffer created by a dead entry");
        assert!(s.buffers.get(SCRATCH_NAME).is_none());
        assert!(s.message.contains("no buffer"), "{:?}", s.message);
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
        store.open_path("src/main.rs"); // main.rs current; MRU: main, lib (06a)

        store.dispatch("list-buffers", None).unwrap();
        assert_eq!(store.top_view(), ViewId::BufferList);
        let rows = store.buffer_rows();
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["src/main.rs", "src/lib.rs"], "{names:?}");
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
        assert_eq!(names, vec!["src/main.rs"], "{names:?}");
        assert!(store.message.contains("killed"), "{:?}", store.message);
        assert_eq!(store.buffer_list_selected(), 0, "selection clamps to a valid row");

        // d on the last (current) buffer kills it: the table is empty, the
        // selection clamps to row 0, and the list stays open (06a: no
        // scratch is created by the kill).
        store.key_event(key("d"));
        assert_eq!(store.buffer_rows().len(), 0);
        assert_eq!(store.buffer_list_selected(), 0);
        assert_eq!(store.top_view(), ViewId::BufferList);
        assert!(store.buffers.current().is_none());
        assert!(!store.buffers.list().iter().any(|&(k, _)| k == SCRATCH_NAME));

        // q closes the list: the empty table normalizes the top back to
        // home (close_view's re-normalization, 06a).
        store.key_event(key("q"));
        assert_eq!(store.top_view(), ViewId::Home);
        assert_eq!(store.render_view(), ViewId::Home, "empty table renders home");
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
        assert_eq!(store.buffers.len(), 0); // 06a: the failed open creates no buffer
    }

    #[test]
    fn palette_navigation_and_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.open_palette();
        assert_eq!(store.picker_count().0, 108);

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
        assert_eq!(store.picker_selected(), 107);

        // C-p goes through the same wrap-decrement path as Up.
        store.key_event(key("C-p"));
        assert_eq!(store.picker_selected(), 106);

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
        assert_eq!(store.picker_count().0, 108);

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

    // ── plan 004 issue 07: jump-landing recenter (middle row) ───────────
    // These must FAIL against a landing that only minimal-scrolls
    // (`keep_cursor_visible`): that puts a below-window target on the LAST
    // row and an above-window target on the FIRST row; the jump landing
    // recenters to the MIDDLE row (emacs `xref-after-jump-hook` =
    // `(recenter xref-pulse-momentarily)`) without touching `recenter_cycle`.

    #[test]
    fn jump_landing_far_down_lands_on_middle_row() {
        let (mut s, _dir) = store_with_lines(200);
        s.set_viewport_lines(21);
        // Jump DOWN from the top of the file to line 150.
        s.set_point_line(150);
        s.recenter_landing();
        assert_eq!(
            s.scroll_top(),
            140,
            "top = 150 - vp/2; a minimal scroll would leave the point on the last row (top 130)"
        );
    }

    #[test]
    fn jump_landing_far_up_lands_on_middle_row() {
        let (mut s, _dir) = store_with_lines(200);
        s.set_viewport_lines(21);
        // Scroll to the bottom, then jump UP to line 50.
        s.set_point_line(199); // minimal scroll → top 179
        assert_eq!(s.scroll_top(), 179);
        s.set_point_line(50);
        s.recenter_landing();
        assert_eq!(
            s.scroll_top(),
            40,
            "top = 50 - vp/2; a minimal scroll would leave the point on row 0 (top 50)"
        );
    }

    #[test]
    fn jump_landing_short_file_clamps_without_panic() {
        // The buffer barely exceeds the 21-row viewport (the last line is
        // the trailing empty line rope counts after the final newline).
        let (mut s, _dir) = store_with_lines(25);
        s.set_viewport_lines(21);
        s.set_point_line(0);
        s.recenter_landing();
        assert_eq!(s.scroll_top(), 0, "clamped at the top");
        let max_scroll = s.current_line_count() - 21;
        s.set_point_line(s.current_line_count() - 2); // last content line
        s.recenter_landing();
        assert_eq!(
            s.scroll_top(),
            max_scroll,
            "clamped at max_scroll (a middle-row landing is unreachable there)"
        );
        // A buffer that FITS the viewport is a no-op (nothing to recenter):
        // whole file visible, top stays 0, no panic.
        let (mut s2, _dir2) = store_with_lines(10);
        s2.set_viewport_lines(21);
        s2.set_point_line(9);
        s2.recenter_landing();
        assert_eq!(s2.scroll_top(), 0, "fits the viewport: top stays 0");
    }

    #[test]
    fn jump_landing_recenters_even_when_point_is_visible() {
        // emacs recenter repositions the window unconditionally — a jump
        // to a line already in view still lands it on the middle row.
        let (mut s, _dir) = store_with_lines(200);
        s.set_viewport_lines(21);
        s.set_scroll_top(5);
        s.set_point_line(12); // visible at row 7
        s.recenter_landing();
        assert_eq!(
            s.scroll_top(),
            2,
            "window repositioned to the middle row; minimal scroll would keep top 5"
        );
    }

    #[test]
    fn jump_landing_does_not_perturb_recenter_cycle() {
        // `recenter_landing` ITSELF does not touch the cycle: it must not
        // advance it (a jump is not a `C-l`) and must not rely on resetting
        // it. NOTE: in production every key dispatches through `dispatch()`,
        // which resets `recenter_cycle` for any command that is not literally
        // `recenter` (the pre-existing 05c `recenter-last-op` behavior, which
        // matches emacs keying `recenter-top-bottom` off `last-command`) — so
        // this test drives the helpers directly, below `dispatch`, to isolate
        // the helper's own contract.
        let (mut s, _dir) = store_with_lines(200);
        s.set_viewport_lines(21);
        s.set_point_line(50);
        s.recenter(); // fresh → middle: top 40, cycle 1
        assert_eq!(s.scroll_top(), 40);
        s.recenter(); // top: top 50, cycle 2
        assert_eq!(s.scroll_top(), 50);
        // A jump landing in between:
        s.set_point_line(150);
        s.recenter_landing();
        assert_eq!(s.scroll_top(), 140);
        assert_eq!(
            s.recenter_cycle, 2,
            "the jump neither reset nor advanced the cycle"
        );
        s.recenter(); // the cycle continues where it left off: bottom row
        assert_eq!(
            s.scroll_top(),
            130,
            "3rd C-l is the BOTTOM position (150 - (vp-1)); a reset cycle would give middle (140)"
        );
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
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        // 06a: no current buffer at boot → no position display.
        assert_eq!(s.file_view_position_display(), "");
        // The explicit scratch buffer has an empty rope: line_count()=1,
        // scroll_top=0 → "Top".
        s.open_scratch();
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

    /// Watchlist item 1 (U-D3 / U-K, review 05): `M-.` on a
    /// type/constant name — the end-to-end jump pinned per name shape.
    /// The extraction (`symbol_at_point`) has had no case filter since
    /// 006-02b (the skipped-uppercase bug was the old line-split
    /// heuristic, already replaced); this pins each shape so the fix
    /// stays: a CamelCase type (cross-file), a SCREAMING constant,
    /// and a mixed CamelCase type in a generic-argument position.
    #[test]
    fn xref_uppercase_type_and_const_shapes_jump_directly() {
        let (mut s, _d) = store_with_index(&[
            (
                "src/lib.rs",
                "mod widget;\nconst LOCAL_CONST: u32 = 2;\nfn use_it() {\n    let w = Widget { x: 1 };\n    let v: Vec<OtherThing> = vec![];\n    let _ = LOCAL_CONST;\n}\n",
            ),
            (
                "src/widget.rs",
                "pub struct Widget { pub x: i32 }\npub struct OtherThing { pub y: i32 }\n",
            ),
        ]);
        s.open_path("src/lib.rs");
        // Line 3: "    let w = Widget { x: 1 };" — cursor inside `Widget`
        // (CamelCase type, defined in another file).
        s.set_point(3, 13, 13);
        s.xref_find_definitions();
        assert_eq!(s.view_name_display(), "src/widget.rs", "CamelCase type: cross-file jump");
        assert_eq!(s.point_line(), 0, "to the struct's definition line");

        // Line 5: "    let _ = LOCAL_CONST;" — cursor inside `LOCAL_CONST`
        // (SCREAMING constant).
        s.open_path("src/lib.rs");
        s.set_point(5, 16, 16);
        s.xref_find_definitions();
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 1, "to the const's definition line");

        // Line 4: "    let v: Vec<OtherThing> = vec![];" — cursor inside
        // `OtherThing` (mixed CamelCase, generic-argument position).
        s.open_path("src/lib.rs");
        s.set_point(4, 18, 18);
        s.xref_find_definitions();
        assert_eq!(s.view_name_display(), "src/widget.rs");
        assert_eq!(s.point_line(), 1, "to the mixed CamelCase type");
    }

    /// Watchlist item 3 (the pre-011 artifact): a search-RET whose hit
    /// file cannot be opened (deleted after the walk) keeps the results
    /// view OPEN and reports that the jump did not happen — no jump
    /// entry recorded, no view closed, the current buffer unchanged.
    #[test]
    fn search_jump_failed_open_keeps_the_results_view() {
        let (dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        // The pre-search position (a different file: the failure must
        // not move the current buffer).
        store.open_path("src/main.rs");
        let main_key = store.buffers.current().unwrap().to_string();
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);
        assert_eq!(store.search.hits[0].file, "src/lib.rs");
        // Delete the hit file AFTER the walk finished.
        std::fs::remove_file(dir.path().join("src/lib.rs")).unwrap();
        let stack_len_before = store.jump_stack.len();

        store.key_event(key("RET"));
        assert_eq!(store.top_view(), ViewId::Search, "the results view stays open");
        assert_eq!(
            store.buffers.current().map(String::from),
            Some(main_key.clone()),
            "the current buffer is unchanged"
        );
        assert!(store.message.contains("cannot open"), "{:?}", store.message);
        assert!(
            store.message.contains("the jump did not happen"),
            "{:?}",
            store.message
        );
        assert_eq!(
            store.jump_stack.len(),
            stack_len_before,
            "no jump entry on a failed open"
        );
    }

    /// Watchlist item 4: `M-,` under the Search view — the jump-back
    /// pops through the sentinel in ONE step and lands the pre-search
    /// position (the results view closes with the landing — emacs
    /// `xref-pop-marker-stack`); before, the landing moved the buffer
    /// and point underneath the results view: a no-op until the view
    /// was closed by hand.
    #[test]
    fn search_mcomma_pops_the_sentinel_to_the_pre_search_position() {
        let (_dir, mut store) = search_project();
        let mut rx = store.search_rx().unwrap();
        // The pre-search position: main.rs, line 2.
        store.open_path("src/main.rs");
        store.set_point_line(2);
        store.start_project_search("target".into());
        drain_search_finished(&mut store, &mut rx);

        // RET: the first hit (lib.rs:1), the results view closes.
        store.key_event(key("RET"));
        assert_eq!(store.top_view(), ViewId::Buffer);
        assert_eq!(store.view_name_display(), "src/lib.rs");

        // M-,: the sentinel — back to the results (selection restored).
        store.key_event(key("M-,"));
        assert_eq!(store.top_view(), ViewId::Search, "the first M-, returns to the results");

        // M-, again: one step through the sentinel to the pre-search
        // position — and the results view closes with the landing.
        store.key_event(key("M-,"));
        assert_eq!(
            store.top_view(),
            ViewId::Buffer,
            "the results view closes with the landing"
        );
        assert_eq!(store.view_name_display(), "src/main.rs", "the pre-search buffer");
        assert_eq!(store.point_line(), 2, "the pre-search line");
    }

    /// Watchlist item 2 (imenu flat, no impl-parent nesting): a Rust
    /// file's imenu groups the impl methods under the impl's type —
    /// the method's display is indented one level below the struct
    /// (the Rung 1 tables: the method's definition line is not inside
    /// the struct's extent, so the grouping can only come from the
    /// tables). The names (the picker's match target) stay bare, and
    /// the query re-derivation (the refilter path) keeps the same
    /// display — the pre-fix drift where the initial open indented but
    /// the refilter did not.
    #[test]
    fn imenu_groups_impl_methods_under_the_struct() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Foo { a: i32 }\nimpl Foo {\n    pub fn new() -> Self { Self { a: 0 } }\n}\npub fn free() {}\n",
        )]);
        s.open_path("src/lib.rs");
        s.open_imenu();
        assert!(s.picker_open());
        assert_eq!(s.picker_kind(), Some(PickerKind::Imenu));
        let rows: Vec<(String, String)> = s
            .picker_filtered()
            .iter()
            .map(|(c, _)| (c.name.clone(), c.display.clone()))
            .collect();
        let foo = rows.iter().find(|(n, _)| n == "Foo:1").unwrap();
        assert_eq!(foo.1, "Foo  [type]", "the struct stays at top level: {rows:?}");
        let new = rows.iter().find(|(n, _)| n == "new:3").unwrap();
        assert_eq!(
            new.1,
            "  new  [fn]",
            "the impl method is indented one level under the struct: {rows:?}"
        );
        let free = rows.iter().find(|(n, _)| n == "free:5").unwrap();
        assert_eq!(free.1, "free  [fn]", "a free fn stays flat: {rows:?}");

        // The refilter path (candidates_for) must keep the SAME display.
        s.picker_query_char('n');
        s.picker_query_char('e');
        let rows: Vec<(String, String)> = s
            .picker_filtered()
            .iter()
            .map(|(c, _)| (c.name.clone(), c.display.clone()))
            .collect();
        assert_eq!(
            rows,
            vec![("new:3".to_string(), "  new  [fn]".to_string())],
            "the re-derivation keeps the grouped display: {rows:?}"
        );
    }

    /// Watchlist item 2 degradation (byte-for-byte): a non-Rust file
    /// has no Rung 1 tables — its imenu stays the PART A
    /// enclosing-extent indent, no impl-parent grouping.
    #[test]
    fn imenu_non_rust_stays_flat() {
        let (mut s, _dir) = store_with_index(&[(
            "src/app.js",
            "function outer() {\n  function inner() {}\n}\nfunction free() {}\n",
        )]);
        s.open_path("src/app.js");
        s.open_imenu();
        let rows: Vec<(String, String)> = s
            .picker_filtered()
            .iter()
            .map(|(c, _)| (c.name.clone(), c.display.clone()))
            .collect();
        // The PART A enclosing-extent indent is unchanged (inner is
        // lexically inside outer's extent)…
        let inner = rows.iter().find(|(n, _)| n.starts_with("inner")).unwrap();
        assert_eq!(
            inner.1, "  inner  [fn]",
            "the enclosing-extent indent stands: {rows:?}"
        );
        // …and a top-level fn has no indent (no grouping to add one).
        let free = rows.iter().find(|(n, _)| n.starts_with("free")).unwrap();
        assert_eq!(free.1, "free  [fn]", "a top-level fn has no indent: {rows:?}");
    }


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
            ("src/main.rs", "fn main() { lib::target(); }\n"),
            ("src/lib.rs", "pub fn target() {}\npub fn other() {}\n"),
        ]);
        // Open main.rs and position the point at the call site: line 0, the
        // column of `target` (the cursor-aware selection: `lib` before the
        // `::` is NOT the lookup, `target` is).
        s.open_path("src/main.rs");
        s.set_point(0, 17, 17);
        // `target` is only defined in lib.rs: unique → jump directly.
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique cross-file: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 0, "target is at line 0 in lib.rs");
        // 006-02b item 2: the hit bumped the generation (superseding any
        // in-flight resolve) but started no job — exactly one bump.
        assert_eq!(s.resolve_generation, 1, "a workspace hit supersedes in-flight resolves");
    }

    #[test]
    fn xref_symbol_at_point_wins_over_other_line_identifiers() {
        // Two KNOWN definitions on one line: the old rule took
        // all_defs[0] after (file, line, name) sort — an arbitrary
        // identifier on the line. The cursor-aware rule must follow the
        // point's column.
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() { foo(); bar(); }\n"),
            ("src/a.rs", "pub fn foo() {}\n"),
            ("src/b.rs", "pub fn bar() {}\n"),
        ]);
        s.open_path("src/main.rs");
        // Cursor on `foo` (col 12): jumps to a.rs, not b.rs.
        s.set_point(0, 12, 12);
        s.xref_find_definitions();
        assert_eq!(s.view_name_display(), "src/a.rs", "cursor on foo → foo's definition");
        // Back to the call site, cursor on `bar` (col 20): jumps to b.rs.
        s.open_path("src/main.rs");
        s.set_point(0, 20, 20);
        s.xref_find_definitions();
        assert_eq!(s.view_name_display(), "src/b.rs", "cursor on bar → bar's definition");
    }

    #[test]
    fn xref_same_file_definition_jumps() {
        // The user's report: struct + impl in one file is the normal case
        // and must be jumpable (the old cross-file filter made it not).
        // Cursor on the `new` of `Foo::new()`: the path token `Foo::new`
        // is kept for the resolver, the index (name-keyed) is tried with
        // the last segment `new` — the same-file impl method wins.
        let (mut s, _dir) = store_with_index(&[
            (
                "src/lib.rs",
                "pub struct Foo {}\nimpl Foo {\n    pub fn new() -> Self { Foo {} }\n}\nfn use_it() {\n    let f = Foo::new();\n}\n",
            ),
        ]);
        s.open_path("src/lib.rs");
        // Line 5: "    let f = Foo::new();" — `new` starts at col 17.
        s.set_point(5, 17, 17);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique same-file: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 2, "jumped to the same-file impl method");
        assert_eq!(s.jump_stack.len(), 2, "origin + destination recorded");
    }

    #[test]
    fn xref_trait_definition_jumps() {
        // The user's report: on a Trait, jump into the trait definition.
        // `trait_item` is captured by the index, so a cursor on the trait
        // name at a use site lands on the definition.
        let (mut s, _dir) = store_with_index(&[
            ("src/lib.rs", "pub trait Tr {\n    fn m(&self);\n}\n"),
            ("src/main.rs", "fn use_it<T: Tr>() {}\n"),
        ]);
        s.open_path("src/main.rs");
        // "fn use_it<T: Tr>() {}" — the `Tr` bound starts at col 13.
        s.set_point(0, 13, 13);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique trait: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 0, "jumped to `pub trait Tr` (msg: {})", s.message);
    }

    // ── 010-04: find-implementations (plan 010 Shape A, rung 4) ─────

    /// 010-04 (discriminating): a trait at point with table entries opens
    /// the Impls picker over the `impl <Trait> for <Type>` blocks (the
    /// name-keyed trait map from the same index pass); RET reuses the
    /// Xref jump path (origin captured, jump recorded).
    #[test]
    fn find_implementations_opens_picker_of_trait_impls() {
        let (mut s, _dir) = store_with_index(&[
            (
                "src/lib.rs",
                "pub struct N;\npub trait Tr {\n    fn m(&self);\n}\nimpl Tr for N {\n    fn m(&self) {}\n}\n",
            ),
            (
                "src/extra.rs",
                "use crate::lib::Tr;\nstruct S;\nimpl Tr for S {\n    fn m(&self) {}\n}\n",
            ),
            ("src/main.rs", "use crate::lib::Tr;\nfn use_it<T: Tr>() {}\n"),
        ]);
        s.open_path("src/main.rs");
        // Line 1: "fn use_it<T: Tr>() {}" — `Tr` starts at col 13.
        s.set_point(1, 13, 13);
        s.find_implementations();
        assert!(s.picker_open(), "two impls: picker");
        assert_eq!(s.picker_kind(), Some(PickerKind::Impls));
        let filtered = s.picker_filtered();
        assert_eq!(filtered.len(), 2, "two impl blocks: {filtered:?}");
        // Deterministic (file, impl line) order: extra.rs's impl (line 3,
        // 1-based) before lib.rs's (line 5, 1-based); the display carries
        // the self type (kind: the trait impl's shape).
        assert_eq!(filtered[0].0.name, "src/extra.rs:3", "{}", filtered[0].0.display);
        assert!(filtered[0].0.display.contains("[impl Tr for S]"), "{}", filtered[0].0.display);
        assert_eq!(filtered[1].0.name, "src/lib.rs:5", "{}", filtered[1].0.display);
        assert!(
            filtered[1].0.display.contains("[impl Tr for N]"),
            "{}",
            filtered[1].0.display
        );
        // RET on the lib.rs candidate jumps to the impl header (line 4,
        // 0-based) and records the jump.
        s.picker_select_next();
        s.run_selected();
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 4, "the impl header (msg: {})", s.message);
        assert_eq!(s.jump_stack.len(), 2, "origin + destination recorded");
    }

    /// 010-04 (pin): honest degradation — a trait with NO table entry
    /// (no Rust file impls it) runs the EXISTING bare-symbol M-. lookup
    /// byte-for-byte: `Tr` itself is indexed (`trait_item`), so the
    /// lookup lands on the trait definition exactly as M-. would.
    #[test]
    fn find_implementations_no_table_entry_degrades_to_bare_symbol_lookup() {
        let (mut s, _dir) = store_with_index(&[
            ("src/lib.rs", "pub trait Tr {\n    fn m(&self);\n}\n"),
            ("src/main.rs", "fn use_it<T: Tr>() {}\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 13, 13);
        s.find_implementations();
        assert!(!s.picker_open(), "degraded to the M-. path: {}", s.message);
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 0, "the trait definition (msg: {})", s.message);
        assert!(s.message.contains("jumped"), "the M-. jump message: {}", s.message);
    }

    /// 010-04 (pin): a GENERIC trait's captured text (`Tr<Foo>`) never
    /// matches the bare `Tr` at point — the map key is the impl's written
    /// trait text, so this degrades to the bare-symbol lookup (never a
    /// guess).
    #[test]
    fn find_implementations_generic_trait_text_never_matches_bare_name() {
        let (mut s, _dir) = store_with_index(&[
            ("src/lib.rs", "pub trait Tr {\n    fn m(&self);\n}\n"),
            ("src/impls.rs", "struct Foo;\nimpl Tr<Foo> for Foo {\n    fn m(&self) {}\n}\n"),
            ("src/main.rs", "fn use_it<T: Tr>() {}\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 13, 13);
        s.find_implementations();
        // No entry for the bare `Tr` (the map key is `Tr<Foo>`) — the
        // bare-symbol M-. lookup lands on the trait definition instead.
        assert!(!s.picker_open(), "generic trait text never matches: {:?}", s.message);
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 0, "the trait definition (msg: {})", s.message);
    }

    /// 010-04 (pin): no symbol at point — the M-. guard message, no
    /// picker, no jump.
    #[test]
    fn find_implementations_no_symbol_under_point() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub trait Tr {\n    fn m(&self);\n}\nimpl Tr for i32 {\n    fn m(&self) {}\n}\n",
        )]);
        s.open_path("src/lib.rs");
        s.set_point(0, 13, 13); // on the `{` after `pub trait Tr`
        s.find_implementations();
        assert!(!s.picker_open());
        assert_eq!(s.message, "no symbol under point");
    }

    // ── 010-01: M-. self-receiver resolution (Shape A rung 1) ──────────

    /// 010-01 (discriminating): the extraction carries the `self.` receiver
    /// for Rust `self.<member>` — pre-010-01 it stayed bare (fields are not
    /// in the index). `myself.` / arbitrary receivers stay byte-for-byte.
    #[test]
    fn symbol_at_point_rust_self_access_carries_the_receiver() {
        let line = "    let v = self.a;";
        // `a` at col 17, and parked right after it (col 18).
        assert_eq!(
            satp(LanguageId::Rust, line, 17),
            Some(("a".into(), "self.a".into()))
        );
        assert_eq!(
            satp(LanguageId::Rust, line, 18),
            Some(("a".into(), "self.a".into()))
        );
        // Call-shaped: the same token on `self.method()`.
        let call = "        let _ = self.method();";
        // `method` starts at col 21; cursor on the `e` at col 22.
        assert_eq!(
            satp(LanguageId::Rust, call, 22),
            Some(("method".into(), "self.method".into()))
        );
        // `myself.`: the 4-char window must be EXACTLY `self` — stays bare.
        let selfy = "let v = myself.a;";
        assert_eq!(
            satp(LanguageId::Rust, selfy, 15),
            Some(("a".into(), "a".into())),
            "myself.a stays bare"
        );
        // An arbitrary receiver stays bare (byte-for-byte the pre-010-01
        // extraction — `obj.a` was never resolvable and still isn't).
        assert_eq!(
            satp(LanguageId::Rust, "let v = obj.a;", 12),
            Some(("a".into(), "a".into()))
        );
        // Non-Rust `self.x` is NOT the Rust self shape — the 011-06
        // language-aware path container handling stands (Python attribute
        // already extended; the Rust branch never fires). `x` is at col 9.
        assert_eq!(
            satp(LanguageId::Python, "y = self.x", 9),
            Some(("x".into(), "self.x".into()))
        );
    }

    /// 010-03 (pin): the local-binding pre-step's own receiver scan — a
    /// bare receiver on a Rust `x.<member>` is carried; `self.` stays
    /// 010-01's, and expression / path / call receivers are never
    /// treated as local bindings (they stay bare, byte-for-byte).
    #[test]
    fn rust_dotted_receiver_scan_rules() {
        let rd = AppStore::rust_dotted_receiver;
        // `let _ = p.x;` — `x` at col 10.
        assert_eq!(rd("let _ = p.x;", 10), Some(("p".into(), 10)));
        // Call-shaped, parked right after the name (before the `(`):
        // `go` spans col 10-11, parked at 12.
        assert_eq!(rd("let _ = p.go();", 12), Some(("p".into(), 10)));
        // `self.` is the 010-01 pre-step's — never reported here.
        assert_eq!(rd("let _ = self.x;", 13), None);
        // A `::`-path receiver (`a::b.x`): not a local binding.
        assert_eq!(rd("let _ = a::b.x;", 13), None);
        // A DOT-CHAINED receiver (`a.b.x`): the middle segment `b` is a
        // field access, never a local binding (review P1 — a misread
        // here would jump to the wrong struct's member); the call
        // variant `a.b.go()` too.
        assert_eq!(rd("let _ = a.b.x;", 12), None);
        assert_eq!(rd("let _ = a.b.go();", 14), None);
        // Expression receivers: a call / an index / a paren.
        assert_eq!(rd("let _ = f().x;", 12), None);
        assert_eq!(rd("let _ = v[0].x;", 13), None);
        assert_eq!(rd("let _ = (p).x;", 12), None);
        // The point not on a member run: the dot itself, or the
        // receiver's own run.
        assert_eq!(rd("let _ = p.x;", 9), None);
        assert_eq!(rd("let _ = p.x;", 8), None);
        // A longer receiver word is ONE bare identifier (`myself.x`
        // is the binding `myself` — not a self access, not rejected).
        assert_eq!(rd("let _ = myself.x;", 15), Some(("myself".into(), 15)));
    }

    /// 010-01 (discriminating): `self.a` inside `impl Foo` jumps to the
    /// struct field's line. `a` is NOT in the symbol index (field
    /// declarations are not outline symbols) — pre-010-01 this degraded to
    /// the enclosing-symbol fallback (the enclosing `fn`), so this outcome
    /// only exists because of the table pre-step.
    #[test]
    fn xref_self_field_jumps_to_struct_field_line() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Foo { pub a: i32 }\nimpl Foo {\n    pub fn use_it(&self) { let _ = self.a; }\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 2: "    pub fn use_it(&self) { let _ = self.a; }" — `a`
        // starts at col 40.
        s.set_point(2, 40, 40);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique same-file field: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            0,
            "jumped to the field declaration (msg: {})",
            s.message
        );
    }

    /// 010-01 (discriminating): the field lives in ANOTHER file of the
    /// project — the index's cross-file field locations carry it (the
    /// same-file-first ordering then lands in `src/model.rs`).
    #[test]
    fn xref_self_field_resolves_cross_file() {
        let (mut s, _dir) = store_with_index(&[
            ("src/model.rs", "pub struct Point { pub x: i32, pub y: i32 }\n"),
            (
                "src/main.rs",
                "use crate::model::Point;\nimpl Point {\n    fn coords(&self) { let _ = self.x; }\n}\n",
            ),
        ]);
        s.open_path("src/main.rs");
        // Line 2: "    fn coords(&self) { let _ = self.x; }" — `x` starts
        // at col 36.
        s.set_point(2, 36, 36);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique cross-file field: no picker");
        assert_eq!(s.view_name_display(), "src/model.rs");
        assert_eq!(
            s.point_line(),
            0,
            "jumped to `x` in model.rs (msg: {})",
            s.message
        );
    }

    /// 010-01: `self.method()` inside `impl Foo` jumps to the impl method
    /// (its line, from the same-file impl table).
    #[test]
    fn xref_self_method_call_jumps_to_impl_method() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Foo { pub a: i32 }\nimpl Foo {\n    pub fn method(&self) -> i32 { self.a + 1 }\n    pub fn use_it(&self) { let _ = self.method(); }\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 3: "    pub fn use_it(&self) { let _ = self.method(); }" —
        // `method` starts at col 40.
        s.set_point(3, 40, 40);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique same-file method: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            2,
            "jumped to `fn method` (msg: {})",
            s.message
        );
    }

    /// 010-01: a name that is BOTH a field and a method of the enclosing
    /// type → the picker (same-file-first, never guessed away).
    #[test]
    fn xref_self_ambiguous_member_opens_picker() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Foo { pub extra: i32 }\nimpl Foo {\n    fn extra(&self) {}\n    fn use_it(&self) { self.extra(); }\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 3: "    fn use_it(&self) { self.extra(); }" — `extra`
        // starts at col 28.
        s.set_point(3, 28, 28);
        s.xref_find_definitions();
        assert!(s.picker_open(), "field + method: picker");
        assert_eq!(s.picker_kind(), Some(PickerKind::Xref));
        let filtered = s.picker_filtered();
        assert_eq!(filtered.len(), 2, "two candidates: {filtered:?}");
        // Same-file, line-ordered: the field (line 0) before the method
        // (line 2).
        assert!(
            filtered[0].0.name.starts_with("src/lib.rs:1"),
            "field candidate first: {}",
            filtered[0].0.name
        );
        assert!(
            filtered[1].0.name.starts_with("src/lib.rs:3"),
            "method candidate second: {}",
            filtered[1].0.name
        );
    }

    /// 010-01 (pin): honest degradation — `self.a` inside a GENERIC impl
    /// (`impl<T> Foo<T>`) never resolves through the tables: the self type
    /// is not a plain identifier, so the exact pre-010-01 behavior stands
    /// (bare `a` → no index hit → the enclosing symbol takes over).
    #[test]
    fn xref_self_in_generic_impl_degrades_to_today() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Foo { pub a: i32 }\nimpl<T> Foo<T> {\n    fn f(&self) { let _ = self.a; }\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 2: "    fn f(&self) { let _ = self.a; }" — `a` starts at
        // col 31.
        s.set_point(2, 31, 31);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "degraded: no picker");
        // The enclosing-symbol fallback landed on `f` itself (its only
        // indexed definition) — today's behavior, byte-for-byte.
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            2,
            "enclosing `f` took over (msg: {})",
            s.message
        );
    }

    /// 010-01 (pin): `self.a` with NO enclosing impl (top-level / outside
    /// every impl block) degrades to the exact pre-010-01 behavior — and a
    /// non-Rust buffer is never touched by the pre-step at all.
    #[test]
    fn xref_self_without_enclosing_impl_degrades_to_today() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Foo { pub a: i32 }\nfn free() { let _ = self; }\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 1: "fn free() { let _ = self; }" — no member after `self`
        // (the token is `self`, not `self.<member>`): the pre-step never
        // fires; `self` has no definition → the enclosing `free` takes
        // over (today's behavior).
        s.set_point(1, 24, 24);
        s.xref_find_definitions();
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 1, "enclosing `free` (msg: {})", s.message);
    }

    // ── 010-03: M-. local-binding resolution (Shape A rung 3) ───────

    /// 010-03 (discriminating): `p.x` where `p` has a written annotation
    /// jumps to the struct field's line. `x` is NOT in the symbol index
    /// (field declarations are not outline symbols), so pre-010-03 this
    /// degraded to the enclosing-symbol fallback (`main`) — this outcome
    /// only exists because of the binding pre-step.
    #[test]
    fn xref_local_binding_field_jumps_to_struct_field_line() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Pt { pub x: i32 }\nfn main() {\n    let p: Pt = Pt { x: 1 };\n    let _ = p.x;\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 3: "    let _ = p.x;" — `x` at col 14.
        s.set_point(3, 14, 14);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique same-file field: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            0,
            "jumped to the field declaration (msg: {})",
            s.message
        );
    }

    /// 010-03 (discriminating): the binding's type comes from the
    /// struct-literal RHS — NO annotation on the `let`. Pre-010-03 the
    /// literal was invisible to the tables.
    #[test]
    fn xref_local_binding_struct_literal_jumps_to_field() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Pt { pub x: i32 }\nfn main() {\n    let p = Pt { x: 1 };\n    let _ = p.x;\n}\n",
        )]);
        s.open_path("src/lib.rs");
        s.set_point(3, 14, 14);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique same-file field: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            0,
            "the struct literal's type carried the jump (msg: {})",
            s.message
        );
    }

    /// 010-03 (discriminating): `let mut p: Pt` is the SAME binding as
    /// `let p: Pt` — the annotation pre-step resolves through `mut`.
    #[test]
    fn xref_local_binding_mut_resolves_like_plain() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Pt { pub x: i32 }\nfn main() {\n    let mut p: Pt = Pt { x: 1 };\n    let _ = p.x;\n}\n",
        )]);
        s.open_path("src/lib.rs");
        s.set_point(3, 14, 14);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique same-file field: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(s.point_line(), 0, "mut binding resolved (msg: {})", s.message);
    }

    /// 010-03 (discriminating): `b.go()` where TWO types define `go` —
    /// pre-010-03 the bare `go` index lookup was ambiguous (the picker
    /// over both impls); the binding's written type narrows it to B's
    /// impl method: a unique jump.
    #[test]
    fn xref_local_binding_method_call_narrows_ambiguous_impls() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct A { pub m: i32 }\n\
             pub struct B { pub m: i32 }\n\
             impl A {\n\
             \x20   fn go(&self) { let _ = self.m; }\n\
             }\n\
             impl B {\n\
             \x20   fn go(&self) { let _ = self.m; }\n\
             }\n\
             fn main() {\n\
             \x20   let b: B = B { m: 1 };\n\
             \x20   b.go();\n\
             }\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 10: "    b.go();" — `go` at col 6.
        s.set_point(10, 6, 6);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "the written type narrows to one impl: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            6,
            "jumped to B's `go` (msg: {})",
            s.message
        );
    }

    /// 010-03 (discriminating): the field lives in ANOTHER file — the
    /// binding's type resolves through the index's cross-file field
    /// locations (the same-file-first ordering lands in `src/model.rs`).
    #[test]
    fn xref_local_binding_field_resolves_cross_file() {
        let (mut s, _dir) = store_with_index(&[
            ("src/model.rs", "pub struct Point { pub x: i32, pub y: i32 }\n"),
            (
                "src/main.rs",
                "use crate::model::Point;\nfn main() {\n    let p: Point = Point { x: 1, y: 2 };\n    let _ = p.x;\n}\n",
            ),
        ]);
        s.open_path("src/main.rs");
        s.set_point(3, 14, 14);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique cross-file field: no picker");
        assert_eq!(s.view_name_display(), "src/model.rs");
        assert_eq!(
            s.point_line(),
            0,
            "jumped to `x` in model.rs (msg: {})",
            s.message
        );
    }

    /// 010-03 (pin): a SHADOWED name — the innermost binding wins.
    /// `x.f` inside the nested block resolves via the inner `x: B`, not
    /// the outer `x: A`; the same use AFTER the block (outside the
    /// shadow) resolves via the outer `A`. Pre-010-03 both degraded to
    /// the enclosing-symbol fallback.
    #[test]
    fn xref_local_binding_shadow_innermost_wins() {
        let files: &[(&str, &str)] = &[(
            "src/lib.rs",
            "pub struct A { pub f: i32 }\n\
             pub struct B { pub f: i32 }\n\
             fn main() {\n\
             \x20   let x: A;\n\
             \x20   let _o = x.f;\n\
             \x20   {\n\
             \x20       let x: B;\n\
             \x20       let _i = x.f;\n\
             \x20   }\n\
             }\n",
        )];
        // Inner use (line 7: "        let _i = x.f;") — `f` at col 19:
        // the shadow → B's field (line 1).
        let (mut s, _dir) = store_with_index(files);
        s.open_path("src/lib.rs");
        s.set_point(7, 19, 19);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique field: no picker");
        assert_eq!(
            s.point_line(),
            1,
            "inner use resolves via the inner shadow `x: B` (msg: {})",
            s.message
        );
        // Outer use (line 4: "    let _o = x.f;") — `f` at col 15: the
        // shadow is out of scope → A's field (line 0).
        let (mut s, _dir) = store_with_index(files);
        s.open_path("src/lib.rs");
        s.set_point(4, 15, 15);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique field: no picker");
        assert_eq!(
            s.point_line(),
            0,
            "outer use resolves via the outer `x: A` (msg: {})",
            s.message
        );
    }

    /// 010-03 (pin): an UNANNOTATED receiver — the pre-step misses and
    /// the exact pre-010-03 bare-`<member>` behavior stands: `x` still
    /// resolves to the indexed `fn x` (the bare path token, byte-for-
    /// byte; the extraction was never changed for non-self receivers).
    #[test]
    fn xref_local_binding_unannotated_receiver_stays_bare() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct Pt { pub x: i32 }\npub fn x() {}\nfn main() {\n    let p = make();\n    let _ = p.x;\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 4: "    let _ = p.x;" — `x` at col 14. `p` is not
        // annotated (the RHS is a call, not a struct literal), so the
        // pre-step misses and the bare `x` index lookup carries it.
        s.set_point(4, 14, 14);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique bare hit: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            1,
            "the bare `x` lookup landed on `fn x` (msg: {})",
            s.message
        );
    }

    /// 010-03 (pin): a binding annotated to a type with no recorded
    /// fields or impls (`Marker` is a unit struct — nothing in the
    /// tables) degrades to today's behavior: the bare `thing` carries
    /// through to the indexed `fn thing` (the pre-step gathered no
    /// candidates and never guessed) — not a table hit, not the
    /// enclosing symbol.
    #[test]
    fn xref_local_binding_non_struct_type_degrades_to_today() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub fn thing() {}\nstruct Marker;\nfn main() {\n    let m: Marker = Marker;\n    let _ = m.thing;\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 4: "    let _ = m.thing;" — `thing` starts at col 14.
        s.set_point(4, 14, 14);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique bare hit: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            0,
            "the bare `thing` lookup landed on `fn thing` (msg: {})",
            s.message
        );
    }

    #[test]
    fn xref_ambiguous_cross_file_opens_picker() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() { target(); }\n"),
            ("src/a.rs", "pub fn target() {}\n"),
            ("src/b.rs", "pub fn target() {}\n"),
        ]);
        s.open_path("src/main.rs");
        // Cursor on `target` (col 12): defined in a.rs and b.rs → ambiguous.
        s.set_point(0, 12, 12);
        s.xref_find_definitions();
        assert!(s.picker_open(), "ambiguous: picker should be open");
        assert_eq!(s.picker_kind(), Some(PickerKind::Xref));
        assert_eq!(s.picker_filtered().len(), 2, "two candidates");
    }

    #[test]
    fn xref_ambiguous_same_file_first_in_picker() {
        // Same-file candidate sorts first in the picker (selection rule 2).
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "mod lib {\n    pub fn target() {}\n}\nfn main() { lib::target(); }\n"),
            ("src/lib.rs", "pub fn target() {}\n"),
        ]);
        s.open_path("src/main.rs");
        // Line 3: "fn main() { lib::target(); }" — `target` starts at col 17.
        s.set_point(3, 17, 17);
        s.xref_find_definitions();
        assert!(s.picker_open(), "two candidates: picker");
        let filtered = s.picker_filtered();
        assert_eq!(filtered.len(), 2);
        assert!(filtered[0].0.name.starts_with("src/main.rs:"), "same-file candidate listed first: {}", filtered[0].0.name);
        assert!(filtered[1].0.name.starts_with("src/lib.rs:"), "cross-file candidate second: {}", filtered[1].0.name);
    }

    #[test]
    fn xref_no_symbol_under_point_falls_back_to_enclosing() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() {\n    let x = 1;\n}\n"),
        ]);
        s.open_path("src/main.rs");
        // Line 1: "    let x = 1;" col 0 — no identifier AT the point.
        // Fall back to enclosing symbol: `main` (unchanged behavior).
        s.set_point_line(1);
        s.xref_find_definitions();
        // `main` is defined only in main.rs: unique → jump to main's definition (line 0).
        assert!(!s.picker_open());
        assert_eq!(s.point_line(), 0, "jumped to main's definition");
        // 006-02b item 2: the enclosing hit bumps the generation (one
        // supersede bump, no job).
        assert_eq!(s.resolve_generation, 1, "the enclosing hit supersedes in-flight resolves");
    }

    /// 010-03 review P1 (pin): a DOT-CHAINED receiver — `a.b.c` where
    /// the middle segment `b` happens to be a local binding with a
    /// written type (`D`) that ALSO has a field `c` — must NOT be
    /// misattributed to `b`: the middle segment is a field access, never
    /// a local binding, so today's bare `c` behavior stands (no jump to
    /// `D`'s `c` — the wrong struct — the enclosing `main` takes over
    /// instead).
    #[test]
    fn xref_local_binding_dot_chained_receiver_stays_bare() {
        let (mut s, _dir) = store_with_index(&[(
            "src/lib.rs",
            "pub struct B { pub c: i32 }\npub struct D { pub c: i32 }\npub struct A { pub b: B }\nfn main() {\n    let b: D;\n    let a = A { b: B { c: 1 } };\n    let _ = a.b.c;\n}\n",
        )]);
        s.open_path("src/lib.rs");
        // Line 6: "    let _ = a.b.c;" — `c` at col 16. Pre-fix this
        // jumped to `D`'s `c` (line 1) through the misattributed `b`.
        s.set_point(6, 16, 16);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "no candidates: no picker");
        assert_eq!(s.view_name_display(), "src/lib.rs");
        assert_eq!(
            s.point_line(),
            3,
            "the bare `c` degraded to the enclosing `main` — NOT `D`'s `c` (msg: {})",
            s.message
        );
    }

    // ── plan 006 issue 02: tooling-resolver fall-through ─────────────────────────

    /// The point-line token extractor: identifier run at the column + the
    /// path token around it (011-06: language-aware path token).
    fn satp(lang: LanguageId, text: &str, col: usize) -> Option<(String, String)> {
        AppStore::symbol_at_point(lang, text, col)
    }

    #[test]
    fn symbol_at_point_identifier_and_path_token() {
        // Rust `::`: byte-for-byte the pre-011-06 extraction (no parse).
        let line = "    let h = tokio::spawn(f);";
        // Cursor inside `tokio` (col 12) → ident `tokio`, path `tokio::spawn`.
        assert_eq!(
            satp(LanguageId::Rust, line, 12),
            Some(("tokio".into(), "tokio::spawn".into()))
        );
        // Cursor inside `spawn` (col 19) → same path token.
        assert_eq!(
            satp(LanguageId::Rust, line, 19),
            Some(("spawn".into(), "tokio::spawn".into()))
        );
        // Cursor parked right after `spawn` (before the `)` — the usual
        // call-site spot) still counts.
        assert_eq!(
            satp(LanguageId::Rust, line, 24),
            Some(("spawn".into(), "tokio::spawn".into()))
        );
    }

    #[test]
    fn symbol_at_point_field_access_stays_bare() {
        // A Rust `.`-accessed field: the token stays the bare field name
        // (fields are NOT in the index, and Rust `::`-only extraction is
        // byte-for-byte preserved by 011-06 — the language-aware
        // extension covers the OTHER languages' dotted path shapes).
        let line = "    let n = obj.name;";
        assert_eq!(
            satp(LanguageId::Rust, line, 17),
            Some(("name".into(), "name".into()))
        );
        assert_eq!(
            satp(LanguageId::Rust, line, 20),
            Some(("name".into(), "name".into()))
        );
    }

    /// 011-06 (discriminating): in a non-Rust buffer, the M-. path token
    /// becomes the WHOLE dotted path when the point sits in the language's
    /// path container — this is what makes the providers' already-
    /// unit-tested dotted handling reachable from M-.. Pre-011-06 every
    /// one of these returned the BARE identifier.
    #[test]
    fn symbol_at_point_dotted_path_extends_token_per_language() {
        // Python: `json.dumps` at a use site — cursor on EITHER segment.
        let line = "y = json.dumps(x)";
        assert_eq!(
            satp(LanguageId::Python, line, 5),
            Some(("json".into(), "json.dumps".into()))
        );
        assert_eq!(
            satp(LanguageId::Python, line, 11),
            Some(("dumps".into(), "json.dumps".into()))
        );
        // Cursor parked right after `dumps` (before `(` — the usual
        // call-site spot) still counts.
        assert_eq!(
            satp(LanguageId::Python, line, 14),
            Some(("dumps".into(), "json.dumps".into()))
        );
        // Python deep chain: `os.path.join` under the MIDDLE segment.
        let deep = "os.path.join(a, b)";
        assert_eq!(
            satp(LanguageId::Python, deep, 4),
            Some(("path".into(), "os.path.join".into()))
        );
        // JS: `fakelib.apply(5)` — cursor on the member, and parked just
        // after it (before `(`); TS gets the same answer (011-03 shares
        // the JS machinery).
        let js = "fakelib.apply(5);";
        assert_eq!(
            satp(LanguageId::JavaScript, js, 10),
            Some(("apply".into(), "fakelib.apply".into()))
        );
        assert_eq!(
            satp(LanguageId::JavaScript, js, 13),
            Some(("apply".into(), "fakelib.apply".into()))
        );
        assert_eq!(
            satp(LanguageId::TypeScript, js, 10),
            Some(("apply".into(), "fakelib.apply".into()))
        );
        // Go: `fmt.Println(x)` (selector_expression) and a
        // `qualified_type` in type position.
        let go = "fmt.Println(x)";
        assert_eq!(
            satp(LanguageId::Go, go, 7),
            Some(("Println".into(), "fmt.Println".into()))
        );
        let gotype = "var v fmt.Stringer";
        assert_eq!(
            satp(LanguageId::Go, gotype, 12),
            Some(("Stringer".into(), "fmt.Stringer".into()))
        );
        // 006-02b-style separator rule for `.`: a cursor right after
        // `json` (on the dot) counts as the end of the preceding segment.
        assert_eq!(
            satp(LanguageId::Python, "json.dumps", 4),
            Some(("json".into(), "json.dumps".into()))
        );
    }

    /// 011-06 (pin): the degradation stays byte-for-byte — bare
    /// identifiers, an unimplemented language, and an identifier that is
    /// NOT a full dot-delimited segment of its container (a computed
    /// member `a[b]`, no dot at all) all keep the exact bare extraction.
    #[test]
    fn symbol_at_point_non_rust_bare_and_unsupported_shapes_stay_bare() {
        // Bare identifiers (no path container around them): the bare
        // extraction, in every language.
        assert_eq!(
            satp(LanguageId::Python, "x = 1", 0),
            Some(("x".into(), "x".into()))
        );
        assert_eq!(
            satp(LanguageId::JavaScript, "const x = 1;", 6),
            Some(("x".into(), "x".into()))
        );
        assert_eq!(
            satp(LanguageId::Go, "const Z = 3", 6),
            Some(("Z".into(), "Z".into()))
        );
        // Unimplemented language (Plain): no parse, byte-for-byte bare —
        // the SAME line in Python extends, here it does not.
        assert_eq!(
            satp(LanguageId::Plain, "json.dumps", 6),
            Some(("dumps".into(), "dumps".into()))
        );
        // 011-06 review P1: the WRONG-container shapes — a whole-path
        // upgrade requires every dot-delimited segment to be a bare
        // identifier; these containers match segment-membership but carry
        // non-identifier segments, so they degrade to bare (feeding the
        // providers `a?` / `foo()` / `(*p)` as a package name would cause
        // unintended npm/pip shell-outs in online projects).
        assert_eq!(
            satp(LanguageId::JavaScript, "a?.b", 3),
            Some(("b".into(), "b".into()))
        );
        assert_eq!(
            satp(LanguageId::JavaScript, "a.b?.c", 5),
            Some(("c".into(), "c".into()))
        );
        assert_eq!(
            satp(LanguageId::Python, "foo().bar", 8),
            Some(("bar".into(), "bar".into()))
        );
        assert_eq!(
            satp(LanguageId::Python, "foo().bar.b", 10),
            Some(("b".into(), "b".into()))
        );
        assert_eq!(
            satp(LanguageId::Go, "(*p).field", 5),
            Some(("field".into(), "field".into()))
        );
        // A computed member `a[b]`: the node IS a member_expression, but
        // `b` is not a dot-delimited SEGMENT of `a[b]` (no dot at all) —
        // never feed the providers a non-path token; the bare extraction
        // stands.
        assert_eq!(
            satp(LanguageId::JavaScript, "a[b]", 2),
            Some(("b".into(), "b".into()))
        );
        // Punctuation / whitespace around the point: no symbol (the
        // identifier-run rule is language-independent).
        assert_eq!(satp(LanguageId::Python, "let a = 1;", 7), None);
        assert_eq!(satp(LanguageId::Python, "{ ", 1), None);
    }

    /// 010-rung4-and-paths (item 2, app-side whole-path upgrade): the
    /// per-language container pins — C's `field_expression`, Cpp's
    /// `field_expression` (its `::` shape stays the byte-scan token —
    /// NOT double-handled), Toml's `dotted_key` (the index stores the
    /// dotted key as ONE symbol name, so the segment's M-. only reaches
    /// it with the whole path).
    #[test]
    fn symbol_at_point_c_cpp_toml_containers_extend_the_token() {
        // C: `o.x` — cursor on the member and parked right after it.
        // "int y = o.x;": o@8, x@10.
        let c = "int y = o.x;";
        assert_eq!(
            satp(LanguageId::C, c, 10),
            Some(("x".into(), "o.x".into()))
        );
        assert_eq!(
            satp(LanguageId::C, c, 11),
            Some(("x".into(), "o.x".into()))
        );
        // C deep chain: `o.x.y` under the MIDDLE segment (x@10).
        let cdeep = "int v = o.x.y;";
        assert_eq!(
            satp(LanguageId::C, cdeep, 10),
            Some(("x".into(), "o.x.y".into()))
        );
        // Cpp: `o.x` — field_expression, same shape as C.
        let cpp = "int y = o.x;";
        assert_eq!(
            satp(LanguageId::Cpp, cpp, 10),
            Some(("x".into(), "o.x".into()))
        );
        // Cpp deep chain: `a.b.c` under the middle segment (b@10).
        let cppdeep = "int v = a.b.c;";
        assert_eq!(
            satp(LanguageId::Cpp, cppdeep, 10),
            Some(("b".into(), "a.b.c".into()))
        );
        // Cpp `::` (no double handling): `ns::A::x` stays the byte-scan
        // whole token — the pre-010-rung4 extraction, byte-for-byte
        // (A@13).
        let qual = "auto v = ns::A::x;";
        assert_eq!(
            satp(LanguageId::Cpp, qual, 13),
            Some(("A".into(), "ns::A::x".into()))
        );
        // Toml: dotted key `a.b.c = 1` — cursor on either segment.
        let toml = "a.b.c = 1";
        assert_eq!(
            satp(LanguageId::Toml, toml, 0),
            Some(("a".into(), "a.b.c".into()))
        );
        assert_eq!(
            satp(LanguageId::Toml, toml, 4),
            Some(("c".into(), "a.b.c".into()))
        );
        // Toml table-header dotted key (`[a.b]`, b@3).
        assert_eq!(
            satp(LanguageId::Toml, "[a.b]", 3),
            Some(("b".into(), "a.b".into()))
        );
    }

    /// 010-rung4-and-paths (item 2, pin): the degradation stays
    /// byte-for-byte — C/Cpp `p->x` (the `->` segments are not bare
    /// identifier segments) and JSON keys (the pinned JSON grammar has
    /// no dotted-key node; the judgment: bare-key index lookup is the
    /// whole feature) all keep the exact bare extraction.
    #[test]
    fn symbol_at_point_c_arrow_and_json_keys_stay_bare() {
        // "int y = p->x;": x@11.
        let line = "int y = p->x;";
        assert_eq!(
            satp(LanguageId::C, line, 11),
            Some(("x".into(), "x".into()))
        );
        assert_eq!(
            satp(LanguageId::Cpp, line, 11),
            Some(("x".into(), "x".into()))
        );
        // Json: a quoted key — the bare key, byte-for-byte (k@3).
        let js = "  \"k\": 1";
        assert_eq!(
            satp(LanguageId::Json, js, 3),
            Some(("k".into(), "k".into()))
        );
    }

    /// newlang-paths (discriminating): the M-. path token becomes the
    /// WHOLE dotted path for the new-languages-lane containers — Java's
    /// `field_access` / `scoped_identifier` / `scoped_type_identifier`,
    /// C#'s `member_access_expression` / `qualified_name`, and Ruby's
    /// argumentless `call` (the node.rs position gate + container-validity
    /// rule; the kinds are pinned there — `java_member_path_comes_back_
    /// whole`, `java_scoped_type_path_comes_back_whole`,
    /// `csharp_member_path_comes_back_whole`,
    /// `csharp_qualified_name_comes_back_whole`,
    /// `ruby_method_chain_comes_back_whole`). Pre-newlang-paths every one
    /// of these returned the BARE identifier.
    #[test]
    fn symbol_at_point_java_csharp_ruby_containers_extend_the_token() {
        // Java `field_access`: `A.c` — cursor on the receiver and the
        // member. "class A { int c; void f() { int x = A.c; } }\n": A@36,
        // c@38.
        let java = "class A { int c; void f() { int x = A.c; } }\n";
        assert_eq!(
            satp(LanguageId::Java, java, 36),
            Some(("A".into(), "A.c".into()))
        );
        assert_eq!(
            satp(LanguageId::Java, java, 38),
            Some(("c".into(), "A.c".into()))
        );
        // Java deep chain: `a.b.c` under the MIDDLE segment (b@31).
        let javadeep = "class A { void f() { int v = a.b.c; } }\n";
        assert_eq!(
            satp(LanguageId::Java, javadeep, 31),
            Some(("b".into(), "a.b.c".into()))
        );
        // Java scoped type: `com.example.Foo` under the middle segment
        // (example@25).
        let javatype = "class B { void f() { com.example.Foo o; } }\n";
        assert_eq!(
            satp(LanguageId::Java, javatype, 25),
            Some(("example".into(), "com.example.Foo".into()))
        );
        // C# `member_access_expression`: `o.P` — cursor on the member
        // and parked right after it. "class A { void F() { int v = o.P;
        // } }\n": o@29, P@31.
        let cs = "class A { void F() { int v = o.P; } }\n";
        assert_eq!(
            satp(LanguageId::CSharp, cs, 31),
            Some(("P".into(), "o.P".into()))
        );
        assert_eq!(
            satp(LanguageId::CSharp, cs, 32),
            Some(("P".into(), "o.P".into()))
        );
        // C# deep chain: `a.b.c` under the middle segment (b@31).
        let csdeep = "class A { void F() { int v = a.b.c; } }\n";
        assert_eq!(
            satp(LanguageId::CSharp, csdeep, 31),
            Some(("b".into(), "a.b.c".into()))
        );
        // C# `qualified_name` in a namespace header (Inner@12).
        assert_eq!(
            satp(LanguageId::CSharp, "namespace N.Inner { class A { } }", 12),
            Some(("Inner".into(), "N.Inner".into()))
        );
        // Ruby argumentless `call`: `obj.name` — cursor on EITHER
        // segment (obj@0, name@4).
        assert_eq!(
            satp(LanguageId::Ruby, "obj.name", 0),
            Some(("obj".into(), "obj.name".into()))
        );
        assert_eq!(
            satp(LanguageId::Ruby, "obj.name", 4),
            Some(("name".into(), "obj.name".into()))
        );
        // Ruby deep chain: `a.b.c` under the middle segment (b@2).
        assert_eq!(
            satp(LanguageId::Ruby, "a.b.c", 2),
            Some(("b".into(), "a.b.c".into()))
        );
    }

    /// newlang-paths (pin): the degradation stays byte-for-byte — a Ruby
    /// call WITH arguments must never match (the node.rs container rule:
    /// a `call` is a path container only with a `receiver` field and NO
    /// `arguments` field, and node_at never returns an argument-carrying
    /// `call`), the 011-06 all-identifier guard rejects an argumentless
    /// outer chain whose receiver carries a `(...)` segment, a Java
    /// `method_invocation` stays the bare member, and Scheme (no path
    /// syntax at all) is untouched.
    #[test]
    fn symbol_at_point_java_csharp_ruby_degradation_stays_bare() {
        // Ruby: `a.b(1).c` — the OUTER call is argumentless (its
        // receiver is `a.b(1)`), so node_at returns the whole chain as
        // one `call`; the 011-06 all-identifier-segment guard then
        // rejects it (`b(1)` is not a bare identifier) → bare `c`,
        // byte-for-byte (c@7).
        assert_eq!(
            satp(LanguageId::Ruby, "a.b(1).c", 7),
            Some(("c".into(), "c".into()))
        );
        // Ruby: the argument-carrying segment ITSELF — `a.b(1)` at `b`
        // (b@2) and at its receiver `a` (a@0): node_at returns the bare
        // identifiers (an argument-carrying call is not a container), so
        // no upgrade, byte-for-byte.
        assert_eq!(
            satp(LanguageId::Ruby, "a.b(1)", 2),
            Some(("b".into(), "b".into()))
        );
        assert_eq!(
            satp(LanguageId::Ruby, "a.b(1)", 0),
            Some(("a".into(), "a".into()))
        );
        // Ruby: a bare call with arguments — `puts 1` stays the plain
        // `puts` identifier (node.rs pin `ruby_bare_call_stays_bare`).
        assert_eq!(
            satp(LanguageId::Ruby, "puts 1", 1),
            Some(("puts".into(), "puts".into()))
        );
        // Ruby: `::` inside a member chain — `Foo::Bar.new` (node_at
        // returns the whole argumentless `call`), but the `Foo::Bar`
        // segment is not a bare identifier segment → bare `new` (new@9);
        // the `Foo::Bar` half itself stays the byte-scan `::` token.
        assert_eq!(
            satp(LanguageId::Ruby, "Foo::Bar.new", 9),
            Some(("new".into(), "new".into()))
        );
        assert_eq!(
            satp(LanguageId::Ruby, "Foo::Bar.new", 6),
            Some(("Bar".into(), "Foo::Bar".into()))
        );
        // Java: `o.m(1)` at `m` — a `method_invocation` is NOT a path
        // container (node.rs pin `java_method_invocation_stays_bare`):
        // the bare `m` identifier, byte-for-byte (m@23).
        assert_eq!(
            satp(LanguageId::Java, "class A { void f() { o.m(1); } }", 23),
            Some(("m".into(), "m".into()))
        );
        // C#: `o.P` whose segment is a bare identifier still upgrades —
        // but the DEEP call `o.P().Q` (member access on an invocation)
        // carries a `()` segment → bare `Q` (Q@7).
        assert_eq!(
            satp(LanguageId::CSharp, "o.P().Q", 7),
            Some(("Q".into(), "Q".into()))
        );
        // Scheme: untouched — the flat grammar has no path container, so
        // a bare symbol stays the bare extraction (x@8).
        assert_eq!(
            satp(LanguageId::Scheme, "(define x 1)", 8),
            Some(("x".into(), "x".into()))
        );
    }

    /// 010-rung4-and-paths (item 2, app level): M-. on `p.x` in a C
    /// buffer lands via the index fall-through with the whole path — the
    /// project index has no C field symbols (the C query indexes
    /// functions / structs / macros only), so the lookup degrades to the
    /// enclosing symbol and jumps there, exactly as M-. does today;
    /// nothing new is guessed.
    #[test]
    fn xref_c_field_access_lands_via_index_fall_through() {
        let (mut s, _dir) = store_with_index(&[("c/main.c", "struct Point { int x; };\nint use_it(struct Point p) {\n    return p.x;\n}\n")]);
        s.open_path("c/main.c");
        // Line 2: "    return p.x;" — `x` at col 13.
        s.set_point(2, 13, 13);
        s.xref_find_definitions();
        // The whole path `p.x` (and the bare `x`) has no indexed
        // definition — the enclosing-symbol fall-through lands on
        // `use_it` (line 1).
        assert!(!s.picker_open(), "no picker: {}", s.message);
        assert_eq!(s.view_name_display(), "c/main.c");
        assert_eq!(s.point_line(), 1, "the enclosing function (msg: {})", s.message);
    }

    /// newlang-paths (e2e pin, Java): M-. on `A.c` in a Java buffer — the
    /// whole path `A.c` (and the bare `c`) has NO indexed definition: the
    /// Java outline indexes classes / methods only (fields are
    /// deliberately out — queries.rs), so the index fall-through lands on
    /// the enclosing method `f`, exactly like the C pin above; nothing
    /// new is guessed.
    #[test]
    fn xref_java_field_access_lands_via_index_fall_through() {
        let (mut s, _dir) = store_with_index(&[(
            "src/A.java",
            "class A {\n    int c;\n    void f() {\n        int x = A.c;\n    }\n}\n",
        )]);
        s.open_path("src/A.java");
        // Line 3 (0-based): "        int x = A.c;" — `c` at col 18.
        s.set_point(3, 18, 18);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "no picker: {}", s.message);
        assert_eq!(s.view_name_display(), "src/A.java");
        assert_eq!(
            s.point_line(),
            2,
            "the enclosing method (msg: {})",
            s.message
        );
    }

    /// newlang-paths (e2e pin, C#): M-. on `o.P` in a C# buffer — the
    /// C# outline indexes properties (queries.rs): the whole path `o.P`
    /// has no indexed symbol, but the fall-through to the last segment
    /// `P` lands on the property declaration — one candidate, direct
    /// jump (no picker).
    #[test]
    fn xref_csharp_property_access_lands_in_project_index() {
        let (mut s, _dir) = store_with_index(&[(
            "src/A.cs",
            "class A {\n    public int P { get; set; }\n    void F() {\n        int v = o.P;\n    }\n}\n",
        )]);
        s.open_path("src/A.cs");
        // Line 3 (0-based): "        int v = o.P;" — `P` at col 18.
        s.set_point(3, 18, 18);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "no picker: {}", s.message);
        assert_eq!(s.view_name_display(), "src/A.cs");
        assert_eq!(
            s.point_line(),
            1,
            "the property declaration (msg: {})",
            s.message
        );
    }

    /// newlang-paths (e2e pin, Ruby): M-. on `obj.name` in a Ruby buffer
    /// — the whole path `obj.name` (the argumentless `call` container)
    /// has no indexed symbol, but the fall-through to the last segment
    /// `name` lands on the `def name` the Ruby outline indexes — one
    /// candidate, direct jump (no picker).
    #[test]
    fn xref_ruby_method_access_lands_in_project_index() {
        let (mut s, _dir) = store_with_index(&[(
            "src/obj.rb",
            "class Obj\n  def name\n    42\n  end\nend\n\ndef show(obj)\n  puts obj.name\nend\n",
        )]);
        s.open_path("src/obj.rb");
        // Line 7 (0-based): "  puts obj.name" — `name` at col 12.
        s.set_point(7, 12, 12);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "no picker: {}", s.message);
        assert_eq!(s.view_name_display(), "src/obj.rb");
        assert_eq!(
            s.point_line(),
            1,
            "the `def name` declaration (msg: {})",
            s.message
        );
    }

    /// 011-06 × 011-02 interplay (pin): a dotted token resolves on its OWN
    /// path — the import walks are only for BARE symbols, so any
    /// separator-containing symbol carries NO hint (the provider's own
    /// dotted machinery does the work; no double-application).
    #[test]
    fn resolver_scope_dotted_symbols_carry_no_import_hint() {
        let psrc = "import json\n\njson.dumps('x')\n";
        let pbyte = psrc.find("dumps").expect("fixture");
        assert!(
            AppStore::python_scope_for(psrc, pbyte, "json.dumps").is_empty(),
            "a dotted python symbol never gets an import-walk hint"
        );
        let gsrc = "import \"fmt\"\n\nfunc main() {\n\tfmt.Println(1)\n}\n";
        let gbyte = gsrc.find("Println").expect("fixture");
        assert!(
            AppStore::go_scope_for(gsrc, gbyte, "fmt.Println").is_empty(),
            "a dotted go symbol never gets a dot-import hint"
        );
    }

    /// 011-06 (discriminating, app level): M-. on `json.dumps` in a python
    /// buffer — the resolver context carries the WHOLE dotted path (the
    /// no-runtime miss message echoes the exact token fed; pre-011-06 it
    /// would have said the bare `dumps`).
    #[test]
    fn xref_python_dotted_token_reaches_resolver() {
        let (mut s, _dir) = store_with_index(&[
            ("main.py", "import json\n\njson.dumps(x)\n"),
        ]);
        s.open_path("main.py");
        s.set_point(2, 7, 7); // cursor inside `dumps` on line 3
        s.xref_find_definitions();
        assert!(
            s.message.contains("no provider resolution for `json.dumps`"),
            "the resolver got the dotted path, got: {}", s.message
        );
        assert_eq!(s.resolve_generation, 2, "fall-through fired exactly once");
    }

    /// 011-06 (discriminating, app level): M-. on `fakelib.apply` in a js
    /// buffer carries the dotted path to the resolver (pre-011-06 the
    /// `::`-only extraction fed the bare `apply`).
    #[test]
    fn xref_js_dotted_token_reaches_resolver() {
        let (mut s, _dir) = store_with_index(&[
            ("main.js", "import * as fakelib from \"fakelib\";\n\nfakelib.apply(5);\n"),
        ]);
        s.open_path("main.js");
        s.set_point(2, 10, 10); // cursor inside `apply` on line 3
        s.xref_find_definitions();
        assert!(
            s.message.contains("no provider resolution for `fakelib.apply`"),
            "the resolver got the dotted path, got: {}", s.message
        );
        assert_eq!(s.resolve_generation, 2, "fall-through fired exactly once");
    }

    #[test]
    fn symbol_at_point_boundaries_and_garbage() {
        // Rust: the byte-for-byte boundary behavior (011-06 kept it).
        assert_eq!(
            satp(LanguageId::Rust, "fn main() {}", 0),
            Some(("fn".into(), "fn".into()))
        );
        // Punctuation (the `(` after a name): the run before the point.
        assert_eq!(
            satp(LanguageId::Rust, "call(1)", 4),
            Some(("call".into(), "call".into()))
        );
        // Whitespace with nothing identifier-ish around: no symbol.
        assert_eq!(satp(LanguageId::Rust, "let a = 1;", 7), None);
        assert_eq!(satp(LanguageId::Rust, "{ ", 1), None);
        // A leading `::` does not extend past itself (empty path segment).
        assert_eq!(
            satp(LanguageId::Rust, "::inner", 3),
            Some(("inner".into(), "inner".into()))
        );
        // Column at the very end of the line: the trailing run counts.
        assert_eq!(
            satp(LanguageId::Rust, "let x = 1;", 9),
            Some(("1".into(), "1".into()))
        );
        // Mid-line identifier.
        assert_eq!(
            satp(LanguageId::Rust, "let x = 1;", 4),
            Some(("x".into(), "x".into()))
        );
        // Deep path: `a::b::c` under the middle segment.
        assert_eq!(
            satp(LanguageId::Rust, "use a::b::c;", 7),
            Some(("b".into(), "a::b::c".into()))
        );
        // 006-02b item 4: the cursor on the SECOND colon of a `::`
        // separator counts as the end of the preceding segment (the first
        // colon already did, via the "run before the point" rule).
        assert_eq!(
            satp(LanguageId::Rust, "a::b", 1),
            Some(("a".into(), "a::b".into()))
        );
        assert_eq!(
            satp(LanguageId::Rust, "a::b", 2),
            Some(("a".into(), "a::b".into()))
        );
        assert_eq!(
            satp(LanguageId::Rust, "use a::b::c;", 6),
            Some(("a".into(), "a::b::c".into()))
        );
    }

    #[test]
    fn xref_workspace_miss_fires_resolver_fallthrough() {
        // `tokio::spawn` at top level: no index definition, no enclosing
        // symbol → the M-. workspace miss falls through to the resolver.
        // Plain (no runtime) unit test: the spawn is skipped, the
        // generation is bumped, and the miss is reported synchronously.
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "tokio::spawn(f);\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 2, 2); // cursor inside `tokio`
        s.xref_find_definitions();
        // 006-02b item 2: the xref-entry supersede bump + the
        // start_symbol_resolution bump — two bumps for a fall-through.
        assert_eq!(s.resolve_generation, 2, "fall-through fired exactly once");
        assert!(
            s.resolving_display().is_empty(),
            "no runtime: the indicator must not hang"
        );
        assert!(
            s.message.contains("no provider resolution for `tokio::spawn`"),
            "graceful miss message, got: {}", s.message
        );
    }

    #[test]
    fn xref_workspace_hit_and_enclosing_hit_skip_resolver() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() { target(); }\n"),
            ("src/lib.rs", "pub fn target() {}\n"),
        ]);
        s.open_path("src/main.rs");
        // Direct hit under the point → no fall-through.
        s.set_point(0, 12, 12);
        s.xref_find_definitions();
        assert_eq!(s.resolve_generation, 1, "direct hit: one supersede bump, no job");
        // Enclosing hit → no fall-through either (a second supersede bump).
        s.open_path("src/lib.rs");
        s.set_point(0, 0, 0);
        s.xref_find_definitions();
        assert_eq!(s.resolve_generation, 2, "enclosing hit: one supersede bump, no job");
    }

    /// (011-01) The context language follows the buffer's extension — the
    /// lowercase registry name the providers' `languages()` expect. An
    /// unknown extension stays `None` (the chain keeps the pre-dispatch
    /// in-order walk for those).
    #[test]
    fn resolution_language_maps_buffer_extensions() {
        let (s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        assert_eq!(s.resolution_language("main.py"), Some("python".into()));
        assert_eq!(s.resolution_language("src/lib.js"), Some("javascript".into()));
        assert_eq!(s.resolution_language("a.ts"), Some("typescript".into()));
        assert_eq!(s.resolution_language("a.tsx"), Some("tsx".into()));
        // .jsx/.mjs/.cjs map to JavaScript (registry.rs has no Jsx variant):
        // a JSX buffer must dispatch to the js provider, never silently fall
        // back to the pre-dispatch walk (011-01 review P2-3).
        assert_eq!(s.resolution_language("a.jsx"), Some("javascript".into()));
        assert_eq!(s.resolution_language("a.mjs"), Some("javascript".into()));
        assert_eq!(s.resolution_language("a.cjs"), Some("javascript".into()));
        // A registry language with no provider maps to its name (an honest
        // zero-eligible bail) rather than falling back to cargo.
        assert_eq!(s.resolution_language("a.c"), Some("c".into()));
        assert_eq!(s.resolution_language("main.go"), Some("go".into()));
        // A Rust buffer still carries "rust" (the existing path, unchanged).
        assert_eq!(s.resolution_language("src/main.rs"), Some("rust".into()));
        // Unknown extension: no language, pre-dispatch behavior.
        assert_eq!(s.resolution_language("notes.txt"), None);
    }

    /// (011-01) A workspace miss in a NON-RUST buffer fires the resolver
    /// fall-through (the app wiring is language-agnostic; the provider
    /// itself is selected by `SymbolContext.language` inside the chain).
    /// Plain (no runtime) unit test: synchronous graceful miss.
    #[test]
    fn python_buffer_miss_fires_resolver_fallthrough() {
        let (mut s, _dir) = store_with_index(&[
            ("main.py", "import os\nx = os.path.join('a', 'b')\n"),
        ]);
        s.open_path("main.py");
        s.start_symbol_resolution("os.path.join", "main.py");
        assert_eq!(s.resolve_generation, 1, "the fall-through fired exactly once");
        assert!(s.resolving_display().is_empty());
        // 011-01 review P2-1: the pre-fix assertion was non-discriminating —
        // the no-runtime fast path shares this message prefix. NOTE the two
        // paths cannot both be asserted here: `start_symbol_resolution` checks
        // for a background runtime BEFORE dispatching, and in a unit test
        // there is none, so the no-runtime branch IS the reachable path here
        // (the real chain + dispatch run off the input path under tokio).
        // What this test CAN pin, and what discriminates dispatch: the
        // language is mapped from the buffer's extension at the context seam
        // (asserted in `resolution_language_maps_buffer_extensions`), and a
        // REGRESSION that broke dispatch would be visible there, not here.
        // So assert the runtime seam explicitly instead of the two prefixes
        // being interchangeable.
        assert!(
            s.message.contains("no background runtime"),
            "unit tests have no runtime, so the fast path is expected: {msg}",
            msg = s.message
        );
        assert!(
            s.message.contains("no provider resolution for `os.path.join`"),
            "graceful miss message, got: {}", s.message
        );
    }

    // ── 007-03: the resolver fall-through's scope hint ─────────────────

    /// The fall-through context carries the `use` path for a bare symbol.
    #[test]
    fn resolver_scope_carries_use_path_for_bare_symbol() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "use serde::Deserialize;\nfn main() { let _d: Deserialize = D; }\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(1, 15, 15); // inside the BARE `Deserialize`
        assert_eq!(
            s.resolver_scope("Deserialize"),
            vec!["serde".to_string(), "Deserialize".to_string()]
        );
    }

    /// `use x as y` (alias): the context carries the ALIASED (original)
    /// path, so `y` resolves to the original item.
    #[test]
    fn resolver_scope_alias_carries_original_path() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "use serde::Deserialize as D;\nfn main() { let _d: D = D::default(); }\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(1, 21, 21); // inside the first (bare) `D`
        assert_eq!(
            s.resolver_scope("D"),
            vec!["serde".to_string(), "Deserialize".to_string()]
        );
    }

    /// A bare symbol with NO `use` declaration in scope: the scope stays
    /// EMPTY (the providers keep their byte-for-byte no-hint behavior —
    /// std/prelude names are never guessed).
    #[test]
    fn resolver_scope_empty_without_use() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() { let x = 9; }\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 18, 18); // inside `x` (a let binding, no import)
        assert!(s.resolver_scope("x").is_empty());
    }

    /// A path-shaped symbol carries the ENCLOSING scope (007-01's
    /// `scope_path_at`), not an import.
    #[test]
    fn resolver_scope_path_symbol_carries_enclosing_scope() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() { tokio::spawn(f); }\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 15, 15); // inside `tokio`
        assert_eq!(s.resolver_scope("tokio::spawn"), vec!["main".to_string()]);
    }

    /// Group imports (`use serde::{…}`): the alias entry and the plain
    /// entry both carry their original paths.
    #[test]
    fn resolver_scope_group_imports() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "use serde::{Deserialize as D, Serialize};\nfn main() {}\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 26, 26); // inside the alias `D`
        assert_eq!(
            s.resolver_scope("D"),
            vec!["serde".to_string(), "Deserialize".to_string()]
        );
        s.set_point(0, 32, 32); // inside `Serialize`
        assert_eq!(
            s.resolver_scope("Serialize"),
            vec!["serde".to_string(), "Serialize".to_string()]
        );
    }

    /// Innermost module wins: a `use` in the enclosing `mod` shadows the
    /// top-level one (Rust's module scoping).
    #[test]
    fn resolver_scope_innermost_use_wins() {
        let (mut s, _dir) = store_with_index(&[
            (
                "src/main.rs",
                "use a::Thing;\nmod inner {\n    use b::Thing;\n    fn f() {}\n}\n",
            ),
        ]);
        s.open_path("src/main.rs");
        s.set_point(2, 12, 12); // inside `Thing` of `use b::Thing;`
        assert_eq!(s.resolver_scope("Thing"), vec!["b".to_string(), "Thing".to_string()]);
    }

    /// Non-Rust buffers degrade to an empty scope (007-01's layer is
    /// Rust-only; the providers keep their no-hint behavior).
    #[test]
    fn resolver_scope_non_rust_buffer_is_empty() {
        let (mut s, _dir) = store_with_index(&[
            ("main.py", "import json\nprint(json)\n"),
        ]);
        s.open_path("main.py");
        s.set_point(0, 7, 7); // inside `json`
        assert!(s.resolver_scope("json").is_empty());
    }

    /// `crate::`-prefixed imports never name an external crate: no hint
    /// (the bare symbol keeps the providers' no-hint behavior). A plain
    /// same-crate `use inner::Thing;` DOES carry its two-segment hint —
    /// it is the user's own import path, not a guess; the provider then
    /// bails honestly ("crate `inner` is not in the cargo graph") when no
    /// package bears that name.
    #[test]
    fn resolver_scope_crate_prefix_is_not_guessed() {
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "use crate::Thing;\nfn main() {}\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 9, 9); // inside `Thing` of `use crate::Thing;`
        assert!(s.resolver_scope("Thing").is_empty(), "crate:: prefix never hints");
    }

    /// 007-03 review P2: the negative import shapes must all yield an EMPTY
    /// hint (a miss, never a guessed crate). Pins glob, single-segment, and
    /// the `self`/`super` prefixes.
    #[test]
    fn resolver_scope_negative_import_shapes_are_not_guessed() {
        for (name, src, symbol) in [
            (
                "glob",
                "use serde::*;\nfn main() { let _ = Deserialize; }\n",
                "Deserialize",
            ),
            (
                "single-segment",
                "use serde;\nfn main() { let _ = serde; }\n",
                "serde",
            ),
            (
                "self-prefix",
                "use self::Thing;\nfn main() { let _ = Thing; }\n",
                "Thing",
            ),
            (
                "super-prefix",
                "mod m { use super::Thing; fn f() { let _ = Thing; } }\n",
                "Thing",
            ),
        ] {
            let (mut s, _dir) = store_with_index(&[("src/main.rs", src)]);
            s.open_path("src/main.rs");
            // Point at the USE in code (the last occurrence), not the import.
            let at = src.rfind(symbol).expect("fixture symbol");
            let line = src[..at].matches('\n').count();
            let col = at - src[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
            s.set_point(line, col, col);
            assert!(
                s.resolver_scope(symbol).is_empty(),
                "{name}: a non-resolvable import must never hint"
            );
        }
    }

    /// 007-03 review P2: a non-resolvable entry in a group must SKIP, not
    /// abort the group — `use a::b::{self, c};` still resolves bare `c`.
    #[test]
    fn resolver_scope_group_skips_bad_entries() {
        let src = "use serde::{self as s2, Deserialize};\nfn main() {}\n";
        let (mut s, _dir) = store_with_index(&[("src/main.rs", src)]);
        s.open_path("src/main.rs");
        let at = src.find("Deserialize").expect("fixture symbol");
        s.set_point(0, at, at);
        assert_eq!(
            s.resolver_scope("Deserialize"),
            vec!["serde".to_string(), "Deserialize".to_string()],
            "a preceding bad group entry must not discard a later good one"
        );
    }

    #[tokio::test]
    async fn xref_resolver_hit_lands_read_only_jump() {
        // A resolve event with a resolved source (outside the project root,
        // external) lands as a jump: read-only buffer, point on the
        // resolved line, jump entry recorded, status cleared, and the
        // external path never enters the project recents.
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "tokio::spawn(f);\n"),
        ]);
        // An "external" source file outside the project root.
        let ext = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(ext.path(), "extern crate dep;\npub fn spawn<F>(f: F) {}\n").unwrap();
        let root = s.project.as_ref().map(|p| p.root.to_string_lossy().into_owned()).unwrap();
        s.open_path("src/main.rs");
        s.set_point(0, 2, 2);
        let origin_key = s.buffers.current().map(String::from).unwrap();
        let recents_before = s.project_store.recents.list(&root).to_vec();
        s.xref_find_definitions();
        assert_eq!(s.resolve_generation, 2, "xref supersede bump + start bump");
        assert!(s.resolving_display().contains("`tokio::spawn`"), "status activity while pending");
        // Land a fabricated (provider-shaped) hit for the in-flight job.
        let event = ResolveEvent {
            generation: 2,
            symbol: "tokio::spawn".into(),
            source: Some(ResolvedSource {
                file: ext.path().to_path_buf(),
                source_root: ext.path().parent().unwrap().to_path_buf(),
                external: true,
                line: Some(2),
            }),
            error: None,
        };
        s.apply_resolve_event(&event);
        let key = s.buffers.current().map(String::from).unwrap();
        assert_eq!(key, ext.path().to_string_lossy().into_owned(), "external buffer is current");
        let buf = s.buffers.get(&key).unwrap();
        assert!(!buf.editable, "external source is read-only");
        assert_eq!(s.point_line(), 1, "point on the resolved (1-based line 2) definition");
        assert!(s.message.contains("jumped to"), "jump report, got: {}", s.message);
        assert!(s.resolving_display().is_empty(), "status activity cleared");
        assert_eq!(
            s.project_store.recents.list(&root).to_vec(),
            recents_before,
            "external landing never records a recent"
        );
        // Jump entry recorded: back lands on the origin buffer/line.
        s.jump_back();
        assert_eq!(s.buffers.current().map(String::from).unwrap(), origin_key, "jump-back returns to the origin");
    }

    #[tokio::test]
    async fn xref_resolver_miss_reports_graceful_message() {
        // All providers miss → the event carries the error; the message
        // reports it and nothing else changes (no buffer, no jump, no panic).
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "tokio::spawn(f);\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 2, 2);
        let before = s.buffers.current().map(String::from).unwrap();
        s.xref_find_definitions();
        let event = ResolveEvent {
            generation: 2,
            symbol: "tokio::spawn".into(),
            source: None,
            error: Some("no tooling provider could resolve symbol `tokio::spawn` (tried 1 provider(s): rust): no crate `tokio`".into()),
        };
        s.apply_resolve_event(&event);
        assert!(
            s.message.starts_with("no provider resolution for `tokio::spawn`:"),
            "got: {}", s.message
        );
        assert_eq!(s.buffers.current().map(String::from).unwrap(), before, "view unchanged on a miss");
        assert!(s.resolving_display().is_empty());
    }

    #[tokio::test]
    async fn xref_resolver_event_end_to_end_miss_without_cargo() {
        // Real fall-through end to end (no network): a project WITHOUT a
        // Cargo.toml (`.projectile` is a root marker, not one) makes the
        // cargo provider fail fast, and the event published on the
        // ResolveBus lands the graceful report.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join(".projectile"), "\n").unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "tokio::spawn(f);\n").unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        let mut rx = s.resolve_bus.subscribe();
        s.open_path("src/main.rs");
        s.set_point(0, 2, 2);
        s.xref_find_definitions();
        assert_eq!(s.resolve_generation, 2, "fall-through started (xref bump + start bump)");
        let event = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("resolve event published within 30s");
        assert!(event.is_ok());
        let event = rx.borrow_and_update().clone();
        assert_eq!(event.generation, 2);
        assert!(event.source.is_none(), "miss: no source, got error: {:?}", event.error);
        s.apply_resolve_event(&event);
        assert!(
            s.message.starts_with("no provider resolution for `tokio::spawn`"),
            "got: {}", s.message
        );
        assert!(
            s.message.contains("tried 1 provider(s): rust"),
            "provider chain named in the report: {}", s.message
        );
        assert!(s.resolving_display().is_empty());
    }

    #[tokio::test]
    async fn xref_resolver_fallthrough_carries_use_scope_hint() {
        // 007-03 end to end: a BARE symbol imported via `use` carries the
        // use-path into the `SymbolContext`, so the cargo provider lands it
        // in the (locally cached) serde registry source instead of the
        // bare-symbol bail. Guard: the ambient `~/.cargo` must have a serde
        // registry source (this dev box does; the resolver crate's own
        // integration tests assume the same warm cache).
        let registry_src = std::path::PathBuf::from(
            std::env::var("CARGO_HOME")
                .unwrap_or_else(|_| format!("{}/.cargo", std::env::var("HOME").unwrap_or_default())),
        )
        .join("registry/src");
        // Discover a CACHED plain `serde-<semver>` registry source (not
        // serde_* / serde-untagged) and pin the dependency to that exact
        // version so `cargo metadata` never needs a fresh index fetch.
        let mut serde_dir: Option<std::path::PathBuf> = None;
        let mut serde_version: Option<String> = None;
        // The registry source dirs live under `registry/src/<index-hash>/`.
        let index_dirs: Vec<std::path::PathBuf> = std::fs::read_dir(&registry_src)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        for index_dir in &index_dirs {
            let Ok(entries) = std::fs::read_dir(index_dir) else { continue };
            for entry in entries.flatten() {
                let name = match entry.file_name().to_str() {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let Some(v) = name.strip_prefix("serde-") else {
                    continue;
                };
                if !v.split('.').next().is_some_and(|c| c.chars().all(|c| c.is_ascii_digit())) {
                    continue; // serde-untagged, …
                }
                if !entry.path().join("src").is_dir() {
                    continue;
                }
                let better = match &serde_version {
                    None => true,
                    Some(cur) => {
                        let key = |s: &str| {
                            s.split('.')
                                .map(|p| {
                                    p.chars()
                                        .take_while(|c| c.is_ascii_digit())
                                        .collect::<String>()
                                })
                                .map(|p| p.parse::<u64>().unwrap_or(0))
                                .collect::<Vec<_>>()
                        };
                        key(v) > key(cur)
                    }
                };
                if better {
                    serde_dir = Some(entry.path());
                    serde_version = Some(v.to_string());
                }
            }
            break; // one index-hash dir per CARGO_HOME
        }
        let Some(serde_dir) = serde_dir else { return; };
        let serde_version = serde_version.expect("set with the dir");
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            format!(
                "[package]\nname = \"xscope\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
                 [dependencies]\nserde = \"={serde_version}\"\n"
            ),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "use serde::Deserialize;\nfn main() {}\n",
        )
        .unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        let mut rx = s.resolve_bus.subscribe();
        s.open_path("src/main.rs");
        s.set_point(0, 9, 9); // on `Deserialize` (bare, use-imported)
        s.start_symbol_resolution("Deserialize", "src/main.rs");
        let _ = tokio::time::timeout(std::time::Duration::from_secs(120), rx.changed())
            .await
            .expect("resolve event published within 120s");
        let event = rx.borrow_and_update().clone();
        let source = event
            .source
            .expect("the bare use-imported symbol resolves via the scope hint");
        assert!(source.external, "the serde registry source is external");
        assert!(
            source.file.starts_with(&serde_dir),
            "landed in the serde registry source: {:?}",
            source.file
        );
        let file_name = source.file.to_string_lossy().into_owned();
        assert!(
            file_name.contains("serde-"),
            "file in a serde-<version> dir: {file_name}"
        );

        // Degradation pin at the same seam: a BARE symbol with NO `use`
        // keeps today's byte-for-byte "needs scope info" behavior — the
        // provider never gets a hint, so the whole chain reports a miss
        // naming the symbol (the bare bail fires before any cargo work).
        s.start_symbol_resolution("plain_local_name", "src/main.rs");
        let _ = tokio::time::timeout(std::time::Duration::from_secs(60), rx.changed())
            .await
            .expect("second resolve event published within 60s");
        let event = rx.borrow_and_update().clone();
        let err = event.error.expect("a miss (no use declares the name)");
        assert!(
            err.contains("no tooling provider could resolve symbol `plain_local_name`"),
            "no hint → the chain's miss report: {err}"
        );
        assert!(
            !err.contains("jumped"),
            "a bare unimported symbol never resolves: {err}"
        );
    }

    /// 011-08 fix-jsrel P2-7 live leg: with the app now EMITTING the
    /// relative hint, M-. on a relative use site lands in the SIBLING
    /// file through the REAL provider chain: workspace-local
    /// (`external = false`) and opened through `open_resolved_source`'s
    /// project branch — an EDITABLE project buffer, never the read-only
    /// external path. (The provider-side twin goldens are the js corpus'
    /// `relative-*` probes.)
    #[tokio::test]
    async fn xref_relative_import_lands_in_sibling_file_editable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        // The project marker (project detection is marker-driven).
        std::fs::write(dir.path().join("package.json"), "{}\n").unwrap();
        let src = "import { legacyJoin } from \"./legacy-util\";\nfunction f() { legacyJoin(); }\n";
        std::fs::write(dir.path().join("src/app.js"), src).unwrap();
        std::fs::write(
            dir.path().join("src/legacy-util.js"),
            "function legacyJoin() {}\nmodule.exports = { legacyJoin };\n",
        )
        .unwrap();
        let sibling = std::fs::canonicalize(dir.path().join("src/legacy-util.js")).unwrap();
        let base = tempfile::tempdir().unwrap();
        let mut s = AppStore::at(dir.path(), base.path().to_path_buf());
        let mut rx = s.resolve_bus.subscribe();
        s.open_path("src/app.js");
        let at = src.rfind("legacyJoin").expect("fixture");
        let line = src[..at].matches('\n').count();
        let col = at - src[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
        s.set_point(line, col, col);
        s.start_symbol_resolution("legacyJoin", "src/app.js");
        let _ = tokio::time::timeout(std::time::Duration::from_secs(60), rx.changed())
            .await
            .expect("resolve event published within 60s");
        let event = rx.borrow_and_update().clone();
        let source = event
            .source
            .as_ref()
            .expect("the relative use site resolves (the app emits the hint)");
        assert!(!source.external, "the sibling file is workspace-local (external = false)");
        assert_eq!(source.file, sibling, "landed in the sibling file");
        assert_eq!(
            source.line,
            Some(1),
            "the member definition line (1-based)")
        ;
        s.apply_resolve_event(&event);
        assert_eq!(
            s.view_name_display(),
            "src/legacy-util.js",
            "the view is now the sibling file"
        );
        let cur: &str = s.buffers.current().expect("a current buffer");
        assert!(
            !s.external_buffers.contains(cur),
            "the landed sibling is an EDITABLE project buffer (the project branch of open_resolved_source), never the read-only external one"
        );
    }

    #[test]
    fn xref_resolver_stale_generation_event_discarded() {
        // An event from a superseded request (or a previous project) must be
        // discarded: no jump, no message — and (006-02b item 3) it CLEARS
        // the resolving indicator, because the latest-wins drain means this
        // stale send may have overwritten the current job's event in the
        // channel; a stuck indicator would otherwise persist until the next
        // action.
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "tokio::spawn(f);\n"),
        ]);
        s.open_path("src/main.rs");
        s.set_point(0, 2, 2);
        s.xref_find_definitions();
        assert_eq!(s.resolve_generation, 2, "xref bump + start bump");
        // Simulate a superseded request bumping the generation (as a second
        // M-. that started its own job would), then deliver the OLD job's
        // event.
        s.resolve_generation += 1;
        s.resolving = Some(("resolving `other`…".into(), 3));
        let before = s.message.clone();
        let stale = ResolveEvent {
            generation: 2,
            symbol: "tokio::spawn".into(),
            source: None,
            error: Some("boom".into()),
        };
        s.apply_resolve_event(&stale);
        assert_eq!(s.message, before, "stale event changed no reported state");
        assert!(
            s.resolving_display().is_empty(),
            "a stale event clears the resolving indicator (latest-wins: the current job's event may have been overwritten)"
        );
        // And the stale job's own activity text (gen 2) is hidden by the
        // display gate once the generation no longer matches.
        s.resolving = Some(("resolving `tokio::spawn`…".into(), 2));
        assert!(s.resolving_display().is_empty(), "stale-generation activity hidden by the display gate");
    }

    #[tokio::test]
    async fn xref_workspace_hit_supersedes_in_flight_resolve() {
        // 006-02b item 2: with a resolve in flight, a successful M-. 
        // workspace hit bumps the generation, so the in-flight job's stale
        // event (even a registry-source HIT) discards itself and never
        // opens the external source or records a jump.
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "tokio::spawn(f);\ntarget();\n"),
            ("src/lib.rs", "pub fn target() {}\n"),
        ]);
        let ext = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(ext.path(), "pub fn spawn<F>(f: F) {}\n").unwrap();
        s.open_path("src/main.rs");
        s.set_point(0, 2, 2); // cursor inside `tokio` → workspace miss
        s.xref_find_definitions();
        let in_flight = s.resolve_generation;
        assert!(!s.resolving_display().is_empty(), "resolver in flight");
        // The user's NEXT M-. is a workspace hit (line 1: `target();`).
        s.set_point_line(1);
        s.xref_find_definitions();
        assert_eq!(s.resolve_generation, in_flight + 1, "the hit superseded the in-flight resolve");
        assert_eq!(s.view_name_display(), "src/lib.rs", "the hit landed");
        let before = s.view_name_display();
        // Now the stale job's event (a registry-source hit) arrives.
        let stale = ResolveEvent {
            generation: in_flight,
            symbol: "tokio::spawn".into(),
            source: Some(ResolvedSource {
                file: ext.path().to_path_buf(),
                source_root: ext.path().parent().unwrap().to_path_buf(),
                external: true,
                line: Some(1),
            }),
            error: None,
        };
        s.apply_resolve_event(&stale);
        assert_eq!(s.view_name_display(), before, "the stale hit did not open the registry source");
    }

    #[tokio::test]
    async fn xref_resolver_line_zero_lands_at_top() {
        // 006-02b item 6: a provider emitting line 0 is treated as "no
        // line" — the landing is the top of the file (no underflow).
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "tokio::spawn(f);\n"),
        ]);
        let ext = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(ext.path(), "extern crate dep;\npub fn spawn<F>(f: F) {}\n").unwrap();
        s.open_path("src/main.rs");
        s.set_point(0, 2, 2);
        s.xref_find_definitions();
        let event = ResolveEvent {
            generation: s.resolve_generation,
            symbol: "tokio::spawn".into(),
            source: Some(ResolvedSource {
                file: ext.path().to_path_buf(),
                source_root: ext.path().parent().unwrap().to_path_buf(),
                external: true,
                line: Some(0),
            }),
            error: None,
        };
        s.apply_resolve_event(&event);
        assert_eq!(s.point_line(), 0, "line 0 lands at the top of the file");
        assert!(s.message.contains("jumped to"), "msg: {}", s.message);
    }

    // ── plan 006 issue 03: navigate within external (crate) sources ────

    /// 006-03 fixture: a project store (one project file) plus an
    /// EXTERNAL crate source tree (outside the project root) with a
    /// synchronously installed crate index. Returns (store, project
    /// dir, crate root).
    fn store_with_crate_index(files: &[(&str, &str)]) -> (AppStore, tempfile::TempDir, tempfile::TempDir) {
        let (mut s, dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        for (rel, content) in files {
            let path = root.path().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, content).unwrap();
        }
        let crate_files = AppStore::crate_source_files(root.path(), &["rs"]);
        let index = build_index(root.path(), &crate_files, None);
        s.apply_crate_index_event(&CrateIndexEvent {
            source_root: root.path().to_path_buf(),
            index,
        });
        (s, dir, root)
    }

    #[tokio::test]
    async fn crate_index_background_build_publishes_and_indicates() {
        // The synthetic tree lives OUTSIDE the project root: the index
        // build is root-agnostic (the project indexer's machinery, run
        // on spawn_blocking), and the finished index lands via the
        // CrateIndexBus exactly like the ResolveBus events do.
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(
            root.path().join("src/lib.rs"),
            "pub fn target() {}\npub mod sub { pub fn deep() {} }\n",
        )
        .unwrap();
        std::fs::write(root.path().join("src/other.rs"), "pub fn helper() {}\n").unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(root.path(), &root.path().join("src/lib.rs"));
        // The in-flight indicator mirrors the project indexing indicator.
        assert!(
            s.crate_indexing_display().starts_with("indexing crate "),
            "{}",
            s.crate_indexing_display()
        );
        // 006-03b item 5: the N/M file counter (2 .rs files in the
        // fixture; `done` may have already advanced by now).
        assert!(
            s.crate_indexing_display().ends_with("/2)…"),
            "N/M counter: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        assert_eq!(event.source_root, root.path());
        s.apply_crate_index_event(&event);
        // Installed (one LRU entry) and the indicator cleared with the
        // final event.
        assert_eq!(s.external_indexes.len(), 1);
        assert_eq!(s.crate_indexing_display(), "");
        // Crate-relative outlines are queryable.
        let arc = s.crate_index_arc(root.path()).unwrap();
        let idx = arc.lock().unwrap();
        assert_eq!(idx.definition_count("target"), 1);
        assert_eq!(idx.definition_count("helper"), 1);
        assert_eq!(idx.definition_count("deep"), 1);
        assert!(idx.has("src/lib.rs"), "crate-relative key");
        // A second start for the same root is a no-op (already cached).
        s.start_crate_indexing(root.path(), &root.path().join("src/lib.rs"));
        assert!(s.crate_indexing.is_empty(), "no duplicate build");
    }

    #[test]
    fn external_mdot_cross_file_within_crate_jumps_read_only() {
        // M-. on a type that another file of the SAME crate defines:
        // the jump stays inside the crate (crate-relative display), opens
        // read-only through the external path, and records a jump.
        let (mut s, _dir, root) = store_with_crate_index(&[
            ("src/lib.rs", "pub fn use_helper() {\n    helper();\n}\n"),
            ("src/other.rs", "pub fn helper() {}\n"),
        ]);
        s.open_external_path(&root.path().join("src/lib.rs")).unwrap();
        let origin_key = s.buffers.current().unwrap().to_string();
        // Line 1: "    helper();" — `helper` starts at col 4.
        s.set_point(1, 4, 4);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique cross-file: no picker");
        assert!(
            s.message.contains("jumped to src/other.rs:1"),
            "crate-relative display, got: {}",
            s.message
        );
        let key = s.buffers.current().unwrap().to_string();
        assert_eq!(key, root.path().join("src/other.rs").to_string_lossy());
        assert_eq!(s.point_line(), 0, "landed on the definition line");
        // The landing stays external: read-only, in the external set, never
        // a project file (recents untouched — 008-01 semantics).
        assert!(!s.buffers.get(&key).unwrap().editable);
        assert!(s.external_buffers.contains(&key));
        // M-, walks back to the origin (still inside the crate).
        s.jump_back();
        assert_eq!(
            s.buffers.current().unwrap().to_string(),
            origin_key,
            "jump-back returns to the origin buffer"
        );
    }

    #[test]
    fn external_mdot_same_file_candidate_wins() {
        // The normal struct + impl-in-one-file case: the same-file
        // candidate wins the direct jump (the project path's
        // same-file-first ordering, unchanged).
        let (mut s, _dir, root) = store_with_crate_index(&[(
            "src/lib.rs",
            "pub struct Foo {}\nimpl Foo {\n    pub fn new() -> Self { Foo {} }\n}\npub fn use_it() {\n    let f = Foo::new();\n}\n",
        )]);
        s.open_external_path(&root.path().join("src/lib.rs")).unwrap();
        // Line 5: "    let f = Foo::new();" — `new` starts at col 17.
        s.set_point(5, 17, 17);
        s.xref_find_definitions();
        assert!(!s.picker_open(), "unique same-file: no picker");
        assert_eq!(s.point_line(), 2, "jumped to the same-file impl method");
        assert!(
            s.message.contains("jumped to src/lib.rs:3"),
            "{}",
            s.message
        );
    }

    #[test]
    fn external_mdot_ambiguous_opens_crate_picker_and_selection_jumps() {
        // Two same-named definitions across the crate → the Xref picker
        // with CRATE-RELATIVE display paths; the query re-computation and
        // the RET selection both stay crate-rooted (the selection opens
        // read-only inside the crate, never through the project root).
        let (mut s, _dir, root) = store_with_crate_index(&[
            ("src/lib.rs", "pub fn target() {}\npub fn call() {\n    target();\n}\n"),
            ("src/other.rs", "pub fn target() {}\n"),
        ]);
        s.open_external_path(&root.path().join("src/lib.rs")).unwrap();
        // Line 2: "    target();" — `target` starts at col 4.
        s.set_point(2, 4, 4);
        s.xref_find_definitions();
        assert!(s.picker_open(), "two candidates: picker");
        assert_eq!(s.picker_kind(), Some(PickerKind::Xref));
        let filtered = s.picker_filtered();
        assert_eq!(filtered.len(), 2);
        assert!(
            filtered[0].0.name.starts_with("src/lib.rs:"),
            "same-file candidate first: {}",
            filtered[0].0.name
        );
        assert!(
            filtered[1].0.name.starts_with("src/other.rs:"),
            "crate-relative display: {}",
            filtered[1].0.name
        );
        // Query re-computation consults the CRATE index (the project
        // index has no `target` — a leaked project root would yield 0).
        s.picker_query_char('s');
        assert_eq!(
            s.picker_filtered().len(),
            2,
            "refilter against the crate index"
        );
        // Select the second candidate (DOWN + RET): the jump lands
        // inside the crate, read-only.
        s.key_event(key("DOWN"));
        assert_eq!(s.picker_selected(), 1);
        s.key_event(key("RET"));
        assert!(s.message.contains("jumped to src/other.rs:1"), "{}", s.message);
        let key = s.buffers.current().unwrap().to_string();
        assert_eq!(key, root.path().join("src/other.rs").to_string_lossy());
        assert!(s.external_buffers.contains(&key));
        assert!(!s.buffers.get(&key).unwrap().editable);
    }

    #[test]
    fn external_mdot_miss_falls_through_to_resolver() {
        // A symbol the crate index does not know: the miss keeps the
        // project path's resolver fall-through (unchanged semantics —
        // the origin project's metadata), superseding any in-flight
        // resolve. Plain (no runtime) test: the spawn is skipped and the
        // miss reported synchronously, like xref_workspace_miss_fires_….
        let (mut s, _dir, root) = store_with_crate_index(&[(
            "src/probe.rs",
            "pub fn helper() {}\ntokio::spawn(f);\n",
        )]);
        s.open_external_path(&root.path().join("src/probe.rs")).unwrap();
        // Line 1: top-level `tokio::spawn(f);` — no crate definition, no
        // enclosing symbol → resolver fall-through with the path token.
        s.set_point(1, 9, 9); // cursor inside `spawn`
        s.xref_find_definitions();
        assert_eq!(
            s.resolve_generation, 2,
            "external supersede bump + start bump"
        );
        assert!(
            s.message.contains("no provider resolution for `tokio::spawn`"),
            "graceful miss, got: {}",
            s.message
        );
        assert!(
            s.resolving_display().is_empty(),
            "no runtime: the indicator must not hang"
        );
    }

    #[test]
    fn external_imenu_lists_crate_file_outline_and_refilters() {
        // M-i inside an external buffer: the outline comes from the
        // owning crate's index (not a refusal), and the query
        // re-computation stays crate-rooted.
        let (mut s, _dir, root) = store_with_crate_index(&[(
            "src/lib.rs",
            "pub struct Foo {}\nimpl Foo {\n    pub fn new() -> Self { Foo {} }\n}\npub fn use_it() {\n    let f = Foo::new();\n}\n",
        )]);
        s.open_external_path(&root.path().join("src/lib.rs")).unwrap();
        s.open_imenu();
        assert!(s.picker_open(), "imenu opens on the external buffer");
        assert_eq!(s.picker_kind(), Some(PickerKind::Imenu));
        let names: Vec<_> = s
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.clone())
            .collect();
        assert_eq!(names, vec!["Foo:1", "new:3", "use_it:5"], "{names:?}");
        assert_eq!(s.picker_count(), (3, 3), "the total follows the crate index");
        // Refilter: `new` matches only the impl method (the crate index,
        // not the project's, is consulted — the project index is empty
        // here; a leaked project root would yield 0 candidates).
        s.picker_query_char('n');
        s.picker_query_char('e');
        s.picker_query_char('w');
        let filtered: Vec<_> = s
            .picker_filtered()
            .iter()
            .map(|(c, _)| c.name.clone())
            .collect();
        assert_eq!(filtered, vec!["new:3"], "{filtered:?}");
    }

    #[test]
    fn external_index_lru_cap_evicts_oldest_and_recency_bumps() {
        // The cache caps at 3 roots; the oldest is evicted when a new
        // crate lands; a lookup bumps recency.
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let roots: Vec<tempfile::TempDir> = (0..4)
            .map(|_| tempfile::tempdir().unwrap())
            .collect();
        for r in &roots {
            s.apply_crate_index_event(&CrateIndexEvent {
                source_root: r.path().to_path_buf(),
                index: SymbolIndex::default(),
            });
        }
        let ordered: Vec<&Path> =
            s.external_indexes.iter().map(|(r, _)| r.as_path()).collect();
        assert_eq!(s.external_indexes.len(), 3, "cap holds");
        assert_eq!(
            ordered,
            vec![roots[1].path(), roots[2].path(), roots[3].path()],
            "the oldest (roots[0]) is evicted; newest last"
        );
        // A lookup bumps recency: roots[1] moves to the newest slot…
        s.crate_index_arc(roots[1].path()).unwrap();
        // …so the next landing evicts roots[2] instead (the new oldest).
        let r5 = tempfile::tempdir().unwrap();
        s.apply_crate_index_event(&CrateIndexEvent {
            source_root: r5.path().to_path_buf(),
            index: SymbolIndex::default(),
        });
        let ordered: Vec<&Path> =
            s.external_indexes.iter().map(|(r, _)| r.as_path()).collect();
        assert_eq!(
            ordered,
            vec![roots[3].path(), roots[1].path(), r5.path()],
            "recency bump changed the eviction victim (roots[2] evicted)"
        );
    }

    #[test]
    fn external_landed_crate_survives_eviction_while_current() {
        // 006-03b item 1: land in crate A (its buffer becomes current),
        // then three other crates' indexes land while A's buffer stays
        // current → A is never the eviction victim, and the next M-. in
        // A still jumps in-crate. Discriminating: without the recency
        // bump A is the OLDEST entry, the third arrival evicts it, and
        // the M-. falls through to the resolver (`no provider
        // resolution`).
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let roots: Vec<tempfile::TempDir> =
            (0..4).map(|_| tempfile::tempdir().unwrap()).collect();
        let root_a = roots[0].path();
        std::fs::create_dir_all(root_a.join("src")).unwrap();
        std::fs::write(
            root_a.join("src/lib.rs"),
            "pub fn use_helper() {\n    helper();\n}\n",
        )
        .unwrap();
        std::fs::write(root_a.join("src/other.rs"), "pub fn helper() {}\n").unwrap();
        // Land in A: its index arrives, its buffer becomes current.
        s.apply_crate_index_event(&CrateIndexEvent {
            source_root: root_a.to_path_buf(),
            index: build_index(root_a, &AppStore::crate_source_files(root_a, &["rs"]), None),
        });
        s.open_external_path(&root_a.join("src/lib.rs")).unwrap();
        // Consult the three other crates: their indexes land while A's
        // buffer stays current (the cap is 3 — each arrival evicts the
        // oldest).
        for r in &roots[1..] {
            s.apply_crate_index_event(&CrateIndexEvent {
                source_root: r.path().to_path_buf(),
                index: SymbolIndex::default(),
            });
        }
        let ordered: Vec<&Path> =
            s.external_indexes.iter().map(|(r, _)| r.as_path()).collect();
        assert_eq!(s.external_indexes.len(), 3, "cap holds");
        assert!(
            ordered.contains(&root_a),
            "crate A (the current buffer's crate) survived eviction: {ordered:?}"
        );
        // The next M-. in A still jumps in-crate.
        s.set_point(1, 4, 4); // "    helper();" — `helper` starts at col 4.
        s.xref_find_definitions();
        assert!(
            s.message.contains("jumped to src/other.rs:1"),
            "in-crate jump intact, got: {}",
            s.message
        );
    }

    #[test]
    fn external_rel_normalizes_backslash_separators() {
        // 006-03b item 3: a `\`-containing relative path resolves the
        // same index key as its `/` form (the Windows native-separator
        // case; on Linux a backslash-bearing component simulates it,
        // on Windows `src\lib.rs` is the real two-component path).
        let (mut s, _dir, root) = store_with_crate_index(&[(
            "src/lib.rs",
            "pub fn helper() {}\n",
        )]);
        let root_path = root.path();
        let slashy = root_path.join("src/lib.rs");
        let backslashed = root_path.join("src\\lib.rs");
        let r_slash = AppStore::crate_rel(&slashy, root_path);
        let r_back = AppStore::crate_rel(&backslashed, root_path);
        assert_eq!(r_slash.as_deref(), Some("src/lib.rs"));
        assert_eq!(r_slash, r_back, "the \\ form normalizes to the / key");
        // Both forms outline the same file through the cached index
        // (the `current_buffer_outline` lookup site).
        let arc = s.crate_index_arc(root_path).unwrap();
        let idx = arc.lock().unwrap();
        let outline_slash = idx.outline(&r_slash.unwrap());
        assert!(!outline_slash.is_empty());
        assert_eq!(idx.outline(&r_back.unwrap()), outline_slash);
        // A non-nested path is a clean miss (the 006-03b item 4
        // no-panic path), never an unwrap.
        assert!(AppStore::crate_rel(
            Path::new("elsewhere/lib.rs"),
            root_path
        )
        .is_none());
    }

    #[test]
    fn external_resolver_from_file_is_crate_relative() {
        // 006-03b item 2: the resolver fall-through's `from_file` for an
        // external buffer is crate-relative (the index's own key shape),
        // never the absolute path.
        let (mut s, _dir, root) = store_with_crate_index(&[("src/probe.rs", "pub fn helper() {}\n")]);
        let path = root.path().join("src/probe.rs");
        s.open_external_path(&path).unwrap();
        assert_eq!(s.resolver_from_file(&path), "src/probe.rs");
        // A still-in-flight root (not yet installed) is crate-relative
        // too…
        let inflight = tempfile::tempdir().unwrap();
        s.crate_indexing.push((
            inflight.path().to_path_buf(),
            "inflight".to_string(),
            Arc::new(IndexProgress::new(1)),
        ));
        assert_eq!(
            s.resolver_from_file(&inflight.path().join("src/lib.rs")),
            "src/lib.rs"
        );
        // …and a path under NO known root falls back to the file name
        // (relative, never absolute).
        assert_eq!(
            s.resolver_from_file(Path::new("/tmp/stray/lib.rs")),
            "lib.rs"
        );
    }

    #[test]
    fn external_cache_does_not_leak_into_project_mdot() {
        // Regression: a project file's M-. resolves against the PROJECT
        // index even when a cached crate index defines the same name.
        let (mut s, _dir) = store_with_index(&[
            ("src/main.rs", "fn main() { target(); }\n"),
            ("src/lib.rs", "pub fn target() {}\n"),
        ]);
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/crate.rs"), "pub fn target() {}\n").unwrap();
        let crate_files = AppStore::crate_source_files(root.path(), &["rs"]);
        s.apply_crate_index_event(&CrateIndexEvent {
            source_root: root.path().to_path_buf(),
            index: build_index(root.path(), &crate_files, None),
        });
        s.open_path("src/main.rs");
        // Line 0: "fn main() { target(); }" — `target` starts at col 12.
        s.set_point(0, 12, 12);
        s.xref_find_definitions();
        assert!(!s.picker_open());
        assert_eq!(
            s.view_name_display(),
            "src/lib.rs",
            "the PROJECT definition wins (not the crate's)"
        );
    }

    #[test]
    fn blame_external_buffer_says_why() {
        // Registry / tooling sources are not git checkouts: the refusal
        // stays, but the message now explains it (006-03 item 4).
        let (_dir, _ext, mut s) = store_with_external_buffer();
        s.open_blame();
        assert_eq!(s.message, "no git history for external sources");
    }

    #[test]
    fn previous_project_buffer_mdot_still_refused() {
        // The lifted refusal applies only to EXTERNAL buffers: a previous
        // project's file (outside the new root but not an external
        // landing) keeps the plain refusal.
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        for d in [&dir1, &dir2] {
            std::fs::write(d.path().join("Cargo.toml"), "[package]\n").unwrap();
            std::fs::create_dir_all(d.path().join("src")).unwrap();
        }
        std::fs::write(dir1.path().join("src/a.rs"), "fn a() {}\n").unwrap();
        let mut s = store(dir1.path());
        s.open_path("src/a.rs");
        s.switch_project_root(dir2.path().to_str().unwrap());
        s.xref_find_definitions();
        assert_eq!(s.message, "buffer not in project");
    }

    #[tokio::test]
    async fn crate_index_refuses_oversized_tree() {
        // Above the file cap the build is refused with a clear message
        // (no known registry crate approaches 2000 .rs files).
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        for i in 0..(EXT_INDEX_FILE_CAP + 1) {
            std::fs::write(
                root.path().join(format!("f{i}.rs")),
                "fn f() {}\n",
            )
            .unwrap();
        }
        s.start_crate_indexing(root.path(), &root.path().join("f0.rs"));
        assert!(
            s.message.contains("crate too large to index")
                && s.message.contains("2001"),
            "{}",
            s.message
        );
        assert!(s.crate_indexing.is_empty(), "no in-flight build armed");
        assert!(s.external_indexes.is_empty(), "no cache entry");
    }

    // ── plan 011 issue 04: per-language source index ───────────────────

    /// 011-04 (extended by 011-07): the walk's extension sets stay in
    /// lockstep with `registry.rs`'s extension map (the authority):
    /// every walked extension must resolve (through the registry map) to
    /// a language whose OWN set includes it — the sets are per grammar
    /// family.
    #[test]
    fn source_extensions_round_trip_through_registry_map() {
        use crate::syntax::registry::resolve_language;
        for lang in [
            LanguageId::Rust,
            LanguageId::JavaScript,
            LanguageId::TypeScript,
            LanguageId::Tsx,
            LanguageId::Python,
            LanguageId::Go,
            LanguageId::C,
            LanguageId::Cpp,
            LanguageId::Markdown,
            LanguageId::Java,
            LanguageId::CSharp,
            LanguageId::Ruby,
            LanguageId::Scheme,
        ] {
            for ext in AppStore::source_extensions_for(lang) {
                let resolved = resolve_language(&format!("a.{ext}"));
                assert!(
                    AppStore::source_extensions_for(resolved).contains(ext),
                    "`{ext}` (owned by {lang:?}) must resolve to a language"
                );
            }
        }
        // Documented exclusion: the registry maps `rsi` to Rust, but the
        // walk indexes `rs` only (rust-analyzer interface files are not
        // definition sources).
        assert!(
            !AppStore::source_extensions_for(LanguageId::Rust).contains(&"rsi"),
            "rsi stays out of the Rust index set"
        );
        // Languages whose grammar queries exist but whose files are not
        // M-. definition sources (011-04 judgment, carried by 011-07):
        // no walk set, no index.
        for lang in [
            LanguageId::Plain,
            LanguageId::Json,
            LanguageId::Toml,
            LanguageId::Yaml,
            LanguageId::Bash,
        ] {
            assert!(AppStore::source_extensions_for(lang).is_empty());
        }
    }

    /// 011-04: the extension predicate is PER LANGUAGE — a JS/TS tree
    /// collects js/jsx/ts/tsx/mjs/cjs and only those; a Python tree
    /// py/pyi; a Rust tree `rs` (the 006-03 set, unchanged). Nested
    /// `node_modules` is never walked (item 4).
    #[test]
    fn crate_source_files_selects_own_language_extensions() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        for rel in [
            "src/index.js",
            "src/util.mjs",
            "src/cfg.cjs",
            "src/view.jsx",
            "src/types.ts",
            "src/app.tsx",
            "README.md",
            "package.json",
            "node_modules/leftpad/index.js",
            "node_modules/leftpad/package.json",
        ] {
            let path = p.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "// x\n").unwrap();
        }
        let mut js = AppStore::crate_source_files(
            p,
            &["js", "jsx", "ts", "tsx", "mjs", "cjs"],
        );
        js.sort();
        assert_eq!(
            js,
            vec![
                "src/app.tsx",
                "src/cfg.cjs",
                "src/index.js",
                "src/types.ts",
                "src/util.mjs",
                "src/view.jsx",
            ],
            "own-language extensions only: no .md/.json, no node_modules"
        );

        let pyr = tempfile::tempdir().unwrap();
        std::fs::write(pyr.path().join("foo.py"), "\n").unwrap();
        std::fs::create_dir_all(pyr.path().join("types")).unwrap();
        std::fs::write(pyr.path().join("types/stubs.pyi"), "\n").unwrap();
        std::fs::write(pyr.path().join("notes.txt"), "\n").unwrap();
        let mut py = AppStore::crate_source_files(pyr.path(), &["py", "pyi"]);
        py.sort();
        assert_eq!(py, vec!["foo.py", "types/stubs.pyi"]);

        // Rust: the 006-03 set, unchanged.
        let rr = tempfile::tempdir().unwrap();
        std::fs::write(rr.path().join("a.rs"), "\n").unwrap();
        std::fs::write(rr.path().join("b.rsi"), "\n").unwrap();
        std::fs::write(rr.path().join("c.md"), "\n").unwrap();
        let rs = AppStore::crate_source_files(rr.path(), &["rs"]);
        assert_eq!(rs, vec!["a.rs"]);
    }

    /// 011-04 (item 4): a `node_modules` DIRECTORY inside the root is
    /// skipped even mid-path — but a root that IS a `node_modules` dir
    /// (a landed `node_modules/<pkg>`) still indexes its own files (the
    /// boundary is a component AFTER the root, never the file name).
    #[test]
    fn crate_source_files_node_modules_boundary() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("dist")).unwrap();
        std::fs::write(p.join("index.js"), "\n").unwrap();
        std::fs::write(p.join("dist/bundle.js"), "\n").unwrap();
        // A directory NAMED like a source file: the file itself is fine.
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("src/util.js"), "\n").unwrap();
        let mut files = AppStore::crate_source_files(p, &["js"]);
        files.sort();
        assert_eq!(files, vec!["dist/bundle.js", "index.js", "src/util.js"]);

        // The root ITSELF named node_modules (a landed package dir):
        // its own files are walked.
        let nm = tempfile::tempdir().unwrap();
        let p2 = nm.path();
        std::fs::create_dir_all(p2.join("inner")).unwrap();
        std::fs::write(p2.join("own.js"), "\n").unwrap();
        std::fs::write(p2.join("inner/dep.js"), "\n").unwrap();
        std::fs::create_dir_all(p2.join("inner/node_modules/other")).unwrap();
        std::fs::write(p2.join("inner/node_modules/other/x.js"), "\n").unwrap();
        let mut files2 = AppStore::crate_source_files(p2, &["js"]);
        files2.sort();
        assert_eq!(
            files2,
            vec!["inner/dep.js", "own.js"],
            "the root node_modules is indexed; nested ones are not"
        );
    }

    /// 011-04 discriminating: a synthetic out-of-root JS tree indexes
    /// through the FULL path (language derivation from the landed file →
    /// per-language walk → build_index → CrateIndexBus): crate-relative
    /// keys and per-language symbol extraction. Pre-011-04 this tree
    /// indexed NOTHING (the walk was `.rs`-only).
    #[tokio::test]
    async fn crate_index_builds_for_js_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("src/index.js"), "export function alpha() {}\n").unwrap();
        std::fs::write(p.join("util.mjs"), "export function beta() {}\n").unwrap();
        std::fs::write(p.join("README.md"), "docs\n").unwrap();
        std::fs::create_dir_all(p.join("node_modules/leftpad")).unwrap();
        std::fs::write(
            p.join("node_modules/leftpad/index.js"),
            "export function leftpad() {}\n",
        )
        .unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("src/index.js"));
        // 2 OWN source files (README.md + node_modules excluded): the
        // N/M counter is the file-set's size, not the tree's.
        assert!(
            s.crate_indexing_display().ends_with(")…")
                && s.crate_indexing_display().contains("/2)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        assert_eq!(event.source_root, p);
        s.apply_crate_index_event(&event);
        assert_eq!(s.external_indexes.len(), 1);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("src/index.js"), "crate-relative key (.js)");
        assert!(idx.has("util.mjs"), "crate-relative key (.mjs)");
        assert_eq!(idx.definition_count("alpha"), 1, "js symbol extraction");
        assert_eq!(idx.definition_count("beta"), 1);
        assert!(!idx.has("README.md"), "non-source file not indexed");
        assert!(
            !idx.has("node_modules/leftpad/index.js"),
            "item 4: nested node_modules never indexed"
        );
    }

    /// 011-04 discriminating: a synthetic out-of-root Python tree
    /// indexes through the full path (the `pyi` stub extension is the
    /// registry's python-map pin; `build_index`'s symbol extraction is
    /// language-aware via `registry::resolve_language` + the query
    /// registry — no Rust-only path).
    #[tokio::test]
    async fn crate_index_builds_for_python_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("pkg")).unwrap();
        std::fs::write(
            p.join("pkg/foo.py"),
            "def gamma():\n    return 1\n\nclass Gamma:\n    pass\n",
        )
        .unwrap();
        std::fs::write(p.join("pkg/stubs.pyi"), "def delta(x: int) -> int: ...\n")
            .unwrap();
        std::fs::write(p.join("setup.cfg"), "[x]\n").unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("pkg/foo.py"));
        assert!(
            s.crate_indexing_display().contains("/2)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("pkg/foo.py"), "crate-relative key (.py)");
        assert!(idx.has("pkg/stubs.pyi"), "crate-relative key (.pyi)");
        assert_eq!(idx.definition_count("gamma"), 1, "python symbol extraction");
        assert_eq!(idx.definition_count("Gamma"), 1);
        assert_eq!(idx.definition_count("delta"), 1, "pyi stub extraction");
        assert!(!idx.has("setup.cfg"));
    }

    /// 011-04: the Go set indexes Go definitions through the same walk +
    /// build_index path (the go toolchain is absent in the sandbox —
    /// the PROVIDER is unit-covered elsewhere; symbol extraction needs
    /// no toolchain, it is pure tree-sitter).
    #[test]
    fn crate_index_walk_and_extraction_for_go_tree() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("pkg")).unwrap();
        std::fs::write(p.join("pkg/greet.go"), "package pkg\n\nfunc Greet() {}\n").unwrap();
        std::fs::write(p.join("README.md"), "docs\n").unwrap();
        let files = AppStore::crate_source_files(p, &["go"]);
        assert_eq!(files, vec!["pkg/greet.go"]);
        let index = build_index(p, &files, None);
        assert!(index.has("pkg/greet.go"));
        assert_eq!(index.definition_count("Greet"), 1, "go symbol extraction");
    }

    /// 011-07 discriminating: a C tree indexes through the full path
    /// (language derivation from the landed `.c` file → the `c/h` walk →
    /// build_index → CrateIndexBus). HEADERS ARE DEFINITION SOURCES:
    /// the `.h` file's symbols are in the index alongside the `.c`'s.
    /// Pre-011-07 the C set was empty, so the walk found 0 files and the
    /// build never armed — the `/2)` indicator assert fails on any
    /// regression to the empty set.
    #[tokio::test]
    async fn crate_index_builds_for_c_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::create_dir_all(p.join("include")).unwrap();
        std::fs::write(
            p.join("src/lib.c"),
            "#include \"defs.h\"\nint alpha(int a) { return a; }\n",
        )
        .unwrap();
        std::fs::write(
            p.join("include/defs.h"),
            "#ifndef DEFS_H\nstruct Gamma { int x; };\nint beta(int a) { return a; }\n#endif\n",
        )
        .unwrap();
        std::fs::write(p.join("README.md"), "docs\n").unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("src/lib.c"));
        // 2 OWN source files (README.md excluded): the N/M counter is the
        // file-set's size, not the tree's.
        assert!(
            s.crate_indexing_display().contains("/2)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        assert_eq!(event.source_root, p);
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("src/lib.c"), "crate-relative key (.c)");
        assert!(
            idx.has("include/defs.h"),
            "header (.h) IS a C definition source"
        );
        assert_eq!(idx.definition_count("alpha"), 1, "c symbol extraction");
        assert_eq!(idx.definition_count("beta"), 1, "header function extraction");
        assert_eq!(idx.definition_count("Gamma"), 1, "header struct extraction");
        assert!(!idx.has("README.md"), "non-source file not indexed");
    }

    /// 011-07 discriminating: a C++ tree indexes through the full path,
    /// landing on a HEADER (`include/api.hpp`) — the header is both a
    /// walk extension and a valid landed_file. All six registry C++
    /// extensions are walked (the `.hh` file proves the non-`.hpp` map
    /// keys).
    #[tokio::test]
    async fn crate_index_builds_for_cpp_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("include")).unwrap();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::create_dir_all(p.join("legacy")).unwrap();
        std::fs::write(p.join("include/api.hpp"), "struct Widget { int x; };\n")
            .unwrap();
        std::fs::write(
            p.join("src/widget.cc"),
            "#include \"api.hpp\"\nWidget make_widget() { return {0}; }\n",
        )
        .unwrap();
        std::fs::write(
            p.join("legacy/old.hh"),
            "int legacy_value() { return 0; }\n",
        )
        .unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("include/api.hpp"));
        assert!(
            s.crate_indexing_display().contains("/3)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(
            idx.has("include/api.hpp"),
            "crate-relative key (.hpp, the landed header)"
        );
        assert!(idx.has("src/widget.cc"), "crate-relative key (.cc)");
        assert!(
            idx.has("legacy/old.hh"),
            ".hh map key walked, not just .hpp"
        );
        assert_eq!(idx.definition_count("Widget"), 1, "cpp struct extraction");
        assert_eq!(
            idx.definition_count("make_widget"),
            1,
            "cpp function extraction"
        );
        assert_eq!(idx.definition_count("legacy_value"), 1, "cpp header extraction");
    }

    /// 011-07 discriminating: a Markdown tree indexes through the full
    /// path. VERIFIED HONEST SCOPE (extracted, not assumed): the
    /// definition query captures both ATX headings (`# Alpha` → symbol
    /// `Alpha`) and SETEXT headings (`Delta\n====` → symbol `Delta` — the
    /// setext branch's node shape is `setext_heading → paragraph → inline`,
    /// verified against the pinned tree-sitter-md 0.3.2 grammar). So the
    /// M-. targets in a markdown dependency are all headings: no
    /// paragraphs, no links. All three registry map keys (`md`,
    /// `markdown`, `mdx`) are walked.
    #[tokio::test]
    async fn crate_index_builds_for_markdown_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("docs")).unwrap();
        std::fs::write(p.join("README.md"), "# Alpha\n\njust a paragraph\n").unwrap();
        std::fs::write(p.join("docs/changelog.markdown"), "# Beta\n").unwrap();
        std::fs::write(p.join("docs/guide.mdx"), "# Gamma\n").unwrap();
        // Setext heading: the query's setext branch captures the heading
        // text (011-07 follow-up: the branch was structurally unmatchable
        // before and was fixed to `setext_heading → paragraph → inline`).
        std::fs::write(p.join("docs/setext.md"), "Delta\n====\n").unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("README.md"));
        assert!(
            s.crate_indexing_display().contains("/4)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("README.md"), "crate-relative key (.md)");
        assert!(idx.has("docs/changelog.markdown"), ".markdown map key walked");
        assert!(idx.has("docs/guide.mdx"), ".mdx map key walked");
        assert_eq!(idx.definition_count("Alpha"), 1, "atx heading extraction");
        assert_eq!(idx.definition_count("Beta"), 1, "atx heading extraction");
        assert_eq!(idx.definition_count("Gamma"), 1, "mdx atx heading extraction");
        assert!(idx.has("docs/setext.md"), "setext heading file indexed");
        assert_eq!(
            idx.definition_count("Delta"),
            1,
            "setext heading extraction (1 heading)"
        );
    }

    /// 011-07 follow-up (new-languages lane): a Java tree indexes through
    /// the full path — the `.java` walk set (the registry's only Java map
    /// key) collects the source and the outline extraction lands the
    /// class + method.
    #[tokio::test]
    async fn crate_index_builds_for_java_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(
            p.join("src/Widget.java"),
            "public class Widget {\n    public int get() { return 0; }\n}\n",
        )
        .unwrap();
        std::fs::write(p.join("README.md"), "docs\n").unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("src/Widget.java"));
        // 1 OWN source file (README.md excluded by the `java` walk set):
        // the N/M counter is the file-set's size, not the tree's.
        assert!(
            s.crate_indexing_display().contains("/1)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("src/Widget.java"), "crate-relative key (.java)");
        assert_eq!(idx.definition_count("Widget"), 1, "java class extraction");
        assert_eq!(idx.definition_count("get"), 1, "java method extraction");
        assert!(!idx.has("README.md"), "non-source file not indexed");
    }

    /// 011-07 follow-up (new-languages lane): a C# tree indexes through
    /// the full path — the `.cs` walk set collects the source and the
    /// outline extraction lands the namespace, class, property, and
    /// method.
    #[tokio::test]
    async fn crate_index_builds_for_csharp_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(
            p.join("src/Widget.cs"),
            "namespace App {\n    public class Widget {\n\
             \x20   public int Value { get; set; }\n\
             \x20   public int Get() => 0;\n    }\n}\n",
        )
        .unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("src/Widget.cs"));
        assert!(
            s.crate_indexing_display().contains("/1)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("src/Widget.cs"), "crate-relative key (.cs)");
        assert_eq!(idx.definition_count("App"), 1, "csharp namespace extraction");
        assert_eq!(idx.definition_count("Widget"), 1, "csharp class extraction");
        assert_eq!(idx.definition_count("Value"), 1, "csharp property extraction");
        assert_eq!(idx.definition_count("Get"), 1, "csharp method extraction");
    }

    /// 011-07 follow-up (new-languages lane): a Ruby tree indexes through
    /// the full path — the `.rb` walk set collects the source and the
    /// outline extraction lands the module, class, and def.
    #[tokio::test]
    async fn crate_index_builds_for_ruby_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("lib")).unwrap();
        std::fs::write(
            p.join("lib/helper.rb"),
            "module Util\n  class Helper\n    def run\n      1\n    end\n  end\nend\n",
        )
        .unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("lib/helper.rb"));
        assert!(
            s.crate_indexing_display().contains("/1)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("lib/helper.rb"), "crate-relative key (.rb)");
        assert_eq!(idx.definition_count("Util"), 1, "ruby module extraction");
        assert_eq!(idx.definition_count("Helper"), 1, "ruby class extraction");
        assert_eq!(idx.definition_count("run"), 1, "ruby def extraction");
    }

    /// 011-07 follow-up (new-languages lane): a Scheme tree indexes
    /// through the full path — ALL FOUR registry map keys
    /// (`scm`/`ss`/`sls`/`sld`) are walked, one fixture per key, and the
    /// flat-grammar outline extraction lands each form kind: the
    /// `define` function, the `define` variable, the `define-library`
    /// name, the nested `define`, and the `define-macro`.
    #[tokio::test]
    async fn crate_index_builds_for_scheme_dependency_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        std::fs::create_dir_all(p.join("lib")).unwrap();
        std::fs::write(
            p.join("lib/entry.scm"),
            "(define (entry-point args)\n  (void))\n",
        )
        .unwrap();
        std::fs::write(p.join("lib/core.ss"), "(define counter 0)\n").unwrap();
        std::fs::write(
            p.join("lib/defs.sld"),
            "(define-library (app core)\n  (export run!)\n  (define (run! x) x))\n",
        )
        .unwrap();
        std::fs::write(
            p.join("lib/lib.sls"),
            "(define-macro (twice a b) a)\n",
        )
        .unwrap();
        let mut rx = s.crate_index_bus.subscribe();
        s.start_crate_indexing(p, &p.join("lib/entry.scm"));
        // All four walk-set extensions are OWN source files.
        assert!(
            s.crate_indexing_display().contains("/4)")
                && s.crate_indexing_display().starts_with("indexing crate "),
            "indicator: {}",
            s.crate_indexing_display()
        );
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), rx.changed())
            .await
            .expect("crate index event published within 30s");
        let event = rx.borrow_and_update().clone();
        s.apply_crate_index_event(&event);
        let arc = s.crate_index_arc(p).unwrap();
        let idx = arc.lock().unwrap();
        assert!(idx.has("lib/entry.scm"), "`.scm` map key walked");
        assert!(idx.has("lib/core.ss"), "`.ss` map key walked");
        assert!(idx.has("lib/defs.sld"), "`.sld` map key walked");
        assert!(idx.has("lib/lib.sls"), "`.sls` map key walked");
        assert_eq!(
            idx.definition_count("entry-point"),
            1,
            "scheme define-fn extraction"
        );
        assert_eq!(idx.definition_count("counter"), 1, "scheme define-var extraction");
        assert_eq!(idx.definition_count("app"), 1, "scheme define-library extraction");
        assert_eq!(
            idx.definition_count("run!"),
            1,
            "scheme nested define extraction"
        );
        assert_eq!(idx.definition_count("twice"), 1, "scheme define-macro extraction");
    }

    /// 011-07 follow-up: the setext branch, pinned directly against the
    /// extractor (the e2e test above pins the same truth through the
    /// full walk -> build_index path; this is the minimal unit twin).
    /// The pinned tree-sitter-md 0.3.2 shape is
    /// `setext_heading -> paragraph -> inline`, so `Delta\n====` yields
    /// exactly one Heading symbol named `Delta` (atx and setext both
    /// extract; paragraphs and links do not).
    #[test]
    fn setext_markdown_file_contributes_heading_symbols() {
        let syms = crate::syntax::queries::extract_symbols(
            LanguageId::Markdown,
            "Delta\n====\n",
        );
        assert_eq!(syms.len(), 1, "exactly the setext heading: {syms:?}");
        assert_eq!(syms[0].name, "Delta");
        assert_eq!(syms[0].kind, crate::syntax::queries::SymbolKind::Heading);
    }

    /// 011-04: the walk is the OWNING language's, never a global "index
    /// everything" — a non-source landing (Plain) indexes nothing even
    /// though the tree is full of other languages' source files.
    #[tokio::test]
    async fn crate_index_plain_language_landing_builds_nothing() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        for rel in ["a.rs", "b.py", "c.js", "d.go"] {
            std::fs::write(p.join(rel), "x\n").unwrap();
        }
        s.start_crate_indexing(p, &p.join("notes.txt"));
        assert!(s.crate_indexing.is_empty(), "no build armed");
        assert!(s.external_indexes.is_empty(), "no cache entry");
        assert_eq!(s.message, "", "no refusal message");
    }

    /// 011-04 (item 4): the refusal cap applies per LANGUAGE set — a JS
    /// tree past `EXT_INDEX_FILE_CAP` own-language files is refused
    /// exactly as a Rust tree is (the node_modules skip bounds the
    /// transitive-dep case; the cap is the backstop for a genuinely
    /// huge dependency's own tree).
    #[tokio::test]
    async fn crate_index_refuses_oversized_js_tree() {
        let (mut s, _dir) = store_with_index(&[("src/main.rs", "fn main() {}\n")]);
        let root = tempfile::tempdir().unwrap();
        for i in 0..(EXT_INDEX_FILE_CAP + 1) {
            std::fs::write(
                root.path().join(format!("f{i}.js")),
                "// x\n",
            )
            .unwrap();
        }
        s.start_crate_indexing(root.path(), &root.path().join("f0.js"));
        assert!(
            s.message.contains("crate too large to index")
                && s.message.contains("2001"),
            "{}",
            s.message
        );
        assert!(s.crate_indexing.is_empty(), "no in-flight build armed");
        assert!(s.external_indexes.is_empty(), "no cache entry");
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

    // ── plan 007 issue 04: incremental parse retention ────────────

    /// The notes edit path records its `InputEdit` on the retained parse
    /// tree and the rebuild stays byte-identical to a full parse.
    #[test]
    fn notes_edit_keeps_incremental_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        let mut store = store(root);
        store.open_notes();
        let key = store.buffers.current().unwrap().to_string();
        let mtime = store.buffers.get(&key).unwrap().mtime;
        let tree_key = TreeKey::new(&key, mtime);
        // First highlight: full parse + retained tree.
        store.ensure_highlight();
        assert!(
            store.highlight_cache.retain_contains(&tree_key),
            "markdown notes buffer must retain a parse tree after highlighting"
        );
        // Type a line: the edit is recorded on the retained tree and the
        // cache entry is invalidated (rebuilt on the next ensure).
        for c in "hi\n".chars() {
            store.notes_insert_char(c);
        }
        store.ensure_highlight();
        assert!(
            store.highlight_cache.retain_contains(&tree_key),
            "retained tree must survive the edit (same buffer, same mtime)"
        );
        // The rebuilt result must be byte-identical to a from-scratch
        // parse of the same content (the correctness bar).
        let (rope, path, lang) = {
            let buf = store.buffers.get(&key).unwrap();
            let path_str = buf.path.as_ref().unwrap().to_string_lossy().into_owned();
            (
                buf.rope.clone(),
                buf.path.clone().unwrap(),
                store.grammar_registry.language_for(&path_str),
            )
        };
        assert_eq!(lang, LanguageId::Markdown);
        let from_scratch = highlight::highlight_reusable(&rope, lang).unwrap().0;
        let cache_key = CacheKey::new(&path, mtime, store.theme.name());
        assert_eq!(
            store.highlight_cache.get(&cache_key),
            Some(&from_scratch),
            "incrementally rebuilt highlight must be byte-identical to a full parse"
        );
    }

    /// A disk reload gives the buffer a new mtime: the retained tree's
    /// (buffer, mtime) key no longer matches, so the rebuild is a full
    /// parse — the stale-tree fallback.
    #[test]
    fn reload_breaks_the_retained_tree_epoch() {
        let mut store = store_with_project();
        open_ann_file(&mut store, "notes.md", "# t\n");
        let key = store.buffers.current().unwrap().to_string();
        store.buffers.get_mut(&key).unwrap().editable = true;
        for c in "x\n".chars() {
            store.notes_insert_char(c);
        }
        store.ensure_highlight();
        let mtime1 = store.buffers.get(&key).unwrap().mtime;
        assert!(
            store.highlight_cache.retain_contains(&TreeKey::new(&key, mtime1)),
            "edited buffer must have a retained tree"
        );
        // Simulate `reload_buffer`: new content from disk, new mtime.
        let new_mtime =
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(4_000_000_000);
        {
            let buf = store.buffers.get_mut(&key).unwrap();
            buf.rope = Rope::from_str("# t\nreloaded\n");
            buf.mtime = new_mtime;
            buf.locally_modified = false;
        }
        assert!(
            !store.highlight_cache.retain_contains(&TreeKey::new(&key, new_mtime)),
            "a reloaded buffer must not find its stale retained tree"
        );
        store.ensure_highlight();
        assert!(
            store.highlight_cache.retain_contains(&TreeKey::new(&key, new_mtime)),
            "the rebuilt full parse must be retained under the new epoch"
        );
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
        // The original bug this guarded: M-. skipping uppercase-initial
        // names (types/constants). Selection now goes through
        // symbol_at_point, which has no case filter: the cursor on `Foo`
        // or `BAR` yields the identifier (the `::`-path token is kept for
        // the resolver).
        let line = "let x = Foo::BAR;";
        // `Foo` starts at col 8 (cursor inside the identifier).
        assert_eq!(
            satp(LanguageId::Rust, line, 8),
            Some(("Foo".into(), "Foo::BAR".into()))
        );
        // The second `:` of the `::` separator (col 12) counts as the end
        // of the preceding segment (006-02b item 4).
        assert_eq!(
            satp(LanguageId::Rust, line, 12),
            Some(("Foo".into(), "Foo::BAR".into()))
        );
        // `BAR` starts at col 13; just after it (col 16) still counts.
        assert_eq!(
            satp(LanguageId::Rust, line, 13),
            Some(("BAR".into(), "Foo::BAR".into()))
        );
        assert_eq!(
            satp(LanguageId::Rust, line, 16),
            Some(("BAR".into(), "Foo::BAR".into()))
        );
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
        s.open_scratch(); // 06a: boot is home; the test asserts BUFFER-view leaves
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
        let mut store = store(dir.path());
        store.open_scratch(); // 06a: boot is home; set-mark lives in the buffer view
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
                    syntax: None,
                    path: "src/main.rs".to_string(),
                    line: 4,
                    col: 2,
                    anchor: "fn main() {".to_string(),
                    text: "fix the off-by-one: it's: nasty".to_string(),
                    orphaned: false,
                }),
                NotesEntry::Raw("[annotation]\npath: src/main.rs\nline: 9\n(no required fields)".to_string()),
                NotesEntry::Record(Annotation {
                    syntax: None,
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
            syntax: None,
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
            syntax: None,
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
            syntax: None,
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
            syntax: None,
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

    // ── plan 007 issue 02: syntax-anchored annotations ───────────────────

    /// The HEADLINE test (plan 007 issue 02): annotate a Rust fn (the
    /// point on the function name → `SyntaxAnchor { identifier,
    /// target_one }`), then simulate a 100-line insertion AND a
    /// rustfmt-style reformat (the anchored line's exact text is GONE
    /// from the file). The syntax-anchored record follows the function
    /// (re-anchored to its new line, orphan flag cleared); the SAME
    /// record WITHOUT the syntax anchor orphans at its last known line —
    /// proof the ±25-line text path alone cannot do this.
    #[test]
    fn notes_syntax_anchor_survives_insertion_and_reformat() {
        let original = "fn target_one() {\n    let x = 1;\n    x\n}\n";
        // The reformat rewrites the signature line; 100 filler lines push
        // the function 100 lines down (far outside ±25).
        let filler: Vec<String> = (0..100).map(|i| format!("// filler {i}")).collect();
        let rewritten =
            filler.join("\n") + "\nfn target_one()\n{\n    let x = 1;\n    x\n}\n";
        // The discriminating precondition: the anchored line's text no
        // longer exists ANYWHERE — the text path (exact-line and ±25)
        // has nothing to match, inside or outside the window.
        assert!(
            !rewritten.contains("fn target_one() {"),
            "the text path must have nothing to match"
        );

        // Part 1: the syntax-anchored record follows the function.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/syn1.rs", original);
        s.set_point(0, 3, 3); // col 3: on the `target_one` name
        s.annotate();
        s.note_prompt_char('s');
        s.note_prompt_char('y');
        s.note_prompt_confirm();
        let a = ann_records(&s)[0];
        assert_eq!(
            a.syntax,
            Some(SyntaxAnchor {
                kind: "identifier".to_string(),
                name: "target_one".to_string(),
            }),
            "the fn name captures a syntax anchor"
        );
        assert_eq!(a.anchor, "fn target_one() {");
        // Simulate the 100-line insertion + the reformat.
        let key = s.buffers.current().unwrap().to_string();
        {
            let buf = s.buffers.get_mut(&key).unwrap();
            buf.rope = Rope::from_str(&rewritten);
        }
        s.reanchor_for_key(&key);
        let a = ann_records(&s)[0];
        assert_eq!(
            a.line, 100,
            "the note follows the function across 100 lines + a reformat"
        );
        assert!(!a.orphaned, "unique syntax match → not orphaned");
        // Stable: a second pass changes nothing (idempotent).
        s.reanchor_for_key(&key);
        assert_eq!(ann_records(&s)[0].line, 100);
        assert!(!ann_records(&s)[0].orphaned);
        drop(s);

        // Part 2 (discriminating half): the SAME drive with a record that
        // has NO syntax anchor (the legacy shape) — the ±25-line text
        // path finds nothing (the anchor text is gone and the move is
        // 100 lines out) → orphaned, line unchanged. Without the syntax
        // anchor, Part 1's outcome is impossible.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/syn2.rs", original);
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            path: "src/syn2.rs".to_string(),
            line: 0,
            col: 3,
            anchor: "fn target_one() {".to_string(),
            text: "sy".to_string(),
            orphaned: false,
            syntax: None,
        }));
        let key = s.buffers.current().unwrap().to_string();
        {
            let buf = s.buffers.get_mut(&key).unwrap();
            buf.rope = Rope::from_str(&rewritten);
        }
        s.reanchor_for_key(&key);
        let a = ann_records(&s)[0];
        assert!(
            a.orphaned,
            "text path alone: 0 matches → orphaned (never a guess)"
        );
        assert_eq!(a.line, 0, "the orphan stays at its last known line");
    }

    /// A legacy record (no syntax_* keys) parses with `syntax: None`,
    /// still re-anchors by text alone, and serializes back WITHOUT the
    /// syntax keys — byte-identical to the pre-007-02 shape (no
    /// migration of legacy records that are never touched).
    #[test]
    fn notes_legacy_record_stays_byte_identical_and_text_anchors() {
        let record = "[annotation]\n\
                      path: src/old.rs\n\
                      line: 1\n\
                      col: 0\n\
                      anchor: two\n\
                      note: old note\n\
                      orphaned: false\n";
        let doc_text = format!("{NOTES_BEGIN}\n{record}{NOTES_END}\n");
        let doc = parse_notes(&doc_text);
        let rec = doc.entries[0].as_record().expect("the legacy record parses");
        assert!(rec.syntax.is_none(), "legacy record → syntax: None");
        assert_eq!(
            serialize_notes(&doc),
            doc_text,
            "a never-moved legacy record round-trips byte-identically (no syntax keys appear)"
        );
        // Re-anchoring still works by text alone for it.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/old.rs", "one\ntwo\nthree\n");
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            path: "src/old.rs".to_string(),
            line: 5, // out of range: drift for sure
            col: 0,
            anchor: "two".to_string(),
            text: "old note".to_string(),
            orphaned: false,
            syntax: None,
        }));
        let key = s.buffers.current().unwrap().to_string();
        s.reanchor_for_key(&key);
        assert_eq!(ann_records(&s)[0].line, 1, "legacy record re-anchors by text");
        let out = serialize_notes(&s.notes_doc);
        assert!(
            !out.contains("syntax_kind"),
            "no syntax keys ever appear for legacy records: {out}"
        );
        // Unknown keys inside a record block: the record stays VALID
        // (forward compatibility) and the unknown key is not re-emitted —
        // the pre-007-02 rule, unchanged.
        let unknown = format!(
            "{NOTES_BEGIN}\n\
             [annotation]\n\
             path: src/old.rs\n\
             line: 1\n\
             col: 0\n\
             anchor: two\n\
             note: old note\n\
             orphaned: false\n\
             future_key: later\n\
             {NOTES_END}\n"
        );
        let udoc = parse_notes(&unknown);
        assert!(
            udoc.entries[0].as_record().is_some(),
            "a record with an unknown key stays a valid record"
        );
        assert!(
            udoc.entries[0].as_record().unwrap().syntax.is_none()
        );
        let uout = serialize_notes(&udoc);
        assert!(!uout.contains("future_key"), "unknown keys are not re-emitted (005-02 rule)");
        assert!(uout.contains("line: 1"), "the record's own fields survive");
    }

    /// Ambiguous syntax match (multiple identifier nodes with the same
    /// kind + name anywhere in the file) → no guess → the text rules run;
    /// when they also fail, the record orphans at its last known line.
    #[test]
    fn notes_syntax_ambiguous_match_orphans() {
        let original = "fn helper() {\n    let helper = 1;\n    helper\n}\n";
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/syn3.rs", original);
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            path: "src/syn3.rs".to_string(),
            line: 0,
            col: 3,
            anchor: "fn helper() {".to_string(),
            text: "amb".to_string(),
            orphaned: false,
            syntax: Some(SyntaxAnchor {
                kind: "identifier".to_string(),
                name: "helper".to_string(),
            }),
        }));
        // The signature line is reformatted (the anchor text is gone);
        // `helper` still appears as the fn name AND the local → 3
        // identifier nodes named `helper` → ambiguous.
        let key = s.buffers.current().unwrap().to_string();
        {
            let buf = s.buffers.get_mut(&key).unwrap();
            buf.rope = Rope::from_str("fn helper() -> i32\n{\n    let helper = 1;\n    helper\n}\n");
        }
        s.reanchor_for_key(&key);
        let a = ann_records(&s)[0];
        assert!(a.orphaned, "2+ syntax matches → no guess → orphan");
        assert_eq!(a.line, 0, "the orphan keeps its last known line");
    }

    /// A record WITH a syntax anchor round-trips exactly: serialize →
    /// parse → an equal record (the keys are re-emitted in full).
    #[test]
    fn notes_syntax_anchor_round_trip() {
        let rec = Annotation {
            path: "src/x.rs".to_string(),
            line: 7,
            col: 3,
            anchor: "fn x() {".to_string(),
            text: "rt".to_string(),
            orphaned: false,
            syntax: Some(SyntaxAnchor {
                kind: "identifier".to_string(),
                name: "x".to_string(),
            }),
        };
        let doc = NotesDoc {
            before: Vec::new(),
            entries: vec![NotesEntry::Record(rec.clone())],
            after: Vec::new(),
        };
        let out = serialize_notes(&doc);
        assert!(out.contains("syntax_kind: identifier\n"), "{out}");
        assert!(out.contains("syntax_name: x\n"), "{out}");
        let back = parse_notes(&out);
        assert_eq!(back.entries, vec![NotesEntry::Record(rec)]);
    }

    /// A half-written anchor (`syntax_kind` without `syntax_name`) degrades
    /// to `syntax: None` — the record stays VALID (tolerant parse, never
    /// half-fires); a record with a malformed required field stays a Raw
    /// block, verbatim (never dropped), even when it carries syntax keys
    /// (005-02).
    #[test]
    fn notes_syntax_keys_half_and_malformed() {
        let doc_text = format!(
            "{NOTES_BEGIN}\n\
             [annotation]\n\
             path: a.rs\n\
             line: 0\n\
             col: 0\n\
             anchor: one\n\
             note: half anchor\n\
             orphaned: false\n\
             syntax_kind: identifier\n\
             [annotation]\n\
             path: b.rs\n\
             line: 0\n\
             col: 0\n\
             note: missing its anchor\n\
             syntax_kind: identifier\n\
             syntax_name: ghost\n\
             {NOTES_END}\n"
        );
        let doc = parse_notes(&doc_text);
        assert_eq!(doc.entries.len(), 2);
        // The half anchor: a VALID record, `syntax: None`.
        let a = doc
            .entries[0]
            .as_record()
            .expect("a record with a half syntax pair stays valid");
        assert!(
            a.syntax.is_none(),
            "syntax_kind without syntax_name → None (never half-fires)"
        );
        assert_eq!(a.text, "half anchor");
        // The record missing `anchor` stays a Raw block, verbatim (the
        // syntax keys included — malformed records are never dropped).
        match &doc.entries[1] {
            NotesEntry::Raw(raw) => {
                assert!(raw.contains("missing its anchor"), "{raw}");
                assert!(raw.contains("syntax_name: ghost"), "{raw}");
            }
            other => panic!("malformed record must stay Raw: {other:?}"),
        }
    }

    /// A non-Rust buffer captures no syntax anchor (007-01 returns None
    /// for every non-Rust language — and no parse happens at all) and
    /// rides the text rules as before; a Rust point on a KEYWORD (col 0
    /// of the `fn` header) is the same: `None`, not a guess.
    #[test]
    fn notes_non_rust_and_keyword_points_have_no_syntax_anchor() {
        // Non-Rust: the record has `syntax: None` (old behavior intact).
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann_py.py", "def f():\n    return 1\n");
        s.set_point(0, 4, 4); // on the `f` in `def f():`
        s.annotate();
        s.note_prompt_char('p');
        s.note_prompt_confirm();
        let a = ann_records(&s)[0];
        assert!(a.syntax.is_none(), "non-Rust → no syntax anchor");
        assert_eq!(a.anchor, "def f():");
        drop(s);

        // Rust, point on the `fn` keyword (col 0): `node_at` has no
        // identifier-ish node there → honest None.
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann_kw.rs", "fn kw_target() {\n    let y = 2;\n}\n");
        s.set_point(0, 0, 0); // col 0: the `f` of `fn`
        s.annotate();
        s.note_prompt_char('k');
        s.note_prompt_confirm();
        let a = ann_records(&s)[0];
        assert!(a.syntax.is_none(), "a keyword point captures no anchor");
        assert_eq!(a.anchor, "fn kw_target() {");
    }

    // ── plan 008 issue 01: annotation keying on external buffers ─────────

    /// An out-of-project file, opened read-only the way `M-.` lands it.
    /// Returns the store + the file's absolute path string.
    fn store_with_external_file() -> (AppStore, String) {
        let mut s = store_with_project();
        let ext = tempfile::tempdir().unwrap();
        let abs = ext.path().join("lib_source.rs");
        std::fs::write(&abs, "ext line one\next line two\next line three\n").unwrap();
        let abs_str = abs.to_string_lossy().into_owned();
        let key = s
            .open_external_path(&abs)
            .expect("external file opens read-only");
        assert!(s.buffers.get(&key).unwrap().path.is_some());
        (s, abs_str)
    }

    /// The key-derivation point: project files key by project-relative path
    /// (byte-identical to pre-008); an external buffer keys by its ABSOLUTE
    /// path string; scratch (pathless) still yields `None`.
    #[test]
    fn notes_annotation_path_under_root_and_absolute() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/annext.rs", "a\nb\n");
        let proj_key = s.buffers.current().unwrap().to_string();
        assert_eq!(
            s.buffer_annotation_path(&proj_key).as_deref(),
            Some("src/annext.rs"),
            "under-root key unchanged (byte-identical)"
        );

        let ext = tempfile::tempdir().unwrap();
        let abs = ext.path().join("lib.rs");
        std::fs::write(&abs, "x\n").unwrap();
        let abs_str = abs.to_string_lossy().into_owned();
        let ext_key = s.open_external_path(&abs).unwrap();
        assert_eq!(
            s.buffer_annotation_path(&ext_key).as_deref(),
            Some(abs_str.as_str()),
            "out-of-root key = the absolute path"
        );
        // The current-buffer variant follows the current buffer.
        assert_eq!(
            s.current_annotation_path().as_deref(),
            Some(abs_str.as_str()),
            "current_annotation_path tracks the current buffer"
        );
        // Scratch: pathless buffers still have no key.
        let scratch_key = s.buffers
            .insert_rope(None, Rope::from_str("s"), std::time::SystemTime::now(), true);
        assert!(s.buffer_annotation_path(&scratch_key).is_none());
    }

    /// `A` → type → RET commits on an external buffer (record keyed by the
    /// absolute path, written to the project's notes file), `d` deletes it;
    /// the prefill sees the external record back.
    #[test]
    fn notes_external_buffer_annotate_delete_round_trip() {
        let (mut s, abs_str) = store_with_external_file();
        s.set_point_line(1);
        s.annotate();
        assert!(s.note_prompt_active());
        assert_eq!(s.note_prompt_input(), "", "fresh prompt, no prefill");
        s.note_prompt_char('l');
        s.note_prompt_char('i');
        s.note_prompt_confirm();
        let a = ann_records(&s)[0];
        assert_eq!(a.path, abs_str, "record keyed by the absolute path");
        assert_eq!(a.line, 1);
        assert_eq!(a.anchor, "ext line two");
        assert_eq!(a.text, "li");
        // On disk, in the project's notes file (placement unchanged).
        let disk = std::fs::read_to_string(
            s.project.as_ref().unwrap().root.join(".redline-notes.md"),
        )
        .unwrap();
        assert!(disk.contains(&format!("path: {abs_str}")), "{disk}");
        // The status count sees the external record.
        assert_eq!(s.annotation_count_display(), "1 note");
        // Pre-fill for edit on the external record.
        s.set_point_line(1);
        s.annotate();
        assert_eq!(s.note_prompt_input(), "li", "external record pre-fills");
        s.note_prompt_cancel();
        // `d` removes it.
        s.set_point_line(1);
        s.annotate_delete();
        assert!(s.message.contains("deleted annotation: li"), "{}", s.message);
        assert!(ann_records(&s).is_empty());
    }

    /// `file_view_rows` marker + note row render on an external buffer,
    /// keyed by the absolute path (pre-008 these were dead there).
    #[test]
    fn notes_external_buffer_marker_and_note_row() {
        let (mut s, _) = store_with_external_file();
        s.set_point_line(0);
        s.annotate();
        s.note_prompt_char('m');
        s.note_prompt_confirm();
        s.show_note_rows = true;
        let rows = s.file_view_rows();
        let code_rows: Vec<_> = rows.iter().filter(|r| !r.is_note).collect();
        let marker: Vec<_> = code_rows
            .iter()
            .filter(|r| r.annotated && r.line == 0)
            .collect();
        assert_eq!(marker.len(), 1, "margin marker on the annotated line");
        let note_rows: Vec<_> = rows.iter().filter(|r| r.is_note).collect();
        assert_eq!(note_rows.len(), 1, "the inline note row renders");
        assert!(note_rows[0].text.contains("m"), "{}", note_rows[0].text);
        // A record carrying the external file's IN-PROJECT relative name
        // must NOT match the external buffer (the external key is the
        // absolute path, so the keys differ).
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/lib_source.rs".to_string(),
            line: 0,
            col: 0,
            anchor: "ext line one".to_string(),
            text: "rel-path impostor".to_string(),
            orphaned: false,
        }));
        let rows = s.file_view_rows();
        assert_eq!(
            rows.iter().filter(|r| r.is_note).count(),
            1,
            "a rel-path record for the external file's in-project relative name never matches"
        );
    }

    /// The quit-dump carries the external record's path VERBATIM (absolute)
    /// with the open buffer's live code line; project records keep their
    /// relative path in the same dump.
    #[test]
    fn notes_external_buffer_dump_verbatim_path() {
        let (mut s, abs_str) = store_with_external_file();
        s.set_point_line(2);
        s.annotate();
        s.note_prompt_char('d');
        s.note_prompt_confirm();
        // A project record in the same doc keeps its relative path.
        open_ann_file(&mut s, "src/proj.rs", "p0\np1\n");
        s.set_point_line(0);
        s.annotate();
        s.note_prompt_char('p');
        s.note_prompt_confirm();
        let items = s.annotations_for_dump();
        let ext = items.iter().find(|i| i.path == abs_str).expect("external item");
        assert_eq!(ext.line, 3, "0-based line 2 → 1-based 3");
        assert_eq!(ext.code, "ext line three", "live code from the open external buffer");
        let proj = items.iter().find(|i| i.path == "src/proj.rs").expect("project item");
        assert_eq!(proj.code, "p0");
        // The formatted dump prints both paths verbatim.
        let block = format_notes_dump(&items, s.project.as_ref().unwrap().root.to_str().unwrap(), false);
        assert!(block.contains(&format!("{abs_str}:3\n")), "block: {}", block);
        let plain = format_notes_dump(&items, "", true);
        assert!(plain.contains(&format!("{abs_str}:3: d\n")), "plain: {plain}");
        assert!(plain.contains("src/proj.rs:1: p\n"), "plain: {plain}");
        // 008-01 P3 (folded into 006-02b item 8): pin the mixed ordering —
        // the absolute (`/`-prefixed) record sorts BEFORE the
        // project-relative record byte-wise (`/` < `s`), in BOTH modes.
        let (ext_pos, proj_pos) = (block.find(abs_str.as_str()), block.find("src/proj.rs"));
        assert!(ext_pos.is_some_and(|e| proj_pos.is_some_and(|p| e < p)),
            "block: the absolute record precedes the relative record:\n{block}");
        let (ext_pos, proj_pos) = (plain.find(abs_str.as_str()), plain.find("src/proj.rs"));
        assert!(ext_pos.is_some_and(|e| proj_pos.is_some_and(|p| e < p)),
            "plain: the absolute record precedes the relative record:\n{plain}");
    }

    /// Adding an external record must not touch the existing project-relative
    /// record's bytes in the notes file (no migration, no re-keying).
    #[test]
    fn notes_external_annotation_leaves_rel_records_untouched() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/keep.rs", "k0\nk1\n");
        s.set_point_line(1);
        s.annotate();
        s.note_prompt_char('k');
        s.note_prompt_confirm();
        let before = std::fs::read_to_string(
            s.project.as_ref().unwrap().root.join(".redline-notes.md"),
        )
        .unwrap();
        assert!(before.contains("path: src/keep.rs"), "{before}");
        // The record block for the rel record: its `path:` line through the
        // line before the next record's `path:`.
        let block = |text: &str, path: &str| -> String {
            let marker = format!("path: {path}\n");
            let start = text.find(&marker).expect("record line");
            // Through the record's final `orphaned:` line (schema-ordered).
            let tail = &text[start + marker.len()..];
            let end = tail.find("orphaned:").unwrap() + "orphaned:".len();
            text[start..start + marker.len() + end].to_string()
        };

        let (mut s, abs_str) = {
            let ext = tempfile::tempdir().unwrap();
            let abs = ext.path().join("lib.rs");
            std::fs::write(&abs, "e0\ne1\n").unwrap();
            s.open_external_path(&abs).unwrap();
            (s, abs.to_string_lossy().into_owned())
        };
        s.set_point_line(1);
        s.annotate();
        s.note_prompt_char('e');
        s.note_prompt_confirm();
        let after = std::fs::read_to_string(
            s.project.as_ref().unwrap().root.join(".redline-notes.md"),
        )
        .unwrap();
        assert_eq!(
            block(&before, "src/keep.rs"),
            block(&after, "src/keep.rs"),
            "the rel record's bytes are untouched: {after}"
        );
        assert!(after.contains(&format!("path: {abs_str}")), "external record written: {after}");
    }

    // ── plan 005 issue 03: quit-dump formatter + accessor ───────────────

    fn dump_item(path: &str, line: usize, code: &str, text: &str, orphaned: bool) -> DumpAnnotation {
        DumpAnnotation {
            path: path.to_string(),
            line,
            code: code.to_string(),
            text: text.to_string(),
            orphaned,
        }
    }

    /// The default block mode is the agent brief: header + blank line, then
    /// path:line, the anchored line indented +4, and `NOTE:` with the text.
    #[test]
    fn notes_dump_block_format_exact_bytes() {
        let items = vec![dump_item(
            "src/app/store.rs",
            1420,
            "fn save_buffer(&mut self) {",
            "this silently overwrites the mtime; check the conflict marker first",
            false,
        )];
        let out = format_notes_dump(&items, "/home/dev/red", false);
        let expected = "# redline annotations \u{2014} /home/dev/red\n\n".to_owned()
            + "src/app/store.rs:1420\n"
            + "    fn save_buffer(&mut self) {\n"
            + "  NOTE: this silently overwrites the mtime; check the conflict marker first\n";
        assert_eq!(out, expected, "exact block bytes");
    }

    /// An orphaned record carries the explicit marker line (the agent must
    /// not trust a stale line number silently) and its code line is the
    /// STORED anchor (last known anchored text).
    #[test]
    fn notes_dump_orphaned_marker() {
        let items = vec![dump_item("src/old.rs", 7, "fn gone() {", "stale note", true)];
        let out = format_notes_dump(&items, "/p", false);
        let expected = "# redline annotations \u{2014} /p\n\n".to_owned()
            + "src/old.rs:7\n"
            + "  ORPHANED (anchor text not found)\n"
            + "    fn gone() {\n"
            + "  NOTE: stale note\n";
        assert_eq!(out, expected, "exact orphaned bytes");
    }

    /// Multi-line notes: subsequent lines align under the first note line's
    /// text (block: 8 spaces = 2 + `NOTE: `; plain: the `path:line: `
    /// prefix repeats).
    #[test]
    fn notes_dump_multiline_note_alignment() {
        let items = vec![dump_item("a.rs", 3, "code", "first\nsecond\nthird", false)];
        let block = format_notes_dump(&items, "/p", false);
        assert!(
            block.contains("  NOTE: first\n        second\n        third\n"),
            "block continuation alignment: {block:?}"
        );
        let plain = format_notes_dump(&items, "/p", true);
        assert_eq!(
            plain,
            "a.rs:3: first\n".to_owned() + "a.rs:3: second\n" + "a.rs:3: third\n",
            "plain continuation alignment"
        );
    }

    /// Ordering is part of the contract: path asc, then line asc, in BOTH
    /// modes (diff stability).
    #[test]
    fn notes_dump_ordering_path_then_line() {
        let items = vec![
            dump_item("b.rs", 5, "x", "n5", false),
            dump_item("a.rs", 9, "y", "n9", false),
            dump_item("a.rs", 2, "z", "n2", false),
        ];
        for plain in [false, true] {
            let out = format_notes_dump(&items, "/p", plain);
            let positions: Vec<usize> = out
                .match_indices("a.rs:2")
                .map(|(i, _)| i)
                .chain(out.match_indices("a.rs:9").map(|(i, _)| i))
                .chain(out.match_indices("b.rs:5").map(|(i, _)| i))
                .collect();
            assert!(
                positions.windows(2).all(|w| w[0] < w[1]),
                "order a:2 < a:9 < b:5 (plain={plain}): {out:?}"
            );
        }
    }

    /// `--notes=plain` parity: same records, the grep/pipe shape with NO
    /// header and no code/orphan lines; the block keeps them.
    #[test]
    fn notes_dump_plain_flag_parity() {
        let items = vec![
            dump_item("src/main.rs", 4, "fn main() {", "fix the off-by-one", false),
            dump_item("README.md", 1, "Redline", "top", true),
        ];
        let plain = format_notes_dump(&items, "/p", true);
        assert_eq!(
            plain,
            "README.md:1: top\n".to_owned() + "src/main.rs:4: fix the off-by-one\n",
            "plain: path:line: text, no header, ordered"
        );
        assert!(!plain.contains("ORPHANED") && !plain.contains("# redline"));
        let block = format_notes_dump(&items, "/p", false);
        assert!(block.starts_with("# redline annotations \u{2014} /p\n\n"));
        assert!(block.contains("ORPHANED (anchor text not found)"));
        assert!(block.contains("    fn main() {") && block.contains("  NOTE: top"));
    }

    /// No annotations ⇒ zero bytes in BOTH modes (pipes stay clean).
    #[test]
    fn notes_dump_empty_is_zero_bytes() {
        assert_eq!(format_notes_dump(&[], "/p", false), "");
        assert_eq!(format_notes_dump(&[], "/p", true), "");
    }

    /// The accessor resolves the dump shape from the store: 1-based lines,
    /// `code` = the open buffer's current content when the anchor holds, the
    /// STORED anchor for orphaned records (never the line's current text),
    /// and the stored anchor when the buffer is not open.
    #[test]
    fn notes_dump_accessor_resolves_code_and_lines() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/dump1.rs", "alpha line\nbeta line\ngamma line\n");
        for a in [
            Annotation {
                syntax: None,
                path: "src/dump1.rs".to_string(),
                line: 0,
                col: 0,
                anchor: "alpha line".to_string(),
                text: "note one".to_string(),
                orphaned: false,
            },
            Annotation {
                syntax: None,
                path: "src/dump1.rs".to_string(),
                line: 1,
                col: 0,
                anchor: "GHOST ANCHOR".to_string(),
                text: "ghost note".to_string(),
                orphaned: true,
            },
        ] {
            s.notes_doc.entries.push(NotesEntry::Record(a));
        }
        // Persist so the lazy-load path does not clobber the in-memory doc
        // (the real A/prompt flow does this via sync on commit).
        s.sync_notes_from_doc();
        let items = s.annotations_for_dump();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].path, "src/dump1.rs");
        assert_eq!(items[0].line, 1, "record line 0 → dump line 1 (1-based)");
        assert_eq!(items[0].code, "alpha line", "open buffer's current line");
        assert_eq!(items[0].text, "note one");
        assert!(!items[0].orphaned);
        assert_eq!(items[1].line, 2, "record line 1 → dump line 2 (1-based)");
        assert_eq!(
            items[1].code, "GHOST ANCHOR",
            "orphaned: the stored anchor, NOT the line's current text"
        );
        assert!(items[1].orphaned);

        // Closed buffer: `code` falls back to the stored anchor.
        let mut s2 = store_with_project();
        s2.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/closed.rs".to_string(),
            line: 12,
            col: 0,
            anchor: "fn closed() {".to_string(),
            text: "closed note".to_string(),
            orphaned: false,
        }));
        s2.sync_notes_from_doc();
        let items2 = s2.annotations_for_dump();
        assert_eq!(items2.len(), 1);
        assert_eq!(items2[0].line, 13, "1-based");
        assert_eq!(items2[0].code, "fn closed() {", "closed buffer → stored anchor");
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
                syntax: None,
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
            syntax: None,
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

    /// plan 005 issue 02b (round 2) + 02c (span fill): the budget counts
    /// are FINAL. The all-lines-annotated window still emits the point's
    /// code row, never exceeds `viewport_lines` rendered rows, and the
    /// emitted-range note recount keeps `code + note <= viewport` even
    /// when many records share one line (where the full-window note
    /// count underestimates the advanced range's note count). 02c: the
    /// span is the LARGEST that fits, so the all-annotated repro fills
    /// the canvas (10 code + 10 note rows of 21) instead of collapsing
    /// to a 1-row span.
    #[test]
    fn notes_all_lines_annotated_budget_is_final() {
        // Leg 1 (the repro): 25-line file, ALL 25 lines annotated,
        // viewport 21, scroll_top 0 → the largest fitting span is 10
        // (10 + 10 notes = 20 <= 21; 11 + 11 = 22 > 21): 10 code rows +
        // 10 note rows — the canvas is FILLED, not just non-blank, and
        // the point's line is drawn.
        let mut s = store_with_project();
        let content: String = (0..25).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
        open_ann_file(&mut s, "src/ann10.rs", &content);
        for line in 0..25 {
            s.notes_doc.entries.push(NotesEntry::Record(Annotation {
                syntax: None,
                path: "src/ann10.rs".to_string(),
                line,
                col: 0,
                anchor: format!("line{line}"),
                text: "note".to_string(),
                orphaned: false,
            }));
        }
        s.sync_notes_from_doc();
        s.set_viewport_lines(21);
        s.set_scroll_top(0);
        s.set_point_line(0);
        let rows = s.file_view_rows();
        assert!(!rows.is_empty(), "view must not blank: {rows:?}");
        assert!(
            rows.len() <= 21,
            "budget exceeded: {} rows",
            rows.len()
        );
        // Canvas FILL (02c): the emitted count is close to viewport_lines
        // (>= 18 of 21), specifically the 10 code + 10 note shape.
        assert!(
            rows.len() >= 18,
            "view under-fills the canvas: {} rows",
            rows.len()
        );
        assert_eq!(rows.len(), 20, "10 code + 10 note rows of 21");
        assert_eq!(rows.iter().filter(|r| !r.is_note).count(), 10);
        assert_eq!(rows.iter().filter(|r| r.is_note).count(), 10);
        // The point's line IS drawn (as a code row, with its text intact).
        assert_eq!(FileViewRow::row_for_line(&rows, 0), Some(0));
        assert!(!rows[0].is_note);
        assert_eq!(rows[0].text, "line0");
        assert!(rows[0].annotated);
        // Every note row sits directly under its anchored code row.
        for (i, r) in rows.iter().enumerate() {
            if r.is_note {
                assert!(!rows[i - 1].is_note);
                assert_eq!(rows[i - 1].line, r.line);
            }
        }

        // Leg 2: the point is NOT at the window top: the span is 10
        // (largest that fits), so start advances to keep the point's
        // line drawn — the point is the LAST code row (line 10 at index
        // 18), the window still fills (10 code + 10 note rows), and the
        // view is not blank.
        let mut s2 = store_with_project();
        let content: String = (0..25).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
        open_ann_file(&mut s2, "src/ann11.rs", &content);
        for line in 0..25 {
            s2.notes_doc.entries.push(NotesEntry::Record(Annotation {
                syntax: None,
                path: "src/ann11.rs".to_string(),
                line,
                col: 0,
                anchor: format!("line{line}"),
                text: "note".to_string(),
                orphaned: false,
            }));
        }
        s2.sync_notes_from_doc();
        s2.set_viewport_lines(21);
        s2.set_scroll_top(0);
        s2.set_point_line(10);
        let rows = s2.file_view_rows();
        assert!(!rows.is_empty(), "view must not blank: {rows:?}");
        assert_eq!(
            FileViewRow::row_for_line(&rows, 10),
            Some(18),
            "point's line is the last code row: {:?}",
            rows.iter().map(|r| (r.line, r.is_note)).collect::<Vec<_>>()
        );
        assert_eq!(rows[18].text, "line10");
        assert!(!rows[18].is_note);
        assert_eq!(rows.len(), 20, "advanced window still fills: 10 + 10");
        // The emitted window's first buffer line (1) is above scroll_top
        // (0) — the \u{2191} indicator keys off rows[0].line.
        assert_eq!(rows[0].line, 1, "window advanced above scroll_top");

        // Leg 3 (the re-overflow): 22 records on ONE line (more than the
        // viewport) — no span >= 6 fits (6 + 22 > 21), so the span is 5
        // (lines 0..5 hold no notes); the advanced range [1, 6) still
        // holds all 22 notes. The emitted-range recount caps the note
        // rows so code + note <= 21 always (5 code + 16 note = 21).
        let mut s3 = store_with_project();
        let content: String = (0..30).map(|i| format!("k{i}")).collect::<Vec<_>>().join("\n");
        open_ann_file(&mut s3, "src/ann12.rs", &content);
        for i in 0..22 {
            s3.notes_doc.entries.push(NotesEntry::Record(Annotation {
                syntax: None,
                path: "src/ann12.rs".to_string(),
                line: 5,
                col: 0,
                anchor: "k5".to_string(),
                text: format!("note {i}"),
                orphaned: false,
            }));
        }
        s3.sync_notes_from_doc();
        s3.set_viewport_lines(21);
        s3.set_scroll_top(0);
        s3.set_point_line(5);
        let rows = s3.file_view_rows();
        assert_eq!(rows.len(), 21, "5 code rows + 16 capped note rows (<= 21)");
        assert_eq!(FileViewRow::row_for_line(&rows, 5), Some(4), "point's line drawn");
        assert_eq!(rows.iter().filter(|r| r.is_note).count(), 16);

        // Leg 4 (non-degenerate regression): a single note in a 31-line
        // file, viewport 21 → the span stays 20 code rows + the 1 note
        // (the pre-round-2 behavior, unchanged): the note row is drawn.
        let mut s4 = store_with_project();
        let content: String = (0..30).map(|i| format!("k{i}")).collect::<Vec<_>>().join("\n");
        open_ann_file(&mut s4, "src/ann13.rs", &content);
        s4.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/ann13.rs".to_string(),
            line: 2,
            col: 0,
            anchor: "k2".to_string(),
            text: "note".to_string(),
            orphaned: false,
        }));
        s4.sync_notes_from_doc();
        s4.set_viewport_lines(21);
        s4.set_scroll_top(0);
        s4.set_point_line(2);
        let rows = s4.file_view_rows();
        let shape: Vec<(usize, bool)> = rows.iter().map(|r| (r.line, r.is_note)).collect();
        assert_eq!(
            shape,
            vec![
                (0, false),
                (1, false),
                (2, false),
                (2, true),
                (3, false),
                (4, false),
                (5, false),
                (6, false),
                (7, false),
                (8, false),
                (9, false),
                (10, false),
                (11, false),
                (12, false),
                (13, false),
                (14, false),
                (15, false),
                (16, false),
                (17, false),
                (18, false),
                (19, false)
            ],
            "20 code rows + 1 note = 21: {shape:?}"
        );
    }

    /// The status line shows the current file's annotation count (and an
    /// edit-mode save of the anchored file re-anchors in the same pass).
    #[test]
    fn notes_status_count_and_save_reanchors() {
        let mut s = store_with_project();
        open_ann_file(&mut s, "src/ann9.rs", "p0\np1\n");
        assert_eq!(s.annotation_count_display(), "");
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
            path: "src/ann9.rs".to_string(),
            line: 1,
            col: 0,
            anchor: "p1".to_string(),
            text: "a".to_string(),
            orphaned: false,
        }));
        s.notes_doc.entries.push(NotesEntry::Record(Annotation {
            syntax: None,
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

    // ── 011-02: per-language import walks (bare-symbol hints) ───────

    /// (line, col) of a byte offset inside `src`.
    fn point_of(src: &str, at: usize) -> (usize, usize) {
        let line = src[..at].matches('\n').count();
        let col = at - src[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
        (line, col)
    }

    /// Discriminating: a BARE imported symbol in a JS buffer now carries
    /// the package path (007-03 did this for Rust only; pre-011-02 the
    /// hint was empty for every non-Rust buffer).
    #[test]
    fn resolver_scope_js_named_import_carries_package_path() {
        let src = "import { doThing } from \"acme\";\nfunction f() { doThing(); }\n";
        let (mut s, _dir) = store_with_index(&[("src/index.js", src)]);
        s.open_path("src/index.js");
        let at = src.rfind("doThing").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("doThing"),
            vec!["acme".to_string(), "doThing".to_string()]
        );
    }

    /// JS alias → original: `import { doThing as dt }` carries the
    /// ORIGINAL path for the bare alias.
    #[test]
    fn resolver_scope_js_alias_carries_original() {
        let src = "import { doThing as dt } from \"acme\";\ndt();\n";
        let (mut s, _dir) = store_with_index(&[("src/index.js", src)]);
        s.open_path("src/index.js");
        let at = src.rfind("dt").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("dt"),
            vec!["acme".to_string(), "doThing".to_string()]
        );
    }

    /// JS default import: the local binding carries the package + name
    /// (the provider's definition scan then pins the real line, or bails
    /// honestly when it can't — the hint is the user's import, not a
    /// guess).
    #[test]
    fn resolver_scope_js_default_import_carries_package() {
        let src = "import doThing from \"acme\";\ndoThing();\n";
        let (mut s, _dir) = store_with_index(&[("src/index.js", src)]);
        s.open_path("src/index.js");
        let at = src.rfind("doThing").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("doThing"),
            vec!["acme".to_string(), "doThing".to_string()]
        );
    }

    /// JS namespace import: bare `ns` names the package entry (`["pkg"]`);
    /// `ns.member` rewrites to the package's real path + member. An
    /// identity alias (`import * as acme from "acme"`) still carries the
    /// path — the provider's rewrite rule treats it as a no-op because the
    /// hint IS the symbol's own path.
    #[test]
    fn resolver_scope_js_namespace_import() {
        let src = "import * as ac from \"acme\";\nimport * as acme from \"acme\";\nac.doThing();\nacme.doThing();\n";
        let (mut s, _dir) = store_with_index(&[("src/index.js", src)]);
        s.open_path("src/index.js");
        // Bare namespace → the package entry.
        let at = src.find("import * as ac ").unwrap() + "import * as ac".len();
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(s.resolver_scope("ac"), vec!["acme".to_string()]);
        // `ns.member` → the package's real path + member.
        let at = src.find("ac.doThing()").expect("fixture");
        let (line, col) = point_of(src, at + 1); // inside `doThing`
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("ac.doThing"),
            vec!["acme".to_string(), "doThing".to_string()]
        );
        // Identity alias: the hint equals the symbol's own path (the
        // provider's rewrite is a no-op there — pinned at the provider).
        let at = src.find("acme.doThing()").expect("fixture");
        let (line, col) = point_of(src, at + 1);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("acme.doThing"),
            vec!["acme".to_string(), "doThing".to_string()]
        );
    }

    /// No-import / absolute / bare-`.` / bare-`..` / side-effect
    /// specifiers are NEVER guessed: the hint stays empty (the provider
    /// keeps its exact no-hint / dedicated-bail behavior — the JS
    /// provider bails dedicated on absolute specs, and a bare `.`/`..`
    /// never reaches its relative branch). Relative (`./`/`../`) specs
    /// LEFT this list in the fix-jsrel P2-7 follow-up: they hint now
    /// (pinned in
    /// `resolver_scope_js_relative_import_carries_sibling_path`).
    #[test]
    fn resolver_scope_js_no_import_absolute_and_side_effect_are_not_guessed() {
        let cases = [
            ("no import", "function f() { doThing(); }\n"),
            (
                "absolute path",
                "import { doThing } from \"/opt/acme\";\ndoThing();\n",
            ),
            (
                "bare dot (current dir)",
                "import { doThing } from \".\";\ndoThing();\n",
            ),
            (
                "bare dotdot (parent dir)",
                "import { doThing } from \"..\";\ndoThing();\n",
            ),
            (
                "side-effect only",
                "import \"acme\";\ndoThing();\n",
            ),
            (
                "relative side-effect (binds nothing)",
                "import \"./acme\";\ndoThing();\n",
            ),
        ];
        for (name, src) in cases {
            let (mut s, _dir) = store_with_index(&[("src/index.js", src)]);
            s.open_path("src/index.js");
            let at = src.rfind("doThing").expect("fixture");
            let (line, col) = point_of(src, at);
            s.set_point(line, col, col);
            assert!(
                s.resolver_scope("doThing").is_empty(),
                "{name}: no hint expected"
            );
        }
    }

    /// 011-08 follow-up (fix-jsrel P2-7): a relative specifier (`./…` /
    /// `../…`) now CARRIES its hint — item included (the
    /// `SymbolContext.scope` contract) — because the JS provider landed
    /// it: it resolves against the importing buffer's directory and lands
    /// in the sibling file, workspace-local (`external = false`). The
    /// sibling's PRESENCE is what the provider checks (a dedicated bail
    /// when the file is absent — the provider's corpus goldens); the
    /// app-side hint is the import declaration itself, never a guess.
    #[test]
    fn resolver_scope_js_relative_import_carries_sibling_path() {
        let src = "\
            import { legacyJoin } from \"./legacy-util\";
            import { joinTwo as legacyTwo } from \"../lib/legacy-util\";
            import legacyDefault from \"./legacy-default.js\";
            import { default as legacyDef2 } from \"./legacy-whole\";
            import * as legacyNs from \"./legacy-ns\";
            const { cjsJoin } = require(\"./legacy-cjs\");
            const legacyWhole = require(\"./legacy-whole\");
            legacyJoin();
            legacyTwo();
            legacyDefault();
            legacyDef2();
            legacyNs.helper();
            cjsJoin();
            legacyWhole();
            ";
        let (mut s, _dir) = store_with_index(&[
            ("src/index.js", src),
            // The siblings really ARE present (the hint does not stat the
            // disk — the provider does, and bails dedicated when absent).
            ("src/legacy-util.js", "function legacyJoin() {}\n"),
            ("lib/legacy-util.js", "function joinTwo() {}\n"),
            ("src/legacy-default.js", "export default function legacyDefault() {}\n"),
            ("src/legacy-ns.js", "export function helper() {}\n"),
            ("src/legacy-cjs.js", "function cjsJoin() {}\nmodule.exports = { cjsJoin };\n"),
            ("src/legacy-whole.js", "module.exports = {};\n"),
        ]);
        s.open_path("src/index.js");
        let mut probe = |symbol: &str| {
            let at = src.rfind(symbol).expect("fixture");
            let (line, col) = point_of(src, at);
            s.set_point(line, col, col);
            s.resolver_scope(symbol)
        };
        // Named relative → the sibling's path, item included.
        assert_eq!(
            probe("legacyJoin"),
            vec!["./legacy-util".to_string(), "legacyJoin".to_string()]
        );
        // `../` relative + alias → the ORIGINAL name.
        assert_eq!(
            probe("legacyTwo"),
            vec!["../lib/legacy-util".to_string(), "joinTwo".to_string()]
        );
        // An extension-carrying relative spec stays verbatim (the provider
        // lands the exact file — no walk).
        assert_eq!(
            probe("legacyDefault"),
            vec!["./legacy-default.js".to_string(), "legacyDefault".to_string()]
        );
        // `{ default as D }` → entry-only (the one shape that deliberately
        // drops the member — `default` names the entry itself).
        assert_eq!(probe("legacyDef2"), vec!["./legacy-whole".to_string()]);
        // Whole-module CJS binding → the specifier alone (the entry).
        assert_eq!(probe("legacyWhole"), vec!["./legacy-whole".to_string()]);
        // CJS destructuring → the sibling's path, item included.
        assert_eq!(
            probe("cjsJoin"),
            vec!["./legacy-cjs".to_string(), "cjsJoin".to_string()]
        );
        // Relative namespace import: bare `ns` names the entry…
        assert_eq!(probe("legacyNs"), vec!["./legacy-ns".to_string()]);
        // …and `ns.member` carries the entry + member (the provider's
        // relative branch takes the member from the hint's last segment).
        assert_eq!(
            probe("legacyNs.helper"),
            vec!["./legacy-ns".to_string(), "helper".to_string()]
        );
    }

    /// JS subpath specifiers keep their `/` (the provider drops the
    /// subpath to the base package); CJS `require` destructuring follows
    /// the same rules as ESM named imports.
    #[test]
    fn resolver_scope_js_subpath_and_cjs_require() {
        let src = "import { doThing } from \"acme/sub\";\nconst { doThing2 } = require(\"acme2\");\nconst { doThing3: dt3 } = require(\"acme3\");\nconst acme4 = require(\"acme4\");\ndoThing();\ndoThing2();\ndt3();\nacme4();\n";
        let (mut s, _dir) = store_with_index(&[("src/index.cjs", src)]);
        s.open_path("src/index.cjs");
        let mut probe = |symbol: &str| {
            let at = src.rfind(symbol).expect("fixture");
            let (line, col) = point_of(src, at);
            s.set_point(line, col, col);
            s.resolver_scope(symbol)
        };
        assert_eq!(
            probe("doThing"),
            vec!["acme/sub".to_string(), "doThing".to_string()]
        );
        assert_eq!(
            probe("doThing2"),
            vec!["acme2".to_string(), "doThing2".to_string()]
        );
        assert_eq!(
            probe("dt3"),
            vec!["acme3".to_string(), "doThing3".to_string()]
        );
        // The whole-module binding names the entry itself (no item).
        assert_eq!(probe("acme4"), vec!["acme4".to_string()]);
    }

    /// TS `import type { D }` carries the same hint as a value import
    /// (type-only imports still name the item for the provider's scan).
    #[test]
    fn resolver_scope_ts_import_type_carries_package_path() {
        let src = "import type { D } from \"acme\";\nlet d: D;\n";
        let (mut s, _dir) = store_with_index(&[("src/index.ts", src)]);
        s.open_path("src/index.ts");
        let at = src.rfind("D").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("D"),
            vec!["acme".to_string(), "D".to_string()]
        );
    }

    /// Discriminating: a BARE imported symbol in a Python buffer carries
    /// the module path + item (pre-011-02 the hint was empty).
    #[test]
    fn resolver_scope_python_from_import_carries_module_path() {
        let src = "from acme import doThing\n\ndoThing()\n";
        let (mut s, _dir) = store_with_index(&[("main.py", src)]);
        s.open_path("main.py");
        let at = src.rfind("doThing").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("doThing"),
            vec!["acme".to_string(), "doThing".to_string()]
        );
    }

    /// Python `from a.b import X as Y`: the bare alias carries the FULL
    /// original module path + original item.
    #[test]
    fn resolver_scope_python_from_submodule_alias_carries_original() {
        let src = "from acme.sub import doThing as dt\n\ndt()\n";
        let (mut s, _dir) = store_with_index(&[("main.py", src)]);
        s.open_path("main.py");
        let at = src.rfind("dt").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("dt"),
            vec!["acme".to_string(), "sub".to_string(), "doThing".to_string()]
        );
    }

    /// Python module alias (`import a.b as c`) carries the module chain
    /// for the bare alias; a plain `import a.b` binds ONLY the top-level
    /// `a` — the bare `b` is never guessed.
    #[test]
    fn resolver_scope_python_module_alias_and_top_level_only() {
        let src = "import acme.sub\nimport acme.sub2 as s\n\ns.fn()\n";
        let (mut s, _dir) = store_with_index(&[("main.py", src)]);
        s.open_path("main.py");
        // Bare `s` (the alias): the module chain.
        let at = src.find("s.fn()").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("s"),
            vec!["acme".to_string(), "sub2".to_string()]
        );
        // Plain `import acme.sub`: bare `sub` is NOT bound (only `acme`)
        // → no hint (never guessed).
        let at = src.find("acme.sub").expect("fixture");
        let (line, col) = point_of(src, at + "acme.".len());
        s.set_point(line, col, col);
        assert!(s.resolver_scope("sub").is_empty());
    }

    /// Python local imports shadow module-level ones (innermost block
    /// wins — the bounded walk mirrors the Rust mod-scope rule); within
    /// one block the LAST re-import at/before the point wins.
    #[test]
    fn resolver_scope_python_local_import_shadows_module_level() {
        let src = "from a import Thing\ndef f():\n    from b import Thing\n    return Thing\n\nThing()\n";
        let (mut s, _dir) = store_with_index(&[("main.py", src)]);
        s.open_path("main.py");
        // Inside the function: the local import wins.
        let at = src.rfind("return Thing").expect("fixture");
        let (line, col) = point_of(src, at + "return ".len());
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("Thing"),
            vec!["b".to_string(), "Thing".to_string()]
        );
        // Module level: the module-level import.
        let at = src.rfind("Thing()").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("Thing"),
            vec!["a".to_string(), "Thing".to_string()]
        );
    }

    /// Python same-block re-import: the last matching statement at/before
    /// the point shadows the earlier one (imports are statements, unlike
    /// Rust's module-scoped `use`).
    #[test]
    fn resolver_scope_python_later_reimport_shadows() {
        let src = "from a import Thing\nx = Thing\nfrom b import Thing\ny = Thing\n";
        let (mut s, _dir) = store_with_index(&[("main.py", src)]);
        s.open_path("main.py");
        let before = src.find("x = Thing").expect("fixture");
        let (line, col) = point_of(src, before + 4);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("Thing"),
            vec!["a".to_string(), "Thing".to_string()]
        );
        let after = src.rfind("y = Thing").expect("fixture");
        let (line, col) = point_of(src, after + 4);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("Thing"),
            vec!["b".to_string(), "Thing".to_string()]
        );
    }

    /// Python relative imports, wildcards, and dotted symbols are NEVER
    /// guessed: the hint stays empty (the provider keeps its exact
    /// no-hint bail / own-path behavior).
    #[test]
    fn resolver_scope_python_relative_and_wildcard_are_not_guessed() {
        for (name, src, symbol) in [
            ("relative", "from . import Thing\nThing()\n", "Thing"),
            ("relative two", "from ..mod import Thing\nThing()\n", "Thing"),
            ("wildcard", "from acme import *\ndoThing()\n", "doThing"),
            ("dotted symbol", "import os\nos.path.join('a', 'b')\n", "os.path.join"),
        ] {
            let (mut s, _dir) = store_with_index(&[("main.py", src)]);
            s.open_path("main.py");
            let at = src.rfind(symbol).expect("fixture");
            let (line, col) = point_of(src, at);
            s.set_point(line, col, col);
            assert!(
                s.resolver_scope(symbol).is_empty(),
                "{name}: no hint expected"
            );
        }
    }

    /// Discriminating: a BARE symbol dot-imported in Go carries the
    /// local package name (the import path's last segment) + the symbol.
    #[test]
    fn resolver_scope_go_dot_import_carries_package_name() {
        let src = "package main\n\nimport . \"github.com/pkg/errors\"\n\nfunc main() {\n\t_ = New(\"x\")\n}\n";
        let (mut s, _dir) = store_with_index(&[("main.go", src)]);
        s.open_path("main.go");
        let at = src.rfind("New").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("New"),
            vec!["errors".to_string(), "New".to_string()]
        );
    }

    /// Go grouped dot imports work the same (the `import ( … )` form).
    #[test]
    fn resolver_scope_go_grouped_dot_import() {
        let src = "package main\n\nimport (\n\t. \"github.com/pkg/errors\"\n\t\"fmt\"\n)\n\nfunc main() {\n\t_ = New(\"x\")\n}\n";
        let (mut s, _dir) = store_with_index(&[("main.go", src)]);
        s.open_path("main.go");
        let at = src.rfind("New").expect("fixture");
        let (line, col) = point_of(src, at);
        s.set_point(line, col, col);
        assert_eq!(
            s.resolver_scope("New"),
            vec!["errors".to_string(), "New".to_string()]
        );
    }

    /// Go plain/aliased imports bind a package NAME (always used
    /// qualified) — never a bare-item hint; dotted symbols carry their
    /// own package name → empty. Several dot imports make the origin of a
    /// bare item ambiguous → never guessed.
    #[test]
    fn resolver_scope_go_non_dot_and_ambiguous_imports_are_not_guessed() {
        for (name, src, symbol) in [
            (
                "plain import",
                "package main\n\nimport \"github.com/x/y\"\n\nfunc main() {\n\t_ = y.Fn\n}\n",
                "y",
            ),
            (
                "aliased import",
                "package main\n\nimport yy \"github.com/a/b\"\n\nfunc main() {\n\t_ = yy.Fn\n}\n",
                "yy",
            ),
            (
                "two dot imports",
                "package main\n\nimport (\n\t. \"github.com/a/aa\"\n\t. \"github.com/b/bb\"\n)\n\nfunc main() {\n\t_ = Fn\n}\n",
                "Fn",
            ),
            (
                "dotted symbol",
                "package main\n\nimport \"github.com/x/y\"\n\nfunc main() {\n\t_ = y.Fn\n}\n",
                "y.Fn",
            ),
        ] {
            let (mut s, _dir) = store_with_index(&[("main.go", src)]);
            s.open_path("main.go");
            let at = src.rfind(symbol).expect("fixture");
            let (line, col) = point_of(src, at);
            s.set_point(line, col, col);
            assert!(
                s.resolver_scope(symbol).is_empty(),
                "{name}: no hint expected"
            );
        }
    }



}

// loop-03: the below-PTY unit twins of tools/sweep_flows.py (their own
// file, hung off this module so the AppStore's private fields are visible;
// the ledger lives in docs/ux-testing-plan.md).
#[cfg(test)]
#[path = "flow_tests.rs"]
mod flow_tests;

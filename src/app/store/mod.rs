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
use crate::app::keymap::{Key, KeyCode, KeyMap, KeySeq, KeymapEngine, Lookup, load_bindings, parse_sequence};
use crate::app::watcher::{ActiveWatcher, DEFAULT_DEBOUNCE};
use crate::git::diff::{DiffSide, FileDiff};
use crate::git::status::{RepoStatus, Side};
use crate::git::{GitError, GitRepo};
use crate::model::buffer::{
    is_word_char, load_file, BufferMode, BufferTable, SCRATCH_NAME, UndoStack, UndoStep,
};
use crate::model::files::FileList;
use crate::model::project::{detect_root, Project, ProjectStore};
use crate::model::sections::{MagitRow, RowRole, SectionKind, StatusTree};
use crate::nav::index::{build_index, refresh_in_place, IndexBus, IndexEvent, IndexProgress, SymbolIndex};
use crate::search::occur;
use crate::search::references;
use crate::search::rg::{self, Hit, SearchBus, SearchConfig, SearchEvent};
use redline_syntax::cache::{CacheKey, HighlightCache, TreeKey};
use redline_syntax::highlight::{self, HighlightResult, RetainedTree};
use redline_syntax::registry::{GrammarRegistry, LanguageId};
use crate::theme::Theme;

mod helpers;
pub use self::helpers::reload_anchor;
mod notes_doc;
pub use self::notes_doc::{NotesDoc, parse_notes, serialize_notes};
mod views;
mod project;
mod magit;
mod minibuffer;
mod keys;
mod buffers;
mod notes;
mod search;
mod picker;
mod file_view;
mod index_wiring;
mod navigation;
mod commit;
use self::helpers::{
    blame_line_display, editor_cursor_line, extract_commit_message, file_candidate,
    keep_cursor_visible, log_entry_display, pane_window, prefill_commit_message,
    recenter_top_for, window_slice,
};

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

/// issue-non-rust-receiver-resolution: one fetch-on-demand CONFIRMATION
/// ask, published by the tooling-resolver provider's `confirm_fetch`
/// hook just before it would run an install step (`pip install`,
/// `npm install`, `go mod download`). The provider's hook BLOCKS on the
/// one-shot reply until the operator's `y`/`n` keypress (or the ask's
/// request is superseded — the store declines it then, unblocking the
/// hook; its event is discarded by the generation mismatch).
pub struct FetchConfirmAsk {
    /// The in-flight resolve generation the ask belongs to (the store
    /// only shows the banner for the request still on screen).
    pub generation: usize,
    /// The exact command the operator would be approving (the banner
    /// shows it — `y` never approves something unseen).
    pub command: String,
    /// The file that implied the fetch (workspace-relative — the banner
    /// shows it: `y` never approves something whose origin is unseen).
    pub from_file: String,
    /// The one-shot reply the provider's hook is blocked on (`true` =
    /// approved, `false` = declined / superseded).
    pub reply: std::sync::mpsc::Sender<bool>,
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

// ── Declarative keymap tables (A2): (emacs-notation sequence, command) ──
//
// The old imperative `bind()` calls lived in constructors; these tables are
// loaded through `load_bindings` (the same parser the user-config path uses,
// `Config::validate_bindings`). Row order IS the load order: `bind` validates
// against what is already bound, so a prefix conflict is caught at
// construction (fail-loud, same as the old `.unwrap()`).

/// Global bindings: 23 entries, shared by every view.
pub const GLOBAL_BINDINGS: &[(&str, &str)] = &[
    ("C-g", "cancel"),
    ("M-x", "open-palette"),
    // C-x C-c (quit): bare C-x stays a prefix (pending), so both the
    // C-x C-c global binding and the view-map C-x o / C-x C-i
    // bindings remain reachable.
    ("C-x C-c", "quit"),
    // View cycling (issue 01) was M-s / M-p, but issue 06's `M-s o`
    // (occur) needs the M-s prefix; the engine forbids a command on a
    // strict prefix of a longer binding, so cycling is now M-x only
    // (`cycle-view-next` / `cycle-view-prev`).
    // Browse layer (issue 02).
    ("C-x C-f", "find-file"),
    ("C-x b", "switch-buffer"),
    ("C-x C-b", "list-buffers"),
    ("C-x k", "kill-buffer"),
    ("C-x n", "open-notes"),
    // 015-01: the annotations picker (notes → annotations). `C-c n` is
    // free; a `C-x n …` sequence is impossible — `C-x n` is already a
    // complete binding (open-notes) and the engine forbids a command on
    // a strict prefix of a longer binding.
    ("C-c n a", "annotations-picker"),
    ("C-x C-s", "save-buffer"),
    // plan 005 issue 01: file edit mode (emacs `toggle-read-only`).
    ("C-x C-q", "toggle-read-only"),
    // Magit status (issue 07).
    ("C-x g", "magit-status"),
    // Transient menu (issue 002): `?` opens this view's command menu in
    // any view (magit's hydra tree; `h` does the same in magit views).
    ("?", "open-transient-menu"),
    // Isearch (issue 03).
    ("C-s", "isearch-forward"),
    ("C-r", "isearch-backward"),
    // Projectile prefix (C-c p …): verified projectile-ux keys.
    ("C-c p f", "find-file"),
    ("C-c p p", "switch-project"),
    ("C-c p e", "recent-files"),
    ("C-c p i", "re-walk"),
    ("C-c p s s", "project-search"),
    // Tree sidebar (issue 09).
    ("C-c p t", "toggle-tree"),
    // Search & references (issue 06).
    ("M-?", "references-at-point"),
    ("M-s o", "occur"),
];

/// Buffer view bindings: 53 entries.
pub const BUFFER_BINDINGS: &[(&str, &str)] = &[
    // Bare `q` closes the view (issue 05, finding 5): consistent
    // with the list views. When the main buffer view is the only
    // view, `close-view` is a no-op — it does NOT quit the app
    // (that is still `C-x C-c`).
    ("q", "close-view"),
    ("M-o", "open-scratch"),
    ("C-x o", "open-scratch"),
    // Motion (issue 03 + plan 004 issue 05b). Point motion:
    // C-n/Down and C-p/Up move the point (goal column preserved);
    // C-f/Right and C-b/Left move by character (wrap at EOL/BOL);
    // C-a/C-e jump to line start/end. The window follows the
    // point (the shipped follow-scroll pattern). This supersedes
    // the plan-001 item-4 stopgap that bound the arrows to window
    // scroll (the user directive is explicit: arrows move point).
    ("C-n", "point-down"),
    ("C-p", "point-up"),
    ("DOWN", "point-down"),
    ("UP", "point-up"),
    ("C-f", "point-forward"),
    ("C-b", "point-backward"),
    ("RIGHT", "point-forward"),
    ("LEFT", "point-backward"),
    ("C-a", "point-line-start"),
    ("C-e", "point-line-end"),
    // Word motion (plan 004 issue 05c): M-f / M-b.
    ("M-f", "word-forward"),
    ("M-b", "word-backward"),
    // Window scroll (emacs paging + the non-emacs `j`/`k`):
    // moves the window, the point's screen row stays fixed.
    ("j", "scroll-line-down"),
    ("k", "scroll-line-up"),
    ("C-v", "scroll-page-down"),
    ("M-v", "scroll-page-up"),
    ("PGDN", "scroll-page-down"),
    ("PGUP", "scroll-page-up"),
    // plan 015 issue 03: `C-d` is FREED from half-page scroll (it is not
    // emacs — emacs `C-d` is delete-char, accurate-mode item 5). It now
    // routes to delete-char-forward in `Accurate` mode (via the notes-edit
    // guard) and is unbound in `Annotation` mode; page scrolling keeps
    // `C-v`/`M-v`/PGDN/PGUP. `C-u` STAYS half-page scroll: universal
    // argument (item 9) is out of scope, and un-binding `C-u` without
    // implementing it would remove a working key for nothing.
    ("C-u", "scroll-half-page-up"),
    // `g` = force-reload the current file buffer (issue 04's
    // refresh role; M-< / M-END / G move the point to start/end).
    ("g", "reload-buffer"),
    ("G", "point-buffer-end"),
    ("M-g g", "goto-line"),
    ("M-<", "point-buffer-start"),
    // jump-ambiguity: `M->` frees up the force-definition-list hotkey
    // (user request: "a hotkey to force the list, M-Shift .?" — M-Shift .
    // is M-> in a terminal). `point-buffer-end` moves to `M-END` —
    // verified against the parity reference (vanilla emacs -Q 30.2):
    // `M->` is `end-of-buffer`, but `M-<end>` is
    // `end-of-buffer-OTHER-WINDOW` — a window-splitting command,
    // meaningless under redline's locked single-pane design — so the
    // rebind costs no parity; `G` still binds `point-buffer-end`, so
    // nothing becomes unreachable.
    ("M-END", "point-buffer-end"),
    ("M->", "xref-find-definitions-picker"),
    // Plan 004 row 8: recenter cycle (top → middle → bottom → top).
    ("C-l", "recenter"),
    // Plan 004 issue 03: mark/region + kill ring.
    // C-SPC: set-mark. The terminal delivers C-SPC as NUL,
    // which crossterm/iocraft decode as Char(' ') + CONTROL.
    // The binding must match that representation.
    ("C-SPC", "set-mark"),
    // C-w: kill-region.
    ("C-w", "kill-region"),
    // M-w: copy-region-to-kill-ring.
    ("M-w", "copy-region"),
    // C-y: yank.
    ("C-y", "yank"),
    // M-y: yank-pop.
    ("M-y", "yank-pop"),
    // C-x C-x: exchange point and mark.
    ("C-x C-x", "exchange-point-and-mark"),
    // plan 016 issue 01: undo (undo only; redo + the self-insert-run
    // coalescing rule are issue 04 — without it every keystroke is one undo
    // step, expected at this stage, not a bug). SETTLED by the user on BOTH
    // `C-x u` and `C-/`: `C-x u` fits the `C-x` prefix family (no collision
    // with `C-x 0`/`C-x 1`/`C-x 2`/`C-x C-x`/`C-x o`) and `C-/` is emacs's
    // traditional undo mnemonic. Both are free in this table.
    //
    // Terminal control-code subtlety (PINNED, not assumed — see the
    // input-layer test `crossterm_0x1f_decodes_to_c_7` and the store keymap
    // test `undo_bindings_resolve_and_pin_the_control_code`): `C-/` parses to
    // Char('/') + ctrl. On a byte-based terminal Ctrl+/ and Ctrl-_ both send
    // control byte 0x1F, which crossterm decodes as Char('7') + CONTROL
    // ("C-7"), NOT Char('/') + ctrl — so there the physical Ctrl+/ arrives as
    // C-7 and the `C-/` binding does not fire; on a CSI-u / kitty-protocol
    // terminal the physical Ctrl+/ arrives as Char('/') + ctrl and DOES fire.
    //
    // plan 016 issue 02: the three bindings therefore split terminal
    // coverage (stated, not left to the user to discover):
    //   - `C-x u`: every terminal (two keys, always reachable) — the
    //     universal fallback.
    //   - `C-/`:   CSI-u / kitty-protocol terminals only (a physical Ctrl+/ is
    //              decoded as Char('/') + ctrl and fires this binding).
    //   - `C-7`:   byte-based terminals only (a physical Ctrl+/ / Ctrl-_ sends
    //              0x1F, which crossterm decodes as Char('7') + CONTROL).
    // `C-7` was free in this table; binding it makes byte-based Ctrl+/ undo.
    ("C-x u", "undo"),
    ("C-/", "undo"),
    ("C-7", "undo"),
    // plan 016 issue 04: redo. The binding was settled with the oracle
    // (emacs 30.2, `where-is-internal 'undo-redo` → `C-?` + `C-M-_`; both
    // unusable here: `C-?` is DEL/0x7F, which a terminal delivers as
    // Backspace; `C-M-_` is `ESC 0x1F`). Primary: `C-x U` — free in emacs
    // (NO parity cost, verified with the oracle), byte-reachable on every
    // terminal (a plain character after a prefix), and mnemonic: `C-x u`
    // undoes, `C-x U` redoes, adjacent in the same prefix.
    //
    // Second binding: `C-M-7` — the APP-LEVEL shape of emacs's `C-M-_`
    // on byte-based terminals: Alt+Ctrl-_ sends `ESC 0x1F`, and crossterm
    // 0.29.0's unix parse handles the byte after `ESC` by recursing with
    // the ALT flag OR'd onto whatever the byte decodes to — `0x1F`
    // decodes as `Char('7') + CONTROL` (the same fact that made `C-7` the
    // byte-based undo shape), so the full sequence arrives as
    // `Char('7') + CONTROL | ALT` = `C-M-7`. The decode is pinned in the
    // input-layer test `c_m_7_arrives_from_esc_0x1f`; the app boundary in
    // `redo_bindings_resolve`. NOTE: on a CSI-u / kitty terminal the same
    // physical key arrives as `Char('_') + CONTROL | ALT` (`C-M-_` proper)
    // — deliberately UNBOUND: `C-x U` already covers every terminal, and
    // a second second-binding that cannot be measured end-to-end is worse
    // than none (the bare-`SHIFT` lesson). `C-M-7` was free in this table.
    ("C-x U", "redo"),
    ("C-M-7", "redo"),
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
    ("C-x 0", "close-view"),
    ("C-x 1", "close-other-views"),
    ("C-x 2", "split-window-vertical"),
    // plan 005 issue 02: inline annotations. `A` prompts for a
    // note on the line at point (minibuffer; RET commits and
    // the cue appears immediately); on an annotated line it
    // pre-fills for edit. `d` deletes the annotation on the
    // line at point (a message on an unannotated line — and in
    // EDIT buffers `A`/`d` self-insert as printables, the
    // printable-leaf rule).
    // annotations-fold-visual: the `C-c a` command tree — the bare
    // `C-c a` toggle is gone (the engine forbids a command on a
    // strict prefix of a longer binding, and the user's decision is the
    // tree): `C-c a n` new (genuine alias of `A` → annotate), `C-c a h`
    // TOGGLE the note blocks (genuine alias of `annotate-toggle` — the
    // single fold control; the separate `C-c a s` show is REMOVED, the
    // user asked for one toggle, not a pair), `C-c a l` list (genuine
    // alias of `C-c n a` → annotations-picker). Bare Shift is DELIBERATELY
    // UNBOUND (gate P1 on this lane): crossterm 0.29 only decodes a
    // `KeyCode::Modifier(...)` event when BOTH
    // `KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES` (1) and
    // `REPORT_ALL_KEYS_AS_ESCAPE_CODES` (8) are enabled, and iocraft 0.9.1
    // pushes ONLY `REPORT_EVENT_TYPES` (2) — so no bare-Shift event ever
    // reaches the app, on ANY terminal (byte-based terminals send no
    // bare-Shift bytes at all, kitty-protocol ones included). A binding
    // that never fires is worse than none, so `C-c a h` is the fold path.
    // `C-a` was declined (emacs point-line-start).
    ("A", "annotate"),
    ("d", "annotate-delete"),
    ("C-c a n", "annotate"),
    ("C-c a h", "annotate-toggle"),
    ("C-c a l", "annotations-picker"),
    // Navigation (issue 05).
    ("M-.", "xref-find-definitions"),
    ("M-,", "jump-back"),
    ("C-i", "jump-forward"),
    // C-i (Ctrl+I) arrives as Tab from crossterm: bind Tab too.
    ("TAB", "jump-forward"),
    ("M-i", "imenu"),
];

/// Buffer-list view bindings: 11 entries.
pub const BUFFER_LIST_BINDINGS: &[(&str, &str)] = &[
    ("q", "close-view"),
    ("RET", "open-buffer-list-selected"),
    ("DOWN", "buffer-list-next"),
    ("C-n", "buffer-list-next"),
    ("UP", "buffer-list-prev"),
    ("C-p", "buffer-list-prev"),
    // Issue 05h: `n`/`p` match the magit/log convention (same
    // commands as the arrow / C-n / C-p binds).
    ("n", "buffer-list-next"),
    ("p", "buffer-list-prev"),
    // Issue 05h: `d` is the dired-convention kill verb (parity
    // log row 31). Kills the selected buffer via the same
    // `kill_buffer` path the `C-x k` picker runs; the list
    // stays open.
    ("d", "buffer-list-kill-selected"),
    // PART A fix (item 4): page keys step the selection too.
    ("PGDN", "buffer-list-next"),
    ("PGUP", "buffer-list-prev"),
];

/// Magit-status view bindings: 21 entries.
pub const MAGIT_STATUS_BINDINGS: &[(&str, &str)] = &[
    // Magit dwim keys (issue 07): the section under the cursor
    // determines what `s`/`u`/`RET` do.
    ("q", "close-view"),
    ("s", "magit-stage"),
    ("u", "magit-unstage"),
    ("TAB", "magit-fold"),
    ("RET", "magit-visit-file"),
    ("g", "magit-refresh"),
    ("n", "magit-next"),
    ("C-n", "magit-next"),
    ("p", "magit-prev"),
    ("C-p", "magit-prev"),
    // PART A fix (item 4): arrows + page keys move the cursor too.
    ("DOWN", "magit-next"),
    ("UP", "magit-prev"),
    ("PGDN", "magit-next"),
    ("PGUP", "magit-prev"),
    // Issue 08: the magit-status context keys (log/blame/commit/
    // branch/stash) — redline's binding, documented as a
    // deviation from real magit (where `b` is the branch
    // transient and blame is a file-view prefix).
    ("l", "magit-log"),
    ("b", "magit-blame"),
    ("c", "magit-commit"),
    ("y", "branch-picker"),
    ("z", "stash-list"),
    // Issue 002: `h` is magit's top-level dispatch menu (the
    // same component as `?`), and `k` discards the file/hunk at
    // point (confirmation-gated).
    ("h", "open-transient-menu"),
    ("k", "magit-discard"),
];

/// Log view bindings: 12 entries.
pub const LOG_BINDINGS: &[(&str, &str)] = &[
    // Log (issue 08): n/p page the history, arrows move the
    // in-page selection, RET opens the selected commit's diff,
    // q closes.
    ("q", "close-view"),
    ("n", "log-next-page"),
    ("p", "log-prev-page"),
    ("DOWN", "log-move-down"),
    ("j", "log-move-down"),
    ("C-n", "log-move-down"),
    ("UP", "log-move-up"),
    ("k", "log-move-up"),
    ("C-p", "log-move-up"),
    // PART A fix (item 4): page keys step the selection too.
    ("PGDN", "log-move-down"),
    ("PGUP", "log-move-up"),
    ("RET", "log-open-commit"),
];

/// Blame view bindings: 7 entries.
pub const BLAME_BINDINGS: &[(&str, &str)] = &[
    // Blame (issue 08): read-only; q closes. Emacs motion
    // (issue 003-02): the cursor-following window keeps the
    // selected row in view on every move.
    ("q", "close-view"),
    ("C-n", "blame-next"),
    ("C-p", "blame-prev"),
    ("C-v", "blame-page-down"),
    ("M-v", "blame-page-up"),
    ("M-<", "blame-top"),
    // View-local: the BUFFER view's `M->` now forces the Xref candidate
    // list (jump-ambiguity, the user-requested binding; point-buffer-end
    // moved to `M-END`/`G`). Here `M->` keeps its emacs end-of-buffer
    // analogue — blame-bottom / commit-diff-scroll-bottom below — which is
    // parity-preserving and intentional.
    ("M->", "blame-bottom"),
];

/// Commit-diff view bindings: 7 entries.
pub const COMMIT_DIFF_BINDINGS: &[(&str, &str)] = &[
    // Read-only commit diff (issue 08): q closes back to log.
    // Emacs motion (issue 003-02): the pane has no cursor; these
    // move the window (the FileView vocabulary, no new bindings).
    ("q", "close-view"),
    ("C-n", "commit-diff-scroll-down"),
    ("C-p", "commit-diff-scroll-up"),
    ("C-v", "commit-diff-page-down"),
    ("M-v", "commit-diff-page-up"),
    ("M-<", "commit-diff-scroll-top"),
    ("M->", "commit-diff-scroll-bottom"),
];

/// Commit-editor view bindings: 2 entries.
pub const COMMIT_EDITOR_BINDINGS: &[(&str, &str)] = &[
    // Inline commit editor (issue 08). Printable/motion keys and
    // the ESC/C-g aborts are intercepted in `key_event` before the
    // keymap engine; only the C-c C-c / C-c C-k bindings resolve
    // through the engine (so the `C-c` prefix pending state is
    // visible in the status line). q is deliberately NOT bound:
    // in the message buffer a bare q types "q" (matching magit's
    // message buffer, where q is not a command).
    ("C-c C-c", "commit-editor-commit"),
    ("C-c C-k", "commit-editor-abort"),
];

/// Home view bindings: 0 entries (deliberately empty).
pub const HOME_BINDINGS: &[(&str, &str)] = &[
    // 06a: home has NO view-local bindings. `q` is deliberately
    // unbound (there is no buffer to close); every entry point
    // (C-x C-f, C-x g, C-x n, C-x b, C-x C-c, C-c p …, ?) is a
    // GLOBAL binding, so they all work from home without any
    // home-side inheritance. The render switch still shows home
    // whenever the buffer view has no current buffer.
];

/// Search view bindings: 14 entries.
pub const SEARCH_BINDINGS: &[(&str, &str)] = &[
    // Results view (issue 06): n/p between matches, RET jump
    // (records a jump-stack entry so M-, returns), g re-run,
    // q/ESC close (cancelling an in-flight search), C-g
    // cancels the search without closing.
    ("q", "close-search-view"),
    ("ESC", "close-search-view"),
    ("RET", "search-jump"),
    ("n", "search-next"),
    ("C-n", "search-next"),
    ("p", "search-prev"),
    ("C-p", "search-prev"),
    ("DOWN", "search-next"),
    ("UP", "search-prev"),
    // PART A fix (item 4): page keys step the selection too.
    ("PGDN", "search-next"),
    ("PGUP", "search-prev"),
    ("g", "search-rerun"),
    // Watchlist item 4: `M-,` under the results view — jump-back
    // pops through the sentinel to the pre-search position in
    // one step (the buffer view's M-, only exists once the
    // results view is closed; the sentinel entry's navigation
    // re-opens the view, so the pop must work from it too).
    ("M-,", "jump-back"),
    ("C-g", "search-cancel"),
];

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
        let table = match self {
            ViewId::Buffer => BUFFER_BINDINGS,
            ViewId::BufferList => BUFFER_LIST_BINDINGS,
            ViewId::MagitStatus => MAGIT_STATUS_BINDINGS,
            ViewId::Log => LOG_BINDINGS,
            ViewId::Blame => BLAME_BINDINGS,
            ViewId::CommitDiff => COMMIT_DIFF_BINDINGS,
            ViewId::CommitEditor => COMMIT_EDITOR_BINDINGS,
            ViewId::Home => HOME_BINDINGS,
            ViewId::Search => SEARCH_BINDINGS,
        };
        load_bindings(table)
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
    /// `C-c n a` (015-01): every annotation record in the notes document
    /// (document order — file, then line); RET jumps to the selected
    /// annotation's `(path, line)`, `d` deletes it.
    Annotations,
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
    /// Column (0-based CHAR index within the line; 0 when unknown).
    /// Recorded from `point_col()` (or an explicit byte→char conversion) and
    /// consumed by `set_point` — a byte column would land off-by-N on
    /// multibyte lines.
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
        // Plain point motion (C-n/C-p, arrows, goto-line, isearch, …) never
        // records a jump, so unless the cursor is still where the previous
        // landing put it, the current-position slot (`history[pos]`) is
        // STALE — it holds the previous jump's destination, not where the
        // cursor actually is. Sync the slot to the captured origin so `M-,`
        // returns to the point the jump truly started from (same-file and
        // cross-file alike). A no-op when the cursor has not moved since
        // the last recorded jump.
        if self.history.is_empty() {
            self.history.push(origin.clone());
        } else {
            self.history[self.pos] = origin.clone();
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
/// `display` is the searchable text (nucleo matches it) and, when `label`/`detail`
/// are both empty, the whole left-anchored row.
///
/// The name-first row layout (issue picker-density): when `detail` is non-
/// empty the renderer draws `label` left-anchored and `detail` right-aligned
/// at the candidate column's right edge (the paths form a scannable column).
/// The NAME owns the space: `label` truncates only if the name alone exceeds
/// the whole candidate column, and it is `detail` that truncates (keeping its
/// tail — the file name — so the repetitive path prefix is dropped). When
/// `detail` is empty the renderer draws a single left-anchored string: the
/// `label` when it is non-empty (the non-current branch rows — their
/// `display` keeps the `*` marker slot's leading space for matching, but
/// the name must align with the current branch row's name column), or
/// `display` when the label is empty (the palette — a single command
/// identifier has no context to right-align — and the scratch buffer row,
/// where label == display).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerCandidate {
    pub name: String,
    pub display: String,
    /// Left-anchored row text (the name-first label); also the
    /// single-string fallback when `detail` is empty. `display` (the
    /// nucleo match target, byte-stable) is drawn only when the label
    /// is empty.
    pub label: String,
    /// Right-aligned detail column (`[kind] path:line`); empty = no split.
    pub detail: String,
    pub docs: String,
    pub category: String,
    /// (annotations-picker identity) the annotation record's own `col` —
    /// present ONLY on the Annotations picker's rows, so a line that hosts
    /// several records stays distinguishable (the `detail` `path:line` is
    /// shared by every record on that line). `None` for every other picker.
    /// Rendering caveat (wording, not a defect): a pre-marker-cell legacy
    /// record with a raw mid-token `col` can render sharing its marker cell
    /// with a new sibling on the same line (2 note rows, 1 marker); both
    /// stay visible AND deletable — `ann_col` keeps each addressable even
    /// when the marker cells collide.
    pub ann_col: Option<usize>,
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
/// (the note rendered directly ABOVE the anchored code row `line`,
/// annotations-render-fold — it reads as a header for the code it
/// annotates). Every
/// row carries its buffer-line index so the renderer, the hardware-cursor
/// math, and the click mapping translate `buffer_line` ↔ `rendered_row`
/// both ways — the dense 1:1 "row i == line top+i" assumption of the
/// pre-annotation slice is gone (note rows are extra rows).
#[derive(Clone, Debug, Default)]
pub struct FileViewRow {
    /// The buffer line this row belongs to: the code row itself for code
    /// rows; the anchored line for a virtual note row (a note row sits
    /// IMMEDIATELY BEFORE its code row: for note row index `i`, `rows[i + 1]`
    /// is that code row — annotations-render-fold).
    pub line: usize,
    /// True for a virtual annotation note row (not a buffer line).
    pub is_note: bool,
    /// Code rows only: the line carries at least one annotation (the
    /// margin fold arrow — independent of note-row visibility, `C-c a h`).
    pub annotated: bool,
    /// The indicator column(s) (the `\u{25b4}`/`\u{25b8}` arrow on a code
    /// row, the `\u{256d}` corner on a note row) — the ANCHOR column.
    /// issue-annotations-symbol-precise: the anchor is per-RECORD, not
    /// per-line. A code row carries the SET of its line's indicator columns
    /// (deduplicated, ascending — one `\u{25b4}`/`\u{25b8}` per entry; the
    /// renderer draws them all). Records that resolve to the same column —
    /// e.g. two records on one line both falling back to the same indent
    /// anchor — share that ONE indicator (the dedup is what keeps a code
    /// cell from ever hosting two glyphs). Empty when the line has no
    /// annotations. A note row carries its note slots' anchors, one entry
    /// per slot (issue-annotations-layout: a PACKED note row carries
    /// several, one per `note_slots` entry — no dedup, in slot order):
    /// each corner sits in the same column as that record's arrow on the
    /// code row directly below.
    /// Each entry is the display column of the cell directly BEFORE the
    /// record's symbol when that cell is whitespace (the indicator
    /// overwrites a blank cell — the code does not move); otherwise the
    /// line's indent anchor (`indent_width - 1`, itself column 0 when the
    /// indent is 0, shifting the line right by exactly one cell). Carried
    /// by the STORE (which builds the row and knows the line's full text)
    /// rather than recomputed in the renderer from the (stripped) row text:
    /// the record's stored `col` is a CHAR offset — not a display column —
    /// so the anchor's column is only re-derivable from the un-stripped
    /// line, which the renderer does not hold.
    pub anchors: Vec<usize>,
    /// Code rows only: the display column where `text` is drawn
    /// (issue-annotations-symbol-precise): the line's leading indentation
    /// run's display width when the line has one (the code keeps its source
    /// column; every indicator sits in a blank cell), else column 1 when a
    /// column-0 line is shifted right by exactly one cell for a column-0
    /// indicator, else column 0 (unannotated code rows — and 0 for note
    /// rows, which do not use it).
    pub code_start: usize,
    /// Code rows only: the char (and, for spaces/tabs, byte) count of the
    /// leading indentation run — 0 when the line has no leading
    /// whitespace (the column-0 cases) and 0 for non-annotated rows. The
    /// store strips this run from `text` so the code sits at `code_start`
    /// (the run's display width); the hardware cursor and the click
    /// mapping use it to translate a full-line point column back onto the
    /// stripped row text.
    pub indent_chars: usize,
    /// Code rows only (issue-annotation-marker-cell): the char indexes
    /// into `text` at which the line INSERTS one marker cell — one per
    /// distinct record tied to a symbol whose marker could not overwrite
    /// a blank cell (the cell before its symbol is not whitespace). The
    /// inserted cell sits BEFORE the char at the index, so that char and
    /// its tail each shift right by one display cell: the renderer skips
    /// the cell when placing the text (the marker glyph itself is drawn
    /// from `anchors`), and the hardware cursor / click mapping count the
    /// cell. Empty when no insertion applies (unannotated rows, note
    /// rows, and annotated rows whose markers all overwrote blank cells
    /// or borrowed the line's indent).
    pub insertions: Vec<usize>,
    /// The row's text (without trailing newline; an orphaned note row's
    /// first slot carries an `(orphaned)` tag in its text — note rows are
    /// drawn from `note_slots`, `text` is the first slot's text). For an ANNOTATED code row
    /// the leading indentation run (`indent_chars` chars of spaces/tabs) is
    /// stripped so the code begins at cell `code_start` and no code
    /// character moves (a column-0 line keeps its full text — its
    /// indicators shift the whole line right by one instead when a
    /// column-0 anchor is present, else it stays put); `spans`/`matches`
    /// are re-based by that run's byte length accordingly. Non-annotated and
    /// note rows keep their full text.
    pub text: String,
    /// Highlight spans (code rows only; byte offsets relative to the line
    /// start). Empty for note rows.
    pub spans: Vec<redline_syntax::highlight::LineSpan>,
    /// This line's search-match ranges (issue match-highlight; code rows
    /// only): BYTE offsets relative to the line start, clipped to the line's
    /// byte length. `selected` marks the match the cursor is on (the
    /// renderer's overlay gives its face priority on overlap). Empty for
    /// note rows and when no match context applies to this buffer.
    pub matches: Vec<LineMatch>,
    /// The landing highlight on this line (jump-highlight; code rows
    /// only): the landed-on symbol's BYTE range (line-relative — the same
    /// byte domain as `spans`/`matches`) plus the fade intensity in
    /// `[0, 1]` the snapshot computed for this frame (the pure curve of the
    /// landing's age; 0.0 at or past the duration). `None` for note rows
    /// and whenever no landing highlight applies to this line.
    pub highlight: Option<(usize, usize, f32)>,
    /// Note rows only (issue-annotations-layout): the note SLOTS this
    /// canvas row carries — one `NoteSlot` per annotation note, in
    /// ascending anchor order (record order at a shared anchor). A PACKED
    /// note row carries several slots (the records whose display-cell
    /// footprints do not collide share the row, each ╭ still at its own
    /// anchor cell); a plain note row carries exactly one. Empty for code
    /// rows. `anchors` carries the same columns in the same order, and
    /// `text` is the first slot's text (the renderer draws from
    /// `note_slots`, never from `text` for note rows).
    pub note_slots: Vec<NoteSlot>,
}

/// One annotation note on a (possibly PACKED) note row —
/// issue-annotations-layout. A note row is one CANVAS row: two or more
/// records' notes share it when their display-cell footprints (the ╭
/// cell, the ─ leader, and the text span) do not collide; a colliding
/// note gets its own stacked row instead.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NoteSlot {
    /// The record's anchor (the record's display column): the ╭'s cell —
    /// the SAME cell as that record's ▴ on the code row directly below
    /// (the anchor relationship, asserted per cell in the tests).
    pub anchor: usize,
    /// The ─ leader length in cells (always ≥ 1): the plain note draws a
    /// single bend at `anchor + 1` and the text at `anchor + 2`. A
    /// larger value is the longer leader issue-annotations-layout adds
    /// for the FURTHER-OUT note when its base footprint collides with an
    /// earlier note's on the same line: the text then starts at
    /// `anchor + 1 + leader`, strictly to the right of the colliding
    /// note's text end, so the connector still reads as reaching THIS
    /// record's own anchor rather than its neighbour's. See
    /// `AppStore::pack_line_note_rows` for the stated rule.
    pub leader: usize,
    /// The note text (bare content; an `(orphaned)` suffix when the
    /// record is orphaned — the suffix is part of the footprint).
    pub text: String,
}

impl FileViewRow {
    /// The rendered-row index of buffer line `line`'s CODE row (the note
    /// rows above it never match: they are `is_note`).
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
/// `syntax_*` keys), languages with no identifier kind (Yaml, Markdown,
/// Plain), and points where `node_at` has no identifier-ish answer
/// (keywords, whitespace, operators) — those
/// records ride the text rules alone, exactly as before.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyntaxAnchor {
    /// The tree-sitter node kind — always one of the buffer's language's
    /// identifier-ish kinds (from the descriptor table, e.g. Rust
    /// `identifier` / `scoped_identifier`, Python `identifier` /
    /// `attribute`, Go `identifier` / `selector_expression`).
    pub kind: String,
    /// The node's source text (e.g. `target_one`; a `::` path comes back
    /// whole, exactly as `node_at` returns it — `tokio::spawn`).
    pub name: String,
    /// The enclosing-definition chain (outermost → innermost) at the point
    /// of capture (issue-annotations-symbol-identity, stage 2). This is the
    /// DISAMBIGUATOR for a repeated name: `foo` inside `fn bar` is keyed by
    /// `bar` + `foo`, so it never collides with a `foo` in `baz` or with
    /// `bar`'s own definition. It is stable under an insertion above (the
    /// enclosing definitions don't change).
    ///
    /// `None` for LEGACY records (no `syntax_scope` keys) AND for a top-level
    /// symbol (no enclosing definition to key on) — both ride the scope-blind
    /// `(kind, name)` uniqueness rule exactly as before. Same-scope name
    /// repeats are deliberately treated as AMBIGUOUS (the text rules run,
    /// then `orphaned`) rather than tracked by an ordinal: an ordinal is
    /// unstable under edits (adding/removing a sibling shifts it), so an
    /// ordinal tie would migrate a note to a sibling — a wrong tie, which is
    /// worse than no tie.
    pub scope: Option<Vec<String>>,
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

/// The notes file's structured-section markers (plan 005 issue 02).
pub const NOTES_BEGIN: &str = "<!-- redline-annotations:begin -->";
pub const NOTES_END: &str = "<!-- redline-annotations:end -->";

/// The record start line of the structured section.
pub const NOTES_RECORD_START: &str = "[annotation]";

/// The ±25-line content search window for annotation re-anchoring
/// (plan 005 issue 02).
pub const ANNOTATION_REANCHOR_WINDOW: usize = 25;

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

/// One search-match range on a RENDERED line (issue match-highlight):
/// byte offsets relative to the line start, the end clipped to the line's
/// byte length (a range that would run past the line end — e.g. a stale or
/// line-straddling context — clips rather than overhanging). `selected`
/// marks the match the cursor is on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineMatch {
    /// Byte offset within the line (inclusive).
    pub start: usize,
    /// Byte offset within the line (exclusive, <= the line's byte length).
    pub end: usize,
    /// True for the match the cursor is on (the prominent face).
    pub selected: bool,
}

/// The active match-highlight context for the buffer view (issue
/// match-highlight): the query, its match byte ranges in ONE buffer
/// (buffer-absolute byte offsets, sorted by start), and which one is
/// selected. This is the honest single "what should the buffer highlight
/// right now" field: both match sources feed it — the active isearch (the
/// live `matches`/`current`, synced on every query change and navigation)
/// and the search-results jump (the persisted `search` state, whose hits
/// carry per-file paths, so only THIS buffer's hits enter the context).
///
/// Lifetime (pinned in the store tests): while isearch is active the
/// context tracks it; isearch confirm (RET) and cancel (C-g) BOTH clear
/// it (emacs `isearch-exit` removes the lazy-highlight faces when the
/// search ends); a new search with a DIFFERENT query clears it (a
/// same-query re-run keeps it). A search-results jump REPLACES it with
/// the jumped buffer's hits, and THAT context persists (it is the
/// cursor-visibility case the feature was built for) — until point
/// motion elsewhere, a different query, or closing/cancelling the
/// results view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MatchContext {
    /// The buffer key the ranges belong to (empty = inactive).
    pub buffer_key: String,
    /// The query the ranges were computed for.
    pub query: String,
    /// Match ranges in buffer-absolute byte offsets, sorted by start.
    pub ranges: Vec<(usize, usize)>,
    /// Index into `ranges` of the selected match (out of range = none —
    /// e.g. a jumped hit whose column is undeterminable).
    pub selected: usize,
}

/// The transient landing highlight (jump-highlight): the symbol a jump
/// landed ON — its buffer, line, and the symbol's BYTE range within the
/// line. `set_at` is the animation's t=0. Describes the CURRENT buffer's
/// landing (a cross-file highlight is not rendered anywhere — no
/// per-file map). Lifetime (pinned in the store tests): set during the
/// jump, cleared at the start of the next command dispatch (a jump
/// replaces it).
#[derive(Clone, Debug)]
pub struct LandingHighlight {
    /// The buffer key the landing is in (rendered only while that buffer
    /// is current).
    pub buffer_key: String,
    /// The landing line (0-based).
    pub line: usize,
    /// Byte offset within the line (inclusive).
    pub start: usize,
    /// Byte offset within the line (exclusive, <= the line's byte length).
    pub end: usize,
    /// When the landing was recorded (the fade's t=0).
    pub set_at: std::time::Instant,
}

/// The landing-highlight fade constants (jump-highlight), named in one
/// place: the duration the highlight takes to fade out and the frame
/// interval the animation driver bumps the revision tick at. The driver,
/// the intensity curve, and the tests all read from here. 200 ms / 33 ms
/// (~30 fps, 6-7 frames): clearly perceptible, but shorter than the
/// 250 ms default on purpose — the render loop's cursor workaround
/// (plan 013) runs on every frame it causes, so fewer frames means fewer
/// frames the known CUP race runs on.
pub const JUMP_HIGHLIGHT_DURATION: std::time::Duration = std::time::Duration::from_millis(200);
pub const JUMP_HIGHLIGHT_FRAME: std::time::Duration = std::time::Duration::from_millis(33);

/// The landing-highlight fade curve (jump-highlight): the highlight's
/// intensity as a pure function of the age of a landing and the terminal's
/// color capability — computed in the snapshot (NOT in the renderer), so
/// the curve is unit-testable without rendering anything. With truecolor
/// the face's RGB is interpolated toward the base background, so the
/// intensity decays linearly `1.0 -> 0.0` over `JUMP_HIGHLIGHT_DURATION`
/// (0.0 at or past the duration). Without truecolor a 16-color palette
/// cannot interpolate: the highlight HOLDS at full intensity for the
/// whole duration, then clears (an honest flash — not a fake fade). Either
/// way the curve is monotone non-increasing: `1.0` at the start, `0.0` at
/// or before the duration.
pub fn jump_highlight_intensity(elapsed: std::time::Duration, truecolor: bool) -> f32 {
    if elapsed >= JUMP_HIGHLIGHT_DURATION {
        0.0
    } else if !truecolor {
        1.0 // hold: the palette cannot fade
    } else {
        1.0 - elapsed.as_secs_f32() / JUMP_HIGHLIGHT_DURATION.as_secs_f32()
    }
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
    /// The column (0-based CHAR index — `point_col()`'s unit, consumed
    /// directly by `set_point`) to restore with `pre_search_line` on
    /// cancel.
    pub pre_search_col: usize,
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
            pre_search_col: 0,
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
    /// churn is isolated in `crates/redline-syntax/src/registry.rs`).
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
    /// The active match-highlight context for the buffer view (issue
    /// match-highlight) — see `MatchContext` for the lifetime rule.
    match_context: MatchContext,
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
    // ── jump-ambiguity: the tooling fall-through joins the Xref picker ──
    /// The tooling-resolver result PENDING in the Xref picker: set by
    /// `apply_resolve_event` when a hit lands, cleared when the picker
    /// closes (RET / C-g) or a new M-. press / request supersedes it.
    /// While `Some`, the Xref picker prepends the marked `tooling` row
    /// (preselected), and RET lands through the stored source (the
    /// project-relative or the external read-only open) instead of the
    /// candidate's `file:line` parsing.
    xref_tooling_pending: Option<(ResolvedSource, String)>,
    /// The jump origin captured when the fall-through REQUEST STARTED
    /// (jump-ambiguity: the tooling path is asynchronous, so the picker's
    /// origin is the point at `M-.` time, not at RET time — `M-,` must
    /// return to where the keypress was pressed).
    xref_tooling_origin: Option<JumpEntry>,
    /// The tooling landing's picker index keying (006-03): `None` (the
    /// project index) for the project path's M-., the origin crate's root
    /// for M-. inside an external buffer. Set with `xref_tooling_origin`
    /// in `start_symbol_resolution`.
    xref_tooling_crate_root: Option<PathBuf>,
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
    /// Whether the request of the current `resolve_generation` is still
    /// IN FLIGHT (spawned by `start_symbol_resolution`; cleared when
    /// that generation's event is applied or the generation is bumped
    /// away from it). P3-4, gate: `resolving` is display state — a
    /// stale event may clear it while the current request is still
    /// in flight (006-02b item 3: the stale send can win the watch
    /// slot and lose the current event). The fetch-ask liveness gate
    /// (`apply_fetch_prompt`) keys on THIS, not on the indicator: a
    /// current-generation ask gets the banner exactly while its
    /// request is live (a dead request's ask — no runtime, event
    /// already applied — still declines: the banner never arms for a
    /// request that cannot still ask).
    resolve_in_flight: bool,
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
    // ── jump-highlight: the transient landing highlight ──────────────────
    /// The transient landing highlight (jump-highlight) — see
    /// `LandingHighlight` for the lifetime rule. Cleared at the start of
    /// every command dispatch; set by the jump landing hooks.
    jump_highlight: Option<LandingHighlight>,
    /// The landing-highlight wake channel (jump-highlight): the store owns
    /// the sender; the UI's `use_future` driver in `Root` takes the
    /// receiver exactly once (the issue-04/05 bus precedent). A jump that
    /// sets the highlight sends one message; the bounded (capacity 1)
    /// channel coalesces rapid jump bursts (one animation per burst, the
    /// latest `set_at` wins).
    jump_wake_tx: mpsc::Sender<()>,
    jump_wake_rx: Option<mpsc::Receiver<()>>,
    /// OSC 52 clipboard bus (issue-clipboard-and-selection): the store
    /// keeps the sender for its lifetime; the receiver is handed to
    /// Root's drain exactly once (the `search_rx` precedent). Carries the
    /// FINISHED escape string — `copy_region` builds it (the cap decision
    /// shapes its message), so the drain is a pure writer.
    clipboard_tx: mpsc::UnboundedSender<String>,
    clipboard_rx: Option<mpsc::UnboundedReceiver<String>>,
    /// Drag-select armed state (issue-clipboard-and-selection part 2): the
    /// buffer line a left press landed on, armed by `mouse_drag_begin` (a
    /// press in the file pane with the picker closed); each left DRAG event
    /// rebuilds the whole-line region from this line to the drag line. The
    /// region itself lives in `Buffer.mark`/point and PERSISTS after
    /// release (an emacs mark survives mouse-up, so `M-w` copies what the
    /// drag highlighted). Cleared by `mouse_drag_end` (the release), by a
    /// press in the tree, or by a view switch mid-drag.
    drag_line: Option<usize>,
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
    /// An external-change reload confirm is armed (issue-
    /// external-change-reload): re-opening a file whose on-disk mtime
    /// changed while the buffer carried unsaved edits awaits a `y`/`n`/
    /// C-g decision — `y` reloads from disk (discarding the edits),
    /// `n`/C-g/ESC keep them and the changed-on-disk marker. Holds the
    /// buffer key to confirm.
    reload_confirm: Option<String>,
    // ── issue-non-rust-receiver-resolution: fetch-on-demand confirm ──
    /// The fetch-confirmation bus (issue-non-rust-receiver-resolution):
    /// the store keeps the sender for its lifetime — the resolve
    /// providers' `confirm_fetch` hooks send one
    /// [`FetchConfirmAsk`] per install step they are about to run and
    /// block on its reply; the receiver is handed to Root's
    /// `use_future` drain exactly once (the `search_rx` / `clipboard_rx`
    /// precedent; a test takes it instead).
    fetch_confirm_tx: mpsc::UnboundedSender<FetchConfirmAsk>,
    fetch_confirm_rx: Option<mpsc::UnboundedReceiver<FetchConfirmAsk>>,
    /// The fetch-confirmation prompt on screen (when one is awaiting a
    /// `y`/`n`): the exact command + its in-flight resolve generation.
    /// Every key routes to `fetch_confirm_key` while it is up (the
    /// provider's hook stays blocked until the answer).
    fetch_confirm: Option<FetchConfirmAsk>,
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
    /// Whether the inline annotation note rows render above annotated
    /// lines (annotations-fold-visual: `C-c a h` → `annotate-toggle`
    /// toggles them; the `annotate-fold` command toggles them in read-only
    /// mode — reachable from the M-x palette only, bare Shift is
    /// deliberately unbound — the margin arrows stay either way, the
    /// folded ▸ carrying the state).
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
    // ── plan 016 issue 01: undo ─────────────────────────────────────────
    /// Re-entrancy guard for the undo/redo re-apply path: while `undo` or
    /// `redo` is re-applying an inverse edit through `retain_rope_edit`, the
    /// re-application is NOT recorded as a new undo step (recording it
    /// would make undo flip-flop between two states) and it does not clear
    /// the redo stack. The undo's inverse goes to the redo stack
    /// (`Buffer.redo`); the redo step re-enters the undo stack with its
    /// original id (plan 016 issue 04).
    undo_in_progress: bool,
    // ── plan 016 issue 04: the self-insert-run coalescing rule ────────
    /// The buffer key whose IMMEDIATELY PRECEDING command was a self-insert
    /// (the typing-run marker; the rule is stated in `retain_rope_edit`).
    /// Set ONLY by a successful self-insert on the key path
    /// (`notes_edit_key_event`'s printable branch), and it must survive
    /// ACROSS keystrokes (a run is consecutive self-insert keystrokes). It
    /// is cleared by every path that runs something else: any registry
    /// `dispatch` (motion, kills, yanks, saves, buffer switches,
    /// undo/redo), the non-self-insert editing keys in
    /// `notes_edit_key_event`, the tree-sidebar consuming keys, C-g, every
    /// consuming modal, and the **non-`key_event` input paths** — the mouse
    /// and tree click/wheel handlers (`mouse_click_position`,
    /// `mouse_scroll_up`/`mouse_scroll_down` in `file_view.rs`,
    /// `tree_click_row` in `project.rs`), which is where gate P2 found the
    /// rule being bypassed. So ANY command that RUNS — point motion
    /// (including a click), a kill, a yank, RET, C-o, a save, a buffer
    /// switch, undo/redo, modal input — ends the run. A path that ran NO
    /// command (a pending prefix, an unbound-key echo, a click in a view
    /// that ignores it) deliberately leaves the marker alone. The rule is
    /// about the COMMAND, not a clock: there is no duration and no
    /// "recently" window.
    self_insert_run: Option<String>,
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

        let global = load_bindings(GLOBAL_BINDINGS);

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

        // Fetch-confirmation bus (issue-non-rust-receiver-resolution):
        // same store-owned-sender shape — the resolve providers' confirm
        // hooks send their asks here; Root's drain takes the receiver
        // exactly once (a test takes it instead).
        let (fetch_confirm_tx, fetch_confirm_rx) = mpsc::unbounded_channel();

        // jump-highlight: the landing-highlight wake channel (the store
        // keeps the sender; the Root hook's animation driver takes the
        // receiver exactly once). Capacity 1: rapid jump bursts
        // coalesce into one wake (the latest `set_at` decides the fade).
        let (jump_wake_tx, jump_wake_rx) = mpsc::channel(1);

        // OSC 52 clipboard bus (issue-clipboard-and-selection): the drain
        // in Root takes the receiver exactly once (unbounded — a copy is
        // one escape; nothing can accumulate behind the render loop).
        let (clipboard_tx, clipboard_rx) = mpsc::unbounded_channel();

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
            match_context: MatchContext::default(),
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
            xref_tooling_pending: None,
            xref_tooling_origin: None,
            xref_tooling_crate_root: None,
            impls_keys: Vec::new(),
            resolve_bus: ResolveBus::new(),
            resolve_generation: 0,
            resolving: None,
            resolve_in_flight: false,
            external_buffers: std::collections::HashSet::new(),
            external_indexes: Vec::new(),
            crate_index_bus: CrateIndexBus::new(),
            crate_indexing: Vec::new(),
            xref_crate_root: None,
            search_bus,
            search_rx,
            fetch_confirm_tx,
            fetch_confirm_rx: Some(fetch_confirm_rx),
            fetch_confirm: None,
            index_rx: None,
            jump_highlight: None,
            jump_wake_tx,
            jump_wake_rx: Some(jump_wake_rx),
            clipboard_tx,
            clipboard_rx: Some(clipboard_rx),
            drag_line: None,
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
            reload_confirm: None,
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
            undo_in_progress: false,
            self_insert_run: None,
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
            self.engine.global.bind(&seq, command)?;
        }
        Ok(())
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// jump-highlight: the transient landing highlight, when one is
    /// active (the snapshot reads it to compute the fade intensity; the
    /// animation driver reads it to find the deadline).
    pub fn jump_highlight(&self) -> Option<&LandingHighlight> {
        self.jump_highlight.as_ref()
    }

    /// jump-highlight: take the animation driver's wake receiver out of
    /// the store (exactly once — the Root hook's `use_future` driver
    /// takes it; a second take returns `None`, e.g. on the static render
    /// path or after a test's take). Mirrors `search_rx()`.
    pub fn take_jump_wake_rx(&mut self) -> Option<mpsc::Receiver<()>> {
        self.jump_wake_rx.take()
    }

    /// OSC 52 clipboard (issue-clipboard-and-selection): take the escape
    /// receiver out of the store (exactly once — Root's drain takes it;
    /// a second take returns `None`, e.g. on the static render path or
    /// after a test's take). Mirrors `take_jump_wake_rx`.
    pub fn take_clipboard_rx(&mut self) -> Option<mpsc::UnboundedReceiver<String>> {
        self.clipboard_rx.take()
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














    // ── plan 005 issue 02: inline annotations ────────────────────────




































    // ── plan 004 issue 03: mark / region / kill ring / yank ───────────


















    // ── picker: candidate lists ─────────────────────────────────────────















    // ── picker: open per command ────────────────────────────────────────








    // ── picker: query editing ───────────────────────────────────────────




    // ── picker: selection + preview ─────────────────────────────────────



















    // ── buffers ─────────────────────────────────────────────────────────






    // ── project file-tree sidebar (issue 09) ───────────────────────────












    // ── file view: scroll + isearch + goto-line (issue 03) ────────────

    /// Set the viewport height (in lines); called by the UI on resize.
    pub fn set_viewport_lines(&mut self, n: usize) {
        self.viewport_lines = n.max(1);
    }









    // ── mouse support (issue 09, step 4: best-effort) ─────────────────















    // ── plan 004 issue 05b: file-view point (line, col) + emacs motion ───
























    // ── isearch (C-s / C-r) ────────────────────────────────────────────













    // ── goto-line (M-g g) ──────────────────────────────────────────────









    // ── magit status (issue 07) ─────────────────────────────────────────




















    // ── transient menu (issue 002) ────────────────────────────────────














    // ── discard (issue 002: magit `k`) ───────────────────────────────────






    // ── issue 08: log / blame / commit / branches / stash ────────────────

























































    // ── file watching (issue 04) ───────────────────────────────────────

















    // ── symbol navigation (issue 05) ─────────────────────────────────

























    // ── 011-02: per-language import walks (bare-symbol hints) ────────────

















    // ── external crate index cache (plan 006 issue 03) ───────────────


























    // ── search & references (issue 06) ──────────────────────────────





























    // ── keys & dispatch (unchanged skeleton from issue 01) ──────────────




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
            // (jump-ambiguity) closing the picker drops the pending tooling
            // landing (its row belonged to that picker instance).
            self.xref_tooling_pending = None;
        }
        // Clear the mark on the current buffer (plan 004 issue 03).
        if let Some(key) = self.buffers.current().map(String::from)
            && let Some(buf) = self.buffers.get_mut(&key)
            && buf.mark.is_some()
        {
            buf.mark = None;
            did = true;
        }
        // C-g is a cancel gesture: the search-match highlight goes with the
        // session (issue match-highlight's lifetime rule — it is cleared by
        // cancel, kept by confirm and by point motion).
        self.match_context = MatchContext::default();
        if did {
            self.minibuffer_message("cancel");
        }
    }

    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }


    // ── plan 004 issue 04: quit save-prompt (save-buffers-kill-terminal) ─








}

/// Cursor-move direction for the commit editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorMove {
    Left,
    Right,
    Up,
    Down,
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
mod tests;

// loop-03: the below-PTY unit twins of tools/sweep_flows.py (their own
// file, hung off this module so the AppStore's private fields are visible;
// the ledger lives in docs/ux-testing-plan.md).
#[cfg(test)]
#[path = "../flow_tests.rs"]
mod flow_tests;

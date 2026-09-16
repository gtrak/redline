//! Background symbol indexer (issue 05): a per-session, in-memory per-file
//! symbol table (name, kind, line, byte range) built over the whole project
//! with a rayon-parallel tree-sitter parse, refreshed incrementally on
//! watcher events (only changed files reparse — a full rebuild happens only
//! on project switch or a manual command).
//!
//! Layering (plan): `nav/` is plain Rust — tree-sitter + rayon + tokio are
//! allowed; there is no iocraft. The indexer never holds a lock the UI needs:
//! the heavy parse runs on background threads and hands its result back to the
//! app through the [`IndexBus`] (a `watch` channel, latest-value-wins, mirroring
//! the project-change bus). The app installs the newest result into its store;
//! the UI reads only that installed snapshot.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rayon::prelude::*;
use tokio::sync::watch;

use crate::syntax::queries::extract_symbols;
use crate::syntax::registry::resolve_language;
// Re-exported so the public index/xref API can name the symbol type.
pub use crate::syntax::queries::Symbol;

/// A definition's location in the project: the (project-relative) file and
/// the symbol itself. The tree-sitter `Xref` backend's `find_definition`
/// returns these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub file: String,
    pub symbol: Symbol,
}

/// The in-memory symbol index: per-file outlines plus a
/// name → (file → symbol) map for cross-file definition lookup. Plain data;
/// `Clone` (the app hands a clone of the current index to a background
/// incremental job, and `watch` retains the latest result).
#[derive(Clone, Debug, Default)]
pub struct SymbolIndex {
    /// project-relative path → outline (files with ≥1 symbol only).
    files: HashMap<String, Vec<Symbol>>,
    /// symbol name → (project-relative file → Vec of same-name symbols).
    /// Cross-file lookup; the inner `Vec` preserves all same-file duplicates
    /// (e.g. two `fn f` in different mods of one file).
    by_name: HashMap<String, HashMap<String, Vec<Symbol>>>,
    /// Total symbol count across all files.
    total: usize,
}

impl SymbolIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` when the file has an outline entry.
    #[allow(dead_code)]
    pub fn has(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    /// The outline (symbols) for a project-relative file.
    pub fn outline(&self, path: &str) -> &[Symbol] {
        self.files.get(path).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Total symbol count across all indexed files.
    pub fn total(&self) -> usize {
        self.total
    }

    /// Every definition location for `name` (all files, all symbols of that
    /// name), in a deterministic (file, line, name) order.
    pub fn definitions_of(&self, name: &str) -> Vec<Location> {
        let Some(map) = self.by_name.get(name) else {
            return Vec::new();
        };
        let mut out: Vec<Location> = map
            .iter()
            .flat_map(|(file, syms)| syms.iter().map(move |sym| Location {
                file: file.clone(),
                symbol: sym.clone(),
            }))
            .collect();
        out.sort_by(|a, b| {
            (a.file.as_str(), a.symbol.line, &a.symbol.name).cmp(&(b.file.as_str(), b.symbol.line, &b.symbol.name))
        });
        out
    }

    /// Number of files in which `name` is defined (cross-file).
    #[allow(dead_code)]
    pub fn definition_count(&self, name: &str) -> usize {
        self.by_name.get(name).map(|m| m.len()).unwrap_or(0)
    }

    /// Number of files with ≥1 symbol.
    #[allow(dead_code)]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Every definition location in the index (all files, all symbols).
    pub fn all_locations(&self) -> Vec<Location> {
        let mut out: Vec<Location> = self
            .files
            .iter()
            .flat_map(|(file, syms)| syms.iter().map(|s| Location {
                file: file.clone(),
                symbol: s.clone(),
            }))
            .collect();
        out.sort_by(|a, b| {
            (a.file.as_str(), a.symbol.line, &a.symbol.name).cmp(&(
                b.file.as_str(),
                b.symbol.line,
                &b.symbol.name,
            ))
        });
        out
    }

    /// Replace a file's outline, keeping the cross-file name map consistent.
    /// Returns `true` when the stored outline actually changed.
    pub fn set_file(&mut self, path: &str, symbols: Vec<Symbol>) -> bool {
        let old = self.files.remove(path);
        let same = old.as_ref() == Some(&symbols);
        if same {
            // Restore the unchanged entry and bail.
            self.files.insert(path.to_string(), symbols);
            return false;
        }
        // Drop this file's contributions from the name map.
        if let Some(old_syms) = &old {
            for s in old_syms {
                if let Some(m) = self.by_name.get_mut(&s.name) {
                    m.remove(path);
                    if m.is_empty() {
                        self.by_name.remove(&s.name);
                    }
                }
            }
        }
        self.total = self.total.saturating_sub(old.map(|v| v.len()).unwrap_or(0));
        if symbols.is_empty() {
            return true;
        }
        for s in &symbols {
            self.by_name
                .entry(s.name.clone())
                .or_default()
                .entry(path.to_string())
                .or_default()
                .push(s.clone());
        }
        self.files.insert(path.to_string(), symbols);
        self.total = self.total.saturating_add(self.files[path].len());
        true
    }

    /// Remove a file's outline (its symbols) from the index. Returns `true`
    /// when the file had entries.
    pub fn remove_file(&mut self, path: &str) -> bool {
        let Some(old) = self.files.remove(path) else {
            return false;
        };
        for s in &old {
            if let Some(m) = self.by_name.get_mut(&s.name) {
                m.remove(path);
                if m.is_empty() {
                    self.by_name.remove(&s.name);
                }
            }
        }
        self.total = self.total.saturating_sub(old.len());
        true
    }
}

/// A cheap shared progress counter for a background index job: the number of
/// files parsed so far (`done`) of `total`. The app reads it to render a
/// status-line progress indicator while a job is in flight.
#[derive(Clone, Default, Debug)]
#[allow(dead_code)] // used by the background index thread (future wiring)
pub struct IndexProgress {
    done: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
}

#[allow(dead_code)] // used by tests and future background wiring
impl IndexProgress {
    pub fn new(total: usize) -> Self {
        Self {
            done: Arc::new(AtomicUsize::new(0)),
            total: Arc::new(AtomicUsize::new(total)),
        }
    }

    /// Mark one more file parsed (called by each rayon worker).
    pub fn mark_done(&self) {
        self.done.fetch_add(1, Ordering::Relaxed);
    }

    pub fn done(&self) -> usize {
        self.done.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> usize {
        self.total.load(Ordering::Relaxed)
    }

    pub fn finished(&self) -> bool {
        self.done.load(Ordering::Relaxed) >= self.total.load(Ordering::Relaxed)
    }
}

/// Parse one project file and extract its definition symbols. Plain text and
/// unreadable files yield an empty list.
#[allow(dead_code)] // used by the background index thread (future wiring)
pub fn extract_file(root: &Path, rel: &str) -> Vec<Symbol> {
    let abs = root.join(rel);
    let text = match std::fs::read_to_string(&abs) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let lang = resolve_language(rel);
    extract_symbols(lang, &text)
}

/// Build a full index for `root` over the (project-relative) `files` list
/// using a rayon-parallel parse. **Blocking** — call from a background
/// thread (the app wraps this in `spawn_blocking`). `progress` (if given) is
/// advanced to the file count as each file finishes.
#[allow(dead_code)] // used by the background index thread (future wiring)
pub fn build_index(root: &Path, files: &[String], progress: Option<&IndexProgress>) -> SymbolIndex {
    // Rayon fan-in: parse every file in parallel. Each worker owns its own
    // thread-local parser (see `queries::extract_symbols`); `progress` is
    // shared and cheap to bump from any worker.
    let entries: Vec<(String, Vec<Symbol>)> = files
        .par_iter()
        .map(|rel| {
            let syms = extract_file(root, rel);
            if let Some(p) = progress {
                p.mark_done();
            }
            (rel.clone(), syms)
        })
        .collect();

    let mut index = SymbolIndex::new();
    for (rel, syms) in entries {
        if !syms.is_empty() {
            index.set_file(&rel, syms);
        }
    }
    index
}

/// Re-parse ONLY the `changed` (absolute) files and update `self` in place;
/// deleted/unreadable files have their entries dropped. Returns the set of
/// project-relative paths actually touched — this is the O(changed-files)
/// incremental refresh (every other file's outline is left untouched).
#[allow(dead_code)] // used by the background index thread (future wiring)
pub fn refresh_in_place(root: &Path, changed: &[PathBuf], index: &mut SymbolIndex) -> Vec<String> {
    let mut touched = Vec::new();
    for abs in changed {
        let Some(rel) = abs.strip_prefix(root).ok() else {
            continue;
        };
        let rel = rel.to_string_lossy().into_owned();
        match std::fs::read_to_string(abs) {
            Ok(text) => {
                let lang = resolve_language(&rel);
                let syms = extract_symbols(lang, &text);
                index.set_file(&rel, syms);
            }
            // Deleted / unreadable: drop its outline (no-op if absent).
            Err(_) => {
                index.remove_file(&rel);
            }
        }
        touched.push(rel);
    }
    touched
}

/// The innermost (narrowest) symbol whose `[line, end_line]` extent contains
/// `line`; `None` when the cursor line is outside every definition. Used for
/// which-function. Pure (testable in isolation).
pub fn enclosing_symbol(outline: &[Symbol], line: usize) -> Option<&Symbol> {
    let mut best: Option<&Symbol> = None;
    for s in outline {
        if s.line <= line && line <= s.end_line {
            let span = s.end_line.saturating_sub(s.line);
            let take = match best {
                None => true,
                Some(b) => {
                    let bspan = b.end_line.saturating_sub(b.line);
                    span < bspan || (span == bspan && s.line > b.line)
                }
            };
            if take {
                best = Some(s);
            }
        }
    }
    best
}

/// A result the background indexer publishes to the app via the [`IndexBus`].
/// `index` is the (possibly incremental) snapshot to install; `indexing`
/// drives the status-line indicator; `done`/`total` are the progress;
/// `generation` is the generation counter at the time the job was started,
/// used to discard events from stale jobs (previous project).
#[derive(Clone, Debug, Default)]
pub struct IndexEvent {
    pub index: SymbolIndex,
    pub indexing: bool,
    pub done: usize,
    pub total: usize,
    pub generation: usize,
}

/// The index-result bus: a `watch` channel over [`IndexEvent`]. The background
/// indexer publishes; the app subscribes (a `use_future`) and installs the
/// newest result into the store.
///
/// **Concurrency contract:** the `watch` channel retains the latest
/// *published* value, not the newest *index*: a job that cloned an older
/// base can publish last and erase another job's file updates (lost update).
/// The store wiring therefore runs **at most one index job per generation**
/// at a time. Changed paths arriving while a job is in flight are accumulated
/// in a pending set and coalesced into one incremental job when the flight
/// clears. Events carrying a stale `generation` (from a previous project's
/// job) are discarded by [`AppStore::apply_index_event`](crate::app::store::AppStore::apply_index_event).
#[derive(Clone, Default)]
pub struct IndexBus {
    tx: watch::Sender<IndexEvent>,
}

impl std::fmt::Debug for IndexBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexBus").finish()
    }
}

impl IndexBus {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(IndexEvent::default());
        Self { tx }
    }

    /// Subscribe: a receiver at the current value. `changed()` fires on the
    /// next publish (latest value wins).
    pub fn subscribe(&self) -> watch::Receiver<IndexEvent> {
        self.tx.subscribe()
    }

    /// Publish an index result (sync; safe from a background thread).
    #[allow(dead_code)] // used by the background index thread (future wiring)
    pub fn send(&self, event: IndexEvent) {
        let _ = self.tx.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        fs::write(
            root.join("src/main.rs"),
            "mod lib {\n    pub fn target() {}\n}\nfn main() { lib::target(); }\n",
        )
        .unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn target() {}\npub fn other() {}\n").unwrap();
        fs::write(root.join("src/notes.md"), "# Title\n\n## Sub\n\ntext\n").unwrap();
        fs::write(root.join("src/data.json"), "{\n  \"k1\": 1,\n  \"k2\": {\"k3\": 2}\n}\n").unwrap();
        fs::write(root.join("src/readme.txt"), "just some plain text\n").unwrap();
        dir
    }

    fn rel_files(root: &Path) -> Vec<String> {
        let list = crate::model::files::FileList::build(root).unwrap();
        list.files
    }

    #[test]
    fn parallel_index_finds_cross_file_definitions() {
        let dir = make_project();
        let root = dir.path();
        let files = rel_files(root);
        let index = build_index(root, &files, None);

        // `target` is defined in two files (main.rs's mod lib + lib.rs).
        assert_eq!(index.definition_count("target"), 2, "`target` defined in two files");
        // `other` is defined once (lib.rs).
        assert_eq!(index.definition_count("other"), 1);
        // `main` once (main.rs).
        assert_eq!(index.definition_count("main"), 1);
        // Markdown heading + JSON keys are present.
        assert_eq!(index.definition_count("Title"), 1);
        assert_eq!(index.definition_count("k1"), 1);
        assert_eq!(index.definition_count("k3"), 1);
        // Plain-text (no code grammar) contributes no outline.
        assert!(!index.has("src/readme.txt"), "readme.txt must be plain: no symbols");
        // The TOML key `name` (not the value string) is a symbol.
        assert_eq!(index.definition_count("name"), 1, "TOML key `name` is a symbol");
    }

    #[test]
    fn incremental_refresh_reparses_only_changed_file() {
        let dir = make_project();
        let root = dir.path();
        let files = rel_files(root);
        let mut index = build_index(root, &files, None);
        let baseline_lib = index.outline("src/lib.rs").to_vec();
        let baseline_notes = index.outline("src/notes.md").to_vec();

        // Change only src/lib.rs: rename `other` to `renamed`.
        let path = root.join("src/lib.rs");
        fs::write(&path, "pub fn target() {}\npub fn renamed() {}\n").unwrap();

        let touched = refresh_in_place(root, &[path], &mut index);
        assert_eq!(touched, vec!["src/lib.rs".to_string()], "{touched:?}");

        // lib.rs's outline reflects the change; the other files are untouched.
        let names: Vec<&str> = index.outline("src/lib.rs").iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"renamed"), "{names:?}");
        assert!(!names.contains(&"other"), "{names:?}");
        // `renamed` is now resolvable cross-file; `other` is gone.
        assert_eq!(index.definition_count("renamed"), 1);
        assert_eq!(index.definition_count("other"), 0);
        // The untouched file's outline is byte-for-byte identical.
        assert_eq!(index.outline("src/notes.md"), baseline_notes.as_slice());
        // And lib.rs's own baseline (pre-change) differs from the post-change.
        assert_ne!(index.outline("src/lib.rs"), baseline_lib.as_slice());
    }

    #[test]
    fn refresh_removes_outline_for_deleted_file() {
        let dir = make_project();
        let root = dir.path();
        let files = rel_files(root);
        let mut index = build_index(root, &files, None);
        assert!(index.has("src/notes.md"));
        let path = root.join("src/notes.md");
        fs::remove_file(&path).unwrap();
        refresh_in_place(root, &[path], &mut index);
        assert!(!index.has("src/notes.md"));
        assert_eq!(index.definition_count("Title"), 0);
    }

    #[test]
    fn progress_counter_reaches_total() {
        let dir = make_project();
        let root = dir.path();
        let files = rel_files(root);
        let progress = IndexProgress::new(files.len());
        let _ = build_index(root, &files, Some(&progress));
        assert_eq!(progress.done(), files.len(), "progress must reach total");
        assert!(progress.finished());
    }

    #[test]
    fn enclosing_symbol_picks_innermost() {
        // mod > fn; a cursor inside the function body resolves to the fn,
        // and a cursor inside the mod but outside the fn resolves to the mod.
        let src = "mod outer {\n    fn f() {\n        g()\n    }\n}\n";
        let syms = extract_symbols(crate::syntax::registry::LanguageId::Rust, src);
        // mod outer: line 0..4 ; fn f: line 1..3
        let f_line = 2; // "g()"
        let enc = enclosing_symbol(&syms, f_line).expect("enclosing at line 2");
        assert_eq!(enc.name, "f", "innermost is the function, got {enc:?}");
        // Line 0 is the mod's own line (the fn starts at line 1).
        let enc_mod = enclosing_symbol(&syms, 0).expect("enclosing at line 0");
        assert_eq!(enc_mod.name, "outer");
        // Outside everything.
        assert!(enclosing_symbol(&syms, 99).is_none());
    }

    #[test]
    fn index_bus_latest_value_wins() {
        let bus = IndexBus::new();
        let mut rx = bus.subscribe();
        bus.send(IndexEvent { indexing: true, total: 3, ..Default::default() });
        bus.send(IndexEvent { indexing: false, done: 3, total: 3, ..Default::default() });
        // A receiver sees the newest published value (latest-value-wins).
        assert!(rx.has_changed().unwrap(), "a publish changed the value");
        let got = rx.borrow_and_update().clone();
        assert!(!got.indexing, "latest event wins");
        assert_eq!(got.total, 3);
    }

    #[test]
    fn same_file_duplicate_names_preserved() {
        // Two same-named functions in different mods of one file:
        // `by_name` must keep both (not last-wins).
        let src = "mod a { pub fn target() {} }\nmod b { pub fn target() {} }\n";
        let syms = extract_symbols(crate::syntax::registry::LanguageId::Rust, src);
        // Two `target` functions + two `mod` items = 4 symbols total.
        let targets: Vec<_> = syms.iter().filter(|s| s.name == "target").collect();
        assert_eq!(targets.len(), 2, "two `target` definitions: {syms:?}");
        let mut idx = SymbolIndex::new();
        idx.set_file("dup.rs", syms);
        // `definitions_of` returns both; `definition_count` is 1 (one file).
        let defs = idx.definitions_of("target");
        assert_eq!(defs.len(), 2, "both same-file duplicates preserved: {defs:?}");
        assert_eq!(idx.definition_count("target"), 1, "one file defines `target`");
        // They have different line numbers.
        assert_ne!(defs[0].symbol.line, defs[1].symbol.line);
    }
}


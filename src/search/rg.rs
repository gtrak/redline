//! Embedded-ripgrep search pipeline (issue 06): a streaming, cancelable
//! walk-and-search over a project root.
//!
//! Shape (per the ripgrep-crates skill's reference pipeline):
//! `ignore::WalkBuilder::build_parallel` walks the root (gitignore-aware,
//! hidden-skip, `.git` never descended); each worker thread owns a
//! `grep_searcher::Searcher` (the searcher takes `&mut self`, so it is
//! per-thread) and runs it over each file with a [`StreamSink`] that posts
//! [`SearchEvent`]s to the [`SearchBus`] as hits are found. Results
//! therefore stream: the first hits are observable before the walk
//! finishes.
//!
//! Cancellation is cooperative (the only stop points the embedded crates
//! expose): a shared `Arc<AtomicBool>` checked (1) in the walk's per-entry
//! closure (`WalkState::Quit`) and (2) in every sink callback
//! (`Ok(false)` stops the current file). A canceled search cannot pin a
//! thread walking a huge repo.
//!
//! Layering: plain Rust — grep crates + ignore + tokio (the bus channel) +
//! std threads; zero iocraft. The store consumes the bus through the
//! established async→store pattern (issue 04's bus precedent).

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkFinish, SinkMatch};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::{DirEntry, WalkBuilder, WalkState};
use tokio::sync::mpsc;

use crate::model::files::{gitignore_matches, load_gitignore};

/// One match found by the pipeline: the (project-relative) file, the
/// 1-based line number, the line's text (terminator stripped, UTF-8
/// lossy), and the match's byte column within the line when it is
/// determinable (`None` for a non-literal regex pattern — the line number
/// is always exact).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub file: String,
    pub line_no: u64,
    pub col: Option<u64>,
    pub line: String,
}

/// A pipeline event, streamed to the store. `generation` tags the search
/// job that produced the event so the store discards stale jobs (same
/// contract as `nav::index::IndexEvent`).
#[derive(Clone, Debug)]
pub enum SearchEvent {
    Hit {
        file: String,
        line_no: u64,
        col: Option<u64>,
        line: String,
        generation: usize,
    },
    /// A file finished (its hit count is final); drives per-file counts.
    FileDone {
        file: String,
        hits: u64,
        generation: usize,
    },
    /// The whole search finished (or was canceled).
    Finished {
        cancelled: bool,
        generation: usize,
    },
    /// The search could not start (e.g. an invalid regex).
    Error {
        message: String,
        generation: usize,
    },
}

impl SearchEvent {
    /// The job generation this event belongs to.
    pub fn generation(&self) -> usize {
        match self {
            SearchEvent::Hit { generation, .. }
            | SearchEvent::FileDone { generation, .. }
            | SearchEvent::Finished { generation, .. }
            | SearchEvent::Error { generation, .. } => *generation,
        }
    }
}

/// The search-event bus: an unbounded tokio mpsc over [`SearchEvent`].
/// Unbounded (not a `watch` channel) because search results are a stream
/// of many events, not a latest-value-wins broadcast. The store keeps the
/// sender side for its lifetime (so the receiver never sees
/// `Disconnected` while the store lives); each search job clones a sender
/// for its worker thread.
#[derive(Clone)]
pub struct SearchBus {
    tx: mpsc::UnboundedSender<SearchEvent>,
}

impl std::fmt::Debug for SearchBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchBus").finish()
    }
}

impl SearchBus {
    /// Create the bus; returns (the store-kept handle, the UI drain's
    /// receiver).
    pub fn new() -> (Self, mpsc::UnboundedReceiver<SearchEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx }, rx)
    }

    /// A sender clone for one search job (sync `send`, safe from a
    /// worker thread).
    pub fn sender(&self) -> mpsc::UnboundedSender<SearchEvent> {
        self.tx.clone()
    }

    /// Publish an event (sync; safe from a background thread).
    pub fn send(&self, event: SearchEvent) {
        let _ = self.tx.send(event);
    }
}

impl Default for SearchBus {
    fn default() -> Self {
        let (bus, _rx) = Self::new();
        bus
    }
}

/// A per-file token-class filter: given the file path and its text,
/// returns the absolute byte ranges whose hits must be dropped (e.g.
/// comment/string ranges from tree-sitter), or `None` when the file gets
/// no filtering (plain text — the documented fallback).
pub type FileFilter = dyn Fn(&Path, &str) -> Option<Vec<Range<usize>>> + Send + Sync;

/// Everything a project-wide search needs. Built by the app layer;
/// consumed by [`spawn`] on a fresh worker thread.
pub struct SearchConfig {
    /// The walk root (the project root).
    pub root: PathBuf,
    /// The regex pattern (or literal, when `fixed`).
    pub pattern: String,
    /// Require word-boundary matches (the `M-?` references case).
    pub word: bool,
    /// Match the pattern as a literal string (not a regex).
    pub fixed: bool,
    /// Smart case: case-insensitive only when the pattern's literals are
    /// all lowercase (the `C-c p s s` behavior).
    pub case_smart: bool,
    /// Unconditionally case-insensitive.
    pub case_insensitive: bool,
    /// Path glob filter (rg `-g` / gitignore-glob semantics); `None` =
    /// no restriction.
    pub glob: Option<String>,
    /// File-type filter (rg `--type`, e.g. `"rust"`); `None` = no
    /// restriction.
    pub file_type: Option<String>,
    /// Per-file token-class filter (`None` = no filtering).
    pub filter: Option<Arc<FileFilter>>,
    /// The cooperative cancel flag (checked between files and in every
    /// sink callback).
    pub cancel: Arc<AtomicBool>,
}

/// Spawn the streaming, cancelable project-wide search on a worker
/// thread; events flow to `bus`. Returns immediately (the walk runs in
/// the background).
pub fn spawn(cfg: SearchConfig, bus: &SearchBus, generation: usize) {
    let root = cfg.root.clone();
    let bus = bus.clone();
    std::thread::Builder::new()
        .name("redline-search".into())
        .spawn(move || run(cfg, bus, generation, root))
        .expect("spawn search thread");
}

fn run(cfg: SearchConfig, bus: SearchBus, generation: usize, root: PathBuf) {
    let tx = bus.sender();
    let gen_id = generation;

    let matcher = match RegexMatcherBuilder::new()
        .case_insensitive(cfg.case_insensitive)
        .case_smart(cfg.case_smart)
        .word(cfg.word)
        .fixed_strings(cfg.fixed)
        .build(&cfg.pattern)
    {
        Ok(m) => m,
        Err(e) => {
            bus.send(SearchEvent::Error {
                message: e.to_string(),
                generation: gen_id,
            });
            return;
        }
    };

    // One builder; each worker thread builds its own Searcher
    // (`search_path` takes `&mut self`, so a Searcher is per-thread).
    let mut searcher_builder = SearcherBuilder::new();
    searcher_builder.line_number(true);
    searcher_builder.binary_detection(BinaryDetection::quit(b'\0')); // rg-CLI-like NUL heuristic

    // Glob + gitignore entry filters (rg -g semantics via a
    // gitignore-style matcher; `*` crosses `/`). `filter_entry` takes
    // only ONE predicate, so all entry rules are composed in one closure
    // (see `attach_entry_filters`).
    let mut builder = WalkBuilder::new(&root);
    builder.threads(0);
    // `.git` is never descended by the walk (built-in). Gitignore: native
    // inside a git repo; marker-only (non-git) projects get the same
    // per-directory matcher stack as the file-list walk.
    let is_git_repo = root.join(".git").exists();
    builder.git_ignore(is_git_repo);
    if !is_git_repo || cfg.glob.as_deref().is_some_and(|g| !g.is_empty()) {
        attach_entry_filters(&mut builder, &root, is_git_repo, cfg.glob.as_deref());
    }

    // Type filter (rg --type): only files of the selected type. The
    // pinned `TypesBuilder` starts with NO type selected (an
    // unselected `types` filter matches everything), so redline loads
    // the standard type definitions and selects the named type; a name
    // the standard definitions lack is registered from the built-in
    // name→glob table.
    if let Some(t) = cfg.file_type.as_deref().filter(|t| !t.is_empty()) {
        let mut types_builder = ignore::types::TypesBuilder::new();
        types_builder.add_defaults();
        let known = types_builder.definitions().iter().any(|d| d.name() == t);
        if !known {
            let Some(globs) = type_globs(t) else {
                bus.send(SearchEvent::Error {
                    message: format!("unknown file type `{t}`"),
                    generation: gen_id,
                });
                return;
            };
            for glob in globs {
                if types_builder.add(t, glob).is_err() {
                    bus.send(SearchEvent::Error {
                        message: format!("file type `{t}` is not usable"),
                        generation: gen_id,
                    });
                    return;
                }
            }
        }
        types_builder.select(t);
        match types_builder.build() {
            Ok(types) => {
                builder.types(types);
            }
            Err(e) => {
                bus.send(SearchEvent::Error {
                    message: format!("file type `{t}` is not usable: {e}"),
                    generation: gen_id,
                });
                return;
            }
        }
    }

    let walker = builder.build_parallel();
    let cancel = cfg.cancel.clone();
    let filter = cfg.filter.clone();
    let literal = if cfg.fixed { Some(cfg.pattern.clone()) } else { None };
    let word = cfg.word;

    // A worker-thread panic must never leave the bus unterminated: report
    // it as an Error and still emit the terminal Finished (the plan's
    // stream contract: the stream always terminates with Finished/Error).
    let walk = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| walker.run(|| {
        // Once per worker thread: clone the shared handles; each thread
        // builds its own Searcher from the shared builder (`build` takes
        // `&self`) and borrows the shared matcher (`&RegexMatcher`
        // implements `Matcher` via the blanket impl).
        let matcher = &matcher;
        let root = &root;
        let mut searcher = searcher_builder.build();
        let tx = tx.clone();
        let cancel = cancel.clone();
        let filter = filter.clone();
        let literal = literal.clone();
        Box::new(move |entry: Result<DirEntry, ignore::Error>| -> WalkState {
            if cancel.load(Ordering::Relaxed) {
                return WalkState::Quit; // verified: quits "as soon as possible" (async)
            }
            let Ok(entry) = entry else {
                return WalkState::Continue;
            };
            let Some(ft) = entry.file_type() else {
                return WalkState::Continue;
            };
            if !ft.is_file() {
                return WalkState::Continue;
            }
            let path = entry.path();
            let Ok(rel) = path.strip_prefix(root) else {
                return WalkState::Continue;
            };
            if rel.as_os_str().is_empty() {
                return WalkState::Continue;
            }
            let file = rel.to_string_lossy().into_owned();

            // Token-class filtering (references): parse the file and
            // compute the comment/string ranges up front; the sink drops
            // hits that fall inside them.
            let ranges = filter
                .as_ref()
                .and_then(|f| std::fs::read_to_string(path).ok().and_then(|text| f(path, &text)));

            let mut sink = StreamSink {
                file,
                tx: tx.clone(),
                gen_id,
                cancel: cancel.clone(),
                literal: literal.clone(),
                word,
                ranges,
                hits: 0,
            };
            if let Err(e) = searcher.search_path(matcher, path, &mut sink) {
                tracing::warn!(path = %path.display(), error = %e, "search: cannot search file");
            }
            WalkState::Continue
        })
    })));
    if let Err(payload) = walk {
        let msg = match payload.downcast::<String>() {
            Ok(s) => (*s).clone(),
            Err(p) => p
                .downcast::<&str>()
                .map(|s| s.to_string())
                .unwrap_or_else(|_| "search walk panicked (unknown payload)".into()),
        };
        tracing::error!(error = %msg, "search walk panicked");
        bus.send(SearchEvent::Error {
            message: format!("search crashed: {msg}"),
            generation: gen_id,
        });
    }
    bus.send(SearchEvent::Finished {
        cancelled: cancel.load(Ordering::Relaxed),
        generation: gen_id,
    });
}

/// Attach the combined entry predicate (project gitignore for non-git
/// projects + optional rg-`-g` glob). `filter_entry` takes only ONE
/// predicate (later calls override), so all rules are composed into one
/// closure.
///
/// - git repo, no glob: nothing to attach (native gitignore).
/// - git repo + glob: the glob only.
/// - non-git, no glob: the project `.gitignore` files (ancestor chain).
/// - non-git + glob: the glob and the project gitignore in one closure.
///
/// The glob follows rg `-g` semantics: a POSITIVE glob keeps only
/// matching files (directories always pass so the walk descends); a
/// `!`-prefixed glob EXCLUDES matching files. The inner glob is
/// registered in a gitignore-style matcher as the matching engine
/// (`*` crosses `/`, like rg's globs); a matching pattern reads as
/// `is_ignore` there, so the `!` inversion is applied in the predicate,
/// not by the matcher.
fn attach_entry_filters(
    builder: &mut WalkBuilder,
    root: &Path,
    is_git_repo: bool,
    glob: Option<&str>,
) {
    let (glob_matcher, glob_negate): (Option<Arc<Gitignore>>, bool) = glob
        .filter(|g| !g.is_empty())
        .map(|pattern| {
            let negate = pattern.starts_with('!');
            let inner = pattern.strip_prefix('!').unwrap_or(pattern);
            let mut gb = GitignoreBuilder::new("");
            let matcher = gb
                .add_line(None, inner)
                .ok()
                .and_then(|b| b.build().ok())
                .map(Arc::new);
            (matcher, negate)
        })
        .unwrap_or((None, false));

    // rg `-g` verdict for a file: drop it exactly when `matched ==
    // negate` (positive glob: drop non-matches; `!` glob: drop matches).
    // Directories always pass on the glob alone.
    let glob_drops = |gm: &Option<Arc<Gitignore>>, negate: bool, path: &Path| match gm {
        Some(gm) => gm.matched(path, false).is_ignore() == negate,
        None => false,
    };

    if is_git_repo {
        if glob_matcher.is_some() {
            builder.filter_entry(move |entry: &DirEntry| {
                let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
                is_dir || !glob_drops(&glob_matcher, glob_negate, entry.path())
            });
        }
        return;
    }

    // Non-git: additionally apply the project's `.gitignore` files — the
    // ancestor chain derived per entry from its own path (memoized per
    // directory). The sequential file-list walk's shared per-directory
    // stack is not safe under a parallel walk, so this computes context
    // independently per entry.
    let root_path = root.to_path_buf();
    let cache: Mutex<HashMap<PathBuf, Option<Gitignore>>> = Mutex::new(HashMap::new());
    builder.filter_entry(move |entry: &DirEntry| {
        let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
        // The root entry itself can never be ignored.
        if entry.depth() == 0 {
            return true;
        }
        if !is_dir && glob_drops(&glob_matcher, glob_negate, entry.path()) {
            return false;
        }
        !git_ignored_by_ancestors(&root_path, &cache, entry.path(), is_dir)
    });
}

/// True when any ancestor directory's `.gitignore` — the entry's own
/// directory (a directory entry) or its parent (a file entry), up to and
/// including the project root — ignores the entry. Uses the shared
/// `load_gitignore` + `gitignore_matches` helpers (R3: both walkers must
/// agree on gitignore semantics).
fn git_ignored_by_ancestors(
    root: &Path,
    cache: &Mutex<std::collections::HashMap<PathBuf, Option<ignore::gitignore::Gitignore>>>,
    path: &Path,
    is_dir: bool,
) -> bool {
    let mut dir = if is_dir {
        Some(path.to_path_buf())
    } else {
        path.parent().map(|p| p.to_path_buf())
    };
    let mut cache = cache.lock().unwrap();
    while let Some(d) = dir {
        let matcher = cache
            .entry(d.clone())
            .or_insert_with(|| load_gitignore(&d));
        if matcher.as_ref().is_some_and(|gi| gitignore_matches(gi, path, is_dir)) {
            return true;
        }
        if d == root {
            break; // the root's own `.gitignore` was the last check
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    false
}

/// Built-in type name → representative glob(s) for the `rg --type` filter
/// (the pinned `TypesBuilder` registers types by name + glob; redline maps
/// the common type names to their glob families, mirroring the grammar
/// registry's extension map). `None` for an unknown type name.
fn type_globs(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "rust" => &["*.rs"],
        "typescript" => &["*.ts"],
        "javascript" => &["*.js"],
        "python" => &["*.py"],
        "go" => &["*.go"],
        "c" => &["*.c", "*.h"],
        "cpp" => &["*.cpp", "*.cc", "*.hpp", "*.hh"],
        "toml" => &["*.toml"],
        "json" => &["*.json"],
        "yaml" => &["*.yml", "*.yaml"],
        "bash" => &["*.sh"],
        "markdown" => &["*.md"],
        _ => return None,
    })
}

/// The streaming sink: posts a `Hit` per matched line (the line-oriented
/// sink reports each matched line once, like `rg -n`), a `FileDone` when
/// the file finishes, and honors the cancel flag in every callback.
pub(crate) struct StreamSink {
    pub(crate) file: String,
    pub(crate) tx: mpsc::UnboundedSender<SearchEvent>,
    pub(crate) gen_id: usize,
    pub(crate) cancel: Arc<AtomicBool>,
    /// The literal pattern text when the search is fixed-string (used to
    /// compute the in-line column and to run the token-class check).
    pub(crate) literal: Option<String>,
    pub(crate) word: bool,
    /// Absolute byte ranges whose hits are dropped (token-class filter);
    /// `None` = no filtering.
    pub(crate) ranges: Option<Vec<Range<usize>>>,
    pub(crate) hits: u64,
}

impl StreamSink {
    fn emit_hit(&mut self, line_no: u64, col: Option<u64>, line: String) {
        self.hits += 1;
        let _ = self.tx.send(SearchEvent::Hit {
            file: self.file.clone(),
            line_no,
            col,
            line,
            generation: self.gen_id,
        });
    }
}

impl Sink for StreamSink {
    type Error = std::io::Error; // io::Error implements SinkError out of the box

    fn matched(&mut self, _s: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        if self.cancel.load(Ordering::Relaxed) {
            return Ok(false); // stops this file; finish() is still called
        }
        let line_no = mat.line_number().unwrap_or(0);
        // `bytes()` includes the line terminator — strip it (CRLF: both).
        let line = String::from_utf8_lossy(mat.bytes())
            .trim_end_matches(['\n', '\r'])
            .to_string();

        // Column + token-class check (both need the match's position
        // within the line; only a fixed-string search can determine it).
        if let Some(literal) = &self.literal {
            // Find the first occurrence that (a) honors word semantics
            // when `word` and (b) survives the token-class filter. The
            // line is dropped only when EVERY occurrence is inside a
            // comment/string (the token filter is active); otherwise the
            // first surviving occurrence gives the column.
            let mut from = 0;
            while let Some(rel) = line[from..].find(literal) {
                let idx = from + rel;
                if !self.word || is_word_boundary(&line, idx, literal.len()) {
                    let abs = mat.absolute_byte_offset() as usize + idx;
                    let in_token_range = self
                        .ranges
                        .iter()
                        .any(|r| r.iter().any(|r| r.contains(&abs)));
                    if !in_token_range {
                        self.emit_hit(line_no, Some(idx as u64), line);
                        return Ok(true);
                    }
                }
                // Advance past this occurrence (char-boundary safe).
                let mut next = idx + 1;
                while next < line.len() && !line.is_char_boundary(next) {
                    next += 1;
                }
                from = next;
            }
            if self.ranges.is_some() {
                return Ok(true); // every occurrence is in a comment/string → drop the line
            }
            // The line matched but no occurrence survived the word check
            // (can only diverge if the crate's `word()` is looser than
            // `is_word_boundary`): report it with no column.
            self.emit_hit(line_no, None, line);
            return Ok(true);
        }
        // Regex pattern: rg reports the line, not the in-line offset
        // (the `regex` crate's SearchOffset API is not exposed through
        // grep-searcher's sink), so the column is unknown.
        self.emit_hit(line_no, None, line);
        Ok(true)
    }

    fn finish(&mut self, _s: &Searcher, _f: &SinkFinish) -> Result<(), Self::Error> {
        let _ = self.tx.send(SearchEvent::FileDone {
            file: self.file.clone(),
            hits: self.hits,
            generation: self.gen_id,
        });
        Ok(())
    }
}

/// ripgrep `word` semantics at `[idx, idx+len)` in `line`: both sides
/// of the match must be a line edge or a non-word character (the pinned
/// grep-regex `word()` verified: `mytarget` and `target2` are rejected,
/// `target` and `-target-` are kept). Used by the streaming sink to
/// compute the in-line column for fixed-string searches.
fn is_word_boundary(line: &str, idx: usize, len: usize) -> bool {
    // C15: word-constituency is the crate-wide Unicode rule
    // (`crate::model::buffer::is_word_char`), not ASCII-only — a non-ASCII
    // identifier (e.g. `café`) must not be truncated by an ASCII boundary.
    use crate::model::buffer::is_word_char;
    let prev = line[..idx].chars().next_back();
    let next = line.get(idx + len..).and_then(|rest| rest.chars().next());
    !prev.is_some_and(is_word_char) && !next.is_some_and(is_word_char)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::AtomicBool;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        dir
    }

    fn start(_root: PathBuf, cfg: SearchConfig) -> (SearchBus, mpsc::UnboundedReceiver<SearchEvent>, Arc<AtomicBool>) {
        let cancel = cfg.cancel.clone();
        let (bus, rx) = SearchBus::new();
        spawn(cfg, &bus, 0);
        (bus, rx, cancel)
    }

    /// Drain all pending events (no timeout: the search is small).
    fn drain(rx: &mut mpsc::UnboundedReceiver<SearchEvent>) -> Vec<SearchEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            out.push(ev);
        }
        out
    }

    /// Wait until a `Finished` event arrives (bounded). The terminal
    /// event is included in the returned slice.
    fn drain_until_finished(
        rx: &mut mpsc::UnboundedReceiver<SearchEvent>,
        deadline: std::time::Duration,
    ) -> Vec<SearchEvent> {
        let start = std::time::Instant::now();
        let mut out = Vec::new();
        loop {
            for ev in drain(rx) {
                if matches!(ev, SearchEvent::Finished { .. }) {
                    out.push(ev);
                    return out;
                }
                out.push(ev);
            }
            if start.elapsed() > deadline {
                return out;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn hits(events: &[SearchEvent]) -> Vec<Hit> {
        events
            .iter()
            .filter_map(|ev| match ev {
                SearchEvent::Hit {
                    file,
                    line_no,
                    col,
                    line,
                    ..
                } => Some(Hit {
                    file: file.clone(),
                    line_no: *line_no,
                    col: *col,
                    line: line.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    fn base_cfg(root: PathBuf, pattern: &str) -> SearchConfig {
        SearchConfig {
            root,
            pattern: pattern.to_string(),
            word: false,
            fixed: false,
            case_smart: false,
            case_insensitive: false,
            glob: None,
            file_type: None,
            filter: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Counts match a hand-computed reference for the same query.
    #[test]
    fn pipeline_counts_match_reference_computation() {
        let dir = project();
        fs::write(
            dir.path().join("src/main.rs"),
            "fn alpha() {}\nfn main() { alpha(); alpha(); }\nlet alpha = 1;\n",
        )
        .unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}\npub fn beta() {}\n").unwrap();
        fs::write(dir.path().join("README.md"), "alpha docs\nalpha again\n").unwrap();

        // Hand computation for `alpha` (case-sensitive, no word):
        // src/main.rs lines 1, 2, 3 → 3; src/lib.rs line 1 → 1;
        // README.md lines 1, 2 → 2. Total 6.
        let (bus, mut rx, _cancel) = start(
            dir.path().to_path_buf(),
            base_cfg(dir.path().to_path_buf(), "alpha"),
        );
        let events = drain_until_finished(&mut rx, std::time::Duration::from_secs(5));
        let hits = hits(&events);
        assert_eq!(hits.len(), 6, "6 expected hits: {hits:?}");

        let by_file: std::collections::HashMap<&str, usize> = hits
            .iter()
            .fold(std::collections::HashMap::new(), |mut m, h| {
                *m.entry(h.file.as_str()).or_insert(0) += 1;
                m
            });
        assert_eq!(by_file.get("src/main.rs"), Some(&3));
        assert_eq!(by_file.get("src/lib.rs"), Some(&1));
        assert_eq!(by_file.get("README.md"), Some(&2));

        // Line numbers are exact (1-based) and line text is intact.
        let main_hits: Vec<&Hit> = hits.iter().filter(|h| h.file == "src/main.rs").collect();
        assert_eq!(
            main_hits.iter().map(|h| h.line_no).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(main_hits[0].line.contains("fn alpha"));
        // One Finished event, not cancelled.
        assert!(
            events.iter().any(|e| matches!(
                e,
                SearchEvent::Finished { cancelled: false, .. }
            )),
            "a Finished event must arrive: {events:?}"
        );
        drop(bus);
    }

    /// A `.gitignore`d file must NOT appear (marker-only, non-git
    /// project — the per-directory matcher path).
    #[test]
    fn pipeline_gitignored_file_excluded() {
        let dir = project();
        fs::write(dir.path().join(".gitignore"), "ignored.txt\nbuild/\n").unwrap();
        fs::write(dir.path().join("kept.txt"), "needle here\n").unwrap();
        fs::write(dir.path().join("ignored.txt"), "needle here\n").unwrap();
        fs::create_dir_all(dir.path().join("build")).unwrap();
        fs::write(dir.path().join("build/out.bin"), "needle here\n").unwrap();

        let (bus, mut rx, _cancel) = start(
            dir.path().to_path_buf(),
            base_cfg(dir.path().to_path_buf(), "needle"),
        );
        let events = drain_until_finished(&mut rx, std::time::Duration::from_secs(5));
        let hits = hits(&events);
        assert_eq!(hits.len(), 1, "only kept.txt: {hits:?}");
        assert_eq!(hits[0].file, "kept.txt");
        drop(bus);
    }

    /// A hidden directory must NOT be walked (hidden-skip default).
    #[test]
    fn pipeline_hidden_dir_excluded() {
        let dir = project();
        fs::create_dir_all(dir.path().join(".cache")).unwrap();
        fs::write(dir.path().join(".cache/x.txt"), "needle\n").unwrap();
        fs::write(dir.path().join("visible.txt"), "needle\n").unwrap();

        let (bus, mut rx, _cancel) = start(
            dir.path().to_path_buf(),
            base_cfg(dir.path().to_path_buf(), "needle"),
        );
        let events = drain_until_finished(&mut rx, std::time::Duration::from_secs(5));
        let hits = hits(&events);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].file, "visible.txt");
        drop(bus);
    }

    /// The glob filter (rg -g) excludes non-matching paths.
    #[test]
    fn pipeline_glob_filter_excludes() {
        let dir = project();
        fs::write(dir.path().join("src/one.rs"), "needle\n").unwrap();
        fs::write(dir.path().join("src/two.rs"), "needle\n").unwrap();
        fs::create_dir_all(dir.path().join("docs")).unwrap();
        fs::write(dir.path().join("docs/notes.md"), "needle\n").unwrap();

        let mut cfg = base_cfg(dir.path().to_path_buf(), "needle");
        cfg.glob = Some("*.rs".to_string());
        let (bus, mut rx, _cancel) = start(dir.path().to_path_buf(), cfg);
        let events = drain_until_finished(&mut rx, std::time::Duration::from_secs(5));
        let hits = hits(&events);
        let files: Vec<&str> = hits.iter().map(|h| h.file.as_str()).collect();
        assert_eq!(files.len(), 2, "{files:?}");
        assert!(files.iter().all(|f| f.ends_with(".rs")));
        drop(bus);
    }

    /// The type filter (rg --type) excludes other file types.
    #[test]
    fn pipeline_type_filter_excludes() {
        let dir = project();
        fs::write(dir.path().join("src/one.rs"), "needle\n").unwrap();
        fs::write(dir.path().join("src/two.py"), "needle\n").unwrap();

        let mut cfg = base_cfg(dir.path().to_path_buf(), "needle");
        cfg.file_type = Some("rust".to_string());
        let (bus, mut rx, _cancel) = start(dir.path().to_path_buf(), cfg);
        let events = drain_until_finished(&mut rx, std::time::Duration::from_secs(5));
        let hits = hits(&events);
        let files: Vec<&str> = hits.iter().map(|h| h.file.as_str()).collect();
        assert_eq!(files, vec!["src/one.rs"], "{files:?}");
        drop(bus);
    }

    /// Streaming: the first hit is observable BEFORE the Finished event
    /// (and before the walk completes) — event order guarantees it, and
    /// the partial state carries hits while still `running`.
    #[test]
    fn pipeline_first_hit_arrives_before_finish() {
        let dir = project();
        // Enough files that the walk cannot be instantaneous.
        for i in 0..50 {
            fs::write(dir.path().join(format!("f{i:03}.txt")), "needle\n").unwrap();
        }
        let (bus, mut rx, _cancel) = start(
            dir.path().to_path_buf(),
            base_cfg(dir.path().to_path_buf(), "needle"),
        );
        // Wait for the first Hit (bounded).
        let start = std::time::Instant::now();
        let mut first_hit: Option<Hit> = None;
        let mut finished_seen = false;
        loop {
            while let Ok(ev) = rx.try_recv() {
                if let SearchEvent::Hit {
                    file,
                    line_no,
                    col,
                    line,
                    ..
                } = ev
                {
                    first_hit = Some(Hit {
                        file,
                        line_no,
                        col,
                        line,
                    });
                    break;
                }
                if matches!(ev, SearchEvent::Finished { .. }) {
                    finished_seen = true;
                }
            }
            if first_hit.is_some() || finished_seen {
                break;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "no first hit within 5s"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let hit = first_hit.expect("a hit must stream in");
        assert_eq!(hit.line_no, 1);
        assert!(hit.line.contains("needle"));
        // Drain the rest; every Hit must precede the Finished event.
        let rest = drain_until_finished(&mut rx, std::time::Duration::from_secs(5));
        let mut last_hit_idx = None;
        for (i, ev) in rest.iter().enumerate() {
            match ev {
                SearchEvent::Hit { .. } => last_hit_idx = Some(i),
                SearchEvent::Finished { .. } => {
                    assert!(
                        last_hit_idx.is_none_or(|li| li < i),
                        "a hit must not arrive after Finished"
                    );
                }
                _ => {}
            }
        }
        drop(bus);
    }

    /// Cancellation: start a search over many files, cancel mid-flight,
    /// and assert a prompt stop (bounded wait) with a consistent state.
    #[test]
    fn pipeline_cancel_stops_promptly() {
        let dir = project();
        // A large tree of files so the walk takes real time.
        for i in 0..2000 {
            let d = dir.path().join(format!("d{i:03}"));
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join(format!("f{i:03}.txt")), "needle\n").unwrap();
        }
        let (bus, mut rx, cancel) = start(
            dir.path().to_path_buf(),
            base_cfg(dir.path().to_path_buf(), "needle"),
        );
        // Wait until the walk is streaming (first hit/file done), then
        // cancel mid-flight — a bare sleep can race the finish on a fast
        // machine.
        loop {
            match rx.try_recv() {
                Ok(ev @ SearchEvent::Hit { .. }) | Ok(ev @ SearchEvent::FileDone { .. }) => {
                    let _ = ev;
                    break;
                }
                Ok(ev) => panic!("unexpected event before the cancel: {ev:?}"),
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    panic!("the search channel disconnected before the cancel")
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(1)),
            }
        }
        cancel.store(true, Ordering::Relaxed);

        let start = std::time::Instant::now();
        let events = drain_until_finished(&mut rx, std::time::Duration::from_secs(2));
        let latency = start.elapsed();
        assert!(
            events.iter().any(|e| matches!(
                e,
                SearchEvent::Finished { cancelled: true, .. }
            )),
            "a cancelled Finished event must arrive; got {} events",
            events.len()
        );
        // Prompt stop: the worker observed the flag within the bound.
        assert!(
            latency < std::time::Duration::from_secs(2),
            "cancel latency {latency:?} exceeded the bound"
        );
        drop(bus);
    }

    /// Multibyte-safe column and line text (issue 03's lesson): the
    /// column is a BYTE offset and survives multibyte characters.
    #[test]
    fn pipeline_multibyte_col_and_line_text() {
        let dir = project();
        // "héllo " is 7 bytes (é is 2 bytes); "target" starts at byte 7.
        fs::write(dir.path().join("a.txt"), "héllo target\nplain target\n").unwrap();

        let mut cfg = base_cfg(dir.path().to_path_buf(), "target");
        cfg.fixed = true;
        let (bus, mut rx, _cancel) = start(dir.path().to_path_buf(), cfg);
        let events = drain_until_finished(&mut rx, std::time::Duration::from_secs(5));
        let hits = hits(&events);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert_eq!(hits[0].line_no, 1);
        assert_eq!(hits[0].col, Some(7), "byte column after multibyte prefix");
        assert_eq!(hits[0].line, "héllo target", "line text intact");
        assert_eq!(hits[1].col, Some(6), "`plain ` is 6 bytes");
        drop(bus);
    }

    /// Word-boundary semantics (fixed + word): `target` matches
    /// `target` and `-target-` but not `mytarget`.
    #[test]
    fn pipeline_word_boundaries_for_literals() {
        let dir = project();
        fs::write(
            dir.path().join("a.txt"),
            "target\nmytarget\n-target-\ntarget2\n",
        )
        .unwrap();
        let mut cfg = base_cfg(dir.path().to_path_buf(), "target");
        cfg.fixed = true;
        cfg.word = true;
        let (bus, mut rx, _cancel) = start(dir.path().to_path_buf(), cfg);
        let events = drain_until_finished(&mut rx, std::time::Duration::from_secs(5));
        let line_nos: Vec<u64> = hits(&events).iter().map(|h| h.line_no).collect();
        // `target` (line 1) and `-target-` (line 3): word() requires
        // BOTH sides to be a non-word char or the line edge; `mytarget`/
        // `target2` have word chars on one side → excluded.
        assert_eq!(line_nos, vec![1, 3], "word-boundary hits: {line_nos:?}");
        drop(bus);
    }

    /// C15: the sink's word-boundary rule is the crate-wide Unicode rule
    /// (`model::buffer::is_word_char`), not ASCII-only — `é` and CJK
    /// characters are word constituents, so they block a boundary.
    #[test]
    fn sink_word_boundary_is_unicode_aware() {
        // Full `café` at line edges: boundary.
        assert!(is_word_boundary("café", 0, 4), "line edges around café");
        // `caf` inside `café`: the following `é` is a WORD char → NOT a
        // boundary (the old ASCII rule would have let it through).
        assert!(!is_word_boundary("café", 0, 3), "`é` after `caf` blocks the boundary");
        // `é` at the tail of `café`: the preceding `f` is a word char.
        assert!(!is_word_boundary("café", 3, 1), "`f` before `é` blocks the boundary");
        // A genuine separator still gives a boundary.
        assert!(is_word_boundary("caf é", 0, 3), "space after `caf` is a boundary");
        // CJK: the same rule (Lo chars are alphanumeric).
        assert!(!is_word_boundary("漢字", 0, 3), "CJK `字` after `漢` blocks the boundary");
        assert!(is_word_boundary("漢 字", 0, 3), "space after CJK char is a boundary");
    }

    /// C15, end-to-end: a word search for the ASCII prefix `caf` must NOT
    /// match inside `café` (the `é` is a word char, not a boundary); the
    /// same literal still matches where both sides are real boundaries.
    #[test]
    fn pipeline_word_boundaries_multibyte() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "café\nmycafé\ncaf é\n").unwrap();
        let mut cfg = base_cfg(dir.path().to_path_buf(), "caf");
        cfg.fixed = true;
        cfg.word = true;
        let (bus, mut rx, _cancel) = start(dir.path().to_path_buf(), cfg);
        let events = drain_until_finished(&mut rx, std::time::Duration::from_secs(5));
        let line_nos: Vec<u64> = hits(&events).iter().map(|h| h.line_no).collect();
        // Line 1 `café`: `caf` is followed by the word char `é` → excluded.
        // Line 2 `mycafé`: word char on the left → excluded.
        // Line 3 `caf é`: real boundaries on both sides → hit.
        assert_eq!(line_nos, vec![3], "multibyte word-boundary hits: {line_nos:?}");
        drop(bus);
    }

    /// Without `word`, the same literal matches inside longer words.
    #[test]
    fn pipeline_no_word_matches_substrings() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "target\nmytarget\n").unwrap();
        let mut cfg = base_cfg(dir.path().to_path_buf(), "target");
        cfg.fixed = true;
        let (bus, mut rx, _cancel) = start(dir.path().to_path_buf(), cfg);
        let events = drain_until_finished(&mut rx, std::time::Duration::from_secs(5));
        let line_nos: Vec<u64> = hits(&events).iter().map(|h| h.line_no).collect();
        assert_eq!(line_nos, vec![1, 2], "substring matches: {line_nos:?}");
        drop(bus);
    }

    /// Smart case: an all-lowercase pattern matches case-insensitively;
    /// an uppercase pattern is case-sensitive.
    #[test]
    fn pipeline_smart_case() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "Hello world\nHELLO again\n").unwrap();

        let mut cfg = base_cfg(dir.path().to_path_buf(), "hello");
        cfg.case_smart = true;
        let (bus, mut rx, _cancel) = start(dir.path().to_path_buf(), cfg);
        let events = drain_until_finished(&mut rx, std::time::Duration::from_secs(5));
        assert_eq!(hits(&events).len(), 2, "smart case: both cases match");
        drop(bus);

        let mut cfg = base_cfg(dir.path().to_path_buf(), "Hello");
        cfg.case_smart = true;
        let (bus, mut rx, _cancel) = start(dir.path().to_path_buf(), cfg);
        let events = drain_until_finished(&mut rx, std::time::Duration::from_secs(5));
        let line_nos: Vec<u64> = hits(&events).iter().map(|h| h.line_no).collect();
        assert_eq!(line_nos, vec![1], "uppercase literal: case-sensitive");
        drop(bus);
    }

    /// An invalid regex surfaces as an Error event (no Finished).
    #[test]
    fn pipeline_invalid_regex_reports_error() {
        let dir = project();
        let (bus, mut rx, _cancel) = start(
            dir.path().to_path_buf(),
            base_cfg(dir.path().to_path_buf(), "([unclosed"),
        );
        let events = drain_until_finished(&mut rx, std::time::Duration::from_secs(2));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, SearchEvent::Error { .. })),
            "an Error event must arrive: {events:?}"
        );
        drop(bus);
    }

    /// Item 2 (C14): pin the user-visible asymmetry — a file under
    /// `graft/` IS found by the search pipeline (the search pipeline does
    /// not prune `graft/`), while the file-list walk does prune it
    /// (`files.rs::walk_prunes_graft_cache_directory` pins the other side).
    /// Same fixture layout as that test: `src/main.rs` + `graft/` cards.
    #[test]
    fn search_finds_graft_files_unlike_file_list() {
        let dir = project();
        fs::write(dir.path().join("src/main.rs"), "needle\n").unwrap();
        fs::create_dir_all(dir.path().join("graft/src")).unwrap();
        fs::write(dir.path().join("graft/src/main.md"), "needle\n").unwrap();
        fs::create_dir_all(dir.path().join("graft/cache")).unwrap();
        fs::write(dir.path().join("graft/cache/other.md"), "needle\n").unwrap();

        let (bus, mut rx, _cancel) = start(
            dir.path().to_path_buf(),
            base_cfg(dir.path().to_path_buf(), "needle"),
        );
        let events = drain_until_finished(&mut rx, std::time::Duration::from_secs(5));
        let hits = hits(&events);
        // The search pipeline finds ALL three files (including graft/).
        assert_eq!(hits.len(), 3, "search must find graft/ files: {hits:?}");
        let files: Vec<&str> = hits.iter().map(|h| h.file.as_str()).collect();
        assert!(files.contains(&"src/main.rs"));
        assert!(
            files.contains(&"graft/src/main.md"),
            "graft/ content must be found by search (the documented asymmetry vs FileList)"
        );
        assert!(files.contains(&"graft/cache/other.md"));
        drop(bus);
    }

}

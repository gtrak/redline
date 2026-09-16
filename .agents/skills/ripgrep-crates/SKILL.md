---
name: ripgrep-crates
description: >-
  Reference for the embedded-ripgrep crates pinned in redline's Cargo.toml —
  ignore 0.4.33 (gitignore-aware Walk/WalkParallel directory walking),
  grep-regex 0.1.14 (RegexMatcher/RegexMatcherBuilder with smart-case and
  word-boundary options), and grep-searcher 0.1.17 (Searcher/SearcherBuilder
  plus the push-model Sink trait with verbatim signatures). Scoped to the
  streaming, cancelable search pipeline redline needs for issue 06
  (project search, M-? references, per-file occur) — not the full ripgrep CLI
  feature matrix. All signatures verified against the vendored crate sources
  in ~/.cargo/registry (ignore-0.4.33, grep-searcher-0.1.17, grep-regex-0.1.14,
  grep-matcher-0.1.9).
---

# ripgrep-crates

Embedded ripgrep for redline's search pipeline (plan issue 06): walk the
project → match a regex → collect `(path, line_no, line, context)` → stream
hits to the results view. Pinned in `Cargo.toml`: `ignore = "0.4.33"`,
`grep-searcher = "0.1.17"`, `grep-regex = "0.1.14"`.

Ground truth: vendored sources under
`~/.cargo/registry/src/index.crates.io-*/{ignore-0.4.33,grep-searcher-0.1.17,grep-regex-0.1.14,grep-matcher-0.1.9}/src`.
`grep-matcher` 0.1.9 is a **transitive** dependency (required by both
grep-searcher and grep-regex); it is not in redline's `Cargo.toml` and redline
code should not name it — see Matching below.

## Walking (ignore)

`WalkBuilder` (ignore 0.4.33, `src/walk.rs`):

```rust
pub fn new<P: AsRef<Path>>(path: P) -> WalkBuilder
pub fn add<P: AsRef<Path>>(&mut self, path: P) -> &mut WalkBuilder   // extra roots
pub fn build(&self) -> Walk            // sequential iterator
pub fn build_parallel(&self) -> WalkParallel  // NOT an Iterator — run with a closure
```

Toggles redline needs (all return `&mut WalkBuilder`; all ignore filters are
**enabled by default** — `standard_filters(yes)` toggles the group):

```rust
pub fn hidden(&mut self, yes: bool) -> &mut WalkBuilder        // default: on
pub fn parents(&mut self, yes: bool) -> &mut WalkBuilder      // .gitignore in parent dirs; default: on
pub fn git_ignore(&mut self, yes: bool) -> &mut WalkBuilder   // .gitignore; default: on
pub fn git_global(&mut self, yes: bool) -> &mut WalkBuilder   // global gitignore; default: on
pub fn git_exclude(&mut self, yes: bool) -> &mut WalkBuilder  // .git/info/exclude; default: on
pub fn ignore(&mut self, yes: bool) -> &mut WalkBuilder       // .ignore files; default: on
pub fn require_git(&mut self, yes: bool) -> &mut WalkBuilder  // default: true (git rules only inside a git repo)
pub fn threads(&mut self, n: usize) -> &mut WalkBuilder       // 0 = auto; only affects build_parallel
pub fn max_depth(&mut self, depth: Option<usize>) -> &mut WalkBuilder
pub fn follow_links(&mut self, yes: bool) -> &mut WalkBuilder // default: off
pub fn max_filesize(&mut self, filesize: Option<u64>) -> &mut WalkBuilder
pub fn types(&mut self, types: Types) -> &mut WalkBuilder     // file-type filters (rg --type)
pub fn overrides(&mut self, overrides: Override) -> &mut WalkBuilder  // glob filters (rg -g)
```

`filter_entry` — the entry predicate:

```rust
pub fn filter_entry<P>(&mut self, filter: P) -> &mut WalkBuilder
where
    P: Fn(&DirEntry) -> bool + Send + Sync + 'static,
```

It "yields only entries which satisfy the given predicate and skips descending
into directories that do not satisfy the given predicate." Applied to **all**
entries (files and dirs); a `false` dir is not descended into. Only one filter
may be set — later calls override. (Use this for redline's path/glob
restrictions; `types`/`overrides` for `--type`-style filters, built via
`TypesBuilder` / `OverrideBuilder`.)

**`TypesBuilder` requires an explicit `select`** (verified 0.4.33 by probe):
`TypesBuilder::new()` starts with NO type selected, and a `types(...)` filter
with zero selected types is a NO-OP (every file passes). Standard types come
from `add_defaults()`; then `select(name)` activates one (or more). A custom
type is registered with `add(name, glob)` (2-arg; returns `Result`) and ALSO
needs `select(name)`. `select` itself returns `&mut Self` (no per-call
error); selecting an unknown name fails at `build()` with
`UnrecognizedFileType`. Other methods: `negate`, `clear`, `add_def`,
`definitions()` (takes `&mut self`, returns the registered defs).

`Walk` (sequential) is a plain `Iterator<Item = Result<DirEntry, Error>>` —
cancellation = stop iterating (early `return`/`break` drops the walk).

`WalkParallel` (parallel; per-thread closures):

```rust
pub fn run<'s, F>(self, mkf: F)
where
    F: FnMut() -> FnVisitor<'s>,
// FnVisitor<'s> = Box<dyn FnMut(Result<DirEntry, Error>) -> WalkState + Send + 's>  (private alias in src)
```

`mkf` runs **once per worker thread**; the closure it returns is called for
each visited entry on that thread. Doc-commented usage shape:
`builder.build_parallel().run(|| |path| { /* … */ WalkState::Continue })`.

```rust
pub enum WalkState {
    Continue, // keep walking
    Skip,     // don't descend into this directory (no effect on files)
    Quit,     // "quit the entire iterator as soon as possible" — documented as
              // inherently asynchronous: more entries may still be yielded
}
```

`DirEntry` accessors: `path() -> &Path`, `into_path() -> PathBuf`,
`file_type() -> Option<std::fs::FileType>` (None only for the stdin pseudo-entry),
`file_name() -> &OsStr`, `depth() -> usize`, `metadata() -> Result<Metadata, Error>`,
`path_is_symlink() -> bool`, `error() -> Option<&Error>` (e.g. ignore-file parse
errors; traversal errors arrive as `Err` items instead).

**Recommendation for redline's streaming search: `WalkParallel` + `run`.**
Reasons, all verified: (1) parallel directory traversal with per-thread
closures is exactly the shape of "walk project → search each file" — each
worker thread builds its own `Searcher` (see Searching) and calls
`search_path` on the entries it visits; (2) the only in-walk stop mechanism is
`WalkState::Quit`, which is returned *from* the per-entry closure — the
sequential `Walk` has no equivalent (you'd have to interleave cancellation
checks between `next()` calls in the UI-facing thread, serializing the walk);
(3) `threads(0)` auto-sizes with a verified cap:
`std::thread::available_parallelism().map_or(1, |n| n.get()).min(12)` — at most
12 threads. Sequential `Walk` is fine for tests only.

## Matching (grep-regex)

```rust
pub struct RegexMatcherBuilder { /* private */ }

impl RegexMatcherBuilder {
    pub fn new() -> RegexMatcherBuilder;
    pub fn build(&self, pattern: &str) -> Result<RegexMatcher, Error>; // Error = grep_regex::Error

    // the options redline's plan needs (all verified, all return &mut Self):
    pub fn case_insensitive(&mut self, yes: bool) -> &mut RegexMatcherBuilder;
    pub fn case_smart(&mut self, yes: bool) -> &mut RegexMatcherBuilder;
    pub fn word(&mut self, yes: bool) -> &mut RegexMatcherBuilder;
    pub fn fixed_strings(&mut self, yes: bool) -> &mut RegexMatcherBuilder;
    pub fn line_terminator(&mut self, line_term: Option<LineTerminator>) -> &mut RegexMatcherBuilder;
}

pub struct RegexMatcher { /* private */ }
impl RegexMatcher {
    pub fn new(pattern: &str) -> Result<RegexMatcher, Error>;
    pub fn new_line_matcher(pattern: &str) -> Result<RegexMatcher, Error>; // = new() + line_terminator(b'\n')
}
```

Semantics (verified doc comments, `src/matcher.rs`):

- `case_smart(true)` — "smart case": case-insensitive matching is enabled
  automatically when (1) the pattern contains at least one literal character
  and (2) none of its literals are uppercase. This is the `C-c p s s`
  (smart-case search) behavior.
- `word(true)` — "require that all matches occur on word boundaries."
  Verified against the pinned 0.1.14 (probe: pattern `target` with `word`
  matches `target` and `-target-` but NOT `mytarget`/`target2`): for
  word-char pattern edges, BOTH sides of the match must be a non-word char
  or a line edge (like `\b…\b`). The difference from a literal `\b` wrap
  shows up for patterns with non-word-char edges: `-2` with `word` matches
  in `foo -2 bar`, while `\b-2\b` does not (no word/non-word *transition*
  around `-`). This is what `M-?` references need.
- `fixed_strings(true)` — all characters match literally (use for identifier
  reference search so a pattern like `foo.bar` isn't a regex).
- `line_terminator(Some(b'\n'))` enables line-oriented optimizations; if set,
  it **must equal** the searcher's line terminator or `check_config` fails
  with `ConfigError::MismatchedLineTerminators`.

**The `Matcher` trait the searcher consumes lives in the `grep-matcher` crate
(0.1.9), not in grep-regex or grep-searcher** — neither re-exports it.
`RegexMatcher` implements `grep_matcher::Matcher`
(`type Captures = RegexCaptures; type Error = NoError`); the trait's only two
required methods are `find_at` and `new_captures`. Redline never names the
trait: the search methods are generic (`M: Matcher`) and `&RegexMatcher` is
passed by value into them (there is `impl<'a, M: Matcher> Matcher for &'a M`),
so no `grep-matcher` dependency is needed in `Cargo.toml`.

## Searching (grep-searcher)

`SearcherBuilder` (grep-searcher 0.1.17, `src/searcher/mod.rs`):

```rust
pub fn new() -> SearcherBuilder;
pub fn build(&self) -> Searcher;   // takes &self → one builder can build per-thread Searchers

// options redline's plan needs (verified; all return &mut SearcherBuilder):
pub fn line_number(&mut self, yes: bool) -> &mut SearcherBuilder;      // default: ON
pub fn before_context(&mut self, line_count: usize) -> &mut SearcherBuilder; // default: 0
pub fn after_context(&mut self, line_count: usize) -> &mut SearcherBuilder;  // default: 0
pub fn multi_line(&mut self, yes: bool) -> &mut SearcherBuilder;       // default: off
pub fn binary_detection(&mut self, detection: BinaryDetection) -> &mut SearcherBuilder; // default: none()
pub fn max_matches(&mut self, limit: Option<u64>) -> &mut SearcherBuilder; // cap; 0 = quit immediately
pub fn line_terminator(&mut self, line_term: LineTerminator) -> &mut SearcherBuilder; // default: b'\n'
pub fn encoding(&mut self, encoding: Option<Encoding>) -> &mut SearcherBuilder;
pub fn bom_sniffing(&mut self, yes: bool) -> &mut SearcherBuilder;     // default: on
```

`Searcher` search entry points (all verified; each consumes the matcher and
sink **by value**, and takes `&mut self`):

```rust
pub fn search_path<P, M, S>(&mut self, matcher: M, path: P, write_to: S) -> Result<(), S::Error>
where P: AsRef<Path>, M: Matcher, S: Sink;

pub fn search_file<M, S>(&mut self, matcher: M, file: &File, write_to: S) -> Result<(), S::Error>
where M: Matcher, S: Sink;

pub fn search_slice<M, S>(&mut self, matcher: M, slice: &[u8], write_to: S) -> Result<(), S::Error>
where M: Matcher, S: Sink;   // for per-buffer occur (M-s o)

pub fn search_reader<M, R, S>(&mut self, matcher: M, read_from: R, write_to: S) -> Result<(), S::Error>
where M: Matcher, R: io::Read, S: Sink;
```

`BinaryDetection` (verified): `BinaryDetection::none()` (default — **no**
binary detection, unlike the rg CLI), `BinaryDetection::quit(binary_byte: u8)`
(heuristic binary detection; when triggered, "the search stops as if it
reached EOF" and `Sink::binary_data` is called at least once with the absolute
byte offset where binary data began), `BinaryDetection::convert(u8)`.

### Sink trait — verbatim from `src/sink.rs`

The searcher is a **push** model: it drives execution and calls your sink.

```rust
pub trait Sink {
    /// The type of an error that should be reported by a searcher.
    type Error: SinkError;

    /// REQUIRED. Called whenever a match is found.
    /// If multi line is enabled on the searcher, then the match reported here
    /// may span multiple lines and it may include multiple matches. When multi
    /// line is disabled, then the match is guaranteed to span exactly one
    /// non-empty line (where a single line is, at minimum, a line terminator).
    /// If this returns `true`, then searching continues. If this returns
    /// `false`, then searching is stopped immediately and `finish` is called.
    /// If this returns an error, then searching is stopped immediately,
    /// `finish` is not called and the error is bubbled back up to the caller.
    fn matched(&mut self, _searcher: &Searcher, _mat: &SinkMatch<'_>) -> Result<bool, Self::Error>;

    /// PROVIDED (default: does nothing, returns Ok(true)).
    /// Called whenever a context line is found.
    fn context(&mut self, _searcher: &Searcher, _context: &SinkContext<'_>) -> Result<bool, Self::Error> { Ok(true) }

    /// PROVIDED (default: Ok(true)).
    /// Called whenever a break in contextual lines is found (only when
    /// before_context/after_context > 0); occurs between non-contiguous groups.
    fn context_break(&mut self, _searcher: &Searcher) -> Result<bool, Self::Error> { Ok(true) }

    /// PROVIDED (default: Ok(true)).
    /// Called when binary detection is enabled and binary data is found,
    /// with the absolute byte offset at which the binary data begins.
    fn binary_data(&mut self, _searcher: &Searcher, _binary_byte_offset: u64) -> Result<bool, Self::Error> { Ok(true) }

    /// PROVIDED (default: Ok(true)). Called when a search has begun, before any search is executed.
    fn begin(&mut self, _searcher: &Searcher) -> Result<bool, Self::Error> { Ok(true) }

    /// PROVIDED (default: Ok(())). Called when a search has completed.
    /// (Called even when the sink itself stopped the search via Ok(false);
    /// NOT called when the search stopped via an error.)
    fn finish(&mut self, _searcher: &Searcher, _: &SinkFinish) -> Result<(), Self::Error> { Ok(()) }
}
```

Error type: `SinkError` requires `fn error_message<T: Display>(message: T) -> Self`
(+ provided `error_io(io::Error)` / `error_config(ConfigError)`). `std::io::Error`
and `Box<dyn std::error::Error>` implement it out of the box — use
`type Error = std::io::Error`.

`SinkMatch<'b>` accessors (verified): `bytes() -> &'b [u8]` (**includes the line
terminator** — strip `\n`/`\r` before display), `lines() -> LineIter<'b>`,
`absolute_byte_offset() -> u64`, `line_number() -> Option<u64>` (only when the
searcher counts lines), `buffer() -> &'b [u8]`, `bytes_range_in_buffer() -> Range<usize>`.

`SinkContext<'b>` accessors (verified): `bytes() -> &'b [u8]` (includes the line
terminator), `kind() -> &SinkContextKind` (`Before` / `After` / `Other`),
`absolute_byte_offset() -> u64`, `line_number() -> Option<u64>`.
(`SinkContext::lines()` exists but is `#[cfg(test)]` — not usable.)

`SinkFinish` accessors: `byte_count() -> u64`, `binary_byte_offset() -> Option<u64>`.

Convenience sinks exist (`sinks::UTF8`, `sinks::Lossy`, `sinks::Bytes` —
closure sinks of shape `FnMut(u64, &str) -> Result<bool, io::Error>`) but they
ignore context lines and error on invalid UTF-8 (UTF8) — redline needs a
custom `Sink` because it must receive `SinkContext` and the file path (the
searcher does **not** pass the path to the sink; capture it yourself).

## Reference pipeline

Assembled only from the verified APIs above. One worker thread; hits are
posted to a `std::sync::mpsc` channel as they are found (the UI task consumes
the `Receiver` and renders incrementally).

```rust
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

use grep_regex::RegexMatcherBuilder;
use grep_searcher::{
    BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkContextKind,
    SinkError, SinkFinish, SinkMatch,
};
use ignore::{DirEntry, WalkBuilder, WalkState};

pub struct Hit {
    pub path: PathBuf,
    pub line_no: u64,          // 1-based
    pub line: String,          // matched line, terminator stripped
    pub before: Vec<String>,   // before_context lines (terminators stripped)
    pub after: Vec<String>,    // after_context lines (terminators stripped)
}

pub enum SearchEvent {
    Hit(Hit),
    FileDone { path: PathBuf, total: u64 },
    Finished,
    Error(String),
}

struct StreamSink {
    path: PathBuf,
    tx: mpsc::Sender<SearchEvent>,
    cancel: Arc<AtomicBool>,
    hits: u64,
    pending_before: Vec<String>,
    current: Option<Hit>,      // open group: match emitted once its after-context is complete
}

impl StreamSink {
    fn flush(&mut self) {
        if let Some(hit) = self.current.take() {
            self.hits += 1;
            let _ = self.tx.send(SearchEvent::Hit(hit));
        }
    }
}

impl Sink for StreamSink {
    type Error = io::Error; // io::Error implements SinkError out of the box

    fn matched(&mut self, _s: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        if self.cancel.load(Ordering::Relaxed) {
            return Ok(false); // verified: search stops immediately; finish() IS still called
        }
        self.flush();
        let line_no = mat
            .line_number()
            .ok_or_else(|| io::Error::other("line numbers disabled"))?;
        let line = String::from_utf8_lossy(mat.bytes())
            .trim_end_matches(['\n', '\r'])
            .to_string();
        self.current = Some(Hit {
            path: self.path.clone(),
            line_no,
            line,
            before: std::mem::take(&mut self.pending_before),
            after: Vec::new(),
        });
        Ok(true)
    }

    fn context(&mut self, _s: &Searcher, ctx: &SinkContext<'_>) -> Result<bool, Self::Error> {
        if self.cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let line = String::from_utf8_lossy(ctx.bytes())
            .trim_end_matches(['\n', '\r'])
            .to_string();
        match ctx.kind() {
            SinkContextKind::Before => self.pending_before.push(line),
            // After/Other context arrives AFTER matched(); attach to the open group.
            SinkContextKind::After | SinkContextKind::Other => {
                if let Some(hit) = &mut self.current {
                    hit.after.push(line);
                }
            }
        }
        Ok(true)
    }

    // Group boundary: between non-contiguous context groups.
    fn context_break(&mut self, _s: &Searcher) -> Result<bool, Self::Error> {
        self.flush();
        Ok(true)
    }

    fn finish(&mut self, _s: &Searcher, _f: &SinkFinish) -> Result<(), Self::Error> {
        self.flush();
        let _ = self.tx.send(SearchEvent::FileDone {
            path: self.path.clone(),
            total: self.hits,
        });
        Ok(())
    }
}

/// Spawn the streaming, cancelable project-wide search (smart-case; word
/// boundaries when `word` is set — the M-? references case).
pub fn spawn_search(
    root: PathBuf,
    pattern: String,
    word: bool,
    cancel: Arc<AtomicBool>,
) -> mpsc::Receiver<SearchEvent> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        // Verified grep-regex builder API.
        let matcher = match RegexMatcherBuilder::new()
            .case_smart(true)
            .word(word)
            .build(&pattern)
        {
            Ok(m) => m,
            Err(e) => {
                let _ = tx.send(SearchEvent::Error(e.to_string()));
                return;
            }
        };

        // One builder; each worker thread builds its own Searcher
        // (search_* take &mut self, so a Searcher is per-thread).
        let builder = SearcherBuilder::new()
            .line_number(true)
            .before_context(1)
            .after_context(1)
            .binary_detection(BinaryDetection::quit(b'\0')); // rg-CLI-like NUL heuristic

        let tx = Arc::new(tx);
        let walker = WalkBuilder::new(&root)
            .threads(0) // auto-sized, capped at 12 (verified)
            .build_parallel();

        walker.run(move || {
            let mut searcher = builder.build();
            let tx = tx.clone();
            let cancel = cancel.clone();
            move |entry: Result<DirEntry, ignore::Error>| -> WalkState {
                if cancel.load(Ordering::Relaxed) {
                    return WalkState::Quit; // verified: quits "as soon as possible" (async)
                }
                let Ok(entry) = entry else { return WalkState::Continue };
                let Some(ft) = entry.file_type() else { return WalkState::Continue };
                if !ft.is_file() {
                    return WalkState::Continue;
                }
                let mut sink = StreamSink {
                    path: entry.path().to_path_buf(),
                    tx: tx.clone(),
                    cancel: cancel.clone(),
                    hits: 0,
                    pending_before: Vec::new(),
                    current: None,
                };
                // &matcher: `impl<'a, M: Matcher> Matcher for &'a M` (verified).
                // &mut sink: `impl<'a, S: Sink> Sink for &'a mut S` (verified).
                let _ = searcher.search_path(&matcher, entry.path(), &mut sink);
                WalkState::Continue
            }
        });

        let _ = tx.send(SearchEvent::Finished);
    });
    rx
}
```

**Where the UI posts incremental updates:** the sink's `tx.send(...)` calls —
every `matched`/`context_break`/`finish` on every worker thread pushes into the
shared `mpsc` channel; the UI task (tokio) polls the `Receiver` and updates the
results view (running counts from `FileDone`, hits from `Hit`). Because the
searcher pushes results as it finds them, first hits appear without waiting
for the walk to finish.

**Where cancellation interjects (and its limits — verified, no more exists):**
neither crate has an interrupt flag, cancellation token, or any async
primitive (grep-searcher source contains no "cancel"/"interrupt" at all).
The only stop points are:
1. `Sink::matched`/`context` returning `Ok(false)` — stops the search of the
   **current file** immediately (`finish` is still called);
2. `WalkState::Quit` from the per-entry closure — stops the **walk** "as soon
   as possible", documented as inherently asynchronous (in-flight files on
   other threads may still yield a few entries);
3. dropping a sequential `Walk` (early return from the `for` loop).

So `ESC` in redline = set the `Arc<AtomicBool>`; the next sink call and the
next visited entry observe it. A file already being searched finishes (or
stops at its next sink callback); already-sent channel messages remain and
the UI should discard them when it sees the cancel.

## Usage in redline

Maps to plan issue 06 (`docs/plans/001-redline-code-browser/06-search-and-references.md`;
these files are planned, not yet created — `src/` currently holds only
`main.rs`):

| Planned file | Pipeline piece |
|---|---|
| `src/search/rg.rs` | The reference pipeline above: `WalkParallel` + per-thread `Searcher` + streaming `Sink` → channel. `C-c p s s` (smart case: `case_smart(true)`), pattern/path-glob/type filters via `filter_entry`/`overrides`/`types`, cancel via the shared flag. |
| `src/search/references.rs` | Same pipeline with a `fixed_strings(true) + word(true) + case_smart(true)` matcher built from the identifier under point. **The ripgrep layer supplies candidates only** — comment/string hits are dropped *after* matching, via tree-sitter token-class filtering (per plan: "word-boundary search filtered by tree-sitter token class where a grammar exists; plain search fallback elsewhere"). |
| `src/search/occur.rs` | Per-buffer occur (`M-s o`): no walk at all — `Searcher::search_slice(&matcher, bytes, sink)` over the current ropey buffer's bytes (verified signature). |
| `src/ui/results_view.rs` | Consumes the `Receiver<SearchEvent>`: grouped by path, running counts (`FileDone`), live updates, jumpable hits. |

## Gotchas

All items below verified in the pinned sources (no web docs used):

- **Binary detection is OFF by default** (`BinaryDetection::none()`), unlike
  the ripgrep CLI. Without `binary_detection(quit(b'\0'))`, binary files are
  searched as text (and a long binary line can balloon the line buffer up to
  the heap limit). `binary_data`/`SinkFinish::binary_byte_offset` fire only
  when detection is enabled; with `quit`, the search stops as if EOF.
- **`SinkMatch::bytes()` / `SinkContext::bytes()` include the line
  terminator** — strip `\n`/`\r` (CRLF: strip both) before display/jump.
- **Line numbers are on by default** (`line_number` default true); if you
  disable them, `SinkMatch::line_number()` returns `None` and the convenience
  sinks (`UTF8`/`Lossy`/`Bytes`) error on the first match ("line numbers not
  enabled").
- **Encoding**: no transcoding by default; BOM sniffing is on by default
  (UTF-16 files are searched seamlessly); without a BOM, bytes are searched
  "as if" UTF-8 and invalid UTF-8 passes through — a custom sink must
  `String::from_utf8_lossy` (that's what `sinks::Lossy` does).
- **Line-terminator mismatch is a config error**: if the matcher's
  `line_terminator` is set and differs from the searcher's, `search_*` fails
  with `ConfigError::MismatchedLineTerminators`. Keep both at the default
  `b'\n'`, or set both.
- **`multi_line(true)` needs the whole file in memory** (mmap or heap read of
  the entire contents) — don't enable it for the streaming pipeline.
- **WalkParallel thread count**: `threads(0)` (default) =
  `available_parallelism` capped at **12**; `threads()` only affects
  `build_parallel`, not sequential `Walk`.
- **`filter_entry` takes only one predicate** — later calls override earlier
  ones; compose all entry rules into a single closure.
- **`WalkState::Quit` is async**: "it is possible for more entries to be
  yielded even after instructing the iterator to quit" — expect a handful of
  stray hits after cancel; the UI must treat post-cancel channel messages as
  garbage.
- **The sink never receives the file path** — `search_path` passes only the
  path to the searcher, not to the sink; capture it per file (as the
  reference pipeline does).
- **`search_*` consume matcher and sink by value and take `&mut self`** —
  pass `&matcher` (shared `&RegexMatcher` across threads) and `&mut sink`
  (both have the blanket impls), and give each worker thread its own
  `Searcher` built from a shared `SearcherBuilder` (`build(&self)`).

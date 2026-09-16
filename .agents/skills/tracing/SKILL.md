---
name: tracing
description: In-repo reference for the `tracing` 0.1.44 + `tracing-subscriber` 0.3.23 (fmt + env-filter) event-logging setup redline uses to write structured diagnostics to a FILE (stdout is owned by the TUI). Covers the verified event/span macros, the fmt + EnvFilter + registry subscriber init chain, the file-writer pattern, and RUST_LOG directive filtering. All facts verified against the vendored sources at ~/.cargo/registry/src/.../tracing-0.1.44/ and tracing-subscriber-0.3.23/; use this file, not docs.rs, for these topics.
---

# tracing

redline is a TUI: **stdout is the UI**. All diagnostics go to a log file via
`tracing` + `tracing-subscriber`'s `fmt` layer. This skill documents only what
redline needs, verified against the pinned vendored sources.

## Version & features

Pinned in `Cargo.toml`:

- `tracing = "0.1.44"`
- `tracing-subscriber = { version = "0.3.23", features = ["fmt", "env-filter"] }`

What the features unlock:

- `fmt` — the `fmt()` subscriber builder / `fmt::layer()`, the text formatters
  (`format::Full` default, `Compact`, `Pretty`), and the `MakeWriter` machinery.
- `env-filter` — `EnvFilter` and `SubscriberBuilder::with_env_filter`.
- `tracing-subscriber`'s *default* features also include `tracing-log` and
  `ansi` (both stay on here). Consequences:
  - `init()` installs the `LogTracer`, so `log`-crate records from deps
    (e.g. `notify` emits `log::trace!`/`log::warn!` in `poll.rs`) are converted
    into tracing events and land in the file.
  - ANSI color is **on by default** unless `NO_COLOR` is set — you must call
    `.with_ansi(false)` for file output (see Gotchas).
- `tracing`'s own default features are `std` + `attributes`; the `log` feature
  is **not** enabled, so disabled events skip their field evaluation entirely.

## Emitting events

Level macros: `trace!`, `debug!`, `info!`, `warn!`, `error!`.

```rust
use tracing::{info, info_span};

info!("opened repo", path = "/home/u/proj");          // field = value
info!("indexed {} files", n);                        // message only
info!(?config, "config loaded");                     // Debug-formatted field
info!(%path, "reading");                             // Display-formatted field
info!(target: "git", "commit created");              // explicit target
```

Field syntax (verified in `macros.rs` + `tracing-core/field.rs`):

- `field = value` — `value` must implement `tracing::field::Value` (the
  primitives `i64`/`u64`/`f64`/`bool`/`&str`, or a wrapped value below).
- `?expr` — record via the value's `fmt::Debug` (`field::debug(&expr)`).
- `%expr` — record via the value's `fmt::Display` (`field::display(&expr)`).
- Bare `field` (e.g. `info!(n)`) — shorthand that records the local variable
  `n` itself.
- `Option<T>` implements `Value`: a `None` value records **nothing** (the field
  is simply omitted), so `info!(?maybe)` is safe on optional data.
- A trailing format string after the fields becomes an implicit `message` field.

Spans:

```rust
use tracing::{info_span, Instrument};

let span = info_span!("index_repo", ?path);
{
    let _entered = span.enter();      // guard; exits on drop
    info!("inside the span");
}

// owned guard, created and entered in one step:
let _entered = info_span!("git_status").entered();

// async: enter the span on every poll/drop of the future
some_future.instrument(info_span!("watch")).await;
```

- `Span::enter()` → `Entered<'_>` (borrows the span); `Span::entered()` →
  `EnteredSpan` (owned). `EnteredSpan` is **`!Send`** and the docs warn against
  holding it across an `await` point in async code.
- Span lifecycle (enter/exit) is **not** logged by default (`FmtSpan::NONE`);
  enable with `.with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)` if wanted.
- Beyond `instrument`, spans-across-async deep-dives, custom `Layer`s, and
  OpenTelemetry exporters are out of scope for redline.

## Subscriber setup

The init chain (all methods verified in `fmt/mod.rs`):

```rust
use tracing_subscriber::{fmt, EnvFilter};

let filter = EnvFilter::try_from_default_env()
    .unwrap_or_else(|_| EnvFilter::new("redline=info,notify=warn"));

fmt()
    .with_env_filter(filter)   // env-filter feature
    .with_writer(file)          // see next section
    .with_ansi(false)
    .init();                   // installs the global default subscriber
```

- `fmt()` returns a `SubscriberBuilder`; `with_env_filter(impl Into<EnvFilter>)`
  attaches the filter. `with_max_level(impl Into<LevelFilter>)` is the
  no-env alternative (e.g. `with_max_level(tracing::Level::INFO)`).
- `init()` sets the **global** default subscriber and panics if one is already
  set. `try_init()` returns `Result` instead. `set_global_default` can only
  succeed **once per process** (verified in `tracing-core/dispatcher.rs`).
- `init()` also installs the `LogTracer` (default `tracing-log` feature), which
  is how `notify`'s `log` records reach the file.
- The free function `tracing_subscriber::fmt::init()` is shorthand for
  `fmt().with_env_filter(EnvFilter::from_default_env()).init()` writing to
  **stdout** — do NOT use it in a TUI.

`EnvFilter` constructors (verified in `filter/env/mod.rs`):

- `EnvFilter::try_from_default_env()` — parses `RUST_LOG`; **errors if the var
  is unset** or contains an invalid directive (`FromEnvError`).
- `EnvFilter::from_default_env()` — lossy (bad directives ignored); if unset or
  empty, falls back to a default `ERROR` directive.
- `EnvFilter::try_new("redline=debug,notify=warn")` — strict parse of a string;
  an empty string yields the default `ERROR` directive.
- `EnvFilter::new(...)` — lossy string parse, same `ERROR` fallback.
- `.add_directive("my_mod=trace".parse()?)` — append extra directives on top.
- `EnvFilter::builder().with_default_directive(...)` — set the fallback level
  before parsing (used internally by the lossy constructors).

## Writing to a file

The default `MakeWriter` is `io::stdout` (`Layer::default()` sets
`make_writer: io::stdout`). To log to a file, open a `File` and pass it to
`with_writer`. `with_writer<W2>(self, make_writer: W2)` accepts anything
`for<'writer> MakeWriter<'writer> + 'static`.

Verified `MakeWriter` impls in `fmt/writer.rs`:

- `std::fs::File` **itself** implements `MakeWriter` — it writes through a
  shared `&File`. No `Arc`, no `Mutex`, no closure needed.
- `Arc<W> where &W: io::Write` — also works, same `&W` write path.
- `F: Fn() -> W where W: io::Write` — a zero-arg closure returning a writer
  directly (note: it returns `W`, **not** `Result<W, _>`).
- `Mutex<W>` — for writers that are not `Sync`.

Because the `File`/`Arc` impls just hand out a shared `&File` on every write,
there is no per-event allocation and no locking; `&File` is thread-safe, so
writes from the UI thread, tokio workers, and rayon index threads all share the
one open handle safely. `make_writer` is invoked on each event, but for a
`File` it is a no-op borrow.

The verified pattern for a TUI log file:

```rust
use std::fs::{File, OpenOptions};
use std::path::Path;

fn open_log_file(path: &Path) -> std::io::Result<File> {
    // append so restarts don't clobber history; create if missing
    OpenOptions::new().create(true).append(true).open(path)
}

// in main(), before the UI event loop:
let file = open_log_file(&log_path)?;
tracing_subscriber::fmt()
    .with_env_filter(filter)
    .with_writer(file)      // File: MakeWriter, writes via &File
    .with_ansi(false)      // strip color codes from the file
    .init();
```

Open with **append** (`append(true)`) so concurrent writers and process
restarts append rather than truncate. Passing the `File` by value to
`with_writer` moves the single handle into the subscriber; every thread writes
through the shared `&File`.

## Levels & filtering

- **Static (compile-time) max level**: `tracing`'s `max_level_*` /
  `release_max_level_*` features set `STATIC_MAX_LEVEL`; the macros check it
  before evaluating fields. By default no levels are disabled, so redline needs
  none of these.
- **Runtime filtering**: `EnvFilter` (above) or `with_max_level`.
- **Per-target directives** via `RUST_LOG` (directive syntax
  `target[span{field=value}]=level`, comma-separated, verified in
  `filter/env/mod.rs`):
  - `RUST_LOG=debug` — everything at DEBUG or above.
  - `RUST_LOG="redline=debug,notify=warn"` — redline at debug, `notify` at
    warn, everything else at the default.
  - A bare level sets the fallback for unmatched targets; a directive with no
    level equals `=trace`. Crate names with `-` map to `_` in targets.
- **Runtime reload** of the filter requires the `reload` feature (not enabled
  here) — skip it; restart redline with a new `RUST_LOG` instead.

## Usage in redline

- **Init in `main.rs` before the UI starts** (plan 01: "entry: config load,
  tracing init, event loop"). Open the file, build the filter, `init()`.
- **Log file location**: use the cache dir, consistent with the plan's decision
  that caches live in `~/.cache/redline/` (PLAN.md decision 8 and the
  `support-crates` skill). So log to `~/.cache/redline/redline.log` via
  `dirs::cache_dir().join("redline/redline.log")`. `dirs::state_dir()`
  (`~/.local/state`) is the alternative if you want non-cached state; both are
  `Option<PathBuf>` from the `dirs` crate (7.0).
- **What each module should log**:
  - **Indexer** (rayon/tree-sitter): `debug!` progress (files done, symbols
    found); `warn!`/`error!` on per-file parse or IO failures.
  - **Git ops** (git2): `info!` at op boundaries (status, stage, commit,
    checkout); `error!` with the `git2` error on failure.
  - **Watcher** (notify): `debug!` per debounced event batch; `notify`'s own
    `log` records (e.g. poll rescan warnings) surface automatically via the
    `LogTracer` — tune with `notify=warn` in the filter.
  - Wrap long operations (indexing, git status) in `info_span!`/`instrument` so
    related events group by span in the file.
- Keep `stdout` clean: never `println!` diagnostics; the only thing that writes
  to stdout is the TUI.

## Gotchas

- **Global subscriber is set-once.** `set_global_default` (behind `init()`)
  fails after the first call; a second `init()` panics. Integration tests that
  each call `init()` will collide — use a per-test subscriber or
  `tracing::dispatcher::with_default` for scoped overrides.
- **ANSI is on by default.** `Layer::default()` enables color when the `ansi`
  feature is set and `NO_COLOR` is unset (it is). Always call
  `.with_ansi(false)` for the file writer or you'll get escape codes in the log.
- **`try_from_default_env()` errors when `RUST_LOG` is unset.** Don't blindly
  `?` it in `main`; fall back to a sensible default (see Subscriber setup).
- **`with_max_level` replaces `with_env_filter`.** Calling both makes the
  later one win — don't chain them expecting a union.
- **Disabled levels are cheap.** The event macros check the static level and the
  callsite's interest *before* evaluating field expressions, so `trace!`/`debug!`
  calls you leave in hot paths cost almost nothing when filtered out (and with
  `tracing`'s `log` feature off, the disabled path evaluates nothing).
- **`EnteredSpan` is `!Send`.** Don't move a `.entered()` guard across thread
  boundaries or hold it across an `await`; prefer `enter()` for synchronous
  scopes and `instrument` for async.
- **Spans don't log by themselves.** Only events reach the file unless you add
  `.with_span_events(...)`; a span with no events inside produces no output.

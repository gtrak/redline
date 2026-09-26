//! redline — a read-focused TUI code browser (plan 001, issue 01).
//!
//! Entry point: load config, initialize tracing to a log file, start
//! the iocraft fullscreen render loop on the tokio runtime, and restore
//! the terminal cleanly on every exit path including panics.
//!
//! Module layout: `app/` is plain Rust (store, registry, keymap,
//! config) with zero iocraft/tokio dependencies; `model/` holds the
//! headless data models (project, files, buffers); `ui/` holds all
//! iocraft components; this file only starts/stops the render loop.

mod app;
mod git;
mod index_profile;
mod model;
mod nav;
mod search;
mod theme;
mod ui;

#[cfg(test)]
mod perf;
#[cfg(test)]
pub(crate) mod test_support;

/// The ONE lock every test that mutates process-global environment
/// holds while its env guard is live (P3b): `std::env::set_var`
/// mutates process state, and other threads reading env vars
/// concurrently is UB per the Rust docs (any variable — a reader of
/// `COLORTERM` races a writer of `HOME`) — so env-mutating tests
/// serialize on this shared crate-level mutex (separate per-module
/// locks would not exclude each other). Users: `git::commit`'s
/// `EnvScope` (+ its tests' per-body `TZ`/`GIT_AUTHOR_*` pins),
/// `model::files`'s `EnvGuard` (+ `model::project`'s usage), and the
/// `ui::file_view` tint tests' `COLORTERM` pins. Known residual (P1
/// finding, issue-guardrails): the `app::store` fetch tests read/mutate
/// `PATH` under their own `PATH_LOCK` — recommendation: move them onto
/// this lock (issue-git-test-harness lane owns that file).
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

use app::config;
use app::store::AppStore;
use iocraft::prelude::*;
use std::io::Write;
use std::path::PathBuf;
use ui::root::Root;

/// CLI args. `--notes=plain` switches the quit-dump shape (plan 005
/// issue 03); `--index-profile[=PATH]` runs the headless index profiler
/// (`src/index_profile.rs`) and exits before any terminal/TUI setup.
/// Unknown args are a HARD ERROR (clear feedback beats silently
/// ignoring them): usage on stderr, exit 2 — before the TUI starts.
#[derive(Debug, PartialEq, Eq)]
struct CliArgs {
    /// `--notes=plain`: the quit-dump prints in the grep/pipe shape.
    notes_plain: bool,
    /// `--index-profile[=PATH]`: run the headless index profiler (None =
    /// the normal TUI).
    index_profile: Option<PathBuf>,
    /// `--profile-top=N` (default 25): top-N rows in the report.
    profile_top: usize,
    /// `--profile-out=FILE`: write the per-file CSV.
    profile_out: Option<PathBuf>,
    /// `--profile-repeat=N` (default 1): repeat the run (warm page cache).
    profile_repeat: usize,
    /// `--profile-real-paths`: print real paths (default = anonymized).
    profile_real_paths: bool,
}

impl CliArgs {
    const DEFAULT_TOP: usize = 25;
}

fn usage() {
    eprintln!("usage: redline [--notes=plain]");
    eprintln!("       redline --index-profile[=PATH] [--profile-top=N --profile-out=FILE --profile-repeat=N --profile-real-paths]");
}

/// Parse `args` (without the program name) into [`CliArgs`]. Errors are
/// the user-facing message (the caller prints usage + exits 2).
fn parse_cli(args: impl IntoIterator<Item = String>) -> Result<CliArgs, String> {
    let mut cli = CliArgs {
        notes_plain: false,
        index_profile: None,
        profile_top: CliArgs::DEFAULT_TOP,
        profile_out: None,
        profile_repeat: 1,
        profile_real_paths: false,
    };
    // `--profile-top` / `--profile-repeat` validate to a positive int.
    let positive = |rest: &str, flag: &str| -> Result<usize, String> {
        rest.parse::<usize>()
            .ok()
            .filter(|v| *v > 0)
            .ok_or_else(|| format!("{flag} expects a positive integer, got `{rest}`"))
    };
    for arg in args {
        if arg == "--notes=plain" {
            cli.notes_plain = true;
        } else if arg == "--index-profile" {
            cli.index_profile = Some(PathBuf::from("."));
        } else if let Some(path) = arg.strip_prefix("--index-profile=") {
            if path.is_empty() {
                return Err("--index-profile= requires a path".into());
            }
            cli.index_profile = Some(PathBuf::from(path));
        } else if let Some(n) = arg.strip_prefix("--profile-top=") {
            cli.profile_top = positive(n, "--profile-top")?;
        } else if let Some(path) = arg.strip_prefix("--profile-out=") {
            if path.is_empty() {
                return Err("--profile-out= requires a path".into());
            }
            cli.profile_out = Some(PathBuf::from(path));
        } else if let Some(n) = arg.strip_prefix("--profile-repeat=") {
            cli.profile_repeat = positive(n, "--profile-repeat")?;
        } else if arg == "--profile-real-paths" {
            cli.profile_real_paths = true;
        } else {
            return Err(format!("unknown argument: {arg}"));
        }
    }
    Ok(cli)
}

fn parse_args() -> CliArgs {
    match parse_cli(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(msg) => {
            eprintln!("redline: {msg}");
            usage();
            std::process::exit(2);
        }
    }
}

#[cfg(unix)]
mod tty_stdout {
    //! plan 005 issue 03: when stdout is redirected (not a tty), the TUI's
    //! frames and cursor escapes would land on the redirected fd and bury
    //! the quit-dump (verified live: `redline > notes.txt` captured ~10 KB
    //! of alt-screen/CUP/SGR escapes). The fd-level fix: save the original
    //! stdout fd (`dup`) and point fd 1 at `/dev/tty` (`dup2`) BEFORE the
    //! render loop starts. iocraft/crossterm then write to the tty (their
    //! `std::io::stdout()` writes go through fd 1, which is now the tty),
    //! and the saved fd is used only for the dump. The iocraft
    //! `unstable-output-streams` route is flagged as "has crossterm
    //! caveats" (skills), and `libc` is only a transitive dependency (not
    //! re-exported by crossterm), so the two syscalls are declared here
    //! instead of adding a dependency.
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::fd::RawFd;
    use std::os::unix::io::AsRawFd;

    // SAFETY: POSIX libc declarations with exact C-ABI signatures
    // (`int dup(int)`, `int dup2(int, int)`, `int close(int)`); each call
    // site below states the fd-ownership invariant that makes ITS call
    // sound.
    unsafe extern "C" {
        fn dup(fd: RawFd) -> RawFd;
        fn dup2(oldfd: RawFd, newfd: RawFd) -> RawFd;
        fn close(fd: RawFd) -> i32;
    }

    /// The rerouted state: where the quit-dump goes. `tty` stays open for
    /// the process lifetime — after a successful `dup2`, fd 1 IS a dup of
    /// it and must remain valid under the render loop.
    pub struct ReroutedStdout {
        /// The original (redirected) stdout fd — the dump target when the
        /// `dup2` succeeded; `None` when it failed, in which case the dump
        /// goes to `/dev/tty` (fd 1 still carries the TUI frames and must
        /// not receive the dump).
        pub original_stdout: Option<RawFd>,
        /// The reason the `dup2` failed (reporting only; `None` on success).
        pub dup2_error: Option<std::io::Error>,
        /// `/dev/tty`, kept open for the process lifetime — after a
        /// successful `dup2`, fd 1 is a dup of it and must stay valid
        /// under the render loop.
        pub tty: File,
    }

    /// Save fd 1 and point it at `/dev/tty`. The caller must verify
    /// stdout is not a tty first (this is a no-op mistake on a tty).
    pub fn reroute_stdout_to_tty() -> io::Result<ReroutedStdout> {
        // SAFETY: fd 1 is the process's stdout — valid for the process's
        // whole life (opened before `main`). `dup` returns a NEW fd that
        // is uniquely ours: every path below either closes it (the two
        // `close(original)` calls) or transfers unique ownership to a
        // `File::from_raw_fd` — never both.
        let original = unsafe { dup(1) };
        if original < 0 {
            return Err(io::Error::last_os_error());
        }
        // /dev/tty must be opened READ-WRITE: after the `dup2` it IS fd 1,
        // and an O_RDONLY fd 1 fails every TUI write with EBADF (verified
        // live via strace — the std `File::open` O_RDONLY path). crossterm
        // opens its own /dev/tty handle O_RDWR|O_NONBLOCK for input; this
        // one is the output side.
        let tty = match OpenOptions::new().read(true).write(true).open("/dev/tty") {
            Ok(tty) => tty,
            // No controlling tty: undo the `dup` and report — the dump
            // will be skipped rather than corrupt the redirected stream.
            Err(e) => {
                // SAFETY: `original` came from our `dup` above and has not
                // been closed yet. This path (the `/dev/tty` open failed)
                // is mutually exclusive with the other two paths that end
                // this function — the `dup2`-failure path and the
                // success path (which hands the fd to `from_raw_fd`) — so
                // this is its only close. A double close would be UB, not
                // a leak.
                let _ = unsafe { close(original) };
                return Err(e);
            }
        };
        // SAFETY: `tty` is an owned `File` we just opened, so its raw fd
        // is valid, and fd 1 is the process's own fd — a legal `dup2`
        // target (replaced atomically). OWNERSHIP CONSEQUENCE, the real
        // invariant here: on success, fd 1 *is* a dup of `tty`, so the
        // render loop's writes (which go through fd 1) only work while
        // `tty` stays open — it is kept in `ReroutedStdout` and must
        // outlive the render loop to process exit.
        let dup2 = unsafe { dup2(tty.as_raw_fd(), 1) };
        if dup2 < 0 {
            // The tty fd stays open (closed at process exit); report and
            // degrade: the dump will go to /dev/tty instead. The saved
            // original fd is now dead weight (nothing will write to it,
            // and `original_stdout: None` means `emit_dump` never sees
            // it) — close it rather than leak it for the process life.
            // SAFETY: `original` came from our `dup` and has not been
            // closed yet. This path is mutually exclusive with the other
            // two paths that end this function (the open-failure path
            // already returned; the success path keeps the fd), so this
            // is its only close — a double close would be UB, not a leak.
            let _ = unsafe { close(original) };
            Ok(ReroutedStdout {
                original_stdout: None,
                dup2_error: Some(io::Error::last_os_error()),
                tty,
            })
        } else {
            Ok(ReroutedStdout {
                original_stdout: Some(original),
                dup2_error: None,
                tty,
            })
        }
    }
}

/// Where the quit-dump is written (plan 005 issue 03).
enum DumpSink {
    /// stdout was a tty at startup: the dump prints to stdout after the
    /// alternate screen exits (the terminal is restored by then).
    Stdout,
    /// stdout was redirected: fd 1 was rerouted to /dev/tty before the
    /// render loop; the dump goes to the saved fd (or /dev/tty on reroute
    /// failure). The whole variant is unix-only: on non-unix targets the
    /// reroute machinery (dup/dup2, /dev/tty) does not exist, and the
    /// match in `emit_dump` stays exhaustive with Stdout/Skipped alone.
    #[cfg(unix)]
    Rerouted(tty_stdout::ReroutedStdout),
    /// stdout was redirected but /dev/tty could not be opened: the dump is
    /// SKIPPED (report + skip rather than corrupt the redirected stream).
    Skipped,
}

/// Set up the quit-dump sink (plan 005 issue 03). Must run BEFORE the
/// render loop starts (the `dup2` must land first).
fn init_dump_sink() -> DumpSink {
    if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        return DumpSink::Stdout;
    }
    #[cfg(unix)]
    {
        match tty_stdout::reroute_stdout_to_tty() {
            Ok(rerouted) => {
                if let Some(err) = &rerouted.dup2_error {
                    eprintln!(
                        "redline: stdout is not a tty and the stdout->/dev/tty reroute failed ({err}); the annotation dump will be printed to /dev/tty at quit"
                    );
                }
                DumpSink::Rerouted(rerouted)
            }
            Err(e) => {
                eprintln!(
                    "redline: stdout is not a tty and /dev/tty could not be opened ({e}); the annotation dump is skipped at quit"
                );
                DumpSink::Skipped
            }
        }
    }
    #[cfg(not(unix))]
    {
        eprintln!("redline: stdout is not a tty; the annotation dump is skipped at quit");
        DumpSink::Skipped
    }
}

/// Print the quit-dump to its sink (plan 005 issue 03). `dump` is already
/// the full text (empty = nothing to print).
fn emit_dump(dump: &str, sink: &DumpSink) {
    if dump.is_empty() {
        return;
    }
    let ok = match sink {
        DumpSink::Stdout => std::io::stdout().write_all(dump.as_bytes()).is_ok(),
        DumpSink::Skipped => return,
        #[cfg(unix)]
        DumpSink::Rerouted(rerouted) => {
            if let Some(fd) = rerouted.original_stdout {
                use std::os::fd::FromRawFd;
                // SAFETY: `fd` is the `original_stdout` from our `dup`;
                // on the success path it was never closed (the two
                // `close(original)` calls return before this sink ever
                // exists), so the fd is still valid. `from_raw_fd` takes
                // UNIQUE ownership: this `File` is now the only owner and
                // the only thing that will close it (on drop, after the
                // dump) — closing the fd anywhere else would be a double
                // close (UB).
                unsafe { std::fs::File::from_raw_fd(fd) }
                    .write_all(dump.as_bytes())
                    .is_ok()
            } else {
                // dup2 failed: the dump goes to /dev/tty (fd 1 still
                // carries the TUI frames). Write directly to the file —
                // a BufWriter would defer small writes to drop, whose
                // error is ignored, silently losing a failed tty write.
                let mut tty = &rerouted.tty;
                tty.write_all(dump.as_bytes()).is_ok()
            }
        }
    };
    if !ok {
        eprintln!("redline: could not write the annotation dump");
    }
}


fn log_path() -> std::path::PathBuf {
    dirs::cache_dir()
        .map(|dir| dir.join("redline").join("redline.log"))
        .unwrap_or_else(|| std::path::PathBuf::from("redline.log"))
}

fn init_tracing() -> std::path::PathBuf {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .expect("open log file");
    tracing_subscriber::fmt()
        .with_writer(file)
        .with_ansi(false)
        .with_env_filter(filter)
        .init();
    path
}

/// Terminal restore + panic log, chained to the hook iocraft installed
/// (if any) so a panic never leaves the terminal in raw mode.
fn init_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Best-effort: iocraft's render loop enables raw mode; the call
        // fails (silently) when raw mode was never active (pre-loop
        // panics).
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = crossterm::terminal::disable_raw_mode();
        }));
        tracing::error!("redline panicked: {}", panic_message(info));
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| default(info)));
    }));
}

fn panic_message(info: &std::panic::PanicHookInfo) -> String {
    info.payload()
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| info.payload().downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown panic".to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_panic_hook();
    let cli = parse_args();
    // Headless index profiler (the issue-index-profiler task): run BEFORE
    // any terminal/TUI setup (before the IsTerminal check and everything
    // after it) and exit. Works with no tty. When the flag is absent this
    // is a single branch miss.
    if let Some(root) = cli.index_profile {
        index_profile::run(&index_profile::Options {
            root,
            top: cli.profile_top,
            out: cli.profile_out,
            repeat: cli.profile_repeat,
            real_paths: cli.profile_real_paths,
        })?;
        return Ok(());
    }
    let log_path = init_tracing();
    tracing::info!(log = %log_path.display(), "redline starting");

    let config: config::Config = config::load_tolerant();
    let mut store = AppStore::new();
    // Measure the terminal height at startup: crossterm emits no Resize
    // event at startup, so the store's default viewport_lines (24) would
    // be wrong on terminals that are not ~27 rows.
    if let Ok((_, h)) = crossterm::terminal::size() {
        store.set_viewport_lines(h.saturating_sub(3) as usize);
    }
    if let Err(err) = config.validate_bindings(&store.registry) {
        tracing::warn!("ignoring some key-bindings: {err}");
    }
    if let Err(err) = store.apply_config(&config) {
        tracing::warn!("key-binding overrides not fully applied: {err}");
    }
    theme::set_current(store.theme().clone());
    tracing::info!(bindings = config.key_bindings.len(), "config loaded");

    // Start the file watcher for the initial project (plan decision #7). It
    // runs on this runtime and publishes project-change events to the bus the
    // FileView / git-status subscribers consume. Stops cleanly on quit (the
    // store's Drop signals the watcher to tear down).
    store.start_watcher();

    // Start the background symbol index build (issue 05). Runs on a
    // rayon thread via spawn_blocking; the UI drain installs each result
    // into the store. No-op in plain unit tests (no runtime).
    // Capture the index-bus receiver BEFORE the job starts: a tokio watch
    // send with zero receivers discards the event, which would lose the full
    // index result and leave the status line stuck at "indexing 0/N" forever.
    let index_rx = store.index_bus.subscribe();
    store.set_index_rx(index_rx);
    store.start_indexing();

    // The store lives in the element context; the root component reads and
    // updates it there (including the SearchBus drain, issue 06). Keep a
    // handle so we can stop the watcher cleanly at shutdown (before the
    // runtime tears down).
    let store_handle = std::sync::Arc::new(std::sync::Mutex::new(store));
    let mut app = element! {
        ContextProvider(value: Context::owned(std::sync::Arc::clone(&store_handle))) {
            Root
        }
    };
    // iocraft's fullscreen loop exits on Ctrl+C by default -- which would
    // swallow every C-c-prefixed binding (C-c p ..., the commit editor's
    // C-c C-c/C-c C-k) before the keymap ever sees the key. Opt out: C-c is
    // ours (a mode prefix); C-x C-c is the quit path (bare `q` is a no-op
    // close-view on the root buffer view, issue 05 finding 5).
    // plan 005 issue 03: set up the quit-dump sink BEFORE the loop starts
    // (the stdout->/dev/tty dup2 must land first so the render loop's
    // frames never reach a redirected stdout).
    let dump_sink = init_dump_sink();
    app.fullscreen().ignore_ctrl_c().await?;
    // Clean watcher shutdown: take the watcher out (brief lock) and await its
    // teardown WITHOUT holding the store lock across the await (the runtime
    // must not block on a std Mutex while a task is running).
    let watcher = store_handle.lock().unwrap().take_watcher();
    if let Some(mut w) = watcher {
        w.stop_and_wait().await;
    }
    tracing::info!("redline exited cleanly");
    // plan 005 issue 03: the quit-dump — printed AFTER the alternate screen
    // has exited and the watcher has stopped, from the FINAL store state
    // (any annotations saved during quit save-prompts are included). Never
    // printed before this point (it must not interleave with the TUI).
    {
        let mut store = store_handle.lock().unwrap();
        let items = store.annotations_for_dump();
        let root = store
            .project
            .as_ref()
            .map(|p| p.root.display().to_string())
            .unwrap_or_default();
        let dump = app::store::format_notes_dump(
            &items,
            &root,
            cli.notes_plain,
        );
        emit_dump(&dump, &dump_sink);
    }
    Ok(())
}

#[cfg(test)]
mod cli_tests {
    //! Arg parsing (plan 005 issue 03 extended by the index-profiler
    //! task): `--notes=plain` still parses, the headless profiler flags
    //! parse, and GENUINELY unknown args remain a hard error.
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_args_default_to_the_tui() {
        let cli = parse_cli(strs(&[])).unwrap();
        assert!(!cli.notes_plain);
        assert!(cli.index_profile.is_none());
        assert_eq!(cli.profile_top, CliArgs::DEFAULT_TOP);
        assert_eq!(cli.profile_repeat, 1);
        assert!(cli.profile_out.is_none());
        assert!(!cli.profile_real_paths);
    }

    #[test]
    fn notes_plain_still_accepted() {
        let cli = parse_cli(strs(&["--notes=plain"])).unwrap();
        assert!(cli.notes_plain);
        assert!(cli.index_profile.is_none());
    }

    #[test]
    fn index_profile_flag_and_path() {
        assert_eq!(
            parse_cli(strs(&["--index-profile"])).unwrap().index_profile,
            Some(PathBuf::from("."))
        );
        assert_eq!(
            parse_cli(strs(&["--index-profile=./big-project"]))
                .unwrap()
                .index_profile,
            Some(PathBuf::from("./big-project"))
        );
        assert!(parse_cli(strs(&["--index-profile="])).is_err());
    }

    #[test]
    fn profile_options_parse() {
        let cli = parse_cli(strs(&[
            "--index-profile",
            "--profile-top=10",
            "--profile-out=/tmp/index.csv",
            "--profile-repeat=3",
            "--profile-real-paths",
        ]))
        .unwrap();
        assert_eq!(cli.profile_top, 10);
        assert_eq!(cli.profile_repeat, 3);
        assert_eq!(
            cli.profile_out.as_deref(),
            Some(std::path::Path::new("/tmp/index.csv"))
        );
        assert!(cli.profile_real_paths);
    }

    #[test]
    fn profile_options_validate_values() {
        assert!(parse_cli(strs(&["--profile-top=0"])).is_err());
        assert!(parse_cli(strs(&["--profile-top=abc"])).is_err());
        assert!(parse_cli(strs(&["--profile-repeat=0"])).is_err());
        assert!(parse_cli(strs(&["--profile-out="])).is_err());
        // Valid: top=1 is allowed (just one row).
        assert_eq!(parse_cli(strs(&["--profile-top=1"])).unwrap().profile_top, 1);
    }

    #[test]
    fn genuinely_unknown_args_still_hard_error() {
        let err = parse_cli(strs(&["--bogus"])).unwrap_err();
        assert!(err.contains("--bogus"), "error names the arg: {err}");
        let err = parse_cli(strs(&["--notes=plain", "--nope"]))
            .unwrap_err();
        assert!(err.contains("--nope"), "error names the arg: {err}");
    }
}

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
mod model;
mod nav;
mod search;
mod syntax;
mod theme;
mod ui;

#[cfg(test)]
mod perf;

use app::config;
use app::store::AppStore;
use iocraft::prelude::*;
use std::io::Write;
use ui::root::Root;

/// The quit-dump format (plan 005 issue 03). The default is the
/// agent-consumable block; `--notes=plain` switches to the grep/pipe
/// `path:line: text` shape.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NotesFormat {
    Block,
    Plain,
}

/// CLI args (plan 005 issue 03). The only argument is `--notes=plain`.
/// Unknown args are a HARD ERROR (clear feedback beats silently dumping in
/// the wrong shape): usage on stderr, exit 2 — before the TUI starts.
fn parse_notes_format() -> NotesFormat {
    let mut format = NotesFormat::Block;
    for arg in std::env::args().skip(1) {
        if arg == "--notes=plain" {
            format = NotesFormat::Plain;
        } else {
            eprintln!("redline: unknown argument: {arg}");
            eprintln!("usage: redline [--notes=plain]");
            std::process::exit(2);
        }
    }
    format
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
                let _ = unsafe { close(original) };
                return Err(e);
            }
        };
        let dup2 = unsafe { dup2(tty.as_raw_fd(), 1) };
        if dup2 < 0 {
            // The tty fd stays open (closed at process exit); report and
            // degrade: the dump will go to /dev/tty instead. The saved
            // original fd is now dead weight (nothing will write to it,
            // and `original_stdout: None` means `emit_dump` never sees
            // it) — close it rather than leak it for the process life.
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
                // `from_raw_fd` takes ownership: the fd is closed on drop
                // (fine — the dump is the last thing written to it).
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
    let notes_format = parse_notes_format();
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
            notes_format == NotesFormat::Plain,
        );
        emit_dump(&dump, &dump_sink);
    }
    Ok(())
}

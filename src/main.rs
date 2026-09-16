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

use app::config;
use app::store::AppStore;
use iocraft::prelude::*;
use ui::root::Root;

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
    app.fullscreen().await?;
    // Clean watcher shutdown: take the watcher out (brief lock) and await its
    // teardown WITHOUT holding the store lock across the await (the runtime
    // must not block on a std Mutex while a task is running).
    let watcher = store_handle.lock().unwrap().take_watcher();
    if let Some(mut w) = watcher {
        w.stop_and_wait().await;
    }
    tracing::info!("redline exited cleanly");
    Ok(())
}

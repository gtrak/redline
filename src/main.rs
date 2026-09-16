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
mod model;
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
    if let Err(err) = config.validate_bindings(&store.registry) {
        tracing::warn!("ignoring some key-bindings: {err}");
    }
    if let Err(err) = store.apply_config(&config) {
        tracing::warn!("key-binding overrides not fully applied: {err}");
    }
    theme::set_current(*store.theme());
    tracing::info!(bindings = config.key_bindings.len(), "config loaded");

    // The store lives in the element context; the root component reads
    // and updates it there.
    let mut app = element! {
        ContextProvider(value: Context::owned(std::sync::Arc::new(std::sync::Mutex::new(store)))) {
            Root
        }
    };
    app.fullscreen().await?;
    tracing::info!("redline exited cleanly");
    Ok(())
}

//! App core: central store, command registry, keymap engine, config
//! parsing, the project-change bus, and the per-project file watcher.
//!
//! Most of this layer is plain Rust (unit-testable in isolation; all
//! rendering lives in `ui/`). The `events` and `watcher` modules reach
//! for `tokio` + `notify` (plan decision #7: file watching is
//! first-class), and the store touches tokio for the index and search
//! bus drains (issues 05 and 06). All of them stay free of iocraft.

pub mod command;
pub mod config;
pub mod events;
pub mod keymap;
pub mod store;
pub mod watcher;

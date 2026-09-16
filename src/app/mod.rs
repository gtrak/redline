//! App core: central store, command registry, keymap engine, config
//! parsing, the project-change bus, and the per-project file watcher.
//!
//! Most of this layer is plain Rust (unit-testable in isolation; all
//! rendering lives in `ui/`). `events` and `watcher` are the two
//! modules that reach for `tokio` + `notify` (plan decision #7: file
//! watching is first-class). They stay free of iocraft.

pub mod command;
pub mod config;
pub mod events;
pub mod keymap;
pub mod store;
pub mod watcher;

//! Plain-Rust app core: central store, command registry, keymap engine,
//! and config parsing. No iocraft/tokio dependencies — unit-testable in
//! isolation (plan decision: all rendering lives in `ui/`).

pub mod command;
pub mod config;
pub mod keymap;
pub mod store;

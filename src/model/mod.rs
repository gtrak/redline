//! Data models: project layer, file lists, and the open-buffer set.
//! Plain Rust — zero iocraft/tokio/crossterm (plan layering rule:
//! `src/model/` is headless and unit-testable in isolation).

pub mod buffer;
pub mod files;
pub mod project;
pub mod sections;

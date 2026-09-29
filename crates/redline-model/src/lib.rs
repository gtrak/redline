//! Data models: project layer, file lists, and the open-buffer set.
//! Plain Rust — zero iocraft/tokio/crossterm (plan layering rule:
//! this crate is headless and unit-testable in isolation).
//!
//! Mid-tier crate (plan 014 stage 3): `sections.rs` builds the magit
//! status `Section` tree FROM `redline_git`'s plain status/diff data,
//! so the production edges run model -> {git, syntax} — never the
//! reverse (the bin's app/search layers consume this crate, not the
//! other way around).

pub mod buffer;
pub mod files;
pub mod project;
pub mod sections;
pub mod text_width;
pub mod tree_layout;

/// Test-only fixture writer shared by the `model` test suites (item: the
/// identical `fn file()` twins in `files::tests` / `project::tests`).
/// `#[cfg(test)]` — compiled out of every non-test build.
#[cfg(test)]
pub(crate) fn write_test_file(path: impl AsRef<std::path::Path>, content: &str) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

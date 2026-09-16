//! Git wrappers (libgit2 via `git2`): the magit subset redline needs.
//!
//! Layering rule (plan decision #5): git2 types never leak past this
//! module. `src/git` wraps libgit2 and returns plain data structs
//! (`RepoStatus`, `FileDiff`, `LogEntry`, …) plus a `thiserror` error
//! enum; the section-tree model (`src/model/sections.rs`) and the UI render
//! that plain data.

pub mod blame;
pub mod commit;
pub mod diff;
pub mod error;
pub mod log;
pub mod refs;
pub mod repo;
pub mod status;

pub use error::GitError;
pub use repo::GitRepo;

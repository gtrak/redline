//! Syntax highlighting: grammar registry, highlight pipeline, and cache.
//! Plain Rust — zero iocraft/tokio (plan layering rule).

pub mod cache;
pub mod c_cpp;
pub mod clojure;
pub mod conventions;
pub mod highlight;
pub mod java;
pub mod language;
pub mod node;
pub mod queries;
pub mod registry;
pub mod tokens;

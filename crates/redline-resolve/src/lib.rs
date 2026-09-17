//! Language/tooling-aware symbol resolution for redline.
//!
//! Redline resolves definitions inside the workspace with tree-sitter Xref
//! (the app crate owns that). When the workspace has no definition, the
//! jump falls through to a per-language [`ToolingProvider`]: the language's
//! own tools locate and, if needed, fetch the real source (Rust first:
//! cargo metadata → registry source dir, cargo fetch on demand).
//!
//! This crate is deliberately app-free (no iocraft/store): it takes plain
//! inputs (project root, symbol context) and returns plain locations, so
//! it can be built and tested in parallel with UX lanes.

use std::path::PathBuf;

/// Where a resolution landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSource {
    /// File to open.
    pub file: PathBuf,
    /// Root of the source tree the file belongs to (external crate root or
    /// the workspace itself).
    pub source_root: PathBuf,
    /// True when the source tree is outside the workspace (registry,
    /// vendored) — the app opens these read-only.
    pub external: bool,
}

/// A per-language fall-through resolver. Workspace Xref (app crate) is
/// always tried first; providers only see misses.
pub trait ToolingProvider: Send + Sync {
    /// Human name (Rust/cargo, Go/go-modules, …).
    fn name(&self) -> &'static str;
    /// Languages this provider handles (lowercase, e.g. "rust").
    fn languages(&self) -> &'static [&'static str];
    /// Resolve a symbol context to a concrete source location. May shell
    /// out (fetch on demand is sanctioned by the operator directive).
    fn resolve(&self, ctx: &SymbolContext) -> anyhow::Result<ResolvedSource>;
}

/// What the app hands a provider on a workspace miss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolContext {
    /// Workspace (or open project) root.
    pub workspace_root: PathBuf,
    /// The symbol as typed/navigated (e.g. `tokio::spawn`, `Deserialize`).
    pub symbol: String,
    /// Path of the file the jump started from, workspace-relative.
    pub from_file: PathBuf,
}

/// Guess the crate name from a use-path-shaped symbol
/// (`tokio::spawn` → `tokio`; bare `Deserialize` → None: needs scope info
/// the app's tree-sitter layer provides later).
pub fn crate_from_symbol(symbol: &str) -> Option<&str> {
    let first = symbol.split("::").next()?;
    if first.is_empty() || !first.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_from_symbol_first_segment() {
        assert_eq!(crate_from_symbol("tokio::spawn"), Some("tokio"));
        assert_eq!(crate_from_symbol("std::fs::read"), Some("std"));
        assert_eq!(crate_from_symbol("Deserialize"), Some("Deserialize"));
    }

    #[test]
    fn crate_from_symbol_rejects_garbage() {
        assert_eq!(crate_from_symbol(""), None);
        assert_eq!(crate_from_symbol("::x"), None);
    }
}

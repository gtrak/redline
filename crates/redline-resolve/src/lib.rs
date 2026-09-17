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

pub mod providers;

use std::path::PathBuf;

mod cargo;

pub use cargo::CargoProvider;

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
    /// Best-effort 1-based line of the item's definition inside `file`.
    /// `None` when a file was located but no definition line could be pinned
    /// down; the app refines placement precisely later.
    pub line: Option<u32>,
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

/// One provider attempt, recorded for a transparent trace.
#[derive(Debug, Clone)]
pub struct Attempt {
    /// Provider name (e.g. `"rust"`).
    pub provider: String,
    /// `true` when this provider produced the resolution.
    pub hit: bool,
    /// Human detail: the resolved file on a hit, or the reason on a miss.
    pub detail: String,
}

/// The ordered record of provider attempts for one resolution.
#[derive(Debug, Clone, Default)]
pub struct ResolveTrace {
    pub attempts: Vec<Attempt>,
}

/// The resolution plus the trace of how it was reached.
#[derive(Debug, Clone)]
pub struct ResolveOutcome {
    pub source: ResolvedSource,
    pub trace: ResolveTrace,
}

/// An ordered chain of [`ToolingProvider`]s. On a workspace miss the chain is
/// walked in order; the first provider that returns a `ResolvedSource` wins
/// (first hit). Every attempt — hit or miss — is recorded in the trace.
#[derive(Default)]
pub struct Resolver {
    providers: Vec<Box<dyn ToolingProvider>>,
}

impl Resolver {
    /// An empty chain (no providers).
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Build from a ready list of boxed providers.
    pub fn with_providers(providers: Vec<Box<dyn ToolingProvider>>) -> Self {
        Self { providers }
    }

    /// Append a provider (tried after any previously-added ones).
    pub fn add<P: ToolingProvider + 'static>(&mut self, provider: P) {
        self.providers.push(Box::new(provider));
    }

    /// Number of providers in the chain.
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Provider names in chain order.
    pub fn names(&self) -> Vec<&'static str> {
        self.providers.iter().map(|p| p.name()).collect()
    }

    /// Resolve `ctx`, walking the chain in order; first hit wins. Returns the
    /// resolved source (the trace is discarded).
    pub fn resolve(&self, ctx: &SymbolContext) -> anyhow::Result<ResolvedSource> {
        self.resolve_traced(ctx).map(|o| o.source)
    }

    /// As [`Resolver::resolve`], but also returns the attempt trace.
    pub fn resolve_traced(&self, ctx: &SymbolContext) -> anyhow::Result<ResolveOutcome> {
        let mut trace = ResolveTrace::default();
        for provider in &self.providers {
            match provider.resolve(ctx) {
                Ok(source) => {
                    trace.attempts.push(Attempt {
                        provider: provider.name().to_string(),
                        hit: true,
                        detail: format!("resolved to {}", source.file.display()),
                    });
                    return Ok(ResolveOutcome { source, trace });
                }
                Err(e) => {
                    trace.attempts.push(Attempt {
                        provider: provider.name().to_string(),
                        hit: false,
                        detail: e.to_string(),
                    });
                    // Fall through to the next provider.
                }
            }
        }
        anyhow::bail!(
            "no tooling provider could resolve symbol `{}` (tried {} provider(s): {})",
            ctx.symbol,
            self.providers.len(),
            self
                .providers
                .iter()
                .map(|p| p.name())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn hit(name: &'static str) -> ResolvedSource {
        ResolvedSource {
            file: PathBuf::from(format!("/{name}.rs")),
            source_root: PathBuf::from("/"),
            external: false,
            line: Some(1),
        }
    }

    /// A provider that always hits with a fixed, provider-named file.
    struct StubProvider {
        name: &'static str,
    }
    impl ToolingProvider for StubProvider {
        fn name(&self) -> &'static str {
            self.name
        }
        fn languages(&self) -> &'static [&'static str] {
            &["stub"]
        }
        fn resolve(&self, _ctx: &SymbolContext) -> anyhow::Result<ResolvedSource> {
            Ok(hit(self.name))
        }
    }

    /// A provider that always misses.
    struct MissProvider;
    impl ToolingProvider for MissProvider {
        fn name(&self) -> &'static str {
            "miss"
        }
        fn languages(&self) -> &'static [&'static str] {
            &["stub"]
        }
        fn resolve(&self, _ctx: &SymbolContext) -> anyhow::Result<ResolvedSource> {
            anyhow::bail!("not handled")
        }
    }

    fn ctx() -> SymbolContext {
        SymbolContext {
            workspace_root: PathBuf::from("/tmp"),
            symbol: "crate::sym".to_string(),
            from_file: PathBuf::from("src/lib.rs"),
        }
    }

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

    #[test]
    fn first_hit_wins() {
        let mut r = Resolver::new();
        r.add(StubProvider { name: "first" });
        r.add(StubProvider { name: "second" });
        let out = r.resolve_traced(&ctx()).unwrap();
        assert_eq!(out.source.file, PathBuf::from("/first.rs"));
        // Only the hitting provider is recorded (we stop at first hit).
        assert_eq!(out.trace.attempts.len(), 1);
        assert_eq!(out.trace.attempts[0].provider, "first");
        assert!(out.trace.attempts[0].hit);
    }

    #[test]
    fn falls_through_on_miss() {
        let mut r = Resolver::new();
        r.add(MissProvider);
        r.add(StubProvider { name: "second" });
        let out = r.resolve_traced(&ctx()).unwrap();
        assert_eq!(out.source.file, PathBuf::from("/second.rs"));
        assert_eq!(out.trace.attempts.len(), 2);
        assert!(!out.trace.attempts[0].hit);
        assert_eq!(out.trace.attempts[0].provider, "miss");
        assert!(out.trace.attempts[1].hit);
    }

    #[test]
    fn all_miss_error_names_providers() {
        let mut r = Resolver::new();
        r.add(MissProvider);
        let err = r.resolve_traced(&ctx()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("crate::sym"), "msg: {msg}");
        assert!(msg.contains("miss"), "msg: {msg}");
    }

    #[test]
    fn empty_chain_errors() {
        let r = Resolver::new();
        assert!(r.resolve_traced(&ctx()).is_err());
    }
}

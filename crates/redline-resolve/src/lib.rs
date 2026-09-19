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

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

mod cargo;

pub use cargo::CargoProvider;

/// Run a command with a timeout. The child is spawned directly in this
/// thread; stdout/stderr are drained by reader threads so the pipe buffer
/// does not deadlock the child. On timeout the child is killed and reaped
/// before the error is returned.
pub(crate) fn run_with_timeout(mut cmd: Command, timeout: Duration) -> anyhow::Result<Output> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to run command: {e}"))?;

    // Detach stdout/stderr so we can read them concurrently with waiting.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let stdout_reader = stdout_pipe.map(|mut p| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = Read::read_to_end(&mut p, &mut buf);
            buf
        })
    });
    let stderr_reader = stderr_pipe.map(|mut p| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = Read::read_to_end(&mut p, &mut buf);
            buf
        })
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    match child.wait() {
                        Ok(s) => break s,
                        Err(e) => {
                            return Err(anyhow::anyhow!(
                                "command timed out after {timeout:?} and reap failed: {e}"
                            ));
                        }
                    }
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(anyhow::anyhow!("failed to wait for command: {e}")),
        }
    };

    let stdout = stdout_reader
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();
    let stderr = stderr_reader
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

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
    /// The app's tree-sitter hint for a BARE (separator-free) symbol: the
    /// FULL path the bare name resolves to — the `use` declaration that
    /// brings it into scope, item included
    /// (`use serde::Deserialize;` → `["serde", "Deserialize"]`; an aliased
    /// `use serde::Deserialize as D;` yields the SAME value for bare `D`).
    /// For a path-shaped symbol the app may pass the enclosing item chain
    /// instead (carried, not consumed by the providers today). Empty when
    /// the app has no hint (no such import, non-Rust buffer, failed
    /// parse) — a provider MUST then degrade to its exact no-hint
    /// behavior (the byte-for-byte degradation contract).
    pub scope: Vec<String>,
    /// The buffer's language (lowercase, e.g. `"python"` — matching the
    /// providers' [`ToolingProvider::languages`] strings), or `None` when
    /// the app does not know it. `None` preserves the pre-dispatch
    /// behavior: every provider is tried in chain order. When set, only
    /// providers whose `languages()` contains it are attempted (the
    /// others are not probed and leave no trace entry).
    pub language: Option<String>,
}

/// A bare symbol with a non-empty scope hint → the hint IS the full path
/// the symbol resolves to (a `use` declaration's original path, item
/// included; the provider takes its item from the path's last segment).
/// `None` when the symbol already carries the language's separator
/// (path-shaped — its own path wins) or the scope is empty (no hint —
/// the provider keeps its no-hint behavior byte-for-byte).
pub(crate) fn scope_qualified(sep: &str, symbol: &str, scope: &[String]) -> Option<String> {
    if !scope.is_empty() && !symbol.contains(sep) {
        Some(scope.join(sep))
    } else {
        None
    }
}

/// Guess the crate name from a use-path-shaped symbol
/// (`tokio::spawn` → `tokio`; a bare `Deserialize` → `Some("Deserialize")` —
/// the single-segment result is rejected downstream until the app's
/// scope hint (007-03) turns it into a real path).
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

/// The language-dispatch rule: an unset context language tries every
/// provider (the pre-dispatch behavior); a set one is only handled by
/// providers whose `languages()` contains it (lowercase string match).
fn provider_handles_language(p: &dyn ToolingProvider, language: Option<&str>) -> bool {
    match language {
        Some(lang) => p.languages().contains(&lang),
        None => true,
    }
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
    ///
    /// Language dispatch: when `ctx.language` is set, providers whose
    /// `languages()` do not contain it are NOT probed (they leave no
    /// trace entry — the trace only records real attempts). An unset
    /// language tries every provider in order, exactly as before.
    pub fn resolve_traced(&self, ctx: &SymbolContext) -> anyhow::Result<ResolveOutcome> {
        let language = ctx.language.as_deref();
        let eligible: Vec<&Box<dyn ToolingProvider>> = self
            .providers
            .iter()
            .filter(|p| provider_handles_language(p.as_ref(), language))
            .collect();
        if let Some(lang) = language && eligible.is_empty() {
            anyhow::bail!(
                "no tooling provider handles language `{lang}` ({} provider(s) registered, none attempted)",
                self.providers.len()
            );
        }
        let mut trace = ResolveTrace::default();
        for provider in &eligible {
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
            eligible.len(),
            eligible
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

    /// A provider that records whether `resolve` was actually probed.
    struct SpyProvider {
        name: &'static str,
        langs: &'static [&'static str],
        probed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    impl ToolingProvider for SpyProvider {
        fn name(&self) -> &'static str {
            self.name
        }
        fn languages(&self) -> &'static [&'static str] {
            self.langs
        }
        fn resolve(&self, _ctx: &SymbolContext) -> anyhow::Result<ResolvedSource> {
            self.probed.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(hit(self.name))
        }
    }

    fn ctx() -> SymbolContext {
        SymbolContext {
            workspace_root: PathBuf::from("/tmp"),
            symbol: "crate::sym".to_string(),
            from_file: PathBuf::from("src/lib.rs"),
            scope: Vec::new(),
            language: None,
        }
    }

    #[test]
    fn scope_qualified_bare_symbol_with_hint() {
        assert_eq!(
            scope_qualified("::", "Deserialize", &["serde".into(), "Deserialize".into()]),
            Some("serde::Deserialize".to_string())
        );
        assert_eq!(
            scope_qualified(".", "doThing", &["acme".into(), "doThing".into()]),
            Some("acme.doThing".to_string())
        );
    }

    #[test]
    fn scope_qualified_no_hint_or_path_shaped() {
        // Empty scope: no hint — the provider keeps its no-hint behavior.
        assert_eq!(scope_qualified("::", "Deserialize", &[]), None);
        // A path-shaped symbol already carries its own path.
        assert_eq!(
            scope_qualified(
                "::",
                "serde::Deserialize",
                &["tokio".into(), "spawn".into()]
            ),
            None
        );
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

    /// (011-01) The trace must contain NO attempt for a provider whose
    /// `languages()` does not match the context language — the miss never
    /// probes it (asserted via the probe spy, not just the trace).
    #[test]
    fn language_dispatch_only_matching_provider_attempted() {
        let rust = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let python = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut r = Resolver::new();
        r.add(SpyProvider {
            name: "rust",
            langs: &["rust"],
            probed: rust.clone(),
        });
        r.add(SpyProvider {
            name: "python",
            langs: &["python"],
            probed: python.clone(),
        });

        // A python context: only python is probed; the trace records it
        // alone, and the rust provider was never attempted.
        let mut c = ctx();
        c.language = Some("python".to_string());
        let out = r.resolve_traced(&c).unwrap();
        assert_eq!(out.source.file, PathBuf::from("/python.rs"));
        assert_eq!(out.trace.attempts.len(), 1);
        assert_eq!(out.trace.attempts[0].provider, "python");
        assert!(python.load(std::sync::atomic::Ordering::SeqCst));
        assert!(
            !rust.load(std::sync::atomic::Ordering::SeqCst),
            "rust was probed"
        );

        // A rust context (fresh spies): the mirror image.
        let rust2 = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let python2 = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut r = Resolver::new();
        r.add(SpyProvider {
            name: "rust",
            langs: &["rust"],
            probed: rust2.clone(),
        });
        r.add(SpyProvider {
            name: "python",
            langs: &["python"],
            probed: python2.clone(),
        });
        let mut c = ctx();
        c.language = Some("rust".to_string());
        let out = r.resolve_traced(&c).unwrap();
        assert_eq!(out.source.file, PathBuf::from("/rust.rs"));
        assert_eq!(out.trace.attempts.len(), 1);
        assert_eq!(out.trace.attempts[0].provider, "rust");
        assert!(rust2.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!python2.load(std::sync::atomic::Ordering::SeqCst), "python was probed");
    }

    /// (011-01) Regression pin: an UNSET language preserves today's
    /// behavior — every provider is tried in chain order (first hit wins,
    /// a miss falls through and lands in the trace).
    #[test]
    fn language_dispatch_unset_tries_all_in_order() {
        let rust = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut r = Resolver::new();
        r.add(SpyProvider {
            name: "rust",
            langs: &["rust"],
            probed: rust.clone(),
        });
        r.add(MissProvider);
        let c = ctx();
        assert!(c.language.is_none());
        let out = r.resolve_traced(&c).unwrap();
        assert_eq!(out.source.file, PathBuf::from("/rust.rs"));
        assert_eq!(out.trace.attempts.len(), 1);
        assert!(rust.load(std::sync::atomic::Ordering::SeqCst));

        // And when the first provider misses, the second IS still probed
        // in order (byte-for-byte the pre-dispatch walk).
        let mut r = Resolver::new();
        r.add(MissProvider);
        r.add(StubProvider { name: "second" });
        let out = r.resolve_traced(&c).unwrap();
        assert_eq!(out.source.file, PathBuf::from("/second.rs"));
        assert_eq!(out.trace.attempts.len(), 2);
        assert_eq!(out.trace.attempts[0].provider, "miss");
        assert!(!out.trace.attempts[0].hit);
    }

    /// (011-01) No registered provider handles the context language → a
    /// clear error names the language and probes NOTHING.
    #[test]
    fn language_dispatch_no_matching_provider_errors() {
        let rust = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let python = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut r = Resolver::new();
        r.add(SpyProvider {
            name: "rust",
            langs: &["rust"],
            probed: rust.clone(),
        });
        r.add(SpyProvider {
            name: "python",
            langs: &["python"],
            probed: python.clone(),
        });
        let mut c = ctx();
        c.language = Some("go".to_string());
        let err = r.resolve_traced(&c).unwrap_err().to_string();
        assert!(err.contains("go"), "err: {err}");
        assert!(
            err.contains("no tooling provider handles language"),
            "err: {err}"
        );
        assert!(!rust.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!python.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// (011-01) A provider whose `languages()` is EMPTY is skipped when a
    /// language is set (it claims no language) but still tried for an
    /// unset one (backward-compatible walk).
    #[test]
    fn language_dispatch_empty_languages_provider() {
        struct NoLang;
        impl ToolingProvider for NoLang {
            fn name(&self) -> &'static str {
                "nolang"
            }
            fn languages(&self) -> &'static [&'static str] {
                &[]
            }
            fn resolve(&self, _ctx: &SymbolContext) -> anyhow::Result<ResolvedSource> {
                Ok(hit("nolang"))
            }
        }
        // With a language set, the empty-languages provider is ineligible;
        // the python-matching provider alone is attempted.
        let mut r = Resolver::new();
        r.add(NoLang);
        r.add(StubProvider { name: "second" });
        let mut c = ctx();
        c.language = Some("stub".to_string());
        let out = r.resolve_traced(&c).unwrap();
        assert_eq!(out.source.file, PathBuf::from("/second.rs"));
        assert_eq!(out.trace.attempts.len(), 1);
        assert_eq!(out.trace.attempts[0].provider, "second");

        // With no language, the empty-languages provider IS still tried
        // (and hits first, in chain order).
        let mut c = ctx();
        c.language = None;
        let out = r.resolve_traced(&c).unwrap();
        assert_eq!(out.source.file, PathBuf::from("/nolang.rs"));
        assert_eq!(out.trace.attempts.len(), 1);
        assert_eq!(out.trace.attempts[0].provider, "nolang");
    }

    /// (011-01) A python context in a chain headed by the REAL
    /// CargoProvider never probes cargo (no "no Cargo.toml" confusion):
    /// the trace contains exactly one attempt — python's. A probed cargo
    /// provider would have left its own miss entry first.
    #[test]
    fn python_context_never_reaches_cargo_provider() {
        let mut r = Resolver::new();
        r.add(CargoProvider::new());
        r.add(SpyProvider {
            name: "python",
            langs: &["python"],
            probed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        let mut c = ctx(); // workspace_root /tmp: no Cargo.toml
        c.language = Some("python".to_string());
        let out = r.resolve_traced(&c).unwrap();
        assert_eq!(out.source.file, PathBuf::from("/python.rs"));
        assert_eq!(out.trace.attempts.len(), 1);
        assert_eq!(out.trace.attempts[0].provider, "python");
    }

    /// (011-01) The all-miss error names only the ELIGIBLE providers.
    #[test]
    fn all_miss_error_names_eligible_providers() {
        struct RustOnly;
        impl ToolingProvider for RustOnly {
            fn name(&self) -> &'static str {
                "rustonly"
            }
            fn languages(&self) -> &'static [&'static str] {
                &["rust"]
            }
            fn resolve(&self, _ctx: &SymbolContext) -> anyhow::Result<ResolvedSource> {
                anyhow::bail!("no crate")
            }
        }
        let mut r = Resolver::new();
        r.add(RustOnly);
        r.add(MissProvider);
        let mut c = ctx();
        c.language = Some("rust".to_string());
        let msg = r.resolve_traced(&c).unwrap_err().to_string();
        assert!(msg.contains("rustonly"), "msg: {msg}");
        assert!(!msg.contains("miss"), "ineligible provider named: {msg}");
        assert!(msg.contains("tried 1 provider"), "msg: {msg}");
    }
}

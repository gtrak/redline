//! Grammar registry: one module that pins all grammars + highlight
//! queries. All tree-sitter API churn is isolated here (plan decision).
//!
//! Maps file extensions → `LanguageId`; builds one `HighlightConfiguration`
//! per language at startup; plain-text fallback for unknown extensions.

use std::collections::HashMap;

use tree_sitter::Language;
use tree_sitter_highlight::HighlightConfiguration;

use crate::highlight::HIGHLIGHT_FACES;

/// Identifies a supported language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LanguageId {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
    C,
    Cpp,
    Toml,
    Json,
    Yaml,
    Bash,
    Markdown,
    Java,
    CSharp,
    Ruby,
    Scheme,
    Clojure,
    /// Plain-text fallback (no highlighting).
    Plain,
}

impl LanguageId {
    /// Display name (used in the store for language identification and
    /// debugging) — read from the descriptor table (`language.rs`), the
    /// single source of truth for the per-language facts.
    pub fn name(self) -> &'static str {
        crate::language::spec(self).name
    }
}

/// Extension → language mapping, built from the descriptor table's
/// `extensions` rows (not exhaustive; unknown → Plain via
/// `resolve_language`). The hand-synced per-language insert list is
/// gone — a new language row brings its extensions with it.
fn ext_map() -> HashMap<&'static str, LanguageId> {
    crate::language::LANGUAGES
        .iter()
        .flat_map(|spec| spec.extensions.iter().map(|ext| (*ext, spec.id)))
        .collect()
}

/// Extract the file extension (without the dot) from a path.
/// Returns `None` for paths with no extension, a trailing dot, or a
/// dotfile (e.g. `.gitignore`).
pub fn file_extension(path: &str) -> Option<&str> {
    let filename = path.rsplit('/').next()?;
    // Find the last dot; if it's at position 0 the filename is a dotfile
    // (no extension). If there's no dot at all, there's no extension.
    let last_dot = filename.rfind('.')?;
    if last_dot == 0 {
        return None;
    }
    let ext = &filename[last_dot + 1..];
    if ext.is_empty() {
        None
    } else {
        Some(ext)
    }
}

/// Extension → language, resolved from the shared static map (no registry
/// instance needed). Used by the symbol indexer, which maps a file path to
/// its language without holding a `GrammarRegistry`.
fn ext_map_static() -> &'static HashMap<&'static str, LanguageId> {
    use std::sync::OnceLock;
    static MAP: OnceLock<HashMap<&'static str, LanguageId>> = OnceLock::new();
    MAP.get_or_init(ext_map)
}

/// The language for a file path (extension lookup, plain fallback) as a
/// free function (the registry's `language_for` uses the same map).
pub fn resolve_language(path: &str) -> LanguageId {
    file_extension(path)
        .and_then(|ext| ext_map_static().get(ext))
        .copied()
        .unwrap_or(LanguageId::Plain)
}

/// The full grammar registry: one `HighlightConfiguration` per language,
/// plus the extension→language map. Built once at startup; the
/// `HighlightConfiguration` is `Send + Sync` and immutable after
/// `configure`, so it can be shared across threads.
pub struct GrammarRegistry {
    exts: HashMap<&'static str, LanguageId>,
    /// One config per language; `None` when the config failed to build
    /// (query parse error, ABI mismatch) or for `LanguageId::Plain`.
    configs: HashMap<LanguageId, Option<HighlightConfiguration>>,
}

impl GrammarRegistry {
    /// Build the registry: one `HighlightConfiguration` per grammar-
    /// bearing row of the descriptor table (`language.rs`). The table
    /// row carries the grammar + highlight/injections/locals queries —
    /// the old 18-arm match (and its hand-synced `highlight_query_for`
    /// twin, now deleted) are gone: a grammar re-pin touches exactly one
    /// row, and every consumer reads it. Failures are logged and the
    /// language falls back to plain text at render time.
    pub fn build() -> Self {
        let exts = ext_map();
        let mut configs = HashMap::new();

        for spec in &crate::language::LANGUAGES {
            let id = spec.id;
            // Single source of truth for the locals fact: `build` reads
            // the row's `locals_query`, and `has_locals_queries` reads
            // the same row, so a drift between what the registry uses
            // and what `has_locals_queries` reports is impossible (C13).
            let cfg = spec.grammar.and_then(|grammar| {
                Self::build_config(
                    grammar(),
                    spec.name,
                    spec
                        .highlight_query
                        .expect("grammar-bearing rows carry a highlight query (sync test)"),
                    spec.injections_query,
                    spec.locals_query,
                )
            });
            if cfg.is_none() && spec.grammar.is_some() {
                tracing::warn!("highlight config failed for {id:?}");
            }
            configs.insert(id, cfg);
        }

        Self { exts, configs }
    }

    fn build_config(
        lang: Language,
        name: &str,
        highlights: &'static str,
        injections: &'static str,
        locals: &'static str,
    ) -> Option<HighlightConfiguration> {
        let mut c =
            HighlightConfiguration::new(lang, name, highlights, injections, locals).ok()?;
        c.configure(HIGHLIGHT_FACES);
        Some(c)
    }

    /// The language for a file path (extension lookup, plain fallback).
    pub fn language_for(&self, path: &str) -> LanguageId {
        file_extension(path)
            .and_then(|ext| self.exts.get(ext))
            .copied()
            .unwrap_or(LanguageId::Plain)
    }

    /// The `HighlightConfiguration` for a language; `None` for plain
    /// text or when the config failed to build.
    pub fn config(&self, id: LanguageId) -> Option<&HighlightConfiguration> {
        self.configs.get(&id).and_then(|c| c.as_ref())
    }
}

/// True when the registry's `build_config` for `id` passes a non-empty
/// locals query. Reads the descriptor-table row `build()` uses, so the
/// C13 pin guarantees agreement by construction: the incremental reuse
/// pipeline is byte-identical to the full `Highlighter` **only** when
/// the locals query is empty, and adding a locals query to a reuse
/// language (or changing one for any language) fails the C13 test
/// against `highlight::supports_reuse`.
#[allow(dead_code)] // used by the C13 agreement test in highlight.rs
pub(crate) fn has_locals_queries(id: LanguageId) -> bool {
    !crate::language::spec(id).locals_query.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_map_covers_all_19_languages() {
        let reg = GrammarRegistry::build();
        assert_eq!(reg.language_for("foo.rs"), LanguageId::Rust);
        assert_eq!(reg.language_for("foo.ts"), LanguageId::TypeScript);
        assert_eq!(reg.language_for("foo.tsx"), LanguageId::Tsx);
        assert_eq!(reg.language_for("foo.js"), LanguageId::JavaScript);
        assert_eq!(reg.language_for("foo.py"), LanguageId::Python);
        assert_eq!(reg.language_for("foo.go"), LanguageId::Go);
        assert_eq!(reg.language_for("foo.c"), LanguageId::C);
        assert_eq!(reg.language_for("foo.cpp"), LanguageId::Cpp);
        assert_eq!(reg.language_for("foo.toml"), LanguageId::Toml);
        assert_eq!(reg.language_for("foo.json"), LanguageId::Json);
        assert_eq!(reg.language_for("foo.yaml"), LanguageId::Yaml);
        assert_eq!(reg.language_for("foo.sh"), LanguageId::Bash);
        assert_eq!(reg.language_for("foo.md"), LanguageId::Markdown);
        assert_eq!(reg.language_for("Foo.java"), LanguageId::Java);
        assert_eq!(reg.language_for("Foo.cs"), LanguageId::CSharp);
        assert_eq!(reg.language_for("foo.rb"), LanguageId::Ruby);
        assert_eq!(reg.language_for("foo.scm"), LanguageId::Scheme);
        assert_eq!(reg.language_for("foo.sld"), LanguageId::Scheme);
        assert_eq!(reg.language_for("foo.clj"), LanguageId::Clojure);
        assert_eq!(reg.language_for("foo.cljs"), LanguageId::Clojure);
        assert_eq!(reg.language_for("foo.cljc"), LanguageId::Clojure);
    }

    #[test]
    fn unknown_extension_falls_back_to_plain() {
        let reg = GrammarRegistry::build();
        assert_eq!(reg.language_for("foo.xyz"), LanguageId::Plain);
        assert_eq!(reg.language_for("foo"), LanguageId::Plain);
        assert_eq!(reg.language_for("foo."), LanguageId::Plain);
    }

    #[test]
    fn all_grammars_have_configs() {
        let reg = GrammarRegistry::build();
        for spec in &crate::language::LANGUAGES {
            if spec.grammar.is_some() {
                assert!(
                    reg.config(spec.id).is_some(),
                    "{id:?} should have a highlight config",
                    id = spec.id
                );
            } else {
                assert!(
                    reg.config(spec.id).is_none(),
                    "{id:?} has no grammar and no config",
                    id = spec.id
                );
            }
        }
    }

    /// ABI-pinning guard (001/007 lesson): every registry language's
    /// grammar must `set_language` against the SINGLE pinned
    /// tree-sitter 0.25.10 runtime (ABI window 13..=15). A grammar
    /// built outside that window (e.g. the known-bad stragglers
    /// `tree-sitter-md` 0.5.2+ wanting the 0.26 window) would
    /// otherwise silently fall back to plain text at render time.
    #[test]
    fn all_grammars_set_language_succeeds() {
        let mut parser = tree_sitter::Parser::new();
        // The table's grammar column is the single pin every consumer
        // (registry build, extraction, reuse) reads — pinning it here
        // covers all of them (the old guard only covered
        // `queries::language_for`'s copy of the pin).
        for spec in &crate::language::LANGUAGES {
            let Some(grammar) = spec.grammar else { continue };
            let lang = grammar();
            assert!(
                parser.set_language(&lang).is_ok(),
                "{id:?}: grammar ABI incompatible with the pinned runtime",
                id = spec.id
            );
        }
    }

    #[test]
    fn file_extension_extraction() {
        assert_eq!(file_extension("foo.rs"), Some("rs"));
        assert_eq!(file_extension("src/lib.rs"), Some("rs"));
        assert_eq!(file_extension("foo.tar.gz"), Some("gz"));
        assert_eq!(file_extension("foo"), None);
        assert_eq!(file_extension("foo."), None);
        assert_eq!(file_extension(".gitignore"), None);
        assert_eq!(file_extension("/a/b/c.yaml"), Some("yaml"));
    }

    #[test]
    fn language_for_uses_extension() {
        let reg = GrammarRegistry::build();
        assert_eq!(reg.language_for("a/b/c.rs"), LanguageId::Rust);
        assert_eq!(reg.language_for("a/b/c.unknown"), LanguageId::Plain);
    }
}

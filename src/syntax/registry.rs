//! Grammar registry: one module that pins all grammars + highlight
//! queries. All tree-sitter API churn is isolated here (plan decision).
//!
//! Maps file extensions → `LanguageId`; builds one `HighlightConfiguration`
//! per language at startup; plain-text fallback for unknown extensions.

use std::collections::HashMap;

use tree_sitter::Language;
use tree_sitter_highlight::HighlightConfiguration;

use crate::syntax::highlight::HIGHLIGHT_FACES;

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
    /// debugging).
    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::JavaScript => "javascript",
            Self::Python => "python",
            Self::Go => "go",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Toml => "toml",
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Bash => "bash",
            Self::Markdown => "markdown",
            Self::Java => "java",
            Self::CSharp => "csharp",
            Self::Ruby => "ruby",
            Self::Scheme => "scheme",
            Self::Clojure => "clojure",
            Self::Plain => "plain",
        }
    }

    /// All non-plain languages (used to build the registry).
    pub const ALL: &[LanguageId] = &[
        Self::Rust,
        Self::TypeScript,
        Self::Tsx,
        Self::JavaScript,
        Self::Python,
        Self::Go,
        Self::C,
        Self::Cpp,
        Self::Toml,
        Self::Json,
        Self::Yaml,
        Self::Bash,
        Self::Markdown,
        Self::Java,
        Self::CSharp,
        Self::Ruby,
        Self::Scheme,
        Self::Clojure,
    ];
}

/// Extension → language mapping (not exhaustive; unknown → Plain).
fn ext_map() -> HashMap<&'static str, LanguageId> {
    let mut m = HashMap::new();
    // Rust
    m.insert("rs", LanguageId::Rust);
    m.insert("rsi", LanguageId::Rust);
    // TypeScript / TSX
    m.insert("ts", LanguageId::TypeScript);
    m.insert("tsx", LanguageId::Tsx);
    // JavaScript
    m.insert("js", LanguageId::JavaScript);
    m.insert("jsx", LanguageId::JavaScript);
    m.insert("mjs", LanguageId::JavaScript);
    m.insert("cjs", LanguageId::JavaScript);
    // Python
    m.insert("py", LanguageId::Python);
    m.insert("pyi", LanguageId::Python);
    // Go
    m.insert("go", LanguageId::Go);
    // C
    m.insert("c", LanguageId::C);
    m.insert("h", LanguageId::C);
    // C++
    m.insert("cpp", LanguageId::Cpp);
    m.insert("cc", LanguageId::Cpp);
    m.insert("cxx", LanguageId::Cpp);
    m.insert("hpp", LanguageId::Cpp);
    m.insert("hh", LanguageId::Cpp);
    m.insert("hxx", LanguageId::Cpp);
    // TOML
    m.insert("toml", LanguageId::Toml);
    m.insert("ini", LanguageId::Toml);
    m.insert("conf", LanguageId::Toml);
    // JSON
    m.insert("json", LanguageId::Json);
    m.insert("jsonc", LanguageId::Json);
    // YAML
    m.insert("yaml", LanguageId::Yaml);
    m.insert("yml", LanguageId::Yaml);
    // Bash
    m.insert("sh", LanguageId::Bash);
    m.insert("bash", LanguageId::Bash);
    m.insert("zsh", LanguageId::Bash);
    // Markdown
    m.insert("md", LanguageId::Markdown);
    m.insert("markdown", LanguageId::Markdown);
    m.insert("mdx", LanguageId::Markdown);
    // Java
    m.insert("java", LanguageId::Java);
    // C#
    m.insert("cs", LanguageId::CSharp);
    // Ruby
    m.insert("rb", LanguageId::Ruby);
    // Scheme (the lisp-family landing — see docs/language-coverage.md
    // gap 8 for the family choice rationale)
    m.insert("scm", LanguageId::Scheme);
    m.insert("ss", LanguageId::Scheme);
    m.insert("sls", LanguageId::Scheme);
    m.insert("sld", LanguageId::Scheme);
    // Clojure (the runtime-bump lane — the first 0.25-generation grammar)
    m.insert("clj", LanguageId::Clojure);
    m.insert("cljs", LanguageId::Clojure);
    m.insert("cljc", LanguageId::Clojure);
    m
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
    /// Build the registry: one `HighlightConfiguration` per language in
    /// `LanguageId::ALL`. Failures are logged and the language falls
    /// back to plain text at render time.
    pub fn build() -> Self {
        let exts = ext_map();
        let mut configs = HashMap::new();

        for id in LanguageId::ALL {
            // Single source of truth for the locals fact: `build` no
            // longer carries it positionally per arm, so a drift between
            // what the registry uses and what `has_locals_queries`
            // reports is impossible (C13).
            let locals = locals_query(*id);
            let cfg = match id {
                LanguageId::Rust => Self::build_config(
                    Language::from(tree_sitter_rust::LANGUAGE),
                    "rust",
                    tree_sitter_rust::HIGHLIGHTS_QUERY,
                    tree_sitter_rust::INJECTIONS_QUERY,
                    locals,
                ),
                LanguageId::TypeScript => Self::build_config(
                    Language::from(tree_sitter_typescript::LANGUAGE_TYPESCRIPT),
                    "typescript",
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                    "",
                    locals,
                ),
                LanguageId::Tsx => Self::build_config(
                    Language::from(tree_sitter_typescript::LANGUAGE_TSX),
                    "tsx",
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                    "",
                    locals,
                ),
                LanguageId::JavaScript => Self::build_config(
                    Language::from(tree_sitter_javascript::LANGUAGE),
                    "javascript",
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::INJECTIONS_QUERY,
                    locals,
                ),
                LanguageId::Python => Self::build_config(
                    Language::from(tree_sitter_python::LANGUAGE),
                    "python",
                    tree_sitter_python::HIGHLIGHTS_QUERY,
                    "",
                    locals,
                ),
                LanguageId::Go => Self::build_config(
                    Language::from(tree_sitter_go::LANGUAGE),
                    "go",
                    tree_sitter_go::HIGHLIGHTS_QUERY,
                    "",
                    locals,
                ),
                LanguageId::C => Self::build_config(
                    Language::from(tree_sitter_c::LANGUAGE),
                    "c",
                    tree_sitter_c::HIGHLIGHT_QUERY,
                    "",
                    locals,
                ),
                LanguageId::Cpp => Self::build_config(
                    Language::from(tree_sitter_cpp::LANGUAGE),
                    "cpp",
                    tree_sitter_cpp::HIGHLIGHT_QUERY,
                    "",
                    locals,
                ),
                LanguageId::Toml => Self::build_config(
                    Language::from(tree_sitter_toml_ng::LANGUAGE),
                    "toml",
                    tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
                    "",
                    locals,
                ),
                LanguageId::Json => Self::build_config(
                    Language::from(tree_sitter_json::LANGUAGE),
                    "json",
                    tree_sitter_json::HIGHLIGHTS_QUERY,
                    "",
                    locals,
                ),
                LanguageId::Yaml => Self::build_config(
                    Language::from(tree_sitter_yaml::LANGUAGE),
                    "yaml",
                    tree_sitter_yaml::HIGHLIGHTS_QUERY,
                    "",
                    locals,
                ),
                LanguageId::Bash => Self::build_config(
                    Language::from(tree_sitter_bash::LANGUAGE),
                    "bash",
                    tree_sitter_bash::HIGHLIGHT_QUERY,
                    "",
                    locals,
                ),
                LanguageId::Markdown => Self::build_config(
                    Language::from(tree_sitter_md::LANGUAGE),
                    "markdown",
                    tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
                    tree_sitter_md::INJECTION_QUERY_BLOCK,
                    locals,
                ),
                LanguageId::Java => Self::build_config(
                    Language::from(tree_sitter_java::LANGUAGE),
                    "java",
                    tree_sitter_java::HIGHLIGHTS_QUERY,
                    "",
                    locals,
                ),
                LanguageId::CSharp => Self::build_config(
                    Language::from(tree_sitter_c_sharp::LANGUAGE),
                    "csharp",
                    // The pinned crate ships `queries/highlights.scm` but
                    // does not export a `HIGHLIGHTS_QUERY` constant, so the
                    // query is vendored verbatim (checksum-pinned at copy
                    // time) and `include_str!`ed from `queries.rs`.
                    crate::syntax::queries::C_SHARP_HIGHLIGHTS,
                    "",
                    locals,
                ),
                LanguageId::Ruby => Self::build_config(
                    Language::from(tree_sitter_ruby::LANGUAGE),
                    "ruby",
                    tree_sitter_ruby::HIGHLIGHTS_QUERY,
                    "",
                    locals,
                ),
                LanguageId::Scheme => Self::build_config(
                    Language::from(tree_sitter_scheme::LANGUAGE),
                    "scheme",
                    tree_sitter_scheme::HIGHLIGHTS_QUERY,
                    "",
                    locals,
                ),
                LanguageId::Clojure => Self::build_config(
                    Language::from(tree_sitter_clojure::LANGUAGE),
                    "clojure",
                    // The pinned crate ships `grammar-src/queries/
                    // highlights.scm` but exports no highlights constant,
                    // so the query is vendored verbatim (sha256-pinned at
                    // copy time) and `include_str!`ed from `queries.rs` —
                    // the same pattern as C#.
                    crate::syntax::queries::CLOJURE_HIGHLIGHTS,
                    "",
                    locals,
                ),
                LanguageId::Plain => None,
            };
            if cfg.is_none() && *id != LanguageId::Plain {
                tracing::warn!("highlight config failed for {id:?}");
            }
            configs.insert(*id, cfg);
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

/// The locals query string `build()` passes to `build_config` for `id`
/// — the single source of truth for which languages track local
/// variable scopes (TypeScript, TSX, JavaScript, Ruby pass their crate's
/// `LOCALS_QUERY`; every other language passes `""`). `build()` and
/// `has_locals_queries` both read this, so the C13 pin and the registry
/// cannot drift apart: a locals query added here fails the pin.
fn locals_query(id: LanguageId) -> &'static str {
    match id {
        LanguageId::TypeScript => tree_sitter_typescript::LOCALS_QUERY,
        LanguageId::Tsx => tree_sitter_typescript::LOCALS_QUERY,
        LanguageId::JavaScript => tree_sitter_javascript::LOCALS_QUERY,
        LanguageId::Ruby => tree_sitter_ruby::LOCALS_QUERY,
        _ => "",
    }
}

/// True when the registry's `build_config` for `id` passes a non-empty
/// locals query. Reads the exact fact `build()` uses (via
/// `locals_query`), so the pin guarantees agreement by construction:
/// the incremental reuse pipeline is byte-identical to the full
/// `Highlighter` **only** when the locals query is empty, and adding a
/// locals query to a reuse language (or changing one for any language)
/// fails the C13 test against `highlight::supports_reuse`.
#[allow(dead_code)] // used by the C13 agreement test in highlight.rs
pub(crate) fn has_locals_queries(id: LanguageId) -> bool {
    !locals_query(id).is_empty()
}

/// The highlight query string for a language (the same `&'static str`
/// constants `build` uses for the `HighlightConfiguration`s); `None` for
/// plain text. Used by issue 06's reference filtering to build a
/// token-class (comment/string) byte-range query without re-pinning the
/// grammar constants a second time.
pub fn highlight_query_for(lang: LanguageId) -> Option<&'static str> {
    Some(match lang {
        LanguageId::Rust => tree_sitter_rust::HIGHLIGHTS_QUERY,
        LanguageId::TypeScript => tree_sitter_typescript::HIGHLIGHTS_QUERY,
        LanguageId::Tsx => tree_sitter_typescript::HIGHLIGHTS_QUERY,
        LanguageId::JavaScript => tree_sitter_javascript::HIGHLIGHT_QUERY,
        LanguageId::Python => tree_sitter_python::HIGHLIGHTS_QUERY,
        LanguageId::Go => tree_sitter_go::HIGHLIGHTS_QUERY,
        LanguageId::C => tree_sitter_c::HIGHLIGHT_QUERY,
        LanguageId::Cpp => tree_sitter_cpp::HIGHLIGHT_QUERY,
        LanguageId::Toml => tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
        LanguageId::Json => tree_sitter_json::HIGHLIGHTS_QUERY,
        LanguageId::Yaml => tree_sitter_yaml::HIGHLIGHTS_QUERY,
        LanguageId::Bash => tree_sitter_bash::HIGHLIGHT_QUERY,
        LanguageId::Markdown => tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
        LanguageId::Java => tree_sitter_java::HIGHLIGHTS_QUERY,
        LanguageId::CSharp => crate::syntax::queries::C_SHARP_HIGHLIGHTS,
        LanguageId::Ruby => tree_sitter_ruby::HIGHLIGHTS_QUERY,
        LanguageId::Scheme => tree_sitter_scheme::HIGHLIGHTS_QUERY,
        LanguageId::Clojure => crate::syntax::queries::CLOJURE_HIGHLIGHTS,
        LanguageId::Plain => return None,
    })
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
        for id in LanguageId::ALL {
            assert!(
                reg.config(*id).is_some(),
                "{id:?} should have a highlight config"
            );
        }
        assert!(reg.config(LanguageId::Plain).is_none());
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
        for id in LanguageId::ALL {
            let lang =
                crate::syntax::queries::language_for(*id).expect("grammar for {id:?}");
            assert!(
                parser.set_language(&lang).is_ok(),
                "{id:?}: grammar ABI incompatible with the pinned runtime"
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

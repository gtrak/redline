//! Clojure namespace semantics for M-. (issue-language-aware-symbols,
//! Part 2): the `ns`-form alias map and the namespace → file convention.
//!
//! A namespaced var `jwks/fetch-issuer-info` is NOT a package or a file:
//! `jwks` is a NAMESPACE ALIAS declared in the file's top-level `ns`
//! form (`(:require [some.ns :as jwks])`), and `fetch-issuer-info` is the
//! VAR in that namespace. The `::jwks/local` form is the auto-resolve
//! KEYWORD spelling of the same reference (`:` / `::` introduce keywords;
//! the name is still the last `/` segment) — probe-verified against the
//! pinned tree-sitter-clojure 0.1.0 in `tests/probe_langsym.rs` (the
//! `kwd_lit` children are `::` + `kwd_ns` + `kwd_name`, exactly mirroring
//! the `sym_lit` `sym_ns` + `/` + `sym_name` shape).
//!
//! The namespace → file mapping is a CONVENTION, verified against real
//! Clojure project layouts (kaocha, rewrite-clj-cli), not assumed:
//! `kaocha.core-ext` lives in `src/kaocha/core_ext.clj`,
//! `kaocha.plugin.alpha.info` in `src/kaocha/plugin/alpha/info.clj`,
//! `rewrite-clj.cli` in `src/rewrite_clj/cli.clj`, and
//! `kaocha.plugin.capture-output` in
//! `src/kaocha/plugin/capture_output.cljc` — dots become directory
//! separators, HYPHENS become underscores, and the extension is one of
//! `.clj` / `.cljc` / `.cljs` (kaocha ships both `.clj` and `.cljc`
//! files; the registry row accepts all three). The `src/` (or
//! `test/`) root prefix is project-local, so the convention yields
//! path TAILS that the caller matches against the project's known files
//! — never an absolute guess.

use tree_sitter::Parser;

/// Parse `source` with the pinned Clojure grammar (the table row is the
/// one grammar pin — `language::spec` reads it).
fn parse_clojure(source: &str) -> Option<tree_sitter::Tree> {
    let grammar = crate::language::spec(crate::registry::LanguageId::Clojure).grammar?;
    let mut parser = Parser::new();
    parser.set_language(&grammar()).unwrap();
    parser.parse(source.as_bytes(), None)
}

/// The namespace→alias map declared by the file's top-level `ns` form.
///
/// Every `:require` entry participates:
/// - `[some.ns :as jwks]` → `("jwks", "some.ns")` (the alias spelling);
/// - a required namespace with no `:as` (`some.ns` or `[some.ns :refer
///   [f]]`) → an IDENTITY entry `("some.ns", "some.ns")` — a
///   `some.ns/var` reference carries the namespace itself, not an alias.
///
/// `None` when the file has no parseable top-level `ns` form (a file
/// that does not name a namespace cannot declare aliases — the caller
/// must NOT guess: an unresolvable alias is flagged, not fabricated).
/// Only the FIRST top-level `ns` form counts (a file legally carries
/// exactly one).
pub fn ns_aliases(source: &str) -> Option<Vec<(String, String)>> {
    let tree = parse_clojure(source)?;
    let root = tree.root_node();
    // The top-level `ns` form: a `list_lit` whose head `sym_lit` names
    // `ns` and whose third value is the options `list_lit` (probe:
    // `(ns app.core (:require …))` →
    // list_lit[ `(`, sym_lit ns, sym_lit app.core, list_lit, `)` ]).
    let mut i = 0;
    while let Some(list) = root.child(i) {
        i += 1;
        if list.kind() != "list_lit" || list.child_count() < 5 {
            continue;
        }
        let Some(head) = list.child(1) else { continue };
        if head.kind() != "sym_lit" || head.utf8_text(source.as_bytes()) != Ok("ns") {
            continue;
        }
        // Scan the top-level option lists (child 3+) for the `:require`
        // one (its first value is the `:require` keyword; `(ns foo
        // (:use …) (:require …))` orders options after `:require`).
        let require = (3..list.child_count())
            .filter_map(|ci| list.child(ci))
            .find(|r| {
                r.kind() == "list_lit"
                    && r.child(1).is_some_and(|k| {
                        k.kind() == "kwd_lit"
                            && k.utf8_text(source.as_bytes()) == Ok(":require")
                    })
            });
        let Some(require) = require else { continue };
        let mut out: Vec<(String, String)> = Vec::new();
        let mut ci = 0;
        while let Some(entry) = require.child(ci) {
            ci += 1;
            if entry.kind() != "vec_lit" {
                continue;
            }
            let mut vals: Vec<&str> = Vec::new();
            let mut vi = 0;
            while let Some(v) = entry.child(vi) {
                vi += 1;
                // The bracket nodes (`[` / `]`) are CHILDREN of the
                // vec_lit in the pinned grammar — skip punctuation, keep
                // the value nodes only.
                if v.kind() == "[" || v.kind() == "]" {
                    continue;
                }
                if let Ok(t) = v.utf8_text(source.as_bytes()) {
                    vals.push(t);
                }
            }
            let Some(ns_name) = vals.first() else {
                continue;
            };
            let ns_name = ns_name.to_string();
            // `[some.ns :as jwks]` → the alias; `[some.ns :refer …]` or
            // a bare `[some.ns]` → the identity entry.
            if let Some(pos) = vals.iter().position(|v| *v == ":as")
                && let Some(alias) = vals.get(pos + 1)
            {
                out.push((alias.to_string(), ns_name));
            } else {
                out.push((ns_name.clone(), ns_name));
            }
        }
        return Some(out);
    }
    None
}

/// The namespace → file path TAILS for namespace `ns`: dots become `/`,
/// hyphens become `_`, and each of the three Clojure source extensions
/// completes the tail — `some.ns` → `some/ns.clj`, `some/ns.cljc`,
/// `some/ns.cljs`. The caller matches these tails against the project's
/// known files (the root prefix — `src/`, `test/`, or the root itself —
/// is project-local and must not be guessed).
pub fn namespace_file_tails(ns: &str) -> Vec<String> {
    let path: String = ns
        .split('.')
        .map(|seg| seg.replace('-', "_"))
        .collect::<Vec<_>>()
        .join("/");
    [".clj", ".cljc", ".cljs"]
        .iter()
        .map(|ext| format!("{path}{ext}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
(ns app.core
  (:require [some.ns :as jwks]
            [other.ns]
            [util.ns :refer [helper]]))
"#;

    #[test]
    fn ns_form_alias_and_identity_entries() {
        let aliases = ns_aliases(FIXTURE).expect("fixture has a top-level ns form");
        // The `:as` alias and the two identity entries (a required
        // namespace without `:as` is addressable by its own name).
        assert!(aliases.contains(&("jwks".to_string(), "some.ns".to_string())));
        assert!(aliases.contains(&("other.ns".to_string(), "other.ns".to_string())));
        assert!(aliases.contains(&("util.ns".to_string(), "util.ns".to_string())));
        assert_eq!(aliases.len(), 3, "no other entries: {aliases:?}");
    }

    #[test]
    fn no_ns_form_is_none_not_empty() {
        // A file without an `ns` form (a REPL script / data file): the
        // honest None (the caller flags an unresolvable alias, never
        // fabricates a namespace).
        assert_eq!(ns_aliases("(do (println 1))\n"), None);
        assert_eq!(ns_aliases(""), None);
    }

    #[test]
    fn namespace_file_tails_convention() {
        // The real-layout convention (kaocha / rewrite-clj-cli probes):
        // dots → directories, hyphens → underscores, all three
        // source extensions.
        assert_eq!(
            namespace_file_tails("some.ns"),
            vec![
                "some/ns.clj".to_string(),
                "some/ns.cljc".to_string(),
                "some/ns.cljs".to_string(),
            ]
        );
        assert_eq!(
            namespace_file_tails("kaocha.plugin.capture-output"),
            vec![
                "kaocha/plugin/capture_output.clj".to_string(),
                "kaocha/plugin/capture_output.cljc".to_string(),
                "kaocha/plugin/capture_output.cljs".to_string(),
            ]
        );
    }
}

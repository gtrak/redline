//! plan-017: the CONVENTIONS MECHANISM — per-language import/alias
//! conventions as a table (name → relative path tails) plus a query
//! (the file's import/alias forms), consulted by ONE shared app path.
//!
//! The layers (PLAN.md): tooling providers are authoritative when
//! available; CONVENTION (this module) covers the rest — pure and
//! offline (no toolchain, no network); a new language is a table row
//! here + its extractor, not a branch in the app's M-. path. The
//! mechanism COMPOSES with the index: the convention places a name in a
//! file, and the app narrows the index's existing name-keyed candidates
//! to that file — never a second resolver engine.
//!
//! Wired so far:
//! - Clojure (issue-language-aware-symbols Part 2, the working shape
//!   this module generalizes): the `ns`-form alias map
//!   (`clojure::ns_aliases`) + the namespace → file tails
//!   (`clojure::namespace_file_tails` — dots → `/`, hyphens → `_`,
//!   `.clj` / `.cljc` / `.cljs`);
//! - Java (plan-017 issue 03): the single-type import map
//!   (`java::import_class_bindings`) + the JLS `a.b.C` → `a/b/C.java`
//!   tails (`java::class_file_tails`);
//! - C / C++ (plan-017 issue 04): the QUOTED `#include "a/b.h"` → the
//!   include's path as a tail (`c_cpp::convention_name` / `convention_tails`
//!   — the ISO/GCC quoted-include search order; relative to the including
//!   file / source roots). ANGLE `#include <a/b.h>` is the `-I` limit: a
//!   different grammar node (`system_lib_string`), never a tail — the name
//!   stays on the B5 named-`unresolved` flag (no tooling provider for
//!   C/C++), never a confident same-named-file jump;
//! - Go (plan-017 issue 05): the in-module import path → the
//!   module-relative package DIRECTORY (`go::reference_convention_name`
//!   / `local_dir_tails` — Go spec "Import Declarations" + "Module paths
//!   and package paths"). OFFLINE: the module path is a line in the
//!   repository's own `go.mod` (`module github.com/x/y` —
//!   `go::module_path_for_file` reads the nearest go.mod walking up from
//!   the file, bounded to the workspace), not a toolchain call. An
//!   import UNDER the module path maps to a directory tail; an import
//!   OUTSIDE it is a third-party dependency — FLAGGED (the `Err` arm),
//!   never a guess (the same rule as the C/C++ angle include). Cross-
//!   module / download resolution stays on the Go tooling provider
//!   (toolchain-gated — audit F4).
//!
//! Not wired (the audit's flag rows — a table row would be a guess):
//! C# (no directory convention), Scheme (library layout
//! implementation-defined), Ruby bare `require` ($LOAD_PATH), C++
//! semantic forms. Their M-. stays on the name-keyed index / the named
//! unresolved flag.

use crate::registry::LanguageId;
use std::path::Path;

/// The CONVENTION NAME for the reference the M-. extraction produced at
/// the point (`path_token`, with its last segment `ident`): the name
/// whose file tails narrow the index's name-keyed candidates.
///
/// `project_root` + `rel` locate the buffer WITHIN the workspace (the
/// Go convention's module discovery reads the nearest `go.mod` walking
/// up from `project_root/rel`, bounded to the root — the other
/// languages' conventions are project-agnostic and ignore them).
///
/// - `Ok(Some(name))` — the convention applies; the name feeds
///   [`convention_file_tails`];
/// - `Ok(None)` — no convention applies (the language has no
///   import/alias construct, or the file declares nothing for this
///   reference, or the reference is not a namespaced / qualified
///   shape): the caller keeps the bare name-keyed lookup byte-for-byte;
/// - `Err(name)` — the reference IS a namespaced / qualified shape the
///   file does not declare (Clojure: a dotless alias absent from the
///   file's `ns` form; Go: an import OUTSIDE the module path — a
///   third-party dependency): FLAGGED, never guessed — the caller
///   reports it and never jumps.
pub fn convention_name_for_reference(
    lang: LanguageId,
    source: &str,
    ident: &str,
    path_token: &str,
    project_root: &Path,
    rel: &str,
) -> Result<Option<String>, String> {
    match lang {
        LanguageId::Clojure => clojure_reference_convention_name(source, path_token),
        LanguageId::Java => Ok(java_reference_convention_name(source, ident, path_token)),
        // C / C++: the convention is the file's QUOTED includes (the path
        // token is not a namespaced shape here — the include IS the
        // reference; the narrowing set is the whole quoted-include set, so
        // `ident` plays no part). The carrier is the newline-joined set;
        // `None` when the file has no quoted include (the bare lookup
        // stands). Angle includes are excluded in the module (the `-I`
        // limit) — never a tail, never a guess.
        LanguageId::C | LanguageId::Cpp => Ok(crate::c_cpp::convention_name(source, lang)),
        // Go: the reference `pkg.Sym` → the file's import binding for
        // `pkg` → under the file's own module path (the go.mod line,
        // read offline), the module-relative package directory is the
        // convention name; outside it, `Err` (third-party — flagged,
        // never a guess).
        LanguageId::Go => {
            crate::go::reference_convention_name(source, ident, path_token, project_root, rel)
        }
        // No convention row yet: the bare name-keyed lookup stands.
        _ => Ok(None),
    }
}

/// The file path TAILS for the convention name `name`, per language
/// (Clojure namespace → its source-file tails; Java fully qualified
/// class name → its `a/b/C.java` tail; Go module-relative package
/// directory → the `dir/` tail). `None` when the language has no
/// convention row (the caller must not guess).
pub fn convention_file_tails(lang: LanguageId, name: &str) -> Option<Vec<String>> {
    match lang {
        LanguageId::Clojure => Some(crate::clojure::namespace_file_tails(name)),
        LanguageId::Java => Some(crate::java::class_file_tails(name)),
        // C / C++: one tail per quoted include (the include's path as
        // written) — the carrier the pre-step built with `convention_name`.
        LanguageId::C | LanguageId::Cpp => Some(crate::c_cpp::convention_tails(name)),
        // Go: the module-relative directory, tail-matched with its
        // trailing `/` (the files of the package directory carry it as
        // a path segment — `convention_file_matches` is the Go row's
        // match, the others' is `ends_with`).
        LanguageId::Go => Some(crate::go::local_dir_tails(name)),
        _ => None,
    }
}

/// Whether the project-relative file `file` sits in a convention tail's
/// place, per language — the pre-step's narrowing predicate:
///
/// - the FILE-tail rows (Clojure / Java / C / C++) are exact file paths
///   (a source-file tail, the include's path as written): `ends_with`
///   (the pre-step's always-behavior — every source-root placement of
///   the same relative path matches);
/// - Go's tail is a DIRECTORY (a package is many files — no single file
///   tail exists, and the package's files sit INSIDE the directory, so
///   `ends_with("sub/")` is false for every file of `sub/` — the
///   directory tail can only match by containment). The module root's
///   offset in the project is unknown to the convention (the module
///   root may be the project root or a subdirectory of it), so the
///   match is containment — the same tail-anywhere semantics the C
///   quoted-include row has (`ends_with` on the include's full path
///   matches `elsewhere/sub/thing.h` too). The trailing `/` in the tail
///   keeps `sub` from over-matching a `sub2` directory (the `mysub`
///   containment false positive is the `ends_with` row's accepted
///   class — a superset row, never a wrong file).
pub fn convention_file_matches(lang: LanguageId, tail: &str, file: &str) -> bool {
    match lang {
        LanguageId::Go => file.contains(tail),
        _ => file.ends_with(tail),
    }
}

/// Clojure: the namespace behind the reference at the point.
/// `alias/var` (and the keyword `::alias/var` — the reader marker is
/// already stripped by the M-. extraction) names the VAR `var` in the
/// NAMESPACE `alias` stands for: either an `:as` alias THIS file's
/// top-level `ns` form declares, or a FULL namespace name (a dotted
/// `alias` — an alias can never contain a `.`). A bare symbol (no `/`)
/// is not a namespaced reference — the bare lookup stands.
fn clojure_reference_convention_name(source: &str, path_token: &str) -> Result<Option<String>, String> {
    let Some((alias, var)) = path_token.rsplit_once('/') else {
        return Ok(None);
    };
    if var.is_empty() {
        return Ok(None);
    }
    if alias.contains('.') {
        // A full namespace reference — identity (no declaration needed).
        return Ok(Some(alias.to_string()));
    }
    // A dotless alias must be declared by THIS file's top-level `ns`
    // form. `None` from `ns_aliases` (no parseable ns form — a file
    // that does not name a namespace cannot declare aliases) is the
    // honest "cannot declare": flagged, never fabricated.
    match crate::clojure::ns_aliases(source) {
        Some(aliases) => match aliases
            .into_iter()
            .find(|(a, _)| a == alias)
            .map(|(_, ns)| ns)
        {
            Some(ns) => Ok(Some(ns)),
            None => Err(alias.to_string()),
        },
        None => Err(alias.to_string()),
    }
}

/// Java: the fully qualified name behind the reference at the point.
/// Two shapes, both decided (never a guess):
/// 1. a BARE simple name the file's single-type imports bind
///    (`import a.b.C;` → the bare `C` carries the package `a.b`). BARE
///    ONLY (`path_token == ident`): a qualified reference (`A.c` field
///    access, the explicit `a.b.C`) resolves through its OWN qualifier,
///    never through an import (JLS §7.5.1 — an import never shadows a
///    qualified reference);
/// 2. an EXPLICITLY-qualified reference at the point (`a.b.C` — the
///    011-06 path token carries it whole): decidable when it has the
///    JLS package-simple shape — a dotted token, every segment a bare
///    identifier, and a LOWERCASE head (the JLS naming convention, §1.3:
///    a package name is all lowercase — the guard is deliberately
///    conservative, so an unusual mixed-case package stays on the bare
///    lookup). A type-headed path (`A.c` field access) never qualifies:
///    the bare extraction stands.
fn java_reference_convention_name(source: &str, ident: &str, path_token: &str) -> Option<String> {
    // (1) The file's single-type imports, for a BARE reference only.
    // An empty `Vec` is the honest "none" — the parse itself never
    // yields `None` in practice; the `?` is the defensive "no tree"
    // shape.
    if path_token == ident
        && let Some(bindings) = crate::java::import_class_bindings(source)
        && let Some((_, fqn)) = bindings.iter().find(|(simple, _)| simple == ident)
    {
        return Some(fqn.clone());
    }
    // (2) The explicitly-qualified reference.
    if path_token != ident
        && path_token
            .split('.')
            .all(|seg| !seg.is_empty() && seg.chars().all(|c| c.is_alphanumeric() || c == '_'))
        && let Some(head) = path_token.split('.').next()
        && !head.is_empty()
        && head.chars().all(|c| c.is_ascii_lowercase() || c == '_')
    {
        return Some(path_token.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The four-argument test shim: dispatch against a NONEXISTENT
    /// workspace root — the languages with no workspace input ignore it
    /// byte-for-byte; the Go row's workspace-reading behavior is pinned
    /// in `go.rs` (tempdir fixtures) and the app's integration tests.
    fn cnr(lang: LanguageId, source: &str, ident: &str, path_token: &str) -> Result<Option<String>, String> {
        convention_name_for_reference(
            lang,
            source,
            ident,
            path_token,
            Path::new("/nonexistent-redline"),
            "f.go",
        )
    }

    const CLOJURE: &str = "(ns app.core (:require [some.ns :as jwks]))\n(jwks/fetch-issuer-info)\n";

    /// The table dispatch: the wired languages answer, the unwired ones
    /// (the audit's flag rows) decline — the bare lookup stands, never a
    /// guess.
    #[test]
    fn table_rows_dispatch_per_language() {
        // Clojure: the alias → namespace mapping (Part 2's behavior).
        assert_eq!(
            cnr(LanguageId::Clojure, CLOJURE, "fetch-issuer-info", "jwks/fetch-issuer-info"),
            Ok(Some("some.ns".to_string()))
        );
        // The full-namespace reference: identity (no declaration).
        assert_eq!(
            cnr(LanguageId::Clojure, CLOJURE, "other", "some.ns/other"),
            Ok(Some("some.ns".to_string()))
        );
        // A bare symbol: no namespaced reference.
        assert_eq!(
            cnr(LanguageId::Clojure, CLOJURE, "thunk", "thunk"),
            Ok(None)
        );
        // The undeclared dotless alias: the FLAG (never a guess).
        assert_eq!(
            cnr(LanguageId::Clojure, CLOJURE, "whatever", "unk/whatever"),
            Err("unk".to_string())
        );
        // Java: the import binding (the shared `source` is the file).
        assert_eq!(
            cnr(
                LanguageId::Java,
                "package m;\nimport a.b.C;\nclass A { void f() { C x; } }\n",
                "C",
                "C"
            ),
            Ok(Some("a.b.C".to_string()))
        );
        // Java: no import for the name, bare reference: no convention.
        assert_eq!(
            cnr(
                LanguageId::Java,
                "package m;\nimport a.b.C;\nclass A { void f() { D x; } }\n",
                "D",
                "D"
            ),
            Ok(None)
        );
        // Java: an EXPLICITLY-qualified reference with no import
        // (`a.b.C` at the point — the JLS package-simple shape,
        // lowercase head): the convention name IS the token.
        assert_eq!(
            cnr(
                LanguageId::Java,
                "class A { void f() { a.b.C x; } }\n",
                "C",
                "a.b.C"
            ),
            Ok(Some("a.b.C".to_string()))
        );
        // Java: a type-headed path (`A.c` field access) is NOT the
        // package-simple shape (the JLS convention: a package name is
        // all lowercase) — the bare extraction stands, never a guess.
        assert_eq!(
            cnr(
                LanguageId::Java,
                "class A { void f() { int y = A.c; } }\n",
                "c",
                "A.c"
            ),
            Ok(None)
        );
        // Java: an import binding NEVER applies to a qualified reference
        // (JLS §7.5.1 — an import never shadows a qualified reference):
        // `A.c` at the point with `import a.b.c;` is `A`'s member, not
        // the imported `c`.
        assert_eq!(
            cnr(
                LanguageId::Java,
                "import a.b.c;\nclass A { void f() { int y = A.c; } }\n",
                "c",
                "A.c"
            ),
            Ok(None)
        );
        // Java: the bare reference with the import DOES bind.
        assert_eq!(
            cnr(
                LanguageId::Java,
                "import a.b.c;\nclass A { void f() { c x; } }\n",
                "c",
                "c"
            ),
            Ok(Some("a.b.c".to_string()))
        );
        // C / C++ (plan-017 issue 04): the convention is the file's
        // QUOTED includes. A quoted include yields the carrier; an
        // angle-only (or include-free) file yields `None` (the `-I`
        // limit: angle includes are never a tail — the name stays on the
        // B5 `unresolved` flag, never a guess).
        assert_eq!(
            cnr(
                LanguageId::C,
                "#include \"sub/thing.h\"\n#include <stdio.h>\n",
                "bar_fn",
                "bar_fn"
            ),
            Ok(Some("sub/thing.h".to_string())),
            "the quoted include is the carrier; the angle include is excluded"
        );
        assert_eq!(
            cnr(
                LanguageId::Cpp,
                "#include <vector>\n#include \"local/a/b.h\"\n",
                "bar_fn",
                "bar_fn"
            ),
            Ok(Some("local/a/b.h".to_string())),
            "Cpp shares the convention; the angle include is excluded"
        );
        // An angle-only C file declares no quoted include: `None`.
        assert_eq!(
            cnr(
                LanguageId::C,
                "#include <stdio.h>\n",
                "bar_fn",
                "bar_fn"
            ),
            Ok(None),
            "angle only: no quoted include, the bare lookup stands"
        );
        // Go (plan-017 issue 05): the dispatch reaches the go module. The
        // in-module convention needs the workspace's go.mod line — under
        // the nonexistent test root there is none, so local vs
        // third-party is undecidable and the convention is SILENT (the
        // name-keyed lookup stands; the Go tooling provider stays
        // authoritative). The module-reading behavior itself is pinned
        // in `go.rs` (tempdir fixtures) and the app's integration tests.
        assert_eq!(
            cnr(
                LanguageId::Go,
                "package main\n\nimport \"github.com/x/y/sub\"\n\nfunc main() { sub.Fn() }\n",
                "Fn",
                "sub.Fn"
            ),
            Ok(None),
            "no readable go.mod under the root: the convention is silent"
        );
        // A bare Go reference (no qualifier): never a package reference
        // the convention places.
        assert_eq!(
            cnr(LanguageId::Go, "package main\n\nfunc main() {}\n", "Fn", "Fn"),
            Ok(None),
            "a bare reference is not a qualified package reference"
        );
        // The unwired rows (the audit's flag rows): they decline — the
        // bare name-keyed lookup stands byte-for-byte (Java-without-an
        // import is covered above; Java is wired, just not for THIS
        // reference).
        for lang in [
            LanguageId::Rust,
            LanguageId::Python,
            LanguageId::Scheme,
            LanguageId::Ruby,
            LanguageId::CSharp,
        ]
        .into_iter()
        {
            assert_eq!(
                cnr(lang, "x", "tokio", "tokio::spawn"),
                Ok(None),
                "{lang:?} has no convention row"
            );
        }
        // The tails table mirrors the dispatch.
        assert_eq!(
            convention_file_tails(LanguageId::Clojure, "some.ns"),
            Some(vec![
                "some/ns.clj".to_string(),
                "some/ns.cljc".to_string(),
                "some/ns.cljs".to_string(),
            ])
        );
        assert_eq!(
            convention_file_tails(LanguageId::Java, "a.b.C"),
            Some(vec!["a/b/C.java".to_string()])
        );
        // C / C++: one tail per quoted include (the path as written).
        assert_eq!(
            convention_file_tails(LanguageId::C, "sub/thing.h\nx.h"),
            Some(vec!["sub/thing.h".to_string(), "x.h".to_string()])
        );
        // Go: the module-relative directory, tail-matched with its
        // trailing `/`.
        assert_eq!(convention_file_tails(LanguageId::Go, "sub"), Some(vec!["sub/".to_string()]));
        // The match predicate per row: the file-tail rows are `ends_with`
        // (byte-for-byte the pre-step's always-behavior); Go's directory
        // tail is containment (the package's files sit INSIDE the
        // directory — `ends_with` would match none of them).
        assert!(convention_file_matches(
            LanguageId::Java,
            "a/b/C.java",
            "src/a/b/C.java"
        ));
        assert!(!convention_file_matches(LanguageId::Java, "a/b/C.java", "src/x/a/b/C.java.bak"));
        assert!(convention_file_matches(LanguageId::Go, "sub/", "sub/thing.go"));
        assert!(convention_file_matches(
            LanguageId::Go,
            "engine/sub/",
            "svc/engine/sub/thing.go"
        ),
        "the module root's project offset is unknown: containment, not prefix");
        assert!(!convention_file_matches(
            LanguageId::Go,
            "sub/",
            "legacy/thing.go"
        ),
        "a same-named package in another directory does not carry the tail");
        assert!(!convention_file_matches(
            LanguageId::Go,
            "engine/sub/",
            "engine/sub2/thing.go"
        ),
        "the trailing / keeps a sibling-prefix directory out");
        assert_eq!(convention_file_tails(LanguageId::Rust, "std"), None);
    }
}

//! Go in-module import semantics for M-. (plan 017, issue 05): the
//! import-declaration extractor and the in-module-path → module-relative
//! directory convention.
//!
//! GRAMMAR EVIDENCE (pinned tree-sitter-go 0.25.0, dumped — not recalled;
//! probe: `examples/probe_go_conv.rs`):
//!
//! ```text
//! import (
//!     . "a/b"
//!     x "a/c"
//!     _ "a/d"
//!     "a/e"
//! )
//!   import_declaration [14..58] "import (…)"
//!     import [14..20] named=false "import"
//!     import_spec_list [21..58] "(…)"          ← GROUPED form: the specs
//!       import_spec [24..31] ". \"a/b\""       are its children
//!         name: dot [24..25] "."
//!         path: interpreted_string_literal [26..31] "\"a/b\""
//!           interpreted_string_literal_content "a/b"
//!       import_spec [33..40] "x \"a/c\""
//!         name: package_identifier [33..34] "x"
//!         path: interpreted_string_literal "…\"a/c\""
//!       import_spec [42..49] "_ \"a/d\""
//!         name: blank_identifier [42..43] "_"
//!         path: interpreted_string_literal "…\"a/d\""
//!       import_spec [51..56] "\"a/e\""         ← PLAIN: no name child
//!         path: interpreted_string_literal [51..56] "\"a/e\""
//!           interpreted_string_literal_content "a/e"
//!
//! import "github.com/x/y/sub"                  ← SINGLE form: the spec
//!   import_declaration [14..41] "…\"github.com/x/y/sub\""   is a DIRECT
//!     import_spec [21..41] "\"github.com/x/y/sub\""         child
//!       interpreted_string_literal [21..41]
//! import alias2 "github.com/x/y/sub2"          ← ALIASED: the alias is
//!   import_declaration [42..77] "…"              a package_identifier
//!     import_spec [49..77] "alias2 \"github.com/x/y/sub2\""
//!       package_identifier [49..55] "alias2"
//!       interpreted_string_literal [56..77]
//! import "C"                                    ← cgo: the same plain
//!   import_declaration [14..24] "…\"C\""         shape, path content "C"
//! ```
//!
//! The import path is carried as a STRING LITERAL
//! (`interpreted_string_literal` → `interpreted_string_literal_content`,
//! the path without the quotes) — not a dotted-name node. The package
//! name a spec binds: an alias (`package_identifier` child) when present,
//! else — plain form only — the LAST path segment (Go spec "Import
//! Declarations": "the package name is the final element of the import
//! path" unless aliased). `dot` (`.`) and `blank_identifier` (`_`) specs
//! bind NO name (a dot import binds the package's exported names BARE —
//! a bare identifier in the file could equally be its own package's, so
//! the convention stays silent; a blank import binds nothing): never a
//! guess.
//!
//! The CONVENTION (offline — no toolchain, no network): an import path
//! is the module path plus the directory inside the module (Go spec,
//! "Module paths and package paths"). The module path is a LINE IN THE
//! REPOSITORY'S OWN `go.mod` (`module github.com/x/y`) — readable as a
//! file, no `go` binary needed (the toolchain gap, audit F4, blocks only
//! the cross-module / download leg, which stays on the Go tooling
//! provider). So an import UNDER the module path
//! (`github.com/x/y/sub` with that module line) maps to the DIRECTORY
//! `sub/` relative to the module root, and a reference to the package
//! resolves within it; an import OUTSIDE the module path is a
//! third-party dependency and is FLAGGED (no toolchain, no network,
//! never a guess — the same rule as the C/C++ angle include).
//!
//! VERIFIED AGAINST A REAL LAYOUT ON THIS BOX (not assumed):
//! `/home/gary/dev/baml/go.mod` carries `module github.com/boundaryml/baml`;
//! `baml-cli/main.go` imports `baml "github.com/boundaryml/baml/engine/
//! language_client_go/pkg"` — under the module path, so the package lives
//! in `engine/language_client_go/pkg/` relative to the module root
//! (`/home/gary/dev/baml`), which exists and whose files declare
//! `package baml` (the alias `baml` shadows the last segment `pkg`).
//!
//! The module root (and hence the tail) is project-local, so the
//! convention yields a path TAIL the caller matches against the
//! project's indexed files (`conventions::convention_file_matches` —
//! containment for the Go directory tail; the same shape as the
//! Clojure namespace / Java class / C quoted-include conventions). The
//! tail carries the module-relative DIRECTORY (every segment after the
//! module prefix), which is what keeps a same-named package in another
//! directory out (the Java two-same-named-classes shape).

use std::path::{Path, PathBuf};

use tree_sitter::Parser;

/// Parse `source` with the pinned Go grammar (the table row is the one
/// grammar pin — `language::spec` reads it).
fn parse_go(source: &str) -> Option<tree_sitter::Tree> {
    let grammar = crate::language::spec(crate::registry::LanguageId::Go).grammar?;
    let mut parser = Parser::new();
    parser.set_language(&grammar()).unwrap();
    parser.parse(source.as_bytes(), None)
}

/// The import bindings the file declares: `("bound name", import path)`
/// per import that binds a name.
///
/// - plain `import "a/b/c"` → `("c", "a/b/c")` (the last path segment —
///   only when it is a Go identifier; a non-identifier last segment
///   (`a/b/2x`) cannot be a reference qualifier, so it binds nothing);
/// - aliased `import s "a/b/c"` → `("s", "a/b/c")` (the alias shadows
///   the last segment — Go spec: the package is referred to by the
///   alias alone);
/// - dot `import . "a/b"` and blank `import _ "a/b"` → NO binding (a dot
///   import's names are bare and cannot be narrowed to one of several
///   dot-imported packages — the convention stays silent, never a
///   guess);
/// - cgo `import "C"` → `("C", "C")` (the same plain shape; a `C.x`
///   reference is placed outside the module path and flagged by
///   [`reference_convention_name`] — a cgo declaration is not a file of
///   this module).
///
/// `None` only when the parse itself yields no tree (defensive — the
/// caller keeps the bare lookup, never a guess); an empty `Vec` is the
/// honest "no imports".
pub fn import_package_bindings(source: &str) -> Option<Vec<(String, String)>> {
    let tree = parse_go(source)?;
    let bytes = source.as_bytes();
    let root = tree.root_node();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    while let Some(decl) = root.child(i) {
        i += 1;
        if decl.kind() != "import_declaration" {
            continue;
        }
        // Each `import_spec` child: either DIRECT (the single-import
        // form `import "a/b"`) or inside the `import_spec_list` child
        // (the grouped form `import ( … )`) — the probe dump above.
        let mut j = 0;
        while let Some(spec) = decl.child(j) {
            j += 1;
            match spec.kind() {
                "import_spec" => {
                    if let Some((name, path)) = spec_binding(spec, bytes) {
                        out.push((name, path));
                    }
                }
                "import_spec_list" => {
                    let mut k = 0;
                    while let Some(inner) = spec.child(k) {
                        k += 1;
                        if inner.kind() == "import_spec"
                            && let Some((name, path)) = spec_binding(inner, bytes)
                        {
                            out.push((name, path));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Some(out)
}

/// The `(bound name, import path)` of one `import_spec` (the probe's
/// four shapes: plain / aliased / dot / blank). `None` for the dot and
/// blank forms (they bind no name) and for a non-identifier plain last
/// segment.
fn spec_binding(spec: tree_sitter::Node, bytes: &[u8]) -> Option<(String, String)> {
    let mut path: Option<String> = None;
    let mut alias: Option<String> = None;
    let mut binds_nothing = false;
    let mut k = 0;
    while let Some(c) = spec.child(k) {
        k += 1;
        match c.kind() {
            "interpreted_string_literal" => {
                // The `interpreted_string_literal_content` child carries
                // the path without the surrounding quotes.
                let mut m = 0;
                while let Some(content) = c.child(m) {
                    m += 1;
                    if content.kind() == "interpreted_string_literal_content"
                        && let Ok(text) = content.utf8_text(bytes)
                    {
                        path = Some(text.to_string());
                    }
                }
            }
            "package_identifier" => {
                // Present only in the ALIASED form (a plain import has
                // no name child — the probe dump): the alias.
                if let Ok(text) = c.utf8_text(bytes) {
                    alias = Some(text.to_string());
                }
            }
            // `.` (dot import) and `_` (blank import): no bound name.
            "dot" | "blank_identifier" => binds_nothing = true,
            _ => {}
        }
    }
    let path = path?;
    if binds_nothing {
        return None;
    }
    let name = if let Some(alias) = alias {
        alias
    } else {
        path
            .rsplit('/')
            .next()
            .filter(|s| is_go_identifier(s))
            .map(String::from)?
    };
    Some((name, path))
}

/// A Go identifier (conservative ASCII reading of the spec's "letter or
/// underscore" lead + letter/digit/underscore rest — `char::is_alphabetic`
/// admits the Unicode letters Go itself allows, so the check errs toward
/// ADMITTING, never toward rejecting a genuine identifier).
fn is_go_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => s
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_'),
        _ => false,
    }
}

/// The module path from a `go.mod`'s text: the `module <path>` directive
/// (the first one — a valid go.mod carries exactly one; the path may be
/// quoted). `None` when the directive is absent or empty — a file with a
/// broken module line cannot declare a module.
pub fn module_path_from_go_mod(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if !(line.starts_with("module ") || line.starts_with("module\t")) {
            continue;
        }
        let path = line["module".len()..].trim();
        let path = path
            .strip_prefix('"')
            .and_then(|p| p.strip_suffix('"'))
            .unwrap_or(path);
        if path.is_empty() {
            return None;
        }
        return Some(path.to_string());
    }
    None
}

/// The module path of the module containing `project_root/rel`: the
/// NEAREST `go.mod` walking UP from the file's own directory to the
/// project root (both ends inclusive — the Go module discovery rule,
/// bounded to the workspace: a go.mod above the root is not the
/// project's). `None` when no go.mod is found, when the nearest one
/// cannot be read, or when it carries no parseable `module` line (the
/// innermost module is broken — a further-up module must not be
/// claimed for this file).
pub fn module_path_for_file(project_root: &Path, rel: &str) -> Option<String> {
    let root = project_root.to_path_buf();
    let full = root.join(rel);
    let mut dir = full.parent().map(PathBuf::from).unwrap_or_else(|| root.clone());
    loop {
        let go_mod = dir.join("go.mod");
        if go_mod.is_file() {
            return std::fs::read_to_string(&go_mod)
                .ok()
                .and_then(|text| module_path_from_go_mod(&text));
        }
        if dir == root {
            return None;
        }
        dir = dir.parent()?.to_path_buf();
    }
}

/// The module-relative DIRECTORY for an import path under the module
/// path: `module github.com/x/y`, import `github.com/x/y/sub` → `sub`;
/// multi-segment, `github.com/x/y/engine/sub` → `engine/sub`. `None`
/// when the import path is the module path itself (the root package has
/// no directory) or is NOT under it (a third-party module — the
/// `module + "/"` boundary: `github.com/x/y2` is NOT under
/// `github.com/x/y`).
pub fn local_dir_tail(module: &str, import_path: &str) -> Option<String> {
    let rest = import_path.strip_prefix(module)?.strip_prefix('/')?;
    if rest.is_empty() {
        return None;
    }
    Some(rest.to_string())
}

/// The file path TAIL for a module-relative package directory (the
/// carrier [`reference_convention_name`] returns): the directory plus
/// the trailing `/`. A package is many files (no single file tail
/// exists), and its files sit INSIDE the directory, so the tail is
/// matched by CONTAINMENT (`conventions::convention_file_matches` —
/// `file.contains(tail)`): the directory segment in the tail is what
/// keeps a same-named package in another directory out (the Java
/// two-same-named-classes shape; `legacy/thing.go` does not carry the
/// `sub/` segment, though it declares `package sub` just as
/// `sub/thing.go` does).
pub fn local_dir_tails(dir: &str) -> Vec<String> {
    vec![format!("{dir}/")]
}

/// The CONVENTION NAME (the module-relative package directory) for the
/// reference the M-. extraction produced at the point (`path_token`,
/// with its last segment `ident`), per the Go in-module convention:
///
/// - a QUALIFIED reference `pkg.Sym` whose qualifier `pkg` is bound by
///   THIS file's imports (plain last-segment or alias) AND whose import
///   path sits under the module path of the file's own go.mod →
///   `Ok(Some("the module-relative directory"))` — the directory tail
///   the indexed files must end in;
/// - `Err(name)` — the import path is OUTSIDE the module path: a
///   third-party dependency (no toolchain, no network, never a guess —
///   the reference is FLAGGED, the flagged name being the one at the
///   point);
/// - `Ok(None)` — the convention is silent (never a guess): a bare
///   reference (same-package / dot-imported — a dot import cannot be
///   narrowed), a qualifier the file's imports do not bind (a type
///   parameter, a package the file never imports), a non-identifier
///   qualifier (a chained selector `a.b.c`), a no-tree parse, no
///   readable go.mod (local vs third-party is undecidable offline —
///   the Go tooling provider stays authoritative), or the module's own
///   root package (no directory tail can narrow — the bare lookup
///   stands).
pub fn reference_convention_name(
    source: &str,
    ident: &str,
    path_token: &str,
    project_root: &Path,
    rel: &str,
) -> Result<Option<String>, String> {
    // Go has no same-package qualified reference: a dot at all is the
    // package-qualifier shape (`pkg.Sym`).
    let Some((qualifier, _member)) = path_token.rsplit_once('.') else {
        return Ok(None);
    };
    if !is_go_identifier(qualifier) {
        return Ok(None);
    }
    let Some(bindings) = import_package_bindings(source) else {
        return Ok(None);
    };
    let Some(import_path) = bindings
        .iter()
        .find(|(name, _)| name == qualifier)
        .map(|(_, path)| path.clone())
    else {
        return Ok(None);
    };
    let Some(module) = module_path_for_file(project_root, rel) else {
        return Ok(None);
    };
    if let Some(dir) = local_dir_tail(&module, &import_path) {
        return Ok(Some(dir));
    }
    if import_path == module {
        return Ok(None);
    }
    // Third-party (outside the module path): flag, never guess.
    Err(ident.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe fixture (`examples/probe_go_conv.rs` — the dump this
    /// module's doc quotes): all four import shapes, grouped.
    const GROUPED: &str = "package main\n\nimport (\n\t. \"a/b\"\n\tx \"a/c\"\n\t_ \"a/d\"\n\t\"a/e\"\n)\n\nfunc main() {}\n";

    #[test]
    fn import_shapes_bind_per_the_grammar() {
        let bindings = import_package_bindings(GROUPED).expect("the fixture parses");
        // The aliased spec binds the ALIAS (the last segment `c` is
        // shadowed); the plain spec binds the last segment; the dot and
        // blank specs bind NOTHING (never a guess).
        assert_eq!(bindings, vec![("x".into(), "a/c".into()), ("e".into(), "a/e".into())]);
    }

    #[test]
    fn single_and_aliased_imports_bind() {
        let src = "package main\n\nimport \"github.com/x/y/sub\"\nimport alias2 \"github.com/x/y/sub2\"\n\nfunc main() {}\n";
        let bindings = import_package_bindings(src).expect("the fixture parses");
        assert_eq!(
            bindings,
            vec![
                ("sub".into(), "github.com/x/y/sub".into()),
                ("alias2".into(), "github.com/x/y/sub2".into()),
            ]
        );
    }

    #[test]
    fn cgo_import_binds_c() {
        let bindings =
            import_package_bindings("package main\n\nimport \"C\"\n\nfunc main() {}\n")
                .expect("the fixture parses");
        assert_eq!(bindings, vec![("C".into(), "C".into())]);
    }

    #[test]
    fn no_imports_is_an_empty_vec_not_none() {
        assert_eq!(
            import_package_bindings("package main\n\nfunc main() {}\n"),
            Some(Vec::new())
        );
    }

    #[test]
    fn plain_last_segment_that_is_not_an_identifier_binds_nothing() {
        // `2x` cannot start a Go identifier, so `a/b/2x` references
        // nothing by a last-segment name — the convention stays silent.
        let bindings =
            import_package_bindings("package main\n\nimport \"a/b/2x\"\n\nfunc main() {}\n")
                .expect("the fixture parses");
        assert_eq!(bindings, Vec::new());
    }

    #[test]
    fn module_line_parsing() {
        assert_eq!(
            module_path_from_go_mod("module github.com/x/y\n\ngo 1.24\n"),
            Some("github.com/x/y".to_string())
        );
        // Comments / blank lines / the require block: the module line
        // stands wherever it sits.
        assert_eq!(
            module_path_from_go_mod("// a module\n\nmodule github.com/x/y\n\nrequire (\n\tgolang.org/x/text v0.1.0\n)\n"),
            Some("github.com/x/y".to_string())
        );
        // A quoted module path (the legal form for odd paths).
        assert_eq!(
            module_path_from_go_mod("module \"github.com/x/y\"\n"),
            Some("github.com/x/y".to_string())
        );
        // No module line / an empty one: no module.
        assert_eq!(module_path_from_go_mod("go 1.24\n"), None);
        assert_eq!(module_path_from_go_mod("module \n"), None);
    }

    #[test]
    fn module_discovery_walks_up_from_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("go.mod"),
            "module github.com/root/mod\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("svc")).unwrap();
        std::fs::write(
            dir.path().join("svc/go.mod"),
            "module github.com/x/y\n",
        )
        .unwrap();
        // The NEAREST go.mod wins (the file's own module, not the
        // repo-root's).
        assert_eq!(
            module_path_for_file(dir.path(), "svc/main.go"),
            Some("github.com/x/y".to_string())
        );
        assert_eq!(
            module_path_for_file(dir.path(), "main.go"),
            Some("github.com/root/mod".to_string())
        );
        // No go.mod at all: the walk finds nothing.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(module_path_for_file(empty.path(), "main.go"), None);
        // The nearest go.mod exists but carries no module line: the
        // innermost module is broken — a further-up module must not be
        // claimed.
        std::fs::create_dir_all(dir.path().join("svc/broken")).unwrap();
        std::fs::write(dir.path().join("svc/broken/go.mod"), "go 1.24\n")
            .unwrap();
        assert_eq!(module_path_for_file(dir.path(), "svc/broken/main.go"), None);
    }

    #[test]
    fn local_dir_tail_is_the_module_boundary() {
        // Under the module path: the module-relative directory.
        assert_eq!(
            local_dir_tail("github.com/x/y", "github.com/x/y/sub"),
            Some("sub".to_string())
        );
        // Multi-segment local directories keep every segment (the
        // baml real layout: `engine/language_client_go/pkg`).
        assert_eq!(
            local_dir_tail(
                "github.com/x/y",
                "github.com/x/y/engine/language_client_go/pkg"
            ),
            Some("engine/language_client_go/pkg".to_string())
        );
        // The module's own root package: no directory.
        assert_eq!(local_dir_tail("github.com/x/y", "github.com/x/y"), None);
        // THIRD-PARTY: outside the module path — and the boundary is
        // `module + "/"`, so `github.com/x/y2` (a sibling module) and
        // the bare domain are NOT under `github.com/x/y`.
        assert_eq!(
            local_dir_tail("github.com/x/y", "github.com/x/y2/sub"),
            None
        );
        assert_eq!(local_dir_tail("github.com/x/y", "github.com/third/pkg"), None);
        assert_eq!(local_dir_tail("github.com/x/y", "github.com/x"), None);
    }

    #[test]
    fn local_dir_tails_are_the_directory_plus_slash() {
        // The trailing `/` is the segment boundary: a `sub2` directory
        // must not read as `sub`.
        assert_eq!(local_dir_tails("sub"), vec!["sub/".to_string()]);
        assert_eq!(
            local_dir_tails("engine/language_client_go/pkg"),
            vec!["engine/language_client_go/pkg/".to_string()]
        );
        // The match is CONTAINMENT (a package's files sit INSIDE the
        // directory — `ends_with` is false for every file of `sub/`):
        // the right file carries the tail as a path segment, and a
        // same-named package in another directory does not.
        assert!(
            "sub/thing.go".contains("sub/") && !"legacy/thing.go".contains("sub/"),
            "the directory segment discriminates the same-named package"
        );
        assert!(
            !"engine/sub2/thing.go".contains("engine/sub/"),
            "the trailing / keeps sub2 out"
        );
    }

    fn module_fixture(module_line: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), module_line).unwrap();
        (dir, "main.go".to_string())
    }

    const LOCAL_SOURCE: &str = "package main\n\nimport \"github.com/x/y/sub\"\n\nfunc main() { sub.Fn() }\n";

    #[test]
    fn local_import_maps_to_the_module_relative_directory() {
        let (dir, rel) = module_fixture("module github.com/x/y\n\ngo 1.24\n");
        assert_eq!(
            reference_convention_name(LOCAL_SOURCE, "Fn", "sub.Fn", dir.path(), &rel),
            Ok(Some("sub".to_string()))
        );
    }

    #[test]
    fn aliased_import_maps_through_the_alias() {
        let (dir, rel) = module_fixture("module github.com/x/y\n");
        // The baml real shape: the alias shadows the last segment.
        let src = "package main\n\nimport w \"github.com/x/y/engine/sub\"\n\nfunc main() { w.Fn() }\n";
        assert_eq!(
            reference_convention_name(src, "Fn", "w.Fn", dir.path(), &rel),
            Ok(Some("engine/sub".to_string()))
        );
        // The shadowed last segment is NOT a binding (Go: the alias
        // alone refers to the package).
        assert_eq!(
            reference_convention_name(src, "Fn", "sub.Fn", dir.path(), &rel),
            Ok(None)
        );
    }

    #[test]
    fn third_party_import_is_flagged_never_a_guess() {
        let (dir, rel) = module_fixture("module github.com/x/y\n");
        let src = "package main\n\nimport \"github.com/third/pkg\"\n\nfunc main() { pkg.Fn() }\n";
        // Outside the module path: a third-party dependency — no
        // toolchain, no network, never a guess. The flagged name is
        // the one at the point.
        assert_eq!(
            reference_convention_name(src, "Fn", "pkg.Fn", dir.path(), &rel),
            Err("Fn".to_string())
        );
    }

    #[test]
    fn reference_shapes_the_convention_stays_silent_on() {
        let (dir, rel) = module_fixture("module github.com/x/y\n");
        // A BARE reference (a same-package call — or a dot-imported
        // name the convention refuses to narrow): the name-keyed
        // lookup stands.
        assert_eq!(
            reference_convention_name(LOCAL_SOURCE, "Fn", "Fn", dir.path(), &rel),
            Ok(None)
        );
        // A qualifier the file's imports do NOT bind: not a package
        // reference the convention can place (a type parameter, a
        // never-imported package, …) — the bare lookup stands.
        assert_eq!(
            reference_convention_name(LOCAL_SOURCE, "Fn", "other.Fn", dir.path(), &rel),
            Ok(None)
        );
        // A CHAINED selector: `a.b` is not a Go identifier — no package
        // reference.
        assert_eq!(
            reference_convention_name(LOCAL_SOURCE, "c", "sub.Fn.c", dir.path(), &rel),
            Ok(None)
        );
        // The module's OWN root package: no directory tail can narrow —
        // the bare lookup stands.
        let root_pkg = "package main\n\nimport \"github.com/x/y\"\n\nfunc main() { y.Fn() }\n";
        assert_eq!(
            reference_convention_name(root_pkg, "Fn", "y.Fn", dir.path(), &rel),
            Ok(None)
        );
    }

    #[test]
    fn no_readable_go_mod_keeps_the_convention_silent() {
        // No go.mod in the walk: local vs third-party is undecidable
        // offline — the Go tooling provider stays authoritative (it
        // flags "not a Go module project").
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            reference_convention_name(LOCAL_SOURCE, "Fn", "sub.Fn", empty.path(), "main.go"),
            Ok(None)
        );
    }
}

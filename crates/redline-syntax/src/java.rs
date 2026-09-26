//! Java single-type import semantics for M-. (plan 017, issue 03): the
//! import-declaration extractor and the JLS `a.b.C` → `a/b/C.java`
//! convention.
//!
//! GRAMMAR EVIDENCE (pinned tree-sitter-java 0.23.5, dumped — not
//! recalled; probe: `examples/probe_java_conv.rs`):
//!
//! ```text
//! import a.b.C;
//!   import_declaration [21..34] "import a.b.C;"
//!     scoped_identifier [28..33] "a.b.C"   (the WHOLE path, one node —
//!                                          the `.`-delimited
//!                                          scoped_identifier the node
//!                                          machinery of `node/paths.rs`
//!                                          already handles)
//! import static a.b.D.m;
//!   import_declaration [49..71] "import static a.b.D.m;"
//!     children: "import" "static" (both anonymous tokens)
//!       + scoped_identifier [63..70] "a.b.D.m"
//! import a.b.*;
//!   import_declaration [35..48] "import a.b.*;"
//!     scoped_identifier [42..45] "a.b"  +  asterisk [46..47] "*"
//! ```
//!
//! The CONVENTION is the JLS one (§7.6 "Package and Class Names": a
//! class's binary name is its package name plus the simple class name,
//! separated by `.`; the source-file path is that binary name with `.`
//! as the path separator, plus the `.java` source extension — `a.b.C`
//! → `a/b/C.java`). The source ROOT (`src/`, `src/main/java/`, the
//! repository root itself) is project-local, so the convention yields a
//! path TAIL that the caller matches against the project's known files
//! — never an absolute guess (the same shape as the Clojure namespace
//! convention).

use tree_sitter::Parser;

/// Parse `source` with the pinned Java grammar (the table row is the one
/// grammar pin — `language::spec` reads it).
fn parse_java(source: &str) -> Option<tree_sitter::Tree> {
    let grammar = crate::language::spec(crate::registry::LanguageId::Java).grammar?;
    let mut parser = Parser::new();
    parser.set_language(&grammar()).unwrap();
    parser.parse(source.as_bytes(), None)
}

/// The single-type import bindings declared by the file:
/// `import a.b.C;` → `("C", "a.b.C")` (the simple name → the fully
/// qualified name).
///
/// The other import shapes are deliberately OUT (each is the audit's
/// flag row, never a guess):
/// - `import static a.b.D.m;` (the `static` token child): a static
///   member import names a METHOD / FIELD of `a.b.D`, not a class — the
///   bare member keeps its bare name-keyed lookup;
/// - `import a.b.*;` (the `asterisk` child): a wildcard places no single
///   class (any class of package `a.b`) — the bare name keeps the
///   name-keyed superset;
/// - a single-segment import (`import C;` — the default package) has no
///   package, hence no directory tail: it stays out.
///
/// `None` only when the parse itself yields no tree (defensive — the
/// caller keeps the bare lookup, never a guess); an empty `Vec` is the
/// honest "no single-type imports" (the convention simply has nothing to
/// narrow by).
pub fn import_class_bindings(source: &str) -> Option<Vec<(String, String)>> {
    let tree = parse_java(source)?;
    let bytes = source.as_bytes();
    let root = tree.root_node();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    while let Some(decl) = root.child(i) {
        i += 1;
        if decl.kind() != "import_declaration" {
            continue;
        }
        let mut static_import = false;
        let mut wildcard = false;
        let mut fqn: Option<String> = None;
        let mut j = 0;
        while let Some(c) = decl.child(j) {
            j += 1;
            match c.kind() {
                "static" => static_import = true,
                "asterisk" => wildcard = true,
                "scoped_identifier"
                    if fqn.is_none() =>
                {
                    fqn = c.utf8_text(bytes).ok().map(|t| t.to_string());
                }
                _ => {}
            }
        }
        if static_import || wildcard {
            continue;
        }
        let Some(fqn) = fqn else { continue };
        // The JLS simple-name / package split: `a.b.C` → package `a.b`,
        // simple name `C`. An empty side is a malformed shape — out.
        let Some((pkg, simple)) = fqn.rsplit_once('.') else {
            continue;
        };
        if pkg.is_empty() || simple.is_empty() {
            continue;
        }
        out.push((simple.to_string(), fqn));
    }
    Some(out)
}

/// The file path TAIL for the fully qualified class name `fqn` (JLS
/// §7.6: the binary name becomes the path (`.` → `/`), plus the
/// `.java` source extension): `a.b.C` → `a/b/C.java`. Java has exactly
/// ONE source extension (unlike Clojure's three), so exactly one tail.
/// The caller matches the tail against the project's known files (the
/// source-root prefix — `src/`, `src/main/java/`, the root itself — is
/// project-local and must not be guessed).
pub fn class_file_tails(fqn: &str) -> Vec<String> {
    vec![format!("{}.java", fqn.replace('.', "/"))]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe fixture (`examples/probe_java_conv.rs` — the dump this
    /// module's doc quotes): the three import shapes in one file.
    const FIXTURE: &str = "package com.example;\n\
                           import a.b.C;\n\
                           import a.b.*;\n\
                           import static a.b.D.m;\n\
                           class A {}\n";

    #[test]
    fn single_type_import_binds_simple_name_to_fqn() {
        let bindings = import_class_bindings(FIXTURE).expect("the fixture parses");
        // The single-type import carries the binding…
        assert!(
            bindings.contains(&("C".to_string(), "a.b.C".to_string())),
            "the binding: {bindings:?}"
        );
        // …and the static + wildcard imports are OUT (flag rows — never
        // a guess): exactly one binding, no `D` / `m` / `*` entry.
        assert_eq!(bindings.len(), 1, "only the single-type import: {bindings:?}");
    }

    #[test]
    fn no_imports_is_an_empty_vec_not_none() {
        assert_eq!(
            import_class_bindings("package com.example;\nclass A {}\n"),
            Some(Vec::new()),
            "no imports: the honest empty (the convention has nothing to narrow by)"
        );
    }

    #[test]
    fn default_package_import_has_no_tail_and_stays_out() {
        // A single-segment import names the DEFAULT package (no
        // directory): the convention has nothing to say — the binding
        // must not exist (a `C.java` tail would be a guess).
        assert_eq!(
            import_class_bindings("import C;\nclass A {}\n"),
            Some(Vec::new())
        );
    }

    #[test]
    fn class_file_tails_convention() {
        // JLS §7.6: `a.b.C` → `a/b/C.java` (dots → `/`, one extension).
        // The pin both directions: a DOT maps to a directory separator
        // (a hyphen/underscore mapping would redden this), and there is
        // exactly ONE tail (Java has one source extension).
        assert_eq!(class_file_tails("a.b.C"), vec!["a/b/C.java".to_string()]);
        assert_eq!(
            class_file_tails("com.example.util.List"),
            vec!["com/example/util/List.java".to_string()]
        );
        // The default-package class: the bare tail.
        assert_eq!(class_file_tails("C"), vec!["C.java".to_string()]);
    }
}

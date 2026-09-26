//! C/C++ quoted `#include` semantics for M-. (plan 017, issue 04): the
//! include extractor and the quoted-include → file-tails convention.
//!
//! GRAMMAR EVIDENCE (pinned tree-sitter-c / tree-sitter-cpp, dumped — not
//! recalled; probe: `examples/probe_ccpp_conv.rs`):
//!
//! ```text
//! #include "sub/thing.h"
//!   preproc_include [0..23] "#include \"sub/thing.h\"\n"
//!     #include [0..8]  named=false
//!     string_literal [9..22] named=true "\"sub/thing.h\""
//!       " [9..10]  (anonymous quote)
//!       string_content [10..21] "sub/thing.h"      ← the path, no quotes
//!       " [21..22] (anonymous quote)
//! #include <stdio.h>
//!   preproc_include [23..42] "#include <stdio.h>\n"
//!     #include [23..31] named=false
//!     system_lib_string [32..41] named=true "<stdio.h>"   ← ANGLE: a
//!                                                          DIFFERENT node
//!                                                          kind, a leaf
//! ```
//!
//! C++ carries the SAME two `preproc_include` shapes (the `using` /
//! namespace-alias forms — `namespace_alias_definition`,
//! `using_declaration`, `alias_declaration` — are reference shapes the
//! name-keyed index already carries; they resolve through the SAME
//! quoted-include narrowing below, never a separate engine).
//!
//! The CONVENTION (ISO/IEC 9899 §6.10.2 / GCC `#include` search order):
//! a QUOTED include `"x/y.h"` is searched relative to the INCLUDING
//! file's directory first, then the `-I` search path. The including
//! file's directory (and, by project layout, the source roots) is what
//! makes a quoted include decidable: the include's path is a TAIL that
//! the project's indexed files may end in — exactly the Clojure
//! namespace / Java class shape (the `src/`, `include/`, source-root
//! prefix is project-local and must not be guessed; a tail match covers
//! every such root at once).
//!
//! The HONEST LIMIT (stated, not papered over): an ANGLE include
//! `<x.h>` is NOT resolvable here. It comes from a system / toolchain
//! search path (`-I`, the compiler's built-in include dirs) that redline
//! does not know. It is a `system_lib_string`, a DIFFERENT grammar node
//! from the quoted `string_literal`, so the extractor excludes it by
//! construction — an angle include never yields a tail, and a name that
//! only a system header would provide stays on the B5 named-`unresolved`
//! flag (no tooling provider for C/C++), never a confident jump to the
//! first same-named workspace file.

use tree_sitter::Parser;

/// Parse `source` with the pinned C or C++ grammar (the table row is the
/// one grammar pin — `language::spec` reads it). Both languages share the
/// `preproc_include` / `string_literal` / `system_lib_string` shapes.
fn parse_ccpp(lang: crate::registry::LanguageId, source: &str) -> Option<tree_sitter::Tree> {
    let grammar = crate::language::spec(lang).grammar?;
    let mut parser = Parser::new();
    parser.set_language(&grammar()).unwrap();
    parser.parse(source.as_bytes(), None)
}

/// The CONVENTION NAME carrier for a C/C++ file: the newline-joined set of
/// QUOTED include paths the file declares (`#include "a/b.h"` → `a/b.h`).
///
/// This is the internal carrier the shared pre-step hands to
/// [`convention_tails`]; it is never user-facing and never an absolute
/// path. Quoted includes only — an angle include (`system_lib_string`) is
/// deliberately excluded (the `-I` search path is unknown: flag, never
/// guess). `None` when the parse yields no tree, or the file declares no
/// QUOTED include (the caller keeps the bare name-keyed lookup
/// byte-for-byte; an empty set is the honest "no quoted include" and is
/// folded into `None` so the pre-step does nothing).
pub fn convention_name(source: &str, lang: crate::registry::LanguageId) -> Option<String> {
    let paths = quoted_include_paths(source, lang)?;
    (!paths.is_empty()).then(|| paths.join("\n"))
}

/// The file path TAILS for a convention-name carrier built by
/// [`convention_name`]: one tail per quoted include, the include's path as
/// written (`a/b.h`). Each is matched as a TAIL against the project's
/// indexed files (`ends_with`), so the including-file-relative location
/// and every source-root placement of the same relative path all match —
/// the directory join in the tail is what keeps a decoy of the same base
/// name in another directory out (the Java two-same-named-classes shape).
pub fn convention_tails(name: &str) -> Vec<String> {
    name.split('\n')
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect()
}

/// The QUOTED include paths declared by the file (the `string_content`
/// text of each `preproc_include` whose path is a `string_literal`). An
/// angle include (`system_lib_string`) is a DIFFERENT node kind and is
/// never collected here — that is the `-I` limit made structural, not a
/// convention choice. `None` only when the parse itself yields no tree
/// (defensive — the caller keeps the bare lookup, never a guess); an
/// empty `Vec` is the honest "no quoted includes".
fn quoted_include_paths(
    source: &str,
    lang: crate::registry::LanguageId,
) -> Option<Vec<String>> {
    let tree = parse_ccpp(lang, source)?;
    let bytes = source.as_bytes();
    let root = tree.root_node();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while let Some(inc) = root.child(i) {
        i += 1;
        if inc.kind() != "preproc_include" {
            continue;
        }
        // The path child: a QUOTED `string_literal` (resolvable) or a
        // `system_lib_string` (angle — excluded by construction).
        let mut j = 0;
        while let Some(path) = inc.child(j) {
            j += 1;
            if path.kind() != "string_literal" {
                continue;
            }
            // The `string_content` child carries the path without the
            // surrounding quotes (probe: `string_literal` → `"`,
            // `string_content`, `"`).
            let mut k = 0;
            while let Some(c) = path.child(k) {
                k += 1;
                if c.kind() == "string_content"
                    && let Ok(text) = c.utf8_text(bytes)
                {
                    out.push(text.to_string());
                }
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: crate::registry::LanguageId = crate::registry::LanguageId::C;
    const CPP: crate::registry::LanguageId = crate::registry::LanguageId::Cpp;

    /// The probe fixture (`examples/probe_ccpp_conv.rs` — the dump this
    /// module's doc quotes): one quoted subdirectory include, one angle
    /// include, one quoted flat include, in a C file.
    const C_FIXTURE: &str =
        "#include \"sub/thing.h\"\n#include <stdio.h>\n#include \"x.h\"\nint main(void) { return 0; }\n";

    /// The QUOTED-only convention: the two quoted includes are the carrier,
    /// the angle include is OUT (the `-I` limit — never a tail, never a
    /// guess). This is the pin the "make angle resolve anyway" mutation
    /// must redden.
    #[test]
    fn c_quoted_includes_are_the_carrier_angle_is_excluded() {
        assert_eq!(
            convention_name(C_FIXTURE, C),
            Some("sub/thing.h\nx.h".to_string()),
            "only the QUOTED includes; the angle <stdio.h> is excluded"
        );
        assert_eq!(
            convention_tails("sub/thing.h\nx.h"),
            vec!["sub/thing.h".to_string(), "x.h".to_string()]
        );
    }

    /// An ANGLE-only file declares NO quoted include: the carrier is
    /// `None` (the pre-step does nothing — the bare name-keyed lookup
    /// stands, and an unresolvable name lands on the B5 `unresolved`
    /// flag, never on a guessed same-named workspace file).
    #[test]
    fn angle_only_file_has_no_convention() {
        assert_eq!(convention_name("#include <stdio.h>\nint main(void) { return 0; }\n", C), None);
    }

    /// Cpp shares the convention (the same `preproc_include` shapes), and
    /// the `using` / namespace-alias forms add nothing to it (they are
    /// reference shapes the index already carries).
    #[test]
    fn cpp_quoted_includes_share_the_convention() {
        let src = "#include <vector>\n#include \"local/a/b.h\"\nnamespace my = myns;\nusing namespace foo;\n";
        assert_eq!(
            convention_name(src, CPP),
            Some("local/a/b.h".to_string()),
            "the angle <vector> is excluded; the using/alias forms are not includes"
        );
    }

    /// The directory join in the tail is what distinguishes the right
    /// header from a same-BASE-name decoy in another directory (the
    /// Java two-same-named-classes shape). A base-name-only tail would
    /// match both — the landing mutation pin.
    #[test]
    fn tails_carry_the_directory_join() {
        assert_eq!(
            convention_tails("sub/thing.h"),
            vec!["sub/thing.h".to_string()],
            "the FULL include path is the tail (the directory join kept)"
        );
        // `sub/thing.h` is the tail; the decoy `elsewhere/thing.h` does
        // NOT end in it, but BOTH end in the base `thing.h` — so a
        // base-name-only tail would over-match (the mutation reddens here).
        assert!(
            !"elsewhere/thing.h".ends_with("sub/thing.h")
                && "sub/thing.h".ends_with("sub/thing.h"),
            "the directory join discriminates the decoy"
        );
    }

    /// No parseable tree (defensive): `None`, the caller keeps the bare
    /// lookup (never a guess).
    #[test]
    fn no_quoted_include_is_none() {
        assert_eq!(convention_name("int main(void) { return 0; }\n", C), None);
    }
}

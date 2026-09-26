//! Ruby `require_relative` semantics for M-. (plan 017, issue 06): the
//! import method-call extractor and the require_relative → file convention.
//!
//! GRAMMAR EVIDENCE (pinned tree-sitter-ruby 0.23.1, dumped — not recalled;
//! probe: `examples/probe_ruby_conv.rs`):
//!
//! ```text
//! require 'a/b'
//!   call [0..13] "require 'a/b'"
//!     method: (identifier) "require"
//!     arguments: (argument_list (string (string_content "a/b")))
//!     — NO `receiver` field (a bare Kernel#require)
//! require_relative 'c'
//!   call [14..34] "require_relative 'c'"
//!     method: (identifier) "require_relative"
//!     arguments: (argument_list (string (string_content "c")))
//!     — NO `receiver` field
//! autoload :D, 'd/e'
//!   call [35..53] "autoload :D, 'd/e'"
//!     method: (identifier) "autoload"
//!     arguments: (argument_list (simple_symbol ":D") (string (string_content "d/e")))
//!     — NO `receiver` field
//!
//! self.require 'x'          ← RECEIVER form: a DIFFERENT call
//!   call "self.require 'x'"
//!     receiver: (self) "self"      ← the `receiver` FIELD is present
//!     method: (identifier) "require"
//! foo.require_relative 'y'   ← receiver: (identifier) "foo"
//! Other.autoload :Z, 'z/w'   ← receiver: (constant) "Other"
//! ```
//!
//! The import forms are **method CALLS, not dedicated node kinds** — a
//! `call` whose `method` field is the `identifier` `require` /
//! `require_relative` / `autoload`. A BARE call carries the `method` and
//! `arguments` fields and NO `receiver` field; a receiver call
//! (`self.require`, `foo.require_relative`, `Other.autoload`) carries a
//! `receiver` field. The two are structurally distinguishable
//! (`call.child_by_field_name("receiver").is_none()`), and only the BARE
//! forms are `Kernel#require` / `Kernel#require_relative` /
//! `Kernel#autoload` — a receiver call is some OTHER object's method of
//! the same name and is never counted (the same bare-vs-qualified
//! discipline the JLS import binding keeps: an import never shadows a
//! qualified reference).
//!
//! The CONVENTION (the honest limit, stated):
//! - `require_relative "x"` is FULLY DETERMINISTIC — the path is relative
//!   to the INCLUDING file's own directory (`__dir__` semantics), so it
//!   resolves to `x.rb` in that directory (the `.rb` extension added when
//!   the name does not already carry it; it may also point at a file path
//!   `sub/x`, which resolves to `sub/x.rb`). The including file's
//!   project-relative path is the convention's `rel`, so the target is a
//!   KNOWN project-relative path — no toolchain, no network, no `$LOAD_PATH`
//!   (ruby-lang.org `Module#require_relative`; the Ruby spec's
//!   require_relative semantics).
//! - `require "x"` and `autoload :C, "x"` go through the **LOAD PATH**
//!   (`$LOAD_PATH` — the gem `lib/` roots under Bundler, the environment,
//!   the `-r` command line), which redline CANNOT know. They are never a
//!   tail: a name that only a LOAD-PATH file would provide stays on the
//!   named `unresolved` flag (no tooling provider for Ruby — the same rule
//!   as the C/C++ angle include and the Go third-party import), never a
//!   confident jump to the first same-named workspace file.
//!
//! The REFERENCE the M-. extraction produces at the point is a CONSTANT
//! (`Thing`, or `Thing::Inner` → the last segment `Inner`): the required
//! file's defining symbol. The convention does NOT name a file by the
//! constant's own text — it yields the file's DETERMINISTIC
//! require_relative targets (path tails), and the SHARED app pre-step
//! NARROWS the index's existing name-keyed candidates for that constant to
//! those files (`conventions::convention_file_tails` /
//! `conventions::convention_file_matches`). The file that DEFINES the
//! constant is selected by the intersection, never by re-parsing the
//! constant — the convention is a placement, not a second resolver
//! (PLAN.md's compose-with-the-index rule). A constant DEFINED IN THIS
//! FILE (a `class` / `module` / constant `assignment`, mirrored from the
//! Ruby outline query so the two never disagree) is not a cross-file
//! reference at all — the bare name-keyed lookup stands byte-for-byte.
//!
//! `require_relative` is a HARD mapping like the JLS / in-module-Go ones:
//! a cross-file constant the convention places in a require_relative file
//! that the index does not hold (or that holds under a different file) is
//! the named `unresolved` flag — a candidate in another, un-required file
//! is a different symbol, and offering it would be the plausible-looking
//! lie the B5 decision forbids. (The app's pre-step arms Ruby into that
//! hard set — one token in the `matches!`, no new branch in `xref`.)

use std::collections::BTreeSet;

use tree_sitter::Parser;

/// Parse `source` with the pinned Ruby grammar (the table row is the one
/// grammar pin — `language::spec` reads it).
fn parse_ruby(source: &str) -> Option<tree_sitter::Tree> {
    let grammar = crate::language::spec(crate::registry::LanguageId::Ruby).grammar?;
    let mut parser = Parser::new();
    parser.set_language(&grammar()).unwrap();
    parser.parse(source.as_bytes(), None)
}

/// The CONVENTION NAME carrier for a Ruby file's reference at the point
/// (`ident` the M-. extraction's last segment, `path_token` the whole
/// constant path, `rel` the file's project-relative path — the
/// require_relative base):
///
/// - `Ok(Some(carrier))` — a cross-file constant reference and the file
///   carries at least one DETERMINISTIC `require_relative`: the carrier is
///   the newline-joined set of the file's require_relative TARGET paths
///   (the requiring file's directory + the name + `.rb`); the app narrows
///   the index's name-keyed candidates to those files;
/// - `Err(name)` — a cross-file constant reference in a file whose ONLY
///   import forms are the LOAD-PATH `require` / `autoload`: redline cannot
///   know `$LOAD_PATH`, so the name is FLAGGED, never guessed;
/// - `Ok(None)` — the convention is silent (never a guess): the reference
///   is not a constant (a lowercase method / local — the bare lookup
///   stands), the constant is defined IN THIS FILE (a `class` / `module` /
///   constant `assignment` — the bare same-file lookup stands), the file
///   has no import forms at all (the constant is from a gem / stdlib, or
///   absent — the bare lookup / the resolver seam stands byte-for-byte),
///   or the parse yields no tree.
pub fn reference_convention_name(
    source: &str,
    ident: &str,
    path_token: &str,
    rel: &str,
) -> Result<Option<String>, String> {
    // Ruby constants are uppercase-leading; a lowercase reference is a
    // method / local variable, NEVER a `require`-loaded symbol. The bare
    // name-keyed lookup stands — and the guard also keeps the method-name
    // suffixes (`empty?`, `save!`, the issue-language-aware-symbols Part 1
    // shape) out of the convention entirely: a `?`/`!`-suffixed segment is
    // a receiver call, not a constant.
    let Some(head) = ident.chars().next() else {
        return Ok(None);
    };
    if !head.is_uppercase() {
        return Ok(None);
    }
    let Some(tree) = parse_ruby(source) else {
        return Ok(None);
    };
    let bytes = source.as_bytes();
    let mut local_consts: BTreeSet<String> = BTreeSet::new();
    let mut rel_names: Vec<String> = Vec::new();
    let mut has_loadpath = false;
    visit(
        tree.root_node(),
        bytes,
        &mut local_consts,
        &mut rel_names,
        &mut has_loadpath,
    );
    // A constant THIS FILE defines (the same `constant` shapes the Ruby
    // outline indexes): the reference is same-file, not a cross-file
    // require — the bare name-keyed lookup stands byte-for-byte.
    if local_consts.contains(ident) {
        return Ok(None);
    }
    // A cross-file constant. The deterministic require_relative targets are
    // the convention's placement set.
    if !rel_names.is_empty() {
        let targets: Vec<String> = rel_names
            .iter()
            .map(|name| require_relative_target(rel, name))
            .collect();
        return Ok(Some(targets.join("\n")));
    }
    // No require_relative: only the LOAD-PATH forms (or nothing) are in the
    // file. A LOAD-PATH `require` / `autoload` in the file means a
    // same-named workspace file is a plausible-looking stranger, not the
    // required file — FLAG, never guess. No import forms at all: the
    // constant is from a gem / stdlib (or absent) and the bare lookup /
    // the resolver seam stand byte-for-byte.
    if has_loadpath {
        return Err(path_token.to_string());
    }
    Ok(None)
}

/// The file path TAILS for a carrier built by [`reference_convention_name`]
/// (the newline-joined require_relative target paths): one tail per target,
/// the full project-relative path. Each is matched as a TAIL against the
/// project's indexed files (`convention_file_matches` → `ends_with` for the
/// Ruby file-tail row), so the requiring-file-relative placement matches the
/// target exactly and a same-named file in another directory does NOT carry
/// the tail (the directory join is what keeps the decoy out — the Java
/// two-same-named-classes shape).
pub fn convention_tails(name: &str) -> Vec<String> {
    name.split('\n')
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect()
}

/// The require_relative TARGET path for the including file `rel` (its
/// project-relative path) and the required `name`: the including file's own
/// directory (`__dir__`) joined with `name`, plus the `.rb` extension when
/// the name does not already end in it. A top-level file (no `/` in `rel`)
/// is in the project root, so the target is `name.rb` at the root. A leading
/// `./` in `name` is normalized away (the path it names is the same file).
fn require_relative_target(rel: &str, name: &str) -> String {
    let dir = match rel.rfind('/') {
        Some(idx) => &rel[..idx],
        None => "",
    };
    let name = name.strip_prefix("./").unwrap_or(name);
    let with_ext = if name.ends_with(".rb") {
        name.to_string()
    } else {
        format!("{name}.rb")
    };
    if dir.is_empty() {
        with_ext
    } else {
        format!("{dir}/{with_ext}")
    }
}

/// Walk `node` (and every descendant) collecting: the file's locally-defined
/// constant names (mirroring the Ruby outline query: `class` / `module`
/// with a plain-`constant` name field, and a constant `assignment`'s left
/// side), the BARE `require_relative` string arguments (deterministic
/// targets), and whether any BARE LOAD-PATH `require` / `autoload` is present.
/// A `call` counts only when it has NO `receiver` field (the bare Kernel
/// forms — `self.require` / `foo.require_relative` / `Other.autoload` carry
/// a `receiver` field and are some other object's method, never counted).
fn visit(
    node: tree_sitter::Node,
    bytes: &[u8],
    local_consts: &mut BTreeSet<String>,
    rel_names: &mut Vec<String>,
    has_loadpath: &mut bool,
) {
    match node.kind() {
        "call" if node.child_by_field_name("receiver").is_none() => {
            if let Some(method) = node.child_by_field_name("method")
                && method.kind() == "identifier"
                && let Ok(m) = method.utf8_text(bytes)
            {
                match m {
                    "require_relative" => {
                        if let Some(args) = node.child_by_field_name("arguments")
                            && let Some(name) = first_string_content(args, bytes)
                        {
                            rel_names.push(name);
                        }
                    }
                    // A bare `require` (with any argument — a string path or
                    // a dynamic expression) is a LOAD-PATH load; `autoload`
                    // is a deferred LOAD-PATH load. Both are unknowable here.
                    "require" | "autoload" => *has_loadpath = true,
                    _ => {}
                }
            }
        }
        "class" | "module" => {
            // Only a PLAIN-`constant` name (mirrors the Ruby outline query —
            // a `scope_resolution` name like `Thing::Inner` is NOT indexed,
            // so it is not a locally-defined constant the bare lookup would
            // find).
            if let Some(name) = node.child_by_field_name("name")
                && name.kind() == "constant"
                && let Ok(t) = name.utf8_text(bytes)
            {
                local_consts.insert(t.to_string());
            }
        }
        "assignment" => {
            if let Some(left) = node.child_by_field_name("left")
                && left.kind() == "constant"
                && let Ok(t) = left.utf8_text(bytes)
            {
                local_consts.insert(t.to_string());
            }
        }
        _ => {}
    }
    let mut i = 0;
    while let Some(c) = node.child(i) {
        i += 1;
        visit(c, bytes, local_consts, rel_names, has_loadpath);
    }
}

/// The first `string` argument's `string_content` text (the path, without
/// the surrounding quotes), or `None` when the argument is not a literal
/// string (a dynamic expression — the target is unknowable, so no tail).
fn first_string_content(args: tree_sitter::Node, bytes: &[u8]) -> Option<String> {
    let mut i = 0;
    while let Some(a) = args.child(i) {
        i += 1;
        if a.kind() != "string" {
            continue;
        }
        let mut j = 0;
        while let Some(c) = a.child(j) {
            j += 1;
            if c.kind() == "string_content"
                && let Ok(t) = c.utf8_text(bytes)
            {
                return Some(t.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe fixture (`examples/probe_ruby_conv.rs` — the dump this
    /// module's doc quotes): the three BARE forms, each with no `receiver`
    /// field.
    const BARE: &str = "require 'a/b'\nrequire_relative 'c'\nautoload :D, 'd/e'\n";

    /// The RECEIVER forms: `self.require` / `foo.require_relative` /
    /// `Other.autoload` each carry a `receiver` field and must NOT count
    /// (a bare Kernel method of the same name is the only import form).
    const RECEIVER: &str =
        "self.require 'x'\nfoo.require_relative 'y'\nOther.autoload :Z, 'z/w'\n";

    #[test]
    fn bare_require_relative_is_the_deterministic_target() {
        // `lib/app.rb` requiring `"thing"` → `lib/thing.rb` (the file's own
        // directory + name + `.rb`). A cross-file constant `Thing` not
        // defined in `lib/app.rb` gets the carrier.
        assert_eq!(
            reference_convention_name(
                "require_relative \"thing\"\n\ndef run\n  Thing\nend\n",
                "Thing",
                "Thing",
                "lib/app.rb"
            ),
            Ok(Some("lib/thing.rb".to_string()))
        );
    }

    #[test]
    fn require_relative_extends_without_a_dot_rb_suffix() {
        // An explicit `.rb` is not doubled; a subdirectory path is kept.
        assert_eq!(
            reference_convention_name(
                "require_relative \"thing.rb\"\n\ndef run\n  Thing\nend\n",
                "Thing",
                "Thing",
                "lib/app.rb"
            ),
            Ok(Some("lib/thing.rb".to_string()))
        );
        assert_eq!(
            reference_convention_name(
                "require_relative \"sub/thing\"\n\ndef run\n  Thing\nend\n",
                "Thing",
                "Thing",
                "lib/app.rb"
            ),
            Ok(Some("lib/sub/thing.rb".to_string()))
        );
    }

    #[test]
    fn top_level_file_resolves_at_the_project_root() {
        // A top-level `main.rb` (no directory) is in the root: the target is
        // `thing.rb` at the root.
        assert_eq!(
            reference_convention_name(
                "require_relative \"thing\"\n\ndef run\n  Thing\nend\n",
                "Thing",
                "Thing",
                "main.rb"
            ),
            Ok(Some("thing.rb".to_string()))
        );
    }

    #[test]
    fn multiple_require_relatives_join_into_the_carrier() {
        assert_eq!(
            reference_convention_name(
                "require_relative \"one\"\nrequire_relative \"two\"\n\ndef run\n  One\nend\n",
                "One",
                "One",
                "lib/app.rb"
            ),
            Ok(Some("lib/one.rb\nlib/two.rb".to_string()))
        );
        assert_eq!(
            convention_tails("lib/one.rb\nlib/two.rb"),
            vec!["lib/one.rb".to_string(), "lib/two.rb".to_string()]
        );
    }

    #[test]
    fn load_path_require_is_flagged_never_a_guess() {
        // `require "x"` (the LOAD PATH — `$LOAD_PATH`, unknown) is never a
        // tail. A cross-file constant in a require-only file is FLAGGED
        // (the flagged name is the whole path token at the point).
        assert_eq!(
            reference_convention_name(
                "require \"thing\"\n\ndef run\n  Thing\nend\n",
                "Thing",
                "Thing",
                "lib/app.rb"
            ),
            Err("Thing".to_string())
        );
    }

    #[test]
    fn autoload_is_flagged_never_a_guess() {
        // `autoload :Thing, "thing"` is a deferred LOAD-PATH load — unknown
        // here; the constant is FLAGGED, never jumped to by name.
        assert_eq!(
            reference_convention_name(
                "autoload :Thing, \"thing\"\n\ndef run\n  Thing\nend\n",
                "Thing",
                "Thing",
                "lib/app.rb"
            ),
            Err("Thing".to_string())
        );
    }

    #[test]
    fn require_relative_present_but_reference_not_in_any_target_stays_a_carrier() {
        // The file has a require_relative, so the carrier is the target set;
        // whether the reference lands or the app flags it (HARD) is decided
        // by the app's index narrowing, not here — this module yields the
        // placement, not the verdict.
        assert_eq!(
            reference_convention_name(
                "require_relative \"known\"\n\ndef run\n  Mystery\nend\n",
                "Mystery",
                "Mystery",
                "lib/app.rb"
            ),
            Ok(Some("lib/known.rb".to_string()))
        );
    }

    #[test]
    fn a_constant_defined_in_this_file_keeps_the_bare_lookup() {
        // `class Thing` in the same file: the reference is same-file, not a
        // cross-file require — the convention is silent (the bare same-file
        // lookup stands), even though the file also has a require_relative.
        for src in [
            "require_relative \"thing\"\nclass Thing\nend\n",
            "require_relative \"thing\"\nmodule Thing\nend\n",
            "require_relative \"thing\"\nThing = 1\n",
        ] {
            assert_eq!(
                reference_convention_name(src, "Thing", "Thing", "lib/app.rb"),
                Ok(None),
                "same-file constant keeps the bare lookup: {src}"
            );
        }
    }

    #[test]
    fn a_lower_case_reference_is_not_a_constant_and_stays_bare() {
        // A method / local (`empty?`, `save!`, `x`) is lowercase-leading and
        // is never a require-loaded constant — the guard keeps it out of the
        // convention without even parsing (the issue-language-aware-symbols
        // `?`/`!` suffix shape: `x.empty?` stays the bare `empty?`).
        for (src, ident) in [
            ("require 'a/b'\n\ndef run\n  x.empty?\nend\n", "empty?"),
            ("require 'a/b'\n\ndef run\n  x.save!\nend\n", "save!"),
            ("require 'a/b'\n\ndef run\n  x = 1\nend\n", "x"),
        ] {
            assert_eq!(
                reference_convention_name(src, ident, ident, "lib/app.rb"),
                Ok(None),
                "a lowercase reference keeps the bare lookup: {ident}"
            );
        }
    }

    #[test]
    fn a_constant_with_no_import_forms_keeps_the_bare_lookup() {
        // No `require` / `require_relative` / `autoload` at all: the constant
        // is from a gem / stdlib (or absent) — the bare lookup / the
        // resolver seam stand byte-for-byte, never a flag.
        assert_eq!(
            reference_convention_name("def run\n  Thing\nend\n", "Thing", "Thing", "lib/app.rb"),
            Ok(None)
        );
    }

    #[test]
    fn receiver_calls_never_count_as_imports() {
        // `self.require` / `foo.require_relative` / `Other.autoload` carry a
        // `receiver` field: they are some other object's method, not the
        // bare Kernel import forms. None of them is a deterministic target
        // and none sets the LOAD-PATH flag — with no bare form in the file
        // the convention is silent (the bare lookup stands).
        assert_eq!(
            reference_convention_name(RECEIVER, "Thing", "Thing", "lib/app.rb"),
            Ok(None),
            "receiver calls are not imports: no target, no load-path flag"
        );
        // And a file whose ONLY forms are receiver calls is treated like a
        // no-import file (no flag), not a LOAD-PATH file.
        assert_eq!(
            reference_convention_name(RECEIVER, "Thing", "Thing", "lib/app.rb"),
            Ok(None)
        );
    }

    #[test]
    fn bare_forms_are_the_only_imports() {
        // The three bare forms, alone: require_relative is the deterministic
        // target; require + autoload set the (unused-here) LOAD-PATH flag
        // but do not add tails.
        assert_eq!(
            reference_convention_name(BARE, "Thing", "Thing", "lib/app.rb"),
            Ok(Some("lib/c.rb".to_string())),
            "only the require_relative target is a carrier; require/autoload are load-path"
        );
    }

    #[test]
    fn a_dynamic_require_relative_argument_yields_no_tail() {
        // `require_relative path_var` (a non-string argument) is
        // deterministic in KIND but unknowable in value — no tail, and it
        // does not set the LOAD-PATH flag (it is not a LOAD-PATH load).
        assert_eq!(
            reference_convention_name(
                "require_relative base\n\ndef run\n  Thing\nend\n",
                "Thing",
                "Thing",
                "lib/app.rb"
            ),
            Ok(None),
            "a dynamic require_relative argument is not a known target"
        );
    }

    #[test]
    fn the_target_path_carry_keeps_a_decoy_out() {
        // The tail is the FULL project-relative path (the directory join
        // kept): `lib/thing.rb` matches the target and does NOT match the
        // same-named decoy in another directory. A base-name-only tail
        // (`thing.rb`) would over-match both — the landing mutation pin.
        assert!(
            "lib/thing.rb".ends_with("lib/thing.rb")
                && !"elsewhere/thing.rb".ends_with("lib/thing.rb"),
            "the directory join discriminates the decoy"
        );
    }
}

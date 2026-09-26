//! Bash `source` / `.` semantics for M-. (plan 017, issue 07): the
//! source-form extractor and the relative/absolute-path → file convention.
//!
//! GRAMMAR EVIDENCE (pinned tree-sitter-bash 0.25.1, dumped — not
//! recalled; probe: `examples/probe_bash_conv.rs`):
//!
//! ```text
//! source ./sub/helper.sh
//!   command [..] "source ./sub/helper.sh"
//!     name: command_name [..] "source"        ← the name field wraps a
//!     argument: word [..] "./sub/helper.sh"   word; its text IS the name
//! . ./other.sh
//!   command [..] ". ./other.sh"
//!     name: command_name [..] "."             ← the dot builtin is a
//!     argument: word [..] "./other.sh"        command whose command_name
//! source helper                                        text is exactly `.`
//!   command [..] "source helper"
//!     name: command_name "source"
//!     argument: word "helper"                 ← the BARE name: no `/`
//! source "/abs/path/helper.sh"
//!   command: argument word "/abs/path/helper.sh"   ← absolute: leading `/`
//! source "sub/helper.sh"
//!   command: argument string "\"sub/helper.sh\""   ← a QUOTED argument is a
//!     children: `"`, string_content "sub/helper.sh", `"`
//! source $VAR
//!   command: argument simple_expansion "$VAR"  ← DYNAMIC: a different
//! source "${X:-./sub/helper.sh}"               argument KIND (expansion
//!   command: argument string "${X:-…}"         inside a string)
//!     children: `"`, expansion "${X:-…}", `"`
//! source $(pwd)/x.sh
//!   command: argument concatenation "$(pwd)/x.sh"  ← DYNAMIC
//!     children: command_substitution, word "/x.sh"
//! source ~/bin/helper.sh
//!   command: argument word "~/bin/helper.sh"  ← tilde: home-relative,
//! source ./sub/*.sh                              NOT workspace-relative
//!   command: argument word "./sub/*.sh"       ← glob: a runtime
//!                                                  file SET, not one file
//!
//! `.` in other positions is a DIFFERENT shape (the dot builtin is
//! structurally distinguishable, not text-guessed):
//!   echo .        → command name "echo", argument word "."
//!   x=1.5         → variable_assignment (no command node)
//!   ./run.sh      → command name command_name "./run.sh" (text ≠ ".")
//!   echo a.b      → command name "echo", argument word "a.b"
//!
//! Source forms are NOT top-level-only: they nest (probe: a `command`
//! inside a `function_definition`'s compound_statement and an
//! `if_statement` branch) — the walk is over the whole tree, like the
//! Ruby row. The local-definition shapes (the same-file exclusion):
//!   helper_fn() { :; }      → function_definition name: word "helper_fn"
//!   function wrapped_fn { } → function_definition name: word "wrapped_fn"
//!   function both_fn() { }  → function_definition name: word "both_fn"
//! (all three parse with the `name` field a `word` — the shapes BASH_QUERY
//! indexes; the exclusion mirrors them so the two never disagree).
//!
//! The CONVENTION (the honest limit, stated):
//! - a `source` / `.` FILE that is a RELATIVE PATH (contains a `/`) or an
//!   ABSOLUTE path names a KNOWN file: relative → the sourcing script's
//!   own directory joined with the path (project-relative, the
//!   convention's `rel`); absolute under the workspace root → the
//!   stripped project-relative path; absolute OUTSIDE the root → a known
//!   file redline's index cannot hold (the flag arm, below). Decidable,
//!   no toolchain, no network.
//! - a BARE name (no `/`) is a **`$PATH` lookup** (the bash manual,
//!   `source` / `.` entries: "FILE is searched for in $PATH"; measured
//!   on this box — both `/bin/bash` and `/bin/sh` (dash) find a
//!   slash-less name in `$PATH`: `PATH=/tmp/fakebin bash -c 'source
//!   helper_fn_file'` sources it). Which file that is is
//!   environment-dependent: NEVER a tail, never a same-named-workspace-
//!   file guess (the same rule as the C/C++ angle include, the Go
//!   third-party import, and the Ruby LOAD-PATH `require`).
//! - a DYNAMIC argument (`$VAR`, `${…}`, `$(…)`, concatenation), a GLOB
//!   argument (`*` / `?` / `[` — a runtime file SET, not one file), a
//!   TILDE argument (home-relative — `$HOME` is environment, not
//!   workspace), and an absolute path OUTSIDE the workspace root are
//!   all "a source whose target redline cannot place": the flag arm.
//!
//! THE BASE-DIRECTORY QUESTION (the audit's build-dependent flag —
//! measured on this box, not assumed): POSIX `.` / bash `source`
//! resolve a relative FILE against the **current working directory** at
//! runtime — measured: with the script at `/tmp/x/run.sh` and cwd `/`,
//! both `bash run.sh` and `dash run.sh` fail to find `./sub/helper.sh`,
//! and both succeed when cwd IS the script's directory. (The audit's
//! "in practice the script's dir under some shells" does not hold for
//! bash or dash on this box.) The cwd is UNKNOWABLE to a static tool —
//! the user may run the script from anywhere — so resolving against it
//! is undecidable by construction. The ONLY static base is the script's
//! own directory (known from `rel`), and that is this row's rule — the
//! same join the Ruby `require_relative` row uses (`__dir__`
//! semantics): `scripts/run.sh` sourcing `./sub/helper.sh` places the
//! names in `scripts/sub/helper.sh`. The divergence is stated, not
//! papered over: a run from a cwd OTHER than the script's directory
//! targets a different file; the failure mode of the approximation is
//! the honest miss (the target is not indexed at the script-dir
//! placement → the named `unresolved` flag), and it becomes a wrong
//! file only in the triple coincidence where a DIFFERENT indexed file
//! defines the same name at the script-dir placement AND the user ran
//! from another cwd — bounded, and the same accepted-heuristic class as
//! the Java source-root and Clojure layout rows (a wrong jump is worse
//! than an honest miss; the alternative rule — flag every relative
//! source because the cwd is unknowable — is undecidable, not safer).
//!
//! The REFERENCE the M-. extraction produces at the point is a bare
//! name (bash has no path-shaped container: the word rule is
//! alnum + `_` and the `word` node is not identifier-ish, so the
//! extraction stays bare — `ident == path_token` always). The
//! convention does NOT name a file by the name's own text — it yields
//! the file's PLACEABLE source targets (path tails) and the shared app
//! pre-step NARROWS the index's existing name-keyed candidates for that
//! name to those files (`conventions::convention_file_tails` /
//! `conventions::convention_file_matches` — the `ends_with` file-tail
//! row; the directory join in the tail keeps a same-named file in
//! another directory out, the Java two-same-named-classes shape). A
//! name DEFINED IN THIS FILE (a `function_definition`, mirrored from
//! BASH_QUERY) is not a cross-file reference at all — the bare
//! name-keyed lookup stands byte-for-byte.
//!
//! The mapping is HARD like the JLS / in-module-Go / require_relative
//! ones: a cross-file name the convention places in a source target the
//! index does not hold — or that only a BARE / dynamic `$PATH`-sourced
//! file would provide, with a same-named stranger indexed elsewhere —
//! is the named `unresolved` flag (the app's pre-step arms Bash into
//! the hard set — a token in the `matches!` gate and in each hard arm;
//! a candidate in an unsourced file is a plausible-looking lie, never a
//! confident jump).

use std::collections::BTreeSet;
use std::path::Path;

use tree_sitter::Parser;

/// Parse `source` with the pinned Bash grammar (the table row is the one
/// grammar pin — `language::spec` reads it).
fn parse_bash(source: &str) -> Option<tree_sitter::Tree> {
    let grammar = crate::language::spec(crate::registry::LanguageId::Bash).grammar?;
    let mut parser = Parser::new();
    parser.set_language(&grammar()).unwrap();
    parser.parse(source.as_bytes(), None)
}

/// The CONVENTION NAME carrier for a Bash file's reference at the point
/// (`ident` the M-. extraction's bare name — `ident == path_token` in
/// Bash, the word rule is alnum + `_` and there is no path-shaped
/// container; `project_root` the workspace root, needed only for
/// absolute paths; `rel` the file's project-relative path — the
/// relative-target base):
///
/// - `Ok(Some(carrier))` — the reference is not defined in this file and
///   the file carries at least one PLACEABLE `source` / `.` target (a
///   relative path — the sourcing file's own directory joined with the
///   path — or an absolute path under the workspace root — stripped):
///   the carrier is the newline-joined set of those targets; the app
///   narrows the index's name-keyed candidates to those files;
/// - `Err(name)` — the reference is not defined in this file and the
///   file's source forms are ONLY unplaceable — a BARE name (the `$PATH`
///   lookup: the bash manual says FILE is searched for in `$PATH`), a
///   dynamic / glob / tilde argument, or an absolute path outside the
///   workspace root: redline cannot know which file provides the name,
///   so it is FLAGGED, never guessed (a same-named workspace file is a
///   plausible-looking stranger, not the sourced one);
/// - `Ok(None)` — the convention is silent (never a guess): the name is
///   a function defined IN THIS FILE (the bare same-file lookup stands),
///   the file has no `source` / `.` forms at all (the bare lookup / the
///   resolver seam stands byte-for-byte), or the parse yields no tree.
pub fn reference_convention_name(
    source: &str,
    ident: &str,
    path_token: &str,
    project_root: &Path,
    rel: &str,
) -> Result<Option<String>, String> {
    let tree = match parse_bash(source) {
        Some(t) => t,
        None => return Ok(None),
    };
    let bytes = source.as_bytes();
    let mut local_fns: BTreeSet<String> = BTreeSet::new();
    let mut targets: Vec<String> = Vec::new();
    let mut has_unplaceable = false;
    visit(
        tree.root_node(),
        bytes,
        project_root,
        rel,
        &mut local_fns,
        &mut targets,
        &mut has_unplaceable,
    );
    // A function THIS FILE defines (the same `function_definition` shape
    // BASH_QUERY indexes): the reference is same-file, not a cross-file
    // source — the bare name-keyed lookup stands byte-for-byte, even
    // when the file also sources other files (a later source can shadow
    // the local definition at runtime, but the file's own definition is
    // the name-keyed answer — never a flag, never a narrowing to an
    // unrelated sourced file).
    if local_fns.contains(ident) {
        return Ok(None);
    }
    // A cross-file reference. The placeable targets are the convention's
    // placement set.
    if !targets.is_empty() {
        return Ok(Some(targets.join("\n")));
    }
    // No placeable target: only UNPLACEABLE forms (or nothing) are in
    // the file. An unplaceable source form means a same-named workspace
    // file is a plausible-looking stranger, not the sourced one —
    // FLAG, never guess. No source forms at all: the name is from
    // somewhere the file does not declare (the environment, an
    // exported function, or nowhere) and the bare lookup / the resolver
    // seam stand byte-for-byte.
    if has_unplaceable {
        return Err(path_token.to_string());
    }
    Ok(None)
}

/// The file path TAILS for a carrier built by [`reference_convention_name`]
/// (the newline-joined placeable target paths): one tail per target, the
/// full project-relative path. Each is matched as a TAIL against the
/// project's indexed files (`convention_file_matches` → `ends_with` for
/// the Bash file-tail row), so the sourcing-file-relative placement
/// matches the target exactly and a same-named file in another directory
/// does NOT carry the tail (the directory join is what keeps the decoy
/// out — the Java two-same-named-classes shape).
pub fn convention_tails(name: &str) -> Vec<String> {
    name.split('\n')
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect()
}

/// The project-relative TARGET for the sourcing file `rel` and a
/// relative `target` (the `source` / `.` argument, already known to
/// contain a `/` and carry no dynamic / glob / tilde characters): the
/// sourcing file's own directory (the static base — see the module
/// doc's base-directory measurement) joined with the target, with `.`
/// and `..` components normalized. `None` when the join escapes the
/// project root (a leading `..` chain past the root — the target is not
/// a project-relative path, so it cannot be a tail) or resolves to no
/// file at all (a bare directory).
fn relative_target(rel: &str, target: &str) -> Option<String> {
    let mut parts: Vec<&str> = rel
        .split('/')
        .filter(|p| !p.is_empty())
        .collect();
    // Drop the file itself: the base is the file's OWN directory.
    parts.pop();
    for comp in target.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// Classify the first `argument` of a `source` / `.` command:
/// `Some(project-relative target)` when it names a placeable file,
/// `None` when the target cannot be placed (the unplaceable arm). The
/// shapes (probe-quoted in the module doc): a plain `word` and a
/// `string` whose only named child is a single `string_content` carry a
/// STATIC path; every other argument kind (`simple_expansion`,
/// `concatenation`, a `string` with an `expansion` child, …) is DYNAMIC.
fn first_argument_target(
    node: tree_sitter::Node,
    bytes: &[u8],
    project_root: &Path,
    rel: &str,
) -> Option<String> {
    let mut i = 0;
    let mut first: Option<tree_sitter::Node> = None;
    while let Some(c) = node.child(i) {
        i += 1;
        if node.field_name_for_child(i as u32 - 1) == Some("argument")
            && first.is_none()
        {
            first = Some(c);
            break;
        }
    }
    let first = first?;
    let text = match first.kind() {
        "word" => first.utf8_text(bytes).ok()?.to_string(),
        "string" => {
            // A quoted argument is a static path only when its SINGLE
            // named child is the `string_content` (an `expansion` child
            // — `"${X:-…}"` — is dynamic, and so is any other shape).
            let mut named: Vec<String> = Vec::new();
            let mut j = 0;
            while let Some(c) = first.child(j) {
                j += 1;
                if c.is_named() {
                    named.push(c.kind().to_string());
                }
            }
            if named != ["string_content".to_string()] {
                return None;
            }
            let content = first
                .child_by_field_name("value")
                .or_else(|| {
                    let mut k = 0;
                    while let Some(c) = first.child(k) {
                        k += 1;
                        if c.kind() == "string_content" {
                            return Some(c);
                        }
                    }
                    None
                })?;
            content.utf8_text(bytes).ok()?.to_string()
        }
        _ => return None,
    };
    placeable_target(&text, project_root, rel)
}

/// The path-text classification (the rule, stated in terms of the
/// cleanest line — a path containing a `/` is a PATH, a bare name is a
/// LOOKUP):
/// - empty / dynamic (`$`, backtick) / glob (`*` / `?` / `[`) / tilde
///   (`~/…` — home-relative, environment, not workspace) → `None`;
/// - absolute (`/…`): under the workspace root → the stripped
///   project-relative path; outside it → `None` (a known file the
///   project's index cannot hold);
/// - relative (contains a `/`): the sourcing file's own directory
///   joined with the path (escaping the root → `None`);
/// - BARE (no `/`): the `$PATH` lookup → `None` (never a tail).
fn placeable_target(text: &str, project_root: &Path, rel: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    if text
        .chars()
        .any(|c| matches!(c, '$' | '`' | '*' | '?' | '['))
    {
        return None;
    }
    if text.starts_with('~') {
        // `~/bin/x.sh` (and the degenerate bare `~`): home-relative —
        // a `$HOME` lookup, not a workspace-relative path.
        return None;
    }
    if text.starts_with('/') {
        // Absolute: only an absolute path UNDER the workspace root is a
        // project-relative placement.
        let stripped = Path::new(text)
            .strip_prefix(project_root)
            .ok()?
            .to_string_lossy()
            .into_owned();
        if stripped.is_empty() {
            return None;
        }
        return Some(stripped);
    }
    if text.contains('/') {
        // A relative path: the static base is the sourcing file's own
        // directory (the module doc's measurement and rule).
        relative_target(rel, text)
    } else {
        // A BARE name: the `$PATH` lookup (bash manual: "FILE is
        // searched for in $PATH") — never a tail, never a guess.
        None
    }
}

/// Walk `node` (and every descendant) collecting: the file's
/// locally-defined function names (mirroring BASH_QUERY's
/// `function_definition` `word` name shapes), the PLACEABLE `source` /
/// `.` targets (project-relative), and whether any UNPLACEABLE source
/// form is present. A `command` counts as a source form only when its
/// `command_name` field's text is exactly `source` or `.` — the dot
/// builtin is structurally distinguished from a `.` in any other
/// position (an argument word, a `./path` command name, an assignment:
/// probe-quoted in the module doc).
fn visit(
    node: tree_sitter::Node,
    bytes: &[u8],
    project_root: &Path,
    rel: &str,
    local_fns: &mut BTreeSet<String>,
    targets: &mut Vec<String>,
    has_unplaceable: &mut bool,
) {
    match node.kind() {
        "command" => {
            if let Some(name) = node.child_by_field_name("name")
                && name.kind() == "command_name"
                && let Ok(n) = name.utf8_text(bytes)
                && (n == "source" || n == ".")
            {
                match first_argument_target(node, bytes, project_root, rel) {
                    Some(target) => targets.push(target),
                    None => *has_unplaceable = true,
                }
            }
        }
        "function_definition" => {
            if let Some(name) = node.child_by_field_name("name")
                && name.kind() == "word"
                && let Ok(t) = name.utf8_text(bytes)
            {
                local_fns.insert(t.to_string());
            }
        }
        _ => {}
    }
    let mut i = 0;
    while let Some(c) = node.child(i) {
        i += 1;
        visit(c, bytes, project_root, rel, local_fns, targets, has_unplaceable);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::LanguageId;
    use std::path::Path;

    /// The test root: a nonexistent absolute path (pure string work —
    /// the absolute-path arm strips against it; no disk read, ever).
    const ROOT: &str = "/nonexistent-redline";

    fn cnr(source: &str, ident: &str, rel: &str) -> Result<Option<String>, String> {
        reference_convention_name(source, ident, ident, Path::new(ROOT), rel)
    }

    /// The probe fixture (`examples/probe_bash_conv.rs` — the dump this
    /// module's doc quotes): the relative `source` form.
    #[test]
    fn relative_source_target_is_the_sourcing_file_directory_join() {
        // `scripts/run.sh` sourcing `./sub/helper.sh` → the names are
        // placed in `scripts/sub/helper.sh` (the sourcing file's own
        // directory + the path — the static base, the module doc's rule).
        assert_eq!(
            cnr(
                "source ./sub/helper.sh\nmain_fn() {\n  helper_fn\n}\n",
                "helper_fn",
                "scripts/run.sh"
            ),
            Ok(Some("scripts/sub/helper.sh".to_string())),
            "the relative target joins the sourcing file's directory"
        );
    }

    /// The `.` special builtin is the SAME carrier (a `command` whose
    /// `command_name` text is exactly `.` — structurally a source form,
    /// probe-quoted).
    #[test]
    fn dot_builtin_is_the_same_carrier() {
        assert_eq!(
            cnr(". ./sub/other.sh\nmain_fn() {\n  helper_fn\n}\n", "helper_fn", "scripts/run.sh"),
            Ok(Some("scripts/sub/other.sh".to_string()))
        );
        // A relative path WITHOUT the `./` prefix is the same form.
        assert_eq!(
            cnr(". sub/other.sh\n", "helper_fn", "scripts/run.sh"),
            Ok(Some("scripts/sub/other.sh".to_string()))
        );
    }

    /// A top-level file (no directory in `rel`) is in the project root:
    /// the target is at the root.
    #[test]
    fn top_level_script_resolves_at_the_project_root() {
        assert_eq!(
            cnr("source ./sub/helper.sh\n", "helper_fn", "run.sh"),
            Ok(Some("sub/helper.sh".to_string()))
        );
    }

    /// A quoted string argument carries the same static path (the
    /// `string` → `string_content` shape, probe-quoted).
    #[test]
    fn quoted_path_argument_is_the_same_carrier() {
        assert_eq!(
            cnr("source \"./sub/helper.sh\"\n", "helper_fn", "scripts/run.sh"),
            Ok(Some("scripts/sub/helper.sh".to_string()))
        );
    }

    /// An absolute path UNDER the workspace root is the stripped
    /// project-relative target; one OUTSIDE the root is unplaceable
    /// (a known file the project's index cannot hold — the flag arm).
    #[test]
    fn absolute_paths_under_and_outside_the_root() {
        assert_eq!(
            cnr(
                format!("source {ROOT}/sub/helper.sh\n").as_str(),
                "helper_fn",
                "scripts/run.sh"
            ),
            Ok(Some("sub/helper.sh".to_string())),
            "an absolute path under the root strips to its project-relative target"
        );
        assert_eq!(
            cnr("source /opt/lib/helper.sh\n", "helper_fn", "scripts/run.sh"),
            Err("helper_fn".to_string()),
            "an absolute path outside the root is unplaceable: flagged, never a guess"
        );
    }

    /// `..` components normalize; a chain escaping the project root is
    /// unplaceable (the target is not a project-relative path).
    #[test]
    fn dotdot_normalizes_and_escaping_dotdot_is_unplaceable() {
        assert_eq!(
            cnr("source ../sub/helper.sh\n", "helper_fn", "scripts/deep/run.sh"),
            Ok(Some("scripts/sub/helper.sh".to_string()))
        );
        assert_eq!(
            cnr("source ../../../out.sh\n", "helper_fn", "scripts/deep/run.sh"),
            Err("helper_fn".to_string()),
            "a target escaping the root is unplaceable: flagged, never a guess"
        );
    }

    /// The BARE name is the `$PATH` lookup (measured on this box: both
    /// bash and dash find a slash-less name in `$PATH`): never a tail,
    /// FLAGGED for a cross-file reference — the flagged name is the
    /// reference at the point, never a same-named-workspace-file guess.
    #[test]
    fn bare_source_name_is_flagged_never_a_guess() {
        assert_eq!(
            cnr("source helper\nmain_fn() {\n  helper_fn\n}\n", "helper_fn", "scripts/run.sh"),
            Err("helper_fn".to_string()),
            "a bare name is a $PATH lookup: flagged, never a guess"
        );
        // The bare name is the SAME form under the `.` builtin.
        assert_eq!(
            cnr(". helper\nmain_fn() {\n  helper_fn\n}\n", "helper_fn", "scripts/run.sh"),
            Err("helper_fn".to_string())
        );
    }

    /// The DYNAMIC arguments (`$VAR`, `${…}`, `$(…)`) are runtime
    /// lookups: unplaceable → the flag arm (never a tail — the value is
    /// unknowable statically).
    #[test]
    fn dynamic_arguments_are_unplaceable_and_flag() {
        for src in [
            "source $VAR\n",
            "source \"${X:-./sub/helper.sh}\"\n",
            "source $(pwd)/x.sh\n",
        ] {
            assert_eq!(
                cnr(src, "helper_fn", "scripts/run.sh"),
                Err("helper_fn".to_string()),
                "a dynamic argument is a runtime lookup: `{src}`"
            );
        }
    }

    /// A GLOB argument is a runtime file SET (not one file) and a TILDE
    /// argument is home-relative (environment, not workspace): both are
    /// unplaceable → the flag arm.
    #[test]
    fn glob_and_tilde_arguments_are_unplaceable_and_flag() {
        assert_eq!(
            cnr("source ./sub/*.sh\n", "helper_fn", "scripts/run.sh"),
            Err("helper_fn".to_string()),
            "a glob names a runtime file set, not one file"
        );
        assert_eq!(
            cnr("source ~/bin/helper.sh\n", "helper_fn", "scripts/run.sh"),
            Err("helper_fn".to_string()),
            "a tilde path is a $HOME lookup, not a workspace path"
        );
    }

    /// A file that has BOTH a placeable and an unplaceable form: the
    /// placeable targets are the carrier (the app decides the flag for a
    /// name none of them holds — the hard arm); the unplaceable form
    /// does not add tails it cannot know.
    #[test]
    fn mixed_forms_yield_the_placeable_targets_only() {
        assert_eq!(
            cnr(
                "source helper\nsource ./sub/helper.sh\nmain_fn() {\n  helper_fn\n}\n",
                "helper_fn",
                "scripts/run.sh"
            ),
            Ok(Some("scripts/sub/helper.sh".to_string())),
            "the placeable target is the carrier; the bare form adds no tail"
        );
    }

    /// Multiple placeable targets join into the carrier (newline-split,
    /// like the Ruby / C rows' carriers).
    #[test]
    fn multiple_targets_join_into_the_carrier() {
        assert_eq!(
            cnr(
                "source ./a/x.sh\n. ./b/y.sh\nmain_fn() {\n  helper_fn\n}\n",
                "helper_fn",
                "scripts/run.sh"
            ),
            Ok(Some("scripts/a/x.sh\nscripts/b/y.sh".to_string()))
        );
        assert_eq!(
            convention_tails("scripts/a/x.sh\nscripts/b/y.sh"),
            vec!["scripts/a/x.sh".to_string(), "scripts/b/y.sh".to_string()]
        );
    }

    /// Source forms NEST (function bodies, `if` branches — probe-quoted):
    /// the walk is over the whole tree.
    #[test]
    fn nested_source_forms_count() {
        assert_eq!(
            cnr(
                "wrap_fn() { source ./nested/x.sh; }\nmain_fn() {\n  helper_fn\n}\n",
                "helper_fn",
                "scripts/run.sh"
            ),
            Ok(Some("scripts/nested/x.sh".to_string()))
        );
    }

    /// A function THIS FILE defines keeps the bare same-file lookup
    /// byte-for-byte — even when the file also sources other files (the
    /// same-file exclusion, mirroring BASH_QUERY's definition shapes).
    #[test]
    fn a_function_defined_in_this_file_keeps_the_bare_lookup() {
        for src in [
            "source ./sub/helper.sh\nwrap_fn() { :; }\n",
            "source helper\nwrap_fn() { :; }\n",
            "wrap_fn() { :; }\nfunction helper_fn { :; }\n",
        ] {
            assert_eq!(
                cnr(src, "wrap_fn", "scripts/run.sh"),
                Ok(None),
                "a same-file function keeps the bare lookup: {src}"
            );
        }
    }

    /// No `source` / `.` forms at all: the convention is silent — the
    /// bare lookup / the resolver seam stand byte-for-byte, never a flag.
    #[test]
    fn a_file_with_no_source_forms_is_silent() {
        assert_eq!(
            cnr("main_fn() {\n  helper_fn\n}\n", "helper_fn", "scripts/run.sh"),
            Ok(None)
        );
    }

    /// A `source`-NAMED name that is not a source form: `echo .`,
    /// `./run.sh` (a PATH execution, not the dot builtin), and a
    /// variable named like a path are not source forms (probe-quoted).
    #[test]
    fn dot_in_other_positions_is_not_a_source_form() {
        for src in [
            "echo .\nmain_fn() {\n  helper_fn\n}\n",
            "./run.sh\nmain_fn() {\n  helper_fn\n}\n",
            "x=1.5\nmain_fn() {\n  helper_fn\n}\n",
        ] {
            assert_eq!(
                cnr(src, "helper_fn", "scripts/run.sh"),
                Ok(None),
                "a `.` in another position is not a source form: {src}"
            );
        }
    }

    /// Extra arguments after the path are positional parameters, not
    /// files: only the FIRST argument is the target.
    #[test]
    fn only_the_first_argument_is_the_target() {
        assert_eq!(
            cnr(
                "source ./a/x.sh --flag val\nmain_fn() {\n  helper_fn\n}\n",
                "helper_fn",
                "scripts/run.sh"
            ),
            Ok(Some("scripts/a/x.sh".to_string()))
        );
    }

    /// The target path's directory join is what keeps a same-BASE-name
    /// decoy in another directory out (the Java two-same-named-classes
    /// shape). A base-name-only tail would over-match both — the landing
    /// mutation pin.
    #[test]
    fn the_target_path_carry_keeps_a_decoy_out() {
        // The tail is the FULL project-relative path (the directory join
        // kept): `scripts/sub/helper.sh` matches the target and does NOT
        // match the same-named decoy in another directory.
        assert!(
            "scripts/sub/helper.sh".ends_with("scripts/sub/helper.sh")
                && !"elsewhere/sub/helper.sh".ends_with("scripts/sub/helper.sh"),
            "the directory join discriminates the decoy"
        );
    }

    /// The dispatch table row (the `conventions`-side pin): the same
    /// shapes through the shared entry point.
    #[test]
    fn the_dispatch_row_answers_per_the_module() {
        assert_eq!(
            crate::conventions::convention_name_for_reference(
                LanguageId::Bash,
                "source ./sub/helper.sh\n",
                "helper_fn",
                "helper_fn",
                Path::new(ROOT),
                "scripts/run.sh"
            ),
            Ok(Some("scripts/sub/helper.sh".to_string()))
        );
        assert_eq!(
            crate::conventions::convention_name_for_reference(
                LanguageId::Bash,
                "source helper\n",
                "helper_fn",
                "helper_fn",
                Path::new(ROOT),
                "scripts/run.sh"
            ),
            Err("helper_fn".to_string())
        );
        assert_eq!(
            crate::conventions::convention_file_tails(LanguageId::Bash, "scripts/a/x.sh"),
            Some(vec!["scripts/a/x.sh".to_string()])
        );
        // The match predicate: the file-tail row is `ends_with`
        // (byte-for-byte the pre-step's always-behavior).
        assert!(crate::conventions::convention_file_matches(
            LanguageId::Bash,
            "scripts/a/x.sh",
            "scripts/a/x.sh"
        ));
        assert!(!crate::conventions::convention_file_matches(
            LanguageId::Bash,
            "scripts/a/x.sh",
            "elsewhere/a/x.sh"
        ));
    }
}

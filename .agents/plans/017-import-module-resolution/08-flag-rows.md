# 017-08 — the flag rows (the residual)

**Base:** `378a577` (worktree slot 2, branch `flag-rows`). **Code + tests:**
`b6397a7`. **Docs:** the follow-up docs commit (this file + the
`docs/language-coverage.md` flag rows).

Written **last on purpose** (the worklist's 08 row): these are the cases
plan 017 has decided **cannot** be resolved statically, named *after*
issues 01–07 landed the resolvable set (Clojure, Java, C/C++ quoted
includes, Go in-module, Ruby `require_relative`, Bash relative
`source`/`.`) — so the residual is exactly what is left, not a guess.
Two halves: **(a) VERIFY** the flag behaviour in the real code path
(measure, do not assume — the B5 named-flag rule landed in issue 04),
and **(b) DOCUMENT** the residual (this file + the grid's flag rows).

## What we decided cannot be resolved — per case: decision, reason, what the user sees

| case | cannot be resolved because | the user sees (measured, not assumed) | pinned by |
|---|---|---|---|
| **C#** (any unindexed name; `using` directives have no file mapping) | no directory convention — a namespace may live ANYWHERE in the project (Roslyn/csc take explicit file lists); the only correct resolution is a project-wide "find the declaration" search, which the plan excludes (non-goal); no tooling provider | the named `unresolved: `<name>`` flag; no picker, no jump — never the enclosing method's picker (the pre-B5 misroute) | `xref_csharp_unresolvable_call_is_unresolved_not_misrouted` |
| **Scheme** (bare library references — `(import (library (foo core)))` → `foo:bar`) | library layout is implementation-defined: R6RS/R7RS mandate the library FORM, not its file mapping (Chicken / Gambit / Racket each differ; Racket has no `.scm` convention at all); no toolchain provider | `unresolved: `foo:bar`` (the whole Lisp word, extracted whole by the word rule) | `xref_scheme_unresolvable_library_name_is_unresolved_not_misrouted` |
| **Ruby bare `require` / `autoload`** | `require "x"` goes through `$LOAD_PATH` (gem `lib/` roots under Bundler, env-dependent) — unknowable offline, no provider | `unresolved: `<CONST>`` (the issue-06 conventions pre-step's `Err` arm — app-level M-. path, not just the ruby.rs unit) | `xref_ruby_load_path_require_is_flagged_not_a_guess`, `xref_ruby_autoload_is_flagged_not_a_guess` |
| **C++ semantic resolution** (ADL / templates / using-directives / angle includes) | needs semantic (compiler-level) resolution; the angle include's `-I` / system path is not in the tree at all (`system_lib_string` is a leaf with no content child — the structural `-I` limit, issue 04); no C++ tooling provider | `unresolved: `swap`` for a `swap(a, b)` call whose declaration lives only in `<algorithm>` — measured NOT to land on the enclosing `run`, and NOT on any plausible same-named symbol | `xref_cpp_semantic_name_is_unresolved_not_a_guess` |
| **Python relative imports** (`from . import x`) | the enclosing package's sys.path root is unknown from the buffer path, and a relative import binds the enclosing PACKAGE — never an installed module | BARE shape: the provider's own honest miss (`no provider resolution for `helper`: bare symbol `helper` has no module path; resolving it to a module needs scope info (tree-sitter) not yet provided by the app …` — a named miss, no jump; live-measured, unchanged pre/post). DOTTED shape: the named `unresolved: `json.dumps`` flag (see the finding below) | `xref_python_relative_import_dotted_use_is_unresolved_not_misresolved`, `xref_python_relative_import_bare_use_keeps_the_provider_miss` |

## The finding: the Python relative import MISRESOLVED — fixed (the one code change)

Python is the only residual case **with** a provider, so "does it flag?"
was a question, not an assumption. Measured in the real app path
(bounded PTY probe, one App; fixture: `pkg/util.py` with
`from . import json` + `def render(): return json.dumps({})`, the local
submodule **absent** from the project tree so the index misses `dumps`):

- **Pre-fix (live):** M-. on `json.dumps` → the tooling seam probed
  `json` as an **absolute** module → **landed in the stdlib json**,
  `/usr/lib/python3.14/json/__init__.py:185` — a plausible-looking
  wrong jump for a name the file says is a local relative submodule.
  A misresolution, exactly the shape the plan's "flag, never guess"
  rule forbids.
- **Post-fix (live):** `unresolved: `json.dumps`` — no picker, no jump,
  the view stays on `pkg/util.py`.
- **Positive (must not regress, re-measured live post-fix):**
  `import json` (ABSOLUTE) + `json.dumps({})` → still lands in the
  stdlib json (`/usr/lib/python3.14/json/__init__.py:185`), no fetch
  prompt — the gate battery's 011-06 L-P1 leg pins the same shape.
- **Bare shape (unchanged, byte-for-byte, live-measured both sides):**
  `from . import helper` + M-. on bare `helper` → the provider's own
  miss (above) — an honest named miss, never the `unresolved` flag,
  never a jump.

**The fix (minimal, disclosed — a code fix was genuinely needed):**
the scope-hint walk (`python_import_path_for_symbol`,
`src/app/store/navigation/imports.rs`) now records the effective
binding as `Absolute(path)` / `Relative` instead of dropping the
relative import (`None`) — the same walk, the same re-import-shadowing
rule, and `python_scope_for`'s output is **byte-for-byte unchanged**
(a relative binding yields no hint, exactly as before; 28
`resolver_scope_*` pins re-run green). The tooling seam
(`start_symbol_resolution` in `definitions.rs` — the ONE site every
resolver fall-through passes through: project path, external buffers,
and the forced list `M->`) consults `python_relative_import_binds` on
the token's **first segment**: a relative binding reports the named
`unresolved` flag instead of probing the chain. No provider code
changed; every other language and every non-relative binding stays on
the chain byte-for-byte.

## The guard that matters most — the flags do not swallow the resolvable set

Every flagged case is pinned **in both directions** (the flagged form
flags AND the things that do resolve in the same language still
resolve):

| flagged form (negative pin) | the same-language positive that still lands |
|---|---|
| C# `ExternalThing.Go()` → `unresolved: `Go`` | `xref_csharp_property_access_lands_in_project_index` (in-project definitions) |
| Scheme `foo:bar` → `unresolved: `foo:bar`` | **NEW** `xref_scheme_in_workspace_definition_still_lands` (cross-file in-workspace defn → picker → RET lands) |
| Ruby `require "thing"` → `unresolved: `Thing`` | `xref_ruby_require_relative_narrows_to_required_file_and_lands` (issue 06) |
| C++ `swap(a, b)` → `unresolved: `swap`` | `xref_cpp_quoted_include_narrows_to_convention_header_and_lands` (issue 04) |
| Python `from . import json` + `json.dumps` → `unresolved: `json.dumps`` | **NEW** `xref_python_absolute_import_dotted_use_still_reaches_the_tooling_seam` + the 011-06 L-P1 live stdlib landing (gate battery) |
| Bash bare `source helper` → `unresolved: `helper_fn`` | `xref_bash_relative_source_narrows_to_sourced_file_and_lands` (issue 07) |

## Mutations (executed, both directions, at the sites the tests exercise)

- **Flag case → made to resolve anyway:** the seam's guard disabled
  (`if false && self.python_relative_import_flags(…)` in
  `start_symbol_resolution`) → **`xref_python_relative_import_dotted_
  use_is_unresolved_not_misresolved` reddened** (definitions.rs:3370 —
  the message reverts to the tooling seam's miss instead of
  `unresolved: `json.dumps``). Reverted.
- **Positive case → broke the resolution:** `ruby::
  reference_convention_name` (crates/redline-syntax/src/ruby.rs — the
  conventions pre-step the landing test exercises end-to-end, NOT the
  ruby.rs unit that would prove nothing at the app level) stubbed to
  `Ok(None)` → **`xref_ruby_require_relative_narrows_to_required_file_
  and_lands` reddened** (definitions.rs:2370 — the picker offers BOTH
  the `elsewhere/thing.rb` decoy and `lib/thing.rb`; the narrowing is
  gone, the jump is wrong). Reverted.

## Verification (measured — all first-run)

- `cargo test --workspace`: 1097 + 136 + 7 + 2 + 2 + 1 + 1 + 231 + 5
  passed, 0 failed (7 new tests: six flag/guard pins in
  `definitions.rs` + the binding-detection table in the imports test
  module).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `tools/gate.sh full` (foreground): **GATE EXIT 0, first run** —
  build 1 s, clippy 5 s, test 115 s; sweep.py 23 s (18/18 flows);
  drive_all 20 s; windowing 2 s / panes 2 s; check_cursor_stream 85 s;
  ux_sweep 9 s; probe_notes_dump 13 s; current-line-tint 3 s / 2 s;
  syntax-notes 2 s (4/4); symbol-precise 10 s (33/33); drive_xref 11 s
  (16/16); external-notes 3 s (6/6); external-crate 3 s (4/4);
  external-use 3 s (5/5); 011-01 3 s (4/4); 011-02 8 s (5/5 — the bare-
  imported `dumps` stdlib landing); 011-04 4 s (8/8); 011-05 17 s
  (20/21 live + the go leg's LOUD skip — toolchain absent, unit-covered);
  011-06 5 s (6/6 — the `json.dumps` / `fakelib.apply` dotted LANDINGS
  still live); F2 6 s (9/9); sweep_flows 34 s (18/18).
- PTY probe (throwaway, `/tmp/flag08/` — bounded: one App, three legs,
  fixed deadlines; the repo's regression pins are the unit tests above,
  and for the no-provider languages the async wrapper adds nothing to
  the message, per the 00-audit layer-2 note): pre-fix and post-fix
  logs at `/tmp/flag08/probe-{pre,post}.log`.

## Residual (recorded, not fixed)

- Plan-013's cursor interleave (~2 % product race) did NOT fire in any
  of the gate's PTY suites on this run — nothing to attribute.
- A relative import inside an EXTERNAL (landed-dependency) buffer is
  covered by the same seam guard; the flag there is honest because the
  owning crate's index already indexes the package's files (the index
  hit precedes the seam). Not separately pinned — no distinct code
  path.
- The probe's cursor placement (goto-line lands at col 0; exact columns
  via C-f byte offsets) was verified by CUP capture before each leg.

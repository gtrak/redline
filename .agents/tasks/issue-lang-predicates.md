# Task: basic node predicates + scope walks for the uncovered languages

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `.agents/skills/tree-sitter/SKILL.md` (ground truth for query/grammar
work) and `.agents/tasks/issue-011-03-impl.md` (the landed pattern for
js/ts/python/go — MIRROR it, including the whole-path rule and the NODE-
TYPES probe discipline).

## Origin (user directive)

The user wants basic support for MANY languages. `src/syntax/node.rs`'s
`node_at`/`scope_path_at` (the per-language M-. machinery, 011-03 +
011-06) currently covers Rust/JS/TS/TSX/Python/Go; the other languages
return None (pinned by `unimplemented_languages_return_none`). This
issue implements the basic predicates + scope walks for the six
remaining languages so path-shaped M-. works on them too (the 011-06
seam is generic — it consumes whatever `node_at` returns).

## Languages (6): C, Cpp, Markdown, Bash, Toml, Json

For each, decide honestly what "basic" means per grammar (NODE_TYPES
probe first, NOT memory — throwaway probe test, removed before commit;
the 011-03 corrections lesson):

- **C / Cpp**: identifier predicates (the whole-path rule per grammar:
  C/C++ member access `a.b.c` (cpp direct_member_access /
  field_expression `->`?) — decide what a "path container" even is per
  grammar; C has `->`/`.` member selection; C++ adds `::` (ALREADY the
  app's `::` scan for Rust — cpp `::` paths: handle honestly or defer);
  scope walks: function/struct/class nesting.
- **Markdown**: headings are the outline; is there a path-shaped
  concept? Likely N/A for path-shaped (no member access) — implement
  the predicate/scope walk ONLY if meaningful (e.g. scope = enclosing
  section by heading level for symbol context); else document N/A and
  pin it.
- **Bash**: `cmd.subcmd`? Function/variable scope walks (a bash "scope"
  is the file — file-scoped like Go). Judge minimally.
- **Toml / Json / Yaml**: dotted KEY paths (`[table.sub]`, nested json
  keys) — these ARE path-shaped: M-. on a dotted key could jump via the
  outline (definition query exists). Node predicates: identifier kinds
  per grammar. This could be genuinely useful (config navigation).
- Every language that turns out to have NO meaningful path/scope
  concept: implement the identifier predicate only (bare M-. context)
  or pin N/A honestly — the 011-03 degradation discipline.

## What lands

`src/syntax/node.rs` (predicates + walks + tests per language; extend
`unimplemented_languages_return_none`'s set honestly as languages land),
+ the language-coverage doc rows get a heads-up comment if the coverage
lane hasn't landed yet (coordinate: the coverage lane is docs-only; if
it landed first, UPDATE your rows in `docs/language-coverage.md` too).

## Constraints

- Gate: `cargo test --workspace` + clippy (read the clippy exit) +
  `tools/gate.sh full` (flock, `cargo build` first). Budget ~55 tool
  calls; honest-stop at half (commit per language — stage the work).
- Scope fence: `src/syntax/node.rs` (+ tests), `docs/language-coverage.md`
  (your rows only — the coverage lane owns the doc's existence),
  `docs/provider-matrix.md` only if a cell goes stale. NO app/store
  changes (the 011-06 seam consumes node_at generically — no store edit
  should be needed; if you find one is, STOP and report).
- Parallel lanes: coverage (docs) + rung3 review finishing — node.rs is
  yours alone.

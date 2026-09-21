# issue-json-yaml-no-symbols — stop treating JSON/YAML keys as symbols

## Why (user directive, backed by a measurement)

A profiler run on a real 3,596-file project reported **252,985 total symbols**, of
which **JSON + YAML were 228,772 — 90%**. JSON alone was 208,069 symbols from 193
files (those ~1 MB files carrying 11,000–15,000 symbols each), and YAML 20,703 from 87.

The user's decision: **"json/yaml yes stop treating the keys as symbols."**

Keys are not navigation targets. They inflate the symbol index by an order of
magnitude, dominate symbol search and `M-.` candidate relevance, and cost memory
and assembly time for something nobody jumps to.

## What to change

The symbol source is already table-driven: each `LanguageSpec` carries
`definition_query: Option<&'static str>` (`crates/redline-syntax/src/language.rs`),
and the extraction engine runs it. JSON and YAML currently point at `JSON_QUERY` /
`YAML_QUERY`.

**Verify the mechanism first, then choose — and state which you chose and why:**
- **(a) `definition_query: None`** for JSON/YAML. The cleanest expression of "no
  symbols", but the field's doc says `None` only for Plain today, so this may widen
  a documented invariant — check every consumer of `definition_query` tolerates
  `None` rather than assuming.
- **(b) keep the query and discard its output** — worse: it pays the full query cost
  for nothing.
- **(c) narrow the query** (e.g. top-level keys only) — only if you find a real
  consumer that needs it. Otherwise it is scope creep against an explicit directive.

Whichever you pick: the **highlight** query must be untouched. JSON/YAML must still
syntax-highlight exactly as before. Only *symbol extraction* changes.

## What must NOT break (check each, and say what you verified)

- **The resolver does not read JSON symbols.** Verified before speccing: it reads
  `package.json` from disk (`crates/redline-resolve/src/cargo.rs:402`;
  `providers/js_provider.rs:470` looks for the file with `is_file()`). So dependency
  resolution is unaffected — but **re-verify for every provider**, not just cargo/js.
- **Imenu/outline for a JSON/YAML buffer** becomes empty if it was symbol-derived.
  Decide and state whether that is acceptable — it is the direct consequence of the
  directive. Do **not** silently keep a second symbol path alive to preserve it.
- **Project search** matches *text*, not symbols, so it must be unaffected. Confirm.
- Any test pinning JSON/YAML symbol **counts** will change. Per the test-authority
  rule the requirement wins, but every changed assertion must be reported with
  **before/after**, and must not be weakened to hide a regression (turning "JSON
  yields 11,107 symbols" into "JSON yields something" is not acceptable).

## Acceptance

- A test asserting JSON and YAML buffers yield **zero** symbols, **and** that a
  control language (Rust/TypeScript) is **unaffected** — the second half is what
  proves you changed only the intended languages.
- A test proving **highlighting still works** for JSON/YAML (faces unchanged). This
  is the regression that would otherwise hide behind the symbol change.
- The fixture's symbol total drops accordingly, with the index/assembly cost
  reported before/after (`--index-profile`).
- `cargo test --workspace` + clippy `-- -D warnings` clean; `tools/gate.sh full` green.

## Fence

`crates/redline-syntax/src/{language,queries}.rs` primarily, plus any test pinning
JSON/YAML symbols and any store-side consumer you find. Disclose anything in `src/`
with before/after.

**Do NOT touch `src/ui/**` or the transient-highlight work** — the `jump-highlight`
lane currently holds those files, and two writers on one file is forbidden.

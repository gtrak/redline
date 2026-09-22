# issue-lang-highlight-parity — bring non-Rust highlighting up to Rust's level

**Reported by the user (spot-checking):** *"the typescript highlighting is pretty lacking
inside a function body"* and *"within C++, I'm just spot-checking, I think probably a lot of
languages are not fully supported to the level of rust."*

## The measured cause (verify it yourself before changing anything)

We pass **each grammar's own `HIGHLIGHTS_QUERY`** straight through
(`crates/redline-syntax/src/language.rs`, e.g. `Some(tree_sitter_typescript::HIGHLIGHTS_QUERY)`),
so richness is whatever the upstream grammar ships. Measured line counts of the upstream
`queries/highlights.scm` in the vendored grammar crates:

| language | upstream query | capture kinds |
|---|---|---|
| Rust | **161 lines** | 21 |
| C++ | **70 lines** | 6 |
| TypeScript | **35 lines** | 5 |

That is the whole story behind "TypeScript is lacking inside a function body": the grammar
simply does not describe locals/parameters/etc. **Reproduce it first** — dump the highlighted
spans for a small TypeScript snippet with a function body containing parameters and locals,
and show which spans come out with **no face**. Put that before/after in the commit.

## Requirements

1. **Fix the capture names we silently drop.** Our face list (`HIGHLIGHT_FACES` in
   `crates/redline-syntax/src/highlight.rs`, 28 names) does not cover every capture the
   grammars emit, so those spans render in the default colour **in every language, including
   Rust**. Measured unmapped: `@escape` (rust, python, go, ruby), `@delimiter` (c), `@module`
   (c-sharp). Add whatever faces are needed (or map the names onto existing faces where that
   is the honest choice) and assert the mapping with a test. State the full before/after list
   of unmapped captures per language — re-measure it rather than trusting this one.
2. **Ship our own highlight queries for the thin languages instead of the upstream ones.**
   Start with **TypeScript** (the reported case), then **C++**. Put them in the crate
   (`crates/redline-syntax/queries/<lang>/highlights.scm`) and wire them through
   `language.rs`. The natural approach is a **superset**: start from the upstream query (it is
   available locally in the vendored grammar crate's `queries/` directory — read it there, do
   not fetch anything) and extend it with the patterns the grammar supports but upstream
   omits (locals, parameters, properties, member access, builtins, escapes, …). Confirm the
   node/field names you use **against the grammar's `node-types.json`** in the same vendored
   crate, not from memory — an invalid query fails at load and a wrong node name silently
   never matches.
3. **Prove the improvement, per language, with a before/after measurement** — e.g. the count
   of distinct capture kinds the query yields over a representative snippet, and the count of
   spans that end up with no face. Do not claim "better highlighting" without numbers.
4. **A rendering-level test for the reported case**: a TypeScript function body whose
   parameters and locals get faces, asserted through the same API the TUI uses (the
   `highlight_source`/`highlight` path), so the test fails if the query regresses.
5. **Do not regress the languages that are already good** (Rust, JavaScript, Java, C#, Ruby,
   Python all have substantial upstream queries). If you touch shared logic, show the other
   languages' measurements are unchanged or better.
6. **Keep the injected/embedded-language behaviour intact** (`injections_query`, markdown
   fenced code, etc.) — those flows are pinned by tests; do not break them.

## Acceptance

* A per-language before/after table (capture kinds; spans with no face) in the commit, for at
  least TypeScript, C++, and Rust as the reference.
* A test proving a TypeScript function body's parameters/locals are highlighted (the reported
  case), plus one for C++.
* The previously-unmapped captures now resolve to a face, asserted.
* Every language's query still **loads** (an invalid query is a load-time error, so a
  smoke test over all languages matters more than any single assertion).
* `cargo test --workspace` + clippy `--workspace --all-targets -- -D warnings` clean.

## Fence

`crates/redline-syntax/` (new `queries/` files, `language.rs`, `highlight.rs`, tests) and the
theme/style tables if a new face needs a colour. Disclose anything else with before/after.

## Notes

* A new face name needs a style in whatever maps faces → colours; find that mapping and wire
  it, or the span will resolve to a face that renders as nothing.
* This is a **quality** change, not a parity feature: do not add new commands.
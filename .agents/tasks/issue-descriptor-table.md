# Task: the per-language descriptor table (D1) — collapse 8 sync sites into 1

## Read this first
`.agents/plans/archive/012-project-organization.md` is the
**full design**: the 8 sync sites, the proposed `LanguageSpec` struct, the 19-row
inventory (what every site currently says per language), which parts are mechanically
collapsible vs. need real thought, and a 7-step migration order. Follow it. Where the
design and this file disagree, the design wins (it was written from a read of the code).

## Why
Adding a language today means editing **six or more hand-synced match sites in three
files** (~110 duplicated arms). Worse, three of the gaps are *correctness*:
- the **grammar is pinned in three places** (`registry::build`, `queries::language_for`,
  `highlight::reuse_language`) and only `queries::language_for`'s pin is covered by the
  ABI guard test — a miss in the other two **silently falls back to plain text**;
- `registry::highlight_query_for` **duplicates `build`'s highlights arms** arm-for-arm
  (including the two vendored `include_str!` arms);
- `node::parse_source`'s language whitelist is a **fourth identity list** duplicating
  `LanguageId::ALL` — adding a language without it makes M-. dead with **zero compile error**.

## Required outcome
One `LanguageSpec` table (19 rows) as the single source of truth, with the declarative
duplication removed from: `registry::{name, ALL, ext_map, build, highlight_query_for}`,
`queries::{query_for, language_for}`, `highlight::{supports_reuse, reuse_language}`,
`tokens::token_class_query_for`, and `node::parse_source`'s whitelist (plus the
identifier-kind lists, which become data — the *algorithms* stay put).

**Keep what is genuinely per-language algorithm**: `node::in_identifier_position`
(Ruby/JSON position gates), `is_path_segment`, the 13 scope walkers, `flat_define_kind`
(head-text gating), and the reuse *policy* rationale. The table removes declarative
duplication only.

## Hard constraints
1. **Fence: `src/syntax/*.rs` only** (registry, queries, highlight, node, tokens) plus
   `docs/`. **Do NOT touch `src/app/store.rs`** — keep `query_for` / `language_for` (and
   any other name the app imports) available as thin wrappers or re-exports over the
   table so the ~10 `store.rs` call sites compile unchanged. If you believe a store.rs
   change is unavoidable, **stop and report** rather than colliding with another lane.
2. **Behavior must be identical.** The backstops are the registry ABI guard
   (`all_grammars_set_language_succeeds`) and `highlight.rs`'s
   `reusable_pipeline_is_byte_identical_to_highlighter`. Run them after every step.
3. **Preserve the pinned const identity** for the vendored highlights
   (`C_SHARP_HIGHLIGHTS`, `CLOJURE_HIGHLIGHTS`): if you move the `include_str!`s, the
   **F-8 checksum test just landed in `queries.rs` must keep working** (move it with the
   consts if that is cleaner, and keep the recorded sha256s intact).
4. **Migration order = the design's 7 steps**, each compiling and test-green before the
   next: (1) add `language.rs` with the table + a sync test; (2) registry reads the table,
   delete `highlight_query_for`; (3) queries read the table (wrappers for store.rs);
   (4) `supports_reuse` becomes a field read, delete `reuse_language` (the third grammar
   pin goes away); (5) `node.rs`: whitelist → `parseable()`, kind lists → data;
   (6) `tokens.rs` → field read; (7) delete the emptied matches, re-run the backstops.
5. **Add the sync test** the design specifies: every non-Plain row has name + ≥1
   extension + grammar + definition query; extensions round-trip through the resolver;
   and the reuse/local s-policy cross-check. It must be **discriminating** (state how you
   verified it fails when a row is wrong).
6. Honest-stop at half budget: a partial migration that is green and leaves the tree
   consistent is a valid landing (say exactly which steps landed). **Never leave the tree
   in a half-migrated state that compiles but disagrees between two sites.**

## Gate
`cargo build` first, then after each step `cargo test -p redline --lib` (fast), and at the
end `cargo test --workspace`, `cargo clippy --workspace --all-targets` (read
`${PIPESTATUS[0]}`), `timeout 900 tools/gate.sh full`.
**Resource guard**: `export CARGO_BUILD_JOBS=4`; if `free -g` "available" < 8 GB, run only
the lib tests and report the full gate as deferred (do not sleep-wait).
Budget ~60 tool calls. Report: which steps landed, the table's row count, every deleted
duplication site, the sync test + its discriminating evidence, gate counts, and any site
you judged must stay per-language.

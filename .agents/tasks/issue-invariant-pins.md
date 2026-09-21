# Task: pin the silent-divergence invariants

## Why this exists
The round-2 cleanup audit found a class of risk that no style debt rivals:
**two places that must agree, with nothing testing that they agree.** Each item
below is a latent correctness bug that would surface as a confusing, hard-to-diagnose
behaviour difference. Pinning them is cheap, and it hardens the refactors that follow
(the descriptor table, the store split) by turning implicit contracts into failing tests.

**Every pin must be DISCRIMINATING**: state in your report how you verified it fails
when the invariant is violated (e.g. temporarily flip the condition and watch it fail).
A pin that cannot fail is worse than no pin.

## Item 1 — `supports_reuse(id)` must agree with the registry's locals queries (C13)
The incremental highlighter is byte-identical to the full `Highlighter` **only** for
languages whose registry config passes an **empty locals query**. Today
`highlight::supports_reuse` (a non-exhaustive `matches!`) and the registry's per-language
locals-query choice are **two hand-maintained lists**, so adding a locals query to, say,
Python silently desyncs the incremental path from the full path — and the byte-identity
test only covers the languages it names.
- Expose the registry fact (e.g. `has_locals_queries(id)` / a spec accessor).
- Add a test over **`LanguageId::ALL`**: `supports_reuse(id) == !has_locals_queries(id)`,
  so a future language or a future locals query fails loudly.
- Files: `src/syntax/highlight.rs`, `src/syntax/registry.rs` (+ tests).

## Item 2 — pin what each walker does, and fix the comments that lie (C14 + C8)
There are **four** file walkers with **no cross-pins**:
- `model/files.rs::FileList::build` — hidden-skip + gitignore + `graft/` prune
- `search/rg.rs::run` — hidden-skip + gitignore, **no** `graft/` prune
- `crates/redline-resolve/src/cargo.rs::walk_rs_files` — `target/` + dot-dirs, **no** gitignore
- `nav/index.rs` — built from `FileList`
Documented as intentional, but untested. Two consequences to pin and one comment to fix:
1. **Pin the user-visible asymmetry**: a file under `graft/` is found by **search** but
   **absent** from the file list (one test each, same fixture).
2. **Investigate before pinning**: for every difference you find (including `target/` and
   hidden dirs), decide whether it is *intended* or a *bug*. Pin intent; **report anything
   that looks like a bug rather than silently pinning it** — e.g. if search walks `target/`
   and returns build-artifact hits, that is a product question, not a test to write.
3. **Fix the stale comments** in `src/model/files.rs`: the header claims the watcher is
   unshipped (it shipped — `src/app/watcher.rs`) and that search uses a `.ignore`-based
   walk (it is **gitignore**-based and never reads `.ignore`). `src/syntax/cache.rs` has
   the same stale "watcher arrives in issue 04" line.
- Files: `src/model/files.rs`, `src/search/rg.rs` (+ tests); comment fixes only elsewhere.

## Item 3 — finder and search must agree on gitignore (R3)
`model/files.rs` and `search/rg.rs` implement the **same per-entry ancestor-chain
gitignore semantics independently** (the files.rs comment admits it). They must agree:
a file visible in the finder but ignored in search (or vice versa) is user-visible.
- Cheapest honest step: (a) extract the shared decision (gitignore construction +
  `matched(..).is_ignore()`) into one helper both call, and (b) add an **agreement test**
  running both walkers over one fixture with a **nested** `.gitignore` (root + subdir,
  including a negation and a directory rule) and asserting identical file sets —
  **modulo the documented `graft/` difference from Item 2** (state the modulo explicitly).
- Files: `src/model/files.rs`, `src/search/rg.rs` (+ tests).

## Item 4 — the vendored highlight queries have no integrity check (F-8)
`C_SHARP_HIGHLIGHTS` and `CLOJURE_HIGHLIGHTS` are `include_str!`s of
`third_party/tree-sitter-c-sharp-0.23.5/highlights.scm` and
`third_party/tree-sitter-clojure-0.1.0/highlights.scm`. Their provenance is recorded
**only in comments** ("sha256 at copy time"), so a local edit to a vendored file is
undetectable.
- Add a test that hashes both files and compares against the sha256 recorded **in the
  test** (compute them once, paste them, and state in the test that they are the
  copy-time hashes).
- Files: a test alongside the consts (`src/syntax/queries.rs` or the module you judge
  cleanest). Do not modify the vendored files.

## Fence and gate
Fence: `src/syntax/{highlight,registry,queries}.rs`, `src/model/files.rs`,
`src/search/rg.rs`, and tests inside those modules. **No `src/app/store.rs`, no UI, no
`crates/redline-resolve` source.** If an item genuinely needs a file outside the fence,
stop and report rather than widening it silently.
Gate: `cargo build` first, then `cargo test --workspace`, `cargo clippy --workspace --all-targets`
(read `${PIPESTATUS[0]}`), and `tools/gate.sh full`.
Budget ~45 tool calls; honest-stop at half. Report per item: the pin, the discriminating
evidence, the walker policy table (Item 2), anything you escalated as a probable bug, gate counts.

# Plan 014 — crate extraction (from the measured import graph)

**Status:** analysis complete, nothing implemented. Stage 1 is high-confidence;
stages 2–3 are viable and staged. Not a rewrite — each stage is a move.

## 1. Why

The workspace is `redline` (the bin, ~43 k lines across `src/`) plus
`crates/redline-resolve` (5,782). Everything else lives in one crate, so **every
edit recompiles and relinks the whole binary**, and there is no enforcement that
the layering the code *claims* is the layering it *has*. The crate boundary is the
only mechanism that makes a layering claim mechanically true.

This is not a speculative refactor: `src/syntax/mod.rs` already declares its own
rule — *"Syntax highlighting: grammar registry, highlight pipeline, and cache.
Plain Rust — zero iocraft/tokio (plan layering rule)"* — and exposes 7 `pub mod`s.
The boundary was designed; it was never materialized.

## 2. The evidence: module sizes and the import graph

Measured (production + test lines):

| Module | Lines | Files |
|---|---|---|
| `src/app` | 30,310 | 38 |
| `src/syntax` | 6,663 | 10 |
| `src/ui` | 4,456 | 20 |
| `src/git` | 3,262 | 12 |
| `src/model` | 2,368 | 7 |
| `src/search` | 1,567 | 4 |
| `src/nav` | 1,155 | 6 |
| `src/theme.rs` | — | 1 |
| `crates/redline-resolve` | 5,782 | 6 |

Cross-module edges (`use crate::X` present in module Y; `-` = none):

```
           app    git   model    nav   perf  search  syntax  theme     ui
    app      -      1      1      1      .      1      1      1      .
    git      .      -      1      .      .      .      .      .      .
   model     .      1      -      .      .      1      .      .      .
    nav      .      .      .      -      .      .      1      .      .
   perf      1      .      1      .      -      1      .      .      .
 search      .      .      1      .      .      -      1      .      .
 syntax      .      .      .      .      .      .      -      .      .
  theme      .      .      .      .      .      .      1      -      .
     ui      1      .      1      .      .      1      .      1      -
```

Two facts decide the whole plan:

1. **`syntax` is a TRUE LEAF** — its row is empty. Nothing in it reaches outside
   itself. Its consumers are `app`, `nav`, `search`, `theme`, and the surface they
   need is small and already `pub`:
   `queries::` (4 uses), `registry::resolve_language` (+2), `highlight::` (+2, and
   `HIGHLIGHT_FACES` in `theme.rs:9`), `tokens::comment_string_ranges`, `cache::`.
2. **The only cycle is `model ↔ git`, and BOTH back-edges are TEST-ONLY:**
   - `git → model` is one line: `src/git/repo/tests.rs:8` (`use crate::model::sections::StatusTree`) — a test file.
   - `model → search` is one line: `src/model/files.rs:311` inside `#[test] fn finder_and_search_agree_on_gitignore`.
   - The *production* direction is `model/sections.rs` → `git::diff` / `git::status` types (that file builds a `Section` tree from git data).

   So `git` is a production leaf; the test edge becomes a **dev-dependency**
   (cargo permits a dev-dep cycle, and it is not a build cycle).

## 3. What a crate boundary actually buys (and costs)

**Buys**
- **Incremental compile isolation — MEASURED, and it is structural, not wall-clock.**
  Stage 1 (`518bd94`) measured it: an app-touch rebuild now shows `Compiling redline`
  only, with `redline-syntax` fingerprinted and skipped — the isolation is real. But
  the **wall-clock win is nil on this machine**: local clean rebuild 4.90s → 4.62s,
  and touching `src/app/store/mod.rs` 1.02s → 0.99s. The 51k-line bin compiles in
  ~1s here, so 6.6k lines (13%) do not move the needle. **The honest case for the
  extraction is therefore the other three bullets** (enforced layering, a home for the
  ABI pins, a focused test loop) — plus isolation that would matter on a slower
  machine or in CI, not a speedup you can feel locally. Do not sell stage 2/3 on
  compile time without re-measuring on the target machine.
- **Parallel codegen** — cargo builds independent crates concurrently.
- **Enforced layering.** A crate cannot reach into another's internals; the
  "zero iocraft/tokio" claim becomes a compile error, not a comment.
- **A focused test loop**: `cargo test -p redline-syntax` instead of the whole bin.
- **A home for the 19 tree-sitter grammar pins**, whose staged-verdict discipline
  (the ABI matrix) currently lives in the bin's `Cargo.toml`.

**Costs** (state these honestly)
- Every cross-boundary item needs a real `pub` API decision. `pub(crate)` cannot
  cross a crate boundary — this is the same class of work as the 73 `pub(super)`
  the store split measured, but it must be *designed* rather than compiler-forced.
- A change to a leaf crate still rebuilds all its dependents. The win is
  directional (app-only edits), not absolute.
- Extraction churn: imports, `Cargo.toml` moves, `docs/architecture.md`.

## 4. Recommendation

| Stage | Crate | Verdict |
|---|---|---|
| **1** | `redline-syntax` | **DO IT.** Proven leaf, already designed as one, owns 19 heavy external deps, 6,663 lines. Highest confidence of anything here. |
| **2** | `redline-git` | **DO IT (after 1).** Production leaf (3,262 lines); its one inbound edge is test-only → dev-dep. Also the natural moment to promote the shared git test harness to `crates/redline-testutil`. |
| **3** | `redline-model` | **CONTINGENT.** Depends on `git` in production, so it is mid-tier, not a leaf. Requires a decision on `model/sections.rs` (it reads git types and builds the magit section tree — arguably a *view-model*, not a leaf model). |
| — | `redline-ui` | **NO (now).** Blocked: `ui → app::store` (`AppStore`, `ViewId`, `BufferRow`, `ResultRow`). Extracting it today yields `redline-ui → redline-app`, the wrong direction. The fix is to move those view-model types into `model` and invert the edge to `app → ui` — a real improvement, but a design change, not a move. |
| — | `nav`, `search`, `theme` | **NO.** 1,155 / 1,567 / small. Too little code to earn a boundary; they ride along in the bin. |
| — | `redline-app` | **NO.** The bin's core; extracting it would only add an API surface with no isolation benefit. |

**The one-line version:** extract the two provable leaves (`syntax`, then `git`),
leave everything downstream in the bin, and revisit `model`/`ui` only if the
view-model ownership question is answered first.

## 5. Success criteria

- `cargo build --workspace`, `cargo test --workspace`, and
  `cargo clippy --workspace --all-targets -- -D warnings` stay green at every stage.
- **Zero behaviour change** — each stage is a move plus `pub` surface; no logic edits.
- The test count is unchanged at each stage — **and stage 1 measured it**: base 867
  (bin) → 713 (bin) + 154 (`redline-syntax`) = 867, with a name-set diff of zero
  vanished / zero new / zero remapped. Resolver 123 + integration 7,2,3,1,1 unchanged.
  (This bullet previously said "the bin's 854" — stale before the extraction landed.)
- `cargo tree` shows one `tree-sitter` runtime in the graph (the ABI rule) — **verified
  after stage 1**: a single v0.25.10, with the 17 grammars hanging off `redline-syntax`
  and the *runtime* a direct dep of **both** crates (`imports.rs`/`notes.rs` parse with
  `tree_sitter::` directly — which the original leaf analysis missed).
- `docs/architecture.md` reflects the new crate map.
- A measurable compile win for an app-only edit (report `cargo build` wall time
  before/after for a touched `src/app` file — reasoning plus a number, not theater).

## 6. Task order

| Issue | Depends on | Notes |
|---|---|---|
| `01-extract-syntax.md` | — | The leaf. Do this first; it validates the whole approach. |
| `02-extract-git.md` | 01 | Includes promoting the git test harness to `crates/redline-testutil`. |
| `03-extract-model.md` | 02 + the `sections.rs` decision | Not specced yet — write the issue when the decision is made. |

## 7. Risks

- **Feature unification.** `tree-sitter` and the grammars move to the syntax
  crate's `[dependencies]`; verify no feature gets unified differently (compare
  `cargo tree -e features` before/after for the `tree-sitter` subtree).
- **`#[cfg(test)]` leakage.** If anything outside `syntax` uses a test-only item
  from it, that item must be gated properly rather than made `pub`.
- **The `-D warnings` gate** is unforgiving of a moved-but-unused `use`; expect a
  tidy-up pass in the moving commit.
- **Do not let a stage mix a move with a behaviour change** — same rule as plan 012.

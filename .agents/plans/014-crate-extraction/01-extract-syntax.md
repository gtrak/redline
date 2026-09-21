# 014-01 — extract `redline-syntax` from the bin

**Objective.** Move `src/syntax/` (6,663 lines, 10 files) into
`crates/redline-syntax`, a workspace library crate, and make the bin depend on it.
**A pure move plus a `pub` surface** — no logic changes.

**Why this one first:** it is a **proven leaf** (`src/syntax` contains no
`use crate::` outside itself — verified), it already declares its own layering rule
(*"Plain Rust — zero iocraft/tokio"*, `src/syntax/mod.rs`), all 7 submodules are
already `pub mod`, and it owns the 19 pinned tree-sitter grammar dependencies.
Stage 1 validates the extraction approach for stages 2–3.

## Key decisions

- **The crate is `crates/redline-syntax`.** The workspace root already has
  `members = ["crates/*"]`, so the crate joins automatically — do not edit the
  members list.
- **Move the 19 `tree-sitter*` dependencies** from the bin's `[dependencies]` into
  `crates/redline-syntax/Cargo.toml`, **with their comments intact** (the comments
  carry the ABI-pinning verdicts and the "do NOT bump further" rules; they are
  load-bearing documentation, not decoration). Also move `tree-sitter-highlight`.
  Anything else `src/syntax` actually uses moves too (check: `unicode-width`?
  `ropey`? `serde`? — decide from the imports, and report).
- **Preserve the pin semantics exactly.** The ABI rule is one runtime in the graph;
  verify with `cargo tree` that the `tree-sitter` runtime is still a single
  version and that no grammar's dev-dependency leaked into the build graph.
- **`pub` surface is compiler-driven, but *designed*, not sprayed.** `pub(crate)`
  cannot cross the boundary. Start from the measured consumers and let `cargo build`
  enumerate the rest:
  `queries::` (4 uses in the bin), `registry::resolve_language` (+2),
  `highlight::` (+2, plus `HIGHLIGHT_FACES` used by `src/theme.rs:9`),
  `tokens::comment_string_ranges`, `cache::`. Report every item you widened to
  `pub` and why it is genuinely part of the crate's contract (not an internal).
- **The crate's own tests come with it.** `src/syntax` has tests in
  `node/mod.rs`, `queries.rs`, `highlight.rs`, `language.rs`, `cache.rs`,
  `registry.rs`, `tokens.rs`. They move and must run as
  `cargo test -p redline-syntax`; the workspace test total must not drop.
- **Do not touch `docs/tree-sitter-runtime-matrix.md`'s verdicts** — only update
  path references if any point at the moved files.
- **No behaviour change.** No rename, no reorder, no refactor while moving.

## Files

| File | Change |
|---|---|
| `crates/redline-syntax/Cargo.toml` | new: package + the moved deps |
| `crates/redline-syntax/src/*` | the moved `src/syntax/*` (mod.rs → lib.rs) |
| `Cargo.toml` (root) | remove the moved deps; add `redline-syntax` to `[dependencies]` |
| `src/syntax/` | deleted |
| every `use crate::syntax::…` | → `use redline_syntax::…` |
| `docs/architecture.md` | the crate map |

## Steps

1. Scaffold `crates/redline-syntax` with `Cargo.toml` + `src/lib.rs` (the old
   `mod.rs` content), and `git mv` the files in. Compile immediately to get the
   first error list.
2. Move the deps in the same commit; `cargo build -p redline-syntax` must pass.
3. Rewrite the bin's imports (`use crate::syntax::` → `use redline_syntax::`) —
   `rg -l 'crate::syntax' src/` is the worklist.
4. Fix the `pub` surface from the compiler's errors. Report each widening.
5. Tidy unused imports (the `-D warnings` gate will name them).

## Verification

- `cargo build --workspace`; `cargo test --workspace`;
  `cargo clippy --workspace --all-targets -- -D warnings` (read `${PIPESTATUS[0]}`).
- `cargo test -p redline-syntax` — the moved tests, count reported.
- **Test-count reconciliation**: report the per-target `test result:` lines before
  and after. The workspace total must be unchanged (tests relocated, not deleted).
- `cargo tree -i tree-sitter` (or `-e features`) — one runtime, no new feature
  unification.
- `timeout 900 tools/gate.sh full` — the PTY battery must stay green (the bin's
  rendering is untouched, so this is a regression check, not a feature check).
- **Compile-time evidence**: report `cargo build` wall time for (a) a clean build
  and (b) touching one file under `src/app/` — before and after. This is the
  payoff claim; measure it rather than asserting it.

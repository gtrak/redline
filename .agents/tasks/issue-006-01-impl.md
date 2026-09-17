# Task: Implement plan 006 issue 01 — resolver chain + Rust/cargo provider

You are the implementation worker. Repo root is your cwd. This spec is
self-contained. `.agents/skills/*.md` are authoritative ground truth.

Context: the repo is a Cargo workspace (root package `redline` = the app;
`crates/redline-resolve` = a NEW app-free resolver crate with a skeleton:
ToolingProvider trait, SymbolContext, ResolvedSource, crate_from_symbol).
Your work happens ONLY inside `crates/redline-resolve/` — the app crate
(src/, plan 004/005 lanes) is OFF LIMITS. This issue runs in parallel with
UX lanes; do not touch them. Operator directive (supersedes the generic
no-fetch rule for this issue ONLY): fetching real source via cargo is
sanctioned here.

## Working agreement

- Skills are truth (.agents/skills/*.md); no registry READING beyond what
  cargo commands provide; no docs.rs browsing.
- Write-first: scaffold provider + chain early, compile, iterate.
- graft CLI available.
- Skill corrections: minimal, listed.

## Read first

1. `.agents/plans/006-tooling-aware-jump/01-resolver-chain.md` — the issue.
2. `crates/redline-resolve/src/lib.rs` — the skeleton you extend.
3. `.agents/skills/git2/SKILL.md` (path conventions context), and any
   cargo/skills relevant.

## What to build

1. **Provider chain** (`Resolver`): ordered providers, first hit wins;
   transparent trace of attempts (for logging later).
2. **Rust/cargo provider** (`CargoProvider`, "rust"): resolve a symbol
   context to concrete source, workspace-miss only:
   - Parse the crate from the symbol (crate_from_symbol; refined by
     scope info when available).
   - `cargo metadata` (offline-capable) on the workspace root → package →
     manifest path → workspace or registry source dir.
   - Registry misses: `cargo fetch` (sanctioned), then re-locate under
     $CARGO_HOME/registry/src/... (path pattern per cargo docs knowledge in
     skills; verify against the real layout at runtime — code reality wins).
   - Workspace members resolve INSIDE the workspace (external=false).
   - Once the source dir is located, locate the FILE for the symbol with
     ripgrep-class search (grep crates are already deps of the workspace —
     but keep this crate's deps minimal: plain fs walk + content scan is
     fine; note perf expectations).
   - Symbol-in-file: best-effort line locate (definition-shaped regex per
     Rust item kinds: fn/struct/enum/trait/impl/const/type). Return
     ResolvedSource; the app does precise placement later.
3. **Failure modes**: unknown crate → clean Err (no panic); fetch refused
   (offline) → Err with the reason; crate found but symbol file not
   located → Err naming the crate + dir searched.
4. **Tests** (tempdir workspaces): workspace member resolves internal
   (external=false); dependency on a fetched crate resolves to the
   registry dir (external=true) — use a real tiny crates.io dep (e.g.
   `anyhow`) in the fixture so fetch is exercised; symbol file locate for
   a known item (e.g. anyhow's `Error` type); offline-refusal error path.

## Constraints

- Deps: serde/serde_json/anyhow already declared; add ONLY if truly needed
  (prefer std). No iocraft/store imports (layering).
- Scope fence: crates/redline-resolve/** ONLY.

## Verification

- `cargo test -p redline-resolve` all green (incl. the fetch test; mark it
  #[ignore] ONLY if the sandbox blocks crates.io — check first, don't
  assume).
- `cargo test` (workspace) all green; `cargo build` clean; clippy -D
  warnings clean.
- The full test suite of the app lane must remain untouched-green (you
  don't run their suites; just don't break the workspace build).

## Report format

- Provider design + resolution trace example (workspace miss → cargo
  metadata → registry path → file → line). Gate outputs (exact counts).
  Skill corrections; deviations; known gaps (languages not yet covered).

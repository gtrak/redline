# Task: redline-resolve — JavaScript/TypeScript provider (parallel lane)

You are the implementation worker. Repo root is your cwd.

**Scope fence (absolute): you own exactly ONE file:
`crates/redline-resolve/src/providers/js_provider.rs`.** Other providers
(go_provider.rs, python_provider.rs) belong to parallel lanes — never touch
them, never touch lib.rs, Cargo.toml, src/, or tools/. Your module is
pre-declared; your `#[cfg(test)] mod tests` lives inside your file.

Context: `crates/redline-resolve` is an app-free resolver crate. Read
`src/lib.rs` (ToolingProvider, SymbolContext, ResolvedSource,
crate_from_symbol) and `src/cargo.rs` (the CargoProvider — the shape to
mirror). Operator directive sanctions fetch-on-demand via the language's
tooling. Toolchain available: node v24 + npm (no yarn/pnpm assumptions).

## What to build (js_provider.rs, name "javascript", languages
["javascript", "typescript", "tsx", "jsx"])

Resolution for a SymbolContext on an npm project:
1. Package from the symbol: `react.useState` → `react`; scoped packages
   `@testing-library/react` → handle `@scope/name` (split on `::` after
   normalizing `/`… define your normalization, document it).
2. Locate `node_modules/<pkg>` by walking up from workspace_root (and
   honoring workspaces/yarn-pnp? NO — plain node_modules only, documented).
   If absent: `npm install` (fetch-on-demand, sanctioned) with a
   `--no-audit --no-fund` fast path, then re-locate. Guard: never install
   outside the workspace (no global installs).
3. Entry point: package.json `main`/`module`/`exports` (exports map is
   modern truth — handle the string form and `.` key); TS: prefer `.ts`
   sources under `src/` if present, else `.d.ts` + `lib/*.js`.
4. File locate for the item: scan the package dir (skip node_modules
   inside it) for a definition-shaped match: `export function X`,
   `export const X`, `export class X`, `function X`, `class X`,
   `const X =` (arrow), TS: `export interface X`, `export type X`. Reject
   import/reference false positives (same discipline as CargoProvider's
   regex). Best-effort line number.
5. ResolvedSource: file, source_root (the package dir), external=true
   unless the package is a workspace-local path dependency (file:/link:
   in package.json — then external=false, source_root = the linked dir).
6. Failure modes: package not in any registry context (no package.json at
   all) → Err explaining; install refused (offline) → Err; item not found
   → Err naming the package + dir searched.

## Tests (inside your file)

- tempdir workspace with package.json + node_modules hand-built (no
  network): resolve a local dep (external=false via file: link).
- entry-point resolution: `exports` map with `.` object, `main` fallback,
  TS src preference.
- definition regex: matches real exports, rejects `import {X}` /
  `X.prototype` / comments.
- one live E2E with a real tiny npm package if the sandbox allows network
  (e.g. `left-pad` — tiny); if the network is blocked, mark #[ignore] and
  say so in the report.

## Verification

- `cargo test -p redline-resolve` — all green (your tests + the
  pre-existing 13; you may NOT edit other files' tests).
- `cargo clippy -p redline-resolve --all-targets -- -D warnings` clean.
- Report: design, per-step shell-outs, gate counts, deviations, gaps.

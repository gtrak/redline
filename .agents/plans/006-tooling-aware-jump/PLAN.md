# 006 — Redline: tooling-aware jump-to-definition

Status: planned
Phases: 1 · Issues: 01–02
Depends on: plan 005 (edit modes) — sequencing only; no code dependency

## Why

User directive 2026-09-17: "I want jump-to-selection to be language and
tooling aware, eg resolve the real cargo source folder, pull it if
necessary by shelling out to those tools."

M-. today resolves definitions inside the workspace (tree-sitter Xref).
Definitions in dependencies (tokio::spawn, serde::Deserialize) go nowhere.
The user wants the jump to fall through to the language's own tooling:
resolve where a crate's real source lives and fetch it on demand.

## What

1. **Issue 01 — resolver chain architecture + Rust/cargo provider**: M-.
   resolution order becomes: workspace tree-sitter Xref → language tooling
   provider (Rust first). The Rust provider shells out to cargo: `cargo
   metadata` to map crate name+version → registry source dir
   (~/.cargo/registry/src/index.crates.io-*/crate-version/); if the source
   for the resolved version is absent, run `cargo fetch` to pull it, then
   jump. Status line shows the activity (resolving/fetching). Provider
   trait so other languages slot in later.
2. **Issue 02 — jump polish for external sources**: external source trees
   open read-only (never edit-mode by default), indexed on demand for
   further jumps (e.g. tokio → tracing), and recents/history handle
   out-of-workspace paths (project registry untouched — external jumps
   are per-session).

## Key decisions

- Shell-outs are the user-sanctioned mechanism ("pull it if necessary"):
  cargo fetch may hit the network. Activity shown in the status line;
  failures degrade gracefully (jump reports "no provider resolution" —
  never blocks the workspace path).
- `cargo metadata --format-version 1` (JSON) is the resolution source of
  truth for package→manifest-path; registry source dir derived from the
  registry root + crate-version. No dependency additions: cargo is
  invoked as a CLI (std::process::Command).
- Providers are opt-in per language detection; workspace resolution always
  wins when it has the symbol.

## Success criteria

- M-. on a workspace symbol unchanged. M-. on `tokio::spawn` (tokio in
  Cargo.toml, source fetched) lands in ~/.cargo/registry/src/.../spawn.rs
  (or the workspace's vendored path); with source absent, the fetch runs
  then the jump lands. Read-only opening + on-demand index confirmed.

## Task order

| Phase | Issues | Depends on |
|---|---|---|
| 1 | 01 resolver chain + cargo provider | plan 005 complete |
| 1 | 02 external-source polish | 01 |

## Issue index

- [01 — resolver chain + cargo provider](01-resolver-chain.md)
- [02 — external source polish](02-external-sources.md)

When complete, archive per plan-process.

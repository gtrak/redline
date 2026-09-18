# 006 — Redline: tooling-aware jump-to-definition

Status: 01+02 complete; 03 folded into 02 (see task record)

## Outcome record

- **01 PASS** (reviewer-gated), **02 PASS** (`d9e181b`, reviewer verdict 2026-09-18:
  all 8 claims verified; 498 tests / 0 failed / 2 ignored; drive_xref 7/7).
  02 absorbed 03's polish (read-only landing, no recents/tree-follow, absolute
  paths out of the watcher). Issue 03 has no remaining independent scope.
- Review follow-ups queued as **006-02b** (non-blocking, small): P2 registry
  edit-mode write via C-x C-q override (guard toggle_read_only/save_buffer_key
  on path ownership), P2 watch latest-wins drain race (clear resolving on any
  event for gen <= current), P2 workspace hit should supersede an in-flight
  resolve (bump generation in the hit path), P3 second-colon boundary in
  symbol_at_point, P3 vacuous m_dot_includes_uppercase_identifiers test, P3
  line==0 guard in open_resolved_source.
- **Issue 03 REVIVED with new scope** (user request 2026-09-18: "I want to
  follow other types once inside a library buffer"): navigate WITHIN external
  sources — a background tree-sitter index of the landed crate's source_root
  (carried by `ResolvedSource.source_root` since 006-01), M-. and imenu keyed
  against it for external buffers instead of refusing "buffer not in project",
  resolver fall-through for symbols not in the crate. See
  `.agents/tasks/issue-006-03-impl.md`.
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

## Architecture (revised 2026-09-17)

The repo is now a **Cargo workspace** (root package `redline` + `crates/*`).
The resolver lives in its own crate **`redline-resolve`** — app-free (no
iocraft/store): plain inputs (project root, symbol context) → plain
locations. This is what makes provider work parallel-safe with UX lanes
(disjoint file trees). The app consumes it as a dependency; deeper core/ui
splits are a later plan if ever needed.

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
- [02 — app wiring: M-. fall-through](02-app-wiring.md)
- [03 — external source polish](03-external-sources.md)

When complete, archive per plan-process.

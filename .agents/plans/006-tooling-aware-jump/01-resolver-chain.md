# 01 — resolver chain + cargo provider

Phase 1 · Depends on: plan 005 complete

## Objective

M-. falls through to language tooling when the workspace Xref has no
definition: Rust first — resolve crate → real source folder via cargo,
fetch if missing, jump.

## Key decisions

- Resolver trait chain: `XrefProvider` (workspace, existing) then
  `ToolingProvider` per language. Rust provider: parse the symbol's crate
  from the use-path/type context (tree-sitter scope), `cargo metadata`
  for the package → manifest/source mapping, derive the registry source
  dir, `cargo fetch` when absent.
- Status-line activity (resolving/fetching crate-x vN) with C-g cancel of
  a running fetch where feasible (fetch is a child process — kill it).
- Failures never break the workspace path: they surface as a minibuffer
  message.

## Files

| File | Change |
|---|---|
| `src/app/xref.rs` (or navigation module) | provider trait + chain. |
| `src/app/providers/rust.rs` (new) | cargo metadata/fetch/source-dir resolution. |
| `src/app/store.rs` | M-. fall-through wiring + status activity. |
| tests + tools/ flows | fixture crate with a dependency; workspace hit unchanged; external jump after fetch. |

## Verification

- Gates green; PTY: M-. on tokio symbol → fetch runs (status line), jump
  lands in registry source; offline-safe failure message; M-. on local
  symbol unchanged.

# 02 — app wiring: M-. fall-through

Phase 1 · Depends on: 01 + plan 005 (src/ work — serialized with UX lanes)

## Objective

The app consumes redline-resolve: M-. misses fall through to the provider
chain; external sources open read-only.

## Key decisions

- store's M-. handler: workspace Xref hit → jump as today; miss → build
  SymbolContext → redline-resolve chain → open ResolvedSource (read-only
  when external). Status-line activity while resolving/fetching.
- Spawn the resolution off the input path (non-blocking UI), result lands
  like a jump-stack entry.

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | fall-through wiring + status activity (serialized with UX lanes). |
| `Cargo.toml` (root) | dependency on redline-resolve (path). |
| tests + tools/ flows | external jump E2E through the app. |

## Verification

- Gates green; PTY: M-. on tokio symbol lands in registry source
  read-only; workspace jumps unchanged.

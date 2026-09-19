# 01 — language dispatch + register the non-Rust providers

Phase 1 · Depends on: 007-03 (SymbolContext scope field)

## Objective

Make `M-.` reach the language's own provider. Today only cargo is registered
and the chain has no language filter.

## Key decisions

- The chain consults the trait's existing `languages()`; a miss probes only
  matching providers (never four toolchains per miss).
- The buffer's language comes from the app's grammar registry
  (`LanguageId::name()`), passed into the resolution context.
- An unset language preserves today's in-order behavior exactly.
- Path-shaped symbols first: this issue needs no syntax changes.

## Files

| File | Change |
|---|---|
| `crates/redline-resolve/src/lib.rs` | language on the context (or an explicit per-language entry) + skip in `resolve_traced` |
| `src/app/store.rs` | pass the language; register js/python/go providers |
| tests | dispatch assertions (only matching providers attempted) |

## Verification

- Unit: python context ⇒ only python attempted; rust ⇒ only cargo; unset ⇒
  legacy order. Rust path byte-identical.
- Resolver-crate tests stay green (the providers are untouched).
- A PTY leg per language if the toolchain is available (else say so).

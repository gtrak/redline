# 01 — Concern inventory + module map

Phase 1 · Depends on: —

## Objective

Plan the `store.rs` split on measured facts: group the 869 methods into
coherent concerns with line ranges, name the target modules, list the
mechanical constraints, and publish it as `docs/architecture.md` so every
later stage (and every reviewer) works from the same map.

## Steps

1. Inventory `src/app/store.rs`: the struct's fields (state) by concern;
   every `fn` with its line and a one-phrase tag; the test module's
   sections. Use grep/read (no AST tooling needed) and produce a table.
2. Propose the module layout (`src/app/store/mod.rs` + submodules) with,
   per module: the methods it takes, the fields it needs (and therefore
   which fields must become `pub(crate)`), its line count estimate.
3. Note the mechanical constraints: multiple `impl AppStore` blocks are
   legal in the same crate; field visibility is the only real friction;
   `#[path]`-style module wiring used by `flow_tests.rs` today; the
   store's `#[cfg(test)]` module move.
4. Write `docs/architecture.md`: the crate/module map (all of `src/` +
   `crates/`), the store split plan, the constraints, and a one-line
   "how to find things" guide.
5. Do NOT move code in this issue. It is analysis + docs only.

## Verification

- The inventory's per-module method counts sum to the real method count
  (869) within a stated tolerance for free functions/constants.
- A reviewer spot-checks 10 tagged methods against the file.
- No source file changes; `cargo test --workspace` unaffected (still run it).

## Files

`docs/architecture.md` (new). Read-only everywhere else.

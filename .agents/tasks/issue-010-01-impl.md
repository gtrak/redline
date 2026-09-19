# Task: plan 010 issue 01 — Shape A Rung 1 (impl association + struct field tables)

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `.agents/plans/010-bundled-static-analysis/PLAN.md` (Shape A, Rung 1)
and `.agents/skills/*.md` (tree-sitter skill is ground truth for the query
work). Your cwd is the MAIN TREE — commit to main per the established
style (no invented identity beyond the established one).

## Origin (user's primary navigation gap, confirmed)

`self.x` / `self.x()` inside `impl Foo` cannot be followed: M-. extraction
extends only across `::` (011-06 made dotted paths work per language, but
`self.bar` in RUST extraction stays the bare `bar`), and no machinery
connects `self` to the enclosing impl's type. Rung 1 builds the two tables
that close this for the self-receiver case — no expression typing, just
"which impl am I lexically inside".

## What to build (Rust only, in-project AND external crates)

1. **New Rust query captures** (src/syntax/queries.rs — the Rust query
   ONLY; no other language touched):
   - impl association: the `impl_item`'s TYPE (the `type_identifier` /
     generic-type node after `impl`) and optional TRAIT (for
     `impl Trait for Type`), plus the impl's method function_items.
   - struct fields: `field_declaration` (name + line) inside
     `struct_item`.
   Verify every node-kind/field against the pinned
   tree-sitter-rust NODE_TYPES via a throwaway probe (NOT memory — the
   011-03 lesson).
2. **Per-file tables** built in the same pass as the symbol index (zero
   extra parse cost — same trees): `fields: {struct_name → [(field,
   line)]}`, `impls: {type_name → [(method_name, line, impl_kind:
   inherent|trait_name)]}`. Decide the data home honestly: extend the
   existing `extract_symbols` result shape vs a sidecar — read
   `src/nav/index.rs` / wherever `extract_symbols` lives and the crate
   index's build; prefer the minimal structure that the M-. consumption
   can query synchronously (the app has the CURRENT file's parse in
   memory — a same-file table may not need the index at all for the
   self-receiver case: the enclosing impl is lexically local. Judge and
   state).
3. **M-. consumption** (`src/app/store.rs`): at `self.bar` / `self.bar()`
   inside an impl Foo, resolve via the enclosing impl's type: field → the
   struct's field line (same file first, then the crate index files);
   method → the impl's method line. Degrades to today's behavior
   (bare `bar`, index fall-through) whenever the enclosing impl/type/
   field can't be found — NEVER a guess. The `self.` path token: M-.
   extraction must produce `self.bar`-shaped input (decide: extend
   extraction or handle at the resolver entry — state the choice).
4. **find-implementations (Rung 4's table, read-only first)**: the impl
   table doubles as "who impls Trait" — a picker away. Include it ONLY
   if cheap (a `M-x`-style command reusing the existing picker); otherwise
   defer and say so.
5. **Degrade everywhere**: generic impls (`impl<T> Foo<T>`), macro-built
   impls, deref chains, non-Rust languages → today's behavior byte-for-
   byte. The plan's non-goals stand.

## Constraints

- Gate: `cargo test --workspace` + clippy + `tools/gate.sh full` (flock,
  `cargo build` first). Budget ~55 tool calls; honest-stop provision at
  half.
- Scope fence: `src/syntax/queries.rs` (Rust query only), the index/
  symbol-extraction module the tables land in, `src/app/store.rs`
  (M-. consumption + tests), `docs/provider-matrix.md` (a Rust section
  note: self-receiver resolution). NO provider-crate changes (the
  resolver stays package-level; this is an app/nav feature). Parallel
  lanes: none — the tree is yours.

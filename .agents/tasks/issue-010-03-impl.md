# Task: plan 010 issue 03 — Shape A Rung 3 (local binding types)

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `.agents/plans/archive/010-bundled-static-analysis.md` (Shape A) and
`.agents/tasks/issue-010-01-impl.md` (Rung 1 — landed, the machinery you
extend) + `.agents/skills/*.md` (tree-sitter skill is ground truth).

## Origin

Rung 1 (landed `53f0965`) resolves `self.bar` inside impls. Rung 3
extends the same honest ladder to LOCAL BINDING TYPES: a receiver
`x.field` / `x.method()` resolves when the binding's type is WRITTEN
DOWN — `let x: Type = ...` in the enclosing scope — or the value is a
struct literal `let x = Type { ... }`. Silent miss when the type isn't
written down (never inferred — that's the compiler's job and the plan's
non-goal).

## What to build (Rust only)

1. **New captures** (Rust query in `src/syntax/queries.rs` — verified
   against pinned NODE_TYPES via a throwaway probe, NOT memory):
   - `let` annotations: `let x: Type` — the binding name + its annotated
     type, keyed by the ENCLOSING function/method scope (the Rung 1
     machinery already identifies the enclosing impl; the binding table
     needs the function grain — decide the scope key honestly: function
     byte-range or the enclosing-block chain, state the choice).
   - struct literals: `Type { ... }` on the RHS of a `let` (binding's
     type without an annotation). Include ONLY if the capture is clean;
     otherwise annotation-only and say so.
2. **Tables**: extend `RustTables` (or a sibling structure — the Rung 1
   data home) with per-file binding maps: `{function_scope → [(binding,
   type_name)]}`. Built in the same single parse (`extract_all`), zero
   extra parse cost. Same maintenance discipline as Rung 1 — the
   same-content-refresh shortcut must NOT strip bindings (the 010-01
   review P1 pattern: same-check BEFORE removal — mirror it).
3. **M-. consumption** (store.rs): at `x.field` / `x.method()` where `x`
   has a written type in the enclosing scope: resolve via Rung 1's field
   table (and method table) keyed by the binding's type. Degradation
   byte-for-byte: unannotated bindings, generic types, shadowed bindings
   (innermost binding wins — pin it), non-Rust → today's behavior.
   `obj.field` on non-annotated receivers stays bare (today's behavior,
   pinned).
4. **Degradation matrix** (each pinned): no annotation, shadowed name,
   annotated to a NON-struct type (type alias? decide: follow
   `type` aliases only if trivially cheap, else miss), generic types,
   `mut` bindings (`let mut x: T` — same as x).

## Constraints

- Gate: `cargo test --workspace` + clippy + `tools/gate.sh full` (flock,
  `cargo build` first, read the clippy exit explicitly). Budget ~50 tool
  calls; honest-stop at half (commit partial + report).
- Scope fence: `src/syntax/queries.rs` (Rust query + tables), `src/nav/
  index.rs` (table extension), `src/app/store.rs` (M-. consumption +
  tests), `docs/provider-matrix.md` (Rung 3 note). NO resolver-crate
  changes, no other language queries.

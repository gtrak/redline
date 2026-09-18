# 010 — Redline: bundled static analysis (SPITBALL — no decisions made)

Status: exploratory sketch. NO LSP (user ruled out 2026-09-18). No worker
dispatched. Two candidate shapes below; the user is spitballing and may
pick, blend, or drop this entirely.
Origin: user asked "are we getting type info from the compiler somehow?"
(answer: no — everything today is syntactic + package layout). User then
ruled out LSP but said bundling some static analysis "might make sense."

## The navigation gaps that motivate this (all real, user-confirmed)

- `.`-field access (`self.x`) → the struct field. Today: name-guess only.
- Method call on a receiver → the method, through trait impls.
- Bare symbols (`Deserialize`) → need scope/use-path walking (plan 007-03).
- goto-type-definition, find-implementations: absent.

## Shape A — homegrown scoped analysis on the existing tree-sitter infra

A ladder of small, honest, always-degrading passes built on the SAME
index machinery we already run (no new dependencies, no external tools,
works inside library buffers via 006-03's crate index):

1. **Rung 1 — `impl` association + struct field tables.** New captures:
   `impl_item` (its Type + trait if any), `field_declaration`,
   `field_identifier`. Build per-struct `{field → line}` tables and
   per-type `{method → (line, impl-kind: inherent | trait)}` maps. Then:
   inside `impl Foo`, `self.bar` / `self.bar()` resolve via the
   enclosing impl's type — no expression typing needed, just "which impl
   am I lexically inside." This alone handles the user's primary `.` case
   for self-receivers, which is most real navigation.
2. **Rung 2 — use-path + module-tree resolution.** `use crate::foo::Bar`
   resolved against the mod tree (mod_item captures → file mapping we
   already maintain for the walk); `crate::`/`super::`/`self::` path
   segments resolved. Bare symbols get their true path → accurate
   resolver input (this largely IS plan 007-03, unified here).
3. **Rung 3 — local binding types.** `let x: Type` annotations and
   `Type { ... }` literals recorded per function scope; receiver `x.field`
   resolves when the binding's type is written down. Silent miss when not.
4. **Trait impl association** from rung 1 doubles as
   find-implementations (who impls `Trait`?) — a picker away.

What it does NOT do: generics, deref chains, method resolution on
arbitrary expressions, macro-generated code, cross-crate type flow. Those
are compiler-shaped problems.

Cost profile: all queries live in `src/syntax/queries.rs` next to the
existing 13; tables build in the same rayon pass as the symbol index
(zero extra parse cost — same trees); memory is per-crate name maps.
Fully offline, works in registry sources, degrades to today's behavior
per rung.

## Shape B — in-process rust-analyzer as a library (the fidelity option)

`rust-analyzer` is librarified: `AnalysisHost`/`Analysis` expose
goto_definition, type info, references — no LSP, no external binary, a
bundled dependency. Full semantic fidelity: generics, trait resolution,
deref, macro expansion.

Honest costs: heavy dependency tree (salsa et al.), slow compiles, real
memory footprint inside a TUI process, an API that churns between
releases, and a second VFS/parse layer alongside ours (we'd feed it our
files or let it own its copy). Degradation is by feature-flag rather
than by absence.

## How the shapes relate

Shape A rungs 1-2 are cheap, align with redline's read-focused identity,
and cover the user's demonstrated desires (field access on self,
trait-method/impl jumps, accurate bare-symbol resolution) — and they work
INSIDE library buffers with zero extra machinery once 006-03 lands the
crate index. Shape B is the "full fidelity" escape hatch that also buys
generics and arbitrary-expression typing, at a real cost in size and
complexity. A plausible path: do Shape A now; revisit Shape B only if
rung-3-style guesses feel dishonest in practice.

## Open questions for the user

- Is Shape A's "self-receiver + written-down types" honesty acceptable,
  or is full compiler fidelity (Shape B) the actual bar?
- If B: acceptable to carry the dependency weight, or is the TUI's
  lightness a hard constraint?

## Non-goals (either shape)

- No LSP, no external language-server process.
- No rename/refactor (external sources are read-only; project rename is a
  separate future plan).
- Non-Rust languages keep tree-sitter + resolver paths.

## Task order (if/when decided — not scheduled)

| Phase | Issue | Depends on |
|---|---|---|
| 1 | 01 impl/field tables (rung 1) | 006-03 crate index |
| 1 | 02 use-path/mod-tree resolution (rung 2, absorbs 007-03) | 01 |
| 2 | 03 local binding types (rung 3) | 02 |
| ? | 04 in-process RA (only if user picks Shape B) | — |

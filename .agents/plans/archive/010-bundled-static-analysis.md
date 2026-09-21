# 010 — Redline: bundled static analysis (ARCHIVED)

Status: ARCHIVED 2026-09-21 — SHAPE A COMPLETE: Rung 1/3/4 LANDED (verified
at landing, evidence rows in the 010 table in `.agents/plans/STATUS.md`),
Rung 2 SUPERSEDED (subsumed by plan 011), new languages + whole-path arms
LANDED.

**Shape B (in-process rust-analyzer) remains an OPEN contingent — issue
010-04 ("only if user picks Shape B") is still tracked as an OPEN
(contingent) row in `.agents/plans/STATUS.md` and in the short-answer
table; it did not vanish with this archival. The Shape B design section
below is the standing decision record for that call.**

This file is the durable record: the full original folder, verbatim. The
folder contained only `PLAN.md` (the rung/shape designs and the live
outcome record); the task specs remain in `.agents/tasks/`
(`issue-010-01-impl`, `issue-010-03-impl`, `issue-rung4-and-paths`,
`issue-new-languages`, `issue-newlang-paths`). No text was edited or
summarized; status lines inside the plan are as written when true.

---

## PLAN.md (full text)

# 010 — Redline: bundled static analysis (SPITBALL — no decisions made)

Status: **SHAPE A COMPLETE** — Rung 1/3/4 landed, Rung 2 subsumed by 011,
new languages + whole-path arms landed (see the outcome record below).
NO LSP (user ruled out 2026-09-18).
Two candidate shapes below; Shape A selected for the rung ladder's honest
degradation (each rung degrades to None, no compiler dependency).
Rung 2 (use-path + module-tree resolution) largely subsumed by plan 011's
landed work (scope hints + per-language walks); Rung 3 (local binding
types) queued behind Rung 1.
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
## Outcome record (live)
- **New languages** PASS (`72f2f06` + merge fixes, merged `3ddb824`):
  Java, C#, Ruby, Scheme landed (Clojure evidenced-N/A — no published
  grammar usable against the pinned 0.24.7 runtime; full record in
  coverage Gaps item 8). Registry at 18 languages; ABI guard
  (all_grammars_set_language_succeeds over ALL) + single runtime in
  the lock; C# highlights vendored verbatim from the crate (checksum
  + byte-comparison verified). Walk sets in registry lockstep (round-
  trip test extended) + per-language index e2e tests (011-04/07
  pattern). Follow-up queued: dotted_path_container arms for the new
  languages (whole-path M-.) + the coverage/matrix doc sweep.

- **Rung 4** PASS (`e3270e9` + review restore, merged `0126adc`):
  find-implementations (M-x `find-implementations`; trait-keyed map in
  the same index pass; Impls picker reusing the Xref jump seam, read-only
  external landings; degradation byte-for-byte; palette-only keybind —
  judged defensible vs the dense M- family). Same-check-before-removal
  pinned by a CONTRIBUTION-CARRYING file (the 010-03 false-mirror
  lesson avoided). Plus the whole-path upgrade for C/Cpp `field_expression`
  + Toml `dotted_key` (the Known corner closed; JSON correctly no —
  no dotted-key node; Cpp `::` stays on the byte-scan). Review
  BLOCKING→fixed: an out-of-fence 19-line docs deletion restored.
  SHAPE A COMPLETE (Rung 2 subsumed by 011; Rung 3 landed).

- **Rung 3** PASS (4 commits + review fixes `171deca`, merged): local
  binding types — `let x: Type` (and struct-literal RHS) keyed by the
  innermost enclosing BLOCK byte-range (exact Rust scoping; last-let-
  before-use wins, innermost shadow wins), M-. on `x.field`/`x.method()`
  resolves through the written type via the field/method tables. Review
  BLOCKING→fixed: the dot-chained receiver guard (`a.b.c` was
  misattributed to the middle segment — a confident wrong-struct jump;
  now rejects `.` gluing, pinned 3 ways incl. the review's exact repro
  proven discriminating). Pattern bindings (`if let`/`while let`/match
  arms) contribute nothing — documented, table-level asserted.


- **Rung 1** PASS (`2111d03` + review fixes `53f0965`, merged): RustTables
  (impl association + struct field tables) in the same parse pass; M-.
  self-receiver resolution (fields cross-file via the index, methods via
  the enclosing impl's type; ambiguous members → picker; generic impls,
  macros, non-Rust degrade byte-for-byte). Review BLOCKING→fixed: the
  same-content shortcut stripped the field map on no-op watcher refreshes
  (P1, regression-pinned); find-implementations deferred with the seam
  stored (honest call — a command surface, not picker reuse). Rung 3
  (local binding types) next; Rung 2 subsumed by 011.

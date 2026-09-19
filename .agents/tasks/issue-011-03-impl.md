# Task: plan 011 issue 03 — node-at-point per language

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/tree-sitter/SKILL.md` is authoritative ground truth for the
API and ABI pinning rules (no registry reads, no docs.rs, no fetch).

## Origin

007-01 built `node_at` / `scope_path_at` in `src/syntax/node.rs` — Rust-first
by design: every other `LanguageId` returns `None`/`[]`. That was correct for
shipping Rust scope resolution (007-03), but it means the syntax layer can't
help JS/TS/Python/Go at all. This issue adds the per-language predicates and
enclosing-scope walks behind the SAME extension point, each independently
degradable.

## Prerequisite

011-01 + 011-02 in HEAD (providers registered + per-language hints), and
007-01's Rust implementation present. Check `git log --oneline -5`; STOP and
report if absent.

## What to build

1. **Per-language identifier predicates** — the analogue of Rust's
   `is_rust_identifier_kind` (`identifier|field_identifier|type_identifier|
   scoped_identifier|scoped_type_identifier|primitive_type`). Determine each
   language's actual node kinds from the pinned grammar (the skill documents
   how to verify against the grammar; do NOT guess from memory):
   - **JS/TS**: `identifier`, `property_identifier`, `type_identifier`,
     `member_expression` (the whole `a.b.c` path), `nested_type_identifier`.
   - **Python**: `identifier`, `attribute` (the whole `a.b.c`).
   - **Go**: `identifier`, `selector_expression` (the whole `pkg.Fn`),
     `field_identifier`, `type_identifier`, `qualified_type`.
2. **Per-language scope walks** — the analogue of `rust_scope_path`: the
   enclosing item chain, outermost-first. Per language decide the meaningful
   item kinds (JS: function/method/class/arrow fn assigned to a name; Python:
   `function_definition`/`class_definition`; Go: `function_declaration`/
   `method_declaration`/`type_declaration`) and the field that carries the
   name.
3. **Preserve the whole-path rule**: `a.b.c` must come back as ONE node (whole
   text), not a fragment — mirror the Rust `is_path_segment` logic per
   language's grammar shape (the JS/Python/Go analogues of `scoped_identifier`).
4. **Rust behavior unchanged** — its predicate, scope walk and tests must be
   byte-identical in behavior. Do not refactor Rust into a generic abstraction
   unless that keeps every existing test green without modification.
5. **Keep the module plain Rust** (no iocraft/tokio), the API surface
   unchanged, and `node_at` returning `None` for `LanguageId::Plain` and any
   still-unimplemented language.

## Explicit non-goals

- No app wiring (011-02 already consumes hints; if a provider benefits from
  real scope paths, that is a follow-up, not this issue).
- No index changes (011-04). No LSP. No new dependencies.
- No changes to `queries.rs`'s existing highlight/symbol queries.

## Constraints

- Skills are truth. Write-first; compile early. Record skill corrections.
- Gate: `tools/gate.sh fast` inner loop, `tools/gate.sh full` final (the gate
  is `--workspace`; do not regress the resolver crate's coverage). Progress
  streams to stderr; do NOT pipe stdout through `tail`.
- PTY flock rules apply if you run any PTY suite; wrap in `timeout`.
- BUDGET: land within ~60 tool calls; no new investigations after it compiles.
- Scope fence: `src/syntax/node.rs` (+ its tests). Nothing else.
- Plain `git commit`; do not `git add -A` other sessions' files.

## Verification (iterate until ALL pass)

- `tools/gate.sh full` green, honest counts; the PTY suites must be UNCHANGED
  (this issue has no user-visible behavior by itself).
- Per-language unit tests mirroring 007-01's Rust set, for each of JS/TS,
  Python, Go:
  - a dotted/member path comes back WHOLE (`a.b.c`, `pkg.Fn`) for an offset on
    any segment — the discriminating case;
  - a plain identifier at top level;
  - scope chain inside a function inside a class/type;
  - boundary offsets (0, end, past-EOF) do not panic;
  - a syntactically broken file does not panic;
  - an unimplemented language still returns None/[].
- If a language's grammar makes a case impossible, say so rather than writing
  a test that passes vacuously.

## Report format

Per-language predicate tables (the actual grammar node kinds, and how you
verified them); the scope-walk design per language; the whole-path handling;
proof Rust behavior is unchanged; test list with discriminating tests called
out; gate counts; skill corrections; deviations.

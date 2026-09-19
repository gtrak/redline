# Task: plan 011 issue 02 — per-language scope hints (bare symbols)

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## Origin

007-03 made BARE symbols resolve — but only for Rust. `resolver_scope_for`
(`src/app/store.rs`) returns empty for any non-Rust buffer, so the js/python/go
providers (now registered by 011-01) always get a no-hint context and bail with
"needs scope info". This issue extends the import walk so a bare imported
symbol resolves in JS/TS, Python, and Go too.

## Prerequisite

011-01 must be in HEAD (language dispatch + providers registered). Check
`git log --oneline -5` first. If absent, STOP and report.

## What to build

1. **Per-language import walk** in `src/app/store.rs`, mirroring 007-03's Rust
   `use_path_for_symbol` but for each language's import syntax:
   - **JS/TS**: `import { X } from "pkg"`, `import X from "pkg"`,
     `import * as ns from "pkg"` (namespace member access), `require("pkg")`
     destructuring if cheap; `import type { X }`.
   - **Python**: `import pkg`, `import pkg.sub`, `from pkg import X`,
     `from pkg.sub import X`, `from pkg import X as Y` (alias → original),
     relative `from . import X` / `from ..pkg import X` (decide: resolve
     against the enclosing package if cheap, else bail honestly).
   - **Go**: `import "github.com/x/y"` + qualified use `y.Fn`; grouped
     `import ( … )`; aliased `import y "github.com/x/y"`.
   Keep each walk **bounded** (same discipline as Rust: scan only the relevant
   import block/ancestor scope, one parse per miss). A parse failure or an
   unsupported shape → empty hint (the existing honest bail), never a guess.
2. **Language-specific symbol shape.** Note the separator differs per
   language: Rust `::`, JS/TS `.`, Python `.`, Go `.`. `scope_qualified`
   already takes a separator — verify each provider passes its own and that
   a JS `lodash.map` (path-shaped) is NOT treated as bare.
3. **Extend each provider to consume the hint where it currently bails.**
   The js/python/go providers already call `scope_qualified` (007-03 shape);
   verify the *item selection* rule is right per language and that a
   dotted/hinted bare symbol flows through the EXISTING machinery
   (`node_modules`/`locate_item`, `find_spec`/`locate_module_file`,
   go.mod/module-cache). No parallel code path.
4. **Degradation is byte-for-byte.** No hint → the exact existing bail message
   per language. Stdlib/prelude/relative-import cases must NOT be guessed.

## Explicit non-goals

- No changes to `src/syntax/node.rs` (that is 011-03).
- No index changes (011-04).
- No LSP, no new dependencies, no provider rewrites beyond hint consumption.

## Constraints

- Skills are truth (no registry/docs.rs/fetch). Write-first; compile early.
- `crates/redline-resolve` stays app-free (plain data only).
- Gate: `tools/gate.sh fast` inner loop, `tools/gate.sh full` final. The gate
  is `--workspace` now — do not regress the resolver crate's coverage.
  Progress streams to stderr; do NOT pipe stdout through `tail`.
- PTY flock: "shared PTY fixture is busy" + exit 3 ⇒ wait and retry; NEVER two
  suites concurrently; wrap EVERY python PTY invocation in `timeout`.
- BUDGET: land within ~50 tool calls; no new investigations after it compiles.
- Plain `git commit`; do not `git add -A` other sessions' files.

## Verification (iterate until ALL pass)

- `tools/gate.sh full` green, honest counts, no legacy suite changed.
- Per-language unit tests: a bare imported symbol resolves via the hint in
  JS, Python, and Go (each against a synthetic/fixture package where the
  toolchain is available, else a unit-level hint test); alias → original;
  no-import → empty hint → the exact existing bail message; stdlib/prelude
  not guessed.
- A PTY leg per language is ideal but only if the toolchain is present —
  otherwise say so plainly (do not fake it). Prefer one Node or Python leg at
  minimum since those toolchains are commonly present.

## Report format

The per-language import-walk design (grammar shape handled, bounds); the
symbol-shape/separator handling; the item-selection rule per language; the
degradation proof (message strings unchanged); test list with discriminating
tests; gate counts; which PTY legs ran and on what toolchain; deviations.

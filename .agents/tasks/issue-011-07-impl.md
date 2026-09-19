# Task: plan 011 issue 07 — walk sets for C/C++/Markdown (and any other language with definition queries)

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## Origin

011-04 review finding (P2): `source_extensions_for` (`src/app/store.rs:8764`)
returns `&[]` for every language outside the four contract families, even
though `query_for` (`src/syntax/queries.rs:156`) HAS definition queries for
`C`, `Cpp`, and `Markdown`. Landing in a C dependency yields the pre-011-04
end state (no index, silent). This issue adds the missing walk sets so those
landings index too.

## Prerequisite

011-01..05 in HEAD. Check `git log --oneline -8`; STOP and report if absent.

## What to build

1. **Extension sets** for every language that has a definition query in
   `queries.rs`, derived honestly from `registry.rs`'s extension map (the
   authority — same round-trip-test discipline as 011-04: every extension in
   a set must resolve back to its language via `resolve_language`, and the
   set must extend the round-trip test). Likely: C (`.c`, `.h`), C++ (`.cc`,
   `.cpp`, `.hpp`, ... — take the map's actual keys), Markdown (`.md`).
   Judge honestly whether headers (`.h`) belong in the C set (headers ARE
   definition sources for C — include them) and whether Markdown headers are
   useful M-. targets (the query exists; verify what it captures before
   promising anything).
2. **The node_at/scope walks stay None for these languages** — that is fine
   and expected (011-03's degradation): the index + path-shaped M-. work;
   bare-symbol hints do not (empty scope → the honest bail). Do NOT add
   identifier predicates for C/C++/Markdown here.
3. **PTY leg**: one leg in a drive (own repo/flock, the established pattern)
   landing in a C dependency (or any of the new languages) and jumping via
   the crate index. If no toolchain is needed to build the fixture (a C
   project is just files + a directory layout the cargo-style provider...
   CAREFUL: there is NO c provider — landing in a C dependency resolves via
   the crate index only (in-library follow-up), NOT via M- from a project
   buffer. Verify which legs are actually possible and pin the TRUE
   behavior, including the honest bail for cross-buffer M-. (no provider
   handles language `c` — the 011-01 honest bail).
4. **Update `docs/provider-matrix.md`**: add the language rows (or extend the
   providerless-languages section) with the in-library-follow-up cell now
   working via the index, and M-. from a project buffer degrading to the
   honest bail. Verification status per cell.

## Explicit non-goals

- No identifier predicates / scope walks for these languages (011-03 territory).
- No new providers (c/cpp have no package manager machinery here — do not
  invent one). No changes to `registry.rs`, `queries.rs`, or `node.rs`.

## Constraints

- Skills are truth. Write-first; compile early.
- Gate: `tools/gate.sh fast` inner loop, `tools/gate.sh full` final
  (`--workspace`). Progress streams to stderr; do NOT pipe stdout through
  `tail`.
- PTY flock discipline; every python PTY invocation under `timeout`.
- Budget: land within ~40 tool calls; no new investigations after it compiles.
- Scope fence: `src/app/store.rs` (`source_extensions_for` + the round-trip
  test + a couple of index tests), `docs/provider-matrix.md`, the drive leg,
  `tools/gate.sh` (SHARED_SUITES line). Nothing else.
- Plain `git commit`; do not `git add -A` other sessions' files.

## Verification

- `tools/gate.sh full` green with HONEST counts.
- The round-trip test extended to the new languages (set ↔ registry map).
- The new-language index e2e test is discriminating (fails if the walk
  returns empty).

## Report

Files changed; the sets chosen and WHY per language (especially the header
and Markdown judgment calls); which cells are live-verified; tests added;
gate counts; deviations with reasons.

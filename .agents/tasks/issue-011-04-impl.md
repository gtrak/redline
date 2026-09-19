# Task: plan 011 issue 04 — per-language source index

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## Origin

006-03 built the background crate index so M-./imenu work INSIDE a landed
dependency buffer ("follow other types once inside a library buffer"). It
walks `**/*.rs` only, so in a landed JS/Python/Go dependency the index is
empty and M-. falls through. This issue generalizes the walk to the owning
language's file extensions.

## Prerequisite

011-01 in HEAD (language dispatch + providers registered). 006-03's crate
index present (`external_indexes`, `CrateIndexBus`, `crate_rs_files`,
`EXT_INDEX_CAP`, the `indexing crate …` indicator). Check `git log`; STOP and
report if absent.

## What to build

1. **Extension-aware walk**: replace the hardcoded `.rs` filter in the crate
   index build with the owning language's extension set, derived from the
   resolved source's language (not a global "index everything" walk). Suggested
   sets — verify against what redline actually supports
   (`src/syntax/registry.rs`'s extension map is the authority):
   - Rust: `rs`
   - JS/TS: `js`, `jsx`, `ts`, `tsx`, `mjs`, `cjs`
   - Python: `py`, `pyi`
   - Go: `go`
2. **Preserve 006-03's machinery unchanged**: the LRU cap, the current-crate
   recency bump (006-03b), the single-flight per root, the >N-file refusal
   cap, the `indexing crate … (i/N)` indicator, and the crate-relative key
   shape (with the `\`→`/` normalization). Only the file-selection predicate
   changes.
3. **Key-shape consistency**: the crate-relative path keys must match what the
   lookup sites derive (`crate_rel`, `current_buffer_outline`) — verify the
   normalization still applies after the extension change.
4. **Refusal cap sanity**: a node_modules tree can be enormous. Decide whether
   the existing cap is right for a JS dependency (a landed package's own
   source dir is usually small, but verify) and state the reasoning; do not
   silently index 50k files.

## Explicit non-goals

- No changes to `src/syntax/node.rs` (011-03) or the resolver crate.
- No annotation changes (keys are absolute/rel per 008-01 — unchanged).
- No LSP, no new dependencies.

## Constraints

- Skills are truth (no registry/docs.rs/fetch). Write-first; compile early.
- Gate: `tools/gate.sh fast` inner loop, `tools/gate.sh full` final (the gate
  is `--workspace`; do not regress the resolver crate's coverage). Progress
  streams to stderr; do NOT pipe stdout through `tail`.
- PTY flock: "shared PTY fixture is busy" + exit 3 ⇒ wait and retry; NEVER two
  suites concurrently; wrap EVERY python PTY invocation in `timeout`.
- BUDGET: land within ~50 tool calls; no new investigations after it compiles.
- Scope fence: `src/app/store.rs` (the walk + tests), a `tools/` PTY leg if
  useful. Nothing else.
- Plain `git commit`; do not `git add -A` other sessions' files.

## Verification (iterate until ALL pass)

- `tools/gate.sh full` green, honest counts; existing suites unchanged
  (drive_external_crate 12/12 etc. must stay green — Rust behavior identical).
- Unit tests: an index builds for a synthetic out-of-root JS tree and a Python
  tree (relative keys, correct file set); the Rust path is unchanged
  (regression); the refusal cap still fires; LRU/recency behavior unchanged.
- A PTY leg for at least one non-Rust language if the toolchain + a landed
  dependency are available (M-. lands in a dependency, then M-. again INSIDE
  it). If not practical, say so plainly.

## Report format

The extension-set derivation + where it lives; the cap reasoning; the key-shape
proof; test list with discriminating tests; gate counts; whether a non-Rust
PTY leg ran; deviations.

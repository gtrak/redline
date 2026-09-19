# Task: plan 011 issue 05 — resolver parity harness + provider matrix

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## Origin

Plan 011 (`resolver parity across languages`). Issues 01–04 make non-Rust
resolution work; this issue LOCKS IT IN with drives and answers the original
question ("do we have resolver parity across languages?") with a documented,
honest matrix — what resolves today, per language, and what deliberately does
not.

## Prerequisite

011-01..04 in HEAD (language dispatch + providers registered; per-language
scope hints; per-language node-at-point; per-language index). Check
`git log --oneline -8`; STOP and report what is missing.

## What to build

1. **A per-language PTY drive**, following the `tools/drive_external_crate.py`
   / `drive_external_use.py` pattern exactly:
   - own fixture repo (never the shared `/tmp/redline_pyte_repo`),
   - own per-repo PTY flock (the driver keys on repo abspath),
   - its own `setup_repo()` that builds the language project,
   - registered in `tools/gate.sh`'s `SHARED_SUITES` (so `full`/`pooled` run
     it) and verified under `timeout`.
   Cover at least: path-shaped symbol → external source landing, and (where
   02/03 landed) a bare imported symbol → landing. One drive per language is
   fine; a shared drive with per-language legs is also fine.
   **Toolchain honesty**: if `npm`/`node`, `python3`, or `go` is unavailable,
   the drive must SKIP WITH A LOUD, VISIBLE NOTE (never a silent pass, never a
   fake fixture that pretends the tool ran). State clearly in the report which
   languages were verified live and which were skipped.
2. **The provider matrix doc** — the honest answer to the parity question.
   A table in `docs/` (extend `docs/ux-testing-plan.md` or a new short doc):
   per language × capability (path-shaped / bare via import / in-library
   follow-up / blame), marking each cell **works**, **degrades to the bail**,
   or **not implemented**, with the reason. Include the deliberate
   non-goals (LSP out of scope; macros/generics inference out of scope) so the
   matrix is not mistaken for a bug list.
3. **A regression guard on the chain**: assert (unit level is fine) that a
   miss probes only the providers whose `languages()` match — i.e. adding a
   language never makes every miss probe every toolchain. If 011-01 already
   pins this, reference it rather than duplicating.

## Explicit non-goals

- No new resolution capability — this issue observes and documents.
- No LSP. No new dependencies.
- Do not modify provider internals or `src/syntax/node.rs`.

## Constraints

- Skills are truth. Write-first; compile early.
- Gate: `tools/gate.sh full` (and `pooled` if useful) green with HONEST
  counts; existing suites unchanged. Progress streams to stderr; do NOT pipe
  stdout through `tail`.
- PTY flock: "shared PTY fixture is busy" + exit 3 ⇒ wait and retry; NEVER two
  suites concurrently; wrap EVERY python PTY invocation in `timeout`. Note the
  pool (`tools/pool.py`) exists for parallel runs — do not hand-roll
  concurrency.
- BUDGET: land within ~50 tool calls; no new investigations after the drives
  pass.
- Scope fence: `tools/drive_*` (new), `tools/gate.sh`, `docs/`. No `src/`.
- Plain `git commit`; do not `git add -A` other sessions' files.

## Verification (iterate until ALL pass)

- `tools/gate.sh full` green; the new drive(s) green on the toolchains that
  ARE present; skipped languages reported loudly.
- The matrix doc's every claim is TRUE for this commit — spot-check at least
  one "works" cell and one "degrades" cell against the actual code path, and
  say which you checked.
- Confirm a miss does not probe mismatched providers (reference the 011-01 pin
  or add one).

## Report format

The drives added (repo, legs, toolchain required, verified-or-skipped); the
matrix (paste it) and which cells you verified against code; the
miss-probes-only-matching proof; gate counts; deviations.

# Task: coverage-tracker review fixes (docs)

You are the implementation worker. Repo root is your cwd. Self-contained.

## Origin

The coverage tracker (8fb0b27) review verdict: BLOCKING with 1 P1 + 6 P2s.
All are doc-accuracy fixes in `docs/language-coverage.md` (+ mirrored rows
in `docs/provider-matrix.md` where the review says the matrix is stale too).

## Items

1. **P1 (language-coverage.md:70, Rust in-library cell)**: claims "works
   (live) — drive_external_crate L4" — L4 is no longer a live leg (the
   loop-03/04 demotion made it `unit_flow_ext_crate_in_crate_mdot` in
   src/app/flow_tests.rs:2830; the drive runs L1 only). Relabel
   `works (unit)` with that citation. The same stale claim is mirrored in
   docs/provider-matrix.md (Rust "in-library follow-up" + the Drives
   table) — fix both.
2. **P2 self-contradicting method section** (language-coverage.md:33-34):
   "(None at time of writing…)" is false — three cells carry the
   `unverified by worker` marker (doc:71-72). State the three markers
   exist.
3. **P2 stale live-dispatch pin** (doc:76, C bare cell): "011-05 L-P2/L-J1a"
   now pin LANDINGS (011-06 superseded the miss pins) — the live
   miss/dispatch pin is 011-02 L2 (drive_issue_011_02.py:112-122).
4. **P2 line-citation drift** (doc:74 Python scope → node.rs:389 not 412;
   doc:72 Tsx bare → store.rs:8173-8176/8441 not 8169; doc:76 C bare →
   store.rs:8184 not 8187; doc:76 C scope → node.rs:350 not 351). Verify
   each before rewriting (lines may have shifted again since the
   review).
5. **P2 undefined cell value**: "not applicable" (doc:78-83) — define it
   in the Verification-method section (a 6th value: the capability
   cannot apply to the language) or reword the cells.
6. **P2 TS tier disagreement**: provider-matrix.md's combined JS/TS
   column claims "works (live)" for path-shaped/bare/in-library — the
   coverage doc (accurate) says TS is unit-only, no live TS leg. Fix the
   matrix's JS/TS cells to the TS-tier truth (per-language rows or a
   footnote).
7. Sweep the whole grid once for any OTHER stale live-leg citation (the
   loop-03/04 demotion touched drive_windowing/panes/xref/external_*:
   any cell citing those legs as live must be re-verified against the
   demoted tiers' unit twins).

## Constraints

- Docs only: the two docs files. NO code. Budget ~20 tool calls.
- Commit to main (established style). Report per-item.

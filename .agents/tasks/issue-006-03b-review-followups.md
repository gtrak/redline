# Task: plan 006 issue 03b — external-crate index review follow-ups

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## Origin

The 006-03 review (verdict PASS, commit `7bab785`) filed five non-blocking
P2/P3 findings against the new crate-index path. All are small and live in
`src/app/store.rs`. Fix them; do not redesign.

## What to fix

1. **P2 — cap-3 eviction can strand a still-open buffer's crate.** Merely
   opening a buffer in crate A does not bump A's recency (only a
   consultation does: M-./imenu/picker step). Land in three other crates
   afterwards and A is evicted while its buffer is still the current one;
   the next M-. in A still takes the external branch but now falls through
   to the resolver, which for a bare in-crate name reports
   `no provider resolution for …` — in-crate navigation appears to "stop
   working" in a buffer the user still has open. **Fix:** make the buffer's
   owning crate MRU on every relevant transition — at minimum
   `open_external_path` landing and whenever a buffer whose path is under a
   cached `source_root` becomes current. Keep it minimal; the goal is that
   the crate you are *in* is never the eviction victim. Add a test: land in
   crate A, make it current, then consult three other crates → A survives;
   the next M-. in A still jumps in-crate.
2. **P2 — resolver `from_file` is absolute where `SymbolContext.from_file`
   is documented workspace-relative.** `xref_in_external_buffer` passes
   `path.display().to_string()` (absolute) into the resolver fall-through,
   while the project path passes the project-relative `rel`. No live bug
   (`crates/redline-resolve`'s cargo provider never reads `from_file`), but
   it violates the documented contract and will mislead a future provider.
   **Fix:** pass the crate-relative `rel` (the index's own key shape) — or
   the origin project's workspace-relative path if that is more
   consistently documented; state your choice and why in the report. Do NOT
   change the resolver crate.
3. **P2 — separator normalization asymmetry.** `crate_rs_files` normalizes
   `\`→`/` when building index keys, but `crate_xref_outcome` and
   `current_buffer_outline` derive `rel` with native separators (no
   `replace`). Harmless on Linux; breaks same-file-first ordering and
   `outline(&rel)` lookup on Windows. **Fix:** apply the same normalization
   at both lookup sites (one small helper is fine). Add a unit test that a
   `\`-containing path resolves the same as its `/` form.
4. **P3 — `current_buffer_outline`'s `strip_prefix(&root).unwrap()`** is
   guarded by the constructor's `path.starts_with(r)` today, so it is safe,
   but a bare `unwrap()` on a path relation is a robustness nit
   (review-gate item 6). **Fix:** use `if let Some(rel) = …` and return the
   empty outline (or the existing fallback) on miss; never panic.
5. **P3 — the "N/M counter would breach the nav/index fence" justification
   overstated the constraint.** An `Arc<IndexProgress::new(n)>` used
   WITHOUT `with_publisher` can carry a total count alongside the
   `crate_indexing` label with zero `src/nav/index.rs` edits. This is
   optional: if it is genuinely small, make the crate indicator read
   `indexing crate <dir> (i/N)…`; if it turns out to need
   `nav/index.rs` changes, SKIP it and say so — the spec's drive leg only
   requires the indicator to appear then clear.

## Constraints

- Skills are truth (`.agents/skills/*.md`; no registry/docs.rs/fetch).
  Write-first; compile early. PTY flock: "shared PTY fixture is busy" +
  exit 3 ⇒ wait and retry; NEVER two suites concurrently; wrap EVERY python
  PTY invocation in `timeout`. Honest gate counts. Plain `git commit`
  (identity configured); no `git add -A` of other sessions' files.
- Scope fence: `src/app/store.rs` (fixes + tests) and, ONLY if item 5 needs
  it, a tiny accessor in `src/nav/index.rs`. No main.rs, no root.rs, no
  resolver crate, no queries, no annotation changes, no tools changes
  (the existing `drive_external_crate.py` 12/12 must still pass unchanged).
- Baseline is HEAD (`7bab785` + the loop-speedup tooling merge): `tools/gate.sh`
  now exists — `tools/gate.sh fast` runs build/clippy/test, `full` runs the
  whole battery at the safe read-quiet. Use `full` for your final gate.

## Verification (iterate until ALL pass)

- `tools/gate.sh full` — build / clippy -D warnings / cargo test (>=520
  passed) / sweep 14/14 / drive_all 8/8 / drive_windowing 28/28 /
  drive_windowing_panes 4/4 / check_cursor_stream 80/80 / ux_sweep 3
  pre-existing / probe_notes_dump 17/17 / drive_xref 10/10 /
  drive_external_notes 16/16 / drive_external_crate 12/12 /
  sweep_flows 65/65. Honest counts.
- Unit tests for items 1–4 (and 5 if done). The eviction test must be
  discriminating: it must FAIL if the recency bump is removed.

## Report format

Per-item fix + file:line; which option you chose for item 2 and why; whether
item 5 was done or skipped and why; gate counts; deviations.

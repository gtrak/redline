# Task: plan 006 issue 02b — review follow-ups for the M-. resolver wiring (small, bounded)

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## Context

006-02 (`d9e181b`) landed M-. cursor-aware selection + resolver fall-through;
the review PASSED all 8 claims but returned 6 non-blocking findings. This
issue fixes all six. They are small and independent; the total diff should
stay modest. If any item balloons beyond its stated size, STOP on that item,
implement the rest, and report it as a deviation.

Baseline facts you can rely on (reviewer-verified against `d9e181b`):
`symbol_at_point` (`store.rs:7221-7250`), `toggle_read_only`
(`store.rs:2518-2570`), `save_buffer_key` (`store.rs:1988-2000`),
`open_external_path` (`store.rs:3050-3062`), `open_resolved_source`
(`store.rs:3065-3097`), the resolve drain (`src/ui/root.rs:455-465`),
`apply_resolve_event` + generation gate (`store.rs:7187-7190`),
`resolving_display` (`store.rs:7207-7212`), `start_symbol_resolution`
(`store.rs:7153`). Line numbers may have drifted; grep the names.

## Fixes

1. **P2 — external buffers must be un-editable even via the C-x C-q
   override.** `toggle_read_only` refuses only `path.is_none()`; for an
   external path the else-branch sets `editable = true`, after which
   `save_buffer_key` (checks only `buf.editable`) writes the file — and
   editing `~/.cargo/registry/src/...` corrupts a cache shared by every
   project on the machine. Add an ownership notion (e.g. `fn
   buffer_is_project_owned(&self, key) -> bool`: path under
   `project.root`, or no path at all (scratch stays as today)) and guard
   BOTH `toggle_read_only` (refuse with a clear message for external
   paths) and `save_buffer_key` (refuse the write; message) on it. Keep
   plan-005 semantics intact for project files and scratch (the notes
   buffers are path-less or under-root — verify which, and keep them
   toggleable per the 005 decision). Tests: external buffer C-x C-q
   refused with message; project file unaffected; scratch unaffected.

2. **P2 — workspace hit should supersede an in-flight resolve.** Only
   `start_symbol_resolution`, `switch_project_root`, and `checkout_branch`
   bump `resolve_generation`; a successful M-. workspace hit leaves the
   generation, so a stale gen-1 event can open a registry source and
   record a jump up to 120s after the user's hit superseded it. Bump the
   generation at the top of `xref_find_definitions` (both hit and miss
   paths — the miss path re-bumps in `start_symbol_resolution`, which is
   fine; verify no double-bump breaks the stale-discard test).

3. **P2 — watch latest-wins can drop the current generation's event.**
   The root drain applies only the newest watch value; if gen-2's send is
   overwritten by gen-1's stale send arriving later within one drain
   window, `resolving` sticks until the next action. Minimal fix per the
   review: when ANY event for generation <= current arrives, clear the
   resolving indicator for that generation (do not attempt full
   cancellation or an mpsc rework — keep it small). If the minimal fix
   cannot be made correct in <30 lines, say so and leave the race
   documented instead.

4. **P3 — cursor on the second colon of `a::b` extracts nothing.**
   `symbol_at_point` returns None when the preceding char is `:` (not an
   identifier), falling to the enclosing fallback. Fix: when the char
   before the point is `:` and the char before THAT is an identifier
   char, treat the point as the end of the preceding path segment
   (extract `a` / `a::b` per the existing rules). Symmetric with the
   existing "run before the point" boundary handling.

5. **P3 — `m_dot_includes_uppercase_identifiers` (`store.rs:13022-13036`)
   is vacuous**: it re-implements the old line-splitting filter and
   asserts against its own copy. Rewrite it against `symbol_at_point`:
   cursor on an uppercase-initial identifier (type/const, e.g. `Foo`)
   yields that identifier (the original bug it guarded was M-. skipping
   uppercase names). Delete the dead inline re-implementation.

6. **P3 — `open_resolved_source` (`store.rs:3090`) computes
   `(l - 1) as usize` unguarded.** Guard `line == 0` (treat as "no line"
   → top), so a future provider emitting 0 cannot panic in debug builds.
   One line + a test if cheap.

## Constraints

- Scope fence: `src/app/store.rs`, `src/ui/root.rs` (item 3 only),
  tests, `tools/drive_xref.py` (a leg only if one of the fixes changes
  observable behavior — item 1 does: the refusal message). No resolver
  crate changes, no annotation changes, no main.rs.
- All suites green: `cargo test`, `tools/sweep.py`, `tools/sweep_flows.py`,
  `tools/drive_all.py`, `tools/drive_windowing.py`,
  `tools/drive_windowing_panes.py`, `tools/check_cursor_stream.py`,
  `tools/ux_sweep.py`, `tools/probe_notes_dump.py`, `tools/drive_xref.py`.
  PTY flock rules as always; wrap EVERY python PTY invocation in
  `timeout`; honest gate counts.
- Repo identity configured — plain `git commit`, no `-c` overrides.

## Report format

Per-item: what changed (file:line), test added/rewritten, size vs. budget.
Gate counts (honest). Deviations (any STOPped item + why).

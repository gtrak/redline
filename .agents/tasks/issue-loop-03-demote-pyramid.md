# Task: loop-03 — demote the test pyramid (below-PTY conversion)

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## Origin

Orchestrator audit (2026-09-20): the PTY battery is ~230 s of which
sweep_flows alone is ~170 s, and its assertions are ~85% store-state +
screen-text checks that duplicate existing unit-level facts (evidence:
the `q`-unbound flow duplicates `bare_q_in_home_view_is_unbound…`;
the 004-06a picker-canvas bug was invisible to 322 static tests BECAUSE
static renders are width-unbounded — iocraft's `render(e, max_width)`
accepts a width and `to_string()` calls `render(None)`).

## What to build

1. **Width-bounded static render** (`src/ui/root.rs` or the test helper
   module): a `render_at_width(80)` (or per-test parameter) helper that
   renders the root with `max_width: Some(80)` — the layout-correctness
   class (content-sized layers escaping the root, off-screen count lines)
   becomes catchable statically. Convert the LAYOUT-SENSITIVE flow
   assertions to this helper first (the picker/canvas class). Prove the
   helper catches the class: a regression-style test that FAILS if the
   count-line lands off-screen at width 80 (the pre-df95113 behavior)
   — you may reconstruct that shape in a test fixture without
   reintroducing the bug.
2. **Convert sweep_flows' 65 flows to store-level unit tests** where the
   assertion is state + screen text: drive each flow via
   `dispatch_key`/store APIs (the same entry points the PTY driver
   targets), assert on `render_view()`/`render_at_width(80)` content and
   store state. Keep the naming (`flow_*` → `unit_flow_*` or a clear
   section) and the discriminating power — NO assertion may be weakened;
   where a PTY flow asserted screen text, the unit twin asserts the same
   text via the static render. Group them in a dedicated test module
   (`sweep_flows_units` or similar) mirroring the flow names so the
   mapping is auditable.
3. **The thin PTY tier stays for what only a live app proves**: input
   encoding, frame cadence / repaint races (the ann-delete transient-echo
   class, U-BHN debounce), process lifecycle (quit-dump), resize, mouse,
   raw CUP stream (`check_cursor_stream` stays as-is), and one
   end-to-end smoke per drive family. Concretely: sweep_flows.py is
   REDUCED to the terminal-only residue (name the kept flows; the rest
   move to units and the PTY file shrinks accordingly). The other suites:
   evaluate each — sweep.py (transitions), drive_windowing(+panes)
   (view-stack/scroll state = store-level), drive_xref/drive_external_*
   (resolution + landing = mostly store-level), probe_notes_dump
   (lifecycle = keep PTY), ux_sweep (keymap derivation = unit-testable),
   drive_syntax_notes, drive_all — demote what is state+text, keep
   what needs the terminal. Publish the kept/converted ledger.
4. **No verdict may be relaxed**: a converted flow's unit twin must be at
   least as discriminating as the PTY flow (same positive signals, no
   absence-style screen-scrapes where a positive assert is possible).
   The fast read-quiet window then applies only to the thin tier.

## Constraints

- Gate: `tools/gate.sh full` green THROUGHOUT (the battery shrinks as you
  convert — the kept suites must stay green). Final: `cargo test
  --workspace` + `tools/gate.sh full` + ONE pooled run to measure the new
  battery time (ALWAYS `cargo build` before the battery — pool.py does
  not rebuild; this bit the session twice).
- Budget: ~70 tool calls (it is a big conversion; stage it: helper →
  sweep_flows conversion in batches of ~20 flows → suites audit).
  If the conversion is not converging by ~half the budget, STOP and
  report the honest state (partial conversion committed, ledger written)
  rather than forcing it.
- Scope fence: `tools/sweep_flows.py`, `tools/*.py` (the drives you
  convert), `tools/gate.sh` (tier definitions + timings), the unit-test
  module in `src/app/store.rs` (or a new `src/app/flow_tests.rs` — the
  app's own test module), `src/ui/root.rs` (the render helper), and
  `docs/ux-testing-plan.md` (the ledger + harness notes: the stale-binary
  lesson — a gate run's REDLINE_BIN must be pinned to the tree being
  tested; pool.py does not rebuild). NO provider/resolver/syntax changes.
- PTY flock discipline as usual.

## Report

The ledger (flow → unit twin or kept-PTY with reason); the new battery
time (pooled, before/after); helper API; tests added (counts);
harness-note updates; honest counts; deviations.

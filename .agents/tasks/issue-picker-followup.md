# Task: finish the picker density work — convert the remaining pickers

## Context
The picker-density lane converted **Xref** and **Symbols** to name-first rows with a
right-aligned detail column (`PickerCandidate` gained `label`/`detail`; the renderer
already does name-first when those are non-empty). **Eight pickers still render the
pre-baked single `display` string**: imenu, impls, palette, find-file, recents, buffers,
project, branch, stash.

The earlier lane stopped short because `src/app/flow_tests.rs` pins contiguous
substrings:
- `flow_tests.rs:3531` — `assert_eq!(new.1, "  new  [fn]")` (pins the candidate's
  `display` field), and
- `flow_tests.rs:3536` — `frame.contains("  new  [fn]")` (pins a contiguous rendered
  substring including its indent and the two-space gap).

**Those are implementation-level pins** (a contiguous rendered substring + an exact
internal field), so under this repo's test-authority policy the requirement wins and
they get **updated**, not treated as a veto. The requirement-level content they protect
must be preserved: **imenu rows still show the kind tag, and impl methods stay
grouped/indented under their struct**.

## What to do
1. For each remaining picker, set `label` and `detail` in its candidate builder so the
   renderer produces name-first rows:
   - **imenu**: label = the (indented) name, detail = `[kind]` — preserving the
     impl-parent grouping/indent that the flow test protects.
   - **impls**: label = the impl/method name, detail = its location/kind.
   - **palette / find-file / recents / buffers / project / branch / stash**: label =
     the item's name (file name, buffer name, branch name, …), detail = the compact
     context (`[kind] path:line`, or the path). For rows whose whole content is a single
     identifier (a palette command name), name-first adds little — say so and use
     judgement; a consistent layout is the goal, not uniformity for its own sake.
2. **Re-pin the two `flow_tests.rs` assertions** to the new shape, keeping the
   requirement-level intent explicit (e.g. assert the label contains `new` AND the detail
   carries `[fn]`, rather than a contiguous `"  new  [fn]"`). Do not simply delete them.
3. Check the PTY drives for picker-row assertions (`tools/sweep_flows.py`,
   `tools/drive_xref.py`): the density lane found none, but re-verify, and mirror any you
   find.
4. Add `render80`-style tests for the converted pickers where a row-shape regression
   would otherwise be unpinned (the store test module is now `src/app/store/tests/*` —
   put them where their subject lives).

## Rules
- **Do not weaken an assertion**: updating a pin to the new shape is fine; dropping the
  *intent* is not. State each re-pin and what it now protects.
- Keep the existing behaviour that matters: the name owns the space and the detail
  truncates tail-keeping; the selected-row bar stays contiguous; nucleo still matches on
  `display` (do not change matching semantics).
- Fence: `src/app/store/mod.rs` (+ `src/app/store/picker.rs` if the builders live there),
  `src/ui/picker.rs`, `src/app/flow_tests.rs`, `src/app/store/tests/*`, docs. **Do NOT
  touch `src/ui/root.rs`** — another lane owns it right now.
- Honest-stop at half budget; converting a subset (say imenu + impls + buffers) is a
  valid landing if the tree is green and the pins are consistent.

## Gate
`cargo build`; `cargo test --workspace`; `cargo clippy --workspace --all-targets` (read
`${PIPESTATUS[0]}`); `timeout 900 tools/gate.sh full`.
**Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` **and swap** — the box is
swap-exhausted and has produced unattributed SIGTERM kills of rustc/test harnesses under
that pressure. If swap is exhausted, run `cargo test --workspace` + the targeted suites
and report the PTY battery as deferred (do not sleep-wait). If a gate run fails, re-run
once, capture the full log, and name the flaking/killed stage.
Budget ~50 tool calls. Report: which pickers converted, the re-pinned assertions and what
they now protect, rendered 80-col samples, gate counts, what remains.

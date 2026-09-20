# Task: investigate + fix — queued/repeating input during streaming (gamestream)

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `src/main.rs` (the event loop), `tools/check_cursor_stream.py` (the
raw-stream harness), and the DA1/startup lessons in docs/ux-testing-plan.md.

## Origin (user report, live usage over herdr gamestream)

"my input feels queued and repeat up/down cursor movement continues
after I let go of the input" — over a streaming connection (latency +
bandwidth), held-key repeats queue faster than the app drains them, and
after release the queue keeps replaying cursor motion. Classic
event-loop starvation: the render tick (or a poll cadence) drains the
event queue slower than it fills, and repeated motion keys are each
APPLIED with full re-parse/render work per event instead of coalesced.

## What to do

1. **Instrument/understand the loop first** (`src/main.rs`): how are
   crossterm events polled (interval? blocking read?), does the render
   tick happen per-event or on a tick, is there any coalescing of
   repeated motion keys (C-n/C-p/C-v/M-v/arrows)? Write a REPRO test:
   a harness that feeds N rapid `C-n` events and asserts (a) final
   state correct, (b) the number of expensive re-render/re-parse passes
   is bounded (coalescing), (c) a release-drain bound (events after a
   simulated "release" don't replay beyond X ms — judge what's
   testable honestly at the unit level vs what needs the PTY tier).
2. **The fix shapes** (decide with evidence): (a) drain-and-coalesce —
   poll ALL pending events each tick, apply the LAST motion event
   (cursor moves) but all state-changing events (alpha, M-x, C-x...)
   individually; (b) render-on-tick instead of render-per-event (the
   render loop already has a tick — verify); (c) key-repeat filter:
   crossterm has KeyEventKind (Press/Repeat) on some platforms — could
   the app track and drop auto-repeat events for motion keys? Judge
   honestly — repeat suppression changes UX (holding C-n in a big file
   SHOULD scroll; the issue is the post-release replay).
3. **The post-release replay is the distinctive bug**: events queued
   during the hold must not replay after release. Coalescing (a) fixes
   it structurally. The PTY harness can simulate burst+release timing
   (check_cursor_stream's raw stream can feed a burst then assert the
   drain window — add a leg).
4. **Byte-for-byte rule**: a single keypress's behavior is unchanged;
   only the queued/coalesced path changes.

## Constraints

- Gate: `cargo test --workspace` + clippy (PIPESTATUS) +
  `tools/gate.sh full` (flock, cargo build first). Budget ~45 tool
  calls; honest-stop at half.
- Scope fence: `src/main.rs` (+ a new event-loop module if the polling
  logic warrants extraction for testability — the 007-04 style),
  `tools/check_cursor_stream.py` (the burst leg), `docs/ux-testing-plan.md`
  (harness notes). NO store.rs logic changes unless the coalescing
  design genuinely requires a store API (state the need in the report;
  prefer keeping store.rs untouched). A parallel lane owns store.rs —
  do not edit it.

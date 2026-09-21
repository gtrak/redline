# Task: fix the racy cursor-position protocol (root cause), then harden the drive

## The mechanism is now PINNED (reproduced live, 78/80 under load)
`src/ui/root.rs:622–650` emits the hardware-cursor position (`?25h` + CUP) in a
**spawned tokio task that sleeps ~12 ms after each frame**, i.e. *outside* the frame's
synchronized-output region. `tools/check_cursor_stream.py` reads with a quiet window
(~150 ms). Under load the timer task is starved past that window, so a read chunk ends at
the frame's `?2026l` with no CUP after it and the drive reports a `None` cursor. It is
not first-frame specific — it is whichever frame's deferred CUP got starved.

**This is a real app-side race, not just a test artifact**: a starved task means the
terminal's hardware cursor is stale for a frame, so a user on a loaded box can see the
cursor lag. That is why Item 1 is the root fix and Item 2 is only the harness gate.

## Item 1 — root fix: the cursor write belongs INSIDE the synchronized frame
In `src/ui/root.rs`, emit the cursor-position sequence as part of the same frame flush
that writes the synchronized update (`?2026h … ?2026l`), instead of from a deferred
timer task. The frame must be self-consistent: whatever the terminal is told about the
cursor must be in the same synchronized region as the content it belongs to.
- Preserve the existing behaviour that matters (cursor hidden/shown appropriately,
  position correct for the active pane, no regression in the existing cursor tests).
- If the 12 ms defer exists for a reason that is not merely "let the frame land" (e.g. a
  flicker workaround), **state that reason** and find a fix that keeps it while removing
  the race — do not silently drop a deliberate behaviour.
- Verify with the drives: `tools/check_cursor_stream.py` (all legs) plus the other
  cursor-touching drives in `tools/gate.sh full`.

## Item 2 — harness: make the drive diagnostic, not flaky
In `tools/check_cursor_stream.py`, stop closing the read window on quiet alone: **keep it
open until a CUP after the final `?2026l` is observed**, with a hard deadline as a
backstop. Then a missing CUP is a real protocol failure (a finding), not a scheduling
artifact. Do not merely increase the sleep — that hides the race and slows the suite.

## Item 3 — the `git::repo` flake: do NOT add retries
The other flake (`stage_file_then_unstage_matches_cli` under two concurrent full suites)
was investigated and has **no timing/ordering assumption to pin**: the wrapper keeps no
index cache, and a git-level probe ruled out the libgit2 stat short-circuit
(`dev=0` vs real `st_dev=30` forces a re-hash). It correlates with a **fully-exhausted
swap** (8 G/8 G used) during the repro. So:
- **Do not** add auto-retry to the gate or the test: an automatic retry converts a
  *detectable* environmental failure into an *invisible* one, which is exactly the
  property we do not want in a gate. (The gate suggested a single auto-retry; this task
  overrides that suggestion, with this reason.)
- Instead: record the correlation in `docs/ux-testing-plan.md` (the flake class, its
  swap-exhaustion signature, and the rule that a lane must not start a full battery when
  swap is exhausted — the resource guards already check `free -g`; add swap to that
  check in the briefs' wording), and make sure a failing run's **full panic text** is
  captured rather than tail-truncated, so the next occurrence is attributable.

## Verification (the point of the task)
- Item 1+2: run `tools/check_cursor_stream.py` **≥10× while the machine is loaded**
  (e.g. concurrently with a full `cargo build` or `cargo test --workspace`) and report
  the pass counts before/after. A single green run proves nothing about a race.
- Then the full battery once.
- Fence: `src/ui/root.rs`, `tools/check_cursor_stream.py`, `docs/ux-testing-plan.md`.
  Nothing else. If Item 1 turns out to need another file, stop and report.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` **and** swap before a
  battery; if memory is tight, run the targeted loops only and report the battery as
  deferred (do not sleep-wait).
Budget ~40 tool calls. Report: the app-side change + why it is race-free, the harness
gate, the ≥10× under-load evidence, and the doc note.

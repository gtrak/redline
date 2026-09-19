# Task: loop-02 (backlog #16) — make the fast PTY read-quiet window safe to default

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## Origin

`tools/pyte_driver.py` has a `REDLINE_PTY_QUIET` knob (default 0.2s). At 0.06
the PTY battery runs **2.9x faster** (measured; `tools/gate.sh full-fast`), but
`tools/sweep_flows.py` drops to 64/65 because two flows assume the app has
settled and use fixed `app.wait(...)` sleeps instead of polling for the
condition they actually need:

- `U-BHN` (banner hint) waits on the watcher's ~500ms debounce.
- `ann-delete` waits on a transient minibuffer message.

That is **recorded as backlog #16** in `docs/ux-testing-plan.md` — read that
row. Full baseline is 65/65 at 0.2, so these are timing-fragile flows, not
regressions.

## Why this matters (the constraint that makes it non-trivial)

Making the fast window the default is a REAL speedup for every gate run
(~2.9x on the PTY battery), but it must not turn real failures into
false PASSES. The dangerous class: `sweep_flows.py` has **~63
absence-style assertions** (`"X" not in screen`, `count == 0`,
`unchanged`). With too-short a settle these pass VACUOUSLY — the screen simply
has not repainted yet, so "the marker is not there" looks identical to "the
feature correctly did not fire". A false positive on an absence assertion is
worse than a slow test: it ships a regression green.

So the job is NOT "lower the window and fix the two known flakes" — it is
"make every timing-sensitive assertion wait on a POSITIVE completion signal,
then lower the window and prove the suite still discriminates".

## What to build

1. **Convert the fragile flows to positive-gated waits.** Preferred shape:
   `key(); wait_for(app, lambda: <positive signal>, timeout); assert(<absence>)`
   where the positive signal proves the action rendered (e.g. the mode line
   back to `ready`, the target row present, an annotation count changed, the
   banner appeared). `wait_for()` already exists in `sweep_flows.py` and is
   predicate-based.
2. **Audit the absence-style assertions.** For each `not in` / `== 0` /
   `unchanged` assertion in `sweep_flows.py`, verify it is gated on a positive
   signal. If a site cannot be gated cheaply, KEEP its explicit settle (raise
   the window for that call) rather than leave a vacuous pass — correctness
   beats speed, and say which sites you left slow and why.
3. **Then lower the default.** Once the suite is robust, make the fast window
   the default for `tools/gate.sh full` (and update the tier docs/comments),
   keeping an escape hatch (env) to run at 0.2. `full-fast` may become
   redundant — decide and document.
4. **Prove it, do not assume it.** Show that the fragile flows now FAIL when
   their condition is genuinely violated (e.g. temporarily break the banner
   hint or the message and confirm the flow reports FAIL) — a test that
   passes both when the feature works and when it is broken is worthless.

## Constraints

- Skills are truth. Write-first; compile early.
- Gate: after the change, `tools/gate.sh full` must be green at the NEW
  default with HONEST counts, and ideally the suite should run FASTER than
  400s (the pooled tier, `tools/gate.sh pooled`, is available and parallel).
- PTY flock: "shared PTY fixture is busy" + exit 3 ⇒ wait and retry; NEVER two
  suites concurrently; wrap EVERY python PTY invocation in `timeout`.
- BUDGET: land within ~60 tool calls; no new investigations after it compiles.
- Scope fence: `tools/sweep_flows.py`, `tools/pyte_driver.py`,
  `tools/gate.sh`, `docs/ux-testing-plan.md` (close backlog #16). No `src/`
  changes.
- Plain `git commit`; do not `git add -A` other sessions' files.

## Verification (iterate until ALL pass)

- `tools/gate.sh full` green at the fast default, honest per-suite counts
  (sweep_flows **65/65**, not 64).
- Run it 3x to show it is STABLE (a flaky fix is not a fix).
- The discrimination proof from item 4 (break a condition, watch it fail,
  restore).
- Report the measured before/after battery time.

## Report format

Which flows/assertions changed and to what positive gate; which sites you
deliberately left slow and why; the default change + escape hatch; the
discrimination proof (what you broke, what failed); 3x stability result;
measured before/after time; deviations.

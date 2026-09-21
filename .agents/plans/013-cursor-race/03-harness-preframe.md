# 013-03 — harness: cover the pre-frame starvation sub-class

**Objective.** The `cup_settle` gate (`b47e03c`) guards the **CUP** only. A second,
independently observed failure sub-class is *not* covered by it: **pre-frame
starvation** — the startup `?25l` itself missed the read window (`?25l=0`), plus
stale-content reads where the drive read before the frame landed. Make those loud
too, or decide explicitly that they are out of scope.

**Independent of the app-side fix** — this is the reader half of the same family
(timing assumptions between a writer and a reader), so it can be done before or
after 013-01/02.

## Key decisions

- **Do not conflate the two sub-classes.** `cup_settle` makes a *starved CUP* a real
  protocol failure; the pre-frame class is a *missing frame*, which the CUP gate
  structurally cannot see (there is no CUP to wait for if the frame never arrived).
- **Same rule as `b47e03c`: make it loud, do not retry.** A read that is missing
  because the frame had not landed must be distinguishable from one that is missing
  because the app failed to emit. The precedent is `check_cursor_stream.py:37-43`:
  keep the read open until the thing being asserted is observed, with a hard deadline
  as a backstop.
- **Consider whether this is a test-only fix.** If the frames are genuinely late (a
  loaded box), the honest outcome may be "this suite is load-sensitive; run it quiet" —
  in which case the deliverable is a documented precondition, not more harness. Decide
  from the evidence, and state which it is.
- **Keep the cost proportional.** `check_cursor_stream.py` is a 1,103-line one-off
  driver; do not grow it into a framework (the 012 worklist already flags it as
  one-off → "only if PTY work resumes").

## Files

| File | Change |
|---|---|
| `tools/check_cursor_stream.py` | the pre-frame assertion / precondition |
| `.agents/plans/archive/012-project-organization.md` | § Gate reliability, class 1 — record the outcome |

## Steps

1. Enumerate the observed pre-frame failures from the `deflake-cursor` gate logs
   (`?25l=0`; stale-content reads) and classify each: late frame vs missing frame.
2. Decide per class: assert-on-observation (like `cup_settle`) or documented
   precondition.
3. Implement the chosen behaviour; keep the hard deadline as a backstop.

## Verification

- Under induced load the suite's failure message names the **class** (starved CUP vs
  missing frame) — the diagnosis must not require re-reading the raw log.
- Idle: still 80/80 (no new false failure).
- `timeout 300 python3 tools/check_cursor_stream.py` clean on a quiet box.

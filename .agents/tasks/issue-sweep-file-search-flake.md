# issue-sweep-file-search-flake: `sweep.py`'s `file -> search` transition intermittently leaves a stale result row

**Observed once, in a full battery on `main` (`ca8a413`), captured before re-running — which is the
point: a flake that actually happened is a data point, and this project has already paid for dismissing
one (the `git::repo` stat-cache lie survived FOUR gates as a "timing flake" and was a real
data-integrity bug).**

```
FAIL  file -> search           keys=C-c p s s,target,RET   mismatch r2:'1  # target README'!=''
FAIL  file -> search
=== sweep.py: FAIL (31s) ===
GATE_EXIT=1
```

The transition opens a project search (`C-c p s s`), types `target`, presses RET, and expects the
result view. Row 2 came back as `1  # target README` where it expected empty — i.e. a **search result
row from a previous state was still on screen**, not a missing result.

**Re-run outcome:** the identical full battery passed immediately afterwards (`GATE_EXIT=0`,
`GATE RESULT: OK (full)`, `sweep.py: OK`), and the lane's own run on the same commit was green. So it is
intermittent.

**A failed isolation attempt, disclosed so nobody repeats it:** running `sweep.py` directly with a
hand-supplied `REDLINE_FIXTURE_ROOT` while *also* deleting that root produces a Python traceback at boot
(`06a home boot ... row0='Traceback (most recent call last):'`) — because the gate treats a
**caller-supplied** root as the caller's to manage and does not create it. That failure was an artifact of
the harness invocation, NOT evidence about this bug. To reproduce, use the gate (`tools/gate.sh full`) or
create the fixture exactly as the gate does.

**What to establish:**
- Whether the stale row is a **render/timing** race (the result view painted a frame late, or a previous
  frame's row survived) or a **state leak** (the previous transition's search buffer/point was reused).
  Those have different fixes, and the assertion to watch is row 2's content, not the presence of results.
- Whether it correlates with load — the box has **0MB free swap** with a resident sglang stack, and a
  sibling lane was active around the failing run. If it only ever appears under load, say so explicitly
  with the conditions; do not close it as "load" without a mechanism, because that is precisely the
  reasoning that dismissed the stat-cache lie.
- Whether `sweep.py`'s transitions share any state that should be per-transition (fixture isolation was
  landed for PTY suites — check whether this path bypasses it).

**Acceptance:** either a mechanism with a deterministic reproducer (then fix it), or a documented,
measured load/condition dependence with the evidence — not a retry that happened to pass.

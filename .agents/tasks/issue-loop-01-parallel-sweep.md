# Task: loop-01 — land the pooled parallel PTY sweep + the status-line wrap fix

You are the implementation worker. Repo root is your cwd **for the working
tree at `/tmp/redline-par` (git branch `par-sweep`)**, NOT `/home/gary/dev/red`
— that main tree is being used by another lane. `.agents/skills/*.md` are
authoritative ground truth.

## Where you are

`/tmp/redline-par` is a git worktree of this repo on branch `par-sweep`, one
WIP commit ahead of `main` (`ba40011`), containing:
- `tools/pool.py` — a NEW pooled parallel sweep harness (see below),
- `src/ui/root.rs` — the status line given `wrap: TextWrap::NoWrap` +
  `overflow: Overflow::Hidden`.

Both came out of showing that the gate battery's ~400 s serial chain is
almost entirely the driver's per-repo flock (all suites share
`/tmp/redline_pyte_repo`). Each suite is an independent process; the only
reason they cannot run together is the SHARED fixture. `pool.py` gives each
concurrently-running suite a private lane (copied fixtures + a rewritten
`tools/` copy pointing at them + a per-lane `XDG_CACHE_HOME`). Because the
driver's flock keys on the repo **abspath** (sha1), lane copies never
contend → real parallelism.

### What is already proven (do not redo)

- DA1 fix is already in `main` (`ab23f03`): App start 2.57s → 0.60s.
- With `pool.py`: **12/12 suites pooled, every verdict identical to serial,
  220.9 s vs 399 s serial (−45%)** on 4 lanes,
  `REDLINE_BIN=/tmp/rl_fixed python3 tools/pool.py runall`.
- The status-line `NoWrap` fix was REQUIRED to get there, and is a genuine
  app bug: a long project path wrapped the status line onto a second row and
  shifted every content row (U-J3 "status line on exactly one row" asserts
  this). Long-path lanes failed before it, passed after.
- Two real bugs were found and fixed in `pool.py` while proving it: a lane
  collision (jobs > lanes shared a lane → flock exit 3) and a fixture-copy
  bug (a sibling pool root got copied in). Verify both fixes survived.

### Hard-won constraints (respect them; they are not obvious)

- **Lane fixture paths must stay the SAME LENGTH OR SHORTER than main's.**
  A longer path wraps the 80-col status line and shifts the hardcoded
  minibuffer row (`MINI=22` in several suites), producing false failures.
  `pool.py` uses `/tmp/rl/<i>/` for exactly this reason. The `NoWrap` fix
  mitigates it, but do not lengthen lane paths.
- **Preserve fixture BASENAMES** (`redline_pyte_repo`, etc.) — suites assert
  those literals (mode-line project name, U-A1/U-J3/U-G5).
- A lane is an exclusive resource: never run two suites in one lane
  concurrently (that is backlog #13's corruption class).

## What to build

1. **Polish `tools/pool.py` into something we keep**: correct `--help`
   output, `setup/run/runall/clean` all working, `existing_lanes()` robust,
   a clear pass/fail summary, a per-suite elapsed time, and honest reporting
   when a lane is busy or a fixture is missing. Remove the leftover
   debugging scaffolding (if any).
2. **Wire it into `tools/gate.sh`** as a new tier, e.g.
   `tools/gate.sh pooled` → `pool.py setup` then `pool.py runall` (keep the
   existing `fast`/`smoke`/`full`/`full-fast`/`equiv` tiers working). The
   existing `full` must stay the sequential, safe, always-works fallback.
   Choose the lane count sensibly (the box has 24 cores; 4–6 lanes measured
   well) and make it overridable via env (e.g. `REDLINE_POOL_LANES`).
3. **Document it**: `docs/ux-testing-plan.md` — a short "Pooled parallel
   sweep" section explaining WHY (the flock serialization), HOW (lanes), the
   two constraints above, and the measured serial-vs-pooled numbers. Add a
   backlog row for `/tmp/redline_pool`-style disk use (4 lanes × ~100 MB) and
   a `pool.py clean`.
4. **Re-verify from scratch on this branch** (do not trust the orchestrator's
   run): build, then `pool.py setup 4` + `pool.py runall` and confirm 12/12
   with the same verdicts as a sequential `tools/gate.sh full` run on the
   SAME binary. Report both timings. Also run the unit tests + clippy (the
   `root.rs` change must keep them green, and add a store/ui unit test that a
   long status line is still one row if feasible — the app fix is the part
   most worth pinning; it may need a render-level test, which is fine).

## Constraints

- Skills are truth (no registry/docs.rs/fetch). Write-first; compile early.
- Work ONLY inside `/tmp/redline-par`. Never touch `/home/gary/dev/red`.
- Every python PTY invocation wrapped in `timeout`; never two suites in one
  lane. Honest gate counts (a pooled summary is NOT a substitute for exact
  per-suite counts — print them).
- Scope fence: `tools/pool.py`, `tools/gate.sh`, `docs/ux-testing-plan.md`,
  and (only if needed to pin the fix) a test in `src/ui/root.rs`. No other
  source changes. Plain `git commit` on `par-sweep`; do not `git add -A`
  unrelated files.

## Report format

pool.py's final interface; the lane model + the two constraints; gate.sh tier
wiring; measured serial vs pooled on the same binary (both timings + exact
per-suite counts); the app-status-line fix + how it is pinned; docs added;
deviations; disk usage + cleanup guidance.

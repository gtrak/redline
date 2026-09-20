# Task: per-invocation PTY fixture isolation (kill the cross-lane contention class)

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `tools/fixture.py`, `tools/pyte_driver.py` (the flock), `tools/pool.py`
(the existing per-lane root), and `tools/gate.sh`.

## Origin (recurring, 3× in one day)

Every battery shares hardcoded fixture roots (`/tmp/redline_pyte_repo` ×17,
plus ~12 other `/tmp/redline_*_repo`s). The flock is keyed to the fixture's
abspath and fails fast, so two CONCURRENT batteries (two lanes, or a lane +
the main checkout) collide: exit-3 refusals, backlog-#12 stray-file residue,
and one lane's app writing into another's fixture (observed: renders
containing another lane's files). Shrinking the battery reduced exposure but
not the design flaw.

## What to build

1. **A single fixture-root knob**: `REDLINE_FIXTURE_ROOT` (default `/tmp`).
   Every fixture path in `tools/` becomes `<root>/<name>` — so a caller can
   give a battery its own tree. Implement the path indirection ONCE (a tiny
   helper in `tools/fixture.py`, imported by the drivers, or an equivalent
   small module) and route ALL hardcoded sites through it (grep for
   `"/tmp/redline` to enumerate; ~29 sites across ~12 files).
2. **gate.sh sets a per-invocation root by default**:
   `REDLINE_FIXTURE_ROOT=${REDLINE_FIXTURE_ROOT:-/tmp/fx$$}` (SHORT — the
   pool-lane path-length lesson: keep the root tiny so fixture paths don't
   grow; `/tmp/fx<pid>` is +3 chars). Then two concurrent `gate.sh full`
   runs cannot collide, and the per-repo flock becomes an intra-root
   guard (belt and braces) rather than the only defence.
3. **pool.py unifies**: its per-lane root (`REDLINE_POOL_ROOT`, default
   `/tmp/rl`) should BE the fixture root (set `REDLINE_FIXTURE_ROOT` per
   lane) instead of a second parallel mechanism.
4. **fixture.py reset** must operate on the resolved root (so
   `tools/fixture.py` resets the caller's tree), and the flock message
   should name the resolved fixture path/root when it refuses.
5. **Preserve**: fixture BASENAMES (assertions depend on them), short
   paths, and the existing single-run behavior byte-for-byte (a lone
   battery is unaffected except for the root spelling).

## Verification (the acceptance criterion)

- **The discriminating test: run TWO batteries concurrently and both must be
  green.** e.g. `REDLINE_FIXTURE_ROOT=/tmp/fa tools/gate.sh full &` and
  `REDLINE_FIXTURE_ROOT=/tmp/fb tools/gate.sh full &` → both `GATE RESULT:
  OK`. (Before this change, at least one fails.) Capture the evidence.
- Also: a single `tools/gate.sh full` green (no regression), and
  `tools/pool.py setup 4 && tools/pool.py runall --lanes 4` green with the
  unified root.
- Add a harness note in `docs/ux-testing-plan.md` replacing the recorded
  "cross-lane PTY contention (open)" item with the resolution (per-invocation
  fixture roots; the contamination class is closed structurally).

## Constraints

- Gate discipline: `cargo build` before batteries; the concurrent-battery
  test is the point (expect memory pressure — two batteries is 2× suites;
  consider `REDLINE_PTY_QUIET` default and the reduced tier make this
  feasible: the battery is now ~85s pooled / ~4min serial each).
- Fence: `tools/*.py` (fixture paths + pool unification), `tools/gate.sh`,
  `docs/ux-testing-plan.md`. NO src/ changes (this is harness-only).
- Budget ~45 tool calls; honest-stop at half (commit the indirection + the
  sites converted, report what remains).

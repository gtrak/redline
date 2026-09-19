#!/usr/bin/env python3
"""Fixture pooling + parallel PTY sweep.

WHY. Every PTY suite hardcodes the same fixture paths (/tmp/redline_pyte_repo
etc.), so the driver's per-repo flock serializes them into one long chain
(~400 s for the gate battery). The suites are independent processes; the only
reason they cannot run together is the SHARED fixture. Give each
concurrently-running suite its own private copy of the fixtures and they
parallelize to max(suite) instead of sum(suite).

HOW (no edits to tools/*.py). For each lane we:
  1. copy the baseline fixture repos (/tmp/redline_*) into  <POOL_ROOT>/<i>/
  2. copy tools/ into that lane and rewrite the hardcoded fixture literals to
     point at the lane copy (and the tools-import path),
  3. run the suite there with a per-lane XDG_CACHE_HOME (isolates the app's
     log + projects registry).
The driver's lock is keyed on repo abspath (sha1), so lane copies never
contend -- real parallelism, and a lane crash cannot corrupt another lane.

Constraints (see docs/ux-testing-plan.md, "Pooled parallel sweep"):
  * Lane fixture paths must stay SHORT. The status line shows the project
    NAME (the basename), and that basename is preserved verbatim (suites
    assert the literals: mode-line project name, U-A1/U-J3/U-G5). The pool
    root is kept tiny (/tmp/rl) so the full path only grows a few chars and
    cannot wrap the 80-col status line onto a second row.
  * A lane is an exclusive resource: never run two suites in one lane
    concurrently (that is backlog #13's corruption class). Enforced by a
    lane queue: each worker checks a lane out for the life of a suite.

Usage:
  tools/pool.py setup [N]                 # create N lanes (default 4)
  tools/pool.py run [--lanes N] SUITE...  # run suites pooled; one per lane
  tools/pool.py runall [--lanes N]        # the gate battery, pooled
  tools/pool.py clean                     # remove the pool root

Env:
  REDLINE_POOL_LANES  default lane count (setup/runall)
  REDLINE_POOL_ROOT   pool root dir (default /tmp/rl)
  REDLINE_MAIN        worktree whose tools/ gets copied into lanes
                      (default: the worktree containing this script)
  REDLINE_BIN         binary under test (default <main>/target/debug/redline)
"""
import os
import shutil
import subprocess
import sys

# Default to the worktree that contains this script, so a lane's tools copy is
# always this tree's (never a sibling checkout the caller is not working in).
MAIN = os.environ.get(
    "REDLINE_MAIN",
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
)
POOL_ROOT = os.environ.get("REDLINE_POOL_ROOT", "/tmp/rl")
FIXTURE_ROOT = "/tmp"
LANE_REPO_PREFIX = "redline_"
DEFAULT_LANES = 4

# The gate battery (tools/gate.sh full), cheapest-heavyweight ordering aside --
# order does not matter here because lanes, not a shared fixture, serialize.
BATTERY = [
    "sweep.py", "drive_all.py", "drive_windowing.py",
    "drive_windowing_panes.py", "check_cursor_stream.py", "ux_sweep.py",
    "probe_notes_dump.py", "drive_syntax_notes.py", "drive_xref.py",
    "drive_external_notes.py", "drive_external_crate.py", "sweep_flows.py",
]


def default_lanes():
    return int(os.environ.get("REDLINE_POOL_LANES", DEFAULT_LANES))


def _fixture_dirs():
    """Every <FIXTURE_ROOT>/redline_* dir, EXCEPT the pool root and any sibling
    pool root (a dir whose children are numbered lane dirs). Never copy a pool
    into a lane (that is the sibling-pool-copy bug)."""
    out = []
    for name in sorted(os.listdir(FIXTURE_ROOT)):
        if not name.startswith(LANE_REPO_PREFIX):
            continue
        p = os.path.join(FIXTURE_ROOT, name)
        if not os.path.isdir(p) or p.startswith(POOL_ROOT):
            continue
        if any(os.path.isdir(os.path.join(p, d)) and d.isdigit() for d in os.listdir(p)):
            continue
        out.append(p)
    return out


def lane_dir(i):
    return os.path.join(POOL_ROOT, str(i))


def setup(n):
    os.makedirs(POOL_ROOT, exist_ok=True)
    fixtures = _fixture_dirs()
    if not fixtures:
        print("pool: no %s/redline_* fixtures found under %s; "
              "run a PTY suite once in the main tree to create them, "
              "or check the env" % (LANE_REPO_PREFIX, FIXTURE_ROOT),
              file=sys.stderr)
        return 1
    for i in range(n):
        lane = lane_dir(i)
        if os.path.exists(lane):
            shutil.rmtree(lane)
        os.makedirs(lane)
        for f in fixtures:
            # Keep the FULL basename (redline_pyte_repo, ...): suites assert
            # those literals (mode-line project name, U-A1/U-J3/U-G5). Only the
            # PARENT dir is a short pool root (/tmp/rl/<i>), so a lane path
            # stays close to main's length and the 80-col status line cannot
            # wrap into a hardcoded row.
            shutil.copytree(f, os.path.join(lane, os.path.basename(f)), symlinks=True)
    # Each lane gets its OWN tools copy (so a mutated .py in one lane cannot
    # leak to another); the copy is identical across lanes.
    for i in range(n):
        lane = lane_dir(i)
        tdir = os.path.join(lane, "tools")
        shutil.copytree(os.path.join(MAIN, "tools"), tdir)
        shutil.rmtree(os.path.join(tdir, "__pycache__"), ignore_errors=True)
        _rewrite(tdir, lane)
        os.makedirs(os.path.join(lane, "xdg"), exist_ok=True)
    print("pool: %d lanes at %s (%d fixtures each, %d total fixture copies)"
          % (n, POOL_ROOT, len(fixtures), len(fixtures)))
    return 0


def _rewrite(tdir, lane):
    """Point the lane's tools at the lane's own fixtures + tools dir."""
    for name in os.listdir(tdir):
        if not name.endswith(".py"):
            continue
        p = os.path.join(tdir, name)
        with open(p) as f:
            s = f.read()
        # Fixture paths: /tmp/redline_X -> <lane>/redline_X (basename preserved).
        s = s.replace("/tmp/" + LANE_REPO_PREFIX, lane + "/" + LANE_REPO_PREFIX)
        # Tools import path (most suites hardcode the main tree's tools dir).
        s = s.replace('sys.path.insert(0, "%s/tools")' % MAIN,
                      'sys.path.insert(0, "%s")' % tdir)
        s = s.replace('"/home/gary/dev/red/tools"', '"%s"' % tdir)
        with open(p, "w") as f:
            f.write(s)


def existing_lanes():
    """Lane indices actually created by setup (ascending). A lane is a numbered
    dir that carries its own tools/ copy."""
    if not os.path.isdir(POOL_ROOT):
        return []
    out = []
    for name in os.listdir(POOL_ROOT):
        if name.isdigit() and os.path.isdir(os.path.join(POOL_ROOT, name, "tools")):
            out.append(int(name))
    return sorted(out)


def _count_line(out):
    """The suite's own verdict line (its exact X/Y count), or its last line."""
    lines = [l for l in out.splitlines() if l.strip()]
    if not lines:
        return "(no output)"
    for l in reversed(lines):
        low = l.lower()
        if any(k in low for k in ("total:", "result:", "driven:", "legs passed",
                                  "finding", "transitions clean", "steps ok",
                                  "checks pass", "scenarios pass", "flows, ")):
            return l.strip()[:80]
    return lines[-1].strip()[:80]


def run_pooled(suites, lanes, timeout=1800, binpath=None):
    """Run each suite in its OWN lane; at most `lanes` run concurrently.

    Returns one row per suite: (suite, rc, stdout, stderr, elapsed_seconds).
    A lane is an exclusive resource (its fixture is private): two suites must
    NEVER share one concurrently (that is exactly backlog #13's corruption).
    So we use a lane QUEUE -- each worker checks a free lane out until all
    lanes are checked out, runs, and returns it. Jobs > lanes then queue rather
    than collide.
    """
    import concurrent.futures as cf
    import queue as queue_mod
    import time
    binpath = binpath or os.environ.get(
        "REDLINE_BIN", os.path.join(MAIN, "target/debug/redline"))
    avail = existing_lanes()
    if not avail:
        raise SystemExit("pool: no lanes set up -- run `tools/pool.py setup` first")
    free_lanes = queue_mod.Queue()
    for i in avail:
        free_lanes.put(i)
    n_workers = max(1, min(lanes, len(avail), len(suites)))

    def one(suite):
        lane_idx = free_lanes.get()
        lane = lane_dir(lane_idx)
        try:
            env = dict(os.environ)
            env["XDG_CACHE_HOME"] = os.path.join(lane, "xdg")
            env["REDLINE_BIN"] = binpath
            env["REDLINE_PTY_QUIET"] = os.environ.get("REDLINE_PTY_QUIET", "0.2")
            env.pop("REDLINE_NO_PTY_LOCK", None)  # keep the per-lane safety lock
            tdir = os.path.join(lane, "tools")
            t0 = time.time()
            try:
                cp = subprocess.run(
                    ["timeout", str(timeout), sys.executable, suite],
                    cwd=tdir, env=env, capture_output=True, text=True,
                    timeout=timeout + 60)
                return suite, cp.returncode, cp.stdout, cp.stderr, time.time() - t0
            except subprocess.TimeoutExpired:
                return suite, 124, "", "pooled run timed out after %ss" % timeout, \
                    time.time() - t0
        finally:
            free_lanes.put(lane_idx)

    out = {}
    with cf.ThreadPoolExecutor(max_workers=n_workers) as ex:
        for row in ex.map(one, suites):
            out[row[0]] = row
    return [out[s] for s in suites]


def cmd_run(suites, lanes):
    import time
    avail = existing_lanes()
    if not avail:
        raise SystemExit("pool: no lanes set up -- run `tools/pool.py setup` first")
    if lanes > len(avail):
        print("pool: requested %d lanes but only %d exist -- running on %d"
              % (lanes, len(avail), len(avail)), file=sys.stderr)

    t0 = time.time()
    rows = run_pooled(suites, lanes)
    dt = time.time() - t0
    bad = 0
    width = max(len(s) for s, *_ in rows)
    for suite, rc, out, err, secs in rows:
        fails = sum(1 for l in out.splitlines()
                    if l.strip().startswith("FAIL"))
        ok = rc == 0 and not fails
        if not ok:
            bad += 1
        flag = "OK " if ok else "BAD"
        note = ""
        if rc == 3:
            note = " [fixture lock contention -- another suite holds this repo]"
        elif rc == 124:
            note = " [timed out]"
        print("%s %-*s  rc=%d fails=%d  %6.1fs  :: %s%s"
              % (flag, width, suite, rc, fails, secs, _count_line(out), note))
        if rc not in (0, 3, 124) and err.strip():
            print("      stderr: " + err.strip().splitlines()[-1][:120])
    print("\npool run: %d/%d suites OK, %d bad, in %.1fs (lanes=%d)"
          % (len(rows) - bad, len(rows), bad, dt, min(lanes, len(avail))))
    return 1 if bad else 0


def main():
    args = sys.argv[1:]
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        return 0 if args else 2
    cmd = args[0]
    if cmd == "clean":
        shutil.rmtree(POOL_ROOT, ignore_errors=True)
        print("pool: cleaned %s" % POOL_ROOT)
        return 0
    if cmd == "setup":
        n = int(args[1]) if len(args) > 1 else default_lanes()
        return setup(n)
    if cmd in ("run", "runall"):
        lanes = default_lanes()
        rest = []
        i = 1
        while i < len(args):
            if args[i] == "--lanes":
                lanes = int(args[i + 1])
                i += 2
            else:
                rest.append(args[i])
                i += 1
        suites = BATTERY if cmd == "runall" else rest
        if not suites:
            print("no suites given", file=sys.stderr)
            return 2
        if not existing_lanes():
            rc = setup(lanes)
            if rc:
                return rc
        return cmd_run(suites, lanes)
    print("unknown command: %s  (try --help)" % cmd, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())

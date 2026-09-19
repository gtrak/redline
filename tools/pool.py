#!/usr/bin/env python3
"""Fixture pooling + parallel PTY sweep.

WHY. Every PTY suite hardcodes the same fixture paths (/tmp/redline_pyte_repo
etc.), so the driver's per-repo flock serializes them into one long chain
(sum ~400s). The suites are independent processes; the only reason they cannot
run together is the SHARED fixture. Give each concurrently-running suite its
own private copy of the fixtures and they parallelize to max() instead of sum().

HOW (no edits to tools/*.py). For each lane we:
  1. copy the baseline fixture repos into  <POOL_ROOT>/lane<N>/
  2. copy tools/ into that lane and rewrite the hardcoded fixture literals to
     point at the lane copy (and the tools-import path),
  3. run the suite there with a per-lane XDG_CACHE_HOME (isolates the app's
     log + projects registry).
The driver's lock is keyed on repo abspath (sha1), so lane copies never
contend -- real parallelism, and a lane crash cannot corrupt another lane.

Usage:
  tools/pool.py setup [N]                 # create N lanes (default nproc/3)
  tools/pool.py run [--lanes N] SUITE...  # run suites pooled; one per lane
  tools/pool.py runall [--lanes N]        # the gate battery, pooled
  tools/pool.py clean
"""
import os
import shutil
import subprocess
import sys

MAIN = os.environ.get("REDLINE_MAIN", "/home/gary/dev/red")
POOL_ROOT = os.environ.get("REDLINE_POOL_ROOT", "/tmp/rl")
LANE_REPO_PREFIX = "redline_"

# Fixture dirs that are CACHES, not repos: copy them too (suites read state
# from them), but they are never git repos.
BATTERY = [
    "sweep.py", "drive_all.py", "drive_windowing.py",
    "drive_windowing_panes.py", "check_cursor_stream.py", "ux_sweep.py",
    "probe_notes_dump.py", "drive_xref.py", "drive_external_notes.py",
    "drive_external_crate.py", "drive_syntax_notes.py", "sweep_flows.py",
]


def _fixture_dirs():
    """Every /tmp/redline_* dir except the pool root itself."""
    out = []
    for name in sorted(os.listdir("/tmp")):
        if not name.startswith(LANE_REPO_PREFIX):
            continue
        p = os.path.join("/tmp", name)
        # Skip this pool root and any sibling pool root (detected by the
        # presence of numbered lane dirs), never copy a pool into a lane.
        if not os.path.isdir(p) or p.startswith(POOL_ROOT):
            continue
        if any(os.path.isdir(os.path.join(p, d)) and d.isdigit()
               for d in (os.listdir(p) if os.path.isdir(p) else [])):
            continue
        out.append(p)
    return out


def lane_dir(i):
    return os.path.join(POOL_ROOT, str(i))


def default_lanes():
    n = os.cpu_count() or 4
    return max(2, min(6, n // 3))


def setup(n):
    os.makedirs(POOL_ROOT, exist_ok=True)
    fixtures = _fixture_dirs()
    for i in range(n):
        lane = lane_dir(i)
        if os.path.exists(lane):
            shutil.rmtree(lane)
        os.makedirs(lane)
        for f in fixtures:
            # Keep the FULL basename (redline_pyte_repo, ...): suites assert
            # those literals (mode-line project name, U-A1/U-J3/U-G5). Only the
            # PARENT dir is shortened, so a lane path stays <= main's length
            # and the 80-col status line cannot wrap into a hardcoded row.
            shutil.copytree(f, os.path.join(lane, os.path.basename(f)), symlinks=True)
    # Lane 0 also carries the tools copy (identical for all lanes; each lane
    # gets its own so a mutated .py in one lane cannot leak).
    for i in range(n):
        lane = lane_dir(i)
        tdir = os.path.join(lane, "tools")
        shutil.copytree(os.path.join(MAIN, "tools"), tdir)
        shutil.rmtree(os.path.join(tdir, "__pycache__"), ignore_errors=True)
        _rewrite(tdir, lane)
        os.makedirs(os.path.join(lane, "xdg"), exist_ok=True)
    print("pool: %d lanes at %s (%d fixtures each)" % (n, POOL_ROOT, len(fixtures)))


def _rewrite(tdir, lane):
    """Point the lane's tools at the lane's own fixtures + tools dir."""
    for name in os.listdir(tdir):
        if not name.endswith(".py"):
            continue
        p = os.path.join(tdir, name)
        with open(p) as f:
            s = f.read()
        # Fixture paths: /tmp/redline_X -> <lane>/redline_X. The lane dir is
        # SHORT (/tmp/rl/<i> = 9 chars) vs main's /tmp/ (5), so a lane path is
        # only +4 -- the full basename is preserved for the suites' literal
        # project-name assertions, and the tiny delta keeps the mode line
        # within 80 cols (a longer path wraps and shifts the minibuffer row).
        s = s.replace("/tmp/" + LANE_REPO_PREFIX, lane + "/" + LANE_REPO_PREFIX)
        # Tools import path (most suites hardcode the main tree's tools dir).
        s = s.replace('sys.path.insert(0, "%s/tools")' % MAIN,
                      'sys.path.insert(0, "%s")' % tdir)
        s = s.replace('"/home/gary/dev/red/tools"', '"%s"' % tdir)
        with open(p, "w") as f:
            f.write(s)


def existing_lanes():
    """Lane indices actually created by setup (ascending)."""
    if not os.path.isdir(POOL_ROOT):
        return []
    out = []
    for name in sorted(os.listdir(POOL_ROOT)):
        if name.isdigit():
            if os.path.isdir(os.path.join(POOL_ROOT, name, "tools")):
                out.append(int(name))
    return sorted(out)


def run_pooled(suites, lanes, timeout=1800, binpath=None):
    """Run each suite in its OWN lane; at most `lanes` run concurrently.

    A lane is an exclusive resource (its fixture is private): two suites must
    NEVER share one concurrently (that is exactly backlog #13's corruption).
    So we use a lane QUEUE — each worker thread takes a free lane until all
    lanes are checked out, runs, and returns it. Jobs > lanes then queue
    rather than collide.
    """
    import concurrent.futures as cf
    import queue as queue_mod
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
        try:
            lane = lane_dir(lane_idx)
            env = dict(os.environ)
            env["XDG_CACHE_HOME"] = os.path.join(lane, "xdg")
            env["REDLINE_BIN"] = binpath
            env["REDLINE_PTY_QUIET"] = os.environ.get("REDLINE_PTY_QUIET", "0.2")
            env.pop("REDLINE_NO_PTY_LOCK", None)  # keep the per-lane safety lock
            tdir = os.path.join(lane, "tools")
            try:
                cp = subprocess.run(
                    ["timeout", str(timeout), sys.executable, suite],
                    cwd=tdir, env=env, capture_output=True, text=True,
                    timeout=timeout + 60)
                return suite, cp.returncode, cp.stdout, cp.stderr
            except subprocess.TimeoutExpired:
                return suite, 124, "", "pooled run timed out"
        finally:
            free_lanes.put(lane_idx)

    out = {}
    with cf.ThreadPoolExecutor(max_workers=n_workers) as ex:
        for suite, rc, o, e in ex.map(one, suites):
            out[suite] = (suite, rc, o, e)
    return [out[s] for s in suites]


def _summary(out):
    for line in reversed(out.splitlines()):
        low = line.lower()
        if any(k in low for k in ("total:", "result:", "legs passed",
                                  "driven:", "finding")):
            return line.strip()[:60]
    return (out.strip().splitlines() or ["(no output)"])[-1][:60]


def cmd_run(suites, lanes):
    import time
    t0 = time.time()
    rows = run_pooled(suites, lanes)
    dt = time.time() - t0
    bad = 0
    for suite, rc, out, err in rows:
        fails = sum(1 for l in out.splitlines() if l.startswith("FAIL"))
        if rc != 0 or fails:
            bad += 1
        flag = "OK " if (rc == 0 and not fails) else "BAD"
        print("%s %-26s rc=%d fails=%d :: %s" % (flag, suite, rc, fails, _summary(out)))
        if rc != 0 and err.strip():
            print("      stderr:", err.strip().splitlines()[-1][:120])
    print("pool run: %d suites in %.1fs (lanes=%d), %d bad" % (len(rows), dt, lanes, bad))
    return 1 if bad else 0


def main():
    args = sys.argv[1:]
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        return 0 if args else 2
    cmd = args[0]
    if cmd == "clean":
        shutil.rmtree(POOL_ROOT, ignore_errors=True)
        print("pool: cleaned")
        return 0
    if cmd == "setup":
        n = int(args[1]) if len(args) > 1 else default_lanes()
        setup(n)
        return 0
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
        if not os.path.isdir(lane_dir(0)):
            setup(lanes)
        return cmd_run(suites, lanes)
    print("unknown command: %s" % cmd, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())

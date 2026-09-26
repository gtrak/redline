#!/usr/bin/env python3
"""probe_file_search — bounded loop probe of the `file -> search` transition.

Issue: issue-sweep-file-search-flake. Captured failure (main ca8a413 gate):
  FAIL  file -> search  keys=C-c p s s,target,RET  mismatch r2:'1  # target README'!=''
The diff() convention is r{i}:<ref>!=<actual>, so the ACTUAL (post-transition)
frame had row 2 BLANK while the clean reference carried the first hit row.
This probe repeats that transition N times (N=PROBE_ITERS, default 15,
hard-capped at 40) and prints the iteration number + rows 0..5 of BOTH
frames on every iteration, dumping the full frames on a mismatch. It also
flags when the REFERENCE itself is unstable (r2 blank in the reference —
the late-frame race on the capture side, not the transition side).

Bounded by construction: fixed iteration cap, one printed row per
iteration, exits at the cap or on the first mismatch (with full dumps).
"""
import os
import shutil
import sys

# PRIVATE fixture root (issue-sweep contention hygiene): the probe seeds its
# own copy of the baseline from /tmp and drives it. It never mutates the
# shared /tmp baseline and never takes the shared baseline's lock, so a
# concurrent gate (which seeds /tmp/fx<pid> from /tmp) cannot observe the
# probe's reset in its own copy, and the probe cannot observe a gate's.
ROOT = "/tmp/fsprobe_%d" % os.getpid()
os.makedirs(ROOT, exist_ok=True)
_dst = os.path.join(ROOT, "redline_pyte_repo")
if not os.path.isdir(_dst):
    shutil.copytree("/tmp/redline_pyte_repo", _dst)
os.environ["REDLINE_FIXTURE_ROOT"] = ROOT

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo, reset as _reset_fixture
from sweep import ROWS, COLS, drive_to, stable_capture, diff

REPO = repo("redline_pyte_repo")
N = min(int(os.environ.get("PROBE_ITERS", "15")), 40)


def selftest():
    """Unit pin for App.frame_complete() (the gate this fix builds on):
    synthetic raw tails, no PTY involved."""
    from pyte_driver import App
    d = App.__new__(App)  # no PTY; frame_complete only reads raw_tail
    h, l, cup = b"\x1b[?2026h", b"\x1b[?2026l", b"\x1b[3;1H"
    d.raw_tail = b""
    assert d.frame_complete(), "no frame yet -> complete"
    d.raw_tail = h + b"title" + l + b"row2 partial"
    assert not d.frame_complete(), "content after sync-close -> torn"
    d.raw_tail = h + b"abc" + l + b"\x1b[?25h" + b"\x1b["
    assert not d.frame_complete(), "CUP split mid-CSI -> torn"
    d.raw_tail = h + b"abc" + l + b"\x1b[?25h" + cup
    assert d.frame_complete(), "sync-close + 25h + CUP -> complete"
    d.raw_tail = h + b"abc" + l
    assert d.frame_complete(), "sync-close, no cursor (picker/home) -> complete"
    print("frame_complete selftest: 5/5")


def frame_rows(app):
    return [app.row_text(r) for r in range(0, ROWS - 2)]


def main():
    selftest()
    _reset_fixture()
    reproduced = False
    for i in range(1, N + 1):
        # The in-flight App (if any) is reaped on ANY exit path: an orphaned
        # app child is exactly the "probe outlives its run" leak the
        # contention class is about.
        app = None
        try:
            # Phase A: the clean reference (sweep.py's own method — fresh
            # PTY, drive search from home, stable capture).
            app = App(REPO, rows=ROWS, cols=COLS)
            drive_to(app, "search")
            ref = stable_capture(app)
            app.kill()
            app = None
            # Phase B: the transition. Faithful copy of sweep.py's
            # transitions() preamble up to the file view (one App lifetime,
            # exactly the view history the real run had), then C-c p s s /
            # target / RET.
            app = App(REPO, rows=ROWS, cols=COLS)
            app.key("C-x g")
            app.wait(1.0)
            app.key("l")
            app.wait(1.0)
            app.key("q")
            app.wait(0.6)
            app.key("q")
            app.wait(0.6)
            app.key("M-x")
            app.wait(0.8)
            app.key("C-g")
            app.wait(0.4)
            app.key("C-x C-f")
            app.wait(0.8)
            app.key("C-g")
            app.wait(0.4)
            drive_to(app, "file")
            app.key("C-c p s s")
            for ch in "target":
                app.key(ch, settle=0.15)
            app.key("RET")
            app.wait_done()
            app.wait(0.5)
            actual = stable_capture(app)
            app.kill()
            app = None
        finally:
            if app is not None:
                try:
                    app.kill()
                except Exception:
                    pass

        ref_r2 = ref[2].strip()
        act_r2 = actual[2].strip()
        print("[%02d] ref_r2=%r act_r2=%r" % (i, ref_r2, act_r2))
        if not ref_r2:
            print("[%02d] REFERENCE unstable: r2 blank in the clean frame" % i)
        stale, mismatch = diff(ref, actual)
        if stale or mismatch:
            reproduced = True
            print("[%02d] MISMATCH -> full frames (ref | actual):" % i)
            for r in range(0, ROWS - 2):
                mark = "*" if ref[r].strip() != actual[r].strip() else " "
                print(" %s r%-2d ref    %r" % (mark, r, ref[r]))
                print(" %s r%-2d actual %r" % (mark, r, actual[r]))
            print("[%02d] stale=%r mismatch=%r" % (i, stale, mismatch))
            break
    print("file-search probe: %d iterations, reproduced=%s" % (i, reproduced))
    sys.exit(0 if not reproduced else 1)


if __name__ == "__main__":
    main()

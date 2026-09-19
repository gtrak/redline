#!/usr/bin/env python3
"""Plan 002 issue 03 — Sweep & re-measure: the layout-shift overprint check.

Method (reference-frame diff, the most rigorous pyte assertion available):

  1. For each view V, drive V to a canonical state in a fresh PTY and capture
     its settled content frame (rows 0..ROWS-3) as the reference.
  2. For each named transition (prev -> V), drive the switch and capture the
     settled content frame.
  3. Diff the post-transition frame against the reference for V:
       * STALE  — a row that is blank in the reference but carries content in
                  the post-transition frame (leftover from the previous view);
       * MISMATCH — a row whose content differs from the reference (wrong or
                  bled-in content on the same row).

The reference frame is the ground truth for "what this view's rows should
contain"; any content on a blank-reference row after a switch is, by
definition, stale content belonging to another view.

Content rows are rows 0..ROWS-3; the minibuffer (row ROWS-2) and status line
(row ROWS-1) are excluded (messages may legitimately persist).

Exit 0 = all transitions clean; 1 = overprint found.
"""
import re
import sys
sys.path.insert(0, "/home/gary/dev/red/tools")
from pyte_driver import App
from fixture import reset as _reset_fixture

REPO = "/tmp/redline_pyte_repo"
ROWS, COLS = 24, 80

# The log view renders a trailing relative-time token (e.g. "48m") that ticks
# with the clock; normalize it out before diffing so a one-minute drift between
# the reference capture and the transition capture is not flagged as overprint.
_TIME_RE = re.compile(r"\s+\d+[smhd]$")
# 06a: the home header gains a live dirty-count suffix (" +1 ~1 ?0") once the
# git status lands (e.g. after C-x g in the same PTY); it is view STATE, not
# overprint — normalize it out like the relative-time token.
_DIRTY_RE = re.compile(r"\s*\+\d+ ~\d+ \?\d+\s*$")


def _norm(row):
    return _DIRTY_RE.sub("", _TIME_RE.sub("", row))


def content_rows(app):
    return list(range(0, app.rows - 2))


def capture(app):
    return [app.row_text(r) for r in content_rows(app)]


def stable_capture(app, tries=8, pause=0.35):
    """Capture until the frame is quiescent (two identical captures `pause`
    seconds apart) so timing variance does not produce spurious diffs."""
    import time
    prev = None
    frame = capture(app)
    for _ in range(tries):
        time.sleep(pause)
        app._read(0.1, quiet=0.02)
        cur = capture(app)
        if cur == prev:
            return cur
        prev = cur
        frame = cur
    return frame


def diff(ref, actual):
    """Return (stale, mismatch): rows blank in ref but not in actual, and rows
    whose content differs between ref and actual."""
    stale, mismatch = [], []
    for i in range(len(ref)):
        a, b = _norm(ref[i]).strip(), _norm(actual[i]).strip()
        if a == "" and b != "":
            stale.append((i, b))
        elif a != b:
            mismatch.append((i, a, b))
    return stale, mismatch


def drive_to(app, view):
    """Drive `app` to the canonical state of `view` from home."""
    if view == "home":
        return
    if view == "file":
        app.key("C-c p t")
        app.key("down")
        app.key("down")
        app.key("RET")
        app.wait(1.0)
        app.key("C-c p t")  # hide the tree so the frame is buffer-only
        app.wait(0.5)
        return
    if view == "magit":
        app.key("C-x g")
        app.wait(1.0)
        return
    if view == "log":
        app.key("C-x g")
        app.wait(0.6)
        app.key("l")
        app.wait(1.0)
        return
    if view == "search":
        app.key("C-c p s s")
        for ch in "target":
            app.key(ch, settle=0.15)
        app.key("RET")
        app.wait_done()
        app.wait(0.5)
        return
    if view == "palette":
        app.key("M-x")
        app.wait(0.8)
        return
    if view == "picker":
        app.key("C-x C-f")
        app.wait(0.8)
        return
    if view == "magit_tree":
        app.key("C-x g")
        app.wait(1.0)
        app.key("C-c p t")
        app.wait(0.8)
        return
    raise ValueError(view)


RESULTS = []


def record(name, keys, ok, detail):
    RESULTS.append((name, keys, ok, detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name:24s} keys={keys:22s} {detail}")


def home_boot():
    """06a boot leg: the first frame is HOME — project + "redline" header,
    the derived group sample (labels/keys exactly once), the standard help
    line, and NO `*scratch*` anywhere. A fresh PTY each."""
    app = App(REPO, rows=ROWS, cols=COLS)
    frame = [app.row_text(r) for r in range(ROWS)]
    text = "\n".join(frame)
    once = lambda tok: sum(1 for r in frame if tok in r) == 1
    header = "redline" in frame[0] and "redline_pyte_repo" in frame[0]
    groups = ("[buffers]" in text and "[files]" in text and "[git]" in text
              and once("[buffers]") and once("[git]") and once("C-x g")
              and "C-x C-f" in text and "C-x C-c" in text)
    help_line = "C-x C-c quit" in text and "? menu" in text
    no_scratch = "*scratch*" not in text
    ok = header and groups and help_line and no_scratch and "ready" in text
    record("06a home boot", "(launch)", ok,
           f"header={header} derived-groups={groups} help-line={help_line} "
           f"no-scratch={no_scratch} ready={'ready' in text} (row0={frame[0]!r})")
    app.kill()


def refs():
    """Capture a clean reference frame for every view (fresh PTY each)."""
    _reset_fixture()
    out = {}
    for view in ["home", "file", "magit", "log", "search", "palette", "picker", "magit_tree"]:
        app = App(REPO, rows=ROWS, cols=COLS)
        drive_to(app, view)
        out[view] = stable_capture(app)
        app.kill()
    return out


def transitions(refs):
    app = App(REPO, rows=ROWS, cols=COLS)
    # home -> magit
    app.key("C-x g")
    run(app, refs, "home -> magit", "C-x g", "magit")
    # magit -> log
    app.key("l")
    run(app, refs, "magit -> log", "l", "log")
    # log -> magit
    app.key("q")
    run(app, refs, "log -> magit (q)", "q", "magit")
    # magit -> home
    app.key("q")
    run(app, refs, "magit -> home (q)", "q", "home")
    # Palette open/close (the historically-open overprint surface) and the
    # file-picker open/close, both on the clean home frame: the picker
    # canvas renders below the main view, so the home rows are part of the
    # diffed frame and must still be the pristine home rows.
    app.key("M-x")
    run(app, refs, "home -> palette (M-x)", "M-x", "palette")
    app.key("C-g")
    run(app, refs, "palette -> home (C-g)", "C-g", "home")
    app.key("C-x C-f")
    run(app, refs, "home -> picker (C-x C-f)", "C-x C-f", "picker")
    app.key("C-g")
    run(app, refs, "picker -> home (C-g)", "C-g", "home")
    # home -> file
    app.key("C-c p t"); app.key("down"); app.key("down"); app.key("RET")
    app.key("C-c p t")
    run(app, refs, "home -> file", "C-c p t,down,down,RET", "file")
    # file -> search
    app.key("C-c p s s")
    for ch in "target":
        app.key(ch, settle=0.15)
    app.key("RET"); app.wait_done()
    run(app, refs, "file -> search", "C-c p s s,target,RET", "search")
    # search -> file
    app.key("q")
    run(app, refs, "search -> file (q)", "q", "file")
    # file -> magit
    app.key("C-x g")
    run(app, refs, "file -> magit", "C-x g", "magit")

    # Tree layout shift (C-c p t): reference-diff against the tree-ON and
    # tree-OFF magit references (stale/mismatch method, same as every other
    # transition — not a presence check).
    app.key("C-c p t")
    run(app, refs, "magit tree OFF -> tree ON", "C-c p t", "magit_tree")
    app.key("C-c p t")
    run(app, refs, "tree ON -> magit tree OFF", "C-c p t", "magit")
    app.kill()


def run(app, refs, name, keys, target):
    stale, mismatch = diff(refs[target], stable_capture(app))
    ok = not stale and not mismatch
    parts = []
    if stale:
        parts.append("stale " + "; ".join(f"r{i}={t!r}" for i, t in stale))
    if mismatch:
        parts.append("mismatch " + "; ".join(f"r{i}:{a!r}!={b!r}" for i, a, b in mismatch))
    record(name, keys, ok, "; ".join(parts) or "clean")


if __name__ == "__main__":
    home_boot()
    print("=== capturing clean references per view ===")
    R = refs()
    print("=== OVERPRINT MATRIX (post-transition frame vs clean reference) ===")
    transitions(R)
    print("\n=== SUMMARY ===")
    for name, keys, ok, detail in RESULTS:
        print(f"{'PASS' if ok else 'FAIL'}  {name}")
    bad = [n for n, _, ok, _ in RESULTS if not ok]
    print(f"\nTOTAL: {len(RESULTS)-len(bad)}/{len(RESULTS)} transitions clean")
    sys.exit(1 if bad else 0)

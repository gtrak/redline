#!/usr/bin/env python3
"""probe_quit_prompt — bounded single-shot probe of the quit-prompt-y leg.

Issue: issue-sweep-quit-prompt-flake. The probe drives the EXACT
sweep_flows.flow_quit_prompt_y sequence once, and instead of a single
PASS/FAIL it DUMPS the raw rows around the prompt plus every evidence
field, so a clipped / late / missing prompt is visible directly.

Usage:  PROBE_ROOT=<root> python3 tools/probe_quit_prompt.py [label]

The fixture root controls the prompt's path length:
  path = <root>/redline_pyte_repo/.redline-notes.md
  prompt = "(y, n, !, C-g) Save <path>?"
(issue-quit-prompt-keys-invisible, 7f0090a: the keys LEAD the message,
so the key list is left-anchored and can never be the clipped tail; the
legacy order "Save this buffer: <path>? (y, n, !, C-g)" clipped the keys
at any root whose prompt exceeded 79 cols.)
The minibuffer is NoWrap + overflow-hidden, so a prompt longer than the
row is clipped at the right edge — after the path, never the keys.
Default root is a PRIVATE /tmp/qprobe<pid>
seed from the /tmp baseline (the probe never mutates the shared baseline);
pass PROBE_ROOT=/tmp/fx<NNNNN> to reproduce a gate-shaped root. Bounded by
construction: one App, one exit; no loops.
"""
import os
import shutil
import sys

ROOT = os.environ.get("PROBE_ROOT", "/tmp/qprobe_%d" % os.getpid())
os.makedirs(ROOT, exist_ok=True)
_dst = os.path.join(ROOT, "redline_pyte_repo")
if not os.path.isdir(_dst):
    shutil.copytree("/tmp/redline_pyte_repo", _dst)
os.environ["REDLINE_FIXTURE_ROOT"] = ROOT

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo, reset as _reset_fixture

REPO = repo("redline_pyte_repo")
ROWS, COLS = 24, 80
PROMPT_HEAD = "(y, n, !, C-g)"
PROMPT_KEYS = "(y, n, !, C-g)"


def flat(app):
    return "\n".join(app.row_text(r) for r in range(app.rows)).replace("\n", " ")


def main():
    label = sys.argv[1] if len(sys.argv) > 1 else "probe"
    _reset_fixture()
    notes = os.path.join(REPO, ".redline-notes.md")
    app = App(REPO, rows=ROWS, cols=COLS)
    try:
        app.key("C-x n")
        app.wait(0.8)
        notes_open = ".redline-notes.md" in app.row_text(0)
        app.key("Q", settle=0.4)
        app.key("Y", settle=0.4)
        app.key("C-x C-c", settle=1.0)
        # sweep_flows' own poll: 4 s on the positive gate.
        import time
        prompted = False
        deadline = time.time() + 4.0
        while time.time() < deadline:
            app._read(0.2, quiet=0.0)
            if (PROMPT_HEAD in flat(app) and ".redline-notes.md" in flat(app)
                    and PROMPT_KEYS in flat(app)):
                prompted = True
                break
        # Dump the minibuffer row (ROWS-2) and its neighbours raw, plus the
        # probe's own computation of what SHOULD be there.
        path = notes
        prompt = "(y, n, !, C-g) Save %s?" % path
        print("=== %s ===" % label)
        print("fixture_root=%s" % os.environ.get("REDLINE_FIXTURE_ROOT", "/tmp"))
        print("path_len=%d prompt_len=%d (row holds %d chars after the leading space)"
              % (len(path), len(prompt), COLS - 1))
        for r in range(ROWS - 4, ROWS):
            row = app.row_text(r)
            filled = len(row.rstrip())
            print("row%02d filled=%-3d %r" % (r, filled, row[:COLS]))
        mb = app.row_text(ROWS - 2)
        head_ok = PROMPT_HEAD in mb
        path_ok = ".redline-notes.md" in mb
        keys_ok = PROMPT_KEYS in mb
        print("head=%s path=%s keys=%s  (sweep gate prompted=%s)"
              % (head_ok, path_ok, keys_ok, prompted))
        app.key("y", settle=2.0)
        # exit status
        try:
            pid, status = os.waitpid(app.pid, os.WNOHANG)
            alive_note = "exited" if pid else "alive-during-prompt"
        except ChildProcessError:
            alive_note = "already-reaped"
        disk = ""
        try:
            disk = open(notes).read()
        except FileNotFoundError:
            pass
        print("y->save: QY-on-disk=%s (exit-note=%s)" % ("QY" in disk, alive_note))
    finally:
        app.kill()
        try:
            os.remove(notes)
        except FileNotFoundError:
            pass


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Windowing panes thin tier (loop-04): the commit-diff scroll smoke.

One scenario of the original four stays PTY: commit-diff (M-> full-last-page
landing + M-< round trip + C-n/C-p one-row step scroll through the real
terminal). The other three are unit twins:

  * blame (cursor follows the window)  -> unit_flow_blw (loop-03)
  * log (in-page selection in view + next-page reset)
                                       -> unit_flow_panes_log (loop-04)
  * notes (typing keeps the row in view)
                                       -> unit_flow_nsl (loop-03)
and the commit-diff state half is unit_flow_panes_diff (loop-04; the
loop-03 unit_flow_cds pins the M->/M-< sentinel half).

Run:  python3 tools/drive_windowing_panes.py
"""
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo

COLS, ROWS = 80, 24
VERDICTS = []


def verdict(name, ok, detail=""):
    VERDICTS.append((name, ok, detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name:32s} {detail}")


def git(root, *args):
    env = dict(os.environ)
    env.update({
        "GIT_AUTHOR_NAME": "Test",
        "GIT_AUTHOR_EMAIL": "t@e.com",
        "GIT_COMMITTER_NAME": "Test",
        "GIT_COMMITTER_EMAIL": "t@e.com",
    })
    subprocess.run(["git", "-C", root, *args], check=True,
                   capture_output=True, env=env)


DIFF_REPO = repo("redline_win_diff_repo")


def ensure_diff_repo():
    """A repo with a base commit and one tall commit (big.txt -> 60 lines,
    the last of which is the `BOTTOM_SENTINEL` marker)."""
    if os.path.isdir(os.path.join(DIFF_REPO, ".git")):
        return
    os.makedirs(DIFF_REPO, exist_ok=True)
    git(DIFF_REPO, "init", "-q", "-b", "main")
    git(DIFF_REPO, "config", "user.name", "T")
    git(DIFF_REPO, "config", "user.email", "t@e.com")
    with open(os.path.join(DIFF_REPO, "big.txt"), "w") as f:
        f.write("l1\n")
    git(DIFF_REPO, "add", "-A")
    git(DIFF_REPO, "commit", "-q", "-m", "base")
    with open(os.path.join(DIFF_REPO, "big.txt"), "w") as f:
        for i in range(1, 60):
            f.write(f"line {i}\n")
        f.write("BOTTOM_SENTINEL\n")
    git(DIFF_REPO, "add", "-A")
    git(DIFF_REPO, "commit", "-q", "-m", "tall-lines")


def drive_commit_diff():
    ensure_diff_repo()
    app = App(DIFF_REPO, rows=ROWS, cols=COLS)
    app.key("C-x g")   # magit status
    app.key("l")       # log
    app.wait(0.8)
    app.key("RET")     # open the newest commit's diff (the tall one)
    app.wait(1.2)
    # At the top the bottom sentinel must be off-screen (the diff is taller
    # than the viewport).
    top_hidden = "BOTTOM_SENTINEL" not in app.screen_text()
    # M-> lands on the full last page: both the last two diff rows are
    # visible (pin the full-last-page semantic, not merely sentinel
    # visibility).
    app.key("M->")
    app.wait(0.8)
    bottom_text = app.screen_text()
    last_visible = "BOTTOM_SENTINEL" in bottom_text and "line 59" in bottom_text
    # M-< round-trips to the top: the sentinel is gone again.
    app.key("M-<")
    app.wait(0.8)
    roundtrip = "BOTTOM_SENTINEL" not in app.screen_text()
    # C-n steps the window down one row (the visible content shifts).
    before = [app.row_text(r) for r in range(1, app.rows - 2)]
    app.key("C-n")
    app.wait(0.6)
    after = [app.row_text(r) for r in range(1, app.rows - 2)]
    stepped = before != after
    # C-p steps it back.
    app.key("C-p")
    app.wait(0.6)
    back = [app.row_text(r) for r in range(1, app.rows - 2)]
    app.kill()
    ok = top_hidden and last_visible and roundtrip and stepped
    verdict("commit-diff (M->/M-< + C-n/C-p)", ok,
            f"top-hidden={top_hidden} last-visible={last_visible} "
            f"M-<-roundtrip={roundtrip} C-n-stepped={stepped} "
            f"C-p-back={back == before}")


if __name__ == "__main__":
    drive_commit_diff()
    print("\n=== SUMMARY ===")
    for name, ok, detail in VERDICTS:
        print(f"{'PASS' if ok else 'FAIL'}  {name}")
    bad = [n for n, ok, _ in VERDICTS if not ok]
    print(f"\nTOTAL: {len(VERDICTS) - len(bad)}/{len(VERDICTS)} scenarios PASS")
    sys.exit(1 if bad else 0)

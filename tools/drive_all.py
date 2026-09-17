#!/usr/bin/env python3
"""One-command re-run of every issue-002-02 cursor-visibility + windowing gate.

Runs each scenario in a fresh sized PTY and prints a PASS/FAIL table. The
discriminating assertion everywhere: at every navigation step exactly ONE
content row carries the blue selected-row signature (bg `48;5;12` -> pyte
`5c5cff`, white fg, NO reverse) and it is the row the store's cursor points
at. The active-buffer status line (bottom row) is excluded from the count.
"""
import sys
sys.path.insert(0, "/home/gary/dev/red/tools")
from pyte_driver import App

VERDICTS = []


def verdict(name, ok, detail=""):
    VERDICTS.append((name, ok, detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name:28s} {detail}")


def one_blue(app, expect_one=True):
    return (len(app.blue_rows()) == 1) and (len(app.reverse_rows()) == 0)


def drive_magit():
    app = App("/tmp/redline_pyte_repo", rows=24, cols=80)
    app.key("C-x g")
    ok = one_blue(app)
    steps = [one_blue(app) for _ in range(8)]
    for _ in range(8):
        app.key("n")
        steps.append(one_blue(app))
    for _ in range(3):
        app.key("p")
        steps.append(one_blue(app))
    app.kill()
    verdict("magit-status (n/p x12)", all(steps) and ok,
            f"{sum(steps)}/{len(steps)} steps exactly-one-blue, no reverse")


def drive_log():
    app = App("/tmp/redline_pyte_repo", rows=24, cols=80)
    app.key("C-x g"); app.key("l")
    steps = [one_blue(app)]
    subjects = ["commit 4", "commit 3", "commit 2", "commit 1", "init"]
    match_all = True
    for i in range(5):
        app.key("down")
        b = app.blue_rows()
        steps.append(one_blue(app))
        if len(b) == 1:
            match_all = match_all and (subjects[i] in app.row_text(b[0]))
    for _ in range(3):
        app.key("up"); steps.append(one_blue(app))
    app.kill()
    verdict("log / MagitRowsView (arrows)", all(steps) and match_all,
            f"{sum(steps)}/{len(steps)} steps one-blue, commit-subject matches={match_all}")


def drive_search():
    app = App("/tmp/redline_pyte_repo", rows=24, cols=80)
    app.key("C-c p s s")
    for ch in "target":
        app.key(ch, settle=0.3)
    app.key("RET")
    app.wait_done()
    steps = [one_blue(app)]
    for _ in range(8):
        app.key("n"); steps.append(one_blue(app))
    for _ in range(3):
        app.key("p"); steps.append(one_blue(app))
    app.kill()
    verdict("search results (n/p x12)", all(steps),
            f"{sum(steps)}/{len(steps)} steps one-blue, no reverse")


def drive_tree():
    app = App("/tmp/redline_pyte_repo", rows=24, cols=80)
    app.key("C-c p t")
    steps = [one_blue(app)]
    for _ in range(3):
        app.key("down"); steps.append(one_blue(app))
    for _ in range(2):
        app.key("up"); steps.append(one_blue(app))
    app.kill()
    verdict("tree sidebar (arrows)", all(steps),
            f"{sum(steps)}/{len(steps)} steps one-blue, no reverse")


def drive_buffer_list():
    app = App("/tmp/redline_pyte_repo", rows=24, cols=80)
    app.key("C-c p t"); app.key("down"); app.key("RET")   # open src/lib.rs
    app.key("C-c p t")                                   # tree off
    app.key("C-x C-b")                                   # buffer list
    steps = [one_blue(app)]
    for _ in range(3):
        app.key("down"); steps.append(one_blue(app))
    app.key("up"); steps.append(one_blue(app))
    app.kill()
    verdict("buffer list (arrows)", all(steps),
            f"{sum(steps)}/{len(steps)} steps one-blue, no reverse")


def drive_windowing():
    app = App("/tmp/redline_tall_repo", rows=24, cols=80)
    app.key("C-x g")
    HELP = "s stage"
    top, bot = 1, app.rows - 3
    in_win = []
    help_ok = []
    scrolled = 0
    prev = app.row_text(1).strip()
    for _ in range(28):
        b = app.blue_rows()
        in_win.append(len(b) == 1 and top <= b[0] <= bot)
        help_ok.append(HELP in app.screen_text())
        first = app.row_text(1).strip()
        if first != prev:
            scrolled += 1
            prev = first
        app.key("n")
    app.kill()
    ok = all(in_win) and all(help_ok) and scrolled > 0
    verdict("magit windowing (n x28)", ok,
            f"cursor in window {sum(in_win)}/28, help visible {sum(help_ok)}/28, window scrolled {scrolled}x")


if __name__ == "__main__":
    drive_magit()
    drive_log()
    drive_search()
    drive_tree()
    drive_buffer_list()
    drive_windowing()
    print("\n=== SUMMARY ===")
    for name, ok, detail in VERDICTS:
        print(f"{'PASS' if ok else 'FAIL'}  {name}")
    bad = [n for n, ok, _ in VERDICTS if not ok]
    print(f"\nTOTAL: {len(VERDICTS)-len(bad)}/{len(VERDICTS)} scenarios PASS")
    sys.exit(1 if bad else 0)

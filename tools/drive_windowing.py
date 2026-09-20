#!/usr/bin/env python3
"""Windowing thin tier (loop-04): one end-to-end magit cursor-follow smoke.

The 28-step `n`-follow state is now the unit twin
`unit_flow_win_magit_follow` in src/app/flow_tests.rs (27 n-steps, the
cursor in view on every step, the scroll, the pinned help line, at store
level + the width-80 render). What stays PTY-only here is the terminal
tier: input encoding through the real terminal, the blue cursor row at the
attribute level (exactly-one-blue), and the real on-screen help line.

Fixture: /tmp/redline_tall_repo (30 changed files — the status buffer is
taller than the window, so the follow must scroll).
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo

app = App(repo("redline_tall_repo"), rows=24, cols=80)
app.key("C-x g")      # magit status on the tall repo

HELP = "s stage"
CONTENT_TOP = 1        # row 0 is the title
CONTENT_BOT = app.rows - 3   # rows 22,23 are mode + status lines


def first_content_row():
    for r in range(CONTENT_TOP, CONTENT_BOT):
        t = app.row_text(r)
        if t.strip():
            return r, t.strip()
    return None, ""


def check(tag):
    blues = app.blue_rows()
    help_ok = HELP in app.screen_text()
    in_window = len(blues) == 1 and CONTENT_TOP <= blues[0] <= CONTENT_BOT
    ok = help_ok and in_window
    print(f"   {'OK' if ok else 'FAIL'} {tag}: blue={blues} help={help_ok} in_window={in_window}")
    return ok


print("=== WINDOWING THIN TIER (21 steps of the 28-step state twin) ===")
results = []
_, prev_first = first_content_row()
results.append(check("initial"))
scrolled = 0
for i in range(20):
    app.key("n")
    results.append(check(f"n x{i+1}"))
    _, first = first_content_row()
    if first != prev_first:
        scrolled += 1
        prev_first = first

ok_all = all(results) and scrolled > 0
print(f"\nwindow scrolled on {scrolled}/20 steps (top row advanced)")
print(f"RESULT: {'PASS' if ok_all else 'FAIL'} ({sum(results)}/{len(results)} steps OK, "
      f"scrolled={scrolled > 0})")
app.kill()
sys.exit(0 if ok_all else 1)

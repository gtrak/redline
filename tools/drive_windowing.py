#!/usr/bin/env python3
"""Windowing gate: a tall magit status buffer (30 changed files) must keep the
cursor in view while pressing `n` past the pane bottom (the window scrolls) and
keep the help line visible. The cursor bar must never clip out of the window.
"""
import sys
sys.path.insert(0, "/home/gary/dev/red/tools")
from pyte_driver import App

app = App("/tmp/redline_tall_repo", rows=24, cols=80)
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


def check(tag, state):
    blues, help_ok, in_window, first = state
    ok = (len(blues) == 1) and help_ok and in_window
    b = blues[0] if blues else "?"
    print(f"   {'OK' if ok else 'FAIL'} {tag}: blue={blues} help={help_ok} in_window={in_window} cursor_row={b} top={first!r}")
    return ok


print("=== WINDOWING (30 changed files, viewport ~21, window ~19) ===")
results = []
prev_first = None
scrolled = 0
# initial
blues = app.blue_rows()
help_ok = HELP in app.screen_text()
first_idx, first = first_content_row()
in_window = len(blues) == 1 and CONTENT_TOP <= blues[0] <= CONTENT_BOT
results.append(check("initial", (blues, help_ok, in_window, first)))
prev_first = first

for i in range(1, 28):
    app.key("n")
    blues = app.blue_rows()
    help_ok = HELP in app.screen_text()
    first_idx, first = first_content_row()
    in_window = len(blues) == 1 and CONTENT_TOP <= blues[0] <= CONTENT_BOT
    if first != prev_first:
        scrolled += 1
        prev_first = first
    results.append(check(f"n x{i}", (blues, help_ok, in_window, first)))

ok_all = all(results)
print(f"\nwindow scrolled on {scrolled}/{len(results)-1} steps (top row advanced)")
print(f"RESULT: {'PASS' if ok_all else 'FAIL'} ({sum(results)}/{len(results)} steps OK)")
app.kill()
sys.exit(0 if ok_all else 1)

#!/usr/bin/env python3
"""Drive the magit status view cursor and pin its blue-row trajectory.

`C-x g` opens the magit status view. The cursor selects exactly ONE row at a
time (a full-bar highlight); `n`/`p` walk the cursor down/up through the
sections (the modified files, the unstaged/untracked headers, the files).

This was a bare capture (report-only, always green — a silent hole: its
failures were invisible to the battery). It now gates: exactly one full-bar
blue row at every step AND the observed trajectory, so a magit-status cursor
regression is a red test (exit 1), not a green print.
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo

app = App(repo("redline_pyte_repo"), rows=24, cols=80)
app.key("C-x g")          # open magit status

# Every check's failure flag; drives a non-zero exit (see end).
BAD = []


def check(tag, cond, detail=""):
    print(f"   {'OK' if cond else 'FAIL'} {tag}" + (f"  {detail}" if detail else ""))
    BAD.append(not cond)
    return cond


def snapshot(tag):
    blues = app.blue_rows()
    rev = app.reverse_rows()
    print(f"\n--- {tag} ---")
    print("blue rows:", blues)
    print("reverse rows:", rev)
    if blues:
        for r in blues:
            print(f"  row {r} blue_count={app.blue_count(r)}: {app.row_text(r)!r}")
    # The cursor must select EXACTLY ONE row, and it must be a full-bar
    # highlight (the whole selected row, not a partial/inline mark).
    one = len(blues) == 1
    full = one and app.blue_count(blues[0]) == app.cols
    detail = f"n={len(blues)}" + (f" blue_count={app.blue_count(blues[0])}" if one else "")
    check(f"{tag}: exactly one full-bar blue row", one and full, detail)
    return blues


# The observed, fixture-deterministic trajectory (the baseline magit status
# view): the first modified file is the default selection, then n/p walk the
# cursor one row at a time through the sections and their entries.
EXPECTED_INITIAL = 3
EXPECTED_DOWN = [4, 5, 6, 7, 8, 9, 10, 11]
EXPECTED_UP = [10, 9, 8]


def row_of(blues):
    return blues[0] if len(blues) == 1 else None


initial = snapshot("magit initial")
check(f"initial selection is row {EXPECTED_INITIAL}",
      row_of(initial) == EXPECTED_INITIAL, f"got {row_of(initial)}")

# Walk down with n
seq = []
for i in range(len(EXPECTED_DOWN)):
    app.key("n")
    b = snapshot(f"n x{i+1}")
    seq.append(row_of(b))
check("n-trajectory matches", seq == EXPECTED_DOWN, f"got {seq}")

# Walk back up with p
up = []
for i in range(len(EXPECTED_UP)):
    app.key("p")
    b = snapshot(f"p x{i+1}")
    up.append(row_of(b))
check("p-trajectory matches", up == EXPECTED_UP, f"got {up}")

print("\n=== TRAJECTORY (row of the blue bar) ===")
print("initial+down:", [EXPECTED_INITIAL] + seq)
app.kill()
print(f"{len(BAD) - sum(BAD)}/{len(BAD)} checks passed")
sys.exit(1 if any(BAD) else 0)

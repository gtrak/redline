#!/usr/bin/env python3
"""Drive the magit status view cursor and report the blue-row trajectory."""
import sys
sys.path.insert(0, "/home/gary/dev/red/tools")
from pyte_driver import App

app = App("/tmp/redline_pyte_repo", rows=24, cols=80)
app.key("C-x g")          # open magit status

def snapshot(tag):
    blues = app.blue_rows()
    rev = app.reverse_rows()
    print(f"\n--- {tag} ---")
    print("blue rows:", blues)
    print("reverse rows:", rev)
    if blues:
        for r in blues:
            print(f"  row {r} blue_count={app.blue_count(r)}: {app.row_text(r)!r}")
    return blues

snapshot("magit initial")

# Walk down with n: 8 steps
seq = []
for i in range(8):
    app.key("n")
    b = snapshot(f"n x{i+1}")
    seq.append(b[0] if b else None)

# Walk back up with p
for i in range(3):
    app.key("p")
    b = snapshot(f"p x{i+1}")

print("\n=== TRAJECTORY (row of the blue bar) ===")
print("initial+down:", seq)
app.kill()

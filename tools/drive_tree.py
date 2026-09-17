#!/usr/bin/env python3
"""Drive the TREE sidebar cursor (tree.rs, list_item_selected, no invert).

`C-c p t` toggles the ignore-aware file-tree sidebar; while it is visible and
the top view is the buffer, arrows move the tree cursor. At every step exactly
ONE content row carries the blue signature (in the tree column) and it is the
file the store's `tree.selected` points at.
"""
import sys
sys.path.insert(0, "/home/gary/dev/red/tools")
from pyte_driver import App

app = App("/tmp/redline_pyte_repo", rows=24, cols=80)
app.key("C-c p t")      # toggle the tree sidebar on


def snap(tag):
    blues = app.blue_rows()
    rev = app.reverse_rows()
    line = f"{tag}: blue={blues} reverse={rev}"
    for r in blues:
        line += f"\n        row{r}={app.row_text(r)!r}"
    print(line)
    return blues


def check(tag, blues):
    ok = len(blues) == 1
    print(f"   {'OK' if ok else 'FAIL'} {tag}: exactly-one={len(blues)==1} (n={len(blues)})")
    return ok


print("=== TREE sidebar (files: README.md, src/lib.rs, src/main.rs) ===")
# show the layout
for r in range(app.rows):
    t = app.row_text(r)
    if t.strip():
        print(f"{r:02d} | {t}  [BLUE x{app.blue_count(r)}]" if app.blue_count(r) else f"{r:02d} | {t}")
print()

b = snap("initial (tree.selected=0)")
check("initial", b)
for i in range(1, 4):
    app.key("down")
    b = snap(f"down x{i}")
    check(f"down x{i}", b)
for i in range(2):
    app.key("up")
    b = snap(f"up x{i+1}")
    check(f"up x{i+1}", b)
app.kill()

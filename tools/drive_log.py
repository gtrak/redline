#!/usr/bin/env python3
"""Drive the LOG view cursor (MagitRowsView via log_view.rs).

From magit status, `l` opens the log. In-page selection moves with the
arrows (log.selected); `n`/`p` page (reset selection to 0). At every in-page
step exactly ONE content row must carry the blue selected-row signature and
it must be the commit the store's cursor (`log.selected`) points at.
"""
import sys
sys.path.insert(0, "/home/gary/dev/red/tools")
from pyte_driver import App

app = App("/tmp/redline_pyte_repo", rows=24, cols=80)
app.key("C-x g")     # magit status
app.key("l")         # magit-log -> Log view (MagitRowsView)


def snap(tag):
    blues = app.blue_rows()
    rev = app.reverse_rows()
    line = f"{tag}: blue={blues} reverse={rev}"
    if len(blues) == 1:
        r = blues[0]
        line += f"  text={app.row_text(r)!r}"
    else:
        for r in blues:
            line += f"\n        row{r}={app.row_text(r)!r}"
    print(line)
    return blues


def check(tag, blues, expect_one):
    ok = (len(blues) == 1) if expect_one else (len(blues) == 0)
    print(f"   {'OK' if ok else 'FAIL'} {tag}: exactly-one={len(blues)==1} (n={len(blues)})")
    return ok


print("=== LOG view (6 commits: commit 5..1, init) ===")
b = snap("initial (log.selected=0 -> commit 5)")
check("initial", b, expect_one=True)

# move down through the commits
expected_subjects = ["commit 5", "commit 4", "commit 3", "commit 2", "commit 1", "init"]
for i in range(1, 6):
    app.key("down")
    b = snap(f"down x{i}")
    check(f"down x{i}", b, expect_one=True)
    if len(b) == 1:
        txt = app.row_text(b[0])
        subj = expected_subjects[i] if i < len(expected_subjects) else "?"
        match = subj in txt
        print(f"        cursor-on-{subj!r}={match}")

# move back up
for i in range(3):
    app.key("up")
    b = snap(f"up x{i+1}")
    check(f"up x{i+1}", b, expect_one=True)

app.kill()

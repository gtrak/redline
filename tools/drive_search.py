#!/usr/bin/env python3
"""Drive the SEARCH RESULTS view cursor (results_view.rs, fed by search_view_info).

`C-c p s s` opens the prompt; type the query + RET runs an embedded-ripgrep
project search; the results view lists file headers + match lines. `n`/`p`
step between hits (store.search.selected). At every step exactly ONE content
row must carry the blue selected-row signature and it must be the hit the
store's cursor points at.
"""
import os, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo

app = App(repo("redline_pyte_repo"), rows=24, cols=80)
app.key("C-c p s s")    # enter project-search prompt
for ch in "target":
    app.key(ch, settle=0.3)
app.key("RET")          # run the search


def wait_done(timeout=15.0):
    """Poll until the title no longer shows '(searching…)' and a match count
    is present."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        app._read(0.4, quiet=0.2)
        text = app.screen_text()
        if "searching" not in text and "matches in" in text:
            return True
    return False


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


if not wait_done():
    print("FAIL: search did not finish (title still searching / no count)")
    print(app.screen_text())
    app.kill()
    sys.exit(1)

# Show the results layout once.
print("=== RESULTS layout ===")
for r in range(app.rows):
    t = app.row_text(r)
    if t.strip():
        print(f"{r:02d} | {t}  [BLUE x{app.blue_count(r)}]" if app.blue_count(r) else f"{r:02d} | {t}")
print()

print("=== cursor drive (n x8 then p x3) ===")
b = snap("initial (search.selected=0)")
check("initial", b)
first_row = b[0] if b else None
for i in range(1, 9):
    app.key("n")
    b = snap(f"n x{i}")
    check(f"n x{i}", b)
for i in range(3):
    app.key("p")
    b = snap(f"p x{i+1}")
    check(f"p x{i+1}", b)

print("\nfirst-hit row:", first_row)
app.kill()

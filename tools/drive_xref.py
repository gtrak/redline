#!/usr/bin/env python3
"""M-. xref thin tier (loop-04): the same-file jump smoke.

One leg of the original six stays PTY: L1 (M-. at the END of the `target_one`
call jumps to the SAME-FILE definition — the user's core complaint — lands
on it, and M-, returns to the call site). The state halves of the rest are
unit twins (src/app/flow_tests.rs, loop-04):

  * L2 cross-file jump          -> unit_flow_xref_l2
  * L3 bare-miss report         -> unit_flow_xref_l3
  * L4 raw `::` path token      -> unit_flow_xref_l4
  * L5 external landing guard   -> unit_flow_xref_l5 (the path-dep's cargo
                                   metadata resolution itself is the
                                   resolver corpus's)
  * L6 middle-landing recenter  -> unit_flow_xref_l6

What stays PTY-only here is the live terminal: the real M-. input through
the terminal encoder and the live landing/repaint.

Fixture: /tmp/redline_pyte_repo (a git repo WITHOUT a Cargo.toml — the
fixture baseline is explicitly cargo-less; the leg files are removed after).
"""
import os
import shutil
import sys
import time

sys.path.insert(0, "/home/gary/dev/red/tools")
from fixture import reset
from pyte_driver import App

REPO = "/tmp/redline_pyte_repo"
LEG_RS = os.path.join(REPO, "src", "leg.rs")
MINI = 22  # minibuffer row (content rows 0..20 in a 24-row PTY, 1-based
           # terminal row 23 is the status line)

results = []


def rec(tag, ok, detail=""):
    results.append(ok)
    print(f"{'OK  ' if ok else 'FAIL'} {tag}" + (f"  [{detail}]" if detail and not ok else ""))


def poll(app, needle, timeout=25.0):
    """Wait until `needle` appears in the minibuffer row."""
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        app.wait(0.4)
        last = app.row_text(MINI)
        if needle in last:
            return True, last
    return False, last


def open_file(app, rel):
    app.key("C-x C-f", 1.0)
    for ch in rel:
        app.key(ch, 0.15)
    app.key("RET", 1.0)


def goto(app, n):
    app.key("M-g g", 0.8)
    app.key(str(n), 0.4)
    app.key("RET", 1.0)


def top_content(app):
    return app.row_text(1)  # terminal row 1 = content row 0


reset()
print("=== M-. same-file jump (thin tier) ===")
os.makedirs(os.path.join(REPO, "src"), exist_ok=True)
with open(LEG_RS, "w") as f:
    f.write("fn leg() {\n    target_lib();\n}\ntokio::spawn(f);\n")

app = App(REPO)
app.wait_ready()
try:
    open_file(app, "src/main.rs")
    goto(app, 4)  # "target_one();"
    app.key("M-f", 0.8)  # point to the END of the `target_one` run
    app.key("M-.", 1.5)
    ok, msg = poll(app, "jumped to src/main.rs", timeout=5.0)
    rec("L1 same-file: message", ok, f"minibuffer={msg!r}")
    ok = "fn target_one() {}" in top_content(app)
    rec("L1 same-file: window landed on the definition", ok,
        f"top={top_content(app)!r}")
    # Jump-back returns to the call site (the landing recorded a jump).
    app.key("M-,", 1.0)
    app.wait(0.5)
    back = "target_one();" in app.screen_text()
    rec("L1 same-file: M-, back to the call site", back,
        f"screen has call site={back}")
finally:
    app.kill()
    for stray in (LEG_RS,):
        try:
            os.remove(stray)
        except FileNotFoundError:
            pass

print(f"\n{sum(results)}/{len(results)} legs passed")
sys.exit(0 if all(results) else 1)

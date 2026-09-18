#!/usr/bin/env python3
"""Drive M-. (xref-find-definitions) — cursor-aware selection + resolver
fall-through (plan 006 issue 02) against the shared fixture.

Legs (fixture /tmp/redline_pyte_repo — a git repo WITHOUT a Cargo.toml, so
the cargo provider fails fast and offline):
  L1 same-file:   cursor at the END of the `target_one` call → jump to the
                  SAME-FILE definition (struct+impl-style case; the old
                  cross-file filter made this unjumpable).
  L2 cross-file:  cursor at the end of `target_lib` in a fresh src/leg.rs →
                  jump to src/lib.rs.
  L3 bare miss:   cursor at the end of `other` in `let other = 9;` — not in
                  the index (it is a let binding), no enclosing symbol →
                  resolver fall-through → a clear
                  "no provider resolution for `other`" message.
  L4 path token:  cursor at the end of `tokio` in `tokio::spawn(f);` — the
                  miss report names the PATH token `tokio::spawn` (the raw
                  token under the point is what the resolver gets).
"""
import os
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
print("=== M-. cursor-aware jump + resolver fall-through ===")
os.makedirs(os.path.join(REPO, "src"), exist_ok=True)
with open(LEG_RS, "w") as f:
    f.write("fn leg() {\n    target_lib();\n}\ntokio::spawn(f);\n")

app = App(REPO)
app.wait_ready()
try:
    # ── L1: same-file jump (the user's core complaint) ──────────────────
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

    # ── L2: cross-file jump ─────────────────────────────────────────────
    open_file(app, "src/leg.rs")
    goto(app, 2)  # "    target_lib();"
    app.key("M-f", 0.8)  # end of `target_lib` (skips `fn`... no: line 2 IS the call)
    app.key("M-.", 1.5)
    ok, msg = poll(app, "jumped to src/lib.rs", timeout=5.0)
    rec("L2 cross-file: message", ok, f"minibuffer={msg!r}")
    ok = "pub fn target_lib() {}" in top_content(app)
    rec("L2 cross-file: window landed on lib.rs definition", ok,
        f"top={top_content(app)!r}")

    # ── L3: bare (field-shaped) name → resolver fall-through ────────────
    open_file(app, "src/main.rs")
    goto(app, 6)  # "let other = 9;"
    app.key("M-f M-f", 0.8)  # end of `let`, then end of `other`
    app.key("M-.", 1.5)
    ok, msg = poll(app, "provider(s): rust", timeout=25.0)
    ok = ok and "`other`" in msg and "`tokio`" not in msg
    rec("L3 bare miss: graceful resolver report", ok, f"minibuffer={msg!r}")

    # ── L4: `::`-path token reaches the resolver raw ────────────────────
    open_file(app, "src/leg.rs")
    goto(app, 4)  # "tokio::spawn(f);"
    app.key("M-f", 0.8)  # end of `tokio`
    app.key("M-.", 1.5)
    ok, msg = poll(app, "provider(s): rust", timeout=25.0)
    ok = ok and "`tokio::spawn`" in msg
    rec("L4 path token: resolver gets the raw `tokio::spawn`", ok,
        f"minibuffer={msg!r}")
finally:
    app.kill()

print(f"\n{sum(results)}/{len(results)} legs passed")
sys.exit(0 if all(results) else 1)

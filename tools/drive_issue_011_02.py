#!/usr/bin/env python3
"""Plan 011 issue 02 — per-language scope hints, PYTHON leg.

In a dedicated repo (its own PTY-flock, keyed to the repo path), two
buffers exercise the new import walk end to end through M-.:

L1 (discriminating): a BARE imported symbol (`dumps` from
`from json import dumps`) at top level misses the project symbol index;
the python provider now receives the scope hint `["json", "dumps"]` and
MUST land in the stdlib `json` source on `def dumps` — pre-011-02 the
hint was empty for every non-Rust buffer and this probe bailed with
"needs scope info".

L2 (degradation pin): a prelude name (`print`, no import) gets NO hint —
the exact existing no-hint bail, byte-for-byte (only the python provider
is attempted, and it misses).

The drive owns /tmp/redline_011_02_py_repo and removes it on exit.
Wrap the invocation in `timeout` (shared PTY flock scheme: if the lock
is busy this exits 3 — wait and retry, never run two suites at once).

Exit 0 = legs pass; 1 = any failed.
"""
import os, sys, shutil, subprocess
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App

REPO = "/tmp/redline_011_02_py_repo"
COLS, ROWS = 200, 24
MINI = ROWS - 2      # minibuffer row (0-based)
STATUS = ROWS - 1    # status-line row (0-based)

CHECKS = []


def rec(name, ok, detail=""):
    CHECKS.append((name, ok, detail))
    print(f"  {'PASS' if ok else 'FAIL'}  {name:60s} {detail}")


def setup_repo():
    if os.path.exists(REPO):
        shutil.rmtree(REPO)
    os.makedirs(REPO)
    with open(os.path.join(REPO, "main.py"), "w") as f:
        # The probe call sits at TOP LEVEL (line 3): M-. selection falls
        # through to the tooling resolver only when the point has no
        # in-project definition AND no enclosing symbol (the 011-01
        # fixture does the same).
        f.write("from json import dumps\n\ndumps('x')\n")
    with open(os.path.join(REPO, "prelude.py"), "w") as f:
        f.write("print('x')\n")
    subprocess.run(["git", "init", "-q", REPO], check=True)
    subprocess.run(["git", "-C", REPO, "add", "-A"], check=True,
                   capture_output=True)
    subprocess.run(["git", "-C", REPO, "commit", "-qm", "011-02 python leg"],
                   check=True, capture_output=True)


def poll_minibuffer(app, needle, timeout=90.0):
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        app._read(0.3, quiet=0.2)
        last = app.row_text(MINI)
        if needle in last:
            return True, last
    return False, last


def poll_screen(app, needles, timeout=90.0):
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        app._read(0.3, quiet=0.2)
        last = app.screen_text()
        if any(n in last for n in needles):
            return True, last
    return False, last


def open_file(app, name):
    app.key("C-x C-f", 1.0)
    for ch in name:
        app.key(ch, 0.25)
    app.key("RET", 1.2)


def main():
    print(f"REPO={REPO}\n")
    app = None
    try:
        setup_repo()
        app = App(REPO, rows=ROWS, cols=COLS)

        print("=== L1: bare imported `dumps` lands in the json source ===")
        open_file(app, "main.py")
        app.key("M-g g", 0.8)
        app.key("3", 0.4)
        app.key("RET", 0.8)    # line 3: top-level `dumps('x')`
        app.key("M-f", 0.8)    # point to the END of the `dumps` run
        app.key("M-.", 0.5)
        ok, screen = poll_screen(app, ["def dumps"], timeout=120.0)
        rec("L1: M-. on the bare import lands in the json stdlib source",
            ok, f"screen has 'def dumps'={ok}")
        miss, msg = poll_minibuffer(app, "no provider resolution", timeout=3.0)
        rec("L1: no resolver miss message on landing",
            not miss, f"minibuffer={msg!r}")

        print("=== L2: prelude `print` (no import) bails byte-for-byte ===")
        open_file(app, "prelude.py")
        app.key("M-g g", 0.8)
        app.key("1", 0.4)
        app.key("RET", 0.8)    # line 1: top-level `print('x')`
        app.key("M-f", 0.8)    # point to the END of the `print` run
        app.key("M-.", 0.5)
        ok, msg = poll_minibuffer(app, "no provider resolution", timeout=90.0)
        rec("L2: prelude name is NOT guessed (the miss lands)",
            ok, f"minibuffer={msg!r}")
        rec("L2: the miss is the EXACT no-hint bail (python provider only)",
            ok and "tried 1 provider(s): python" in msg,
            f"minibuffer={msg!r}")
    finally:
        if app is not None:
            try:
                app.kill()
            except Exception:
                pass
        shutil.rmtree(REPO, ignore_errors=True)

    failed = [c for c in CHECKS if not c[1]]
    print(f"\n{len(CHECKS) - len(failed)}/{len(CHECKS)} legs passed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()

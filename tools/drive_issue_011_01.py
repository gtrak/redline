#!/usr/bin/env python3
"""Plan 011 issue 01 — language dispatch leg: M-. in a PYTHON project.

In a dedicated repo (NOT the shared fixture — it has no Python project),
the cursor sits at the end of `os` in the top-level `os.path.join('a',
'b')` probe. M-. misses the project symbol index; the chain carries the
buffer's language ("python") and MUST dispatch to PythonProvider ONLY —
with all four providers registered, a broken dispatch would probe cargo
first (and in a non-cargo project fail with "no Cargo.toml", shadowing
the real error).

011-06 SUPERSEDED THE MISS PIN: the pre-011-06 leg asserted the exact
bail ("no provider resolution … tried 1 provider(s): python") — the
`::`-only extraction fed the BARE `os`. Since 011-06 the M-. token IS
the dotted path `os.path.join`, so the python provider's dotted handling
LANDS (exact stdlib `posixpath.py:72`). The dispatch guarantee (python
probed, cargo never) is now carried by the resolver-crate unit pins
(`language_dispatch_only_matching_provider_attempted`,
`python_context_never_reaches_cargo_provider`) plus this leg's landing
itself: a broken dispatch probing cargo first would FAIL with "no
Cargo.toml" in this non-cargo project, never land. (The live "tried 1
provider(s): python" miss pin remains in drive_issue_011_02 L2's prelude
`print` leg.)

The drive owns its own repo dir (/tmp/redline_011_01_py_repo) under the
shared PTY-flock scheme (the lock is keyed to the repo path, so it never
contends with the other suites) and removes the repo on exit. Wrap the
invocation in `timeout`.

Exit 0 = legs pass; 1 = any failed.
"""
import os, sys, shutil, subprocess
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App

REPO = "/tmp/redline_011_01_py_repo"
COLS, ROWS = 200, 24
MINI = ROWS - 2      # minibuffer row (0-based)
STATUS = ROWS - 1    # status-line row (0-based)

CHECKS = []


def rec(name, ok, detail=""):
    CHECKS.append((name, ok, detail))
    print(f"  {'PASS' if ok else 'FAIL'}  {name:58s} {detail}")


def setup_repo():
    if os.path.exists(REPO):
        shutil.rmtree(REPO)
    os.makedirs(REPO)
    with open(os.path.join(REPO, "main.py"), "w") as f:
        # The probe call sits at TOP LEVEL (line 3): M-. selection falls
        # through to the tooling resolver only when the point has no
        # in-project definition AND no enclosing symbol (drive_xref's
        # fixture does the same with a bare top-level call).
        f.write("import os\n\nos.path.join('a', 'b')\n")
    subprocess.run(["git", "init", "-q", REPO], check=True)
    subprocess.run(["git", "-C", REPO, "add", "-A"], check=True,
                   capture_output=True)
    subprocess.run(["git", "-C", REPO, "commit", "-qm", "011-01 python leg"],
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


def main():
    print(f"REPO={REPO}\n")
    app = None
    try:
        setup_repo()
        app = App(REPO, rows=ROWS, cols=COLS)
        app.key("C-x C-f", 1.0)
        for ch in "main.py":
            app.key(ch, 0.25)
        app.key("RET", 1.2)
        app.key("M-g g", 0.8)
        app.key("3", 0.4)
        app.key("RET", 0.8)   # line 3: top-level `os.path.join('a', 'b')`
        app.key("M-f", 0.8)   # point to the END of the `os` run
        print("=== L1: M-. in a python buffer LANDS via python ONLY (011-06) ===")
        app.key("M-.", 0.5)
        ok, msg = poll_minibuffer(app, "jumped to", timeout=120.0)
        rec("L1: M-. on `os.path.join` LANDS (011-06 dotted token; not a bail)",
            ok and "no provider resolution" not in msg
            and "no background runtime" not in msg,
            f"minibuffer={msg!r}")
        rec("L1: the landing is EXACTLY the stdlib path.join (posixpath.py:72)",
            "posixpath.py:72" in msg, f"minibuffer={msg!r}")
        rec("L1: the cargo provider was never probed (no Cargo.toml confusion)",
            "cargo" not in msg.lower() and "Cargo.toml" not in msg,
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

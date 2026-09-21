#!/usr/bin/env python3
"""Plan 011 issue 04 — per-language source index, PYTHON leg.

The 011-02 python fixture (`from json import dumps`) is reused: L1 lands
M-. in the stdlib `json` source. 011-04's change is what happens NEXT:
the landing registers the source tree for background indexing, and the
walk now collects the OWNING language's extensions (python: py/pyi)
instead of the hardcoded `**/*.rs` (which finds 0 files under
/usr/lib/python3.14, so pre-011-04 the index is never built for this
root at all).

L2 (discriminating): the `indexing crate python3.14 …` indicator
APPEARS in the status line during the window between the landing and
the index arrival — it cannot appear pre-011-04 (the `.rs`-only walk
finds 0 files under /usr/lib/python3.14, so no build is ever armed for
this root). The stdlib root is a few hundred files, so the window is
short: the drive polls the status line tightly from the M-. keypress
onwards (the build is armed the moment the landing event is applied,
i.e. while L1's `def dumps` screen is still on its way).
L3 (discriminating): INSIDE the landed dependency, M-. on the top-level
bare `JSONDecoder` (defined in the sibling module `decoder.py` of the
same source root) jumps IN-CRATE through the freshly built index —
pre-011-04 the crate index is empty and this M-. bails to the resolver
("no provider resolution … tried 1 provider(s): python").

The drive owns /tmp/redline_011_04_py_repo under the shared PTY-flock
scheme (the lock is keyed to the repo path, so it never contends with
the other suites) and removes the repo on exit. Wrap the invocation in
`timeout`.

Exit 0 = legs pass; 1 = any failed.
"""
import os, sys, shutil, subprocess
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo

REPO = repo("redline_011_04_py_repo")
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
        # in-project definition AND no enclosing symbol (drive 011-02's
        # fixture does the same).
        f.write("from json import dumps\n\ndumps('x')\n")
    subprocess.run(["git", "init", "-q", REPO], check=True)
    subprocess.run(["git", "-C", REPO, "add", "-A"], check=True,
                   capture_output=True)
    subprocess.run(["git", "-C", REPO, "commit", "-qm", "011-04 python leg"],
                   check=True, capture_output=True)


def json_import_decoder_line():
    """The 1-based line of `from .decoder import JSONDecoder, …` in the
    installed stdlib json/__init__.py (version-dependent — never hard-
    code it)."""
    import json as _json
    path = os.path.join(os.path.dirname(_json.__file__), "__init__.py")
    with open(path) as f:
        for i, line in enumerate(f, 1):
            if line.startswith("from .decoder import JSONDecoder"):
                return i
    raise SystemExit("drive fixture stale: no "
                     "'from .decoder import JSONDecoder' line in json/__init__.py")


def poll_screen(app, needle, timeout=90.0):
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        app._read(0.3, quiet=0.2)
        last = app.screen_text()
        if needle in last:
            return True, last
    return False, last


def poll_minibuffer(app, needle, timeout=90.0):
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        app._read(0.3, quiet=0.2)
        last = app.row_text(MINI)
        if needle in last:
            return True, last
    return False, last


def open_file(app, name):
    app.key("C-x C-f", 1.0)
    for ch in name:
        app.key(ch, 0.25)
    app.key("RET", 1.2)


def main():
    print(f"REPO={REPO}\n")
    import_line = json_import_decoder_line()
    print(f"json/__init__.py decoder-import line: {import_line}\n")
    app = None
    try:
        setup_repo()
        app = App(REPO, rows=ROWS, cols=COLS)
        open_file(app, "main.py")

        print("=== L1/L2: landing + the python-tree index build ===")
        app.key("M-g g", 0.8)
        app.key("3", 0.4)
        app.key("RET", 0.8)    # line 3: top-level `dumps('x')`
        app.key("M-f", 0.8)    # point to the END of the `dumps` run
        app.key("M-.", 0.5)
        # Poll tightly from the keypress itself (as before the change):
        # the index build is armed the moment the landing event applies —
        # jump-ambiguity: that is the tooling row's PICKER open (never a
        # silent jump) — and the stdlib-root build is fast: the watch
        # must run from the keypress or the `indexing crate` window is
        # missed. Accept the top guess (RET) the moment the picker
        # appears, under the same watch.
        landed = False
        indicator_seen = False
        refused = False
        last_status = ""
        ret_pressed = False
        picker_ok = False
        marker_ok = False
        deadline = time.time() + 120.0
        while time.time() < deadline:
            app._read(0.15, quiet=0.1)
            last_status = app.row_text(STATUS)
            screen = app.screen_text()
            if "indexing crate" in last_status:
                indicator_seen = True
            if "crate too large" in last_status or \
                    "crate too large" in app.row_text(MINI):
                refused = True
            if not ret_pressed and "Definition:" in screen:
                picker_ok = True
                marker_ok = "tooling" in screen
                app.key("RET", 0.2)
                ret_pressed = True
            if not landed and "def dumps" in screen:
                landed = True
            if landed and "indexing crate" not in last_status:
                break
        rec("L1: the tooling hit joins the Xref picker (marked row)",
            picker_ok and marker_ok,
            f"prompt={picker_ok!r} marker={marker_ok!r}")
        rec("L1: M-. on the bare import lands in the json stdlib source",
            landed)
        rec("L1: the landing is a resolved-source jump",
            "jumped to" in app.row_text(MINI) and "json" in app.row_text(MINI),
            f"minibuffer={app.row_text(MINI)!r}")
        rec("L2: the `indexing crate` indicator appears (python tree indexed)",
            indicator_seen and not refused,
            f"seen={indicator_seen} last_status={last_status.strip()!r}")
        rec("L2: the stdlib root is UNDER the refusal cap (indexable)",
            not refused, f"minibuffer={app.row_text(MINI)!r}")
        rec("L2: the `indexing crate` indicator clears (index installed)",
            landed and "indexing crate" not in last_status,
            f"status={last_status.strip()!r}")

        print("=== L3: M-. AGAIN INSIDE the dependency jumps in-crate ===")
        # Line `<import_line>`: `from .decoder import JSONDecoder,
        # JSONDecodeError` — TOP LEVEL (no enclosing symbol), and
        # `JSONDecoder` is NOT defined in __init__.py. M-f x4 lands the
        # point at the END of the `JSONDecoder` run (from -> .decoder ->
        # import -> JSONDecoder). The in-crate jump needs the freshly
        # built index: pre-011-04 the crate index for this root is
        # empty, so this M-. bails to the resolver instead.
        app.key("M-g g", 0.8)
        for ch in str(import_line):
            app.key(ch, 0.3)
        app.key("RET", 0.8)
        for _ in range(4):
            app.key("M-f", 0.4)
        app.key("M-.", 0.5)
        # (jump-ambiguity) cross-file unique → the Xref picker (best
        # preselected); RET accepts the top guess.
        ok, prompt = poll_screen(app, "Definition:", timeout=90.0)
        app.key("RET", 1.0)
        ok, msg = poll_minibuffer(app, "jumped to json/decoder.py", timeout=30.0)
        rec("L3: M-. inside the dependency lands in decoder.py (in-crate index)",
            ok, f"minibuffer={msg!r}")
        rec("L3: NOT a resolver bail (the index answered)",
            not ("no provider resolution" in msg), f"minibuffer={msg!r}")
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

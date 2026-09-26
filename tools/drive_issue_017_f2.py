#!/usr/bin/env python3
"""Plan 017 finding F2 — the provider-miss detail reaches the minibuffer.

The fetch-confirmation gate (issue-non-rust-receiver-resolution) refuses an
install on a decline, but pre-F2 the chain-level bail
("no tooling provider could resolve symbol … (tried N provider(s): …)")
swallowed the provider's OWN reason — a refusal was indistinguishable from a
bare miss, inverting the gate (the user just made a decision and could not
tell the gate was even involved).

The F2 fix surfaces the per-provider detail in the aggregated miss: the
reason LEADS the report (the status line is a single clipped row, so the
right-edge clip must never hide the decision), the generic
"(tried N provider(s): …)" tail follows byte-for-byte, and a multi-provider
walk (unknown language) with no refusal keeps the generic shape (no
unrelated-provider noise).

Three legs, one App, own fixture repo (its own PTY-flock, keyed to the repo
path):

L-D  DECLINE: `import missingpkg_f2` + `print(missingpkg_f2.ghost)` — M-. on
     `ghost` raises the fetch banner ("fetch on demand: pip install
     missingpkg_f2 (from main.py) (y/n)?"); the operator declines (`n`); the
     MINIBUFFER (not just the unit-built string) must name the refusal:
     "install refused" + "declined at the fetch confirmation" + the exact
     command that was NOT run. No pip invocation ever happens.
L-N  ORDINARY MISS: a prelude name (`print`, no import) — the python
     provider's OWN no-hint bail ("needs scope info") leads the report; it
     reads as a named reason, not a bare "symbol not found".
L-G  NO-POLLUTION: an unknown-extension file (language = None → the chain
     walks ALL four providers, all bail with their own reasons) — the report
     keeps the generic byte-for-byte shape ("tried 4 provider(s): …") and
     carries NO per-provider noise ("no Cargo.toml" / "no package.json" /
     "no go.mod" must not appear).

The drive owns /tmp/redline_017_f2_repo and removes it on exit.
Wrap the invocation in `timeout` (shared PTY flock scheme: if the lock
is busy this exits 3 — wait and retry, never run two suites at once).

Exit 0 = legs pass; 1 = any failed.
"""
import os, sys, shutil, subprocess
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo

REPO = repo("redline_017_f2_repo")
COLS, ROWS = 200, 24
MINI = ROWS - 2      # minibuffer row (0-based)

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
        # in-project definition AND no enclosing symbol.
        f.write("import missingpkg_f2\n\nprint(missingpkg_f2.ghost)\n")
    with open(os.path.join(REPO, "prelude.py"), "w") as f:
        f.write("print('x')\n")
    with open(os.path.join(REPO, "junk.f2xyz"), "w") as f:
        # Unknown extension → language None → the chain walks ALL four
        # providers (the no-pollution leg).
        f.write("foo_f2(1)\n")
    subprocess.run(["git", "init", "-q", REPO], check=True)
    subprocess.run(["git", "-C", REPO, "add", "-A"], check=True,
                   capture_output=True)
    subprocess.run(["git", "-C", REPO, "commit", "-qm", "017-f2 miss-detail legs"],
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


def open_file(app, name):
    app.key("C-x C-f", 1.0)
    for ch in name:
        app.key(ch, 0.25)
    app.key("RET", 1.2)


def main():
    app = None
    try:
        setup_repo()
        app = App(REPO, rows=ROWS, cols=COLS)

        print("=== L-D: a DECLINED fetch names its refusal in the minibuffer ===")
        open_file(app, "main.py")
        app.key("M-g g", 0.8)
        app.key("3", 0.4)
        app.key("RET", 0.8)    # line 3: `print(missingpkg_f2.ghost)`
        app.key("M-f", 0.8)    # end of `print`
        app.key("M-f", 0.8)    # end of `missingpkg_f2`
        app.key("M-f", 0.8)    # end of `ghost`
        app.key("M-.", 0.5)
        # The gate's banner: the EXACT command + the file that implied it.
        ok, msg = poll_minibuffer(
            app, "fetch on demand: pip install missingpkg_f2 (from main.py) (y/n)?",
            timeout=90.0)
        rec("L-D: the fetch banner shows the exact command and origin",
            ok, f"minibuffer={msg!r}")
        # The operator declines.
        app.key("n", 0.8)
        # The miss report (the resolve event lands after the decline
        # unblocks the provider) must name the REFUSAL — the operator's
        # own decision, with the command that was NOT run.
        ok, msg = poll_minibuffer(app, "install refused", timeout=90.0)
        rec("L-D: the MINIBUFFER names the refusal on decline",
            ok, f"minibuffer={msg!r}")
        rec("L-D: the gate's own reason ('declined at the fetch confirmation') is on screen",
            "declined at the fetch confirmation" in msg, f"minibuffer={msg!r}")
        rec("L-D: the exact command that was NOT run is on screen",
            "pip install missingpkg_f2" in msg, f"minibuffer={msg!r}")

        print("=== L-N: an ordinary miss leads with the provider's own reason ===")
        open_file(app, "prelude.py")
        app.key("M-g g", 0.8)
        app.key("1", 0.4)
        app.key("RET", 0.8)    # line 1: `print('x')`
        app.key("M-f", 0.8)    # end of `print`
        app.key("M-.", 0.5)
        ok, msg = poll_minibuffer(app, "no provider resolution for `print`", timeout=90.0)
        rec("L-N: the miss lands", ok, f"minibuffer={msg!r}")
        rec("L-N: the python provider's OWN no-hint bail leads the report",
            "needs scope info" in msg, f"minibuffer={msg!r}")

        print("=== L-G: a no-refusal multi-provider walk stays generic (no noise) ===")
        open_file(app, "junk.f2xyz")
        app.key("M-g g", 0.8)
        app.key("1", 0.4)
        app.key("RET", 0.8)    # line 1: `foo_f2(1)`
        app.key("M-f", 0.8)    # end of `foo_f2`
        app.key("M-.", 0.5)
        ok, msg = poll_minibuffer(app, "no provider resolution for `foo_f2`", timeout=120.0)
        rec("L-G: the miss lands", ok, f"minibuffer={msg!r}")
        rec("L-G: the generic shape names all four providers (byte-for-byte tail)",
            "tried 4 provider(s): rust, javascript, python, go" in msg,
            f"minibuffer={msg!r}")
        rec("L-G: no unrelated provider noise (no cargo/npm/go reasons)",
            ("no Cargo.toml" not in msg) and ("no package.json" not in msg)
            and ("no go.mod" not in msg),
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
    if failed:
        for name, _, detail in failed:
            print(f"  FAILED: {name} {detail}")
        sys.exit(1)
    sys.exit(0)


if __name__ == "__main__":
    main()

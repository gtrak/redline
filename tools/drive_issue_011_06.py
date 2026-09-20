#!/usr/bin/env python3
"""Plan 011 issue 06 — language-aware M-. tokens drive (python + js legs).

011-06 made `symbol_at_point` language-aware: in a non-Rust buffer the M-.
path token is the WHOLE dotted path when the point sits in the language's
path container (011-03's `node_at` machinery), so the providers' already-
unit-tested dotted handling is reachable from M-.. This drive verifies
exactly the two cells that changed in docs/provider-matrix.md, live, at
the level ONLY a live app can prove (the matrix cells themselves):

  L-P1  PYTHON: plain `import json` + `json.dumps('x')` — M-. on the
        dotted use now lands in the stdlib json source (`def dumps`).
        Pre-011-06 the same keystrokes degraded to the exact no-hint
        bail (pinned live by 011-05 L-P2).
  L-J1  JS: `import * as fakelib` + `fakelib.apply(5)` — M-. on the
        dotted use now lands on the `function apply` in the package's
        entry file. Pre-011-06 the same keystrokes degraded to the exact
        bail (pinned live by 011-05 L-J1a).

Per the updated spec: NO PTY anywhere else — the app-level seam (store
feeds dotted tokens to the resolver) and the degradation pins are unit
tests in store.rs; the providers' dotted handling is provider-level
(real-toolchain shell-outs in js_provider.rs / python_provider.rs,
unchanged by this issue). Go: no leg — the toolchain is absent in this
sandbox (011-05's drive records the loud go skip; the 011-06 Go token
extraction is unit-pinned in store.rs).

The drive owns /tmp/redline_011_06_repo under the per-repo PTY-flock
scheme (keyed on the repo abspath — never contends with the other
suites), wrapped in `timeout` by gate.sh. Exit 0 = legs pass (or skipped
loudly); 1 = a live leg failed. A "shared PTY fixture is busy" (exit 3)
means another suite is live — wait and retry.
"""
import os, sys, shutil, subprocess
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo

REPO = repo("redline_011_06_repo")   # drive-owned root (removed on exit)
PROJ = os.path.join(REPO, "proj")   # the app's workspace root (flock key)
COLS, ROWS = 200, 24
MINI = ROWS - 2      # minibuffer row (0-based)
STATUS = ROWS - 1    # status-line row (0-based)

CHECKS = []


def rec(name, ok, detail=""):
    CHECKS.append((name, ok, detail))
    print(f"  {'PASS' if ok else 'FAIL'}  {name:58s} {detail}")


def rec_skip(name, detail):
    # A skipped leg is a PASS for the gate (nothing failed) but is labeled
    # SKIP in the output so no one mistakes it for a verified leg.
    CHECKS.append((name, True, detail))
    print(f"  SKIP  {name:58s} {detail}")


def setup_repo():
    if os.path.exists(REPO):
        shutil.rmtree(REPO)
    os.makedirs(PROJ)
    # ── python leg fixture ─────────────────────────────────────────────────
    # Top-level use (no in-project definition, no enclosing symbol) so the
    # M-. selection falls through to the tooling resolver.
    with open(os.path.join(PROJ, "py_dotted.py"), "w") as f:
        # Plain `import json`: the dotted token `json.dumps` reaches the
        # provider, which resolves it on its OWN path (no import-walk hint
        # for dotted symbols — 011-02/011-06 interplay).
        f.write("import json\n\njson.dumps('x')\n")
    # ── js leg fixture ─────────────────────────────────────────────────────
    with open(os.path.join(PROJ, "main.js"), "w") as f:
        f.write('import * as fakelib from "fakelib";\n\n'
                "fakelib.apply(5);\n")
    with open(os.path.join(PROJ, "package.json"), "w") as f:
        f.write('{"name": "011-06-fixture", "private": true}\n')
    # The "installed" package at the ANCESTOR level (hoisted node_modules
    # shape), built BY HAND (no npm install runs in the drive; the js
    # provider's node_modules walk is pure file-system). Outside the
    # project root so the landing is an EXTERNAL buffer.
    pkg = os.path.join(REPO, "node_modules", "fakelib")
    os.makedirs(pkg)
    with open(os.path.join(pkg, "package.json"), "w") as f:
        f.write('{"name": "fakelib", "version": "1.0.0", "main": "index.js"}\n')
    with open(os.path.join(pkg, "index.js"), "w") as f:
        f.write("function apply(v) {\n"
                "  return v + 1;\n"
                "}\n"
                "export { apply };\n")
    # node_modules is a gitignore EXCLUSION: the app's project file walk
    # must not see the package's definitions, so M-. genuinely falls
    # through to the provider.
    with open(os.path.join(REPO, ".gitignore"), "w") as f:
        f.write("node_modules/\n")
    subprocess.run(["git", "init", "-q", PROJ], check=True)
    subprocess.run(["git", "-C", PROJ, "add", "-A"], check=True,
                   capture_output=True)
    subprocess.run(["git", "-C", PROJ, "commit", "-qm", "011-06 dotted-token legs"],
                   check=True, capture_output=True)


def open_file(app, name):
    app.key("C-x C-f", 1.0)
    for ch in name:
        app.key(ch, 0.25)
    app.key("RET", 1.2)


def poll_minibuffer(app, needle, timeout=90.0):
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        app._read(0.3, quiet=0.2)
        last = app.row_text(MINI)
        if needle in last:
            return True, last
    return False, last


def poll_screen(app, needle, timeout=90.0):
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        app._read(0.3, quiet=0.2)
        last = app.screen_text()
        if needle in last:
            return True, last
    return False, last


def main():
    print(f"REPO={REPO} (workspace {PROJ})\n")
    python3_ok = shutil.which("python3") is not None
    node_ok = shutil.which("node") is not None and shutil.which("npm") is not None
    print(f"toolchains: python3={python3_ok}"
          f" node={shutil.which('node') is not None}"
          f" npm={shutil.which('npm') is not None}\n")
    if not python3_ok:
        print("┌" + "─" * 76 + "┐")
        print("│ PYTHON LEG: SKIPPED — `python3` ABSENT (LOUD) — the provider's      │")
        print("│   find_spec shell-out cannot run; the python path-shaped cell  │")
        print("│   stays UNIT-covered (store.rs 011-06 tests + python_provider.│")
        print("│   rs dotted tests).                                            │")
        print("└" + "─" * 76 + "┘")
        rec_skip("python leg: SKIPPED LOUDLY — `python3` absent",
                 "unit-covered only")
    if not node_ok:
        print("┌" + "─" * 76 + "┐")
        print("│ JS LEG: SKIPPED — node/npm ABSENT (LOUD) — the js path-shaped │")
        print("│   cell stays UNIT-covered (store.rs 011-06 tests +            │")
        print("│   js_provider.rs dotted tests).                               │")
        print("└" + "─" * 76 + "┘")
        rec_skip("js leg: SKIPPED LOUDLY — node/npm absent",
                 "unit-covered only")
    if not (python3_ok or node_ok):
        print("No runnable toolchain — nothing to drive live. Exit 0 (skips "
              "recorded above, no silent pass).")
        sys.exit(0)

    app = None
    try:
        setup_repo()
        app = App(PROJ, rows=ROWS, cols=COLS)

        if python3_ok:
            print("=== PYTHON L-P1: dotted `json.dumps` lands (011-06) ===")
            # Pre-011-06: the app's `::`-only extraction fed the BARE
            # `dumps` with no hint → the exact no-hint bail (011-05 L-P2).
            # Now: the path token IS `json.dumps`; the python provider's
            # dotted handling resolves it on its own path.
            open_file(app, "py_dotted.py")
            app.key("M-g g", 0.8)
            app.key("3", 0.4)
            app.key("RET", 0.8)    # line 3: top-level `json.dumps('x')`
            app.key("M-f", 0.6)    # end of the `json` run
            app.key("M-f", 0.6)    # end of the `dumps` run
            app.key("M-.", 0.5)
            ok, screen = poll_screen(app, "def dumps", timeout=120.0)
            rec("L-P1: M-. on the dotted `json.dumps` lands in the json stdlib",
                ok, f"screen has 'def dumps'={ok}")
            ok2, mini = poll_minibuffer(app, "jumped to", timeout=5.0)
            rec("L-P1: the landing is a resolved-source jump (json path)",
                ok2 and "json" in mini, f"minibuffer={mini!r}")

        if node_ok:
            print("=== JS L-J1: dotted `fakelib.apply` lands (011-06) ===")
            # Pre-011-06: the resolver got the BARE `apply`; the namespace
            # import hints only the entry name and the 011-02 walk never
            # guesses a member's path → the exact bail (011-05 L-J1a).
            # Now: the path token IS `fakelib.apply`; the js provider's
            # dotted handling locates the member in the package.
            open_file(app, "main.js")
            app.key("M-g g", 0.8)
            app.key("3", 0.4)
            app.key("RET", 0.8)    # line 3: top-level `fakelib.apply(5);`
            app.key("M-f", 0.6)    # end of the `fakelib` run
            app.key("M-f", 0.6)    # end of the `apply` run
            app.key("M-.", 0.5)
            ok, screen = poll_screen(app, "function apply", timeout=120.0)
            rec("L-J1: M-. on the dotted `fakelib.apply` lands in the package",
                ok, f"screen has 'function apply'={ok}")
            ok2, mini = poll_minibuffer(app, "jumped to", timeout=5.0)
            rec("L-J1: the landing is a resolved-source jump (index.js)",
                ok2 and "index.js" in mini, f"minibuffer={mini!r}")
    finally:
        if app is not None:
            try:
                app.kill()
            except Exception:
                pass
        shutil.rmtree(REPO, ignore_errors=True)  # the whole drive-owned tree

    failed = [c for c in CHECKS if not c[1]]
    passed = len(CHECKS) - len(failed)
    skips = [c for c in CHECKS if "SKIPPED" in c[0]]
    live = len(CHECKS) - len(failed) - len(skips)
    if skips:
        print(
            f"\n{live}/{len(CHECKS)} live legs passed; "
            f"{len(skips)} recorded skip(s): "
            + ", ".join(c[0] for c in skips)
        )
    else:
        print(f"\n{len(CHECKS) - len(failed)}/{len(CHECKS)} legs passed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()

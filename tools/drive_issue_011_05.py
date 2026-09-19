#!/usr/bin/env python3
"""Plan 011 issue 05 — resolver parity LOCK-IN drive (python + js legs).

The final 011 issue observes and documents; this drive pins, live, the
per-language end states the matrix doc (docs/provider-matrix.md) reports,
following the drive_external_crate/drive_external_use pattern exactly:
own fixture repo, own per-repo PTY flock (keyed on the repo abspath, so it
never contends with the other suites), its own setup_repo(), wrapped in
`timeout`, wired into gate.sh SHARED_SUITES.

One drive, per-language legs (the contract allows either shape). ONE app,
ONE repo; the legs are sequential:

PYTHON (requires python3 — the provider shelling out to it is the live
  toolchain; a missing python3 makes the leg SKIP WITH A LOUD NOTE):
  L-P1  BARE imported symbol (`dumps` from `from json import dumps`, the
        011-02 hint) lands in the stdlib json source (re-pin of 011-02's
        live leg in this drive's own repo).
  L-P3  BLAME on the landed external buffer (`C-x g` then `b`): degrades to
        the exact external bail "no git history for external sources"
        (store.rs open_blame external_buffers branch — one code path for
        every language, so this single live pin covers the cell).
  L-P2  PATH-SHAPED use with a plain `import json` (`json.dumps('x')`):
        the app's char-based M-. extraction is `::`-only (Rust), so the
        resolver gets the BARE `dumps` with NO scope hint — it degrades to
        the exact no-hint bail. The provider's dotted handling (json.dumps)
        is unit-covered only (crates/redline-resolve/src/providers/
        python_provider.rs resolve_stdlib_module / resolve_dotted_chain_
        fallback). DISCRIMINATING BONUS: the miss message reads
        "tried 1 provider(s): python" — a live pin that the 011-01
        language dispatch means a miss probes ONLY the matching provider
        (never all four toolchains).

JAVASCRIPT (requires node + npm — both are present for the drive to run;
  the provider itself only needs node_modules, which setup_repo builds by
  hand, so no install runs):
  L-J1a PATH-SHAPED `ns.member` use (`fakelib.apply(…)` from
        `import * as fakelib`): the app's M-. extraction is `::`-only
        (Rust), so the resolver gets the BARE `apply` — and a namespace
        import hints only the ENTRY name, never members (011-02's bounded
        walk — never guessed). It degrades to the exact bail. The
        provider's own dotted handling (fakelib.apply) is unit-covered
        only (js_provider.rs). DISCRIMINATING BONUS: the miss reads
        "tried 1 provider(s): javascript" — the live dispatch pin.
  L-J1b PATH-SHAPED namespace ENTRY (`fakelib(5);` — the bare namespace
        binding): the 011-02 P2-1 1-segment hint `["fakelib"]` lands on
        the package ENTRY file (index.js) — the live JS external landing.
  L-J3  IN-LIBRARY FOLLOW-UP (011-04): INSIDE the landed index.js, M-. on
        the bare `clamp` (defined in the sibling util.js of the same
        package) jumps IN-CRATE through the freshly built js-tree crate
        index — crate-relative "jumped to util.js:1", never a resolver
        bail.
  L-J2  BARE imported symbol (`import { clamp } from "fakelib"`, the 011-02
        named-import hint `["fakelib","clamp"]`) lands in util.js (re-pin
        of 011-02's JS path in this drive's own repo).

GO: the `go` toolchain is ABSENT in this sandbox. The contract says the go
  leg must SKIP WITH A LOUD, VISIBLE NOTE — never a silent pass, never a
  fake fixture. So: when `go` is missing the drive prints a big banner,
  builds NO go fixture (there is nothing to run), records a SKIP check,
  and exits 0. Go resolution stays unit-covered only (go_provider.rs's
  39 live-shelling unit tests + the 011-04 pure-tree-sitter walk tests).

The drive owns /tmp/redline_011_05_repo under the shared PTY-flock scheme
(Never the shared /tmp/redline_pyte_repo) and removes the tree on exit.
The app's workspace is REPO/proj; node_modules sits at REPO level (one
ancestor up — the js provider's locate walk is ancestor-walking by design,
and keeping the package OUTSIDE the project root makes the landing an
EXTERNAL buffer, which is what the in-crate follow-up leg (L-J3) requires:
exactly the real-world monorepo/hoisted-node_modules shape).
Wrap the invocation in `timeout`. Shared-fixture busy (exit 3) ⇒ wait and
retry; never run two suites concurrently.

Exit 0 = all legs pass (skipped legs recorded loudly, not failed);
1 = any live leg failed.
"""
import os, sys, shutil, subprocess
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App

REPO = "/tmp/redline_011_05_repo"   # drive-owned root (removed on exit)
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
    # ── python leg fixtures ────────────────────────────────────────────────
    # Probe calls sit at TOP LEVEL so the M-. selection falls through to
    # the tooling resolver (no in-project definition, no enclosing symbol).
    with open(os.path.join(PROJ, "py_bare.py"), "w") as f:
        f.write("from json import dumps\n\ndumps('x')\n")
    with open(os.path.join(PROJ, "py_path.py"), "w") as f:
        # Plain `import json` binds only the top-level `json` — the 011-02
        # walk never fabricates a module path for the bare `dumps` of the
        # attribute chain (never guessed).
        f.write("import json\n\njson.dumps('x')\n")
    # ── js leg fixtures ────────────────────────────────────────────────────
    with open(os.path.join(PROJ, "main.js"), "w") as f:
        f.write('import * as fakelib from "fakelib";\n\n'
                "fakelib.apply(5);\n")
    with open(os.path.join(PROJ, "ns.js"), "w") as f:
        # The namespace binding used BARE (the entry name itself): the
        # 011-02 P2-1 1-segment hint `["fakelib"]` lands on the package
        # entry file.
        f.write('import * as fakelib from "fakelib";\n\n'
                "fakelib(5);\n")
    with open(os.path.join(PROJ, "named.js"), "w") as f:
        f.write('import { clamp } from "fakelib";\n\n'
                "clamp(1, 0, 10);\n")
    # A realistic npm root (the js provider's find_local_path_dep reads it:
    # no file:/link: deps, so nothing is mis-located).
    with open(os.path.join(PROJ, "package.json"), "w") as f:
        f.write('{"name": "parity-fixture", "private": true}\n')
    # The "installed" package at the ANCESTOR level (hoisted node_modules
    # shape): built BY HAND (no npm install runs in the drive; the provider's
    # node_modules walk is pure file-system).
    pkg = os.path.join(REPO, "node_modules", "fakelib")
    os.makedirs(pkg)
    with open(os.path.join(pkg, "package.json"), "w") as f:
        f.write('{"name": "fakelib", "version": "1.0.0", "main": "index.js"}\n')
    with open(os.path.join(pkg, "index.js"), "w") as f:
        f.write('import { clamp } from "./util.js";\n\n'
                "function apply(v) {\n"
                "  return clamp(v, 0, 10);\n"
                "}\n"
                "export { apply };\n")
    with open(os.path.join(pkg, "util.js"), "w") as f:
        f.write("function clamp(v, lo, hi) {\n"
                "  return v < lo ? lo : v > hi ? hi : v;\n"
                "}\n"
                "export { clamp };\n")
    # node_modules is a gitignore EXCLUSION: the app's project file walk
    # (and thus the workspace symbol index) must not see the package's
    # definitions, so M-. genuinely falls through to the provider.
    with open(os.path.join(REPO, ".gitignore"), "w") as f:
        f.write("node_modules/\n")
    # No go fixture at all: with the toolchain absent a fixture would be a
    # fake (the contract forbids it). setup builds nothing go-shaped.
    subprocess.run(["git", "init", "-q", PROJ], check=True)
    subprocess.run(["git", "-C", PROJ, "add", "-A"], check=True,
                   capture_output=True)
    subprocess.run(["git", "-C", PROJ, "commit", "-qm", "011-05 parity legs"],
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


def wait_index_settled(app, timeout=90.0):
    """Block until the `indexing crate …` indicator has disappeared (the
    landed tree's background build finished). A short settle first: a tiny
    tree (2 files) may build faster than a single poll interval, so the
    indicator window can be sub-poll — the absence check after the settle
    is the readiness gate."""
    time.sleep(2.0)
    deadline = time.time() + timeout
    while time.time() < deadline:
        app._read(0.3, quiet=0.2)
        if "indexing crate" not in app.row_text(STATUS):
            return True
    return False


def main():
    print(f"REPO={REPO} (workspace {PROJ})\n")
    python3_ok = shutil.which("python3") is not None
    node_ok = shutil.which("node") is not None and shutil.which("npm") is not None
    go_ok = shutil.which("go") is not None
    print(f"toolchains: python3={python3_ok} node={shutil.which('node') is not None}"
          f" npm={shutil.which('npm') is not None} go={go_ok}\n")

    # ── GO: skip loudly (the contract's explicit requirement) ─────────────
    if go_ok:
        print("NOTE: `go` present — this drive version carries no go legs "
              "(011-05 observes what the sandbox can verify; go legs would "
              "need a go-module cache fixture). Go stays unit-covered.")
    else:
        print("┌" + "─" * 76 + "┐")
        print("│ GO LEG: SKIPPED — `go` toolchain ABSENT in this environment (LOUD)   │")
        print("│   No go fixture is built, no go leg runs, no silent pass: go        │")
        print("│   resolution stays UNIT-covered only (go_provider.rs live tests +  │")
        print("│   the 011-04 pure-tree-sitter go walk/extraction tests).            │")
        print("└" + "─" * 76 + "┘")
        rec_skip("go leg: SKIPPED LOUDLY — `go` toolchain absent",
                 "no fixture built; unit-covered only")
    if not python3_ok:
        print("┌" + "─" * 76 + "┐")
        print("│ PYTHON LEG: SKIPPED — `python3` ABSENT (LOUD) — the provider's      │")
        print("│   find_spec/pip shell-out cannot run; python stays unit-covered.    │")
        print("└" + "─" * 76 + "┘")
        rec_skip("python legs: SKIPPED LOUDLY — `python3` absent", "unit-covered only")
    if not node_ok:
        print("┌" + "─" * 76 + "┐")
        print("│ JS LEG: SKIPPED — node/npm ABSENT (LOUD) — js resolution stays      │")
        print("│   UNIT-covered only (js_provider.rs live tests).                    │")
        print("└" + "─" * 76 + "┘")
        rec_skip("js legs: SKIPPED LOUDLY — node/npm absent", "unit-covered only")
    if not (python3_ok or node_ok):
        # Nothing can run live: report it and stop (loudly) without a PTY.
        print("No runnable toolchain — nothing to drive live. Exit 0 (skips "
              "recorded above, no silent pass).")
        sys.exit(0)

    app = None
    try:
        setup_repo()
        app = App(PROJ, rows=ROWS, cols=COLS)

        if python3_ok:
            print("=== PYTHON L-P1: bare imported `dumps` lands (011-02 hint) ===")
            open_file(app, "py_bare.py")
            app.key("M-g g", 0.8)
            app.key("3", 0.4)
            app.key("RET", 0.8)    # line 3: top-level `dumps('x')`
            app.key("M-f", 0.8)    # point to the END of the `dumps` run
            app.key("M-.", 0.5)
            ok, screen = poll_screen(app, "def dumps", timeout=120.0)
            rec("L-P1: M-. on the bare import lands in the json stdlib source",
                ok, f"screen has 'def dumps'={ok}")
            ok2, mini = poll_minibuffer(app, "jumped to", timeout=5.0)
            rec("L-P1: the landing is a resolved-source jump (json path)",
                ok2 and "json" in mini, f"minibuffer={mini!r}")
            rec("L-P1: the landed stdlib tree's index settles",
                wait_index_settled(app), "indexing indicator cleared")

            print("=== PYTHON L-P3: blame on the external buffer bails ===")
            # Current buffer = the landed json/__init__.py (external).
            # C-x g → magit status view (the current buffer is unchanged),
            # b → open_blame: the external_buffers branch refuses with the
            # exact WHY message.
            app.key("C-x g", 0.8)
            app.key("b", 0.8)
            ok, msg = poll_minibuffer(
                app, "no git history for external sources", timeout=30.0)
            rec("L-P3: blame degrades to the external bail (matrix 'blame' cell)",
                ok, f"minibuffer={msg!r}")
            # Exit the magit status view (q = close-view) back to the
            # buffer view: M-. / C-x C-f are buffer-view dispatch, and the
            # next leg needs a clean buffer-view state.
            app.key("q", 0.8)

            print("=== PYTHON L-P2: path-shaped `json.dumps` bails honestly ===")
            open_file(app, "py_path.py")
            app.key("M-g g", 0.8)
            app.key("3", 0.4)
            app.key("RET", 0.8)    # line 3: top-level `json.dumps('x')`
            app.key("M-f", 0.6)    # end of the `json` run
            app.key("M-f", 0.6)    # end of the `dumps` run
            app.key("M-.", 0.5)
            ok, msg = poll_minibuffer(
                app, "no provider resolution", timeout=90.0)
            rec("L-P2: plain-import path-shaped use is NOT guessed (the miss lands)",
                ok, f"minibuffer={msg!r}")
            rec("L-P2: the miss probes ONLY the matching provider (live dispatch pin)",
                ok and "tried 1 provider(s): python" in msg, f"minibuffer={msg!r}")

        if node_ok:
            print("=== JS L-J1a: path-shaped `fakelib.apply` bails honestly ===")
            # The app's char-based M-. extraction is `::`-only (Rust): in a
            # js buffer the resolver gets the BARE `apply`. A namespace
            # import hints only the entry name, never members — the 011-02
            # walk never guesses a module path for a member (byte-for-byte
            # degradation). The provider's own dotted handling is unit-only.
            open_file(app, "main.js")
            app.key("M-g g", 0.8)
            app.key("3", 0.4)
            app.key("RET", 0.8)    # line 3: top-level `fakelib.apply(5);`
            app.key("M-f", 0.6)    # end of the `fakelib` run
            app.key("M-f", 0.6)    # end of the `apply` run
            app.key("M-.", 0.5)
            ok, msg = poll_minibuffer(
                app, "no provider resolution", timeout=90.0)
            rec("L-J1a: a namespace member is NOT guessed (the miss lands)",
                ok, f"minibuffer={msg!r}")
            rec("L-J1a: the miss probes ONLY the matching provider (live dispatch pin)",
                ok and "tried 1 provider(s): javascript" in msg,
                f"minibuffer={msg!r}")

            print("=== JS L-J1b: the namespace ENTRY lands in the package ===")
            open_file(app, "ns.js")
            app.key("M-g g", 0.8)
            app.key("3", 0.4)
            app.key("RET", 0.8)    # line 3: top-level `fakelib(5);`
            app.key("M-f", 0.8)    # point to the END of the `fakelib` run
            app.key("M-.", 0.5)
            ok, screen = poll_screen(app, "import { clamp }", timeout=120.0)
            rec("L-J1b: the bare namespace lands on the package entry (index.js)",
                ok, f"screen has the index.js import line={ok}")
            ok2, mini = poll_minibuffer(app, "jumped to", timeout=5.0)
            rec("L-J1b: the landing is a resolved-source jump (index.js)",
                ok2 and "index.js" in mini, f"minibuffer={mini!r}")
            rec("L-J1b: the landed package's index settles",
                wait_index_settled(app), "indexing indicator cleared")

            print("=== JS L-J3: M-. INSIDE the package jumps in-crate ===")
            # Line 4 of index.js: `  return clamp(v, 0, 10);` — `clamp` is
            # defined in the sibling util.js. Through the fresh js-tree
            # crate index (011-04) this jumps IN-CRATE (crate-relative
            # message); pre-011-04 the js tree was never indexed and this
            # would bail to the resolver instead.
            app.key("M-g g", 0.8)
            app.key("4", 0.4)
            app.key("RET", 0.8)
            app.key("M-f", 0.5)    # end of the `return` run
            app.key("M-f", 0.5)    # end of the `clamp` run
            app.key("M-.", 0.5)
            ok, msg = poll_minibuffer(app, "jumped to util.js", timeout=90.0)
            rec("L-J3: M-. inside the dependency lands in util.js (in-crate index)",
                ok, f"minibuffer={msg!r}")
            rec("L-J3: NOT a resolver bail (the index answered)",
                not ("no provider resolution" in msg), f"minibuffer={msg!r}")

            print("=== JS L-J2: bare `import { clamp }` lands (011-02 hint) ===")
            open_file(app, "named.js")
            app.key("M-g g", 0.8)
            app.key("3", 0.4)
            app.key("RET", 0.8)    # line 3: top-level `clamp(1, 0, 10);`
            app.key("M-f", 0.8)    # point to the END of the `clamp` run
            app.key("M-.", 0.5)
            ok, screen = poll_screen(app, "function clamp", timeout=120.0)
            rec("L-J2: M-. on the bare named import lands in util.js",
                ok, f"screen has 'function clamp'={ok}")
            ok2, mini = poll_minibuffer(app, "jumped to", timeout=5.0)
            rec("L-J2: the landing is a resolved-source jump (util.js)",
                ok2 and "util.js" in mini, f"minibuffer={mini!r}")
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

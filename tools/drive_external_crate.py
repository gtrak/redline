#!/usr/bin/env python3
"""In-crate navigation thin tier (loop-04): the registry-landing smoke.

One of the original sections stays PTY (only a live app proves it):
  L1 M-. lands in the registry source: in a dedicated repo (NOT the shared
     fixture — the fixture has no Cargo.toml, and these legs need a cargo
     graph) the cursor sits at the end of `ropey` in the top-level
     `ropey::Rope::new()` probe. M-. misses the project symbol index and
     the rust provider resolves via `cargo metadata` (the registry source
     is cached, so it is fast and offline) → lands READ-ONLY in
     ~/.cargo/registry/src/<hash>/ropey-1.6.1/src/rope.rs.

The rest is store-level now (unit twins in src/app/flow_tests.rs, loop-04 —
the `indexing crate …` bus and the crate index are store state, driven
through the same store entry points):
  L2/L3 `indexing crate …` indicator appears + clears
                                       -> unit_flow_ext_crate_landing_indicator
  L4 M-. within the crate (crate-relative jump)
                                       -> unit_flow_ext_crate_in_crate_mdot
  L5 imenu on the external buffer (crate index, not a refusal)
                                       -> unit_flow_ext_crate_imenu
  L6/L7 M-, walks the jump stack back
                                       -> unit_flow_ext_crate_in_crate_mdot

The drive owns its own repo dir (/tmp/redline_ext_crate_repo) under the
shared PTY-flock scheme (the lock is keyed to the repo path, so it never
contends with the /tmp/redline_pyte_repo suites) and removes the repo on
exit. Wrap the invocation in `timeout`.

Exit 0 = all legs pass; 1 = any failed.
"""
import os, re, sys, shutil
import subprocess
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App

REPO = "/tmp/redline_ext_crate_repo"
ROPEY_ROOT = os.path.expanduser(
    "~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ropey-1.6.1")
ROPE_RS = os.path.join(ROPEY_ROOT, "src", "rope.rs")
REGISTRY_SRC_ROOT = os.path.expanduser("~/.cargo/registry/src")
COLS, ROWS = 200, 24
# 200 cols: the M-. landing message carries the FULL absolute registry path
# (90+ chars); the default 80-col PTY would clip it and break the parse.
MINI = ROWS - 2      # minibuffer row (0-based)

CHECKS = []


def rec(name, ok, detail=""):
    CHECKS.append((name, ok, detail))
    print(f"  {'PASS' if ok else 'FAIL'}  {name:58s} {detail}")


def setup_repo():
    if os.path.exists(REPO):
        shutil.rmtree(REPO)
    os.makedirs(os.path.join(REPO, "src"))
    with open(os.path.join(REPO, "Cargo.toml"), "w") as f:
        # The dependency is RENAMED (`extrope`) so the crate name `ropey`
        # never appears as a key the project symbol-indexer would index as a
        # definition (a bare `ropey = "1.6.1"` key made M-. jump to
        # Cargo.toml instead of falling through to the resolver). cargo
        # metadata still lists `ropey` in the graph, which is all the rust
        # provider needs to locate the registry source.
        f.write('[package]\nname = "ext_crate"\nversion = "0.1.0"\n'
                'edition = "2021"\n\n[dependencies]\n'
                'extrope = { package = "ropey", version = "1.6.1" }\n')
    with open(os.path.join(REPO, "src", "main.rs"), "w") as f:
        # The probe call sits at TOP LEVEL (line 5): M-. selection falls
        # through to the tooling resolver only when the point has no
        # in-project definition AND no enclosing symbol (drive_xref's
        # fixture does the same with a bare top-level call).
        f.write("fn main() {\n"
                "    let r = ropey::Rope::new();\n"
                "    let _ = &r;\n"
                "}\n"
                "ropey::Rope::new();\n")
    subprocess.run(["git", "init", "-q", REPO], check=True)
    subprocess.run(["git", "-C", REPO, "add", "-A"], check=True,
                   capture_output=True)
    subprocess.run(["git", "-C", REPO, "commit", "-qm", "ext crate leg"],
                   check=True, capture_output=True)


def open_main(app):
    app.key("C-x C-f", 1.0)
    for ch in "main":
        app.key(ch, 0.25)
    app.key("RET", 1.2)



def prewarm_cargo():
    """Run `cargo metadata` up front. A cold registry index makes the
    in-app provider's `cargo metadata` pay a network fetch inside its own
    30s budget (then a 120s `cargo fetch` fallback) — paying it here keeps
    the landing deterministic and fails fast with a readable message.
    """
    out = subprocess.run(["cargo", "metadata", "--format-version", "1"],
                         cwd=REPO, capture_output=True, timeout=120)
    if out.returncode != 0:
        print(f"FAIL cargo metadata pre-warm failed "
              f"(is the crates.io index reachable?): "
              f"{out.stderr.decode(errors='replace')[:200]!r}")
        sys.exit(1)


def main():
    print(f"REPO={REPO}\n")
    if not os.path.exists(ROPE_RS):
        print(f"FAIL ropey registry source missing at {ROPE_RS}")
        sys.exit(1)
    setup_repo()
    prewarm_cargo()
    app = None
    try:
        app = App(REPO, rows=ROWS, cols=COLS)

        # ── L1: M-. into the ropey registry source (read-only external) ─
        print("=== L1: M-. into the ropey registry source ===")
        open_main(app)
        app.key("M-g g", 0.8)
        app.key("5", 0.4)
        app.key("RET", 0.8)   # line 5: top-level `ropey::Rope::new();` probe
        app.key("M-f", 0.8)   # point to the END of the `ropey` run
        # Poll the minibuffer for the landing (the `indexing crate`
        # indicator may share the status line meanwhile — that timing half
        # is the unit twin's now).
        deadline = time.time() + 60.0
        ok = False
        msg = ""
        while time.time() < deadline:
            app._read(0.3, quiet=0.2)
            if "jumped to" in app.row_text(MINI):
                ok = True
                msg = app.row_text(MINI)
                break
        m = re.search(r"jumped to (\S+):(\d+)", msg)
        landed = m.group(1) if m else None
        rec("L1: M-. reports the jump", ok and landed is not None,
            f"minibuffer={msg!r}")
        rec("L1: the landing path is a registry source (absolute, external)",
            bool(landed) and landed.startswith(REGISTRY_SRC_ROOT)
            and "ropey-1.6.1/src/rope.rs" in landed,
            f"landed={landed!r}")
        rec("L1: the window landed on rope.rs (Rope in view)",
            "Rope" in app.screen_text(),
            f"top={app.row_text(1)!r}")
    finally:
        if app is not None:
            app.kill()
        shutil.rmtree(REPO, ignore_errors=True)
    bad = [n for n, ok, _ in CHECKS if not ok]
    print("\n=== SUMMARY ===")
    for n, ok, _ in CHECKS:
        print(f"  {'PASS' if ok else 'FAIL'}  {n}")
    print(f"\n{len(CHECKS) - len(bad)}/{len(CHECKS)} external-crate legs passed")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

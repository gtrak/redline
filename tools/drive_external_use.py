#!/usr/bin/env python3
"""Plan 007 issue 03 — scope-aware resolver fall-through (M-. on a BARE
symbol that a `use` declaration brings into scope).

Legs (a dedicated repo, NOT the shared fixture — these legs need a cargo
graph; the drive owns /tmp/redline_ext_use_repo under the shared PTY-flock
scheme, so the driver's abspath-keyed lock never contends with the
/tmp/redline_pyte_repo suites; the repo is removed on exit):

  L1 bare use-imported symbol resolves: `use serde::Deserialize;` at the
     top of main.rs; the M-. probe is a TOP-LEVEL `let d = Deserialize;`
     (an expression statement is not an outline item, so the M-. selection
     falls through to the tooling resolver — the drive_external_crate
     precedent). The app now supplies the use path as the SymbolContext
     scope hint (007-03) and the rust provider resolves through the SAME
     locate_source_dir / locate_in_pkg machinery as a path-shaped symbol,
     landing READ-ONLY in the (cached) serde registry source — the `pub
     trait Deserialize` definition.
  L2 degradation pin: a BARE symbol with NO `use` (`let other = 9;`) still
     misses with the graceful "no provider resolution for `other`" report
     (byte-for-byte behavior — no hint, no guessing).

Exit 0 = all legs pass; 1 = any failed. Wrap the invocation in `timeout`.
"""
import os
import re
import sys
import shutil
import subprocess
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App

REPO = "/tmp/redline_ext_use_repo"
REGISTRY_SRC_ROOT = os.path.expanduser("~/.cargo/registry/src")
COLS, ROWS = 200, 24
# 200 cols: the M-. landing message carries the FULL absolute registry path
# (90+ chars); the default 80-col PTY would clip it and break the parse.
MINI = ROWS - 2      # minibuffer row (0-based)
STATUS = ROWS - 1    # status-line row (0-based)

CHECKS = []


def rec(name, ok, detail=""):
    CHECKS.append((name, ok, detail))
    print(f"  {'PASS' if ok else 'FAIL'}  {name:58s} {detail}")


def find_cached_serde():
    """A cached `serde-<semver>` registry source dir (not serde-untagged /
    serde_core). Returns (dir, version) or (None, None)."""
    best = None
    for hash_dir in os.listdir(REGISTRY_SRC_ROOT):
        full = os.path.join(REGISTRY_SRC_ROOT, hash_dir)
        if not os.path.isdir(full):
            continue
        for name in os.listdir(full):
            m = re.match(r"serde-(\d+)\.(\d+)\.(\d+)(-.*)?$", name)
            if not m or not os.path.isdir(os.path.join(full, name, "src")):
                continue
            key = tuple(int(x) for x in m.groups()[:3])
            if best is None or key > best[0]:
                best = (key, os.path.join(full, name), f"{key[0]}.{key[1]}.{key[2]}")
    if best is None:
        return None, None
    return best[1], best[2]


def setup_repo(serde_version):
    if os.path.exists(REPO):
        shutil.rmtree(REPO)
    os.makedirs(os.path.join(REPO, "src"))
    with open(os.path.join(REPO, "Cargo.toml"), "w") as f:
        f.write('[package]\nname = "x_use"\nversion = "0.1.0"\n'
                'edition = "2021"\n\n[dependencies]\n'
                f'serde = "= {serde_version}"\n')
    with open(os.path.join(REPO, "src", "main.rs"), "w") as f:
        # Top-level probes (expression statements, not outline items): M-.
        # selection falls through to the tooling resolver.
        f.write("use serde::Deserialize;\n"      # line 1
                "let d = Deserialize;\n"         # line 2 (L1 probe)
                "let other = 9;\n")             # line 3 (L2 probe)
    subprocess.run(["git", "init", "-q", REPO], check=True)
    subprocess.run(["git", "-C", REPO, "add", "-A"], check=True,
                   capture_output=True)
    subprocess.run(["git", "-C", REPO, "commit", "-qm", "use-scope leg"],
                   check=True, capture_output=True)


def open_main(app):
    app.key("C-x C-f", 1.0)
    for ch in "main":
        app.key(ch, 0.25)
    app.key("RET", 1.2)


def goto(app, n):
    app.key("M-g g", 0.8)
    for ch in str(n):
        app.key(ch, 0.35)
    app.key("RET", 0.8)


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
    serde_dir, serde_version = find_cached_serde()
    print(f"REPO={REPO}\n")
    if serde_dir is None:
        print(f"FAIL no cached serde registry source under {REGISTRY_SRC_ROOT}")
        sys.exit(1)
    print(f"serde cache: {serde_dir}\n")
    setup_repo(serde_version)
    app = None
    try:
        app = App(REPO, rows=ROWS, cols=COLS)

        # ── L1: M-. on a bare use-imported symbol lands in serde ──────
        print("=== L1: M-. on bare `Deserialize` (use-imported) ===")
        open_main(app)
        goto(app, 2)          # "let d = Deserialize;"
        app.key("M-f M-f M-f", 1.0)  # end of the `Deserialize` run
        app.key("M-.", 0.5)
        ok, msg = poll_minibuffer(app, "jumped to", timeout=120.0)
        m = re.search(r"jumped to (\S+):(\d+)", msg) if ok else None
        landed = m.group(1) if m else None
        rec("L1: M-. reports the jump", bool(ok and landed), f"minibuffer={msg!r}")
        rec("L1: the landing is the serde registry source (external)",
            bool(landed) and landed.startswith(REGISTRY_SRC_ROOT)
            and "serde-" in landed,
            f"landed={landed!r}")
        rec("L1: the window shows the Deserialize trait definition",
            "trait Deserialize" in app.screen_text(),
            f"top={app.row_text(1)!r}")
        # Back to the project file for L2 (the landing recorded a jump).
        app.key("M-,", 1.2)
        rec("L1: M-, returns to the project file (src/main.rs)",
            "src/main.rs" in app.row_text(STATUS),
            f"status={app.row_text(STATUS)!r}")

        # ── L2: degradation pin — a bare symbol with NO `use` ─────────
        print("\n=== L2: bare symbol without an import still misses ===")
        goto(app, 3)          # "let other = 9;"
        app.key("M-f M-f", 1.0)  # end of the `other` run
        app.key("M-.", 0.5)
        ok, msg = poll_minibuffer(app, "no provider resolution", timeout=120.0)
        rec("L2: graceful miss report names the symbol",
            bool(ok) and "`other`" in msg and "tried 1 provider(s): rust" in msg,
            f"minibuffer={msg!r}")
        rec("L2: a bare unimported symbol never jumps",
            "jumped to" not in msg, f"minibuffer={msg!r}")
    finally:
        if app is not None:
            app.kill()
        shutil.rmtree(REPO, ignore_errors=True)
    bad = [n for n, ok, _ in CHECKS if not ok]
    print("\n=== SUMMARY ===")
    for n, ok, _ in CHECKS:
        print(f"  {'PASS' if ok else 'FAIL'}  {n}")
    print(f"\n{len(CHECKS) - len(bad)}/{len(CHECKS)} external-use legs passed")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

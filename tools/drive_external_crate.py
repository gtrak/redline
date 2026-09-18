#!/usr/bin/env python3
"""Plan 006 issue 03 — navigation INSIDE external (registry) sources.

Drives the in-crate navigation that pre-006-03 refused ("buffer not in
project"):

  L1 M-. lands in the registry source: in a dedicated repo (NOT the shared
     fixture — the fixture has no Cargo.toml, and these legs need a cargo
     graph) the cursor sits at the end of `ropey` in the top-level
     `ropey::Rope::new()` probe. M-. misses the project symbol index and
     the rust provider resolves via `cargo metadata` (the registry source
     is cached, so it is fast and offline) → lands READ-ONLY in
     ~/.cargo/registry/src/<hash>/ropey-1.6.1/src/rope.rs.
  L2/L3 `indexing crate …` indicator: the landing registers the crate's
     source_root for a background crate index build (off the input path,
     published on the CrateIndexBus). The status line shows the indicator
     while the build is in flight and it clears when the index lands.
  L4 M-. WITHIN the crate: after the indicator clears, M-. on
     `RopeBuilder` (defined in rope_builder.rs, used in rope.rs) jumps
     CROSS-FILE inside the crate — the jump message is CRATE-RELATIVE
     (`src/rope_builder.rs:…`, not the absolute registry path) and the
     landing buffer is read-only.
  L5 imenu on the external buffer: M-i lists the current file's symbols
     from the crate index (not a refusal).
  L6/L7 M-, walks the jump stack back: first to the in-crate origin
     (rope.rs), then to the project file (src/main.rs).

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
STATUS = ROWS - 1    # status-line row (0-based)

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


def poll_minibuffer(app, needle, timeout=60.0):
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
    if not os.path.exists(ROPE_RS):
        print(f"FAIL ropey registry source missing at {ROPE_RS}")
        sys.exit(1)
    setup_repo()
    app = None
    indicator_seen = False
    try:
        app = App(REPO, rows=ROWS, cols=COLS)

        # ── L1: M-. into the ropey registry source (read-only external) ─
        print("=== L1: M-. into the ropey registry source ===")
        open_main(app)
        app.key("M-g g", 0.8)
        app.key("5", 0.4)
        app.key("RET", 0.8)   # line 5: top-level `ropey::Rope::new();` probe
        app.key("M-f", 0.8)   # point to the END of the `ropey` run
        # From here the status line is polled continuously with FAST reads:
        # the `indexing crate` indicator exists only between the landing and
        # the build's final event, and the build can finish in well under a
        # second on a warm page cache — a coarse poll would miss it.
        app.key("M-.", 0.5)
        ok = False
        landed, landed_line = None, 0
        deadline = time.time() + 60.0
        msg = ""
        while time.time() < deadline:
            app._read(0.03, quiet=0.02)
            if "indexing crate" in app.row_text(STATUS):
                indicator_seen = True
            if "jumped to" in app.row_text(MINI):
                ok = True
                msg = app.row_text(MINI)
                break
        m = re.search(r"jumped to (\S+):(\d+)", msg)
        if m:
            landed, landed_line = m.group(1), int(m.group(2))
        rec("L1: M-. reports the jump", ok and landed is not None,
            f"minibuffer={msg!r}")
        rec("L1: the landing path is a registry source (absolute, external)",
            bool(landed) and landed.startswith(REGISTRY_SRC_ROOT)
            and "ropey-1.6.1/src/rope.rs" in landed,
            f"landed={landed!r}")
        rec("L1: the window landed on rope.rs (Rope in view)",
            "Rope" in app.screen_text(),
            f"top={app.row_text(1)!r}")

        # ── L2/L3: the `indexing crate …` indicator appears and clears ──
        print("\n=== L2/L3: indexing crate indicator ===")
        rec("L2: the `indexing crate` indicator appears in the status line",
            indicator_seen,
            "polled continuously from the M-. press")
        # Wait for the build to finish (indicator cleared), still watching
        # closely enough that a fast build cannot slip past the check.
        deadline = time.time() + 90.0
        cleared = False
        while time.time() < deadline:
            app._read(0.03, quiet=0.02)
            status = app.row_text(STATUS)
            if "indexing crate" in status:
                indicator_seen = True
            if "indexing crate" not in status:
                cleared = True
                break
        rec("L3: the `indexing crate` indicator clears (index ready)",
            cleared, f"status={app.row_text(STATUS)!r}")

        # ── L4: M-. WITHIN the crate (cross-file, crate-relative) ──────
        print("\n=== L4: M-. on RopeBuilder jumps within ropey ===")
        # rope.rs line 104: "        RopeBuilder::new().build_at_once(text);"
        # M-f from col 0 lands at the END of the `RopeBuilder` run.
        app.key("M-g g", 0.8)
        app.key("1", 0.35)
        app.key("0", 0.35)
        app.key("4", 0.35)
        app.key("RET", 0.8)
        app.key("M-f", 0.8)
        app.key("M-.", 0.5)
        ok, msg = poll_minibuffer(app, "jumped to", timeout=60.0)
        # The jump message is CRATE-RELATIVE (the crate index keys files
        # against the crate root) — not the absolute registry path the
        # resolver landing reports.
        rec("L4: M-. on RopeBuilder reports a CRATE-RELATIVE jump",
            bool(ok) and "jumped to src/rope_builder.rs:" in msg,
            f"minibuffer={msg!r}")
        rec("L4: the landing shows the RopeBuilder struct",
            "pub struct RopeBuilder" in app.screen_text(),
            f"top={app.row_text(1)!r}")
        rec("L4: the current buffer is the crate's rope_builder.rs",
            "rope_builder.rs" in app.row_text(STATUS),
            f"status={app.row_text(STATUS)!r}")

        # ── L5: imenu on the external buffer ───────────────────────────
        print("\n=== L5: M-i lists the crate file's symbols ===")
        app.key("M-i", 1.0)
        screen = app.screen_text()
        rec("L5: imenu opens on the external buffer", "Imenu:" in screen,
            f"prompt row={app.row_text(MINI)!r}")
        rec("L5: the outline lists RopeBuilder (crate index, not a refusal)",
            "RopeBuilder" in screen and "not in project" not in screen,
            "no refusal message")
        app.key("C-g", 0.8)   # cancel the picker

        # ── L6/L7: M-, walks the jump stack back ───────────────────────
        print("\n=== L6/L7: M-, back through the jump stack ===")
        app.key("M-,", 1.0)
        rec("L6: M-, returns to the in-crate origin (src/rope.rs)",
            "src/rope.rs" in app.row_text(STATUS),
            f"status={app.row_text(STATUS)!r}")
        app.key("M-,", 1.0)
        rec("L7: M-, returns to the project file (src/main.rs)",
            "src/main.rs" in app.row_text(STATUS),
            f"status={app.row_text(STATUS)!r}")
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

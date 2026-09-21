#!/usr/bin/env python3
"""External-buffer annotation thin tier (loop-04): the registry-landing +
ownership-guard smoke.

Two of the original sections stay PTY (only a live app proves them):
  E1  M-. lands external: in a dedicated repo (NOT the shared fixture — the
      fixture has no Cargo.toml, and these legs need a cargo graph) the
      cursor sits at the end of `ropey` in `ropey::Rope::new()`. M-. misses
      the project symbol index and the rust provider resolves via
      `cargo metadata` (the registry source is cached, so it is fast and
      offline) → (jump-ambiguity: the tooling hit joins the Xref picker as
      the `tooling`-marked row, never a silent jump; RET accepts the top
      guess) → lands READ-ONLY in
      ~/.cargo/registry/src/<hash>/ropey-1.6.1/src/rope.rs.
  E1b ownership guard (006-02b item 1): C-x C-q AND C-x C-s are REFUSED on
      the external buffer ("external buffer is read-only (not
      project-owned)") — the deliberate edit-mode override must never turn
      a registry source (a cache shared by every project on the machine)
      editable or writable.

The annotation STATE on the external buffer is now store-level (the 008
machinery is store-level; unit twins in src/app/flow_tests.rs, loop-04):
  E2 annotate (marker + note row, absolute-path record key)
                                       -> unit_flow_ext_notes_annotate
  E3 delete (d removes the record)     -> unit_flow_ext_notes_delete
  E4 quit dump carries the path verbatim
                                       -> unit_flow_ext_notes_dump
(the dump's terminal-tier half — the pipe bytes, the exit behavior — stays
with probe_notes_dump.py's lifecycle family).

The drive owns its own repo dir (/tmp/redline_ext_repo) under the shared
PTY-flock scheme and removes the repo on exit. Wrap the invocation in
`timeout`.

Exit 0 = all legs pass; 1 = any failed.
"""
import os, re, sys, shutil
import subprocess
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo

BIN = os.environ.get("REDLINE_BIN",
                     os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                  "..", "target", "debug", "redline"))
REPO = repo("redline_ext_repo")
ROPEY_SRC = os.path.expanduser(
    "~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ropey-1.6.1/src/rope.rs")
REGISTRY_SRC_ROOT = os.path.expanduser("~/.cargo/registry/src")
COLS, ROWS = int(os.environ.get("EXT_NOTES_COLS", "200")), 24
# 200 cols: the M-. landing message carries the FULL absolute registry path
# (90+ chars); the default 80-col PTY would clip it and break the parse.
MINI = 22  # minibuffer row (1-based terminal row 23)

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
        f.write('[package]\nname = "ext_notes"\nversion = "0.1.0"\n'
                'edition = "2021"\n\n[dependencies]\n'
                'extrope = { package = "ropey", version = "1.6.1" }\n')
    with open(os.path.join(REPO, "src", "main.rs"), "w") as f:
        # The probe call sits at TOP LEVEL (line 5): M-. selection falls
        # through to the tooling resolver only when the point has no
        # in-project definition AND no enclosing symbol — a call inside
        # `fn main` would resolve to `main` via the enclosing-symbol
        # fallback. (drive_xref's fixture does the same with a bare
        # top-level call.)
        f.write("fn main() {\n"
                "    let r = ropey::Rope::new();\n"
                "    let _ = &r;\n"
                "}\n"
                "ropey::Rope::new();\n")
    subprocess.run(["git", "init", "-q", REPO], check=True)
    subprocess.run(["git", "-C", REPO, "add", "-A"], check=True,
                   capture_output=True)
    subprocess.run(["git", "-C", REPO, "commit", "-qm", "ext notes leg"],
                   check=True, capture_output=True)


def open_main(app):
    app.key("C-x C-f", 1.0)
    for ch in "main":
        app.key(ch, 0.25)
    app.key("RET", 1.2)


def poll_screen(app, needle, timeout=30.0):
    """Wait until `needle` appears in the main screen (the Xref picker's
    prompt row is the top of the main area — the picker is a canvas,
    jump-ambiguity: the tooling hit joins it, never a silent jump)."""
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        app.wait(0.4)
        last = app.screen_text()
        if needle in last:
            return True, last
    return False, last


def poll(app, needle, timeout=30.0):
    """Wait until `needle` appears in the minibuffer row."""
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        app.wait(0.4)
        last = app.row_text(MINI)
        if needle in last:
            return True, last
    return False, last



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
    print(f"BIN={BIN}\nREPO={REPO}\n")
    if not os.path.exists(ROPEY_SRC):
        print(f"FAIL ropey registry source missing at {ROPEY_SRC}")
        sys.exit(1)
    setup_repo()
    prewarm_cargo()
    app = None
    try:
        app = App(REPO, rows=ROWS, cols=COLS)

        # ── E1: M-. lands in the registry source (read-only external) ───
        print("=== E1: M-. into the ropey registry source ===")
        open_main(app)
        app.key("M-g g", 0.8)
        app.key("5", 0.4)
        app.key("RET", 0.8)   # line 5: top-level `ropey::Rope::new();` probe call
        app.key("M-f", 0.8)   # point to the END of the `ropey` run on the line-5 probe
        app.key("M-.", 1.5)
        # (jump-ambiguity) the tooling hit joins the Xref picker as the
        # `tooling`-marked row (the weak resolve is visible, not a
        # mystery): wait for the picker prompt, verify the marker, then
        # accept the preselected row.
        ok, msg = poll_screen(app, "Definition:", timeout=40.0)
        marked = "tooling" in app.screen_text()
        rec("E1: the tooling hit joins the Xref picker (marked row)",
            ok and marked, f"prompt={ok} marker={marked}")
        app.key("RET", 1.5)
        ok, msg = poll(app, "jumped to", timeout=15.0)
        m = re.search(r"jumped to (\S+):(\d+)", msg)
        landed, landed_line = None, 0
        if m:
            landed, landed_line = m.group(1), int(m.group(2))
        rec("E1: M-. reports the jump", ok and landed is not None,
            f"minibuffer={msg!r}")
        rec("E1: the landing path is a registry source (absolute, external)",
            bool(landed) and landed.startswith(REGISTRY_SRC_ROOT)
            and "ropey-1.6.1/src/rope.rs" in landed,
            f"landed={landed!r}")
        ok = "Rope" in app.screen_text()
        rec("E1: the window landed on rope.rs (Rope in view)", ok,
            f"top={app.row_text(1)!r}")

        # ── E1b: the ownership guard (006-02b item 1) ─────────────────
        # The registry source is a cache shared by every project on the
        # machine: the C-x C-q edit-mode override and C-x C-s must both be
        # refused on it.
        print("\n=== E1b: C-x C-q / C-x C-s refused on the external buffer ===")
        app.key("C-x C-q", 1.0)
        ok, msg = poll(app, "external buffer is read-only", timeout=10.0)
        rec("E1b: C-x C-q is refused on the external buffer",
            ok, f"minibuffer={msg!r}")
        app.key("C-x C-s", 1.0)
        ok, msg = poll(app, "external buffer is read-only", timeout=10.0)
        rec("E1b: C-x C-s is refused on the external buffer",
            ok, f"minibuffer={msg!r}")
    finally:
        if app is not None:
            app.kill()
        shutil.rmtree(REPO, ignore_errors=True)
    bad = [n for n, ok, _ in CHECKS if not ok]
    print("\n=== SUMMARY ===")
    for n, ok, _ in CHECKS:
        print(f"  {'PASS' if ok else 'FAIL'}  {n}")
    print(f"\n{len(CHECKS) - len(bad)}/{len(CHECKS)} external-notes legs passed")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

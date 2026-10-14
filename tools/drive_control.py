#!/usr/bin/env python3
"""Plan 019 issue 02 — drive the LIVE TUI over the control socket (PTY leg).

Launches `redline --socket <tmpsock>` in a sized PTY (its own fixture repo),
then drives the SAME live session from a SECOND process over the unix control
socket:

  * the TUI's stderr prints `redline: control socket at <path>` (asserted from
    the PTY's raw byte stream — the message lands before the alternate screen
    and the TUI clears it, but the bytes remain in the driver's raw tail);
  * `redline --socket <tmpsock> -c 'do find-file' -c 'text Cargo.toml'
    -c 'key RET' quote` connects, opens Cargo.toml, and prints the live frame
    — asserted to name Cargo.toml;
  * `redline --socket <tmpsock> quit` ends the TUI (the PTY child exits).

Own fixture repo (redline_control_repo) + own per-repo PTY flock, so it never
contends with the other suites. Registered in tools/gate.sh SHARED_SUITES and
tools/pool.py BATTERY.

Exit 0 = all checks pass; 1 = any failed.
"""
import os
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App, BIN
from fixture import repo, fixture_root

REPO = repo("redline_control_repo")   # drive-owned root (removed on exit)
SOCK = os.path.join(fixture_root(), "redline-ctl-%d.sock" % os.getpid())

CHECKS = []


def rec(name, ok, detail=""):
    CHECKS.append((name, ok, detail))
    print(f"  {'PASS' if ok else 'FAIL'}  {name:56s} {detail}")
    return ok


def setup_repo():
    if os.path.exists(REPO):
        shutil.rmtree(REPO)
    os.makedirs(REPO)
    with open(os.path.join(REPO, "Cargo.toml"), "w") as f:
        f.write('[package]\nname = "control-fixture"\nversion = "0.1.0"\n')
    with open(os.path.join(REPO, "README.md"), "w") as f:
        f.write("# control fixture\n")
    subprocess.run(["git", "init", "-q", REPO], check=True)
    subprocess.run(["git", "-C", REPO, "add", "-A"], check=True,
                   capture_output=True)
    subprocess.run(["git", "-C", REPO, "commit", "-qm", "control fixture"],
                   check=True, capture_output=True)


def wait_socket(timeout=30.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if os.path.exists(SOCK):
            return True
        time.sleep(0.1)
    return False


def run_client(*args):
    """Run the redline client against the live socket; return (rc, out, err)."""
    cp = subprocess.run(
        [BIN, "--socket", SOCK, *args],
        cwd=REPO, capture_output=True, text=True, timeout=60)
    return cp.returncode, cp.stdout, cp.stderr


def main():
    for stray in (SOCK,):
        try:
            os.remove(stray)
        except FileNotFoundError:
            pass
    setup_repo()

    app = App(REPO, rows=24, cols=80, args=["--socket", SOCK])
    try:
        # The TUI printed the bound path to stderr (captured in the raw
        # stream even though the alternate screen later overwrote the text).
        rec("tui prints control socket path",
            b"control socket at" in app.raw_tail,
            "stderr line seen" if b"control socket at" in app.raw_tail else "missing")

        if not rec("control socket bound", wait_socket(), SOCK):
            return 1

        # Drive the live session over the wire: open Cargo.toml, print a frame.
        rc, out, err = run_client(
            "-c", "do find-file", "-c", "text Cargo.toml", "-c", "key RET",
            "quote")
        ok = rc == 0 and "Cargo.toml" in out
        rec("remote find-file + quote shows Cargo.toml", ok,
            f"rc={rc} has={('Cargo.toml' in out)} err={err.strip()[:80]}")
        if not ok:
            print("--- client stdout tail ---")
            print("\n".join(out.splitlines()[-8:]))

        # A remote quit ends the live TUI.
        rc, out, err = run_client("quit")
        rec("remote quit accepted", rc == 0, f"rc={rc} err={err.strip()[:80]}")

        exited = False
        deadline = time.time() + 10.0
        while time.time() < deadline:
            pid, status = os.waitpid(app.pid, os.WNOHANG)
            if pid == app.pid:
                exited = True
                break
            time.sleep(0.2)
        rec("tui process exits after remote quit", exited)
    finally:
        try:
            app.kill()
        except Exception:
            pass
        try:
            os.remove(SOCK)
        except FileNotFoundError:
            pass
        shutil.rmtree(REPO, ignore_errors=True)

    passed = sum(1 for _, ok, _ in CHECKS if ok)
    print(f"\ncontrol drive: {passed}/{len(CHECKS)} checks passed")
    return 0 if passed == len(CHECKS) else 1


if __name__ == "__main__":
    sys.exit(main())

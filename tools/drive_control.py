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
  * W14: the SAME mutation is proven on the live PYTE SCREEN (title row +
    status line name the file), not just the client's `quote`;
  * a leg per sugar verb (open/goto/search/select+confirm/annotate/magit)
    asserts each real store op;
  * `redline --socket <tmpsock> quit` ends the TUI (the PTY child exits) and
    W16/W17: the socket file is gone and `quit` printed no state block.

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


# W22: the DECISION the human/agent actually makes is *no* `--socket` — the
# client must DISCOVER the app's socket via `$XDG_RUNTIME_DIR/redline-<real
# uid>.sock`. The rest of this drive always passes an explicit `--socket`, so
# that default path was never exercised end to end and a broken discovery
# (the W22 bug: `UID` is a bash builtin, absent from the env, so both sides
# fell back to their own pid) shipped silently. This leg points the app and a
# bare client at a SCRATCH runtime dir so discovery is tested without
# touching the user's real session.
RUNTIME = os.path.join(fixture_root(), "ctl-rt-%d" % os.getpid())
DISCOVERY_SOCK = os.path.join(RUNTIME, "redline-1000.sock")  # real uid below


def run_bare_client(runtime, *args):
    """Run the client with NO --socket, against a scratch XDG_RUNTIME_DIR."""
    env = dict(os.environ)
    env["XDG_RUNTIME_DIR"] = runtime
    cp = subprocess.run(
        [BIN, *args],
        cwd=REPO, capture_output=True, text=True, timeout=60, env=env)
    return cp.returncode, cp.stdout, cp.stderr


def discover_leg():
    """W22: a bare `redline <verb>` (no --socket) discovers a running app."""
    os.makedirs(RUNTIME, exist_ok=True)
    # The app name comes from the REAL uid (read from /proc/self/status);
    # compute the same token here so the test knows the expected path.
    uid = "1000"
    try:
        with open("/proc/self/status") as f:
            for line in f:
                if line.startswith("Uid:"):
                    uid = line.split()[1]
                    break
    except OSError:
        pass
    want = os.path.join(RUNTIME, "redline-%s.sock" % uid)

    app = App(REPO, rows=24, cols=80, env_extra={"XDG_RUNTIME_DIR": RUNTIME})
    try:
        bound = False
        deadline = time.time() + 30.0
        while time.time() < deadline:
            if os.path.exists(want):
                bound = True
                break
            time.sleep(0.1)
        if not rec("default-path: app binds $XDG_RUNTIME_DIR/redline-<uid>.sock",
                   bound, want):
            return
        # A BARE client (no --socket) must discover and drive it.
        rc, out, err = run_bare_client(RUNTIME, "-c", "open Cargo.toml:2", "state")
        ok = rc == 0 and "view_name: Cargo.toml" in out and "point.line: 1" in out
        rec("default-path: bare `redline` (no --socket) discovers + drives it",
            ok, f"rc={rc} err={err.strip()[:70]}")
        rc2, out2, err2 = run_bare_client(RUNTIME, "quit")
        rec("default-path: bare `redline quit` accepted", rc2 == 0,
            f"rc={rc2} err={err2.strip()[:70]}")
    finally:
        try:
            app.kill()
        except Exception:
            pass
        shutil.rmtree(RUNTIME, ignore_errors=True)


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

        # W14: the STRONG claim — the human's live PYTE SCREEN moved, not just
        # a client-side `quote` read of store state. Read the PTY (the repaint
        # lands asynchronously) and assert the file's CONTENT row (W13: the
        # file view has no title row — row 0 is the first code line) AND the
        # status line names the file. This is the assertion that FAILS if the
        # repaint breaks.
        app._read(2.0, quiet=0.3)
        content_row = app.row_text(0)
        status_row = app.row_text(app.rows - 1)
        screen_ok = "[package]" in content_row and "Cargo.toml" in status_row
        rec("live PYTE screen repaints (content + status name the file)", screen_ok,
            f"content={content_row.strip()!r} status={status_row.strip()[:60]!r}")

        # ── sugar verbs, one leg each (019-03) ──────────────────────────

        # open PATH:LINE — lands the point, asserted via `state` + `quote`.
        rc, out, err = run_client("-c", "open Cargo.toml:3", "state")
        ok = rc == 0 and "point.line: 2" in out
        rec("sugar open Cargo.toml:3 lands line 2", ok,
            f"rc={rc} point={'point.line: 2' in out} err={err.strip()[:60]}")

        # goto LINE.
        rc, out, err = run_client("-c", "open Cargo.toml", "-c", "goto 1", "state")
        rec("sugar goto 1 lands line 0", rc == 0 and "point.line: 0" in out,
            f"rc={rc} err={err.strip()[:60]}")

        # search QUERY — the real project search op. Run this leg LAST among
        # the mutations: it pushes the Search view on top, which would block
        # the Buffer-only legs (annotate) that follow.

        # select + confirm on a picker (find-file lists the fixture files).
        rc, out, err = run_client(
            "-c", "do find-file", "-c", "select 1", "-c", "confirm",
            "state")
        ok = rc == 0 and "view: Buffer" in out
        rec("sugar select + confirm opens a picker row", ok,
            f"rc={rc} buffer={'view: Buffer' in out} err={err.strip()[:60]}")

        # annotate TEXT at point.
        rc, out, err = run_client(
            "-c", "open Cargo.toml:1", "-c", "annotate a drive note", "state")
        ok = rc == 0 and "note" in out
        rec("sugar annotate writes a note", ok,
            f"rc={rc} annotated={'note' in out} err={err.strip()[:60]}")

        # magit status.
        rc, out, err = run_client("-c", "magit", "state")
        ok = rc == 0 and "view: MagitStatus" in out
        rec("sugar magit opens the status view", ok,
            f"rc={rc} magit={'view: MagitStatus' in out} err={err.strip()[:60]}")

        # search QUERY — the real project search op.
        rc, out, err = run_client("-c", "search control-fixture", "state")
        ok = rc == 0 and "search.title: Search: 'control-fixture'" in out
        rec("sugar search starts a project search", ok,
            f"rc={rc} title={'search.title:' in out} err={err.strip()[:60]}")

        # A remote quit ends the live TUI (and W16: the socket file is gone).
        rc, out, err = run_client("quit")
        rec("remote quit accepted", rc == 0, f"rc={rc} err={err.strip()[:80]}")
        # W17: quit must NOT dump state.
        rec("remote quit prints no state block", "view:" not in out,
            f"stdout={out.strip()[:60]!r}")

        exited = False
        deadline = time.time() + 10.0
        while time.time() < deadline:
            pid, status = os.waitpid(app.pid, os.WNOHANG)
            if pid == app.pid:
                exited = True
                break
            time.sleep(0.2)
        rec("tui process exits after remote quit", exited)
        # W16: the socket file is unlinked on clean exit (the child exited
        # above, so give the unlink a moment).
        gone = False
        deadline = time.time() + 5.0
        while time.time() < deadline:
            if not os.path.exists(SOCK):
                gone = True
                break
            time.sleep(0.1)
        rec("socket file removed on clean exit", gone, SOCK)

        # W22: the DEFAULT discovery path (no --socket) — its own scratch
        # runtime dir, so it never touches the user's real session.
        discover_leg()
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

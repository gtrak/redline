"""PTY + pyte driver for the redline TUI binary.

Spawns `redline` in a sized PTY (cwd = repo), feeds keystrokes, and
reconstructs the screen with pyte so we can inspect per-cell fg/bg/reverse.

The selected-row cursor bar is a per-row View with the selected face's
background. For the default (dark) theme that is White-on-Blue: iocraft
emits fg `38;5;15` -> pyte `ffffff`. The bar background is the theme's
bright blue: as 256-color `48;5;12` -> pyte `5c5cff`, or (plan-004 issue 05,
when `COLORTERM=truecolor`) as truecolor `48;2;0;0;255` -> pyte `0000ff`.
So the discriminating signature of the cursor row is a bg in `BAR_BGS`.
"""
import os, pty, fcntl, termios, struct, time, select, signal, sys

import pyte

BIN = os.environ.get("REDLINE_BIN", "/home/gary/dev/red/target/debug/redline")
COLS = int(os.environ.get("COLS", "80"))
ROWS = int(os.environ.get("ROWS", "24"))

# ── Read-quiet tuning (harness latency) ─────────────────────────────────
# `_read` returns once the PTY byte stream has been SILENT for `quiet`
# seconds. The app paints a complete frame in ~15 ms (measured: first byte
# 0 ms, last byte 15 ms), so the old hardcoded 0.2 s was ~13x the real
# work — the single largest tax on every PTY suite (559 .key() + 212
# .wait() call sites; ~250 s of pure slack across the gate set).
#
# Default stays 0.2 (byte-for-byte the old behavior) so nothing silently
# changes semantics; set REDLINE_PTY_QUIET=0.04..0.08 for the fast loop.
# The select granularity is decoupled from `quiet` (below) so shrinking
# `quiet` actually takes effect instead of being floored at 100 ms.
PTY_QUIET = float(os.environ.get("REDLINE_PTY_QUIET", "0.2"))
# Max select() block. Must be <= the quiet window or the quiet check can
# only fire after a full granularity tick (the old 0.1 s cap is exactly
# why a smaller `quiet` alone would NOT have helped).
_POLL_GRAN = 0.01

# ── Shared-fixture mutual exclusion ──────────────────────────────────────
# Every PTY suite drives the SAME fixture repo (/tmp/redline_pyte_repo) and
# several of them mutate it (flow legs create/save/delete files, magit
# stages things). Two suites running at once therefore corrupt each other
# and produce PHANTOM failures — observed live on 2026-09-18: an
# independent `sweep.py` gave 10/14 while the worker's own run gave 13/14
# purely because both were driving the fixture concurrently.
#
# Fail fast instead: take an exclusive flock on a lock file keyed to the
# repo path when an App starts, and release it on kill(). A second suite
# aborts with a clear message rather than silently producing bad results.
# Set REDLINE_NO_PTY_LOCK=1 to opt out (e.g. a deliberate two-fixture run
# using REDLINE_REPO).
_LOCK_FD = None

def _acquire_fixture_lock(repo):
    global _LOCK_FD
    if os.environ.get("REDLINE_NO_PTY_LOCK"):
        return
    if _LOCK_FD is not None:
        return
    import fcntl as _f
    import hashlib as _h
    # NOTE: do NOT use the builtin hash() — string hashing is randomized per
    # process (PYTHONHASHSEED), so two runs would compute different lock
    # paths and never contend.
    digest = _h.sha1(os.path.abspath(repo).encode()).hexdigest()[:16]
    path = "/tmp/redline_pty_%s.lock" % digest
    fd = os.open(path, os.O_CREAT | os.O_RDWR, 0o644)
    try:
        _f.flock(fd, _f.LOCK_EX | _f.LOCK_NB)
    except OSError:
        os.close(fd)
        sys.stderr.write(
            "\n*** shared PTY fixture is busy (another suite is running).\n"
            "*** Refusing to start to avoid phantom failures.\n"
            "*** Wait for it to finish, or set REDLINE_NO_PTY_LOCK=1.\n\n")
        raise SystemExit(3)
    os.write(fd, str(os.getpid()).encode())
    _LOCK_FD = fd

def _release_fixture_lock():
    global _LOCK_FD
    if _LOCK_FD is not None:
        try:
            os.close(_LOCK_FD)
        finally:
            _LOCK_FD = None

import atexit as _atexit
_atexit.register(_release_fixture_lock)

# pyte's rendering of the selected-row bar background, per color mode:
#   256-color (default / no COLORTERM): 48;5;12  -> '5c5cff'
#   truecolor (COLORTERM=truecolor):    48;2;0;0;255 -> '0000ff'
BLUE_BG = "5c5cff"  # legacy 256-color signature (kept for reference)
BAR_BGS = {"5c5cff", "0000ff"}


def _set_winsize(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def encode_key(seq):
    """Encode an emacs-notation key sequence into raw terminal bytes.

    Tokens are space-separated. Recognised: `C-<c>` (ctrl), `M-<c>` (alt),
    `down`/`up`/`right`/`left` (arrows), `RET`, `ESC`, `TAB`, `space`, and
    any other single character (sent literally).
    """
    out = b""
    for tok in seq.split():
        if tok.upper() == "DOWN":
            out += b"\x1b[B"
        elif tok.upper() == "UP":
            out += b"\x1b[A"
        elif tok.upper() == "RIGHT":
            out += b"\x1b[C"
        elif tok.upper() == "LEFT":
            out += b"\x1b[D"
        elif tok.upper() == "RET":
            out += b"\r"
        elif tok.upper() == "ESC":
            out += b"\x1b"
        elif tok.upper() == "TAB":
            out += b"\t"
        elif tok.upper() == "SPACE":
            out += b" "
        elif tok.startswith("C-") and len(tok) == 3:
            out += bytes([ord(tok[2].lower()) & 0x1F])
        elif tok.startswith("M-") and len(tok) == 3:
            out += b"\x1b" + tok[2].encode()
        else:
            out += tok.encode()
    return out


class App:
    def __init__(self, repo, rows=ROWS, cols=COLS, startup_wait=6.0, colorterm=None):
        # The whole suite process owns the shared fixture (released at exit),
        # not just one App: a suite that released between Apps would still
        # interleave with a rival.
        _acquire_fixture_lock(repo)
        self.repo = repo
        self.rows = rows
        self.cols = cols
        self.colorterm = colorterm
        master, slave = pty.openpty()
        _set_winsize(slave, rows, cols)
        _set_winsize(master, rows, cols)
        self.pid = os.fork()
        if self.pid == 0:
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
            os.dup2(slave, 0)
            os.dup2(slave, 1)
            os.dup2(slave, 2)
            if slave > 2:
                os.close(slave)
            os.close(master)
            env = dict(os.environ)
            env["TERM"] = "xterm-256color"
            env["RUST_LOG"] = "info"
            # Deterministic bar color for the pyte drives: default to the
            # 256-color path (pop any ambient COLORTERM); pass colorterm="truecolor"
            # to exercise the truecolor bar (plan-004 issue 05).
            if colorterm is None:
                env.pop("COLORTERM", None)
            else:
                env["COLORTERM"] = colorterm
            os.chdir(repo)
            os.execvpe(BIN, [BIN], env)
        os.close(slave)
        self.master = master
        self.screen = pyte.Screen(cols, rows)
        self.stream = pyte.ByteStream(self.screen)
        # Wait for the app to fully start (config + index + watcher) and the
        # root buffer to render. Poll until the mode line reads `ready`
        # (the startup symbol-index build clears the screen while it runs).
        self.wait_ready(30.0)

    def wait_ready(self, timeout=30.0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            self._read(0.4, quiet=PTY_QUIET)
            text = self.screen_text()
            # `ready` is the idle mode-line token; `indexing` means still busy.
            if "ready" in text and "indexing" not in text:
                return True
        return False

    def _read(self, timeout, quiet=0.0):
        deadline = time.time() + timeout
        last = time.time()
        # Tighter poll than the quiet window so the idle check can fire on
        # time (a coarse select() blocks the whole window before the check
        # even runs). 10 ms is well under the app's 15 ms frame time.
        gran = _POLL_GRAN if quiet > 0 else 0.1
        while True:
            remain = deadline - time.time()
            if remain <= 0:
                break
            r, _, _ = select.select([self.master], [], [], min(remain, gran))
            if r:
                try:
                    data = os.read(self.master, 65536)
                except OSError:
                    break
                if not data:
                    break
                self.stream.feed(data)
                last = time.time()
            if quiet > 0 and (time.time() - last) >= quiet:
                break

    def key(self, s, settle=1.0, quiet=None):
        """Send a key sequence and read until the stream is quiet.

        A multi-token sequence (`"C-x C-f"`) is written as ONE burst and
        read once — it is already batched. `quiet` overrides the
        REDLINE_PTY_QUIET default for this call only.
        """
        os.write(self.master, encode_key(s))
        self._read(settle, quiet=PTY_QUIET if quiet is None else quiet)

    def type_text(self, text, settle=1.0, quiet=None):
        """Type a literal string as ONE burst + ONE read.

        Replaces the `for ch in text: app.key(ch)` pattern, which pays a
        full read-quiet window PER CHARACTER (measured 6.0x slower for a
        6-char query). The app processes the whole burst in a single
        render, so one read is both faster and equivalent.
        """
        os.write(self.master, text.encode())
        self._read(settle, quiet=PTY_QUIET if quiet is None else quiet)

    def feed(self, data, settle=1.0, quiet=None):
        os.write(self.master, data)
        self._read(settle, quiet=PTY_QUIET if quiet is None else quiet)

    def wait(self, t=1.0, quiet=None):
        self._read(t, quiet=PTY_QUIET if quiet is None else quiet)

    def wait_done(self, timeout=15.0):
        """Wait until a streamed search has finished: the title stops showing
        'searching' and a match count is present."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            self._read(0.4, quiet=PTY_QUIET)
            text = self.screen_text()
            if "searching" not in text and "matches in" in text:
                return True
        return False

    def row_text(self, row):
        return "".join(self.screen.buffer[row][i].data for i in range(self.cols))

    def screen_text(self):
        return "\n".join(self.row_text(r) for r in range(self.rows))

    def blue_rows(self, include_status_line=False):
        """Content rows containing at least one cell with the blue selected-bg.

        The active-buffer status line (bottom row) also uses the white-on-blue
        face, so it is excluded from the cursor-bar count by default.
        """
        upper = self.rows if include_status_line else self.rows - 1
        out = []
        for r in range(upper):
            row = self.screen.buffer[r]
            for i in range(self.cols):
                if str(row[i].bg).lower() in BAR_BGS:
                    out.append(r)
                    break
        return out

    def blue_count(self, row):
        """Number of bar-bg cells in a row (256-color or truecolor)."""
        return sum(1 for i in range(self.cols)
                   if str(self.screen.buffer[row][i].bg).lower() in BAR_BGS)

    def reverse_rows(self):
        """Rows containing any reversed cell (sanity: the fix must use NO reverse)."""
        out = []
        for r in range(self.rows):
            row = self.screen.buffer[r]
            for i in range(self.cols):
                if row[i].reverse:
                    out.append(r)
                    break
        return out

    def kill(self):
        try:
            os.kill(self.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            os.waitpid(self.pid, 0)
        except ChildProcessError:
            pass
        try:
            os.close(self.master)
        except OSError:
            pass

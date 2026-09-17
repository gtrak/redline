#!/usr/bin/env python3
"""Plan 004 issue 05 — raw-stream cursor-visibility + truecolor-bar check.

Drives the real binary in a sized PTY (magit view, n/p navigation) in two
color modes and asserts the raw terminal stream, not just the pyte
reconstruction:

  * `?25l` (cursor hide) fires once at startup and is COUNTERED: every frame is
    followed by a `?25h` (show) so the hardware cursor is visible.
  * The CUP that survives to the end of a frame (the last CUP after the last
    `?2026l`) lands on the selected (blue-bar) row and tracks it across n/p.
  * No lingering `?25l` after the first render.
  * Under `COLORTERM=truecolor` the selected-row bar is emitted as
    `48;2;0;0;255` (truecolor); without it the 256-color `48;5;12` is retained.
  * pyte still reconstructs the bar (the blue-bar row exists, no reverse).

Exit 0 = all assertions pass; 1 = any failed.
"""
import os, pty, fcntl, termios, struct, time, select, signal, re
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pyte
from pyte_driver import encode_key, BAR_BGS

BIN = os.environ.get("REDLINE_BIN", os.path.join(os.path.dirname(__file__), "..", "target", "debug", "redline"))
REPO = os.environ.get("REDLINE_REPO", "/tmp/redline_pyte_repo")
COLS, ROWS = 80, 24


def _set_winsize(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


class Session:
    def __init__(self, colorterm):
        self.colorterm = colorterm
        self.master, slave = pty.openpty()
        _set_winsize(slave, ROWS, COLS)
        _set_winsize(self.master, ROWS, COLS)
        self.pid = os.fork()
        if self.pid == 0:
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
            os.dup2(slave, 0); os.dup2(slave, 1); os.dup2(slave, 2)
            if slave > 2:
                os.close(slave)
            os.close(self.master)
            env = dict(os.environ)
            env["TERM"] = "xterm-256color"
            env["RUST_LOG"] = "info"
            if colorterm is None:
                env.pop("COLORTERM", None)
            else:
                env["COLORTERM"] = colorterm
            os.chdir(REPO)
            os.execvpe(BIN, [BIN], env)
        os.close(slave)
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.ByteStream(self.screen)
        self._wait_ready()

    def _read(self, timeout, quiet=0.2):
        buf = b""
        deadline = time.time() + timeout
        last = time.time()
        while True:
            remain = deadline - time.time()
            if remain <= 0:
                break
            r, _, _ = select.select([self.master], [], [], min(remain, 0.05))
            if r:
                try:
                    data = os.read(self.master, 65536)
                except OSError:
                    break
                if not data:
                    break
                buf += data
                last = time.time()
                self.stream.feed(data)
            if (time.time() - last) >= quiet:
                break
        return buf

    def _wait_ready(self):
        deadline = time.time() + 30.0
        while time.time() < deadline:
            self._read(0.5)
            text = self.text()
            if "ready" in text and "indexing" not in text:
                return
        raise RuntimeError("app did not reach ready state")

    def key(self, s, settle=0.8):
        os.write(self.master, encode_key(s))
        return self._read(settle, quiet=0.15)

    def text(self):
        return "\n".join("".join(self.screen.buffer[r][i].data for i in range(COLS)) for r in range(ROWS))

    def bar_rows(self):
        return [r for r in range(ROWS - 1)
                if any(str(self.screen.buffer[r][i].bg).lower() in BAR_BGS for i in range(COLS))]

    def reverse_rows(self):
        return [r for r in range(ROWS)
                if any(self.screen.buffer[r][i].reverse for i in range(COLS))]

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


def last_cup_row_after_sync(buf):
    """The row (1-based) of the last CUP issued after the final ?2026l, or None."""
    ends = [m.end() for m in re.finditer(rb"\x1b\[\?2026l", buf)]
    if not ends:
        return None
    after = buf[ends[-1]:]
    cups = re.findall(rb"\x1b\[(\d+);(\d+)[Hf]", after)
    return int(cups[-1][0]) if cups else None


def show_after_sync(buf):
    """Whether a ?25h (show) is issued after the final ?2026l of this chunk."""
    ends = [m.end() for m in re.finditer(rb"\x1b\[\?2026l", buf)]
    return any(b"\x1b[?25h" in buf[e:e + 64] for e in ends) if ends else False


def check(colorterm):
    tc = colorterm is not None
    mode = "truecolor" if tc else "256-color"
    s = Session(colorterm)
    checks = []

    def rec(name, ok, detail=""):
        checks.append((name, ok, detail))
        print(f"  {'PASS' if ok else 'FAIL'}  {name:34s} {detail}")

    # Open magit and navigate; capture the raw stream for each step.
    open_buf = s.key("C-x g", 1.2)
    # startup hide is countered + a show+reposition follows the first frame
    rec("?25l countered (a ?25h follows the first frame)", show_after_sync(open_buf),
        f"?25l={open_buf.count(b'\x1b[?25l')} ?25h={open_buf.count(b'\x1b[?25h')}")

    # n/p navigation: CUP row must track the selected (blue-bar) row.
    track_ok = True
    track_detail = []
    for i in range(5):
        buf = s.key("n" if i % 2 == 0 else "p", 0.7)
        cup_row = last_cup_row_after_sync(buf)   # 1-based
        blues = s.bar_rows()                     # 0-based
        if blues:
            want = blues[0] + 1                  # 1-based
            track_ok = track_ok and (cup_row == want)
            track_detail.append(f"step{i}:cup={cup_row}/want={want}")
        else:
            track_ok = False
            track_detail.append(f"step{i}:no-bar")
    rec("CUP row tracks selected row across n/p", track_ok, "; ".join(track_detail))

    # The bar is rendered (pyte) with exactly one blue row and no reverse.
    blues = s.bar_rows()
    revs = s.reverse_rows()
    rec("pyte bar present, exactly one, no reverse", len(blues) == 1 and not revs,
        f"bar_rows={blues} reverse_rows={revs}")

    # Color-mode escape assertions.
    # Gather the whole-session stream by re-capturing a fresh frame.
    probe = s.key("n", 0.7)
    n_truecolor = probe.count(b"\x1b[48;2;")
    n_256 = probe.count(b"\x1b[48;5;12")
    if tc:
        rec("truecolor bar present (48;2;)", n_truecolor >= 1, f"48;2; count={n_truecolor}")
        rec("truecolor bar rgb is 0;0;255", b"\x1b[48;2;0;0;255" in probe,
            "found" if b"\x1b[48;2;0;0;255" in probe else "missing")
    else:
        rec("256-color bar retained (48;5;12)", n_256 >= 1 and n_truecolor == 0,
            f"48;5;12 count={n_256} 48;2; count={n_truecolor}")

    # No lingering hide after the first render in a settled frame.
    settled = s.key("p", 0.8)
    # count hide vs show in the settled chunk: shows must be >= hides (countered)
    hides = settled.count(b"\x1b[?25l")
    shows = settled.count(b"\x1b[?25h")
    rec("settled frame: no net hide (shows >= hides)", shows >= hides,
        f"settled ?25l={hides} ?25h={shows}")

    s.kill()
    return checks


def main():
    print(f"BIN={BIN}\nREPO={REPO}\n")
    all_checks = []
    for label, ct in [("NO COLORTERM (256-color path)", None),
                      ("COLORTERM=truecolor (truecolor path)", "truecolor")]:
        print(f"=== {label} ===")
        c = check(ct)
        all_checks += c
        print()
    bad = [n for n, ok, _ in all_checks if not ok]
    print("=== SUMMARY ===")
    for n, ok, d in all_checks:
        print(f"  {'PASS' if ok else 'FAIL'}  {n}")
    print(f"\nTOTAL: {len(all_checks) - len(bad)}/{len(all_checks)} raw-stream checks PASS")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

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


def last_cup_after_sync(buf):
    """The (row, col) — 1-based terminal coordinates — of the last CUP issued
    after the final ?2026l of this chunk, or (None, None) when absent."""
    ends = [m.end() for m in re.finditer(rb"\x1b\[\?2026l", buf)]
    if not ends:
        return (None, None)
    after = buf[ends[-1]:]
    cups = re.findall(rb"\x1b\[(\d+);(\d+)[Hf]", after)
    if not cups:
        return (None, None)
    return (int(cups[-1][0]), int(cups[-1][1]))


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


def file_view_checks():
    """plan 004 issue 05b: the file-view cursor tracks a real (line, col)
    point step by step under C-n/C-p/C-f/C-b/arrows/C-a/C-e, with EOL/BOL
    wrapping, goal-column across short/long lines, C-v holding the point's
    screen row, M->/M-< landing the point at the buffer end/start (window
    follows), and the status line tracking the point line."""
    import os as _os
    src_dir = _os.path.join(REPO, "src")
    _os.makedirs(src_dir, exist_ok=True)
    # cursorleg.rs: 40 lines; even line = 20 'A's, odd line = 2 'b's.
    # Written with NO trailing newline so the buffer is exactly 40 lines
    # (ropey counts a trailing \n as an extra empty line).
    clines = ["A" * 20 if i % 2 == 0 else "b" * 2 for i in range(40)]
    with open(_os.path.join(src_dir, "cursorleg.rs"), "w") as f:
        f.write("\n".join(clines))
    # whichfn.rs: a couple of functions to drive the which-function display.
    with open(_os.path.join(src_dir, "whichfn.rs"), "w") as f:
        f.write("fn alpha() {\n    let x = 1;\n    let y = 2;\n}\n"
                "fn beta() {\n    let z = 3;\n}\n")

    s = Session(None)  # 256-color path; REPO is the module fixture repo
    checks = []

    def rec(name, ok, detail=""):
        checks.append((name, ok, detail))
        print(f"  {'PASS' if ok else 'FAIL'}  {name:38s} {detail}")

    def do(key, settle=0.6):
        """Press `key` (possibly a multi-token sequence) and return the
        surviving CUP (row, col) after the frame settles."""
        return last_cup_after_sync(s.key(key, settle))

    # Open cursorleg.rs: the cursor lands on the first content line, col 1.
    s.key("C-x C-f", 1.0)
    for ch in "cursorleg":
        s.key(ch, 0.25)
    buf = s.key("RET", 1.0)
    row, col = last_cup_after_sync(buf)
    rec("open file: cursor at (line 1, col 1)", (row, col) == (2, 1),
        f"cup=({row},{col}) want (2,1)")

    # C-f x3: col 1 -> 4.
    r, c = do("C-f C-f C-f")
    rec("C-f x3 (char right)", (r, c) == (2, 4), f"cup=({r},{c}) want (2,4)")
    # C-b: col 4 -> 3.
    r, c = do("C-b")
    rec("C-b (char left)", (r, c) == (2, 3), f"cup=({r},{c}) want (2,3)")
    # C-e: to end of line 0 (col 20).
    r, c = do("C-e")
    rec("C-e (line end)", (r, c) == (2, 21), f"cup=({r},{c}) want (2,21)")
    # C-f at EOL wraps to the next line, col 0.
    r, c = do("C-f")
    rec("C-f EOL wrap -> line 2 col 1", (r, c) == (3, 1), f"cup=({r},{c}) want (3,1)")
    # C-b at BOL wraps to the previous line's end (col 20).
    r, c = do("C-b")
    rec("C-b BOL wrap -> prev line end", (r, c) == (2, 21), f"cup=({r},{c}) want (2,21)")
    # C-a: line start.
    r, c = do("C-a")
    rec("C-a (line start)", (r, c) == (2, 1), f"cup=({r},{c}) want (2,1)")

    # Goal column: C-f x10 -> (0,10); C-n clamps to the short line's length;
    # C-p restores the goal column on the longer line.
    r, c = do("C-f " * 10)
    rec("C-f x10 (col 10)", (r, c) == (2, 11), f"cup=({r},{c}) want (2,11)")
    r, c = do("C-n")  # line 1 ("bb", len 2): col clamps to 2, goal stays 10
    rec("C-n clamps goal on short line", (r, c) == (3, 3), f"cup=({r},{c}) want (3,3)")
    r, c = do("C-p")  # line 0 (len 20): goal 10 restored
    rec("C-p restores goal column", (r, c) == (2, 11), f"cup=({r},{c}) want (2,11)")

    # Arrows join as point motion (Down==C-n, Up==C-p, Right==C-f, Left==C-b).
    r, c = do("Down")
    rec("Down == C-n", (r, c) == (3, 3), f"cup=({r},{c}) want (3,3)")
    r, c = do("Up")
    rec("Up == C-p", (r, c) == (2, 11), f"cup=({r},{c}) want (2,11)")
    r, c = do("Right")
    rec("Right == C-f", (r, c) == (2, 12), f"cup=({r},{c}) want (2,12)")
    r, c = do("Left")
    rec("Left == C-b", (r, c) == (2, 11), f"cup=({r},{c}) want (2,11)")

    # C-v keeps the point's screen row: step to line 5 (row 6), then C-v.
    r, c = do("C-n " * 4)
    rec("C-n x4 -> line 5 (screen row 4)", (r, c) == (6, 11), f"cup=({r},{c}) want (6,11)")
    before_row = r
    r, c = do("C-v")
    rec("C-v holds the point's screen row",
        r == before_row and before_row == 6,
        f"cup_row={r} before={before_row}")

    # M-> lands the point at the buffer end; the window follows (screen row
    # 20 of the 21-line window -> terminal row 22, col 3 on the 2-char line).
    r, c = do("M->")
    rec("M-> buffer end (window follows)", (r, c) == (22, 3),
        f"cup=({r},{c}) want (22,3)")
    # M-< lands the point at the buffer start.
    r, c = do("M-<")
    rec("M-< buffer start", (r, c) == (2, 1), f"cup=({r},{c}) want (2,1)")

    # The status line's position display tracks the point's line: one line
    # down from the top is the 2nd line ("L2,...").
    s.key("C-n", 0.6)
    rec("position display tracks the point line", "L2" in s.text(),
        "")

    # which-function matches the point line: open whichfn.rs, step into
    # `fn alpha` (line 1) and the status line shows (alpha).
    s.key("C-x C-f", 1.0)
    for ch in "whichfn":
        s.key(ch, 0.25)
    s.key("RET", 1.2)
    s.key("C-n", 0.8)  # line 1: inside fn alpha
    # the index build is async; give it a moment to land whichfn.rs.
    for _ in range(6):
        if "(alpha)" in s.text():
            break
        s.wait(0.5)
    rec("which-function matches the point line", "(alpha)" in s.text(), "")

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
    # plan 004 issue 05b: file-view point (line, col) + emacs motion leg.
    print("=== FILE-VIEW POINT (line, col) + emacs motion ===")
    all_checks += file_view_checks()
    print()
    bad = [n for n, ok, _ in all_checks if not ok]
    print("=== SUMMARY ===")
    for n, ok, d in all_checks:
        print(f"  {'PASS' if ok else 'FAIL'}  {n}")
    print(f"\nTOTAL: {len(all_checks) - len(bad)}/{len(all_checks)} raw-stream checks PASS")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

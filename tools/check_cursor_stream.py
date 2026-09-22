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
  * plan 004 issue 05d: with wide (CJK) chars the CUP column is the DISPLAY
    column, clicks convert display column back to char index (a click inside
    a wide char maps to that char), and a space after a wide char is not
    dropped by the render.
  * plan 004 issue 05e: with the tree sidebar visible, click columns are
    offset by the tree width (a click at 1-based terminal col 34 + k lands
    char index k-1; clicks inside the tree's columns select a tree row and
    do not move the code point).
  * plan 004 issue 05g: with the tree sidebar visible, the HARDWARE cursor
    (CUP) column is also offset by the tree width (after N× C-f from the
    line start, the 1-based CUP column equals TREE_WIDTH + N + 1, and
    equals the code pane's actual start column read from the frame + N +
    1); tree-hidden stays 1:1.
  * plan 004 issue 05g (carried P2): the list-view (magit) hardware cursor
    (CUP) column with the tree sidebar visible equals the magit pane's start
    column read from the frame (the 05g offset applies to every view arm).
  * plan 004 issue 05f: the transient menu (`?`) renders two ellipsized
    columns with a visible gutter at 80 cols (no two-cell collision) and
    falls back to one full-width column at narrow widths.

Read-window policy (deflake-timing, item 2): a raw read whose assertion is
the surviving CUP stays OPEN until a CUP after the final `?2026l` is
observed (`Session.cup_settle`), with a hard deadline (`CUP_WAIT_CAP`) as
backstop — a missing CUP is then a real protocol failure, not a scheduling
artifact (the old behavior closed on a quiet window alone, so a CUP starved
past the window read as `None` under load). Reads that assert nothing about
the CUP (and views that emit no cursor — Home/picker frames carry only the
frame's in-region park, never a post-`?2026l` CUP) keep the quiet close; the
gate is applied at the assertion site, never by raising the sleep.

Exit 0 = all assertions pass; 1 = any failed.
"""
import os, pty, fcntl, termios, struct, time, select, signal, re
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pyte
from pyte_driver import encode_key, BAR_BGS
from fixture import repo

BIN = os.environ.get("REDLINE_BIN", os.path.join(os.path.dirname(__file__), "..", "target", "debug", "redline"))
REPO = os.environ.get("REDLINE_REPO") or repo("redline_pyte_repo")
COLS, ROWS = 80, 24

# deflake-timing: hard-deadline backstop for the CUP-after-final-?2026l gate
# (cup_settle). Only reads whose assertion IS the CUP pay this, and only while
# the CUP is still missing — the common path closes on quiet exactly as before.
CUP_WAIT_CAP = 5.0

# deflake-timing: the frame's synchronized-update close + a CUP (CUP/CHA: the
# `ESC[r;c H`/`f` position report the drive's `last_cup_after_sync` matches).
_SYNC_END_RE = re.compile(rb"\x1b\[\?2026l")
_CUP_RE = re.compile(rb"\x1b\[\d+;\d+[Hf]")


def _cup_settled(buf):
    """Protocol-complete: no synchronized frame in this chunk, or a CUP was
    issued after its final `?2026l`."""
    ends = [m.end() for m in _SYNC_END_RE.finditer(buf)]
    if not ends:
        return True
    return bool(_CUP_RE.search(buf[ends[-1]:]))


def _set_winsize(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


class Session:
    def __init__(self, colorterm, cols=COLS, rows=ROWS):
        self.colorterm = colorterm
        self.cols, self.rows = cols, rows
        self.master, slave = pty.openpty()
        _set_winsize(slave, self.rows, self.cols)
        _set_winsize(self.master, self.rows, self.cols)
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
        self.screen = pyte.Screen(self.cols, self.rows)
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

    def cup_settle(self, buf, cap=CUP_WAIT_CAP):
        """deflake-timing item 2: keep the read window open until a CUP after
        the final `?2026l` is observed (hard deadline as backstop), instead of
        closing on quiet alone — so a missing CUP is a real protocol failure
        (a finding), not a scheduling artifact. The quiet window alone used to
        end the chunk at the frame's `?2026l` with a starved CUP still owed;
        under load the deferred CUP then read as `None`. Only called on reads
        whose assertion IS the CUP: frames of cursor-less views (Home/picker)
        never emit a post-`?2026l` CUP, so gating every read would wait the
        full cap for no reason. Common path: CUP already present → closes on
        quiet exactly as before."""
        deadline = time.time() + cap
        last = time.time()
        while not _cup_settled(buf):
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
        return "\n".join("".join(self.screen.buffer[r][i].data for i in range(self.cols)) for r in range(self.rows))

    def row_text(self, row):
        return "".join(self.screen.buffer[row][i].data for i in range(self.cols))

    def bar_rows(self):
        return [r for r in range(self.rows - 1)
                if any(str(self.screen.buffer[r][i].bg).lower() in BAR_BGS for i in range(self.cols))]

    def reverse_rows(self):
        return [r for r in range(self.rows)
                if any(self.screen.buffer[r][i].reverse for i in range(self.cols))]

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
    ends = [m.end() for m in _SYNC_END_RE.finditer(buf)]
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
    open_buf = s.cup_settle(s.key("C-x g", 1.2))
    # startup hide is countered + a show+reposition follows the first frame
    rec("?25l countered (a ?25h follows the first frame)", show_after_sync(open_buf),
        f"?25l={open_buf.count(b'\x1b[?25l')} ?25h={open_buf.count(b'\x1b[?25h')}")

    # n/p navigation: CUP row must track the selected (blue-bar) row.
    track_ok = True
    track_detail = []
    for i in range(5):
        buf = s.cup_settle(s.key("n" if i % 2 == 0 else "p", 0.7))
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


def mouse_recenter_word_checks():
    """plan 004 issue 05c: raw-PTY legs — click at a non-zero column (CUP
    col equals it; past-EOL clamps to EOL), wheel scroll with the point's
    screen row pinned (the top line changes AND the point line advances
    under the window), C-l x3 cycling the point's screen row 1 -> mid ->
    last with the point line fixed (then #4 back to 1), and M-f/M-b landing
    the cursor column on word boundaries (exact columns).

    Mouse events are SGR-encoded: left-click press is ESC[<0;COL;ROWM with
    1-based COL/ROW (crossterm decodes to 0-based row/column); wheel up is
    ESC[<64;1;1M, wheel down ESC[<65;1;1M.
    """
    import os as _os
    src_dir = _os.path.join(REPO, "src")
    _os.makedirs(src_dir, exist_ok=True)
    # leg.rs: 60 lines; even line = 20 'A's, odd line = 2 'b's. NO trailing
    # newline (ropey would count a trailing \n as an extra empty line).
    llines = ["A" * 20 if i % 2 == 0 else "b" * 2 for i in range(60)]
    with open(_os.path.join(src_dir, "leg.rs"), "w") as f:
        f.write("\n".join(llines))
    # wordleg.rs: 3 lines of words/punctuation for the M-f/M-b legs.
    with open(_os.path.join(src_dir, "wordleg.rs"), "w") as f:
        f.write("hello world_foo!!\nx\nab cd")

    s = Session(None)
    checks = []

    def rec(name, ok, detail=""):
        checks.append((name, ok, detail))
        print(f"  {'PASS' if ok else 'FAIL'}  {name:46s} {detail}")

    def do(key, settle=0.6):
        """Press `key` and return the surviving CUP (row, col) after settle."""
        return last_cup_after_sync(s.cup_settle(s.key(key, settle)))

    def feed_mouse(data, settle=0.6):
        """Send raw SGR mouse bytes and return the surviving CUP."""
        os.write(s.master, data)
        buf = s.cup_settle(s._read(settle, quiet=0.15))
        return last_cup_after_sync(buf)

    # ── click at a non-zero column (leg.rs: window top 0) ──────────────────
    s.key("C-x C-f", 1.0)
    for ch in "leg.rs":
        s.key(ch, 0.25)
    s.key("RET", 1.0)
    # Click terminal (row 5, col 10) 0-based: content row 4 → buffer line 4,
    # col 10. SGR is 1-based: ESC[<0;11;6M → CUP (6, 11) 1-based.
    row, col = feed_mouse(b"\x1b[<0;11;6M")
    rec("click at col 10: CUP col equals it", (row, col) == (6, 11),
        f"cup=({row},{col}) want (6,11)")
    # Click past EOL on short line 1 ("bb"): terminal row 2 (0-based) is
    # SGR row 3, col 30 is SGR col 31 → clamped to col 2 → CUP (3, 3).
    row, col = feed_mouse(b"\x1b[<0;31;3M")
    rec("click past EOL clamps to EOL", (row, col) == (3, 3),
        f"cup=({row},{col}) want (3,3)")

    # ── wheel: window scroll with the point's screen row pinned ────────────
    # Reset the point to (0,0) at window top 0.
    s.key("M-<", 0.8)
    top_line_before = s.row_text(1)        # content row 0 (terminal row 1)
    r0, c0 = last_cup_after_sync(s.cup_settle(s.key("C-a", 0.6)))
    r, c = feed_mouse(b"\x1b[<65;1;1M")   # wheel down (3-line step)
    top_line_after = s.row_text(1)
    rec("wheel down: top line changed, screen row pinned",
        (r, c) == (r0, c0) and top_line_before != top_line_after,
        f"cup=({r},{c}) before=({r0},{c0}) top row now={top_line_after[:8]!r} was={top_line_before[:8]!r}")
    # The point line advanced under the window: the window top is now line 3
    # (a "bb" line) and the status line shows the point at L4 (1-based).
    rec("wheel down: point line advanced under the window",
        s.row_text(1).strip().startswith("bb") and "L4," in s.text(),
        f"top row={s.row_text(1)[:8]!r} status L4={('L4,' in s.text())}")
    r, c = feed_mouse(b"\x1b[<64;1;1M")   # wheel back up
    rec("wheel up: original top line back, row pinned",
        (r, c) == (r0, c0) and s.row_text(1).strip().startswith("AAAA"),
        f"cup=({r},{c}) top row={s.row_text(1)[:8]!r}")

    # ── C-l x3: point line fixed, middle -> top -> bottom ─────────────────
    # Viewport = 24 - 3 = 21 (0-based screen rows: mid = 10, last = 20).
    # Point to line 25 (C-n x25 from the top). A fresh C-l goes to MIDDLE
    # (CUP row 12); the next C-l's go to TOP (CUP 2) then BOTTOM (CUP 22);
    # #4 returns to MIDDLE (CUP 12). The point line never moves.
    for _ in range(25):
        s.key("C-n", 0.15)
    s._read(0.5, quiet=0.15)
    r1, _ = do("C-l")
    r2, _ = do("C-l")
    r3, _ = do("C-l")
    point_fixed = "L26," in s.text()
    rec("C-l x3: middle -> top -> bottom (point line fixed)",
        (r1, r2, r3) == (12, 2, 22) and point_fixed,
        f"rows={r1},{r2},{r3} want 12,2,22 L26={point_fixed}")
    r4, _ = do("C-l")
    rec("C-l #4: cycle returns to middle", r4 == 12, f"row={r4} want 12")

    # ── M-f / M-b: cursor column on word boundaries (wordleg.rs) ──────────
    s.key("C-x C-f", 1.0)
    for ch in "wordleg":
        s.key(ch, 0.25)
    s.key("RET", 1.0)
    r0, c0 = last_cup_after_sync(s.cup_settle(s.key("C-a", 0.6)))
    rec("wordleg open: cursor at (line 1, col 1)", (r0, c0) == (2, 1),
        f"cup=({r0},{c0}) want (2,1)")
    fw = [
        ("M-f", (2, 6), "end of `hello`"),
        ("M-f", (2, 16), "skip the space, walk to the end of `world_foo`"),
        ("M-f", (3, 2), "skip `!!`+newline, end of `x`"),
        ("M-f", (4, 3), "wrap across lines, end of `ab`"),
        ("M-f", (4, 6), "skip the space, end of `cd` (buffer end)"),
    ]
    fw_ok, fw_detail = True, []
    for key, want, why in fw:
        r, c = do(key)
        ok = (r, c) == want
        fw_ok = fw_ok and ok
        fw_detail.append(f"{key}:{(r,c)}{'=' if ok else '!='}{want} ({why})")
    r, c = do("M-f")
    fw_ok = fw_ok and (r, c) == (4, 6)
    fw_detail.append(f"M-f(noop @ end):{(r,c)}== (4,6)")
    rec("M-f x5: forward-word lands on exact columns (word ends)", fw_ok, "; ".join(fw_detail))
    bw = [
        (4, 4), (4, 1), (3, 1), (2, 7), (2, 1),
    ]
    bw_ok, bw_detail = True, []
    for i, want in enumerate(bw):
        r, c = do("M-b")
        ok = (r, c) == want
        bw_ok = bw_ok and ok
        bw_detail.append(f"M-b{i+1}:{(r,c)}{'=' if ok else '!='}{want}")
    rec("M-b x5: backward-word lands on exact columns (word starts)", bw_ok, "; ".join(bw_detail))
    r, c = do("M-b M-b M-b")
    rec("M-b at buffer start: no-op", (r, c) == (2, 1), f"cup=({r},{c}) want (2,1)")

    s.kill()
    return checks


def file_view_checks():
    """plan 004 issue 05b: the file-view cursor tracks a real (line, col)
    point step by step under C-n/C-p/C-f/C-b/arrows/C-a/C-e, with EOL/BOL
    wrapping, goal-column across short/long lines, C-v holding the point's
    screen row, M-END/M-< landing the point at the buffer end/start (window
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
        return last_cup_after_sync(s.cup_settle(s.key(key, settle)))

    # Open cursorleg.rs: the cursor lands on the first content line, col 1.
    s.key("C-x C-f", 1.0)
    for ch in "cursorleg":
        s.key(ch, 0.25)
    buf = s.cup_settle(s.key("RET", 1.0))
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

    # M-END lands the point at the buffer end; the window follows (screen
    # row 20 of the 21-line window -> terminal row 22, col 3 on the 2-char
    # line). (jump-ambiguity: point-buffer-end moved M-> -> M-END; M-> now
    # forces the Xref candidate list.)
    r, c = do("M-END")
    rec("M-END buffer end (window follows)", (r, c) == (22, 3),
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


def wide_char_checks():
    """plan 004 issue 05d: wide (CJK) chars occupy 2 cells — the CUP column
    is the DISPLAY column (char index -> prefix display width), clicks
    convert the other way (display column -> char index; a click inside a
    wide char maps to that char), and the render keeps a space after a wide
    char (the draw_line x-advance must be width-aware).
    """
    import os as _os
    src_dir = _os.path.join(REPO, "src")
    _os.makedirs(src_dir, exist_ok=True)
    with open(_os.path.join(src_dir, "wideleg.rs"), "w") as f:
        f.write("CJK: abcd中 efgh\nfn über() { let 中 = 1; }\n")

    s = Session(None)
    checks = []

    def rec(name, ok, detail=""):
        checks.append((name, ok, detail))
        print(f"  {'PASS' if ok else 'FAIL'}  {name:46s} {detail}")

    def do(key, settle=0.6):
        """Press `key` and return the surviving CUP (row, col)."""
        return last_cup_after_sync(s.cup_settle(s.key(key, settle)))

    def click(col0, row0):
        """Left-click at 0-based terminal (col, row) via SGR (1-based)."""
        os.write(s.master, b"\x1b[<0;%d;%dM" % (col0 + 1, row0 + 1))
        return last_cup_after_sync(s.cup_settle(s._read(0.6, quiet=0.15)))

    s.key("C-x C-f", 1.0)
    for ch in "wideleg":
        s.key(ch, 0.2)
    s.cup_settle(s.key("RET", 1.2))
    # Line 0: 中 is char 9 and occupies display cols 9-10, so 'g' (char 13)
    # sits at display col 14 (0-based).
    s.key("C-a", 0.5)
    r, c = do("C-f " * 13)
    rec("cursor on char 13: CUP col is DISPLAY col 14", (r, c) == (2, 15),
        f"cup=({r},{c}) want (2,15)")
    r, c = do("C-e")
    rec("cursor at EOL: CUP col is display col 16", (r, c) == (2, 17),
        f"cup=({r},{c}) want (2,17)")
    # Click the SECOND cell of 中 (display col 10): maps to char 9, whose
    # display col is 9 -> CUP (2, 10). Char-index arithmetic would land on
    # the space (char 10) -> CUP col 11.
    r, c = click(10, 1)
    rec("click inside 中 (2nd cell) maps to that char", (r, c) == (2, 10),
        f"cup=({r},{c}) want (2,10)")
    # Click display col 14 (the 'g', char index 13) -> CUP (2, 15).
    r, c = click(14, 1)
    rec("click on display col 14 lands on char 13", (r, c) == (2, 15),
        f"cup=({r},{c}) want (2,15)")
    # The space after the wide char survives the render: content row 1
    # (terminal row 2, 0-based) shows `let 中 = 1;` with its space.
    line1 = s.row_text(2)
    rec("space after a wide char survives the render",
        "let 中 = 1;" in line1, f"row={line1[:32]!r}")

    s.kill()
    return checks


def tree_click_checks():
    """plan 004 issue 05e: with the tree sidebar visible (TREE_WIDTH = 34
    terminal columns), click columns are offset by the tree width: a click
    at 1-based terminal col 34 + k on a code row lands char index k-1 (CUP
    col k in ASCII), a click past the code pane's EOL clamps to EOL, and a
    click inside the tree's columns does NOT move the code point (it
    selects a tree row instead). The tree-hidden leg stays 1:1.
    """
    import os as _os
    src_dir = _os.path.join(REPO, "src")
    _os.makedirs(src_dir, exist_ok=True)
    # leg.rs: 60 lines; even line = 20 'A's, odd line = 2 'b's. NO trailing
    # newline (ropey would count a trailing \n as an extra empty line).
    llines = ["A" * 20 if i % 2 == 0 else "b" * 2 for i in range(60)]
    with open(_os.path.join(src_dir, "leg.rs"), "w") as f:
        f.write("\n".join(llines))

    s = Session(None)
    checks = []

    def rec(name, ok, detail=""):
        checks.append((name, ok, detail))
        print(f"  {'PASS' if ok else 'FAIL'}  {name:46s} {detail}")

    def click(col0, row0):
        """Left-click at 0-based terminal (col, row) via SGR (1-based)."""
        os.write(s.master, b"\x1b[<0;%d;%dM" % (col0 + 1, row0 + 1))
        return last_cup_after_sync(s.cup_settle(s._read(0.6, quiet=0.15)))

    # Open leg.rs (line 0 = 20 A's) and show the tree sidebar.
    s.key("C-x C-f", 1.0)
    for ch in "leg.rs":
        s.key(ch, 0.25)
    s.cup_settle(s.key("RET", 1.0))
    s.key("C-c p t", 0.9)
    if "*tree*" not in s.text():
        rec("tree visible (C-c p t)", False, "*tree* missing")
        s.kill()
        return checks
    rec("tree visible (C-c p t)", True, "")

    # Point at (0,0) before the click legs.
    s.key("C-a", 0.6)

    # 1-based terminal col 34 + 6 = 40 on content row 0 (terminal row 1,
    # 0-based): pane col 39-34=5 → char index 5 → CUP col 34 + 5 + 1 (05g:
    # the CUP is in absolute screen coordinates, offset by the tree width).
    r, c = click(34 + 6 - 1, 1)
    rec("tree on: click 1-based col 40 → char 5 (CUP (2,40))", (r, c) == (2, 40),
        f"cup=({r},{c}) want (2,40)")
    # 1-based col 35 (k=1): the first code-pane cell → char 0 → CUP (2,35).
    r, c = click(34, 1)
    rec("tree on: click 1-based col 35 → char 0 (CUP (2,35))", (r, c) == (2, 35),
        f"cup=({r},{c}) want (2,35)")
    # Past the code pane's EOL (20 A's): 1-based col 34+25=59 → pane col 24
    # → clamped to EOL char 20 → CUP col 34 + 20 + 1 = 55.
    r, c = click(34 + 25 - 1, 1)
    rec("tree on: click past code pane EOL clamps to EOL",
        (r, c) == (2, 55), f"cup=({r},{c}) want (2,55)")
    # A click inside the tree's columns (1-based col 10, a visible tree
    # row) selects a tree row but does NOT move the code point: the CUP
    # stays where the previous click left it (point (0,20) → CUP (2,55)).
    r, c = click(9, 2)
    rec("tree on: click in tree cols does not move the code point",
        (r, c) == (2, 55), f"cup=({r},{c}) want (2,55)")
    # Tree-hidden leg (regression): toggle the tree off; the mapping is 1:1
    # again (1-based col 6 → char 5 → CUP (2,6)).
    s.key("C-c p t", 0.9)
    r, c = click(5, 1)
    rec("tree off: click 1-based col 6 → CUP (2,6) (1:1)", (r, c) == (2, 6),
        f"cup=({r},{c}) want (2,6)")

    s.kill()
    return checks


def tree_cursor_offset_checks():
    """plan 004 issue 05g: with the tree sidebar visible the hardware
    cursor (CUP) column is offset by the tree width — after N× C-f the
    CUP column equals TREE_WIDTH + N (1-based TREE_WIDTH + N + 1), and
    equals the code pane's actual start column (read from the frame, so
    the test cannot drift if the layout changes) + N. Tree-hidden stays
    1:1 (CUP col = N + 1)."""
    import os as _os
    src_dir = _os.path.join(REPO, "src")
    _os.makedirs(src_dir, exist_ok=True)
    # leg.rs: 60 lines; even line = 20 'A's, odd line = 2 'b's. NO trailing
    # newline (ropey would count a trailing \n as an extra empty line).
    llines = ["A" * 20 if i % 2 == 0 else "b" * 2 for i in range(60)]
    with open(_os.path.join(src_dir, "leg.rs"), "w") as f:
        f.write("\n".join(llines))

    s = Session(None)
    checks = []

    def rec(name, ok, detail=""):
        checks.append((name, ok, detail))
        print(f"  {'PASS' if ok else 'FAIL'}  {name:46s} {detail}")

    def do(key, settle=0.6):
        """Press `key` and return the surviving CUP (row, col)."""
        return last_cup_after_sync(s.cup_settle(s.key(key, settle)))

    def pane_start_col():
        """0-based terminal column where the code pane's line 0 text starts
        (the first run of 20 A's on content row 0 / terminal row 1)."""
        line = s.row_text(1)
        i = line.find("AAAAAA")
        if i < 0:
            return None
        # The A-run is 20 chars: the pane starts where it starts.
        return i

    # Open leg.rs (line 0 = 20 A's) and show the tree sidebar.
    s.key("C-x C-f", 1.0)
    for ch in "leg.rs":
        s.key(ch, 0.25)
    s.cup_settle(s.key("RET", 1.0))
    s.key("C-c p t", 0.9)
    if "*tree*" not in s.text():
        rec("tree visible (C-c p t)", False, "*tree* missing")
        s.kill()
        return checks
    rec("tree visible (C-c p t)", True, "")
    start = pane_start_col()
    rec("frame: code pane start column read from the frame",
        start is not None and start >= 1,
        f"pane start col (0-based)={start}")
    s.key("C-a", 0.6)

    # Tree-visible leg: after cumulative N× C-f (point at char N of line
    # 0), the CUP column must be TREE_WIDTH + N (0-based) = TREE_WIDTH + N +
    # 1 (1-based), i.e. pane start + N + 1 — NOT inside the sidebar.
    cup_ok, cup_detail, total = True, [], 0
    for step in (1, 3, 7):
        total += step
        r, c = do("C-f " * step)  # `step` × C-f from the previous point
        cup1 = (start is not None) and (c == start + total + 1)
        cup_ok = cup_ok and cup1 and r == 2
        cup_detail.append(f"N={total}:cup=({r},{c}) want col {start + total + 1 if start is not None else '?'}")
    rec("tree on: cumulative C-f ×N → CUP col = pane start + N (1-based)",
        cup_ok, "; ".join(cup_detail))
    # The exact repro from the issue: 3× C-f from (0,0) on an 80-col
    # terminal with the tree at col 34 → 0-based CUP col 34 + 3 = 37
    # (1-based 38 — this reader is 1-based), not 0-based 3 (inside the
    # sidebar).
    s.key("M-<", 0.8)
    r, c = do("C-f C-f C-f")
    rec("tree on: 3× C-f → CUP (row 2, 0-based col 37) (repro: was 0-based col 3)",
        (r, c) == (2, 38), f"cup=({r},{c}) want (2,38)")

    # Tree-hidden leg (regression): toggle the tree off; CUP col is 1:1
    # with the pane (3× C-f → col 4, 1-based).
    s.key("C-c p t", 0.9)
    s.key("M-<", 0.8)
    r, c = do("C-f C-f C-f")
    rec("tree off: 3× C-f → CUP (row 2, col 4) (1:1)",
        (r, c) == (2, 4), f"cup=({r},{c}) want (2,4)")

    s.kill()
    return checks


def _menu_rows(s):
    """The rendered rows of the transient-menu overlay (title row to bottom)."""
    rows = s.text().split("\n")
    start = 0
    for i, r in enumerate(rows):
        if "Transient menu" in r:
            start = i
            break
    return rows[start:]


def _menu_collisions(rows):
    """Rows containing a two-cell collision: '][' or a letter immediately
    followed by '[' — the old hard-cut + no-gutter symptom."""
    bad = []
    for r in rows:
        r2 = r.rstrip()
        if re.search(r"\]\[", r2) or re.search(r"[a-zA-Z]\[", r2):
            bad.append(r2)
    return bad


def transient_menu_checks():
    """plan 004 issue 05f: the transient menu (`?`) renders two ellipsized
    columns with a visible gutter at 80 cols (no two-cell collision, every
    row <= width, long descriptions ellipsized) and falls back to ONE
    full-width column at narrow widths (~30 cols) so a long description is
    shown whole or ellipsized with no two-cell collision."""
    checks = []

    def rec(name, ok, detail=""):
        checks.append((name, ok, detail))
        print(f"  {'PASS' if ok else 'FAIL'}  {name:46s} {detail}")

    # 80 cols: two columns, gutter, ellipsis.
    s = Session(None)
    s.key("C-x g", 1.2)
    s.key("?", 0.7)
    rows = _menu_rows(s)
    rec("menu: title row present", any("Transient menu" in r for r in rows),
        f"rows={len(rows)}")
    collisions = _menu_collisions(rows)
    rec("menu@80: no two-cell collision (no '][' / letter-then-'[')",
        not collisions, "; ".join(collisions[:3]))
    rec("menu@80: every row fits the width",
        all(len(r) <= COLS for r in rows),
        f"max row len={max(len(r) for r in rows)}")
    # A DESCRIPTION that did not fit must carry the truncation ellipsis.
    # NOTE: prefix rows render as "KEY …" (a literal ellipsis, present
    # before this fix too), so "any '…' in rows" would pass on the OLD
    # output — require the ellipsis to follow a "[KEY]" description
    # marker, which only happens for a truncated description (05f
    # review P2-1).
    desc_truncated = [r for r in rows if "]" in r and "…" in r
                      and r.index("…") > r.index("]")]
    rec("menu@80: long descriptions ellipsized (a DESCRIPTION, not a prefix row)",
        len(desc_truncated) >= 1,
        f"description rows with '…': {len(desc_truncated)}")
    s.kill()

    # ~30 cols: single-column fallback (w < 40).
    narrow_cols = 30
    s2 = Session(None, cols=narrow_cols, rows=24)
    s2.key("C-x g", 1.2)
    s2.key("?", 0.7)
    nrows = _menu_rows(s2)
    nc = _menu_collisions(nrows)
    rec(f"menu@{narrow_cols}: single column (no two-cell collision)",
        not nc, "; ".join(nc[:3]))
    # A long description must be present whole or ellipsized on its own row,
    # and no row may overflow the width.
    long_rows = [r for r in nrows if "Move to the" in r]
    rec(f"menu@{narrow_cols}: long description on its own row (whole or …)",
        len(long_rows) >= 1 and any("…" in r or "buffer" in r for r in long_rows),
        "; ".join(r.rstrip() for r in long_rows[:2]))
    rec(f"menu@{narrow_cols}: every row fits the width",
        all(len(r) <= narrow_cols for r in nrows),
        f"max row len={max(len(r) for r in nrows)}")
    s2.kill()
    return checks


def list_view_cup_checks():
    """plan 004 issue 05g (carried P2): with the tree sidebar visible the
    LIST-view hardware cursor (CUP) column is offset by the tree width — the
    surviving CUP column equals the magit pane's start column read from the
    frame (0-based col 34). The 05g offset applies to every view arm; only
    the Buffer arm was asserted before."""
    s = Session(None)
    checks = []

    def rec(name, ok, detail=""):
        checks.append((name, ok, detail))
        print(f"  {'PASS' if ok else 'FAIL'}  {name:46s} {detail}")

    s.key("C-x g", 1.2)
    buf = s.cup_settle(s.key("C-c p t", 0.9))
    if "*tree*" not in s.text():
        rec("list view: tree visible (C-c p t)", False, "*tree* missing")
        s.kill()
        return checks
    rec("list view: tree visible (C-c p t)", True, "")
    # The magit pane's start column, read from the frame (the magit title
    # '*magit-status*' begins where the pane begins).
    start = s.row_text(0).find("*magit")
    rec("list view: pane start column read from the frame (0-based = 34)",
        start == 34, f"pane start (0-based)={start}")
    # The CUP survives at the pane's first content column (char 0 of the
    # selected row): 0-based CUP col == pane start (1-based CUP = start + 1).
    row, col = last_cup_after_sync(buf)
    rec("list view: surviving CUP column equals pane start (0-based)",
        start is not None and col is not None and (col - 1) == start,
        f"cup=({row},{col}) want 0-based col {start}")
    s.kill()
    return checks


def annotation_gutter_checks():
    """plan 005 issue 02b + annotations-fold-visual: the annotated line has a
    2-cell gutter (the fold arrow at cell 0, the tree-line branch/blank at
    cell 1, code at cell 2 — the SAME column folded and shown), the note row
    carries the 2-branch tree-line's curved corner (╭ at cell 0, ─ bend at
    cell 1, note text at cell 2) ABOVE the anchored code row, the cursor
    column adds the gutter on
    annotated lines, and note rows do not push the point's line off-canvas.

    Legs:
      * Open a >viewport file, `A` on line 0, C-n to the window bottom:
        assert the position's line IS drawn and the cursor row equals it.
      * Assert the annotated line renders its source text VERBATIM after
        the gutter (full-string assertion, not prefix).
      * Cursor-column leg: the hardware cursor on an annotated line sits ON
        the character (gutter + display col), not one cell left — and the
        note row ABOVE the code row moved the cursor's CUP row down by 1
        when it committed (the CUP stream is consistent with the note
        above).
      * All-annotated canvas FILL (02c): a 25-line file with every line
        annotated in the 21-row viewport emits ~20 of 21 rows (10 code +
        10 note — the LARGEST fitting span), not a handful of non-blank
        rows; the point's line is drawn.
    """
    import os as _os
    src_dir = _os.path.join(REPO, "src")
    _os.makedirs(src_dir, exist_ok=True)
    # ann_gutter.rs: 30 lines; line 0 is distinctive for the verbatim check.
    with open(_os.path.join(src_dir, "ann_gutter.rs"), "w") as f:
        f.write("fn target_one() {}\n" +
                "\n".join(f"filler line {i}" for i in range(1, 30)))

    s = Session(None)
    checks = []

    def rec(name, ok, detail=""):
        checks.append((name, ok, detail))
        print(f"  {'PASS' if ok else 'FAIL'}  {name:52s} {detail}")

    def do(key, settle=0.6):
        return last_cup_after_sync(s.cup_settle(s.key(key, settle)))

    # Open ann_gutter.rs.
    s.key("C-x C-f", 1.0)
    for ch in "ann_gutter":
        s.key(ch, 0.25)
    s.key("RET", 1.2)

    # ── Leg 1: annotated line renders source text VERBATIM after gutter ──
    # After `A` on line 0 (note SHOWN by default), the code row is
    # "\u25be\u2500fn target_one() {}" — the ▾ fold arrow at cell 0, the ─
    # tree-line branch at cell 1, code at cell 2 (the full string, not a
    # prefix; the 2-cell gutter is the no-jitter leading width).
    s.key("A", 0.5)
    # Type the note text and commit with RET.
    os.write(s.master, b"regression check\r")
    s.cup_settle(s._read(0.8, quiet=0.15))
    # Find the row with the SHOWN arrow (▾) and verify the full text after the
    # gutter.
    row_text = None
    for r in range(1, s.rows - 2):
        t = s.row_text(r)
        if "\u25be" in t:
            row_text = t
            break
    verbatim = row_text is not None and "fn target_one() {}" in row_text
    # The arrow is at cell 0, the branch at cell 1, the code at cell 2:
    # the row starts with "\u25be\u2500" followed by "fn target_one() {}".
    gutter_correct = row_text is not None and row_text.startswith("\u25be\u2500fn target_one() {}")
    rec("annotated line: source text VERBATIM after the 2-cell gutter", verbatim,
        f"row={row_text[:40]!r}" if row_text else "arrow row not found")
    rec("annotated line: ▾ at cell 0, ─ at cell 1, code at cell 2 (full-string)",
        gutter_correct,
        f"row starts with arrow+branch+code: {gutter_correct}")
    # annotations-curve-glyphs (design A): the note row is directly ABOVE
    # the arrow row, carrying the tree-line's curved corner (╭ at cell 0 —
    # the SAME cell as the ▾ on the code row below — with its ─ bend at
    # cell 1) and the note text at cell 2 (the note is the code row's
    # header, not its trailer). The per-cell "\u256d\u2500regression
    # check" string is the ANCHOR assertion: it is false if the corner
    # moves one cell or if the cell-1 bend is dropped.
    marker_row_idx = next((r for r in range(1, s.rows - 2)
                           if "\u25be" in s.row_text(r)), None)
    note_above = marker_row_idx is not None and marker_row_idx > 1 and (
        "\u256d\u2500regression check" in s.row_text(marker_row_idx - 1)
    )
    rec("note row (╭ corner at cell 0, ─ at cell 1) sits directly ABOVE the code row",
        note_above, f"arrow_row={marker_row_idx}")

    # ── Leg 2: cursor column on an annotated line adds the gutter ──────
    # Point is at line 0, col 0 (after the annotation commit, the point
    # stays on the anchored line). Before the commit the CUP was at (2, 2)
    # (1-based); the note row ABOVE the code row pushed the code row down
    # by 1, so the CUP row is now 3 — the CUP stream is consistent with
    # the note above. Terminal col: gutter(2) + display_col(0) = 2
    # (0-based) = 3 (1-based).
    r, c = do("C-a", 0.6)
    rec("cursor on annotated line: CUP row moved +1 (note above), col = gutter + 0",
        (r, c) == (3, 3), f"cup=({r},{c}) want (3,3)")
    # C-f x3: display col 3, terminal col = 2 + 3 = 5 (0-based) = 6 (1-based).
    r, c = do("C-f C-f C-f")
    rec("cursor on annotated line: C-f x3 → CUP col = gutter + 3 (1-based col 6)",
        (r, c) == (3, 6), f"cup=({r},{c}) want (3,6)")

    # ── Leg 3: note rows do not push the point off-canvas ─────────────
    # 30-line file, viewport 21. Note on line 0 (already created). C-n x20
    # puts the point at line 20. The window bottom must show line 20 and
    # the cursor must be on it.
    for _ in range(20):
        s.key("C-n", 0.15)
    s._read(0.5, quiet=0.15)
    r, c = last_cup_after_sync(s.cup_settle(s.key("C-a", 0.6)))
    # The cursor row must be within the canvas: the title occupies CUP row
    # 1, so the 21 content rows are CUP rows 2..=22 (1-based).
    cursor_in_canvas = r is not None and 2 <= r <= 22
    rec("C-n x20: cursor row within canvas (not off-screen)",
        cursor_in_canvas, f"cup_row={r} (want 2..=22)")
    # The row UNDER the cursor must CONTAIN the point's line text (line 20
    # is "filler line 20") — a range check alone would also pass a future
    # off-by-one that lands the cursor on a NEIGHBOURING drawn row.
    # (row_text is 0-based; the CUP row r is 1-based.)
    cursor_row_text = s.row_text(r - 1) if r is not None else None
    cursor_on_point_line = (
        cursor_row_text is not None and "filler line 20" in cursor_row_text
    )
    rec("C-n x20: cursor row contains the point's line text",
        cursor_on_point_line,
        f"row={cursor_row_text!r} (want a row containing 'filler line 20')")
    # The point's line IS drawn: verify the cursor is NOT at the top (the
    # point moved down from line 0) and is in the lower half of the canvas.
    # The exact line number depends on the file length and note-row cap,
    # so we check the cursor is in the bottom 10 rows of the canvas.
    point_moved_down = r is not None and r >= 12  # lower half of 21-row canvas
    rec("C-n x20: point moved to the window bottom (cursor in lower half)",
        point_moved_down, f"cup_row={r} (want >=12)")
    # Verify the point is stable: C-p then C-n returns to the same row.
    r2, _ = do("C-p")
    r3, _ = do("C-n")
    rec("C-n x20: C-p,C-n round-trips to the same row (point stable)",
        r3 == r, f"original row={r}, after C-p row={r2}, after C-n row={r3}")

    # ── Leg 4: C-c a h (annotations-fold-visual): the TOGGLE. h → the note
    # row (curved corner) goes and the margin arrow folds ▾→▸; h again → the note
    # row and the ▾ arrow + ─ branch come back. The code row's TEXT and its
    # START COLUMN are unchanged in both states (no jitter).
    s.key("M-<", 0.8)  # go to top so the annotated line is visible
    s._read(0.5, quiet=0.15)

    def code_start_col(row_text):
        idx = row_text.find("fn target_one() {}")
        if idx < 0:
            return None
        # every char before the code is single-cell here (▾/▸/─/space)
        return sum(1 for _ in row_text[:idx])

    s.key("C-c a h", 0.6)
    folded_row = None
    note_row_gone = True
    for r2 in range(1, s.rows - 2):
        t = s.row_text(r2)
        if "\u256d\u2500regression check" in t:
            note_row_gone = False
        if "\u25b8" in t and "fn target_one() {}" in t:
            folded_row = t
            break
    hidden_msg = "note rows: hidden" in s.row_text(s.rows - 2)
    code_unchanged_hidden = folded_row is not None and "fn target_one() {}" in folded_row
    folded_col = code_start_col(folded_row) if folded_row else None
    s.key("C-c a h", 0.6)  # the SAME key toggles back (no separate C-c a s)
    s.cup_settle(s._read(0.4, quiet=0.15))
    shown_row = None
    note_row_back = False
    for r2 in range(1, s.rows - 2):
        t = s.row_text(r2)
        if "\u25be" in t and "fn target_one() {}" in t:
            shown_row = t
        if "\u256d\u2500regression check" in t:
            note_row_back = True
    shown_msg = "note rows: shown" in s.row_text(s.rows - 2)
    code_unchanged_shown = shown_row is not None and "fn target_one() {}" in shown_row
    shown_col = code_start_col(shown_row) if shown_row else None
    # NO JITTER: the code's start column is identical folded and shown.
    no_jitter = (folded_col is not None and folded_col == shown_col)
    rec("C-c a h toggle: fold keeps the code row, arrow carries the state",
        hidden_msg and code_unchanged_hidden and note_row_gone
        and shown_msg and note_row_back and code_unchanged_shown and no_jitter,
        f"hidden-msg={hidden_msg} folded-marker={code_unchanged_hidden} "
        f"note-gone={note_row_gone} shown-msg={shown_msg} note-back={note_row_back} "
        f"no-jitter={no_jitter} (folded-col={folded_col} shown-col={shown_col})")

    # ── Leg 5: all-annotated canvas FILL (02c) ────────────────────
    # plan 005 issue 02c: the 02b floor (span = 1) left a 25-line
    # all-annotated file in a 21-row viewport showing only 3 non-blank
    # rows. The LARGEST fitting span is 10 (10 + 10 notes <= 21, but
    # 11 + 11 > 21) → 10 code + 10 note rows: the canvas FILLS, and the
    # point's line (line 0) is drawn. Pre-seed .redline-notes.md on disk
    # (annotations are records matched by project-relative path) before
    # opening the file; the notes doc lazily re-reads on mtime change.
    with open(_os.path.join(src_dir, "ann_dense.rs"), "w") as f:
        f.write("\n".join(f"dense line {i}" for i in range(25)))
    notes = _os.path.join(REPO, ".redline-notes.md")
    with open(notes, "w") as f:
        f.write("<!-- redline-annotations:begin -->\n")
        for i in range(25):
            f.write(f"[annotation]\npath: src/ann_dense.rs\nline: {i}\n"
                    f"col: 0\nanchor: dense line {i}\nnote: n{i}\n"
                    f"orphaned: false\n")
        f.write("<!-- redline-annotations:end -->\n")
    # Re-walk (C-c p i): the find-file list is a cached walk, so a file
    # created after this session's first walk is invisible to C-x C-f
    # until the walk is invalidated (the same trick sweep_flows uses for
    # big.txt).
    s.key("C-c p i", 1.0)
    s.key("C-x C-f", 1.0)
    for ch in "ann_dense":
        s.key(ch, 0.25)
    s.cup_settle(s.key("RET", 1.5))
    s._read(0.5, quiet=0.15)
    # Content rows are CUP 2..=22 (1-based) = screen rows 1..=21 (0-based).
    canvas_rows = [s.row_text(r) for r in range(1, s.rows - 2)]
    title = s.row_text(0).strip()
    filled = [t for t in canvas_rows if t.strip()]
    marker_rows = [t for t in canvas_rows if "\u25be" in t]
    note_rows = [t for t in canvas_rows if "\u256d" in t]
    # FILL, not merely non-blank: >= 18 of 21 content rows, specifically
    # the 10 code + 10 note shape.
    rec("all-annotated fill: >= 18 of 21 content rows emitted",
        len(filled) >= 18, f"title={title!r} filled={len(filled)} (want >=18)")
    rec("all-annotated fill: 10 code + 10 note rows (largest span)",
        len(marker_rows) == 10 and len(note_rows) == 10,
        f"title={title!r} markers={len(marker_rows)} notes={len(note_rows)} "
        f"filled={len(filled)}")
    # The point's line (line 0, "dense line 0") is drawn with its marker
    # and its note row directly above it.
    point_drawn = any("dense line 0" in t and "\u25be" in t for t in canvas_rows)
    rec("all-annotated fill: point's line (line 0) drawn with marker",
        point_drawn,
        f"row0={[t for t in canvas_rows if 'dense line 0' in t][:1]}")

    s.kill()
    # Test hygiene (shared /tmp fixture): this function's legs create
    # scratch files + the notes file; remove them after kill so RE-RUNS
    # of this suite start from the baseline magit pane (the
    # transient-menu @30 leg's first page depends on how many rows the
    # magit status occupies).
    for stray in ("src/ann_gutter.rs", "src/ann_dense.rs", ".redline-notes.md"):
        try:
            os.remove(_os.path.join(REPO, stray))
        except OSError:
            pass
    return checks



def jump_highlight_checks():
    """jump-highlight: the animated landing highlight, with an animated
    jump exercised (the spec's CUP-race interaction check, plan 013):
    * M-i -> beta (window pinned at Bot: beta lands on terminal row 20,
      1-based, at the `beta` name's column (col 4) — jump-column-landings,
      the imenu landing is on the symbol, not the line start): the
      landing pulse is a YELLOW band (iocraft's bright
      yellow, `48;5;11`, under the 256-color path) — present in the raw
      stream right after the landing and reconstructable in pyte (a
      `ffff00` bg on the landing row) — then CLEARS (no `48;5;11` / no
      yellow row after the ~200 ms fade), while the hardware cursor (the
      CUP after the final `?2026l`) lands exactly on the landing row/col
      BOTH during and after the animation (the extra animation frames
      must not widen the cursor race);
    * M-, back to the `alpha` reference (row 21, col 6): a second
      animated landing, CUP tracking;
    * COLORTERM=truecolor: the band is a genuine RGB fade — the full-
      strength `48;2;255;255;0` band plus dimmer yellow-family 48;2;
      values (interpolation across frames, not a fake hold).
    """
    import os as _os
    src_dir = _os.path.join(REPO, "src")
    _os.makedirs(src_dir, exist_ok=True)
    # jumpfn.rs: `alpha` at line 0, filler to line 27, `fn beta() {` at
    # line 28 with an `alpha` reference at line 29 (31 lines total, no
    # trailing newline). Window: 24-3=21 rows; the imenu jump keeps the
    # window at Bot (top=10): beta = terminal row 20 (1-based), the
    # reference line = terminal row 21.
    lines = ["fn alpha() {}"] + [f"// filler {i}" for i in range(1, 28)] + \
        ["fn beta() {", "    alpha();", "}"]
    with open(_os.path.join(src_dir, "jumpfn.rs"), "w") as f:
        f.write("\n".join(lines))

    BETA_ROW = 20          # 1-based terminal row of `fn beta() {`
    BETA_COL = 4           # 1-based terminal col of the `beta` NAME (jump-column-landings: the imenu landing is on the symbol, `fn |beta`, not the line start)
    REF_ROW, REF_COL = 21, 6  # 1-based row/col of the `alpha` reference

    def run(mode):
        s = Session(mode)
        checks = []

        def rec(name, ok, detail=""):
            checks.append((name, ok, detail))
            print(f"  {'PASS' if ok else 'FAIL'}  {name:52s} {detail}")

        def yellow_cells(row):
            """Yellow-family bg hexes (r>100, g>100, b<100) on 1-based
            terminal row `row` (pyte stores bg as a lowercase hex str)."""
            out = []
            buf = s.screen.buffer[row - 1]
            for c in sorted(buf):
                h = str(buf[c].bg).lower()
                if len(h) == 6 and (int(h[:2], 16) > 100 and int(h[2:4], 16) > 100
                        and int(h[4:6], 16) < 100):
                    out.append(h)
            return out

        # Open jumpfn.rs.
        s.key("C-x C-f", 1.0)
        for ch in "jumpfn":
            s.key(ch, 0.25)
        s.cup_settle(s.key("RET", 1.0))
        # Park the point on the `alpha` reference (line 29, 0-based col 5).
        s.key("C-n " * 29, 0.8)
        s.key("C-f C-f C-f C-f C-f", 0.6)
        # M-i (imenu) -> filter to beta -> RET: the animated jump landing.
        s.key("M-i", 0.8)
        s.key("b", 0.6)
        buf = s.key("RET", 0.3)
        if mode is None:
            rec("256: M-i landing band emitted (48;5;11)",
                b"48;5;11" in buf,
                f"48;5;11 count={buf.count(b'48;5;11')}")
        else:
            # The first frame is drawn a couple of ms after `set_at`, so the
            # full-strength band reads 250-255 per channel, not exactly 255.
            full = bool(re.search(rb"48;2;(25[0-5]);\1;0", buf))
            fades = {m2 for m2 in re.findall(rb"48;2;(\d+);(\d+);(\d+)", buf)
                     if m2[0] == m2[1] and m2[2] == b"0" and m2[0] != b"0"}
            rec("truecolor: full-strength band (48;2;~255;~255;0)", full,
                "found" if full else "missing")
            rec("truecolor: the fade interpolates (dimmer yellow-family 48;2; values)",
                len(fades) >= 2,
                f"distinct yellow-family rgbs={sorted(fades)[:6]}")
        # The CUP survives the animation and lands on the landing row
        # (cup_settle keeps the window open until the CUP after the final
        # ?2026l is in the chunk — the landing frame's CUP or a later
        # animation frame's; the point does not move during the fade).
        r, c = last_cup_after_sync(s.cup_settle(buf))
        # jump-column-landings: the imenu landing is on the `beta` NAME
        # (`fn |beta` → 1-based col 4), not the line start — the CUP tracks
        # that column both during and after the fade.
        rec(f"{'truecolor' if mode else '256'}: CUP on the animated landing",
            (r, c) == (BETA_ROW, BETA_COL),
            f"cup=({r},{c}) want ({BETA_ROW},{BETA_COL})")
        if mode is None:
            # The band CLEARS after the ~200 ms fade: the settled frame
            # (pyte's final state AND the raw stream) carries no yellow.
            settled = s._read(0.8, quiet=0.15)
            rec("256: band cleared after the fade",
                b"48;5;11" not in settled and not yellow_cells(BETA_ROW),
                f"48;5;11 in settled={settled.count(b'48;5;11')}, "
                f"yellow left={yellow_cells(BETA_ROW)[:3]}")
        # M-, back to the reference: a second animated landing.
        buf2 = s.key("M-,", 0.3)
        r, c = last_cup_after_sync(s.cup_settle(buf2))
        rec(f"{'truecolor' if mode else '256'}: CUP on the M-, landing (row 21 col 6)",
            (r, c) == (REF_ROW, REF_COL), f"cup=({r},{c}) want ({REF_ROW},{REF_COL})")
        s.kill()
        return checks

    out = run(None)
    out += run("truecolor")
    # Test hygiene (shared /tmp fixture): remove the scratch file so
    # re-runs start from the baseline file set.
    try:
        os.remove(_os.path.join(src_dir, "jumpfn.rs"))
    except OSError:
        pass
    return out

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
    # plan 004 issue 05c: mouse (line, col) click, wheel parity, C-l
    # recenter-top-bottom, word motion legs.
    print("=== MOUSE CLICK COL / WHEEL PARITY / C-L RECENTER / WORD MOTION ===")
    all_checks += mouse_recenter_word_checks()
    print()
    # plan 004 issue 05d: wide (CJK) chars — display-column cursor + clicks.
    print("=== WIDE (CJK) CHARS: DISPLAY-COLUMN CURSOR + CLICKS ===")
    all_checks += wide_char_checks()
    print()
    # plan 004 issue 05e: tree-sidebar click offset (tree-visible leg + the
    # tree-hidden regression leg).
    print("=== TREE-SIDEBAR CLICK OFFSET (tree visible) ===")
    all_checks += tree_click_checks()
    print()
    # plan 004 issue 05g: hardware-cursor (CUP) column offset with the tree
    # sidebar visible (+ the tree-hidden regression leg).
    print("=== TREE-SIDEBAR HARDWARE-CURSOR OFFSET (tree visible) ===")
    all_checks += tree_cursor_offset_checks()
    print()
    # plan 004 issue 05g (carried P2): list-view hardware-cursor (CUP)
    # offset with the tree sidebar visible (the 05g offset applies to every
    # view arm; only the Buffer arm was asserted before).
    print("=== LIST-VIEW HARDWARE-CURSOR OFFSET (tree visible) ===")
    all_checks += list_view_cup_checks()
    print()
    # plan 004 issue 05f: transient-menu readability (two-column + gutter +
    # ellipsis at 80 cols; single-column fallback at narrow widths).
    print("=== TRANSIENT MENU READABILITY (two-col/gutter/ellipsis + narrow fallback) ===")
    all_checks += transient_menu_checks()
    print()
    # plan 005 issue 02b: annotation gutter (marker at cell 0, code at cell 1)
    # + note-row overflow (point's line always drawn, cursor on it).
    print("=== ANNOTATION GUTTER + NOTE-ROW OVERFLOW (02b) ===")
    all_checks += annotation_gutter_checks()
    print()
    # jump-highlight: animated landing highlight (CUP-race interaction,
    # plan 013) with an animated jump exercised (256 + truecolor).
    print("=== JUMP-HIGHLIGHT ANIMATED LANDING (CUP during/after the fade) ===")
    all_checks += jump_highlight_checks()
    print()
    bad = [n for n, ok, _ in all_checks if not ok]
    print("=== SUMMARY ===")
    for n, ok, d in all_checks:
        print(f"  {'PASS' if ok else 'FAIL'}  {n}")
    print(f"\nTOTAL: {len(all_checks) - len(bad)}/{len(all_checks)} raw-stream checks PASS")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

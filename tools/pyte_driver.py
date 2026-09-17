"""PTY + pyte driver for the redline TUI binary.

Spawns `redline` in a sized PTY (cwd = repo), feeds keystrokes, and
reconstructs the screen with pyte so we can inspect per-cell fg/bg/reverse.

The selected-row cursor bar is a per-row View with the selected face's
background. For the default (dark) theme that is White-on-Blue: iocraft
emits fg `38;5;15` -> pyte `ffffff`, bg `48;5;12` -> pyte `5c5cff`.
So the discriminating signature of the cursor row is bg == '5c5cff'.
"""
import os, pty, fcntl, termios, struct, time, select, signal, sys

import pyte

BIN = os.environ.get("REDLINE_BIN", "/home/gary/dev/red/target/debug/redline")
COLS = int(os.environ.get("COLS", "80"))
ROWS = int(os.environ.get("ROWS", "24"))
BLUE_BG = "5c5cff"  # pyte's rendering of iocraft Color::Blue (48;5;12)


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
    def __init__(self, repo, rows=ROWS, cols=COLS, startup_wait=6.0):
        self.repo = repo
        self.rows = rows
        self.cols = cols
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
            self._read(0.4, quiet=0.2)
            text = self.screen_text()
            # `ready` is the idle mode-line token; `indexing` means still busy.
            if "ready" in text and "indexing" not in text:
                return True
        return False

    def _read(self, timeout, quiet=0.0):
        deadline = time.time() + timeout
        last = time.time()
        while True:
            remain = deadline - time.time()
            if remain <= 0:
                break
            r, _, _ = select.select([self.master], [], [], min(remain, 0.1))
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

    def key(self, s, settle=1.0):
        os.write(self.master, encode_key(s))
        self._read(settle, quiet=0.2)

    def feed(self, data, settle=1.0):
        os.write(self.master, data)
        self._read(settle, quiet=0.2)

    def wait(self, t=1.0):
        self._read(t, quiet=0.2)

    def wait_done(self, timeout=15.0):
        """Wait until a streamed search has finished: the title stops showing
        'searching' and a match count is present."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            self._read(0.4, quiet=0.2)
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
                if str(row[i].bg).lower() == BLUE_BG:
                    out.append(r)
                    break
        return out

    def blue_count(self, row):
        """Number of blue-bg cells in a row."""
        return sum(1 for i in range(self.cols)
                   if str(self.screen.buffer[row][i].bg).lower() == BLUE_BG)

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

#!/usr/bin/env python3
"""Plan 005 issue 03 — quit-dump stdout pipe probe.

Forks the real binary with its controlling terminal on a PTY slave and its
stdout on a PIPE (stdin + stderr on the pty), so the app sees "stdout is
not a tty" and must reroute its render stream to /dev/tty (the fd-1
dup2). The pipe then carries ONLY the quit-dump — the TUI frames stay on
the pty master. Legs:

  1. block (default format): seed two records (one valid anchor, one that
     will orphan on load), open the file, add ONE live annotation via the
     `A` prompt, quit with C-x C-c, and assert:
       * the pipe carries the agent block (header + path:line + anchored
         code + NOTE lines, 1-based lines, ORPHANED marker for the stale
         record) with ZERO escape bytes;
       * the pty stream still contains the TUI frames (alt-screen enter)
         and does NOT contain any dump text.
  2. plain: `--notes=plain` prints the `path:line: text` grep/pipe shape
     (no header, no code/orphan lines), zero escape bytes.
  3. empty: no annotations → the pipe gets ZERO bytes (pipes stay clean).
  4. unknown arg: `redline --bogus` exits 2 with usage on stderr, before
     the TUI starts.

The probe drives the shared fixture repo under the shared PTY flock (via
pyte_driver), resets the fixture baseline at start, and removes its own
stray files (src/dumpleg.rs, .redline-notes.md) at the end. Wrap the
invocation in `timeout`.

Exit 0 = all assertions pass; 1 = any failed.
"""
import os, sys, pty, fcntl, termios, struct, time, select, signal
import subprocess

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pyte_driver
from pyte_driver import encode_key
from fixture import repo, reset

BIN = os.environ.get("REDLINE_BIN",
                     os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                  "..", "target", "debug", "redline"))
REPO = os.environ.get("REDLINE_REPO") or repo("redline_pyte_repo")
COLS, ROWS = 80, 24

LEG_RS = "src/dumpleg.rs"
LEG_CONTENT = "fn alpha() {\n    let x = 1;\n}"
SEED_NOTES = (
    "<!-- redline-annotations:begin -->\n"
    "[annotation]\n"
    "path: src/dumpleg.rs\n"
    "line: 0\n"
    "col: 0\n"
    "anchor: fn alpha() {\n"
    "note: seeded anchor note\n"
    "orphaned: false\n"
    "[annotation]\n"
    "path: src/dumpleg.rs\n"
    "line: 1\n"
    "col: 0\n"
    "anchor: GHOST ANCHOR LINE\n"
    "note: seeded orphan note\n"
    "orphaned: false\n"
    "<!-- redline-annotations:end -->\n"
)

CHECKS = []


def rec(name, ok, detail=""):
    CHECKS.append((name, ok, detail))
    print(f"  {'PASS' if ok else 'FAIL'}  {name:56s} {detail}")


def _set_winsize(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


class PipedSession:
    """The app with its controlling terminal on a pty and stdout on a pipe."""

    def __init__(self, args):
        pyte_driver._acquire_fixture_lock(REPO)  # the suite owns the fixture
        self.master, self.slave = pty.openpty()
        _set_winsize(self.slave, ROWS, COLS)
        _set_winsize(self.master, ROWS, COLS)
        self.pipe_r, pipe_w = os.pipe()
        self.pid = os.fork()
        if self.pid == 0:
            os.setsid()
            fcntl.ioctl(self.slave, termios.TIOCSCTTY, 0)
            os.dup2(self.slave, 0)
            os.dup2(pipe_w, 1)          # stdout -> pipe (NOT a tty)
            os.dup2(self.slave, 2)
            if self.slave > 2:
                os.close(self.slave)
            os.close(self.master)
            os.close(self.pipe_r)
            os.close(pipe_w)            # the dup on fd 1 is enough
            env = dict(os.environ)
            env["TERM"] = "xterm-256color"
            env["RUST_LOG"] = "info"
            env.pop("COLORTERM", None)
            os.chdir(REPO)
            os.execvpe(BIN, [BIN] + args, env)
        os.close(self.slave)
        os.close(pipe_w)
        self.raw = b""                  # every byte that reached the pty
        import pyte
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.ByteStream(self.screen)
        if not self.wait_ready(30.0):
            raise RuntimeError("app did not reach ready state")

    def _read(self, timeout, quiet=0.15):
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
                self.raw += data
                buf += data
                self.stream.feed(data)
                last = time.time()
            if (time.time() - last) >= quiet:
                break
        return buf

    def wait_ready(self, timeout=30.0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            self._read(0.5)
            text = "\n".join("".join(self.screen.buffer[r][i].data
                                     for i in range(COLS)) for r in range(ROWS))
            if "ready" in text and "indexing" not in text:
                return True
        return False

    def key(self, s, settle=0.8):
        os.write(self.master, encode_key(s))
        return self._read(settle)

    def open_leg_file(self):
        self.key("C-x C-f", 1.0)
        for ch in "dumpleg":
            self.key(ch, 0.25)
        self.key("RET", 1.2)

    def add_live_annotation(self):
        # Point at line 2 (the `}` line — lines 0 and 1 already carry the
        # seeded records): A → empty prompt → type the note → RET commits.
        self.key("C-n C-n", 0.6)
        self.key("A", 0.5)
        os.write(self.master, b"live note here\r")
        self._read(1.0)

    def quit_and_capture_dump(self):
        """C-x C-c, wait for exit, drain the pipe (the dump) to EOF."""
        os.write(self.master, encode_key("C-x C-c"))
        os.waitpid(self.pid, 0)          # bounded by the outer `timeout`
        dump = b""
        while True:
            chunk = os.read(self.pipe_r, 65536)
            if not chunk:
                break
            dump += chunk
        os.close(self.pipe_r)
        tail = self._read(0.5, quiet=0.1)  # any teardown bytes on the pty
        self.raw += tail
        return dump

    def kill(self):
        try:
            os.kill(self.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            os.waitpid(self.pid, 0)
        except (ChildProcessError, OSError):
            pass
        for fd in (self.master, self.pipe_r):
            try:
                os.close(fd)
            except OSError:
                pass


def _write_fixture_files(with_notes):
    src = os.path.join(REPO, "src")
    os.makedirs(src, exist_ok=True)
    with open(os.path.join(src, "dumpleg.rs"), "w") as f:
        f.write(LEG_CONTENT)
    notes = os.path.join(REPO, ".redline-notes.md")
    if with_notes:
        with open(notes, "w") as f:
            f.write(SEED_NOTES)
    elif os.path.exists(notes):
        os.remove(notes)


def _cleanup_strays():
    for stray in (LEG_RS, ".redline-notes.md"):
        try:
            os.remove(os.path.join(REPO, stray))
        except OSError:
            pass


def block_leg():
    """Default format: the agent block on the pipe, frames on the pty."""
    print("=== LEG 1: block dump on the stdout pipe, frames on the pty ===")
    _write_fixture_files(with_notes=True)
    s = PipedSession([])
    s.open_leg_file()
    s.add_live_annotation()
    dump = s.quit_and_capture_dump()
    text = dump.decode("utf-8", "replace")
    em = "\u2014"
    header = f"# redline annotations {em} {REPO}\n\n"
    rec("block: header names the project root", dump.startswith(header.encode()),
        f"dump[:60]={dump[:60]!r}")
    block1 = (b"src/dumpleg.rs:1\n"
              b"    fn alpha() {\n"
              b"  NOTE: seeded anchor note\n")
    rec("block: seeded record — path:line(1-based) + code + NOTE", block1 in dump,
        f"missing={block1!r}")
    block2 = (b"src/dumpleg.rs:2\n"
              b"  ORPHANED (anchor text not found)\n"
              b"    GHOST ANCHOR LINE\n"
              b"  NOTE: seeded orphan note\n")
    rec("block: orphaned record carries the explicit marker", block2 in dump,
        f"dump={text!r}")
    block3 = (b"src/dumpleg.rs:3\n"
              b"    }\n"
              b"  NOTE: live note here\n")
    rec("block: the live `A`-prompt annotation is in the final dump",
        block3 in dump, f"dump={text!r}")
    ordered = (dump.find(b"src/dumpleg.rs:1") < dump.find(b"src/dumpleg.rs:2")
               < dump.find(b"src/dumpleg.rs:3"))
    rec("block: ordered path asc then line asc", ordered, f"dump={text!r}")
    rec("block: pipe carries ZERO escape bytes", b"\x1b" not in dump,
        f"esc count={dump.count(bytes([0x1b]))}")
    rec("pty: TUI frames still render (alt-screen enter on the tty)",
        b"\x1b[?1049h" in s.raw, f"raw bytes={len(s.raw)}")
    rec("pty: NO dump text leaked onto the tty",
        b"# redline annotations" not in s.raw and b"NOTE:" not in s.raw,
        f"raw tail={s.raw[-40:]!r}")
    s.kill()


def plain_leg():
    """--notes=plain: the grep/pipe shape, no header, no escape bytes."""
    print("\n=== LEG 2: --notes=plain ===")
    _write_fixture_files(with_notes=True)
    s = PipedSession(["--notes=plain"])
    s.open_leg_file()
    s.add_live_annotation()
    dump = s.quit_and_capture_dump()
    text = dump.decode("utf-8", "replace")
    lines = dump.decode("utf-8", "replace").splitlines()
    rec("plain: line 1 is `path:line: text`",
        lines and lines[0] == "src/dumpleg.rs:1: seeded anchor note",
        f"first={lines[:1]}")
    rec("plain: every record as `path:line: text` (3 records)",
        "src/dumpleg.rs:2: seeded orphan note" in text
        and "src/dumpleg.rs:3: live note here" in text,
        f"lines={lines}")
    rec("plain: no header, no code/orphan lines",
        "# redline" not in text and "ORPHANED" not in text
        and "fn alpha" not in text,
        f"text={text!r}")
    rec("plain: ZERO escape bytes", dump.count(bytes([0x1b])) == 0,
        f"esc count={dump.count(bytes([0x1b]))}")
    s.kill()


def empty_leg():
    """No annotations → the pipe gets ZERO bytes."""
    print("\n=== LEG 3: no annotations → 0 bytes on the pipe ===")
    _write_fixture_files(with_notes=False)
    s = PipedSession([])
    dump = s.quit_and_capture_dump()
    rec("empty: the pipe carried exactly 0 bytes", dump == b"",
        f"len={len(dump)} dump={dump[:80]!r}")
    rec("empty: frames still rendered on the pty", b"\x1b[?1049h" in s.raw,
        f"raw bytes={len(s.raw)}")
    s.kill()


def unknown_arg_leg():
    """Unknown args are a hard error: usage on stderr, exit 2, no TUI."""
    print("\n=== LEG 4: unknown argument → exit 2 ===")
    r = subprocess.run([BIN, "--bogus"], cwd=REPO, capture_output=True,
                       timeout=20)
    rec("unknown arg: exit code 2", r.returncode == 2,
        f"rc={r.returncode} stderr={r.stderr[:80]!r}")
    rec("unknown arg: usage on stderr names the only flag",
        b"unknown argument: --bogus" in r.stderr
        and b"--notes=plain" in r.stderr,
        f"stderr={r.stderr[:120]!r}")
    rec("unknown arg: nothing on stdout (TUI never started)",
        r.stdout == b"", f"stdout={r.stdout[:80]!r}")


def main():
    print(f"BIN={BIN}\nREPO={REPO}\n")
    pyte_driver._acquire_fixture_lock(REPO)
    _cleanup_strays()
    reset()
    try:
        block_leg()
        plain_leg()
        empty_leg()
        unknown_arg_leg()
    finally:
        _cleanup_strays()
    bad = [n for n, ok, _ in CHECKS if not ok]
    print("\n=== SUMMARY ===")
    for n, ok, d in CHECKS:
        print(f"  {'PASS' if ok else 'FAIL'}  {n}")
    print(f"\nTOTAL: {len(CHECKS) - len(bad)}/{len(CHECKS)} quit-dump checks PASS")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

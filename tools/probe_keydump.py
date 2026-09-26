#!/usr/bin/env python3
"""Measure the terminal event/char path: run the keydump example under a PTY,
feed RAW BYTES, and print exactly what crossterm decodes them into. Bounded:
keydump has a 4 s internal deadline; the driver also kills it after a hard cap.
Prints once and returns.
"""
import os, pty, fcntl, termios, struct, time, select, signal, sys

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "target", "debug", "examples", "keydump"))
COLS, ROWS = 80, 24

def setwinsz(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

master, slave = pty.openpty()
setwinsz(slave, ROWS, COLS)
setwinsz(master, ROWS, COLS)
pid = os.fork()
if pid == 0:
    os.setsid()
    fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    os.dup2(slave, 0)
    os.dup2(slave, 1)
    os.dup2(slave, 2)
    if slave > 2: os.close(slave)
    os.close(master)
    env = dict(os.environ)
    env["TERM"] = "xterm-256color"
    env.pop("COLORTERM", None)
    os.execve(BIN, [BIN], env)
os.close(slave)

def read_all(timeout):
    buf = b""
    deadline = time.time() + timeout
    while time.time() < deadline:
        r, _, _ = select.select([master], [], [], 0.1)
        if r:
            try:
                d = os.read(master, 65536)
            except OSError:
                break
            if not d: break
            buf += d
    return buf

# Give it a moment to enable raw mode and spin up the event reader.
read_all(0.6)

# The feed: plain chars, then the two newline bytes, then more chars.
feed = b"abc" + b"\n" + b"\r" + b"def"
os.write(master, feed)

out = read_all(5.0)  # keydump's own 4 s deadline fires before this

try:
    os.kill(pid, signal.SIGKILL)
except ProcessLookupError:
    pass
try:
    os.waitpid(pid, 0)
except ChildProcessError:
    pass
os.close(master)

sys.stdout.write("=== keydump decoded events (fed bytes: %r) ===\n" % feed)
for line in out.decode("utf-8", "replace").splitlines():
    line = line.strip()
    if line:
        sys.stdout.write(line + "\n")
sys.stdout.write("=== end ===\n")

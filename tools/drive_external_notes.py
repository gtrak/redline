#!/usr/bin/env python3
"""Plan 008 issue 01 — annotation flow on EXTERNAL (library) buffers.

Drives a REAL M-. resolver landing into a cargo-registry source (read-only
external buffer) and exercises the whole annotation machinery that pre-008
was keyed off project-relative paths (dead there):

  E1 M-. lands external: in a dedicated repo (NOT the shared fixture — the
     fixture has no Cargo.toml, and these legs need a cargo graph) the
     cursor sits at the end of `ropey` in `ropey::Rope::new()`. M-. misses
     the project symbol index and the rust provider resolves via
     `cargo metadata` (the registry source is cached, so it is fast and
     offline) → lands READ-ONLY in
     ~/.cargo/registry/src/<hash>/ropey-1.6.1/src/rope.rs.
  E2 annotate: `A` on the landing line → type the note → RET: the margin
     marker AND the inline note row render on the external buffer (the
     record is keyed by the ABSOLUTE path, stored in the project's
     .redline-notes.md).
  E3 delete: `d` removes the record (echo), the note row disappears.
  E4 dump: re-add the note, quit (C-x C-c) with stdout on a pipe — the
     dump carries the record's path VERBATIM (the absolute registry path)
     with the anchored code line, both the project header and the NOTE
     text intact.

The drive owns its own repo dir (/tmp/redline_ext_repo) under the shared
PTY-flock scheme (the lock is keyed to the repo path, so it never
contends with the /tmp/redline_pyte_repo suites) and removes the repo on
exit. Wrap the invocation in `timeout`.

Exit 0 = all legs pass; 1 = any failed.
"""
import os, re, sys, pty, fcntl, termios, struct, time, select, signal, shutil
import subprocess

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pyte_driver
import pyte
from pyte_driver import encode_key

BIN = os.environ.get("REDLINE_BIN",
                     os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                  "..", "target", "debug", "redline"))
REPO = "/tmp/redline_ext_repo"
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


def _set_winsize(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


class Session:
    """The app with its controlling terminal on a pty and stdout on a
    pipe (the pipe carries ONLY the quit-dump — the TUI frames stay on
    the pty master), mirroring tools/probe_notes_dump.py."""

    def __init__(self):
        pyte_driver._acquire_fixture_lock(REPO)  # this suite owns REPO
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
            os.close(pipe_w)
            env = dict(os.environ)
            env["TERM"] = "xterm-256color"
            env["RUST_LOG"] = "info"
            env.pop("COLORTERM", None)
            os.chdir(REPO)
            os.execvpe(BIN, [BIN], env)
        os.close(self.slave)
        os.close(pipe_w)
        self.raw = b""
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
            text = self.screen_text()
            if "ready" in text and "indexing" not in text:
                return True
        return False

    def key(self, s, settle=0.8):
        os.write(self.master, encode_key(s))
        return self._read(settle)

    def row_text(self, row):
        return "".join(self.screen.buffer[row][i].data for i in range(COLS))

    def screen_text(self):
        return "\n".join(self.row_text(r) for r in range(ROWS))

    def content_text(self):
        """Content rows only (excludes the minibuffer + status rows)."""
        return "\n".join(self.row_text(r) for r in range(ROWS - 2))

    def poll(self, needle, timeout=30.0):
        """Wait until `needle` appears in the minibuffer row."""
        deadline = time.time() + timeout
        last = ""
        while time.time() < deadline:
            self._read(0.4, quiet=0.2)
            last = self.row_text(MINI)
            if needle in last:
                return True, last
        return False, last

    def quit_and_capture_dump(self):
        os.write(self.master, encode_key("C-x C-c"))
        os.waitpid(self.pid, 0)
        dump = b""
        while True:
            chunk = os.read(self.pipe_r, 65536)
            if not chunk:
                break
            dump += chunk
        os.close(self.pipe_r)
        tail = self._read(0.5, quiet=0.1)
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


def main():
    print(f"BIN={BIN}\nREPO={REPO}\n")
    if not os.path.exists(ROPEY_SRC):
        print(f"FAIL ropey registry source missing at {ROPEY_SRC}")
        sys.exit(1)
    setup_repo()
    s = None
    try:
        s = Session()

        # ── E1: M-. lands in the registry source (read-only external) ───
        print("=== E1: M-. into the ropey registry source ===")
        open_main(s)
        s.key("M-g g", 0.8)
        s.key("5", 0.4)
        s.key("RET", 0.8)   # line 5: top-level `ropey::Rope::new();` probe call
        s.key("M-f", 0.8)   # let, r, then the END of the `ropey` run
        s.key("M-.", 1.5)
        ok, msg = s.poll("jumped to", timeout=40.0)
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
        ok = "Rope" in s.screen_text()
        rec("E1: the window landed on rope.rs (Rope in view)", ok,
            f"top={s.row_text(1)!r}")

        # ── E2: annotate on the external buffer ─────────────────────────
        print("\n=== E2: A on the external buffer renders marker + note row ===")
        s.key("A", 0.5)
        os.write(s.master, b"external note\r")
        s._read(1.2)
        ok, msg = s.poll("note saved", timeout=10.0)
        rec("E2: RET commits on the external buffer", ok,
            f"minibuffer={msg!r}")
        rec("E2: the absolute path is the record key in the save echo",
            bool(landed) and f"in {landed}" in msg,
            f"minibuffer={msg!r}")
        ok = "▎" in s.content_text()
        rec("E2: the margin marker renders on the external buffer", ok,
            f"content has {'▎'!r}={ok}")
        ok = "external note" in s.content_text()
        rec("E2: the inline note row renders on the external buffer", ok,
            f"content has note={ok}")

        # ── E3: d removes the annotation ─────────────────────────────────
        print("\n=== E3: d deletes the external-buffer annotation ===")
        s.key("d", 1.0)
        ok, msg = s.poll("deleted annotation", timeout=10.0)
        rec("E3: d echoes the removed note", ok and "external note" in msg,
            f"minibuffer={msg!r}")
        ok = "external note" not in s.content_text()
        rec("E3: the note row is gone from the external buffer", ok,
            f"content has note={not ok}")

        # ── E4: the quit dump carries the absolute path verbatim ────────
        print("\n=== E4: quit dump carries the absolute path + code line ===")
        s.key("A", 0.5)
        os.write(s.master, b"external note\r")
        s._read(1.2)
        ok, msg = s.poll("note saved", timeout=10.0)
        rec("E4: re-annotated before quit", ok, f"minibuffer={msg!r}")
        dump = s.quit_and_capture_dump()
        text = dump.decode("utf-8", "replace")
        # The anchored line (1-based) text from the registry source itself.
        with open(ROPEY_SRC) as f:
            rope_lines = f.read().split("\n")
        anchored = rope_lines[landed_line - 1] if landed_line else "?"
        abs_re = re.escape(ROPEY_SRC)
        m = re.search(rf"^{abs_re}:(\d+)\n(    .*\n)  NOTE: external note\n",
                      text, re.M)
        rec("E4: the dump prints the record's path VERBATIM (absolute)",
            bool(re.search(rf"^{abs_re}:\d+\n", text, re.M)),
            f"dump={text!r}")
        rec("E4: the dump's code line is the anchored registry line",
            bool(m) and m.group(2).strip() == anchored,
            f"anchored={anchored!r} match={bool(m)}")
        rec("E4: the project-root header is unchanged",
            text.startswith(f"# redline annotations \u2014 {REPO}\n\n"),
            f"head={text[:60]!r}")
        rec("E4: ZERO escape bytes on the pipe",
            b"\x1b" not in dump, f"esc={dump.count(bytes([0x1b]))}")
    finally:
        if s is not None:
            s.kill()
        shutil.rmtree(REPO, ignore_errors=True)
    bad = [n for n, ok, _ in CHECKS if not ok]
    print("\n=== SUMMARY ===")
    for n, ok, _ in CHECKS:
        print(f"  {'PASS' if ok else 'FAIL'}  {n}")
    print(f"\n{len(CHECKS) - len(bad)}/{len(CHECKS)} external-notes legs passed")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

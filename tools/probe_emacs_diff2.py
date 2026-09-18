#!/usr/bin/env python3
"""Differential probe #2: behaviors NOT yet column-verified, run identically
on vanilla emacs (-Q -nw) and redline. Same method as probe_emacs_diff.py
(the method that found the 05c word-motion bug).

NOTE on the scroll legs (C-v/M-v/C-d/C-u): redline pins the point's SCREEN
ROW during a window scroll; emacs keeps the point's BUFFER line and only
clamps it to the window edge when it would scroll off. The rows below
therefore DIFF by design (documented divergence, parity-log row 32), not
because of a bug. C-d/C-u also differ by adoption (redline: half-page
scroll; emacs: delete-char / universal prefix).

Areas probed:
  * C-a / C-e on a line with content vs empty lines
  * C-f / C-b wrap at EOL / BOL
  * M-< / M-> (buffer start/end, and the trailing-newline empty last line)
  * C-v / M-v / C-d / C-u page scroll screen-row pinning
  * M-w / C-w / C-y kill-ring round trip (copy only in read-only emacs)
  * C-n / C-p goal column across a long/short/long line triple
  * C-g after a partial command leaves the point where it was

Diagnostic, not a gate. Run manually: python3 tools/probe_emacs_diff2.py
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from drive_emacs import EmacsSession, fixture as emacs_fixture
from pyte_driver import App as Redline
from fixture import reset as redline_reset

ROWS, COLS = 24, 80


class SizedEmacs(EmacsSession):
    """EmacsSession starts a pyte Screen at drive_emacs's fixed W,H (100x30);
    for a fair comparison with the redline probe (24x80) we resize the pty to
    24x80 immediately after fork and rebuild the Screen to match."""

    def __init__(self, cwd, args=None):
        import pty as _pty, fcntl as _fcntl, termios as _termios, struct as _struct
        import pyte as _pyte
        self.screen = _pyte.Screen(COLS, ROWS)
        self.stream = _pyte.ByteStream(self.screen)
        pid, fd = _pty.fork()
        if pid == 0:
            import os as _os
            _os.chdir(cwd)
            _os.environ['TERM'] = 'xterm-256color'
            argv = ['emacs', '-Q', '-nw'] + (args or [])
            _os.execvp(argv[0], argv)
        self.pid, self.fd = pid, fd
        _fcntl.ioctl(fd, _termios.TIOCSWINSZ,
                     _struct.pack('HHHH', ROWS, COLS, 0, 0))
        self.pump(1.0)
        _fcntl.ioctl(fd, _termios.TIOCSWINSZ,
                     _struct.pack('HHHH', ROWS, COLS, 0, 0))
        self.pump(1.5)

LINE = 'fn alpha() { let x = 1; }'
EMPTY = ''
TALL_LINES = (
    [LINE, 'bb', 'cccccccccccccccccccc', EMPTY, LINE]
    + [f'fn filler_{i}() {{ /* pad */ }}' for i in range(60)]
)


def write_fixture(root):
    os.makedirs(os.path.join(root, 'src'), exist_ok=True)
    with open(os.path.join(root, 'src', 'p2.rs'), 'w') as f:
        f.write('\n'.join(TALL_LINES) + '\n')


def run_emacs(root):
    write_fixture(root)
    s = SizedEmacs(root, ['src/p2.rs'])
    s.pump(1.0)
    out = {}
    s.send(b'\x1b<', 0.3)

    def col():
        return s.cursor()[0]

    def row():
        return s.cursor()[1]

    # C-e / C-a
    s.send(b'\x1b<', 0.2); s.send(b'\x01', 0.2)
    s.send(b'\x05', 0.3); out['C-e row0'] = col()
    s.send(b'\x01', 0.3); out['C-a row0'] = col()
    # C-f wrap: go to EOL then one more
    s.send(b'\x05', 0.2); s.send(b'\x06', 0.3); out['C-f at EOL'] = (row(), col())
    s.send(b'\x01', 0.2); s.send(b'\x02', 0.3); out['C-b at BOL'] = (row(), col())
    # nested empties: C-n x3 lands on the empty line 3 -> col should be 0
    s.send(b'\x1b<', 0.2); s.send(b'\x0e\x0e\x0e', 0.4); out['C-n x3 (empty ln)'] = (row(), col())
    s.send(b'\x0e', 0.3); out['C-n -> line4'] = (row(), col())
    # goal column: line0 col 12, C-n to short (clamp), C-n (restore), C-n to empty
    s.send(b'\x1b<', 0.2); s.send(b'\x01', 0.2); s.send(b'\x06' * 12, 0.4)
    out['goal set col12'] = (row(), col())
    s.send(b'\x0e', 0.3); out['goal C-n->bb'] = (row(), col())
    s.send(b'\x0e', 0.3); out['goal C-n->cccc'] = (row(), col())
    s.send(b'\x0e', 0.3); out['goal C-n->empty'] = (row(), col())
    # M-> (end of buffer: emacs lands on the final empty line if trailing \n)
    s.send(b'\x1b>', 0.5); out['M->'] = (row(), col(), s.lines()[row()][:10])
    s.send(b'\x1b<', 0.4); out['M-<'] = (row(), col())
    # page scroll: point's screen row pinned?
    s.send(b'\x0e' * 10, 0.5); out['C-n x10'] = (row(), col())
    s.send(b'\x16', 0.4); out['C-v'] = (row(), col())
    s.send(b'\x16', 0.4); out['C-v2'] = (row(), col())
    s.send(b'\x1bv', 0.4); out['M-v'] = (row(), col())
    s.send(b'\x04', 0.4); out['C-d'] = (row(), col())
    s.send(b'\x15', 0.4); out['C-u'] = (row(), col())
    # mark + copy (emacs read-only allows M-w copy; C-w refuses)
    s.send(b'\x1b<', 0.3); s.send(b'\x0e\x0e', 0.3)
    s.send(b'\x00', 0.3); out['C-SPC'] = (row(), col())
    s.send(b'\x0e\x0e', 0.4); out['mark extend'] = (row(), col())
    s.send(b'\x1bw', 0.4); out['M-w copy'] = (row(), col())   # copy, should not move
    s.send(b'\x07', 0.3); out['C-g'] = (row(), col())
    s.quit()
    return out


def run_redline(root):
    redline_reset()
    write_fixture(root)
    a = Redline(root, rows=ROWS, cols=COLS)
    a.key("C-x C-f"); a.key("p2.rs"); a.key("RET", settle=2.0)
    out = {}

    def col():
        return a.screen.cursor.x

    def row():
        return a.screen.cursor.y

    a.key("M-<")
    a.key("C-a"); a.key("C-e", settle=0.5); out['C-e row0'] = col()
    a.key("C-a", settle=0.5); out['C-a row0'] = col()
    a.key("C-e"); a.key("C-f", settle=0.5); out['C-f at EOL'] = (row(), col())
    a.key("C-a"); a.key("C-b", settle=0.5); out['C-b at BOL'] = (row(), col())
    a.key("M-<")
    for _ in range(3):
        a.key("C-n")
    out['C-n x3 (empty ln)'] = (row(), col())
    a.key("C-n", settle=0.5); out['C-n -> line4'] = (row(), col())
    a.key("M-<"); a.key("C-a")
    for _ in range(12):
        a.key("C-f")
    out['goal set col12'] = (row(), col())
    a.key("C-n"); out['goal C-n->bb'] = (row(), col())
    a.key("C-n"); out['goal C-n->cccc'] = (row(), col())
    a.key("C-n"); out['goal C-n->empty'] = (row(), col())
    a.key("M->", settle=0.7); out['M->'] = (row(), col(), a.row_text(row())[:10])
    a.key("M-<", settle=0.7); out['M-<'] = (row(), col())
    for _ in range(10):
        a.key("C-n")
    out['C-n x10'] = (row(), col())
    a.key("C-v", settle=0.6); out['C-v'] = (row(), col())
    a.key("C-v", settle=0.6); out['C-v2'] = (row(), col())
    a.key("M-v", settle=0.6); out['M-v'] = (row(), col())
    a.key("C-d", settle=0.6); out['C-d'] = (row(), col())
    a.key("C-u", settle=0.6); out['C-u'] = (row(), col())
    a.key("M-<"); a.key("C-n"); a.key("C-n")
    a.key("C-@", settle=0.5); out['C-SPC'] = (row(), col())
    a.key("C-n"); a.key("C-n")
    out['mark extend'] = (row(), col())
    a.key("M-w", settle=0.5); out['M-w copy'] = (row(), col())
    a.key("C-g", settle=0.5); out['C-g'] = (row(), col())
    a.kill()
    return out


if __name__ == '__main__':
    em = run_emacs('/tmp/emacs_probe2_repo')
    rl = run_redline('/tmp/redline_pyte_repo')
    keys = list(em.keys())
    print(f'{"probe":22} | {"emacs":>26} | {"redline":>26} | match')
    for k in keys:
        e, r = em.get(k), rl.get(k)
        print(f'{k:22} | {str(e):>26} | {str(r):>26} | {"OK" if e == r else "DIFF"}')

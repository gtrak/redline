#!/usr/bin/env python3
"""Differential probe: run the SAME controlled positions on vanilla emacs
(`-Q -nw`) and redline, and print a comparison table. This is the harness
that caught the 004-05c word-motion bug (redline landed on word STARTS
where emacs lands on word ENDS / the wrap case undershot) and the C-l
cycle-order difference (emacs default is middle -> top -> bottom).

Not part of the gate suite: it needs emacs installed, and it is
diagnostic, not a pass/fail gate. Run manually:
    python3 tools/probe_emacs_diff.py
"""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + '/tools')
sys.path.insert(0, '/home/gary/dev/red/tools')
from drive_emacs import EmacsSession, fixture as emacs_fixture
from pyte_driver import App as Redline
from fixture import reset as redline_reset

LINE = 'fn alpha() { let x = 1; }'          # len 25
TALL = '\n'.join([LINE] + [f'fn filler_{i}() {{ /* pad */ }}' for i in range(60)]) + '\n'


def write_fixture(root):
    os.makedirs(os.path.join(root, 'src'), exist_ok=True)
    with open(os.path.join(root, 'src', 'probe.rs'), 'w') as f:
        f.write(TALL)


def ecursor(s):
    x, y = s.cursor()                        # (col, row)
    return x, y


def probe_emacs(root):
    write_fixture(root)
    s = EmacsSession(root, ['src/probe.rs'])
    s.pump(1.0)
    out = {}
    # ---- word motion: place point at col via C-a + C-f*n, then M-f/M-b
    for col in (0, 2, 3, 8, 9, 12, 25):
        s.send(b'\x1b<', 0.3)                # M-< top (line 0)
        s.send(b'\x01', 0.2)                 # C-a
        if col:
            s.send(b'\x06' * col, 0.3)       # C-f * col
        _, before = ecursor(s)
        s.send(b'\x1bf', 0.4)                # M-f
        x, _ = ecursor(s)
        out[('M-f', col)] = x
        s.send(b'\x1b<', 0.3); s.send(b'\x01', 0.2)
        if col:
            s.send(b'\x06' * col, 0.3)
        s.send(b'\x1bb', 0.4)                # M-b
        x2, _ = ecursor(s)
        out[('M-b', col)] = x2
    # ---- C-l cycle: point at buffer line 5, window clamped at top
    s.send(b'\x1b<', 0.3)
    s.send(b'\x0e' * 5, 0.4)                 # C-n x5 -> line 5
    for i in range(4):
        s.send(b'\x0c', 0.5)
        out[('C-l', i)] = (s.cursor()[1], s.lines()[1][:22])   # (cursor row, top line)
    s.quit()
    return out


def probe_redline(root):
    write_fixture(root)
    a = Redline(root, rows=24, cols=80)
    a.key("C-x C-f"); a.key("probe.rs"); a.key("RET", settle=2.0)
    out = {}
    for col in (0, 2, 3, 8, 9, 12, 25):
        a.key("M-<"); a.key("C-a")
        for _ in range(col):
            a.key("C-f")
        a.key("M-f", settle=0.6)
        out[('M-f', col)] = a.screen.cursor.x
        a.key("M-<"); a.key("C-a")
        for _ in range(col):
            a.key("C-f")
        a.key("M-b", settle=0.6)
        out[('M-b', col)] = a.screen.cursor.x
    a.key("M-<")
    for _ in range(5):
        a.key("C-n")
    for i in range(4):
        a.key("C-l", settle=0.6)
        out[('C-l', i)] = (a.screen.cursor.y, a.row_text(1)[:22])
    a.kill()
    return out


if __name__ == '__main__':
    em = probe_emacs('/tmp/emacs_probe_repo')
    redline_reset()
    rl = probe_redline('/tmp/redline_pyte_repo')
    print(f'line = {LINE!r}  (len {len(LINE)})')
    print(f'{"op":4} {"from":>4} | {"emacs":>5} | {"redline":>7} | match')
    for col in (0, 2, 3, 8, 9, 12, 25):
        for op in ('M-f', 'M-b'):
            e, r = em[(op, col)], rl[(op, col)]
            print(f'{op:4} {col:>4} | {e:>5} | {r:>7} | {"OK" if e == r else "DIFF"}')
    print()
    print(f'{"C-l#":5} | {"emacs(row,top)":>22} | {"redline(row,top)":>22}')
    for i in range(4):
        print(f'{i:5} | {str(em[("C-l", i)]):>22} | {str(rl[("C-l", i)]):>22}')

#!/usr/bin/env python3
"""Redline battery mirrored on tools/drive_emacs.py (same pty+pyte method)."""
import sys, os, shutil, subprocess
sys.path.insert(0, '/tmp')
from uxdrive import Session
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fixture import repo

def fixture(root):
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root + '/src', exist_ok=True)
    subprocess.run(['git', 'init', '-q', root], check=True)
    with open(root + '/src/main.rs', 'w') as f:
        for i in range(1, 61):
            f.write(f'fn line_{i}() {{ // line {i}\n}}\n')
    with open(root + '/README.md', 'w') as f:
        f.write('# fixture\nsome words to search here\nmore words to search here\n')
    subprocess.run(['git', '-C', root, 'add', '-A'], check=True)
    subprocess.run(['git', '-C', root, '-c', 'user.email=t@t', '-c',
                    'user.name=t', 'commit', '-qm', 'base'], check=True)

def step(s, name, keys=b'', wait=0.8):
    if keys:
        s.send(keys, wait)
    print(f'=== {name} ===')
    print(s.show())
    print(f'cursor-cell: {s.screen.cursor.x},{s.screen.cursor.y}')
    print()

if __name__ == '__main__':
    root = repo('redline_parity_repo')
    fixture(root)
    s = Session(root)
    step(s, 'startup (file given)', b'', 1.2)          # observe startup state
    # Open src/main.rs (60 lines) before scroll/position/recenter legs
    step(s, 'C-x C-f (find file)', b'\x18\x06', 0.8)
    step(s, 'type main + RET (open src/main.rs)', b'main\r', 1.2)
    step(s, 'C-n x5', b'\x0e' * 5)
    # ── plan 004 row 9: scroll overlap (C-v / M-v leave 2 context rows) ──
    step(s, 'C-v (page down, 2-row overlap)', b'\x16')
    step(s, 'M-v (page up, 2-row overlap)', b'\x1bv')
    step(s, 'M-<', b'\x1b<')
    step(s, 'M->', b'\x1b>')
    # ── plan 004 row 11: position segment in status line ──
    # Observe the status line: at top it shows "Top", after C-n x5 it
    # shows "L6,8%"-style, at bottom it shows "Bot".
    step(s, 'position: after M-< (should show Top)', b'\x1b<')
    step(s, 'position: C-n x5 (should show L6,~8%)', b'\x0e' * 5)
    step(s, 'position: M-> (should show Bot)', b'\x1b>')
    # ── plan 004 row 8: C-l recenter cycle (top → middle → bottom → top) ──
    step(s, 'C-l cycle 1: at top → middle', b'\x0c')
    step(s, 'C-l cycle 2: at middle → bottom', b'\x0c')
    step(s, 'C-l cycle 3: at bottom → top', b'\x0c')
    # ── isearch (plan 004 row 1: bound-letter interception) ──
    step(s, 'C-s isearch', b'\x13')
    # 'line_5' contains 'n' and 'l' which are depth-1 leaf commands in the
    # file view; the isearch interception must extend the query with every
    # printable, not drop or dispatch them. With the old code the query
    # would be 'lie_5' (the 'n' vanished); with the fix it is 'line_5'.
    step(s, 'type line_5 (bound-letter interception)', b'line_5', 1.0)
    step(s, 'C-s again', b'\x13', 0.6)
    step(s, 'RET (end at match)', b'\r', 0.6)
    step(s, 'C-g', b'\x07')
    step(s, 'C-x C-f', b'\x18\x06', 1.0)
    step(s, 'type READ + RET', b'READ\r', 1.2)
    # ── plan 004 row 6: C-x b (buffer switch, already bound) ──
    step(s, 'C-x b (buffer list)', b'\x18\x62', 1.0)
    step(s, 'RET (back to main.rs)', b'\r', 1.0)
    step(s, 'M-g g 30 RET', b'\x1bgg30\r', 1.0)
    step(s, 'M-x', b'\x1bx', 1.0)
    step(s, 'M-x type goto', b'goto', 1.0)
    step(s, 'M-x C-g', b'\x07')
    step(s, 'C-x 2', b'\x18\x32')
    step(s, 'C-x o', b'\x18\x6f', 1.0)
    step(s, 'C-x C-c quit', b'\x18\x18', 1.0)

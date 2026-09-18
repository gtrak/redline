#!/usr/bin/env python3
"""Battery 3 (emacs reference) -- COLUMN-LEVEL motion, recenter, region,
mark, and word motion, with explicit cursor (row,col) capture at every
step. Fixes battery 2's gap (frames alone could not show columns).

Vanilla emacs -Q -nw. Same pty+pyte method as drive_emacs.py.
NOTE: EmacsSession.cursor() returns (x, y) = (col, row).
"""
import os, sys
sys.path.insert(0, os.path.dirname(__file__))
from drive_emacs import EmacsSession, fixture


def step(s, name, keys=b'', wait=0.8):
    if keys:
        s.send(keys, wait)
    lines = s.lines()
    x, y = s.cursor()          # x = col, y = row
    print(f'=== {name} ===')
    print(f'cursor(row={y},col={x})')
    for r in range(max(0, y - 1), min(len(lines), y + 2)):
        print(f'{r:3}| {lines[r][:100]!r}')
    print()


def make_tall(root):
    """Written BEFORE the emacs session starts so the file exists."""
    tall = os.path.join(root, 'src', 'tall.rs')
    with open(tall, 'w') as f:
        f.write('fn alpha() { let x = 1; }\n')       # row 0, len 25
        f.write('bb\n')                              # row 1, len 2  (short)
        f.write('fn gamma_delta() { let yy = 22; }\n')  # row 2, len 33
        f.write('cccccccccccccccccccc\n')            # row 3, len 20
        for i in range(60):
            f.write(f'fn filler_{i}() {{ /* pad */ }}\n')


if __name__ == '__main__':
    root = '/tmp/emacs_parity3_repo'
    fixture(root)
    make_tall(root)
    s = EmacsSession(root, ['src/tall.rs'])
    step(s, 'b3 startup (tall.rs)')
    s.send(b'\x0e\x0e', 0.6)       # C-n x2 -> row 2 (gamma, len 33)
    step(s, 'b3 C-n x2 (gamma line)')
    s.send(b'\x05', 0.6)           # C-e -> end of gamma
    step(s, 'b3 C-e (end of gamma line, col=33)')
    s.send(b'\x0e', 0.6)           # C-n -> short line bb (goal clamp to 2)
    step(s, 'b3 C-n to short line (goal col CLAMP)')
    s.send(b'\x0e', 0.6)           # C-n -> cccc line (goal restore to 20)
    step(s, 'b3 C-n to cccc line (goal col RESTORE)')
    s.send(b'\x10', 0.6)           # C-p -> back to bb (clamp)
    step(s, 'b3 C-p back to short (clamp)')
    s.send(b'\x10', 0.6)           # C-p -> gamma (restore 20)
    step(s, 'b3 C-p to gamma (restore)')
    # word motion with columns (row 3 has cccc...; go to alpha line first)
    s.send(b'\x1b<', 0.6)          # M-< top
    s.send(b'\x01', 0.5)           # C-a
    step(s, 'b3 C-a (col 0)')
    s.send(b'\x1bf', 0.6)
    step(s, 'b3 M-f (end of "fn" -> col 2)')
    s.send(b'\x1bf', 0.6)
    step(s, 'b3 M-f (-> col 7, start of "("? emacs: end of "alpha")')
    s.send(b'\x1bb', 0.6)
    step(s, 'b3 M-b (back)')
    # recenter (point stays, window moves)
    s.send(b'\x0e\x0e\x0e\x0e\x0e', 0.8)   # C-n x5 -> some row
    step(s, 'b3 C-n x5 (before recenter)')
    s.send(b'\x0c', 0.7); step(s, 'b3 C-l (-> middle)')
    s.send(b'\x0c', 0.7); step(s, 'b3 C-l (-> bottom)')
    s.send(b'\x0c', 0.7); step(s, 'b3 C-l (-> top)')
    # mark + region
    s.send(b'\x0e\x0e', 0.6); s.send(b'\x00', 0.5)   # C-SPC
    s.send(b'\x0e\x0e', 0.7)                          # extend
    step(s, 'b3 set mark + extend (region)')
    s.send(b'\x07', 0.5); step(s, 'b3 C-g (deactivate mark)')
    s.quit()
    print('=== battery3 (emacs) done ===')

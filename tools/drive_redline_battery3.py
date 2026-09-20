#!/usr/bin/env python3
"""Battery 3 (redline mirror) -- COLUMN-LEVEL motion, recenter, region,
word motion. Mirrors tools/drive_emacs_battery3.py step for step so the
two captures can be compared row/col.

IMPORTANT: only run this when the redline binary reflects the code under
test (the 004-05c lane rebuilds it). Region highlight appears as pyte
attributes, not text, so the region legs report the blue/attr scan.
"""
import os, sys
sys.path.insert(0, os.path.dirname(__file__))
from pyte_driver import App
from fixture import repo
from fixture import reset


def step(a, name, keys=None, settle=1.0):
    if keys:
        a.key(keys, settle=settle)
    x, y = a.screen.cursor.x, a.screen.cursor.y
    print(f'=== {name} ===')
    print(f'cursor(row={y},col={x})')
    for r in range(max(0, y - 1), min(a.rows, y + 2)):
        print(f'{r:3}| {a.row_text(r)[:100]!r}')
    print()


if __name__ == '__main__':
    reset()
    root = repo('redline_pyte_repo')
    # mirror the emacs fixture: a tall file with varied line lengths
    tall = os.path.join(root, 'src', 'tall.rs')
    with open(tall, 'w') as f:
        f.write('fn alpha() { let x = 1; }\n')          # len 25
        f.write('bb\n')                                 # len 2  (short)
        f.write('fn gamma_delta() { let yy = 22; }\n')  # len 33
        f.write('cccccccccccccccccccc\n')               # len 20
        for i in range(60):
            f.write(f'fn filler_{i}() {{ /* pad */ }}\n')
    a = App(root, rows=24, cols=80)
    a.key("C-x C-f"); a.key("tall.rs"); a.key("RET", settle=2.0)
    step(a, 'b3 startup (tall.rs)')
    a.key("C-n"); a.key("C-n"); step(a, 'b3 C-n x2 (gamma line)')
    a.key("C-e"); step(a, 'b3 C-e (end of gamma line, col=33)')
    a.key("C-n"); step(a, 'b3 C-n (goal col CLAMP)')
    a.key("C-n"); step(a, 'b3 C-n (goal col, next line)')
    a.key("C-p"); step(a, 'b3 C-p (clamp)')
    a.key("C-p"); step(a, 'b3 C-p (goal col RESTORE)')
    a.key("M-<"); a.key("C-a"); step(a, 'b3 M-< then C-a (col 0)')
    a.key("M-f"); step(a, 'b3 M-f (end of "fn" -> col 2)')
    a.key("M-f"); step(a, 'b3 M-f (end of "alpha" -> col 8)')
    a.key("M-b"); step(a, 'b3 M-b (start of "alpha" -> col 3)')
    for _ in range(5):
        a.key("C-n")
    step(a, 'b3 C-n x5 (before recenter)')
    a.key("C-l"); step(a, 'b3 C-l (-> middle)')
    a.key("C-l"); step(a, 'b3 C-l (-> bottom)')
    a.key("C-l"); step(a, 'b3 C-l (-> top)')
    a.key("C-n"); a.key("C-n")
    a.key("C-SPC" if False else "C-@", settle=1.0)   # set mark
    a.key("C-n"); a.key("C-n")
    step(a, 'b3 set mark + extend (region)')
    a.key("C-g"); step(a, 'b3 C-g (deactivate mark)')
    a.kill()
    print('=== battery3 (redline) done ===')

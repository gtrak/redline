#!/usr/bin/env python3
"""Battery 2 (redline mirror): same areas as drive_emacs_battery2.py."""
import os, sys
sys.path.insert(0, '/tmp')
from uxdrive import Session
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fixture import repo

def fixture(root):
    import shutil, subprocess
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root + '/src', exist_ok=True)
    subprocess.run(['git', 'init', '-q', root], check=True)
    with open(root + '/src/main.rs', 'w') as f:
        for i in range(1, 61):
            f.write(f'fn line_{i}() {{ // line {i}\n}}\n')
    with open(root + '/README.md', 'w') as f:
        f.write('# fixture\nsome words to search here\nmore words to search here\n')
    subprocess.run(['git', '-C', root, 'add', '-A'], check=True)
    subprocess.run(['git', '-C', root, '-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-qm', 'base'], check=True)

def step(s, name, keys=b'', wait=0.8):
    if keys:
        s.send(keys, wait)
    print(f'=== {name} ===')
    print(s.show() or '(blank)')
    print()

if __name__ == '__main__':
    root = repo('redline_parity2_repo')
    fixture(root)
    s = Session(root)
    s.pump(3.0)
    s.send(b'\x18\x06', 1.2); s.send(b'main\r', 1.5)  # open src/main.rs
    step(s, 'b2 startup (main.rs open)')
    step(s, 'b2 M-f (forward word)', b'\x1bf')
    step(s, 'b2 M-b (backward word)', b'\x1bb')
    step(s, 'b2 M-f x2 then M-d (kill word)', b'\x1bf\x1bf\x1bd')
    step(s, 'b2 C-k (kill line)', b'\x0b')
    step(s, 'b2 C-/ (undo?)', b'\x1f')
    step(s, 'b2 C-u 3 C-n (numeric prefix)', b'\x15\x33\x0e')
    step(s, 'b2 M-5 C-n', b'\x1b5\x0e')
    step(s, 'b2 C-x r s a (register save?)', b'\x18rs', 0.6)
    step(s, 'b2 M-% (query-replace?)', b'\x1b%')
    step(s, 'b2 C-x d (dired?)', b'\x18d', 1.0)
    step(s, 'b2 C-x k (kill buffer?)', b'\x18k', 1.0)
    step(s, 'b2 C-h k (describe key?)', b'\x08k', 0.8)
    step(s, 'b2 C-x C-f TAB TAB (completions?)', b'\x18\x06\t\t', 1.2)
    step(s, 'b2 picker C-g', b'\x07', 0.6)
    step(s, 'b2 C-x g (magit vs emacs vc)', b'\x18g', 1.2)
    step(s, 'b2 s (stage, vs C-x v v)', b's', 1.0)
    step(s, 'b2 g (refresh)', b'g', 1.0)
    s.send(b'\x18\x18', 1.0)
    print('=== battery2 done ===')

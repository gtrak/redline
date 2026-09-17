#!/usr/bin/env python3
"""Battery 2 (emacs reference): word motion, numeric prefix, registers,
query-replace, minibuffer completion/history, dired, VC git, kill-line,
help, buffer kill. Same pty+pyte method; vanilla emacs -Q -nw."""
import os, sys
sys.path.insert(0, os.path.dirname(__file__))
from drive_emacs import EmacsSession, fixture, step, KEYS

if __name__ == '__main__':
    root = '/tmp/emacs_parity2_repo'
    fixture(root)
    s = EmacsSession(root, ['src/main.rs'])
    step(s, 'b2 startup', b'', 1.0)
    step(s, 'b2 M-f (forward word)', b'\x1bf')
    step(s, 'b2 M-b (backward word)', b'\x1bb')
    step(s, 'b2 M-f x2 then M-d (kill word)', b'\x1bf\x1bf\x1bd')
    step(s, 'b2 C-k (kill line)', b'\x0b')
    step(s, 'b2 C-/ (undo the kills)', b'\x1f')
    step(s, 'b2 C-u 3 C-n (numeric prefix)', b'\x15\x33\x0e')
    step(s, 'b2 M-5 C-n (meta digit prefix)', b'\x1b5\x0e')
    step(s, 'b2 C-x r s a (register save)', b'\x18rs', 0.5)
    step(s, 'b2 register name a', b'a')
    step(s, 'b2 C-x r i a (register insert)', b'\x18ri', 0.5)
    step(s, 'b2 register name a', b'a')
    step(s, 'b2 M-% (query-replace)', b'\x1b%')
    step(s, 'b2 qr: type line + RET', b'line\r')
    step(s, 'b2 qr: type repl + RET', b'rope\r')
    step(s, 'b2 qr: ! (replace all)', b'!')
    step(s, 'b2 M-s w (word search)', b'\x1bsw', 0.5)
    step(s, 'b2 ws: type rope + RET', b'rope\r')
    step(s, 'b2 ws C-g', b'\x07')
    step(s, 'b2 C-x d (dired)', b'\x18d', 1.0)
    step(s, 'b2 dired: type src/ RET', b'src/\r', 1.0)
    step(s, 'b2 dired nav C-n + RET open', b'\x0e\r', 1.0)
    step(s, 'b2 C-x k (kill buffer)', b'\x18k', 0.7)
    step(s, 'b2 C-x k RET (default yes)', b'\r', 0.8)
    step(s, 'b2 C-h k C-n (describe key)', b'\x08\x0e'.replace(b'\x08', b'\x08'), 0.7)
    step(s, 'b2 C-x C-f with TAB twice (completions)', b'\x18\x06\t\t', 1.0)
    step(s, 'b2 minibuffer M-p (history)', b'\x1bp', 0.6)
    step(s, 'b2 minibuffer C-g', b'\x07')
    step(s, 'b2 C-x v v (vc next action)', b'\x18vv', 1.0)
    step(s, 'b2 C-x v d (vc dir)', b'\x18vd', 1.0)
    step(s, 'b2 vc C-g + q', b'\x07q', 0.8)
    s.quit()
    print('=== battery2 done ===')

#!/usr/bin/env python3
"""Emacs reference driver: same pty+pyte method as the redline drives.

Runs vanilla emacs (-Q -nw, no init) in a fixture repo and records what
each battery step renders, so redline behavior can be compared
attribute-level. Requires emacs on PATH. Output: human-readable log lines
on stdout (assertions are made by the caller/orchestrator, since this is a
reference capture, not a pass/fail gate).
"""
import os, pty, time, fcntl, termios, select, sys, shutil, struct, subprocess
import pyte

W, H = 100, 30

class EmacsSession:
    def __init__(self, cwd, args=None):
        self.screen = pyte.Screen(W, H)
        self.stream = pyte.ByteStream(self.screen)
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(cwd)
            os.environ['TERM'] = 'xterm-256color'
            argv = ['emacs', '-Q', '-nw'] + (args or [])
            os.execvp(argv[0], argv)
        self.pid, self.fd = pid, fd
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', H, W, 0, 0))
        self.pump(2.0)

    def pump(self, seconds):
        end = time.time() + seconds
        while time.time() < end:
            r, _, _ = select.select([self.fd], [], [], 0.1)
            if r:
                try:
                    self.stream.feed(os.read(self.fd, 65536))
                except OSError:
                    return

    def send(self, keys, wait=0.6):
        if isinstance(keys, str):
            keys = keys.encode()
        os.write(self.fd, keys)
        self.pump(wait)

    def lines(self):
        return [''.join(self.screen.buffer[y][x].data for x in range(W)).rstrip()
                for y in range(H)]

    def show(self):
        return '\n'.join(l for l in self.lines())

    def cursor(self):
        return (self.screen.cursor.x, self.screen.cursor.y)

    def quit(self):
        try:
            self.send(b'\x18\x18', 0.5)      # C-x C-c
            # if a save prompt appears, answer no
            self.send(b'n', 0.4)
            self.pump(0.5)
        except OSError:
            pass


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


KEYS = {
    'C-n': b'\x0e', 'C-p': b'\x10', 'C-f': b'\x06', 'C-b': b'\x02',
    'C-a': b'\x01', 'C-e': b'\x05', 'C-v': b'\x16', 'M-v': b'\x1bv',
    'M-<': b'\x1b<', 'M->': b'\x1b>', 'C-s': b'\x13', 'RET': b'\r',
    'C-g': b'\x07', 'C-xC-f': b'\x18\x06', 'C-xb': b'\x18b',
    'M-x': b'\x1bx', 'C-l': b'\x0c', 'C-x2': b'\x18\x12',
    'C-xo': b'\x18\x0f', 'C-x1': b'\x181', 'M-gg': b'\x1bg\x01b',
    'C-xC-c': b'\x18\x18', 'C-/': b'\x1f', 'C-aw': b'\x01\x17',
}

def step(s, name, keys, wait=0.8):
    s.send(KEYS[keys] if keys in KEYS else keys, wait)
    print(f'=== {name} ===')
    for y, line in enumerate(s.lines()):
        if line.strip():
            print(f'{y:2}| {line}')
    print(f'cursor: {s.cursor()}')
    print()

if __name__ == '__main__':
    root = '/tmp/emacs_parity_repo'
    fixture(root)
    s = EmacsSession(root, ['src/main.rs'])
    step(s, 'startup (file given)', b'', 1.0)
    step(s, 'C-n x5', b'\x0e' * 5)
    step(s, 'C-e (end of line)', b'\x05')
    step(s, 'C-v (scroll down)', b'\x16')
    step(s, 'M-< (top)', b'\x1b<')
    step(s, 'C-s isearch "line_5"', b'\x13line_5')
    step(s, 'C-s again (next match)', b'\x13')
    step(s, 'isearch C-g (cancel)', b'\x07')
    step(s, 'C-x C-f (find file)', b'\x18\x06')
    step(s, 'find-file type READ + tab', b'READ\t')
    step(s, 'find-file RET', b'\r', 1.0)
    step(s, 'C-x b (switch buffer)', b'\x18b')
    step(s, 'C-x b RET (back to main.rs)', b'\r', 0.8)
    step(s, 'M-g g 30 RET (goto line)', b'\x1bgg30\r')
    step(s, 'C-l (recenter)', b'\x0c')
    step(s, 'M-x (palette)', b'\x1bx')
    step(s, 'M-x type "goto"', b'\x1bgoto')
    step(s, 'M-x C-g', b'\x07')
    step(s, 'C-x 2 (split window)', b'\x18\x12')
    step(s, 'C-x o (other window)', b'\x18\x0f')
    step(s, 'C-x 1 (one window)', b'\x181')
    step(s, 'M-< then M-> (bottom)', b'\x1b<')
    step(s, 'M-> ', b'\x1b>')
    s.quit()
    print('=== done ===')

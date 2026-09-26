#!/usr/bin/env python3
"""probe_cursor_interleave — one-shot raw-stream monitor: does the app's
DETACHED cursor write (plan 013: `sleep(12ms)` + tokio::spawn MoveTo,
src/ui/root/hooks.rs install_cursor_effect) land INSIDE a synchronized
frame region (`?2026h` … `?2026l`)?

A correct stream has ZERO cursor bytes (`ESC[?25h/l`, `ESC[r;cH/f`)
between a `?2026h` open and its matching `?2026l`: the cursor write is
meant to land AFTER the close (measured quiet: every post-close tail is
empty or `?25h`+CUP only). An interleave — the cursor task firing while
the frame's content bytes are still in flight (frame flush is ~5 ms at
idle but exceeds the 12 ms deferral under load) — corrupts the on-wire
stream: pyte then renders CUP params as text ('2;1H…README.md (2)'),
truncates rows (r9 = 'let target_thre' in the 3-parallel probe run), or
crashes on the mangled CSI (pyte `cursor_down() takes 1 to 2 but 3 were
given`, same run).

Bounded by construction: MON_SECS seconds (default 90, cap 300), one App,
one summary, then exit. It keeps a steady key stream going so cursor-
bearing frames keep painting; the caller supplies the load (2-3 parallel
probe_file_search instances in other shells), so run it as:
  (python3 tools/probe_cursor_interleave.py > /tmp/interleave.log 2>&1) &
  ... two probe_file_search instances ...
and read the verdict AFTER the load finishes.
"""
import os
import re
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo, reset as _reset_fixture

SECS = min(int(os.environ.get("MON_SECS", "90")), 300)
OPEN = re.compile(rb"\x1b\[\?2026h")
CLOSE = re.compile(rb"\x1b\[\?2026l")
# The discriminating signature: `ESC[?25h` (cursor Show). The detached
# cursor task writes Show + MoveTo; the ONLY legal ?25h is the one AFTER
# a frame's ?2026l close. A ?25h INSIDE an open ?2026h…?2026l region is
# the interleave. (Plain CUPs `ESC[r;cH` are normal intra-frame row
# positioning — they are not evidence.)
CURSOR = re.compile(rb"\x1b\[\?25[hl]")


def scan(raw):
    """One left-to-right pass: every cursor byte while inside an open
    `?2026h` region is an interleave. Returns (count, spans)."""
    count = 0
    spans = []
    in_sync = False
    open_at = 0
    events = []
    for m in OPEN.finditer(raw):
        events.append((m.start(), "o"))
    for m in CLOSE.finditer(raw):
        events.append((m.start(), "c"))
    for m in CURSOR.finditer(raw):
        events.append((m.start(), "u"))
    events.sort(key=lambda e: e[0])
    for pos, kind in events:
        if kind == "o":
            in_sync = True
            open_at = pos
        elif kind == "c":
            in_sync = False
        else:
            if in_sync:
                count += 1
                if len(spans) < 5:
                    spans.append(raw[pos - 40:pos + 10])
    return count, spans


def main():
    _reset_fixture()
    app = App(repo("redline_pyte_repo"), rows=24, cols=80)
    # Cycle cursor-bearing views (search/magit/buffer) so the cursor
    # effect keeps firing on every frame.
    keys = ["C-c p s s", "target", "RET", "q", "C-x g", "C-c p t", "l", "q"]
    ki = 0
    t0 = time.time()
    while time.time() - t0 < SECS:
        app.key(keys[ki % len(keys)], settle=1.2)
        ki += 1
    n, spans = scan(app.raw_tail)
    frames = len(OPEN.findall(app.raw_tail))
    print("frames observed: %d; interleaved cursor bytes inside an open "
          "sync region: %d" % (frames, n))
    for s in spans:
        print("span:", s)
    app.kill()
    sys.exit(0 if n == 0 else 1)


if __name__ == "__main__":
    main()

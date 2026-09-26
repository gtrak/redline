#!/usr/bin/env python3
"""probe_cursor_interleave — one-shot raw-stream monitor: does a cursor
write land MID-FRAME (plan 013: before 013-02 the app's DETACHED cursor
write — `sleep(12ms)` + tokio::spawn MoveTo, src/ui/root/hooks.rs
install_cursor_effect — raced the frame flush; after 013-02 iocraft
itself emits the CUP, program-ordered)?

The legal shapes (013-02 onward):
  * every `?25h` (cursor Show) inside a synchronized frame region
    (`?2026h` … `?2026l`) is the VENDORED iocraft's program-order CUP: it
    sits after the canvas park and is the frame's final cursor movement —
    the bytes between it and the matching `?2026l` are EXACTLY one CUP
    (`ESC[r;cH`) and nothing else;
  * a `?25h` after a region's `?2026l` (a legacy/tail write) is also fine
    — it is outside any region and never counted;
  * a `?25l` inside a region is NEVER legal (upstream hides once at
    startup, outside frames);
  * plain CUPs `ESC[r;cH` are normal intra-frame row positioning.

An INTERLEAVE — the bug — is a cursor write whose tail does not end the
frame: a `?25h` with content (or the park, or another `?2026h`) still in
flight before its `?2026l`, or any `?25l` inside a region. Pre-013-02 the
probe counted every cursor byte inside a region (the old code never legally
emitted in-region, only by racing); post-013-02 the count must be ZERO
while the legal program-order CUPs keep flowing.

Bounded by construction: MON_SECS seconds (default 90, cap 300), one App,
one summary, then exit. It keeps a steady key stream going so cursor-
bearing frames keep painting; the caller supplies the load (2-3 parallel
probe_file_search instances in other shells, or `cargo test --workspace`),
so run it as:
  (python3 tools/probe_cursor_interleave.py > /tmp/interleave.log 2>&1) &
  ... load ... and read the verdict AFTER the load finishes.
"""
import os
import re
import sys
import time
import bisect

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo, reset as _reset_fixture

SECS = min(int(os.environ.get("MON_SECS", "90")), 300)
OPEN = re.compile(rb"\x1b\[\?2026h")
CLOSE = re.compile(rb"\x1b\[\?2026l")
# The discriminating signatures: `ESC[?25h` (cursor Show) and `ESC[?25l`
# (cursor Hide). Post-013-02 a ?25h INSIDE a region is legal ONLY as the
# program-order CUP (Show + exactly one CUP, then the region's ?2026l);
# anything else in-region — a Show with content still in flight, or any
# Hide — is the interleave. (Plain CUPs `ESC[r;cH` are normal intra-frame
# row positioning — they are not evidence.)
CURSOR = re.compile(rb"\x1b\[\?25[hl]")
CUP = re.compile(rb"\x1b\[\d+;\d+H")


def scan(raw):
    """One left-to-right pass over every cursor write. A `?25h`/`?25l`
    inside an open `?2026h` region is a mid-frame interleave unless it is
    the program-order CUP: exactly one CUP between the Show and the
    region's `?2026l`. Returns (count, spans)."""
    count = 0
    spans = []
    opens = [m.start() for m in OPEN.finditer(raw)]
    closes = [m.start() for m in CLOSE.finditer(raw)]
    for m in CURSOR.finditer(raw):
        p = m.start()
        # Inside a region iff the last OPEN before p postdates the last
        # CLOSE before p.
        i = bisect.bisect_left(opens, p)
        j = bisect.bisect_left(closes, p)
        last_open = opens[i - 1] if i > 0 else -1
        last_close = closes[j - 1] if j > 0 else -1
        if last_open <= last_close:
            continue  # outside any synchronized region: never an interleave
        if m.group(0) == b"\x1b[?25l":
            # A Hide inside a region is never legal (upstream hides once
            # at startup, outside frames).
            count += 1
            if len(spans) < 5:
                spans.append(raw[p - 40:p + 10])
            continue
        # ?25h: legal only when the bytes between it and the matching
        # ?2026l are exactly one CUP (the vendored iocraft emit: Show +
        # MoveTo, after the park, the frame's last bytes).
        k = bisect.bisect_right(closes, p)  # first close strictly after p
        if k >= len(closes):
            continue  # unterminated region; nothing after
        tail = raw[p + len(m.group(0)):closes[k]]
        if CUP.fullmatch(tail):
            continue  # the program-order CUP (013-02): not an interleave
        count += 1
        if len(spans) < 5:
            spans.append(raw[p - 40:p + 10])
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
    print("frames observed: %d; mid-frame cursor interleaves (cursor write "
          "whose tail is not exactly CUP + ?2026l): %d" % (frames, n))
    for s in spans:
        print("span:", s)
    app.kill()
    sys.exit(0 if n == 0 else 1)


if __name__ == "__main__":
    main()

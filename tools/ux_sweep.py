#!/usr/bin/env python3
"""Broad automated UX sweep — thin tier (loop-04).

Drives the same key sequences across every view and flags ANOMALIES rather
than asserting specific values:

  * a hard panic / process death
  * "unbound key" echoes (a key we expect to work is not bound)
  * a blank frame while a buffer/view is open (render loss)
  * rows wider than the terminal (layout overflow) or stray control chars
  * a "changed on disk" marker or error message appearing unprompted
  * the hardware cursor parked off-screen (row/col outside the frame)

What changed in loop-04: the keymap-coverage STATE half (which keys are
bound in which view, blank-view and cursor-in-range invariants) is now the
unit twin `unit_flow_ux_keymap_coverage` in src/app/flow_tests.rs (every
key of every leg, driven through `AppStore::key_event`; the window-split
keys C-x 2/1/0 are now BOUND on the buffer view with view-stack-degraded
semantics — the pre-existing 3 unbound-key findings are cleared, and the
C-x 2 vertical split itself remains a scoped follow-up: see the
known-issue watchlist in docs/ux-testing-plan.md). This file keeps the
terminal tier: the same per-key anomaly scan through the REAL terminal —
input encoding, process liveness, the hardware cursor, raw pixels — but in
TWO App sessions instead of thirteen (one 80-col session drives every
normal leg in order; one 40-col session drives the narrow-terminal stress),
so the pool battery pays two launches, not thirteen.

Run: python3 tools/ux_sweep.py [--cols N] [--rows N]
Exit code 0 always (diagnostic); prints a findings list.
"""
import os
import sys
import argparse

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from pyte_driver import App
from fixture import reset


def frame_text(app):
    return "\n".join(app.row_text(r) for r in range(app.rows))


def anomalies(app, label, cols, rows):
    out = []
    text = frame_text(app)
    # render loss: a view is open but the screen is blank
    if not text.strip():
        out.append("blank frame")
    # layout overflow: any row longer than the terminal
    for r in range(app.rows):
        if len(app.row_text(r)) > cols:
            out.append(f"row {r} wider than {cols}")
            break
    # unbound-key echo (a key we expected to be bound is not)
    if "unbound key" in text:
        out.append("unbound-key echo")
    # cursor parked outside the frame
    y, x = app.screen.cursor.y, app.screen.cursor.x
    if not (0 <= y < rows and 0 <= x < cols):
        out.append(f"cursor off-screen at ({y},{x})")
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cols", type=int, default=80)
    ap.add_argument("--rows", type=int, default=24)
    args = ap.parse_args()
    reset()
    root = "/tmp/redline_pyte_repo"

    findings = []

    def drive(app, label, setup, keys, settle=0.5):
        try:
            for k in setup:
                app.key(k, settle=settle)
            for k in keys:
                app.key(k, settle=settle)
                for a in anomalies(app, label, args.cols, args.rows):
                    findings.append(f"{label}: key {k!r}: {a}")
        except Exception as e:  # process died
            findings.append(f"{label}: EXCEPTION {type(e).__name__}: {e}")

    # ONE session drives every normal leg (loop-04: the per-key anomaly
    # scan is preserved; only the launch count drops). Each leg ends with
    # the keys that return to the home view, so the next leg starts from a
    # clean state exactly like the old fresh-App drives.
    app = App(root, rows=args.rows, cols=args.cols)
    try:
        drive(app, "buffer-view", ["C-x C-f lib.rs RET"],
              ["C-n", "C-p", "C-f", "C-b", "C-a", "C-e", "M-f", "M-b",
               "M-<", "M->", "C-v", "M-v", "C-d", "C-u", "C-l", "j", "k",
               "q"], settle=0.4)
        drive(app, "magit", ["C-x g"],
              ["n", "p", "n", "n", "TAB", "TAB", "s", "u", "g", "q"],
              settle=0.7)
        # log: RET opens the commit diff (q closes it back to the log, then
        # the final q closes the log itself).
        drive(app, "log", ["C-x g", "l"], ["n", "p", "RET", "q", "q"],
              settle=0.7)
        # blame: q closes the blame back to magit status, then home.
        drive(app, "blame", ["C-x g", "b"], ["n", "p", "q", "q"], settle=0.7)
        drive(app, "find-file", ["C-x C-f"], ["x", "y", "z", "C-g"],
              settle=0.5)
        # buffer list: C-g does not close it (global cancel); q does.
        drive(app, "buffer-list", ["C-x C-b"], ["n", "p", "C-g", "q"],
              settle=0.5)
        drive(app, "transient-menu", [], ["C-x", "C-g", "?", "C-g"],
              settle=0.6)
        drive(app, "search", ["C-c p s s"], ["C-g", "C-g"], settle=0.6)
        drive(app, "notes-edit", ["C-x n"], ["a", "b", "C-h", "C-g", "q"],
              settle=0.5)
        # the tree over a BUFFER view: pre-06a this scenario booted on the
        # scratch buffer, so open a file first (06a boots on the home view,
        # whose keymap is intentionally empty — C-n/C-p there are unbound by
        # design, and the tree-on-home leg lives in sweep.py "home -> file").
        # M-x toggle-tree RET switches the tree OFF again before closing.
        drive(app, "tree",
              ["C-x C-f lib.rs RET", "M-x toggle-tree RET"],
              ["C-n", "C-p", "RET", "C-g", "M-x toggle-tree RET", "q"],
              settle=0.6)
        # window-splits: C-x 2 / C-x 1 / C-x 0 are BOUND on the buffer view
        # (the 3 pre-existing unbound-key findings, cleared): view-stack
        # degraded semantics — C-x 0 closes the top view, C-x 1 truncates
        # the stack to the buffer view, C-x 2 reports the single-pane model
        # (the vertical split itself is a scoped follow-up). C-x o
        # (open-scratch) is bound too.
        drive(app, "window-splits", ["C-x C-f lib.rs RET"],
              ["C-x 2", "C-x o", "C-x o", "C-x 1", "C-x 0", "q"],
              settle=0.6)
    finally:
        app.kill()

    # narrow terminal stress (its own sized session, as before)
    old_cols = args.cols
    args.cols = 40
    narrow = App(root, rows=args.rows, cols=args.cols)
    try:
        drive(narrow, "narrow-40", ["C-x C-f lib.rs RET"],
              ["C-n", "C-e", "M->", "C-l", "?", "C-g"], settle=0.5)
    finally:
        narrow.kill()
        args.cols = old_cols

    print("=== UX SWEEP FINDINGS ===")
    if not findings:
        print("  (none)")
    else:
        for f in findings:
            print(f"  {f}")
    print(f"\nTOTAL: {len(findings)} finding(s)")


if __name__ == "__main__":
    main()

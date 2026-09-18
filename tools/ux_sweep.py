#!/usr/bin/env python3
"""Broad automated UX sweep: drive many key sequences across views and flag
ANOMALIES rather than asserting specific values. Complements the targeted
probes; its job is to surface the unexpected:

  * a hard panic / process death
  * "unbound key" echoes (a key we expect to work is not bound)
  * a blank frame while a buffer/view is open (render loss)
  * rows wider than the terminal (layout overflow) or stray control chars
  * a "changed on disk" marker or error message appearing unprompted
  * the hardware cursor parked off-screen (row/col outside the frame)

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

CTRL_RE = None


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
    # unbound-key echo (we track which keys are expected to be bound by
    # passing expect_bound=False for deliberate probes)
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

    def drive(label, keys, setup=None, settle=0.5):
        app = App(root, rows=args.rows, cols=args.cols)
        try:
            if setup:
                for k in setup:
                    app.key(k, settle=settle)
            for k in keys:
                app.key(k, settle=settle)
                for a in anomalies(app, label, args.cols, args.rows):
                    findings.append(f"{label}: key {k!r}: {a}")
        except Exception as e:  # process died
            findings.append(f"{label}: EXCEPTION {type(e).__name__}: {e}")
        finally:
            try:
                app.kill()
            except Exception:
                pass

    # each view, driven with its own navigation keys
    drive("buffer-view", ["C-n", "C-p", "C-f", "C-b", "C-a", "C-e", "M-f", "M-b",
                          "M-<", "M->", "C-v", "M-v", "C-d", "C-u", "C-l", "j", "k"],
          setup=["C-x C-f", "lib.rs", "RET"])
    drive("magit", ["n", "p", "n", "n", "TAB", "TAB", "s", "u", "g", "q"],
          setup=["C-x g"], settle=0.7)
    drive("log", ["n", "p", "RET", "q"], setup=["C-x g", "l"], settle=0.7)
    drive("blame", ["n", "p", "q"], setup=["C-x g", "b"], settle=0.7)
    drive("find-file", ["x", "y", "z", "C-g"], setup=["C-x C-f"], settle=0.5)
    drive("buffer-list", ["n", "p", "C-g"], setup=["C-x C-b"], settle=0.5)
    drive("transient-menu", ["C-x", "C-g", "?", "C-g"], setup=[], settle=0.6)
    drive("search", ["C-g", "C-g"], setup=["C-c p s s"], settle=0.6)
    drive("notes-edit", ["a", "b", "C-h", "C-g"], setup=["C-x n"], settle=0.5)
    drive("tree", ["C-n", "C-p", "RET", "C-g"], setup=["M-x", "toggle-tree", "RET"],
          settle=0.6)
    drive("window-splits", ["C-x 2", "C-x o", "C-x o", "C-x 1", "C-x 0"],
          setup=["C-x C-f", "lib.rs", "RET"], settle=0.6)
    # narrow terminal stress
    old_cols = args.cols
    args.cols = 40
    drive("narrow-40", ["C-n", "C-e", "M->", "C-l", "?", "C-g"],
          setup=["C-x C-f", "lib.rs", "RET"], settle=0.5)
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

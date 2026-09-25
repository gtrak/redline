#!/usr/bin/env python3
"""issue-current-line-highlight (follow-up) — PTY probe: the 16-colour
fallback path. When COLORTERM is NOT truecolor/24bit, the tint falls back
to the 16-colour palette's DarkGrey (SGR 48;5;8 -> pyte `7f7f7f`).

Legs (one App, the shared fixture under the shared PTY flock):
  1. the point's row carries the DarkGrey fallback background on EVERY cell
     (pyte `7f7f7f`), and the rows immediately above and below carry the
     view's normal background (Black -> pyte `000000`).

Exit 0 = all assertions pass; 1 = any failed.
"""
import os, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pyte_driver
from fixture import repo, reset

REPO = os.environ.get("REDLINE_REPO") or repo("redline_pyte_repo")
COLS, ROWS = 80, 24

LEG_RS = "src/clhprobe16.rs"
LEG_LINES = [
    "alpha row one",
    "bravo row two",
    "charlie row three",
    "delta row four",
    "echo row five",
    "foxtrot row six",
]
TINT_16 = "7f7f7f"  # DarkGrey (ANSI 8) = SGR 48;5;8, the 16-colour fallback
VIEW_BG = "000000"  # the dark theme's view background (Black)

CHECKS = []


def rec(name, ok, detail=""):
    CHECKS.append((name, ok, detail))
    print(f"  {'PASS' if ok else 'FAIL'}  {name:60s} {detail}")


def row_bg_set(app, row):
    return {str(app.screen.buffer[row][i].bg).lower() for i in range(COLS)}


def row_all_bg(app, row, bg):
    return all(str(app.screen.buffer[row][i].bg).lower() == bg for i in range(COLS))


def find_row(app, needle):
    for r in range(ROWS):
        text = "".join(app.screen.buffer[r][i].data for i in range(COLS))
        if needle in text:
            return r
    return None


def dump_frame(app, label):
    print(f"\n── literal frame: {label} ──")
    for r in range(ROWS):
        row = app.screen.buffer[r]
        text = "".join(row[i].data for i in range(COLS))
        bgs = {str(row[i].bg).lower() for i in range(COLS)}
        sig = " ".join(sorted(b for b in bgs if b != "none"))
        print(f"  [{r:2d}] {text}")
        print(f"       bg: {sig}")
    print("  (bg legend: 000000 = view background, 7f7f7f = current-line "
          "tint 16-colour fallback (48;5;8), 5c5cff/0000ff = status line)")


def main():
    print(f"REPO={REPO}")
    # colorterm=None (default) pops COLORTERM from the spawned process,
    # forcing the 16-colour fallback path.
    reset()
    with open(os.path.join(REPO, LEG_RS), "w", encoding="utf-8") as f:
        f.write("\n".join(LEG_LINES) + "\n")

    app = pyte_driver.App(REPO, rows=ROWS, cols=COLS)
    try:
        app.key("C-x C-f")
        app.wait(0.8)
        app.type_text("clhprobe16", settle=0.6)
        app.key("RET")
        app.wait(0.8)

        # The point starts at line 0; land it on line 2 (goto-line, 1-based).
        app.key("M-g g", settle=0.4)
        app.wait(0.4)
        app.type_text("3", settle=0.3)
        app.key("RET")
        app.wait(0.8)

        # ── Leg 1: the 16-colour fallback tint on the point's row ───
        pt = find_row(app, LEG_LINES[2])
        rec("the point's row renders", pt is not None, f"row={pt}")
        if pt is not None:
            above = pt - 1
            below = pt + 1
            rec("point row: every cell carries the 16-colour fallback",
                row_all_bg(app, pt, TINT_16),
                f"bgs={sorted(row_bg_set(app, pt))}")
            rec("row above: every cell carries the view background",
                row_all_bg(app, above, VIEW_BG),
                f"bgs={sorted(row_bg_set(app, above))}")
            rec("row below: every cell carries the view background",
                row_all_bg(app, below, VIEW_BG),
                f"bgs={sorted(row_bg_set(app, below))}")
        dump_frame(app, "point on line 2 (16-colour fallback: DarkGrey 48;5;8)")

    finally:
        app.key("C-x C-c")
        app.wait(0.5)
        app.kill()
    try:
        os.remove(os.path.join(REPO, LEG_RS))
    except FileNotFoundError:
        pass
    reset()

    failed = [c for c in CHECKS if not c[1]]
    print(f"\n{len(CHECKS) - len(failed)}/{len(CHECKS)} checks passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

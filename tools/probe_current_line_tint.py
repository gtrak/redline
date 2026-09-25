#!/usr/bin/env python3
"""issue-current-line-highlight — PTY probe: the literal rendered frame the
user judges the subtlety on, plus the per-cell background assertions the
unit tests pin at the canvas level.

Legs (one App, the shared fixture under the shared PTY flock):
  1. the point's row carries the tint background on EVERY cell (the dark
     theme's truecolor shade 48;2;35;35;35 -> pyte `232323`), and
     the rows immediately above and below carry the view's normal
     background (Black -> pyte `000000`).
  2. moving the point down one line (C-n) moves the tint with it: the new
     row is the tint, the old row reverts to the view background.
  3. a region (C-@ mark at the start of line 3, point back on line 2) — the
     region face WINS over the tint on the region's rows (region bg
     `5f5f5f` in the 16-color path), asserted per cell.

The probe sets COLORTERM=truecolor to exercise the truecolor path (the
SGR 48;2;35;35;35 escape). A separate probe (probe_current_line_tint_16.py)
exercises the 16-colour fallback (SGR 48;5;8 -> pyte `7f7f7f`).

The probe prints the literal frame (text rows + a per-row background
signature) at the end of leg 1 — the artifact the report embeds.

Exit 0 = all assertions pass; 1 = any failed.
"""
import os, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pyte_driver
from fixture import repo, reset

REPO = os.environ.get("REDLINE_REPO") or repo("redline_pyte_repo")
COLS, ROWS = 80, 24

LEG_RS = "src/clhprobe.rs"
LEG_LINES = [
    "alpha row one",
    "bravo row two",
    "charlie row three",
    "delta row four",
    "echo row five",
    "foxtrot row six",
]
TINT = "232323"    # the dark theme's current_line face, 48;2;35;35;35
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
    sigs = {}
    for r in range(ROWS):
        row = app.screen.buffer[r]
        text = "".join(row[i].data for i in range(COLS))
        bgs = {str(row[i].bg).lower() for i in range(COLS)}
        sig = " ".join(sorted(b for b in bgs if b != "none"))
        print(f"  [{r:2d}] {text}")
        print(f"       bg: {sig}")
        sigs[r] = sig
    print("  (bg legend: 000000 = view background, 232323 = current-line "
          "tint (48;2;35;35;35), 5c5cff/0000ff = status line, 7f7f7f = region face)")


def main():
    print(f"REPO={REPO}")
    # Force truecolor via the App's colorterm parameter (the pyte_driver
    # explicitly pops COLORTERM unless colorterm="truecolor" is passed).
    reset()
    with open(os.path.join(REPO, LEG_RS), "w", encoding="utf-8") as f:
        f.write("\n".join(LEG_LINES) + "\n")

    app = pyte_driver.App(REPO, rows=ROWS, cols=COLS, colorterm="truecolor")
    try:
        app.key("C-x C-f")
        app.wait(0.8)
        app.type_text("clhprobe", settle=0.6)
        app.key("RET")
        app.wait(0.8)

        # The point starts at line 0; land it on line 2 (goto-line, 1-based).
        app.key("M-g g", settle=0.4)
        app.wait(0.4)
        app.type_text("3", settle=0.3)
        app.key("RET")
        app.wait(0.8)

        # ── Leg 1: the tint on the point's row, neighbours untouched ───
        pt = find_row(app, LEG_LINES[2])
        rec("the point's row renders", pt is not None, f"row={pt}")
        if pt is not None:
            above = pt - 1
            below = pt + 1
            # The title/indicator rows are NOT in the file-view content:
            # the rows immediately above/below the point's row are content
            # rows (lines 1 and 3).
            rec("row above is a content row",
                LEG_LINES[1] in "".join(app.screen.buffer[above][i].data for i in range(COLS)),
                f"above row text")
            rec("row below is a content row",
                LEG_LINES[3] in "".join(app.screen.buffer[below][i].data for i in range(COLS)),
                f"below row text")
            rec("point row: every cell carries the tint",
                row_all_bg(app, pt, TINT),
                f"bgs={sorted(row_bg_set(app, pt))}")
            rec("row above: every cell carries the view background",
                row_all_bg(app, above, VIEW_BG),
                f"bgs={sorted(row_bg_set(app, above))}")
            rec("row below: every cell carries the view background",
                row_all_bg(app, below, VIEW_BG),
                f"bgs={sorted(row_bg_set(app, below))}")
        dump_frame(app, "point on line 2 (charlie row three)")

        # ── Leg 2: the tint follows the point ──────────────────────────
        app.key("C-n", settle=0.6)
        app.wait(0.8)
        pt2 = find_row(app, LEG_LINES[3])
        rec("after C-n: the new point's row carries the tint",
            pt2 is not None and row_all_bg(app, pt2, TINT),
            f"row={pt2} bgs={sorted(row_bg_set(app, pt2)) if pt2 is not None else '-'}")
        pt1 = find_row(app, LEG_LINES[2])
        rec("after C-n: the old point's row reverts to the view background",
            pt1 is not None and row_all_bg(app, pt1, VIEW_BG),
            f"row={pt1} bgs={sorted(row_bg_set(app, pt1)) if pt1 is not None else '-'}")

        # ── Leg 3: the region wins over the tint ───────────────────────
        # Mark one char into line 3 (the point is on line 3 now), then
        # move the point back to line 2 col 0: the region spans lines
        # 2..3 (the region's END is exclusive at the mark, so a mark at
        # col 0 would end the region at line 2's end), and BOTH rows are
        # in it — including the point's row. The region face must win per
        # cell (pyte reads the DarkGrey face as `7f7f7f`).
        app.key("C-f", settle=0.4)
        app.wait(0.4)
        app.key("C-@", settle=0.5)
        app.wait(0.5)
        app.key("C-p", settle=0.6)
        app.wait(0.8)
        for line_no, label in ((2, "line 2 (point's row)"), (3, "line 3 (mark's row)")):
            r = find_row(app, LEG_LINES[line_no])
            rec(f"region: {label} carries the region face (not the tint)",
                r is not None and row_all_bg(app, r, "7f7f7f"),
                f"row={r} bgs={sorted(row_bg_set(app, r)) if r is not None else '-'}")
        dump_frame(app, "region over lines 2-3, point on line 2")

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

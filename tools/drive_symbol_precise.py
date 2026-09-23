#!/usr/bin/env python3
"""issue-annotations-symbol-precise + issue-annotation-marker-cell — the
symbol-precise anchor frames (thin PTY tier). The ANCHOR rule is
unit-pinned in src/app/store/file_view.rs (tests) +
src/app/store/tests/file_view.rs; this drive captures the literal
per-cell frames the user judges:

  1. a MID-LINE symbol (`    let x = 1;`, record on `x` char 8): the
     indicator sits at display col 7 (the cell before the symbol), not
     the indent anchor 3; the code is cell-for-cell the source;
  2. a WIDE-CHAR-preceded symbol (`  中中 y = 2;`, record on `y` char 5):
     the anchor is display col 6 (a char-offset error would land at 4,
     inside the second CJK glyph) — the units trap, invisible on ASCII;
  3. a TAB-indented mid-line symbol (`\\tlet z = 3;`, record on `z`
     char 5): the anchor is display col 11, the code at the 8-column
     tab stop;
  4. the NO-WHITESPACE INSERT (issue-annotation-marker-cell, `    a+b`,
     record on `b` char 6): the record is tied to a symbol and the cell
     before it is not whitespace, so the line INSERTS one cell at the
     symbol's start — `a+b` renders `a+\\u25b4b` (\\u25b4 at 6, `b` at
     7), NOT the old fallback at the line's indent (col 4, a different
     symbol);
  5. TWO ANNOTATIONS on one line (`    a b`, records on `a` char 4 and
     `b` char 6): two indicators at two columns (3 and 5), two note
     rows, each \\u256d at its own anchor. The `A` key path dedupes per
     line, so leg 5 is a HAND-EDITED `.redline-notes.md` (pre-seeded
     before the app starts), not something the UI can produce;
  6. the NO-SYMBOL POINT (issue-annotation-marker-cell, `    // hi`,
     record on `h` char 7): a comment point captures nothing, so the
     record stays line-tied and the behavior is UNCHANGED — the
     marker overwrites the blank cell before the point (`//\\u25b4hi`),
     no insertion.

Legs 1-4 and 6 drive the real `A` key path (the A key event with the
point moved onto the symbol via `M-g g` goto-line + C-f steps). This
suite owns the shared fixture under the shared PTY flock (via
pyte_driver), resets the baseline at start and end, and removes its
own strays. Wrap the invocation in `timeout`.

Exit 0 = all assertions pass; 1 = any failed.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo, reset

REPO = os.environ.get("REDLINE_REPO") or repo("redline_pyte_repo")
ROWS, COLS = 24, 80

CHECKS = []


def rec(name, ok, detail=""):
    CHECKS.append((name, ok, detail))
    print(f"  {'PASS' if ok else 'FAIL'}  {name:56s} {detail}")


def text(app):
    return "\n".join(app.row_text(r) for r in range(app.rows))


def row_cells(app, row):
    """The row as a list of per-cell data strings (0-based cells)."""
    return [app.screen.buffer[row][i].data for i in range(app.cols)]


def find_row(app, needle):
    for r in range(app.rows):
        if needle in app.row_text(r):
            return r
    return None


def col_of(cells, ch):
    """Display column of `ch` in a cells list (each cell holds one
    terminal cell; wide chars occupy two cells, the second empty)."""
    for i, d in enumerate(cells):
        if d and d[0] == ch:
            return i
    return None


def dump(app, rows, label):
    """Literal per-cell frame: a ruler + each row's cells 0..max+1."""
    if not rows:
        return
    width = max(len(app.row_text(r).rstrip()) for r in rows) + 1
    print(f"  --- {label} (cells 0..{min(width, COLS)-1}) ---")
    ruler = "     " + "".join(str(i % 10) for i in range(min(width, COLS)))
    print("  " + ruler)
    for r in rows:
        line = app.row_text(r).rstrip()
        print(f"  L{r:02d} {line}")
    print()


def cleanup():
    for p in (os.path.join(REPO, "src", "symleg.rs"),
              os.path.join(REPO, "src", "symleg2.rs"),
              os.path.join(REPO, "src", "symleg3.rs"),
              os.path.join(REPO, ".redline-notes.md")):
        try:
            os.remove(p)
        except OSError:
            pass


LEG1 = "fn main() {\n    let x = 1;\n  \u4e2d\u4e2d y = 2;\n\tlet z = 3;\n}\n"

LEG3 = "fn main() {\n    a+b\n    // hi\n}\n"
NOTES2 = """<!-- redline-annotations:begin -->
[annotation]
path: src/symleg2.rs
line: 1
col: 4
anchor:     a b
note: note a
orphaned: false
[annotation]
path: src/symleg2.rs
line: 1
col: 6
anchor:     a b
note: note b
orphaned: false
<!-- redline-annotations:end -->
"""


def open_file(app, name):
    app.key("C-x C-f")
    app.wait(0.8)
    app.type_text(name, settle=0.6)
    app.key("RET")
    app.wait(0.8)


def annotate(app, line, char_col, note):
    """Move the point to (line, char_col) — `M-g g` goto-line lands at the
    line's START (col 0, 1-based prompt), then C-f steps to the symbol —
    and commit a note via the A key path (the record's `col` = the
    point's char offset — the anchor's input). Goto-line (not relative
    C-n steps) keeps each leg independent of where the previous note left
    the point.
    """
    app.key("M-g g", settle=0.4)
    app.wait(0.4)
    app.type_text(str(line + 1), settle=0.3)
    app.key("RET")
    app.wait(0.5)
    for _ in range(char_col):
        app.key("C-f", settle=0.2)
    app.key("A")
    app.wait(0.5)
    app.type_text(note, settle=0.5)
    app.key("RET")
    app.wait(0.8)


def main():
    print(f"REPO={REPO}\n")
    reset()
    cleanup()

    # ── Legs 1-4: the A-key path, one record per line ─────────────────
    with open(os.path.join(REPO, "src", "symleg.rs"), "w", encoding="utf-8") as f:
        f.write(LEG1)
    # symleg3.rs must exist before the app boots — the find-file
    # candidate index is built at startup.
    with open(os.path.join(REPO, "src", "symleg3.rs"), "w", encoding="utf-8") as f:
        f.write(LEG3)
    app = App(REPO, rows=ROWS, cols=COLS)
    try:
        rec("L0: the app reached ready", "ready" in text(app))
        open_file(app, "symleg.rs")
        rec("L1: symleg.rs is open", "fn main() {" in text(app),
            f"row0={app.row_text(0)!r}")

        # Line 1: `    let x = 1;` — record on `x` (char 8, mid-line).
        annotate(app, 1, 8, "note about x")
        # Line 2: `  中中 y = 2;` — record on `y` (char 5, CJK-preceded).
        annotate(app, 2, 5, "cjk note")
        # Line 3: `\tlet z = 3;` — record on `z` (char 5, tab-indented).
        annotate(app, 3, 5, "tabbed")
        saved = "note saved" in text(app)
        rec("L2: all three A-key notes committed", saved,
            f"minibuffer={app.row_text(app.rows - 2)!r}")
        app.wait(0.8)

        # ── Case 1: the mid-line symbol (line 1) ────────────────────────
        c1 = find_row(app, "note about x")
        c1d = find_row(app, "x = 1;")
        ok = c1 is not None and c1d is not None and c1 == c1d - 1
        rec("C1: note row directly above the code row", ok,
            f"note_row={c1} code_row={c1d}")
        if ok:
            code_cells = row_cells(app, c1d)
            arrow = col_of(code_cells, "\u25b4")
            rec("C1: \u25b4 at display col 7 (before the symbol, not the indent anchor 3)",
                arrow == 7, f"\u25b4 col={arrow}")
            rec("C1: `x` keeps its source column (8)",
                col_of(code_cells, "x") == 8,
                f"x col={col_of(code_cells, 'x')}")
            rec("C1: the note row's \u256d anchors at the same cell (7)",
                col_of(row_cells(app, c1), "\u256d") == 7,
                f"\u256d col={col_of(row_cells(app, c1), '\u256d')}")
            rec("C1: cell-for-cell code (`let` at 4, no shift)",
                col_of(code_cells, "l") == 4 and col_of(code_cells, "e") == 5
                and col_of(code_cells, "t") == 6,
                f"let cols={[col_of(code_cells, c) for c in 'let']}")
            dump(app, [c1, c1d], "case 1: mid-line symbol")

        # ── Case 2: the wide-char-preceded symbol (line 2) ─────────────
        c2n = find_row(app, "cjk note")
        c2 = find_row(app, "y = 2;")
        ok = c2n is not None and c2 is not None and c2n == c2 - 1
        rec("C2: note row directly above the code row", ok,
            f"note_row={c2n} code_row={c2}")
        if ok:
            code_cells = row_cells(app, c2)
            arrow = col_of(code_cells, "\u25b4")
            # 中 occupies cells 2-3 and 4-5; `y` is at display 7.
            rec("C2: \u25b4 at display col 6 (char-offset error would be 4)",
                arrow == 6, f"\u25b4 col={arrow}")
            rec("C2: `y` keeps its source column (7)",
                col_of(code_cells, "y") == 7,
                f"y col={col_of(code_cells, 'y')}")
            rec("C2: the note row's \u256d anchors at col 6",
                col_of(row_cells(app, c2n), "\u256d") == 6,
                f"\u256d col={col_of(row_cells(app, c2n), '\u256d')}")
            dump(app, [c2n, c2], "case 2: wide-char-preceded symbol")

        # ── Case 3: the tab-indented mid-line symbol (line 3) ──────────
        c3n = find_row(app, "tabbed")
        c3 = find_row(app, "z = 3;")
        ok = c3n is not None and c3 is not None and c3n == c3 - 1
        rec("C3: note row directly above the code row", ok,
            f"note_row={c3n} code_row={c3}")
        if ok:
            code_cells = row_cells(app, c3)
            arrow = col_of(code_cells, "\u25b4")
            rec("C3: \u25b4 at display col 11 (before `z` at 12; the tab stop is owned)",
                arrow == 11, f"\u25b4 col={arrow}")
            rec("C3: the code sits at the 8-column tab stop (`l` at 8)",
                col_of(code_cells, "l") == 8,
                f"l col={col_of(code_cells, 'l')}")
            rec("C3: the note row's \u256d anchors at col 11",
                col_of(row_cells(app, c3n), "\u256d") == 11,
                f"\u256d col={col_of(row_cells(app, c3n), '\u256d')}")
            dump(app, [c3n, c3], "case 3: tab-indented symbol")

        # ── Cases 4 + 6: the no-whitespace INSERT and the no-symbol
        # point (a clean file — the marker-cell rule's premise is the
        # record's syntax tie, which needs a clean parse) ───────────────
        open_file(app, "symleg3.rs")
        rec("L3: symleg3.rs is open", "a+b" in text(app),
            f"row0={app.row_text(0)!r}")

        # Line 1: `    a+b` — record on `b` (char 6, no whitespace
        # before; the capture ties the record to `b`).
        annotate(app, 1, 6, "no ws")
        # Line 2: `    // hi` — record on `h` (char 7; a comment point
        # captures nothing — the record stays line-tied, unchanged).
        annotate(app, 2, 7, "comment point")
        app.wait(0.8)

        # ── Case 4: the no-whitespace insert (line 1) ───────────────────
        c4n = find_row(app, "no ws")
        c4 = find_row(app, "a+\u25b4b")
        ok = c4n is not None and c4 is not None and c4n == c4 - 1
        rec("C4: note row directly above the code row", ok,
            f"note_row={c4n} code_row={c4}")
        if ok:
            code_cells = row_cells(app, c4)
            arrow = col_of(code_cells, "\u25b4")
            rec("C4: inserted \u25b4 at the symbol's own column (6), NOT the indent anchor (4)",
                arrow == 6, f"\u25b4 col={arrow}")
            rec("C4: `a` stays put (col 4), `b` shifted exactly one (col 7)",
                col_of(code_cells, "a") == 4
                and col_of(code_cells, "b") == 7,
                f"a col={col_of(code_cells, 'a')} b col={col_of(code_cells, 'b')}")
            rec("C4: the note row's \u256d anchors at col 6",
                col_of(row_cells(app, c4n), "\u256d") == 6,
                f"\u256d col={col_of(row_cells(app, c4n), '\u256d')}")
            dump(app, [c4n, c4], "case 4: no-whitespace insert")

        # ── Case 6: the no-symbol point is unchanged (line 2) ──────
        c6n = find_row(app, "comment point")
        c6 = find_row(app, "//\u25b4hi")
        ok = c6n is not None and c6 is not None and c6n == c6 - 1
        rec("C6: note row directly above the code row", ok,
            f"note_row={c6n} code_row={c6}")
        if ok:
            code_cells = row_cells(app, c6)
            arrow = col_of(code_cells, "\u25b4")
            rec("C6: \u25b4 overwrites the blank cell before the point (6) — no insertion",
                arrow == 6, f"\u25b4 col={arrow}")
            rec("C6: the code does not move (`h` keeps col 7)",
                col_of(code_cells, "h") == 7,
                f"h col={col_of(code_cells, 'h')}")
            rec("C6: the note row's \u256d anchors at col 6",
                col_of(row_cells(app, c6n), "\u256d") == 6,
                f"\u256d col={col_of(row_cells(app, c6n), '\u256d')}")
            dump(app, [c6n, c6], "case 6: no-symbol point unchanged")
    finally:
        app.kill()

    # ── Leg 5: two annotations on one line (hand-edited notes file) ────
    reset()
    cleanup()
    with open(os.path.join(REPO, "src", "symleg2.rs"), "w", encoding="utf-8") as f:
        f.write("fn main() {\n    a b\n}\n")
    with open(os.path.join(REPO, ".redline-notes.md"), "w", encoding="utf-8") as f:
        f.write(NOTES2)
    app = App(REPO, rows=ROWS, cols=COLS)
    try:
        rec("L5: the app reached ready (hand-seeded notes)", "ready" in text(app))
        open_file(app, "symleg2.rs")
        rec("L6: symleg2.rs is open", "fn main() {" in text(app))
        app.wait(0.8)

        na = find_row(app, "note a")
        nb = find_row(app, "note b")
        cb = find_row(app, "a")
        ok_code = cb is not None
        # The code row is `   \u25b4a\u25b4b` — find the row with two arrows.
        cb = None
        for r in range(app.rows):
            cells = row_cells(app, r)
            if sum(1 for d in cells if d and d[0] == "\u25b4") == 2 and "a" in app.row_text(r):
                cb = r
        ok = na is not None and nb is not None and cb is not None
        rec("C5: two note rows + the code row all present", ok,
            f"note_a={na} note_b={nb} code={cb}")
        if ok:
            rec("C5: both note rows directly above the code row (record order)",
                nb == cb - 1 and na == cb - 2,
                f"note_a={na} note_b={nb} code={cb}")
            code_cells = row_cells(app, cb)
            arrows = [i for i, d in enumerate(code_cells) if d and d[0] == "\u25b4"]
            rec("C5: two \u25b4 indicators at the two anchors (3 and 5)",
                arrows == [3, 5], f"arrows={arrows}")
            rec("C5: the code keeps its source columns (`a` at 4, `b` at 6)",
                col_of(code_cells, "a") == 4 and col_of(code_cells, "b") == 6,
                f"a col={col_of(code_cells, 'a')} b col={col_of(code_cells, 'b')}")
            rec("C5: each note \u256d at its own anchor (3 and 5)",
                col_of(row_cells(app, na), "\u256d") == 3
                and col_of(row_cells(app, nb), "\u256d") == 5,
                f"\u256d a={col_of(row_cells(app, na), '\u256d')} "
                f"\u256d b={col_of(row_cells(app, nb), '\u256d')}")
            dump(app, [na, nb, cb], "case 5: two annotations on one line")

            # Fold: both note rows vanish; one \u25b8 PER ANNOTATION (two).
            app.key("C-c")
            app.wait(0.4)
            app.key("a")
            app.wait(0.4)
            app.key("h")
            app.wait(0.8)
            folded_code = None
            for r in range(app.rows):
                cells = row_cells(app, r)
                if sum(1 for d in cells if d and d[0] == "\u25b8") == 2 and "a" in app.row_text(r):
                    folded_code = r
            ok_fold = (find_row(app, "note a") is None
                       and find_row(app, "note b") is None
                       and folded_code is not None)
            rec("C5: folded — both note rows gone, two \u25b8 at the two anchors",
                ok_fold and col_of(row_cells(app, folded_code), "a") == 4,
                f"folded_code={folded_code}")
            if ok_fold:
                dump(app, [folded_code], "case 5 folded: one \u25b8 per annotation")
    finally:
        app.kill()
        cleanup()

    bad = [n for n, ok, _ in CHECKS if not ok]
    print("\n=== SUMMARY ===")
    for n, ok, _ in CHECKS:
        print(f"  {'PASS' if ok else 'FAIL'}  {n}")
    print(f"\n{len(CHECKS) - len(bad)}/{len(CHECKS)} symbol-precise legs passed")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

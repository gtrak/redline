#!/usr/bin/env python3
"""Plan 018 issue 02 — isearch is a browsable list (the helm-occur shape).
U-E13 — isearch's second query dimension: a nucleo FILTER inside the
literal-search list (the shared-core seam, FilterOnly over the rows' line
projections). `C-o` toggles the filter input; the filter survives a
further literal-search character (the set it filters re-derives); C-g is
layered (first clears the filter, second cancels isearch).

The user's verbatim complaint: "the i-search results could be more
interactive and helm-like UX." `C-s`/`C-r` used to arm an invisible
cursor that teleported match-to-match with only a minibuffer echo. Now
the buffer's live match rows render as an OVERLAY in place of the buffer
content (the picker-overlay precedent, NOT a new ViewId): one row per
match — `line number + line text + the match's column` — in search order
(FilterOnly: the literal search's result set IS the list; typing narrows
it because the query IS the search), with a selection the buffer view
behind follows (the current-match highlight band tracks it).

Three legs, one App, own fixture repo (its own PTY-flock, keyed to the
repo path):

L-1  LIST + NARROW: `C-s` + `om` opens the list (both "café om*" lines,
     line numbers 1-based, the match's CHAR column — 5, not byte 6);
     typing `y` narrows it to the "café omyga" row alone (`[1/1]`);
     Backspace widens it back (`[2/2]`, the selection re-derives to the
     first match in the search direction from the point).
L-2  RET ON THE SELECTED MATCH: C-s moves the list selection to the
     second match; RET confirms it — the list and the highlight band
     vanish and the hardware cursor (the CUP the stream emits) lands on
     the SELECTED match's column: "café omyga"'s 'o' is CHAR 5 (BYTE 6 —
     the é is 2 bytes but 1 cell), display cell 5 → 1-based CUP column 6.
     A byte-based landing (char 6, the 'm' → CUP column 7) or a
     line-start landing (CUP column 1) fails this.
L-3  C-g RESTORE: a fresh `C-s` + `om` from the landed point re-opens
     the list; C-r moves the selection off the pre-search point; C-g
     restores the pre-search line AND column (the CUP returns to the
     exact cell it held before the search — not the line start), the
     list disappears, and the highlight band vanishes.
L-4  U-E13 FILTER: C-o arms the filter (echo `(filter: )`, full set
     still shown); typing `my` narrows to the "café omyga" row alone
     (`[1/1]`, the surviving match keeps the selection, CUP (5,6)); RET
     confirms the SELECTED filtered row — the L-2 landing outcome holds.
L-5  U-E13 FILTER SURVIVAL: C-o off keeps the filter; a further
     literal-search char (`y` → "omy") re-derives the match set AND the
     filter survives (`I-search: omy (filter: my) [1/1]`).
L-6  U-E13 HONEST EMPTY: a filter matching nothing (`myz`) shows NO
     rows, NO highlight, the `[no matches]` echo with the clause, and
     RET is an honest `[not found]` (no silent jump).
L-7  U-E13 LAYERED C-g: the FIRST C-g clears the filter (the full set
     comes back, the selection follows its match); the SECOND cancels
     (echo `cancel`, pre-search point restored).

Color mode: COLORTERM=truecolor (the match-highlight band's blue renders
as a BAR_BGS hex — pyte's 256 palette maps blue to 5c5cff, truecolor to
0000ff). The list's selected-row bar is the shared cursor treatment (the
picker's `invert` + gap-paint): the TEXT cells carry pyte's `reverse`
flag (their backgrounds stay the view black) and the painted gap reads
plain white — so bar detection is `blue_rows ∪ reverse_rows` (the band
is an explicit background, never reverse).

The drive owns /tmp/redline_018_02_isearch_repo and removes it on exit.
Wrap the invocation in `timeout` (shared PTY flock scheme: if the lock
is busy this exits 3 — wait and retry, never run two suites at once).

Exit 0 = legs pass; 1 = any failed.
"""
import os, sys, shutil, subprocess
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo

REPO = repo("redline_018_02_isearch_repo")
COLS, ROWS = 80, 24
MINI = ROWS - 2      # minibuffer row (0-based)

# Screen geometry (tree sidebar hidden): row 0 = the file title, content
# line i (0-based) = screen row i + 1 (the buffer text starts at screen
# column 0), row 22 = minibuffer, 23 = status. The isearch list (2 rows)
# is bottom-aligned above the minibuffer: screen rows 20 and 21 (1 row:
# row 21). A list row reads `    2 café omega …  (col 5)`: the 1-based
# line number right-aligned in the 4-cell gutter (cells 1..=4), one blank
# gutter cell, the line text, and the detail pinned to the right edge.
FILE = "t.txt"
CONTENT = (
    "line0 alpha\n"
    "café omega\n"   # line 1: 'om' at CHAR 5 / BYTE 6 (é is 2 bytes, 1 cell)
    "line2 beta\n"
    "café omyga\n"   # line 3: 'om' at CHAR 5 / BYTE 6 (é is 2 bytes, 1 cell)
    "line4 gamma\n"
)
ROW_OMEGA = "    2 café omega"
ROW_OMYGA = "    4 café omyga"

CHECKS = []


def rec(name, ok, detail=""):
    CHECKS.append((name, ok, detail))
    print(f"  {'PASS' if ok else 'FAIL'}  {name:60s} {detail}")


def list_rows(app):
    """The screen rows that ARE list rows (carry the `(col N)` detail)."""
    return [r for r in app.screen_text().splitlines() if "(col " in r]


def setup_repo():
    if os.path.exists(REPO):
        shutil.rmtree(REPO)
    os.makedirs(REPO)
    with open(os.path.join(REPO, FILE), "w") as f:
        f.write(CONTENT)
    env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@e.com",
               GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@e.com")
    subprocess.run(["git", "init", "-q", REPO], check=True, env=env)
    subprocess.run(["git", "-C", REPO, "add", "-A"], check=True, env=env,
                   capture_output=True)
    subprocess.run(["git", "-C", REPO, "commit", "-qm", "018-02 fixture"],
                   check=True, env=env, capture_output=True)


def mini(app):
    # The message sits at column 1 of the minibuffer row (a leading-space
    # pad) and the renderer right-pads the row to full width — strip the
    # padding, keep the message itself byte-for-byte.
    return app.row_text(MINI).strip()


def bar_rows(app):
    """Rows carrying the selection signature: the explicit-background blue
    (the highlight band, the status bar) OR the inverted bar cells (the
    list's selected row — pyte keeps `reverse` as a flag, its background
    stays the view black)."""
    return set(app.blue_rows()) | set(app.reverse_rows())


def cup(app):
    """The surviving CUP (1-based row, col) — the cursor position the user
    sees (the landing-column oracle, drive_xref's jump-column-pty rule)."""
    return app.cup_settle()


def main():
    app = None
    try:
        setup_repo()
        app = App(REPO, rows=ROWS, cols=COLS, colorterm="truecolor")
        app.key("C-x C-f", 1.0)
        for ch in FILE:
            app.key(ch, 0.25)
        app.key("RET", 1.2)
        rec("open: the file buffer is on screen",
            FILE in app.screen_text() and "café omega" in app.screen_text())

        print("=== L-1: C-s opens the list; typing narrows it ===")
        app.key("C-s", 0.8)
        rec("L-1: C-s arms isearch (minibuffer prompt, byte-for-byte incl. the trailing space)",
            app.row_text(MINI).lstrip()[:10] == "I-search: ",
            f"minibuffer={app.row_text(MINI).lstrip()[:12]!r}")
        app.key("o", 0.4)
        app.key("m", 0.4)
        text = app.screen_text()
        rows = list_rows(app)
        rec("L-1: the list shows BOTH match rows (1-based line number, line text, match column)",
            any(r.startswith(ROW_OMEGA) and r.endswith("(col 5)") for r in rows)
            and any(r.startswith(ROW_OMYGA) and r.endswith("(col 5)") for r in rows),
            f"rows={rows!r}")
        rec("L-1: the minibuffer keeps its echo byte-for-byte",
            mini(app) == "I-search: om [1/2]", f"minibuffer={mini(app)!r}")
        r, c = cup(app)
        rec("L-1: the point (CUP) is on the first-in-direction match (line 1, display cell 5 → 1-based col 6)",
            (r, c) == (3, 6), f"cup=({r},{c}) want (3,6)")
        blues = bar_rows(app)
        rec("L-1: the highlight band tracks the selection behind the list (content row 2) and the list bar (row 20)",
            blues == {2, 20}, f"bar={sorted(blues)}")

        # Narrow: 'omy' matches only line 3.
        app.key("y", 0.4)
        rows = list_rows(app)
        rec("L-1: typing narrows the list to the single matching row",
            any(r.startswith(ROW_OMYGA) for r in rows)
            and not any(r.startswith(ROW_OMEGA) for r in rows),
            f"rows={rows!r}")
        rec("L-1: the echo follows the narrowed count",
            mini(app) == "I-search: omy [1/1]", f"minibuffer={mini(app)!r}")
        r, c = cup(app)
        rec("L-1: the selection re-derives to the (only) match — the point follows (line 3)",
            (r, c) == (5, 6), f"cup=({r},{c}) want (5,6)")

        # Backspace widens it back; the selection re-derives from the point
        # (line 3 — the first match at or after the point).
        app.key("C-h", 0.4)
        rows = list_rows(app)
        rec("L-1: Backspace widens the list back to both rows",
            any(r.startswith(ROW_OMEGA) for r in rows)
            and any(r.startswith(ROW_OMYGA) for r in rows),
            f"rows={rows!r}")
        rec("L-1: the selection re-derives to the first match in the search direction from the point (line 3 → [2/2])",
            mini(app) == "I-search: om [2/2]", f"minibuffer={mini(app)!r}")
        blues = bar_rows(app)
        rec("L-1: the band + list bar moved with the selection (content row 4, list row 21)",
            blues == {4, 21}, f"bar={sorted(blues)}")

        print("=== L-2: C-s moves the selection; RET confirms the SELECTED match ===")
        app.key("C-r", 0.4)   # selection back to the first match (line 1)
        rec("L-2: C-r moved the selection to the first match ([1/2])",
            mini(app) == "I-search: om [1/2]", f"minibuffer={mini(app)!r}")
        app.key("C-s", 0.4)   # selection onto the SECOND match (line 3)
        rec("L-2: C-s moved the selection to the second match ([2/2])",
            mini(app) == "I-search: om [2/2]", f"minibuffer={mini(app)!r}")
        r, c = cup(app)
        rec("L-2: the buffer view behind the list scrolled to the selected match (line 3)",
            (r, c) == (5, 6), f"cup=({r},{c}) want (5,6)")
        app.key("RET", 0.8)
        text = app.screen_text()
        rec("L-2: RET removes the list (no match rows on screen)",
            "(col " not in text,
            f"rows={list_rows(app)!r}")
        rec("L-2: RET clears the match highlight (the band vanishes with the session)",
            app.blue_rows() == [] and app.reverse_rows() == [],
            f"blue={app.blue_rows()} reverse={app.reverse_rows()}")
        r, c = cup(app)
        rec("L-2: the point landed ON the SELECTED match's column (line 3, char 5 — a byte landing (char 6, the 'm') would be col 7, a line start col 1)",
            (r, c) == (5, 6), f"cup=({r},{c}) want (5,6)")
        rec("L-2: the isearch echo clears on confirm (back to the default hint)",
            "I-search" not in app.row_text(MINI),
            f"minibuffer={app.row_text(MINI)[:12]!r}")

        print("=== L-3: C-g restores the pre-search line AND column; the highlight vanishes ===")
        app.key("C-s", 0.8)
        app.key("o", 0.4)
        app.key("m", 0.4)
        # Pre-search point: line 3, char 5 (the L-2 landing). Forward from
        # line 3: the first match at or after the point is line 3 itself.
        rec("L-3: a fresh query re-opens the list (2 rows, selection on line 3 → [2/2])",
            mini(app) == "I-search: om [2/2]"
            and any(r.startswith(ROW_OMEGA) for r in list_rows(app)),
            f"minibuffer={mini(app)!r}")
        app.key("C-r", 0.4)   # move the selection OFF the pre-search point (line 1)
        rec("L-3: C-r moved the selection off the pre-search point ([1/2])",
            mini(app) == "I-search: om [1/2]", f"minibuffer={mini(app)!r}")
        r, c = cup(app)
        rec("L-3: the selection is now on line 1 (the point left the pre-search point)",
            (r, c) == (3, 6), f"cup=({r},{c}) want (3,6)")
        app.key("C-g", 0.8)
        text = app.screen_text()
        rec("L-3: C-g echoes 'cancel' and removes the list",
            mini(app) == "cancel" and "(col " not in text,
            f"minibuffer={mini(app)!r}")
        rec("L-3: the match highlight vanishes on cancel (no blue rows, no inverted bar)",
            app.blue_rows() == [] and app.reverse_rows() == [],
            f"blue={app.blue_rows()} reverse={app.reverse_rows()}")
        r, c = cup(app)
        rec("L-3: the pre-search LINE AND COLUMN are restored (line 3, char 5 → CUP (5,6) — not the line start (col 1), not where the selection sat ((3,6)))",
            (r, c) == (5, 6), f"cup=({r},{c}) want (5,6); pre-search=(5,6), selection=(3,6)")

        print("=== L-4: C-o arms the filter; typing narrows the matches (FilterOnly) ===")
        # Pre-search point: line 3, char 5 (the L-2/L-4 landing) — forward
        # from it, the first match is line 3 itself → [2/2].
        app.key("C-s", 0.8)
        app.key("o", 0.4)
        app.key("m", 0.4)
        rec("L-4: the list re-opens at [2/2] (both rows, selection on line 3)",
            mini(app) == "I-search: om [2/2]" and len(list_rows(app)) == 2,
            f"minibuffer={mini(app)!r} rows={list_rows(app)!r}")
        app.key("C-o", 0.4)
        rec("L-4: C-o arms the filter input (clause shown, the full set is still the list)",
            mini(app) == "I-search: om (filter: ) [2/2]" and len(list_rows(app)) == 2,
            f"minibuffer={mini(app)!r} rows={list_rows(app)!r}")
        app.key("m", 0.4)
        app.key("y", 0.4)
        rows = list_rows(app)
        rec("L-4: the filter narrows to the one surviving row (line 4: 'café omyga')",
            len(rows) == 1 and any(r.startswith(ROW_OMYGA) for r in rows)
            and not any(r.startswith(ROW_OMEGA) for r in rows),
            f"rows={rows!r}")
        rec("L-4: the echo carries the filter clause and the FILTERED count (byte-for-byte)",
            mini(app) == "I-search: om (filter: my) [1/1]", f"minibuffer={mini(app)!r}")
        r, c = cup(app)
        rec("L-4: the selection stays on the surviving match (CUP (5,6))",
            (r, c) == (5, 6), f"cup=({r},{c}) want (5,6)")
        app.key("RET", 0.8)
        rec("L-4: RET confirms the SELECTED filtered row — the old outcome holds (list gone, CUP on line 3 col 5, no highlight)",
            "(col " not in app.screen_text() and cup(app) == (5, 6)
            and app.blue_rows() == [] and app.reverse_rows() == [],
            f"cup={cup(app)} blue={app.blue_rows()}")

        print("=== L-5: the filter SURVIVES a further literal-search character ===")
        app.key("C-s", 0.8)
        app.key("o", 0.4)
        app.key("m", 0.4)
        app.key("C-o", 0.4)
        app.key("m", 0.4)
        app.key("y", 0.4)   # filter 'my' → one row
        app.key("C-o", 0.4)  # toggle the filter input OFF — the filter survives
        rec("L-5: C-o off keeps the filter (clause + narrowed set)",
            mini(app) == "I-search: om (filter: my) [1/1]"
            and len(list_rows(app)) == 1,
            f"minibuffer={mini(app)!r} rows={list_rows(app)!r}")
        app.key("y", 0.4)   # literal 'omy' — re-derives the match set
        rows = list_rows(app)
        rec("L-5: the literal extension re-derives the set AND the filter survives",
            mini(app) == "I-search: omy (filter: my) [1/1]"
            and len(rows) == 1 and any(r.startswith(ROW_OMYGA) for r in rows),
            f"minibuffer={mini(app)!r} rows={rows!r}")
        r, c = cup(app)
        rec("L-5: the point stays on the surviving match (line 3 → CUP (5,6))",
            (r, c) == (5, 6), f"cup=({r},{c}) want (5,6)")

        print("=== L-6: a filter matching nothing shows an honest empty state ===")
        app.key("C-h", 0.4)  # literal back to 'om' (2 matches; the filter keeps line 3)
        rec("L-6: backspacing the literal re-derives the set, the filter survives",
            mini(app) == "I-search: om (filter: my) [1/1]"
            and len(list_rows(app)) == 1,
            f"minibuffer={mini(app)!r} rows={list_rows(app)!r}")
        app.key("C-o", 0.4)  # re-arm the filter input (L-5 parked it)
        app.key("z", 0.4)    # filter 'myz' — no survivor
        rows = list_rows(app)
        rec("L-6: the list goes away (no stale rows)", rows == [], f"rows={rows!r}")
        rec("L-6: the echo says [no matches] with the filter clause (byte-for-byte)",
            mini(app) == "I-search: om (filter: myz) [no matches]",
            f"minibuffer={mini(app)!r}")
        rec("L-6: the highlight vanishes (no stale band, no list bar)",
            app.blue_rows() == [] and app.reverse_rows() == [],
            f"blue={app.blue_rows()} reverse={app.reverse_rows()}")
        app.key("RET", 0.8)
        rec("L-6: RET on an empty filtered set is an honest not-found (no silent jump)",
            mini(app) == "I-search: om [not found]" and cup(app) == (5, 6),
            f"minibuffer={mini(app)!r} cup={cup(app)}")

        print("=== L-7: layered C-g — first clears the filter, second cancels ===")
        app.key("C-s", 0.8)
        app.key("o", 0.4)
        app.key("m", 0.4)
        app.key("C-o", 0.4)
        app.key("m", 0.4)
        app.key("y", 0.4)   # [1/1] on line 3
        rec("L-7: precondition — the filter is active ([1/1] on line 3)",
            mini(app) == "I-search: om (filter: my) [1/1]",
            f"minibuffer={mini(app)!r}")
        app.key("C-g", 0.8)
        rec("L-7: the FIRST C-g clears the filter (full set back, isearch still active)",
            mini(app) == "I-search: om [2/2]" and len(list_rows(app)) == 2,
            f"minibuffer={mini(app)!r} rows={list_rows(app)!r}")
        r, c = cup(app)
        rec("L-7: the selection follows its match (line 3 → CUP (5,6), not reset to line 1)",
            (r, c) == (5, 6), f"cup=({r},{c}) want (5,6)")
        app.key("C-g", 0.8)
        rec("L-7: the SECOND C-g cancels (echo 'cancel', list gone, pre-search point restored)",
            mini(app) == "cancel" and list_rows(app) == [] and cup(app) == (5, 6)
            and app.blue_rows() == [] and app.reverse_rows() == [],
            f"minibuffer={mini(app)!r} cup={cup(app)}")
    finally:
        if app is not None:
            try:
                app.kill()
            except Exception:
                pass
        shutil.rmtree(REPO, ignore_errors=True)

    failed = [c for c in CHECKS if not c[1]]
    print(f"\n{len(CHECKS) - len(failed)}/{len(CHECKS)} legs passed")
    if failed:
        for name, _, detail in failed:
            print(f"  FAILED: {name} {detail}")
        sys.exit(1)
    sys.exit(0)


if __name__ == "__main__":
    main()

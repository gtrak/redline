#!/usr/bin/env python3
"""issue-redo-live-leg -- the PTY leg for redo (`C-x U` / `C-M-7`).

Nothing has ever exercised redo through a real terminal. The UNDO leg exists
(`drive_redline_battery2.py` sends `0x1F` = `C-/`), but redo -- a user-facing
binding with unit tests and an oracle-settled point landing -- had no drive.
That is the "a lane that changes behaviour owns the drives" gap this closes.

Oracle (emacs 30.2, measured): for `abc <left> def` -> undo -> redo the text
reads `abdefc` AND the point stays at the START of the redone insertion
(char index 2 -> 1-based column 3), which is exactly what the implementation's
`land_point_at_char(key, start)` does. A leg that checked only the text would
pass with the cursor in the wrong place, so the COLUMN is pinned, not just
the text.

The cursor is pinned BEHAVIORALLY, not by the hardware CUP: in this buffer
view a self-insert / redo draws the point as a highlight and parks the
terminal cursor elsewhere (measured: the surviving CUP stays at a fixed row
through the whole edit), so the surviving CUP is NOT the point. The column is
therefore pinned by where the NEXT self-insert lands -- a char typed right
after the redo must land at the start of the redone run. This is the same
row-content method the `accurate-mode` sweep flow uses to pin a landing.

Target: the notes buffer (`.redline-notes.md`) toggled into Accurate mode.
A bare `*scratch*` buffer has no path, so it never self-inserts; the notes
buffer is the editable, per-char-point buffer the edit flows use, and its
on-disk file is already on the fixture's cleanup list (no stray).

Legs (each leg group runs in its own app session so the notes buffer's undo /
redo stacks are clean):
  R1  minimal leg (C-x U): type a run (`abc`); `C-x u` reverts it to the bare
      header (proving the self-insert run is coalesced); `C-x U` restores it
      byte-identically.
  R2  oracle pin (C-x U): `abc <left> def` -> `C-x u` -> `C-x U`. The line
      reads `abdefc` and the next char lands at the start of the redone
      run (`abXdefc`), pinning the point at char index 2.
  R3  second binding (C-M-7): the byte-based `ESC 0x1F` shape of emacs's
      `C-M-_`. Same oracle sequence, but the redo key is the raw `ESC 0x1F`
      bytes the terminal sends for Alt+Ctrl-_; it must restore the text and
      land the cursor identically.

Exit 0 = all assertions pass; 1 = any failed.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo, reset

REPO = os.environ.get("REDLINE_REPO") or repo("redline_pyte_repo")
ROWS, COLS = 24, 80
# The notes buffer's baseline content line: every text assertion is the
# edited prefix + this header, so a "byte-identical" restore is checked
# against the whole line, not a bare substring.
HEADER = "# Notes"

CHECKS = []


def rec(name, ok, detail=""):
    CHECKS.append((name, ok, detail))
    print(f"  {'PASS' if ok else 'FAIL'}  {name:56s} {detail}")


def line0(app):
    # Content line 0 (terminal row 1, 0-based) is where the notes buffer's
    # editable text lives; the title is row 0.
    return app.row_text(1).rstrip()


def open_notes_accurate(app):
    # Open the notes buffer (has a path -> self-inserts) and toggle it into
    # Accurate mode (per-char point; Annotation mode would append at the end).
    app.key("C-x n")
    app.wait(0.8)
    app.key("C-x C-q")
    app.wait(0.6)


def run_oracle(app, redo_key, redo_label, probe_label):
    """`abc <left> def` -> C-x u -> <redo>. Asserts the text reverts and is
    restored byte-identically, then pins the cursor with a probe char that
    must land at the start of the redone run."""
    # `abc <left> def`: type the run, step left (C-b = `<left>`), then a
    # second run that inserts mid-line, before the `c`.
    app.type_text("abc", settle=0.6)
    app.key("C-b", settle=0.4)
    app.type_text("def", settle=0.6)
    rec(f"{probe_label}: mid-line insert reads `abdefc`",
        line0(app) == "abdefc" + HEADER, f"line0={line0(app)!r}")
    app.key("C-x u", settle=0.6)
    rec(f"{probe_label}: C-x u reverts the `def` run (back to `abc`)",
        line0(app) == "abc" + HEADER, f"line0={line0(app)!r}")
    if isinstance(redo_key, bytes):
        # A raw byte sequence (the C-M-7 `ESC 0x1F` shape) -- feed it verbatim,
        # it is not an emacs-notation key string.
        app.feed(redo_key, settle=0.6)
    else:
        app.key(redo_key, settle=0.6)
    rec(f"{probe_label}: {redo_label} restores `abdefc` byte-identically",
        line0(app) == "abdefc" + HEADER, f"line0={line0(app)!r}")
    # Cursor pin: the point must sit at the START of the redone insertion
    # (char index 2). A col-0 or end-of-run landing puts the probe char
    # somewhere else.
    app.key("X", settle=0.5)
    rec(f"{probe_label}: point is at the START of the redone run (`abXdefc`)",
        line0(app) == "abXdefc" + HEADER, f"line0={line0(app)!r}")


def session(label, leg):
    """Run `leg(app)` in a FRESH app session so the notes buffer's undo / redo
    stacks start clean (a new process = in-memory baseline `# Notes`)."""
    reset()
    app = App(REPO, rows=ROWS, cols=COLS)
    try:
        open_notes_accurate(app)
        rec(f"{label}: notes buffer is in Accurate (editable) mode",
            "Accurate" in app.row_text(app.rows - 1)
            or "accurate mode" in app.row_text(app.rows - 2),
            f"status={app.row_text(app.rows - 1)!r}")
        leg(app)
    finally:
        app.kill()


def leg_r1(app):
    # R1: the minimal leg. type a run -> undo reverts -> redo restores it
    # byte-identically. (The revert also proves the 3-char run is coalesced
    # into one undo step: one C-x u removes all of it.)
    app.type_text("abc", settle=0.6)
    rec("R1: typed run lands before the header", line0(app) == "abc" + HEADER,
        f"line0={line0(app)!r}")
    app.key("C-x u", settle=0.6)
    rec("R1: C-x u reverts the run (self-insert coalesced, back to header)",
        line0(app) == HEADER, f"line0={line0(app)!r}")
    app.key("C-x U", settle=0.6)
    rec("R1: C-x U restores the run byte-identically", line0(app) == "abc" + HEADER,
        f"line0={line0(app)!r}")


def leg_r2(app):
    # R2: the oracle pin (text + cursor) on the primary C-x U binding.
    run_oracle(app, "C-x U", "C-x U", "R2")


def leg_r3(app):
    # R3: the second binding, C-M-7. Alt+Ctrl-_ arrives as `ESC 0x1F` on
    # byte-based terminals, which crossterm 0.29 decodes as Char('7') +
    # CONTROL | ALT = the app key `C-M-7`. Same oracle, different redo key.
    run_oracle(app, b"\x1b\x1f", "C-M-7 (raw ESC 0x1F)", "R3")


def main():
    # Each session's App() call already waits until the mode line reads
    # `ready` (App.wait_ready), so the app is up before any leg runs.
    print(f"REPO={REPO}\n")
    print("=== R1: type `abc` -> C-x u (revert) -> C-x U (restore) ===")
    session("R1", leg_r1)
    print("=== R2: `abc <left> def` -> C-x u -> C-x U (oracle: `abdefc`, cursor) ===")
    session("R2", leg_r2)
    print("=== R3: second binding C-M-7 (raw ESC 0x1F) ===")
    session("R3", leg_r3)
    reset()

    bad = [n for n, ok, _ in CHECKS if not ok]
    print("\n=== SUMMARY ===")
    for n, ok, _ in CHECKS:
        print(f"  {'PASS' if ok else 'FAIL'}  {n}")
    print(f"\n{len(CHECKS) - len(bad)}/{len(CHECKS)} redo-live legs passed")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Plan 007 issue 02 — syntax-anchored annotations, the PTY drive.

Drives the REAL binary through the syntax-anchor end-to-end drive that
the unit test covers headlessly:

  1. seed `src/synleg.rs` with a single `fn target_one() { ... }`;
  2. open it, put the point ON the function name (M-< then C-f x3),
     and commit a note via the `A` prompt;
  3. assert the on-disk record carries the syntax keys
     (`syntax_kind: identifier` / `syntax_name: target_one`);
  4. rewrite the file OUT OF BAND: a 100-line insertion on top AND a
     reformat of the signature line (the anchored line's exact text
     disappears from the file — the ±25-line text path provably has
     nothing to match, inside or outside the window);
  5. `g` (force reload) runs the re-anchor pass; assert the note's
     marker + note row follow the function to its new line, the
     on-disk record re-anchored to `line: 100` with `orphaned: false`,
     and no `(orphaned)` tag renders.

This suite owns the shared fixture under the shared PTY flock (via
pyte_driver), resets the fixture baseline at start, and removes its own
stray files (src/synleg.rs, .redline-notes.md) at the end. Wrap the
invocation in `timeout`.

Exit 0 = all assertions pass; 1 = any failed.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import reset

REPO = os.environ.get("REDLINE_REPO", "/tmp/redline_pyte_repo")
ROWS, COLS = 24, 80

CHECKS = []


def rec(name, ok, detail=""):
    CHECKS.append((name, ok, detail))
    print(f"  {'PASS' if ok else 'FAIL'}  {name:56s} {detail}")


def text(app):
    return "\n".join(app.row_text(r) for r in range(app.rows))


def rows_containing(app, token):
    return [r for r in range(app.rows) if token in app.row_text(r)]


ORIGINAL = "fn target_one() {\n    let x = 1;\n    x\n}\n"
# The out-of-band rewrite: 100 filler lines on top (far outside ±25) AND
# the signature line reformatted — `fn target_one() {` no longer exists
# anywhere, so ONLY the syntax anchor can re-place the note.
REWRITTEN = "\n".join(f"// filler {i}" for i in range(100)) \
    + "\nfn target_one()\n{\n    let x = 1;\n    x\n}\n"
assert "fn target_one() {" not in REWRITTEN  # the text path's precondition

LEG = os.path.join(REPO, "src", "synleg.rs")
NOTES = os.path.join(REPO, ".redline-notes.md")


def read_notes():
    if os.path.exists(NOTES):
        with open(NOTES) as f:
            return f.read()
    return ""


def main():
    print(f"REPO={REPO}\n")
    reset()
    with open(LEG, "w") as f:
        f.write(ORIGINAL)
    app = App(REPO, rows=ROWS, cols=COLS)
    try:
        ready = "ready" in text(app)
        rec("S0: the app reached ready", ready, f"row0={app.row_text(0)!r}")

        # Open src/synleg.rs via the find-file picker.
        app.key("C-x C-f")
        app.wait(0.8)
        app.type_text("synleg.rs", settle=0.6)
        app.key("RET")
        app.wait(0.8)
        opened = "fn target_one() {" in text(app)
        rec("S1: src/synleg.rs is open", opened, f"row0={app.row_text(0)!r}")

        # Point ON the function name: buffer start, then 3 char-forwards
        # (`f`,`n`,` `, then `t` of `target_one`).
        app.key("M-<")
        app.wait(0.3)
        for _ in range(3):
            app.key("C-f", settle=0.2)
        app.key("A")
        app.wait(0.5)
        prompt = "Note: " in app.row_text(app.rows - 2)
        app.type_text("follow fn", settle=0.5)
        app.key("RET")
        app.wait(0.8)
        saved = "note saved" in app.row_text(app.rows - 2)
        rec("S2: A prompt opened and RET committed the note",
            prompt and saved, f"minibuffer={app.row_text(app.rows - 2)!r}")

        disk = read_notes()
        has_keys = ("syntax_kind: identifier" in disk
                    and "syntax_name: target_one" in disk)
        at_line_0 = "line: 0" in disk and "anchor: fn target_one() {" in disk
        rec("S3: the on-disk record carries the syntax keys",
            has_keys and at_line_0, f"notes={disk!r}")

        # Out-of-band rewrite: the 100-line insertion + the reformat.
        with open(LEG, "w") as f:
            f.write(REWRITTEN)
        app.key("g")
        app.wait(1.0)
        reloaded = ("reloaded" in app.row_text(app.rows - 2)
                    and "// filler 0" in text(app))
        rec("S4: g force-reloaded the rewritten file", reloaded,
            f"minibuffer={app.row_text(app.rows - 2)!r}")

        # Jump to the end so the function (now at line 100) is in view.
        app.key("M->")
        app.wait(0.6)
        fn_rows = rows_containing(app, "target_one")
        marker = any("\u258e" in app.row_text(r) for r in fn_rows)
        note_under = bool(fn_rows) and any(
            r + 1 < app.rows and "follow fn" in app.row_text(r + 1)
            for r in fn_rows)
        rec("S5: the marker + note row FOLLOWED the function to its new line",
            bool(fn_rows) and marker and note_under,
            f"fn_rows={fn_rows} marker={marker} note_under={note_under}")
        not_orphaned = "(orphaned)" not in text(app)
        rec("S6: no (orphaned) tag anywhere in the view", not_orphaned,
            f"orphaned-visible={not not_orphaned}")

        disk = read_notes()
        rec("S7: the on-disk record re-anchored to line 100, not orphaned",
            "line: 100" in disk and "orphaned: false" in disk
            and "syntax_name: target_one" in disk,
            f"notes={disk!r}")
    finally:
        app.kill()
        for p in (LEG, NOTES):
            try:
                os.remove(p)
            except OSError:
                pass
    bad = [n for n, ok, _ in CHECKS if not ok]
    print("\n=== SUMMARY ===")
    for n, ok, _ in CHECKS:
        print(f"  {'PASS' if ok else 'FAIL'}  {n}")
    print(f"\n{len(CHECKS) - len(bad)}/{len(CHECKS)} syntax-notes legs passed")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

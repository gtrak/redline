#!/usr/bin/env python3
"""Probe: the annotations picker frame for two records on ONE line.

Seeds src/perc.rs + .redline-notes.md (two records, same (path, line),
different cols, with syntax names a/b), opens the annotations picker
(C-c n a), and dumps the frame rows. Used to quote the screen BEFORE and
AFTER the same-line distinguishability fix. Runs against a PRIVATE fixture
root (REDLINE_FIXTURE_ROOT=/tmp/fx3) so it never touches the shared
battery fixture.
"""
import os
import shutil
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
os.environ["REDLINE_FIXTURE_ROOT"] = "/tmp/fx3"
from pyte_driver import App
from fixture import repo

REPO = repo("redline_pyte_repo")
ROWS, COLS = 24, 80

FILE_CONTENT = "fn f() {\n    let a = b;\n}\n"
NOTES_CONTENT = (
    "# Notes\n\n<!-- redline-annotations:begin -->\n"
    "[annotation]\npath: src/perc.rs\nline: 1\ncol: 8\n"
    "anchor:     let a = b;\nnote: note a\n"
    "syntax_kind: identifier\nsyntax_name: a\n"
    "[annotation]\npath: src/perc.rs\nline: 1\ncol: 12\n"
    "anchor:     let a = b;\nnote: note b\n"
    "syntax_kind: identifier\nsyntax_name: b\n"
    "<!-- redline-annotations:end -->\n"
)


def main():
    fx3 = "/tmp/fx3"
    if os.path.exists(fx3):
        shutil.rmtree(fx3)
    os.makedirs(fx3)
    shutil.copytree("/tmp/redline_pyte_repo", REPO,
                    ignore=shutil.ignore_patterns("target"))
    with open(os.path.join(REPO, "src", "perc.rs"), "w") as f:
        f.write(FILE_CONTENT)
    with open(os.path.join(REPO, ".redline-notes.md"), "w") as f:
        f.write(NOTES_CONTENT)

    app = App(REPO, rows=ROWS, cols=COLS)
    try:
        print("== root frame ==")
        for r in range(app.rows):
            print(f"{r:2d}| {app.row_text(r)!r}")
        app.key("C-c n a", settle=1.5)
        print("== annotations picker frame ==")
        for r in range(app.rows):
            print(f"{r:2d}| {app.row_text(r)!r}")
    finally:
        app.kill()
    print("PROBE DONE")


if __name__ == "__main__":
    main()

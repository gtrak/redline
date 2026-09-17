#!/usr/bin/env python3
"""Issue 003-02 — shared windowing drives for the four remaining panes.

Drives the real binary in a sized PTY (80x24). Each scenario is
self-contained and builds its own disposable git fixture under /tmp so the
main fixture baseline (/tmp/redline_pyte_repo) is never touched:

  * commit-diff: scroll a diff taller than the viewport — `M->` lands on the
    last row, `M-<` round-trips to the top, `C-n` steps the window one row.
  * blame: the cursor-following window keeps `b.selected` in view across a
    long file (`C-n` past the bottom, `M->` to the last line).
  * log: the in-page selection stays in view after many in-page moves on a
    page taller than the viewport.
  * notes: typing near the bottom keeps the active (insertion) row in view.

Run:  python3 tools/drive_windowing_panes.py
"""
import os
import subprocess
import sys

sys.path.insert(0, "/home/gary/dev/red/tools")
from pyte_driver import App

COLS, ROWS = 80, 24
VERDICTS = []


def verdict(name, ok, detail=""):
    VERDICTS.append((name, ok, detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name:32s} {detail}")


def git(root, *args):
    env = dict(os.environ)
    env.update({
        "GIT_AUTHOR_NAME": "Test",
        "GIT_AUTHOR_EMAIL": "t@e.com",
        "GIT_COMMITTER_NAME": "Test",
        "GIT_COMMITTER_EMAIL": "t@e.com",
    })
    subprocess.run(["git", "-C", root, *args], check=True,
                   capture_output=True, env=env)


# ── fixtures ─────────────────────────────────────────────────────────────────
DIFF_REPO = "/tmp/redline_win_diff_repo"
LOG_REPO = "/tmp/redline_win_log_repo"
NOTES_REPO = "/tmp/redline_win_notes_repo"


def ensure_diff_repo():
    """A repo with a base commit and one tall commit (big.txt -> 60 lines, the
    last of which is the `BOTTOM_SENTINEL` marker)."""
    if os.path.isdir(os.path.join(DIFF_REPO, ".git")):
        return
    os.makedirs(DIFF_REPO, exist_ok=True)
    git(DIFF_REPO, "init", "-q", "-b", "main")
    git(DIFF_REPO, "config", "user.name", "T")
    git(DIFF_REPO, "config", "user.email", "t@e.com")
    with open(os.path.join(DIFF_REPO, "big.txt"), "w") as f:
        f.write("l1\n")
    git(DIFF_REPO, "add", "-A")
    git(DIFF_REPO, "commit", "-q", "-m", "base")
    with open(os.path.join(DIFF_REPO, "big.txt"), "w") as f:
        for i in range(1, 60):
            f.write(f"line {i}\n")
        f.write("BOTTOM_SENTINEL\n")
    git(DIFF_REPO, "add", "-A")
    git(DIFF_REPO, "commit", "-q", "-m", "tall-lines")


def ensure_log_repo():
    """A repo with 30 commits; the current-branch log page (25) exceeds the
    ~19-row window, so the in-page selection must be kept in view."""
    if os.path.isdir(os.path.join(LOG_REPO, ".git")):
        return
    os.makedirs(LOG_REPO, exist_ok=True)
    git(LOG_REPO, "init", "-q", "-b", "main")
    git(LOG_REPO, "config", "user.name", "T")
    git(LOG_REPO, "config", "user.email", "t@e.com")
    for i in range(30):
        with open(os.path.join(LOG_REPO, f"s{i}.txt"), "w") as f:
            f.write(f"filler {i}\n")
        git(LOG_REPO, "add", "-A")
        git(LOG_REPO, "commit", "-q", "-m", f"commit {i}")


def ensure_notes_repo():
    """A repo whose notes file is pre-seeded with 30 lines (no trailing
    newline) so opening notes loads a buffer taller than the viewport."""
    if os.path.isdir(os.path.join(NOTES_REPO, ".git")):
        return
    os.makedirs(NOTES_REPO, exist_ok=True)
    git(NOTES_REPO, "init", "-q", "-b", "main")
    git(NOTES_REPO, "config", "user.name", "T")
    git(NOTES_REPO, "config", "user.email", "t@e.com")
    with open(os.path.join(NOTES_REPO, "README.md"), "w") as f:
        f.write("# notes fixture\n")
    git(NOTES_REPO, "add", "-A")
    git(NOTES_REPO, "commit", "-q", "-m", "init")
    with open(os.path.join(NOTES_REPO, ".redline-notes.md"), "w") as f:
        for i in range(1, 30):
            f.write(f"note line {i}\n")
        f.write("note line 30")  # last line, no trailing newline


# ── scenarios ────────────────────────────────────────────────────────────────

def drive_commit_diff():
    ensure_diff_repo()
    app = App(DIFF_REPO, rows=ROWS, cols=COLS)
    app.key("C-x g")   # magit status
    app.key("l")       # log
    app.wait(0.8)
    app.key("RET")     # open the newest commit's diff (the tall one)
    app.wait(1.2)
    # At the top the bottom sentinel must be off-screen (the diff is taller
    # than the viewport).
    top_hidden = "BOTTOM_SENTINEL" not in app.screen_text()
    # M-> lands on the full last page: both the last two diff rows are
    # visible (suggestion 3: pin the full-last-page semantic, not merely
    # sentinel visibility).
    app.key("M->")
    app.wait(0.8)
    bottom_text = app.screen_text()
    last_visible = "BOTTOM_SENTINEL" in bottom_text and "line 59" in bottom_text
    # M-< round-trips to the top: the sentinel is gone again.
    app.key("M-<")
    app.wait(0.8)
    roundtrip = "BOTTOM_SENTINEL" not in app.screen_text()
    # C-n steps the window down one row (the visible content shifts).
    before = [app.row_text(r) for r in range(1, app.rows - 2)]
    app.key("C-n")
    app.wait(0.6)
    after = [app.row_text(r) for r in range(1, app.rows - 2)]
    stepped = before != after
    # C-p steps it back.
    app.key("C-p")
    app.wait(0.6)
    back = [app.row_text(r) for r in range(1, app.rows - 2)]
    app.kill()
    ok = top_hidden and last_visible and roundtrip and stepped
    verdict("commit-diff (M->/M-< + C-n/C-p)", ok,
            f"top-hidden={top_hidden} last-visible={last_visible} "
            f"M-<-roundtrip={roundtrip} C-n-stepped={stepped}")


def drive_blame():
    ensure_diff_repo()  # has big.txt (60 lines) committed on main
    app = App(DIFF_REPO, rows=ROWS, cols=COLS)
    # Open big.txt in the buffer view.
    app.key("C-x C-f")
    app.wait(0.8)
    for ch in "big":
        app.key(ch, settle=0.25)
    app.key("RET")
    app.wait(1.0)
    app.key("C-x g")
    app.key("b")     # blame the current buffer's file
    app.wait(1.2)
    # Move the cursor down past the first window; the (single) blue cursor
    # row must stay inside the content area the whole way.
    in_window = []
    for _ in range(22):
        b = app.blue_rows()
        in_window.append(len(b) == 1 and 1 <= b[0] <= app.rows - 3)
        app.key("C-n")
    # M-> moves the cursor to the last line; exactly one blue row, in view.
    app.key("M->")
    app.wait(0.6)
    b = app.blue_rows()
    last_ok = len(b) == 1 and 1 <= b[0] <= app.rows - 3
    app.kill()
    verdict("blame (cursor follows window)", all(in_window) and last_ok,
            f"cursor in view {sum(in_window)}/22, M-> last-line {last_ok}")


def drive_log():
    ensure_log_repo()
    app = App(LOG_REPO, rows=ROWS, cols=COLS)
    app.key("C-x g")
    app.key("l")     # log (page of 25 entries > the ~19-row window)
    app.wait(1.2)
    # In-page selection: the (single) blue row stays in view across many
    # down-moves.
    in_window = []
    for _ in range(20):
        b = app.blue_rows()
        in_window.append(len(b) == 1 and 1 <= b[0] <= app.rows - 3)
        app.key("down")
    # `n` (next page) resets the in-page window to the top; the selection is
    # back on the first entry, in view.
    app.key("n")
    app.wait(0.8)
    b = app.blue_rows()
    paged_ok = len(b) == 1 and 1 <= b[0] <= app.rows - 3
    app.kill()
    verdict("log (in-page selection in view)", all(in_window) and paged_ok,
            f"selection in view {sum(in_window)}/20, after-next-page {paged_ok}")


def drive_notes():
    ensure_notes_repo()
    app = App(NOTES_REPO, rows=ROWS, cols=COLS)
    app.key("C-x n")  # open notes (loads the 30-line file)
    app.wait(1.2)
    # The last line is off-screen (the buffer is taller than the viewport).
    before = app.screen_text()
    bottom_hidden = "note line 30" not in before
    # Type one char: the insertion row (the last line) must scroll into view.
    app.key("Z")
    app.wait(0.8)
    after = app.screen_text()
    bottom_shown = "note line 30" in after
    # It must also be locally modified (a real edit, not a no-op).
    edited = "note line 30Z" in after
    app.kill()
    ok = bottom_hidden and bottom_shown and edited
    verdict("notes (typing keeps row in view)", ok,
            f"bottom-hidden-before={bottom_hidden} bottom-shown-after={bottom_shown} "
            f"self-insert={edited}")


if __name__ == "__main__":
    drive_commit_diff()
    drive_blame()
    drive_log()
    drive_notes()
    print("\n=== SUMMARY ===")
    for name, ok, detail in VERDICTS:
        print(f"{'PASS' if ok else 'FAIL'}  {name}")
    bad = [n for n, ok, _ in VERDICTS if not ok]
    print(f"\nTOTAL: {len(VERDICTS) - len(bad)}/{len(VERDICTS)} scenarios PASS")
    sys.exit(1 if bad else 0)

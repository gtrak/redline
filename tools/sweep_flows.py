#!/usr/bin/env python3
"""Plan 002 issue 03 — Sweep & re-measure: the U-* flow sweep table.

Drives the REAL binary in a sized PTY (80x24, the matrix's bold cell) through
every U-* flow in docs/ux-testing-plan.md that applies to the current build
and is verifiable by pyte reconstruction (no second pane, human hands, or a
10k-file repo needed). For each flow: the keys sent and the pyte-verified
outcome. Flows that require a live second pane / human hands / a special repo
are listed in NOT_APPLICABLE below, not silently passed.

This is the committed, reproducible evidence for the sweep table.
"""
import os
import re
import shutil
import subprocess
import sys
import time
sys.path.insert(0, "/home/gary/dev/red/tools")
from pyte_driver import App, encode_key, BAR_BGS
from fixture import reset as _reset_fixture

REPO = "/tmp/redline_pyte_repo"
# A second registered project for the U-G5 project-switch flow: a real, small
# git repo distinct from REPO, so `C-c p p` has a genuine second candidate.
REPO2 = "/tmp/redline_pyte_repo2"
# An isolated cache dir for the U-G5 flow: the app's project registry
# (cache_dir()/redline/projects.json) must list BOTH projects. XDG_CACHE_HOME
# points `dirs::cache_dir()` (and thus redline's persistence base) here so the
# real user cache is never touched.
G5_CACHE = "/tmp/redline_g5_cache"
# A dedicated, disposable git repo for the U-F5 commit flow: F5 creates a
# commit, so it runs here (never on REPO) to keep REPO's history/baseline
# pristine across repeated sweep runs.
REPO5 = "/tmp/redline_pyte_repo_commit"
# A dedicated repo for the U-H2 "search running" C-g state: large enough that
# the walk is still in flight when C-g lands immediately after RET (the small
# fixture repo finishes a search before C-g can arrive).
SLOW_REPO = "/tmp/redline_sweep_slow_repo"
# Throwaway repos for the plan-003-03 new windowing/scroll sweep legs (kept
# off REPO so the main fixture baseline is never touched; the deep drives live
# in tools/drive_windowing_panes.py and are cited by the thin legs below):
# a repo with a tall committed file (commit-diff + blame legs) and a repo with
# a notes file taller than the viewport (notes-scroll leg).
WIN_DIFF_REPO = "/tmp/redline_sweep_win_diff"
WIN_NOTES_REPO = "/tmp/redline_sweep_win_notes"
SLOW_FILES = 6000
ROWS, COLS = 24, 80

RESULTS = []


def record(flow, keys, ok, evidence):
    RESULTS.append((flow, keys, ok, evidence))
    print(f"{'PASS' if ok else 'FAIL'}  {flow:8s} keys={keys:26s} {evidence}")


def count_line(app):
    """The picker/palette count line ('N of M', right-aligned), if present.
    Returns (n, m) as strings, or None."""
    for r in range(app.rows):
        m = re.match(r"^\s*(\d+) of (\d+)\s*$", app.row_text(r))
        if m:
            return m.group(1), m.group(2)
    return None


def text(app):
    return "\n".join(app.row_text(r) for r in range(app.rows))


def row0(app):
    return app.row_text(0)


def has(app, s):
    return s in text(app)


def rows_containing(app, token):
    """Row indices whose text contains `token` (the no-wrap discriminator)."""
    return [r for r in range(app.rows) if token in app.row_text(r)]


def pump(app, secs):
    """Drain the PTY for `secs` on a real wall clock, no quiet-exit.

    `App.wait`/`App.key` return after ~0.2 s of silence (the quiet-exit in
    `_read`), which is the right behavior for a keypress (the app repaints
    immediately) but the WRONG tool when waiting on a watcher event: the app
    is silent until the 400 ms debounce fires. A watcher wait must run on a
    real deadline, so every disk-write-then-watch leg pumps the PTY this way.
    """
    deadline = time.time() + secs
    while time.time() < deadline:
        app._read(0.2, quiet=0.0)


def wait_for(app, pred, timeout=4.0):
    """Poll the pyte screen until `pred()` is true (real wall-clock deadline).
    Returns whether it landed before `timeout`."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        app._read(0.2, quiet=0.0)
        if pred():
            return True
    return pred()


def flow_a1(app):
    """U-A1 Cold start: *scratch* frame + status line + ready."""
    t = text(app)
    ok = "*scratch*" in row0(app) and "ready" in t and "* redline_pyte_repo *  *scratch*" in t
    record("U-A1", "(launch)", ok, f"title={row0(app)!r}, status present={'* redline_pyte_repo' in t}")


def flow_b1(app):
    """U-B1 C-x C-f: prompt, filter, RET opens the highlighted file."""
    app.key("C-x C-f")
    app.wait(0.8)
    prompt_ok = "Find file:" in text(app)
    for ch in "main":
        app.key(ch, settle=0.25)
    filtered_ok = "src/main.rs" in text(app)
    app.key("RET")
    app.wait(1.0)
    opened_ok = "src/main.rs" in row0(app) and "fn target_one" in text(app)
    app.key("C-g")  # clear any echo; we are now on the file
    ok = prompt_ok and filtered_ok and opened_ok
    record("U-B1", "C-x C-f,main,RET", ok,
           f"prompt={prompt_ok} filtered={filtered_ok} opened={opened_ok} title={row0(app)!r}")


def flow_b2(app):
    """U-B2 Ignored paths (.git/, target/) never appear in candidates."""
    app.key("C-x C-f")
    app.wait(0.8)
    t = text(app)
    git_leak = ".git/" in t
    target_leak = "target/" in t  # this repo has no target/ dir; walk ignores it anyway
    ok = not git_leak and not target_leak
    record("U-B2", "C-x C-f", ok, f".git/ leaked={git_leak} target/ leaked={target_leak}")
    app.key("C-g")


def flow_b3(app):
    """U-B3 (partial leg — no-match only): no match shows a clean empty state
    (count 0, no crash). The Backspace-edit and empty-query legs of U-B3 are
    not PTY-driven here (marked partial in the record, per the log's own
    convention for partial legs, e.g. the B5 populate-leg note)."""
    app.key("C-x C-f")
    app.wait(0.6)
    for ch in "zzzznomatch":
        app.key(ch, settle=0.2)
    t = text(app)
    empty_ok = ("0 of" in t) or ("0 candidates" in t) or (has(app, "0 of"))
    ok = empty_ok
    record("U-B3 (no-match leg only)", "C-x C-f,zzzznomatch", ok,
           f"clean-empty-state(count line present)={empty_ok} "
           f"[partial leg: Backspace-edit + empty-query legs not PTY-driven]")
    app.key("C-g")


def flow_b4(app):
    """U-B4 C-x C-b buffer list: *list-buffers*, *scratch* present."""
    app.key("C-x C-b")
    app.wait(0.8)
    ok = "*list-buffers*" in text(app) and "*scratch*" in text(app)
    record("U-B4", "C-x C-b", ok, f"list-buffers={'*list-buffers*' in text(app)} scratch-row={'*scratch*' in text(app)}")
    app.key("q")
    app.wait(0.6)


def flow_c1(app):
    """U-C1 Motion: C-d scrolls (window moves, point's screen row held)."""
    # Create a tall file (60 lines) so a half-page scroll is observable.
    tall_path = os.path.join(REPO, "src", "tall_sweep.rs")
    with open(tall_path, "w") as f:
        for i in range(1, 61):
            f.write(f"line {i}\n")
    try:
        app.key("C-c p i")
        app.wait(1.0)
        app.key("C-x C-f")
        app.wait(0.8)
        for ch in "tall_sweep":
            app.key(ch, settle=0.2)
        app.key("RET")
        app.wait(0.8)
        open_ok = "tall_sweep.rs" in row0(app)
        # Point starts at line 0 (top of buffer); cursor at screen row 1.
        top_before = app.row_text(1)
        cursor_row_before = app.screen.cursor.y
        # C-d: scroll down half a page (~11 rows with 22-row viewport).
        app.key("C-d")
        app.wait(0.6)
        top_after = app.row_text(1)
        cursor_row_after = app.screen.cursor.y
        # (a) Real scroll: the top visible line changed (non-vacuous).
        scrolled = top_before != top_after
        # (b) Point's screen row preserved: cursor stayed at the same row.
        row_held = (cursor_row_before == cursor_row_after)
        ok = open_ok and scrolled and row_held
        record("U-C1", "C-d", ok,
               f"opened={open_ok} scrolled={scrolled} "
               f"(top {top_before.strip()!r}->{top_after.strip()!r}) "
               f"point-row-held={row_held} "
               f"(cursor row {cursor_row_before}->{cursor_row_after})")
    finally:
        try:
            os.remove(tall_path)
        except FileNotFoundError:
            pass


def flow_e1(app):
    """U-E1 C-c p s s: first hits appear while/after the walk; count in title."""
    app.key("C-c p s s")
    for ch in "target":
        app.key(ch, settle=0.2)
    app.key("RET")
    app.wait_done()
    t = text(app)
    ok = ("matches in" in t) and ("fn target_one" in t or "target" in t)
    n = 0
    for r in range(app.rows):
        if "matches in" in app.row_text(r):
            n = 1
    record("U-E1", "C-c p s s,target,RET", ok,
           f"results-with-count={'matches in' in t} hits-present={'fn target_one' in t}")


def flow_e3(app):
    """U-E3 results navigation: n moves selection; RET jumps to the file."""
    # (continue from e1 if search is open, else start)
    if "Search:" not in row0(app):
        app.key("C-c p s s")
        for ch in "target":
            app.key(ch, settle=0.2)
        app.key("RET")
        app.wait_done()
    blues_before = app.blue_rows()
    app.key("n")
    app.wait(0.5)
    blues_after = app.blue_rows()
    moved = blues_before != blues_after
    app.key("RET")
    app.wait(1.0)
    jumped = "src/main.rs" in row0(app) or "src/lib.rs" in row0(app) or "README.md" in row0(app)
    ok = moved and jumped
    record("U-E3", "n,RET", ok, f"selection-moved={moved} (before={blues_before} after={blues_after}) jump-landed={jumped} title={row0(app)!r}")
    app.key("q")  # close any open view back toward buffer


def flow_f1(app):
    """U-F1 C-x g status: Staged/Unstaged sections + dirty counts in status line."""
    app.key("C-x g")
    app.wait(1.0)
    t = text(app)
    ok = "Staged" in t and "Unstaged" in t and "+1 ~1" in t  # repo: 1 staged, 1 unstaged
    dirty = "+1 ~1" in app.row_text(app.rows - 1)
    record("U-F1", "C-x g", ok, f"staged={'Staged' in t} unstaged={'Unstaged' in t} dirty-counts-in-status-line={dirty}")
    app.key("q")
    app.wait(0.6)


def flow_f2(app):
    """U-F2 stage/unstage: a keypress toggles the git index (git diff --cached
    agrees). The cursor starts on the staged row (src/lib.rs); `u` unstages it.
    The fixture is restored via git afterwards (test hygiene), independent of
    the app's post-op cursor position."""
    def cached_names():
        out = subprocess.run(["git", "-C", REPO, "diff", "--cached", "--name-only"],
                             capture_output=True, text=True).stdout.strip()
        return set(out.splitlines())
    before = cached_names()
    app.key("C-x g")
    app.wait(1.0)
    app.key("u")  # unstage the row under the cursor (the staged src/lib.rs)
    app.wait(0.8)
    after_u = cached_names()
    index_moved = before != after_u
    # Restore the fixture baseline for subsequent flows.
    subprocess.run(["git", "-C", REPO, "add", "src/lib.rs"],
                   capture_output=True, text=True)
    restored = cached_names() == before
    app.key("q")
    app.wait(0.5)
    ok = index_moved and restored
    record("U-F2", "C-x g,u", ok,
           f"git index toggled by key: {sorted(before)}->{sorted(after_u)} (fixture-restored={restored})")


def flow_h1(app):
    """U-H1 pending prefix: C-x shows pending in status; unmatchable key cancels."""
    app.key("C-x")
    app.wait(0.5)
    pending = any("[" in app.row_text(app.rows - 1) for _ in [0])
    app.key("z")  # unmatchable tail -> cancels
    app.wait(0.5)
    still_pending = any("C-x" in app.row_text(app.rows - 1) for _ in [0])
    ok = pending and not still_pending
    record("U-H1", "C-x,z", ok, f"pending-shown={pending} cleared-after-unmatchable={not still_pending}")


def flow_h2(app):
    """U-H2 C-g matrix: five states, each with a pyte-verified outcome.

    States driven here (single PTY): idle, picker open, pending prefix,
    isearch active. The "search running" state needs a large repo (the
    cancel must land while the walk is in flight) and is driven by
    flow_h2_search on its own App.
    """
    # 1) idle: C-g is a clean no-op — the whole frame (content, minibuffer,
    #    status) must be byte-identical afterwards: no view change, no
    #    pending, no spurious "cancel" echo.
    before = [app.row_text(r) for r in range(app.rows)]
    app.key("C-g")
    app.wait(0.5)
    after = [app.row_text(r) for r in range(app.rows)]
    same = before == after
    alive = any(row.strip() for row in after)
    record("U-H2 idle", "C-g (idle)", same and alive,
           f"frame-byte-identical-after-idle-C-g={same} alive={alive} "
           f"(no echo, no view change, no pending)")

    # 2) picker open: C-g closes the picker and echoes cancel.
    app.key("C-x C-f")
    app.wait(0.8)
    picker_open = "Find file:" in text(app)
    app.key("C-g")
    app.wait(0.6)
    picker_gone = "Find file:" not in text(app)
    echo = "cancel" in app.row_text(app.rows - 2)
    back = "*scratch*" in row0(app)
    record("U-H2 picker", "C-x C-f,C-g", picker_open and picker_gone and echo and back,
           f"picker-opened={picker_open} closed-by-C-g={picker_gone} "
           f"cancel-echo={echo} back-to-buffer={back}")

    # 3) pending prefix: C-x arms a prefix; C-g clears it (with echo).
    app.key("C-x")
    app.wait(0.5)
    pending_shown = "[C-x]" in app.row_text(app.rows - 1)
    app.key("C-g")
    app.wait(0.5)
    pending_cleared = "[C-x]" not in app.row_text(app.rows - 1)
    echo2 = "cancel" in app.row_text(app.rows - 2)
    record("U-H2 pending", "C-x,C-g", pending_shown and pending_cleared and echo2,
           f"pending-shown={pending_shown} cleared-by-C-g={pending_cleared} "
           f"cancel-echo={echo2}")

    # 4) isearch active: C-s arms isearch; C-g exits it (with echo), view
    #    stays the buffer view.
    app.key("C-s")
    app.wait(0.5)
    isearch_on = "I-search" in app.row_text(app.rows - 2)
    app.key("C-g")
    app.wait(0.5)
    isearch_off = "I-search" not in app.row_text(app.rows - 2)
    echo3 = "cancel" in app.row_text(app.rows - 2)
    view_kept = "*scratch*" in row0(app)
    record("U-H2 isearch", "C-s,C-g", isearch_on and isearch_off and echo3 and view_kept,
           f"isearch-armed={isearch_on} exited-by-C-g={isearch_off} "
           f"cancel-echo={echo3} view-kept={view_kept}")


def flow_h2_search(app):
    """U-H2 "search running" state: C-g cancels the in-flight search and the
    view stays open on the (partial) results. RET and C-g are written to the
    PTY in a single buffer so C-g is processed while the walk over the
    6000-file repo is still running."""
    app.key("C-c p s s")
    for ch in "target":
        app.key(ch, settle=0.15)
    app.feed(encode_key("RET") + encode_key("C-g"), settle=1.5)
    t = text(app)
    cancelled = "search cancelled" in app.row_text(app.rows - 2)
    view_stays = "Search:" in row0(app)
    alive = "(cancelled)" in row0(app)
    record("U-H2 search", "C-c p s s,target,RET+C-g(fast)", cancelled and view_stays and alive,
           f"in-flight-cancel-message={cancelled} view-stays-open={view_stays} "
           f"title={row0(app)[:40]!r}")
    app.key("q")
    app.wait(0.6)


def ensure_slow_repo():
    """Create SLOW_REPO once (6000 small files, git init for project
    detection). Files are inert filler; none contain the search query, so
    the only way the run can report 'search cancelled' is a real in-flight
    cancel."""
    if os.path.isfile(os.path.join(SLOW_REPO, "f_0000.txt")):
        return
    os.makedirs(os.path.join(SLOW_REPO, "data"), exist_ok=True)
    line = "lorem ipsum dolor sit amet\n"
    with open(os.path.join(SLOW_REPO, "filler.txt"), "w") as f:
        f.write(line * 10)
    for i in range(SLOW_FILES):
        with open(os.path.join(SLOW_REPO, "data", f"f_{i:04d}.txt"), "w") as f:
            f.write(line * 4)
    subprocess.run(["git", "-C", SLOW_REPO, "init", "-q"], check=True,
                   capture_output=True)


def flow_editable_keys(app):
    """Finding (plan 002): editable-buffer key interception must let
    multi-key sequences through while editing notes. A plain printable
    self-inserts; the C-x g tail must DISPATCH (open magit), not be typed
    into the buffer."""
    app.key("C-x n")
    app.wait(0.8)
    notes_open = ".redline-notes.md" in row0(app)
    for ch in "abc":
        app.key(ch, settle=0.3)
    # Self-insert: 'abc' is present as its own content row (row index may
    # shift if the conflict-marker row is present — match by content, not
    # index).
    typed = any(app.row_text(r).strip() == "abc" for r in range(app.rows - 2))
    # C-x g while editing: the prefix tail must reach the keymap engine and
    # open magit, not be typed into the notes buffer.
    app.key("C-x")
    app.wait(0.4)
    pending = "[C-x]" in app.row_text(app.rows - 1)
    app.key("g")
    app.wait(0.8)
    magit_opened = "Staged" in text(app)
    # Close magit (q on the magit view) back to the notes buffer.
    app.key("q")
    app.wait(0.8)
    back_to_notes = ".redline-notes.md" in row0(app)
    # The prefix tail must NOT have been typed: 'abc' stays exactly 'abc'
    # (a 'g' typed into the buffer would make the row 'abcg').
    intact = any(app.row_text(r).strip() == "abc" for r in range(app.rows - 2))
    no_tail_typed = not any("abcg" in app.row_text(r) for r in range(app.rows - 2))
    ok = notes_open and typed and pending and magit_opened and back_to_notes and intact and no_tail_typed
    record("editable-keys", "C-x n,abc,C-x g,q",
           ok,
           f"notes-open={notes_open} self-insert={typed} pending-armed={pending} "
           f"c-x-g-dispatched={magit_opened} back-to-notes={back_to_notes} "
           f"text-intact={intact} tail-not-typed={no_tail_typed}")


def flow_graft():
    """Finding (plan 002): graft cache cards (agent output, not project
    source) must not surface as file-finder candidates. The graft dir is
    created BEFORE the app starts so the startup file walk sees it; the
    walk must prune the whole subtree."""
    graft = os.path.join(REPO, "graft")
    os.makedirs(graft, exist_ok=True)
    with open(os.path.join(graft, "card.md"), "w") as f:
        f.write("# agent cache card\\n")
    try:
        app = App(REPO, rows=ROWS, cols=COLS)
        app.key("C-x C-f")
        app.wait(0.8)
        t = text(app)
        # No graft path may appear in the candidate list (empty query: the
        # full list is on screen). The query itself is empty here, so the
        # word 'graft' cannot appear from the prompt.
        no_graft_full = "graft" not in t
        # Fuzzy-filtering on the query 'graft' must yield a clean empty
        # state: the count line reads '0 of N' (a graft candidate would
        # make it '1 of N'). The query text itself echoes in the prompt
        # line, so the count line is the discriminator.
        for ch in "graft":
            app.key(ch, settle=0.2)
        t2 = text(app)
        empty_state = "0 of" in t2
        app.key("C-g")
        app.wait(0.5)
        app.kill()
    finally:
        shutil.rmtree(graft, ignore_errors=True)
    record("graft-pollution", "C-x C-f,graft",
           no_graft_full and empty_state,
           f"graft-in-full-list={not no_graft_full} "
           f"empty-state-on-query={empty_state} (count line: "
           f"{[l for l in t2.splitlines() if 'of' in l and l.strip().startswith(('0','1'))][:1]})")


def flow_b5(app):
    """U-B5 populate leg: recents populate as files are visited (C-c p e shows them)."""
    # Open src/main.rs to add it to recents.
    app.key("C-x C-f")
    app.wait(0.8)
    for ch in "main":
        app.key(ch, settle=0.2)
    app.key("RET")
    app.wait(0.8)
    # Now open the recents picker: src/main.rs must appear.
    app.key("C-c p e")
    app.wait(0.8)
    t = text(app)
    prompt_ok = "Recent file:" in t
    main_listed = "src/main.rs" in t
    app.key("C-g")
    app.wait(0.4)
    ok = prompt_ok and main_listed
    record("U-B5", "C-x C-f,main,RET; C-c p e", ok,
           f"recents-prompt={prompt_ok} visited-file-listed={main_listed}")


def flow_b6(app):
    """U-B6 deleted-file edge: a recents entry for a file deleted on disk is
    skipped (not listed); the picker stays usable."""
    # Open two files so recents has at least two entries.
    app.key("C-x C-f")
    app.wait(0.8)
    for ch in "main":
        app.key(ch, settle=0.2)
    app.key("RET")
    app.wait(0.8)
    app.key("C-x C-f")
    app.wait(0.8)
    for ch in "lib":
        app.key(ch, settle=0.2)
    app.key("RET")
    app.wait(0.8)
    # Now delete src/lib.rs on disk and check the recents picker.
    lib_path = os.path.join(REPO, "src", "lib.rs")
    lib_backup = os.path.join(REPO, "src", "lib.rs.bak")
    os.rename(lib_path, lib_backup)
    try:
        app.key("C-c p e")
        app.wait(0.8)
        t = text(app)
        prompt_ok = "Recent file:" in t
        main_listed = "src/main.rs" in t
        # The title bar (row 0) still shows the current buffer name
        # (src/lib.rs); check the picker content rows only.
        lib_in_content = any("src/lib.rs" in app.row_text(r) for r in range(1, app.rows - 1))
        lib_absent = not lib_in_content
        app.key("C-g")
        app.wait(0.4)
    finally:
        os.rename(lib_backup, lib_path)
    ok = prompt_ok and main_listed and lib_absent
    record("U-B6", "C-c p e (lib.rs deleted)", ok,
           f"picker-opens={prompt_ok} surviving-recent-listed={main_listed} "
           f"deleted-file-absent={lib_absent}")


def flow_c6(app):
    """U-C6 M-< / M-> / G: top/bottom scroll and G→M-< round trip."""
    # Create a 50-line file so the content exceeds the 22-row viewport.
    long_path = os.path.join(REPO, "src", "long_sweep.rs")
    with open(long_path, "w") as f:
        for i in range(1, 51):
            f.write(f"line {i}\n")
    try:
        # Re-walk so the file walk picks up the new file.
        app.key("C-c p i")
        app.wait(1.0)
        app.key("C-x C-f")
        app.wait(0.8)
        for ch in "long_sweep":
            app.key(ch, settle=0.2)
        app.key("RET")
        app.wait(0.8)
        open_ok = "long_sweep.rs" in row0(app)
        # Record the first content row at the top.
        top_row1 = app.row_text(1)
        # M-> scrolls to bottom: the first content row must change.
        app.key("M->")
        app.wait(0.6)
        bottom_row1 = app.row_text(1)
        m_gt_changed = bottom_row1 != top_row1
        # Bottom anchor: the last content row reads the final line (line 50),
        # not just "the first row changed". Content occupies rows 1..app.rows-4
        # (title on row 0; the "↑" scroll-indicator row is app.rows-3; status
        # line + minibuffer on the last two rows).
        last_content_row = app.rows - 4
        m_gt_bottom_anchor = app.row_text(last_content_row).strip() == "line 50"
        # M-< scrolls back to top: the first content row must match the original.
        app.key("M-<")
        app.wait(0.6)
        back_top_row1 = app.row_text(1)
        m_lt_roundtrip = back_top_row1 == top_row1
        # G scrolls to bottom again (same effect as M->).
        app.key("G")
        app.wait(0.6)
        g_row1 = app.row_text(1)
        g_changed = g_row1 != top_row1
        # Bottom anchor after G: the last content row still reads line 50.
        g_bottom_anchor = app.row_text(last_content_row).strip() == "line 50"
        # M-< round trip after G.
        app.key("M-<")
        app.wait(0.6)
        g_roundtrip = app.row_text(1) == top_row1
        ok = (open_ok and m_gt_changed and m_lt_roundtrip and g_changed
              and g_roundtrip and m_gt_bottom_anchor and g_bottom_anchor)
        record("U-C6", "M->,M-<,G,M-<", ok,
               f"opened={open_ok} M-> scrolled={m_gt_changed} "
               f"M-> bottom-anchor(line50)={m_gt_bottom_anchor} "
               f"M-< roundtrip={m_lt_roundtrip} G scrolled={g_changed} "
               f"G bottom-anchor(line50)={g_bottom_anchor} "
               f"G→M-< roundtrip={g_roundtrip}")
    finally:
        try:
            os.remove(long_path)
        except FileNotFoundError:
            pass


def flow_e2(app):
    """U-E2 cancel paths mid-search: ESC and q close the results view.
    (The C-g leg — cancel without closing — is covered by H2-search.)"""
    # ESC leg
    app.key("C-c p s s")
    for ch in "target":
        app.key(ch, settle=0.15)
    app.key("RET")
    app.wait_done()
    search_open = "Search:" in row0(app)
    app.key("ESC")
    app.wait(0.6)
    view_closed = "Search:" not in row0(app)
    alive = any(row.strip() for row in [app.row_text(r) for r in range(app.rows)])
    esc_ok = search_open and view_closed and alive
    record("U-E2 ESC", "C-c p s s,target,RET;ESC", esc_ok,
           f"search-was-open={search_open} view-closed-by-ESC={view_closed} alive={alive} "
           f"title={row0(app)!r}")
    # q leg
    app.key("C-c p s s")
    for ch in "target":
        app.key(ch, settle=0.15)
    app.key("RET")
    app.wait_done()
    search_open2 = "Search:" in row0(app)
    app.key("q")
    app.wait(0.6)
    view_closed2 = "Search:" not in row0(app)
    alive2 = any(row.strip() for row in [app.row_text(r) for r in range(app.rows)])
    q_ok = search_open2 and view_closed2 and alive2
    record("U-E2 q", "C-c p s s,target,RET;q", q_ok,
           f"search-was-open={search_open2} view-closed-by-q={view_closed2} alive={alive2} "
           f"title={row0(app)!r}")


def flow_g3(app):
    """U-G3 conflict path: a locally-owned buffer shows the 'changed on disk'
    marker ONLY after a real external disk edit; the type-then-idle leg proves
    the Access-event self-sustain is gone (no false marker after typing + idle).
    Force-reload supersedes the marker (cleared, disk content shown, local edit
    dropped)."""
    notes_path = os.path.join(REPO, ".redline-notes.md")
    try:
        app.key("C-x n")
        app.wait(0.8)
        notes_open = ".redline-notes.md" in row0(app)
        # One keystroke: the notes buffer becomes locally-owned.
        app.key("x", settle=0.3)
        # Type-then-idle leg: pump ~3 s (well past the 500 ms debounce
        # cadence). The old Access self-sustain would have raised a false
        # marker within ~0.5 s. After the fix, no marker appears.
        pump(app, 3.0)
        no_false_marker = "changed on disk" not in text(app)
        # Typed-char precondition: the local edit (the typed 'x') must still
        # be present in the notes buffer BEFORE the reload supersedes it —
        # proves the edit actually landed and that the idle pump did not
        # clobber it with a false reload.
        typed_char_present = any(
            app.row_text(r).strip() == "x" for r in range(1, app.rows - 2)
        )
        # External disk edit: append a real line to the notes file.
        with open(notes_path, "a") as f:
            f.write("\nexternal_change_marker\n")
        # Wait for the watcher to pick up the real external edit.
        marker_landed = wait_for(app, lambda: "changed on disk" in text(app), 4.0)
        # Force-reload via the reload-buffer command (the command `g` runs).
        app.key("M-x")
        app.wait(0.8)
        for ch in "reload-buffer":
            app.key(ch, settle=0.2)
        cl = count_line(app)
        only_match = cl is not None and cl[0] == "1"
        app.key("RET")
        app.wait(1.0)
        t2 = text(app)
        marker_cleared = "changed on disk" not in t2
        reloaded = "external_change_marker" in t2
        local_edit_superseded = not any(
            app.row_text(r).strip() == "x" for r in range(1, app.rows - 2)
        )
        ok = (notes_open and no_false_marker and typed_char_present and marker_landed
              and only_match and marker_cleared and reloaded and local_edit_superseded)
        record("U-G3", "C-x n,x;idle 3s;disk-append;M-x reload-buffer,RET", ok,
               f"notes-open={notes_open} no-false-marker-after-idle={no_false_marker} "
               f"typed-char-present-before-reload={typed_char_present} "
               f"marker-from-real-append={marker_landed} command-unique={only_match} "
               f"marker-cleared-by-reload={marker_cleared} "
               f"disk-edit-reloaded={reloaded} "
               f"local-edit-superseded={local_edit_superseded}")
    finally:
        try:
            os.remove(notes_path)
        except FileNotFoundError:
            pass


def flow_h3(app):
    """U-H3 unknown key echoes in the minibuffer."""
    app.key("z")
    app.wait(0.5)
    ok = "unbound key: z" in text(app)
    record("U-H3", "z", ok, f"echo-present={'unbound key: z' in text(app)}")


def flow_qquit(app):
    """Finding 5 (plan 001): bare `q` on the root buffer view must NOT quit."""
    app.key("q")
    app.wait(0.6)
    alive = "scratch" in text(app) or "ready" in text(app)
    ok = alive
    record("q-quit", "q (root buffer)", ok, f"still-alive-after-bare-q={alive} (title={row0(app)!r})")


def flow_notes(app):
    """Finding (plan 002): opening notes must NOT flag a false 'changed on disk'."""
    app.key("C-x n")
    app.wait(0.8)
    t = text(app)
    conflict = "changed on disk" in t
    ok = not conflict
    record("notes", "C-x n", ok, f"false-conflict-marker={conflict} (title={row0(app)!r})")
    app.key("C-g")
    app.wait(0.4)


def flow_palette(app):
    """Finding (plan 002): the palette must not show demo/placeholder commands."""
    app.key("M-x")
    app.wait(0.8)
    t = text(app)
    demo_leak = any(x in t for x in ("demo-message-1", "demo-message-2", "insert-demo-text"))
    ok = not demo_leak and "open-palette" in t or (not demo_leak and "quit" in t)
    record("palette", "M-x", ok, f"demo-commands-leaked={demo_leak}")
    app.key("C-g")
    app.wait(0.4)


# ── round-3 driven flows (plan 002 issue 03 must-fixes) ────────────────────
#
# These drive the flows the final review flagged as falsely (or without a
# reason) marked NOT_APPLICABLE. Every disk-write-then-watch leg pumps the PTY
# on a real wall-clock deadline (pump/wait_for), because App.wait's quiet-exit
# returns after ~0.2 s of silence — before the watcher's 400 ms debounce.


def flow_j3(app):
    """U-J3 80x24 everywhere: the status line renders on exactly one row (the
    bottom row), with no wrap artifact, across two views. The whole sweep runs
    at 80x24 (ROWS, COLS) with per-flow frame/status assertions; this is the
    dedicated no-wrap check on top of that."""
    proj = "redline_pyte_repo"
    buf_rows = rows_containing(app, proj)  # buffer view (scratch)
    buf_ok = buf_rows == [app.rows - 1]
    app.key("C-x g")
    app.wait(1.0)
    mag_rows = rows_containing(app, proj)  # magit status view
    mag_ok = mag_rows == [app.rows - 1]
    no_spill = proj not in app.row_text(app.rows - 2)
    ok = buf_ok and mag_ok and no_spill
    record("U-J3", "(80x24 buffer+magit)", ok,
           f"status-line-on-exactly-one-row: buffer={buf_rows} magit={mag_rows} "
           f"(no spill onto row {app.rows - 2}={no_spill}) at 80x24")


def flow_f3(app):
    """U-F3 magit fold/unfold + RET visit: file sections start folded (hunk rows
    hidden); TAB reveals them, TAB again hides them, and RET on the file row
    opens that file in the buffer view."""
    app.key("C-x g")
    app.wait(1.0)

    def has_hunk():
        # Unfolded src/lib.rs shows its hunk header (@@ ...) and added marker
        # line; folded, neither is present.
        t = text(app)
        return "@@" in t and "staged_change_marker" in t

    folded = not has_hunk()
    app.key("TAB")
    app.wait(0.6)
    unfolded = has_hunk()
    app.key("TAB")
    app.wait(0.6)
    refolded = not has_hunk()
    app.key("TAB")
    app.wait(0.6)  # unfold again so RET has a file row under the cursor
    app.key("RET")
    app.wait(1.2)
    visited = "src/lib.rs" in row0(app) and "staged_change_marker" in text(app)
    ok = folded and unfolded and refolded and visited
    record("U-F3", "C-x g,TAB,TAB,TAB,RET", ok,
           f"hunk-folded-at-start={folded} TAB-reveals={unfolded} "
           f"TAB-again-hides={refolded} RET-opens-file={visited} title={row0(app)!r}")


def flow_f4(app, path, query):
    """U-F4 live status on a FILE buffer: an external disk edit is picked up by
    the watcher and the file view repaints within the debounce, with no
    keypress. A plain (non-locally-owned) file buffer auto-reloads rather than
    raising the 'changed on disk' marker (that marker is the locally-owned
    path, driven by flow_g3 on the notes buffer); `g` force-reloads either way."""
    name = os.path.basename(path)
    app.key("C-x C-f")
    app.wait(0.8)
    for ch in query:
        app.key(ch, settle=0.2)
    app.key("RET")
    app.wait(0.8)
    open_ok = name in row0(app)
    with open(path, "a") as f:
        f.write("F4_WATCHER_APPEND\n")
    reloaded = wait_for(app, lambda: "F4_WATCHER_APPEND" in text(app), 4.0)
    no_marker = "changed on disk" not in text(app)
    app.key("g")
    app.wait(0.8)
    g_reload = "reloaded" in app.row_text(app.rows - 2)
    ok = open_ok and reloaded and no_marker and g_reload
    record("U-F4", "C-x C-f,<f>,RET;disk-append;g", ok,
           f"file-view-open={open_ok} watcher-auto-reloaded-new-line={reloaded} "
           f"(no marker on non-locally-owned buffer={no_marker}) "
           f"g-reload-confirmed={g_reload}")


def _ensure_commit_repo():
    """Create a fresh REPO5 with a deterministic commit-flow baseline: one base
    commit, a STAGED working-tree change on src/lib.rs, and an UNSTAGED
    working-tree change on README.md. Wiped each run so history never grows."""
    shutil.rmtree(REPO5, ignore_errors=True)
    os.makedirs(os.path.join(REPO5, "src"), exist_ok=True)
    with open(os.path.join(REPO5, "src", "lib.rs"), "w") as f:
        f.write("pub fn target_lib() {}\npub fn other_lib() {}\n")
    with open(os.path.join(REPO5, "README.md"), "w") as f:
        f.write("# commit fixture\n")

    def git(*a):
        subprocess.run(["git", "-C", REPO5, *a], check=True, capture_output=True)

    git("init", "-q", "-b", "main")
    git("config", "user.name", "T")
    git("config", "user.email", "t@e.com")
    git("add", "-A")
    git("commit", "-q", "-m", "base")
    with open(os.path.join(REPO5, "src", "lib.rs"), "a") as f:
        f.write("staged_change_marker\n")
    git("add", "src/lib.rs")
    with open(os.path.join(REPO5, "README.md"), "a") as f:
        f.write("unstaged_change_marker\n")


def flow_f5(app, repo):
    """U-F5 commit flow: stage the unstaged file, open the commit editor (`c`),
    type a message, commit (`C-c C-c`) — verified by `git log` in the fixture
    and the status buffer going clean. (The `C-c C-k` abort leg is covered by
    the store's commit_editor_abort; this drives the commit leg.) Runs on a
    dedicated repo (`repo`) so REPO's baseline is never committed."""
    def git(*a):
        return subprocess.run(["git", "-C", repo, *a],
                              capture_output=True, text=True).stdout

    app.key("C-x g")
    app.wait(1.0)
    # Cursor starts on the staged file; n -> "Unstaged changes" group,
    # n -> the README.md file row, then stage it (both files now staged).
    app.key("n")
    app.wait(0.3)
    app.key("n")
    app.wait(0.3)
    app.key("s")
    app.wait(0.8)
    staged_both = set(git("diff", "--cached", "--name-only").splitlines()) \
        >= {"src/lib.rs", "README.md"}
    app.key("c")
    app.wait(0.8)
    editor_open = "commit" in row0(app) and "Staged changes" in text(app)
    for ch in "sweepf5marker":
        app.key(ch, settle=0.15)
    app.key("C-c C-c")
    app.wait(1.6)
    log_top = git("log", "--oneline", "-1").strip()
    committed = "sweepf5marker" in log_top
    status_clean = git("status", "--porcelain").strip() == ""
    magit_clean = "M src/lib.rs" not in text(app) and "M README.md" not in text(app)
    ok = staged_both and editor_open and committed and status_clean and magit_clean
    record("U-F5", "C-x g,n,n,s,c,<msg>,C-c C-c", ok,
           f"both-staged={staged_both} editor-open={editor_open} "
           f"git-log-has-message={committed} (top={log_top!r}) status-clean={status_clean} "
           f"magit-clean={magit_clean}")


def flow_f6(app):
    """U-F6 log: `l` lists commits, an in-page key moves the selection (exactly
    one cursor row), and RET opens the selected commit's diff."""
    app.key("C-x g")
    app.wait(1.0)
    app.key("l")
    app.wait(1.0)
    rows = text(app)
    rows_render = "commit 5" in rows and "commit 4" in rows and "init" in rows
    blue_before = app.blue_rows()
    app.key("down")
    app.wait(0.5)
    blue_after = app.blue_rows()
    cursor_moved = (len(blue_before) == 1 and len(blue_after) == 1
                    and blue_before != blue_after)
    app.key("RET")
    app.wait(1.0)
    diff_open = ("commit" in row0(app) and "read-only" in text(app)
                 and "commit-diff" in app.row_text(app.rows - 1))
    ok = rows_render and cursor_moved and diff_open
    record("U-F6", "C-x g,l,down,RET", ok,
           f"log-rows-render={rows_render} cursor-moved={cursor_moved} "
           f"(blue {blue_before}->{blue_after}) RET-opens-commit-diff={diff_open}")
    app.key("q")
    app.wait(0.5)


def flow_f7(app):
    """U-F7 blame: `b` blames the current buffer's file — per-line rows with a
    commit-hash/author/age prefix and exactly one selected (cursor) row. Note
    find-file sets the *current buffer* (not the top view), so the current-file
    check reads the status line's which-function (a symbol from src/main.rs),
    not the view title."""
    app.key("C-x C-f")
    app.wait(0.8)
    for ch in "main":
        app.key(ch, settle=0.2)
    app.key("RET")
    app.wait(1.0)
    file_current = "(target_one)" in app.row_text(app.rows - 1)
    app.key("C-x g")
    app.wait(0.8)
    app.key("b")
    app.wait(1.0)
    rows = text(app)
    title_ok = "blame: src/main.rs" in row0(app)
    per_line = rows.count("Test") >= 5  # one author token per blamed line
    blues = app.blue_rows()
    cursor = len(blues) == 1
    ok = file_current and title_ok and per_line and cursor
    record("U-F7", "C-x C-f,main,RET;C-x g,b", ok,
           f"file-current(which-fn)={file_current} blame-title={title_ok} "
           f"per-line-rows(author-token>=5)={per_line} exactly-one-cursor={cursor} "
           f"(blue={blues})")
    app.key("q")
    app.wait(0.5)


def flow_f8(app):
    """U-F8 branch/stash: `y` opens the branch picker (lists the local branch);
    `z` with no stashes shows the empty state ('no stashes'), not a panic."""
    app.key("C-x g")
    app.wait(1.0)
    app.key("y")
    app.wait(0.8)
    t = text(app)
    branch_listed = "Branch:" in t and "*main" in t
    app.key("C-g")
    app.wait(0.5)
    app.key("z")
    app.wait(0.8)
    empty_state = "no stashes" in app.row_text(app.rows - 2)
    ok = branch_listed and empty_state
    record("U-F8", "C-x g,y,C-g,z", ok,
           f"branch-picker-lists-main={branch_listed} stash-empty-state={empty_state}")


def flow_g1(app, path):
    """U-G1 live-edit scroll preservation: an external append at the bottom of a
    viewed file repaints the view (watcher) and preserves the scroll anchor
    (the top line stays put, since that line still exists). The shrink-clamp
    half of G1 (file shrinks below the anchor) is unit-tested in reload_anchor."""
    top_before = app.row_text(1)
    with open(path, "a") as f:
        f.write("G1_BOTTOM_APPEND\n")
    shown = wait_for(app, lambda: "G1_BOTTOM_APPEND" in text(app), 4.0)
    top_after = app.row_text(1)
    scroll_preserved = top_before == top_after and top_before.strip() != ""
    ok = shown and scroll_preserved
    record("U-G1", "disk-append (bottom)", ok,
           f"view-repaints-on-external-edit={shown} scroll-anchor-preserved={scroll_preserved} "
           f"(top line {top_before.strip()!r} unchanged; shrink-clamp unit-tested)")


def flow_g2(app, path):
    """U-G2 agent churn: 10 rapid disk writes — the view shows the FINAL content
    and the app stays responsive (a keypress repaints), i.e. no freeze. The
    'bounded repaints / no event storm' property is the debounce coalescing,
    unit-tested (coalesces_rapid_writes_to_one_event)."""
    for i in range(10):
        with open(path, "a") as f:
            f.write(f"G2_CHURN_{i}\n")
    final_shown = wait_for(app, lambda: "G2_CHURN_9" in text(app), 5.0)
    before = [app.row_text(r) for r in range(app.rows)]
    app.key("C-d")
    app.wait(0.6)
    after = [app.row_text(r) for r in range(app.rows)]
    responsive = before != after
    ok = final_shown and responsive
    record("U-G2", "10 rapid writes;C-d", ok,
           f"final-content-shown={final_shown} app-still-responsive(no-freeze)={responsive} "
           f"(coalescing unit-tested)")


def flow_g6(app, path):
    """U-G6 suspend: `M-x toggle-watcher` OFF -> a disk edit produces NO reload
    or marker; ON again -> the next edit lands. Drives the toggle command and
    the suspend/resume watcher behavior end to end."""

    def toggle():
        app.key("M-x")
        app.wait(0.6)
        for ch in "toggle-watcher":
            app.key(ch, settle=0.12)
        app.key("RET")
        app.wait(0.6)

    toggle()  # OFF
    suspended_msg = "file watching suspended" in app.row_text(app.rows - 2)
    with open(path, "a") as f:
        f.write("G6_SUSPENDED_LINE\n")
    pump(app, 2.0)  # well past the 400 ms debounce
    no_reload_suspended = "G6_SUSPENDED_LINE" not in text(app)
    toggle()  # ON
    resumed_msg = "file watching resumed" in app.row_text(app.rows - 2)
    with open(path, "a") as f:
        f.write("G6_RESUMED_LINE\n")
    landed = wait_for(app, lambda: "G6_RESUMED_LINE" in text(app), 4.0)
    ok = suspended_msg and no_reload_suspended and resumed_msg and landed
    record("U-G6", "M-x toggle-watcher OFF;disk;ON;disk", ok,
           f"suspend-message={suspended_msg} no-reload-while-suspended={no_reload_suspended} "
           f"resume-message={resumed_msg} event-lands-after-resume={landed}")


def _ensure_repo2():
    """Create REPO2 (a small, committed git repo distinct from REPO) once."""
    if os.path.isdir(os.path.join(REPO2, ".git")):
        return
    os.makedirs(os.path.join(REPO2, "src"), exist_ok=True)
    with open(os.path.join(REPO2, "Cargo.toml"), "w") as f:
        f.write("[package]\n")
    with open(os.path.join(REPO2, "src", "b.py"), "w") as f:
        f.write("def hello():\n    pass\n")
    subprocess.run(["git", "-C", REPO2, "init", "-q"], check=True, capture_output=True)
    subprocess.run(["git", "-C", REPO2, "config", "user.name", "T"], capture_output=True)
    subprocess.run(["git", "-C", REPO2, "config", "user.email", "t@e.com"], capture_output=True)
    subprocess.run(["git", "-C", REPO2, "add", "-A"], capture_output=True)
    subprocess.run(["git", "-C", REPO2, "commit", "-q", "-m", "init2"], capture_output=True)


def flow_g5():
    """U-G5 project switch (`C-c p p`): register a SECOND project in an isolated
    cache, drive the switch, and verify the watcher/index land on the new
    project (status line + find-file picker). The harness already runs
    sequential apps, so a real two-project switch is drivable here."""
    import json
    _ensure_repo2()
    os.makedirs(os.path.join(G5_CACHE, "redline"), exist_ok=True)
    reg = {"projects": [
        {"root": REPO, "name": "redline_pyte_repo"},
        {"root": REPO2, "name": "redline_pyte_repo2"},
    ]}
    with open(os.path.join(G5_CACHE, "redline", "projects.json"), "w") as f:
        json.dump(reg, f)
    prev = os.environ.get("XDG_CACHE_HOME")
    os.environ["XDG_CACHE_HOME"] = G5_CACHE
    try:
        app = App(REPO, rows=ROWS, cols=COLS)
        app.key("C-c p p")
        app.wait(0.8)
        t = text(app)
        picker_ok = "Switch project:" in t and "redline_pyte_repo2" in t
        app.key("RET")
        switched = wait_for(
            app, lambda: "redline_pyte_repo2" in app.row_text(app.rows - 1), 8.0
        )
        app.wait(1.0)
        landed = "Find file:" in text(app) and "src/b.py" in text(app)
        msg = "project: redline_pyte_repo2" in app.row_text(app.rows - 2)
        ok = picker_ok and switched and landed
        app.kill()
    finally:
        if prev is None:
            os.environ.pop("XDG_CACHE_HOME", None)
        else:
            os.environ["XDG_CACHE_HOME"] = prev
    record("U-G5", "C-c p p,RET", ok,
           f"switch-picker-lists-2nd-project={picker_ok} status-line-switched={switched} "
           f"lands-in-new-project-find-file={landed} project-message={msg}")


# ── plan-003-03 new windowing/scroll sweep legs ────────────────────────────────
# Thin sweep legs that point at the deep drives in tools/drive_windowing_panes.py
# (which build their own disposable git fixtures). Each builds its own throwaway
# repo (WIN_DIFF_REPO / WIN_NOTES_REPO) so REPO's baseline is never touched.

def _ensure_win_diff_repo():
    """A throwaway repo with a base commit and one tall commit (big.txt -> 60
    lines). Wiped each run so history never grows. Feeds the commit-diff and
    blame sweep legs (deep drives: tools/drive_windowing_panes.py:drive_commit_diff
    / :drive_blame)."""
    shutil.rmtree(WIN_DIFF_REPO, ignore_errors=True)
    os.makedirs(WIN_DIFF_REPO, exist_ok=True)

    def git(*a):
        subprocess.run(["git", "-C", WIN_DIFF_REPO, *a], check=True, capture_output=True)

    git("init", "-q", "-b", "main")
    git("config", "user.name", "T")
    git("config", "user.email", "t@e.com")
    with open(os.path.join(WIN_DIFF_REPO, "big.txt"), "w") as f:
        f.write("l1\n")
    git("add", "-A")
    git("commit", "-q", "-m", "base")
    with open(os.path.join(WIN_DIFF_REPO, "big.txt"), "w") as f:
        for i in range(1, 60):
            f.write(f"line {i}\n")
        f.write("BOTTOM_SENTINEL\n")
    git("add", "-A")
    git("commit", "-q", "-m", "tall-lines")


def _ensure_win_notes_repo():
    """A throwaway repo whose notes file is pre-seeded with 30 lines (no
    trailing newline) so opening notes loads a buffer taller than the viewport.
    Wiped each run. Feeds the notes-scroll sweep leg (deep drive:
    tools/drive_windowing_panes.py:drive_notes)."""
    shutil.rmtree(WIN_NOTES_REPO, ignore_errors=True)
    os.makedirs(WIN_NOTES_REPO, exist_ok=True)

    def git(*a):
        subprocess.run(["git", "-C", WIN_NOTES_REPO, *a], check=True, capture_output=True)

    git("init", "-q", "-b", "main")
    git("config", "user.name", "T")
    git("config", "user.email", "t@e.com")
    with open(os.path.join(WIN_NOTES_REPO, "README.md"), "w") as f:
        f.write("# notes fixture\n")
    git("add", "-A")
    git("commit", "-q", "-m", "init")
    with open(os.path.join(WIN_NOTES_REPO, ".redline-notes.md"), "w") as f:
        for i in range(1, 30):
            f.write(f"note line {i}\n")
        f.write("note line 30")  # last line, no trailing newline


def flow_commit_diff_scroll():
    """NEW (plan-003-03) commit-diff scroll leg (thin): the commit-diff pane
    scrolls past the 80x24 viewport — M-> lands on the last page (the sentinel
    row shows), M-< round-trips to the top (the sentinel hides). The deep drive
    (full-last-page semantic + C-n/C-p stepping) is
    tools/drive_windowing_panes.py:drive_commit_diff."""
    _ensure_win_diff_repo()
    app = App(WIN_DIFF_REPO, rows=ROWS, cols=COLS)
    try:
        app.key("C-x g")
        app.wait(0.8)
        app.key("l")      # log
        app.wait(0.8)
        app.key("RET")    # open the newest (tall) commit's diff
        app.wait(1.2)
        top_hidden = "BOTTOM_SENTINEL" not in app.screen_text()
        app.key("M->")
        app.wait(0.8)
        bottom_shown = "BOTTOM_SENTINEL" in app.screen_text()
        app.key("M-<")
        app.wait(0.8)
        roundtrip = "BOTTOM_SENTINEL" not in app.screen_text()
        ok = top_hidden and bottom_shown and roundtrip
        record("U-CDS", "C-x g,l,RET;M->;M-<", ok,
               f"top-hidden={top_hidden} M->-bottom-shown={bottom_shown} "
               f"M-<-roundtrip={roundtrip} (deep drive: drive_windowing_panes.py)")
    finally:
        app.kill()


def flow_blame_windowing():
    """NEW (plan-003-03) blame windowing leg (thin): the cursor-following blame
    window keeps the (single) cursor row in view across C-n moves past the
    bottom and M-> to the last line. The deep drive is
    tools/drive_windowing_panes.py:drive_blame."""
    _ensure_win_diff_repo()
    app = App(WIN_DIFF_REPO, rows=ROWS, cols=COLS)
    try:
        app.key("C-c p i")   # re-walk so big.txt is listed
        app.wait(1.0)
        app.key("C-x C-f")
        app.wait(0.8)
        for ch in "big":
            app.key(ch, settle=0.25)
        app.key("RET")
        app.wait(1.0)
        app.key("C-x g")
        app.key("b")         # blame the current buffer's file
        app.wait(1.2)
        in_window = []
        for _ in range(20):
            b = app.blue_rows()
            in_window.append(len(b) == 1 and 1 <= b[0] <= app.rows - 3)
            app.key("C-n")
        app.key("M->")
        app.wait(0.6)
        b = app.blue_rows()
        last_ok = len(b) == 1 and 1 <= b[0] <= app.rows - 3
        ok = all(in_window) and last_ok
        record("U-BLW", "C-c p i;C-x C-f,big,RET;C-x g,b;C-n x20;M->", ok,
               f"cursor-in-view {sum(in_window)}/{len(in_window)}, M->-last-line "
               f"{last_ok} (deep drive: drive_windowing_panes.py)")
    finally:
        app.kill()


def flow_notes_scroll():
    """NEW (plan-003-03) notes-scroll leg (thin): a notes buffer taller than the
    viewport keeps the active (insertion) row in view — typing near the bottom
    scrolls the last line into view. The deep drive is
    tools/drive_windowing_panes.py:drive_notes."""
    _ensure_win_notes_repo()
    app = App(WIN_NOTES_REPO, rows=ROWS, cols=COLS)
    try:
        app.key("C-x n")    # open notes (loads the 30-line file)
        app.wait(1.2)
        before = app.screen_text()
        bottom_hidden = "note line 30" not in before
        app.key("Z")        # type one char at the end (the insertion row)
        app.wait(0.8)
        after = app.screen_text()
        bottom_shown = "note line 30" in after
        edited = "note line 30Z" in after
        ok = bottom_hidden and bottom_shown and edited
        record("U-NSL", "C-x n;Z", ok,
               f"bottom-hidden-before={bottom_hidden} bottom-shown-after={bottom_shown} "
               f"self-insert={edited} (deep drive: drive_windowing_panes.py)")
    finally:
        app.kill()


def flow_banner_hint():
    """NEW (plan-003-03) banner-hint check, driven per kind.

    Reachable case (the bug report's actual case): on the notes buffer (the
    only key-editable, locally-owned buffer) an external disk append raises the
    "changed on disk" banner, and the hint must read "M-x reload-buffer" (plain
    `g` self-inserts by design on an editable buffer) — and must NOT say
    "press g to reload".

    The plain-file "g" hint is NOT PTY-banner-reachable: plain file buffers are
    read-only (editable=false) and never locally-owned, so a plain file
    auto-reloads and never renders the banner (flow_f4's no-marker leg asserts
    exactly this). That hint text is unit-covered via the
    changed_on_disk_hint(false) helper, not PTY-driven.
    """
    notes_path = os.path.join(REPO, ".redline-notes.md")
    try:
        app = App(REPO, rows=ROWS, cols=COLS)
        try:
            app.key("C-x n")
            app.wait(0.8)
            notes_open = ".redline-notes.md" in row0(app)
            # One keystroke: the notes buffer becomes locally-owned.
            app.key("x", settle=0.3)
            # External disk edit: append a real line to the notes file.
            with open(notes_path, "a") as f:
                f.write("\nbanner_hint_marker\n")
            # Wait for the watcher to raise the "changed on disk" banner.
            banner_landed = wait_for(app, lambda: "changed on disk" in text(app), 4.0)
            # The banner is a single row; capture it to assert the per-kind hint.
            hint = ""
            for r in range(app.rows):
                rt = app.row_text(r)
                if "changed on disk" in rt:
                    hint = rt
                    break
            editable_hint = "M-x reload-buffer" in hint
            no_plain_g = "press g to reload" not in hint
            ok = notes_open and banner_landed and editable_hint and no_plain_g
            record("U-BHN", "C-x n,x;disk-append", ok,
                   f"notes-open={notes_open} banner-landed={banner_landed} "
                   f"hint-says-M-x-reload-buffer={editable_hint} "
                   f"no-press-g-to-reload={no_plain_g} "
                   f"[plain-file 'g' hint: unit-covered via changed_on_disk_hint(false), "
                   f"not PTY-banner-reachable — plain files auto-reload (flow_f4 no-marker)] "
                   f"(hint={hint!r})")
        finally:
            app.kill()
    finally:
        try:
            os.remove(notes_path)
        except FileNotFoundError:
            pass


NOT_APPLICABLE = [
    "U-A2 (non-git folder) — needs a non-git repo fixture",
    "U-A3/A3b (10k-file index progress) — needs a 10k+ file repo",
    "U-A4 (second instance) — two live processes; not a single-PTY assertion",
    "U-C2 (syntax faces per-language) — color plausibility is a human judgment",
    "U-C3/C4 (50k/10MB files) — needs large-file fixtures",
    "U-C5 (CRLF/binary/empty files) — needs nasty-file fixtures",
    "U-C7/J4 (resize reflow/storm) — needs a live resize driver",
    "U-D1/D2/D3/D4/D5/D6/D7 (xref/imenu/which-function) — symbol-index dependent; "
    "covered by unit tests + the index",
    "U-E4/E5/E6/E7 (count hand-check, references, occur, rapid re-search) — "
    "E7/generation + E5/E6 have store-level unit tests",
    # U-J3, U-F3/F4/F5/F6/F7/F8, and U-G1/G2/G3/G5/G6 are now DRIVEN (see the
    # flow_* functions + the Findings Log). Only U-G4 stays a true non-PTY item:
    "U-G4 (noise immunity) — the watcher's noise filter (is_noise) drops .git/ "
    "internal churn and redline.log before any bus publish; it's a pure function, "
    "unit-tested (summarize_drops_git_and_log_noise, git_and_log_noise_produce_no_event). "
    "A .git write can never reach a buffer reload, so there is no single-PTY behavior "
    "to drive beyond what the unit tests already prove.",
    "U-H4 (key mashing), U-H5 (mode overlap) — fuzz-ish; H5 is unit-tested",
    "U-I1..I5 (config/persistence) — config-file driven; unit-tested",
    "U-J1/J2/J5/J6/J7/J8/J9 (restore/panic/no-truecolor/multibyte/mouse/idle/burst) — "
    "terminal-restore and coalescing are unit/log verified; J9 coalescing is unit-tested",
]


# ── plan-004-issue-03 mark/kill/yank sweep legs ──────────────────────────────

def flow_mark_kill_yank(app):
    """U-M1..U-M9: mark/region/kill/yank verification legs.

    Driven across two buffers: the read-only src/main.rs (multiple lines,
    for the region face + mark + copy) and the notes buffer (editable,
    for yank). C-SPC (NUL) sets the mark, C-n moves the point (region
    active), M-w copies to ring, C-y yanks into notes, C-g clears.
    """
    # Open the read-only file (src/main.rs has multiple lines).
    app.key("C-x C-f")
    app.wait(0.8)
    for ch in "main":
        app.key(ch, settle=0.2)
    app.key("RET")
    app.wait(1.0)
    file_open = "src/main.rs" in row0(app)

    # Move down 2 lines so the top visible line is at line 2.
    app.key("C-n")
    app.wait(0.4)
    app.key("C-n")
    app.wait(0.4)
    # U-M1: C-SPC (NUL byte) sets the mark at line 2 (the current top visible line).
    app.feed(b"\x00", settle=0.8)
    mark_set = "Mark set" in app.row_text(app.rows - 2)

    # U-M9: mark persists after movement; move up 2 lines so the top
    # visible line is back at line 0. The region is now lines 0-2.
    app.key("C-p")
    app.wait(0.4)
    app.key("C-p")
    app.wait(0.4)
    # U-M2: region face visible (attribute-level: background on region lines).
    region_visible = False
    for r in range(1, app.rows - 2):
        row = app.screen.buffer[r]
        for i in range(app.cols):
            bg = str(row[i].bg).lower()
            if bg and bg != '000000' and bg not in BAR_BGS:
                region_visible = True
                break
        if region_visible:
            break

    # U-M6 leg: M-w copy region to kill ring (read-only: buffer unchanged).
    app.key("M-w")
    app.wait(0.8)
    copy_echo = "copied to kill ring" in app.row_text(app.rows - 2)

    # Switch to notes and yank (cross-buffer kill ring).
    app.key("C-x n")
    app.wait(0.8)
    notes_open = ".redline-notes.md" in row0(app)
    before = app.screen_text()
    # U-M4: C-y yank the copied text into the notes buffer.
    app.key("C-y")
    app.wait(0.8)
    after = app.screen_text()
    yank_landed = before != after
    yank_done = "Kill ring is empty" not in app.row_text(app.rows - 2)

    # U-M5: M-y yank-pop (only 1 ring entry, so "end of kill ring" is valid).
    app.key("M-y")
    app.wait(0.8)
    alive = any(row.strip() for row in [app.row_text(r) for r in range(app.rows)])

    # U-M7: C-g clears the region/mark (switch back to the file view first).
    app.key("C-x C-f")
    app.wait(0.6)
    for ch in "main":
        app.key(ch, settle=0.2)
    app.key("RET")
    app.wait(0.8)
    app.key("C-g")
    app.wait(0.5)
    region_cleared = True
    for r in range(1, app.rows - 2):
        row = app.screen.buffer[r]
        for i in range(app.cols):
            bg = str(row[i].bg).lower()
            if bg and bg != '000000' and bg not in BAR_BGS:
                region_cleared = False
                break
        if not region_cleared:
            break

    ok = (file_open and mark_set and region_visible and copy_echo
          and notes_open and yank_done and yank_landed and alive and region_cleared)
    record("U-M1..M7", "C-x C-f,main,RET;NUL;C-n x2;M-w;C-x n;C-y;M-y;C-g", ok,
           f"file-open={file_open} mark-set-echo={mark_set} "
           f"region-face-visible={region_visible} copy-echo={copy_echo} "
           f"notes-open={notes_open} yank-done={yank_done} "
           f"yank-landed={yank_landed} alive-after-m-y={alive} "
           f"region-cleared-by-c-g={region_cleared}")


def flow_mark_exchange(app):
    """U-M8: C-x C-x exchange point and mark (point moves to mark's line;
    window follows so the point stays visible)."""
    # Open the read-only file (src/main.rs, 16 lines — fits in 22-row viewport).
    app.key("C-x C-f")
    app.wait(0.8)
    for ch in "main":
        app.key(ch, settle=0.2)
    app.key("RET")
    app.wait(1.0)
    file_open = "src/main.rs" in row0(app)
    # Set mark at line 0 (point starts at line 0).
    app.feed(b"\x00", settle=0.6)
    mark_set = "Mark set" in app.row_text(app.rows - 2)
    # Move point to line 2 (C-n x2): cursor moves from row 1 to row 3.
    app.key("C-n")
    app.wait(0.4)
    app.key("C-n")
    app.wait(0.4)
    cursor_at_b = app.screen.cursor.y
    # C-x C-x: exchange point and mark → point moves to line 0 (mark's line),
    # cursor returns to row 1. The window follows (no-op here since the
    # file fits the viewport).
    app.key("C-x C-x")
    app.wait(0.6)
    cursor_at_a = app.screen.cursor.y
    exchanged_to_mark = (cursor_at_a != cursor_at_b)
    # C-x C-x again: exchange back → point returns to line 2, cursor at row 3.
    app.key("C-x C-x")
    app.wait(0.6)
    cursor_back = app.screen.cursor.y
    roundtrip = cursor_back == cursor_at_b
    ok = file_open and mark_set and exchanged_to_mark and roundtrip
    record("U-M8", "C-x C-f,main,RET;NUL;C-n x2;C-x C-x;C-x C-x", ok,
           f"file-open={file_open} mark-set={mark_set} "
           f"point-at-mark-line={exchanged_to_mark} "
           f"(cursor row {cursor_at_b}->{cursor_at_a}) "
           f"roundtrip-back-to-point={roundtrip} "
           f"(cursor row {cursor_at_a}->{cursor_back})")


def flow_cross_buffer_kill(app):
    """U-M6: M-w in a read-only view copies to ring; C-y in notes yanks it."""
    # Open a read-only file (src/main.rs from the fixture).
    app.key("C-x C-f")
    app.wait(0.8)
    for ch in "main":
        app.key(ch, settle=0.2)
    app.key("RET")
    app.wait(0.8)
    file_open = "src/main.rs" in row0(app)
    # Set mark at line 0 (NUL byte = C-SPC).
    app.feed(b"\x00", settle=0.6)
    mark_set = "Mark set" in app.row_text(app.rows - 2)
    # Move down 2 lines to create a region.
    app.key("C-n")
    app.wait(0.4)
    app.key("C-n")
    app.wait(0.4)
    # M-w: copy region to kill ring (read-only: buffer unchanged).
    app.key("M-w")
    app.wait(0.8)
    copy_echo = "copied to kill ring" in app.row_text(app.rows - 2)
    # Switch to notes and yank.
    app.key("C-x n")
    app.wait(0.8)
    notes_open = ".redline-notes.md" in row0(app)
    # Record the notes content before yank.
    before = app.screen_text()
    app.key("C-y")
    app.wait(0.8)
    after = app.screen_text()
    yank_landed = before != after
    ok = file_open and mark_set and copy_echo and notes_open and yank_landed
    record("U-M6", "C-x C-f,main,RET;NUL;C-n x2;M-w;C-x n;C-y", ok,
           f"file-open={file_open} mark-set={mark_set} copy-echo={copy_echo} "
           f"notes-open={notes_open} cross-buffer-yank-landed={yank_landed}")


def main():
    _reset_fixture()
    app = App(REPO, rows=ROWS, cols=COLS)
    flow_a1(app)
    flow_b1(app)
    app.kill()

    app = App(REPO, rows=ROWS, cols=COLS)
    flow_b2(app)
    flow_b3(app)
    flow_b4(app)
    flow_b5(app)
    flow_b6(app)
    app.kill()

    app = App(REPO, rows=ROWS, cols=COLS)
    flow_c1(app)
    flow_c6(app)
    app.kill()

    app = App(REPO, rows=ROWS, cols=COLS)
    flow_e1(app)
    flow_e3(app)
    flow_e2(app)
    app.kill()

    app = App(REPO, rows=ROWS, cols=COLS)
    flow_f1(app)
    flow_f2(app)
    app.kill()

    # Graft pollution needs its own App: the graft dir must exist before the
    # startup file walk so the pruned walk is what the finder lists.
    flow_graft()

    app = App(REPO, rows=ROWS, cols=COLS)
    flow_h1(app)
    flow_h2(app)
    flow_h3(app)
    flow_qquit(app)
    flow_notes(app)
    flow_palette(app)
    app.kill()

    # Editable-keys: its own App (it leaves the notes buffer current, which
    # would change the view state the remaining flows assume).
    app = App(REPO, rows=ROWS, cols=COLS)
    flow_editable_keys(app)
    app.kill()

    # U-H2 "search running" state: a large repo so the walk is in flight
    # when C-g lands (the small fixture finishes a search too fast).
    ensure_slow_repo()
    app = App(SLOW_REPO, rows=ROWS, cols=COLS)
    flow_h2_search(app)
    app.kill()

    # U-G3: needs its own App (it modifies src/main.rs on disk and the
    # watcher must fire while the buffer is locally-owned).
    app = App(REPO, rows=ROWS, cols=COLS)
    flow_g3(app)
    app.kill()

    # ── round-3 driven flows (plan 002 issue 03 must-fixes) ───────────────
    # Magit-context flows (J3 no-wrap; F3 fold/visit; F6 log; F7 blame; F8
    # branch/stash): read-only on the fixture baseline (+1 ~1), share one App.
    app = App(REPO, rows=ROWS, cols=COLS)
    flow_j3(app)
    flow_f3(app)
    flow_f6(app)
    flow_f7(app)
    flow_f8(app)
    app.kill()

    # U-F5 commit flow: creates a commit, so it runs on a dedicated throwaway
    # repo (REPO5) — REPO's baseline/history are never touched.
    _ensure_commit_repo()
    app = App(REPO5, rows=ROWS, cols=COLS)
    flow_f5(app, REPO5)
    app.kill()

    # Watcher flows (F4 live reload; G1 scroll-preservation; G2 churn; G6
    # suspend): all against a dedicated src/g_watch.rs so the baseline fixture
    # files are never mutated by the append legs.
    app = App(REPO, rows=ROWS, cols=COLS)
    gw = os.path.join(REPO, "src", "g_watch.rs")
    with open(gw, "w") as f:
        f.write("gwatch line one\ngwatch line two\n")
    try:
        app.key("C-c p i")
        app.wait(1.0)
        app.key("C-x C-f")
        app.wait(0.8)
        for ch in "g_watch":
            app.key(ch, settle=0.2)
        app.key("RET")
        app.wait(0.8)
        flow_f4(app, gw, "g_watch")
        flow_g1(app, gw)
        flow_g2(app, gw)
        flow_g6(app, gw)
    finally:
        app.kill()
        try:
            os.remove(gw)
        except FileNotFoundError:
            pass

    # U-G5 project switch: needs a second registered project + an isolated
    # cache so the real user registry is never touched (own App inside).
    flow_g5()

    # plan-003-03 new windowing/scroll sweep legs: commit-diff scroll, blame
    # windowing, notes scroll (each self-contained on a throwaway repo; the
    # deep drives are tools/drive_windowing_panes.py).
    flow_commit_diff_scroll()
    flow_blame_windowing()
    flow_notes_scroll()
    flow_banner_hint()

    # plan-004-issue-03 mark/kill/yank sweep legs (on the fixture repo).
    app = App(REPO, rows=ROWS, cols=COLS)
    flow_mark_kill_yank(app)
    flow_mark_exchange(app)
    app.kill()

    app = App(REPO, rows=ROWS, cols=COLS)
    flow_cross_buffer_kill(app)
    app.kill()

    print("\n=== SUMMARY ===")
    for flow, keys, ok, _ in RESULTS:
        print(f"{'PASS' if ok else 'FAIL'}  {flow}  {keys}")
    bad = [f for f, _, ok, _ in RESULTS if not ok]
    print(f"\nDRIVEN: {len(RESULTS)} flows, {len(RESULTS)-len(bad)} PASS, {len(bad)} FAIL")
    print(f"NOT APPLICABLE to a single-PTY pyte pass: {len(NOT_APPLICABLE)} groups")
    for n in NOT_APPLICABLE:
        print(f"  - {n}")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

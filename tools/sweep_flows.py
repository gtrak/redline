#!/usr/bin/env python3
"""Plan 002 issue 03 — Sweep & re-measure: the U-* flow sweep table.

loop-03 (demote the test pyramid): this file is now the THIN PTY tier.
~60 of the 65 driven flows are demoted to store-level unit twins in
`src/app/flow_tests.rs` (driven through the same entry points — the
store's key_event / apply_project_change / SearchBus — and asserting the
same positive signals on state + the width-bounded static render,
`render_at_width(80)`; the kept/converted ledger is in
docs/ux-testing-plan.md). What remains here is what only a live app
proves:

  - U-A1                 boot/frame smoke (launch -> home, 80x24)
  - editable-keys        input encoding: multi-key sequences + self-insert
                         through the real terminal encoder
  - U-H2 search          in-flight search cancel — C-g must land while a
                         real rg walk over the 6000-file repo is in flight
                         (a wall-clock race no store call can reproduce)
  - U-BHN                banner debounce: the hint only lands after the
                         watcher cadence (repaint-race class)
  - annotation suite     ann-delete's transient-echo / repaint race + the
                         raw space-byte encoding (state halves are the
                         unit_flow_ann_* twins)
  - accurate-mode        the point-accurate edit bindings (insert / C-t / M-DEL
                         / C-d / C-o / C-k / C-u) driven through the real
                         encoder + guard, each proven by byte + a save read
                         back from disk (the state halves are the
                         accurate_* store-level twins)
  - U-G6                 watcher SUSPENDED: a disk edit produces no reload
                         (the gate is at the watcher source, which the
                         store-level apply path deliberately bypasses)
  - U-G1                 watcher delivery smoke: disk append -> repaint +
                         scroll anchor (one end-to-end watcher leg)
  - quit-prompt-y        quit lifecycle: prompt -> y -> on-disk write ->
                         exit 0 (process lifecycle; the state halves are
                         the unit_flow_quit_prompt_* twins)

Flows that require a live second pane / human hands / a special repo are
listed in NOT_APPLICABLE below, not silently passed.
"""
import os
import re
import subprocess
import sys
import time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App, encode_key
from fixture import repo, reset as _reset_fixture

REPO = repo("redline_pyte_repo")
# A dedicated repo for the U-H2 "search running" C-g state: large enough
# that the walk is still in flight when C-g lands immediately after RET
# (the small fixture repo finishes a search before C-g can arrive).
SLOW_REPO = repo("redline_sweep_slow_repo")
SLOW_FILES = 6000
ROWS, COLS = 24, 80

# plan-005 issue 02: inline annotations (the thin-tier suite's files).
ANN_PATH = os.path.join(REPO, "src", "notes_ann.rs")
ANN2_PATH = os.path.join(REPO, "src", "notes_ann2.rs")
NOTES_PATH = os.path.join(REPO, ".redline-notes.md")
# plan-015 issue 03: accurate-mode point-accurate editing (the thin-tier
# leg's dedicated file). Tall enough that a half-page scroll is observable.
ACC_PATH = os.path.join(REPO, "src", "acc_edit.rs")
# The save/quit-confirm prompt markers (asserted on by quit-prompt-y —
# the on-terminal width-80 wrap check lives with this leg).
PROMPT_HEAD = "Save this buffer:"
PROMPT_KEYS = "(y, n, !, C-g)"

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


def flat_text(app):
    """Screen text with all whitespace runs collapsed to single spaces.

    A prompt or message that wraps past 80 cols (the pool-lane fixture paths
    are longer than the shared /tmp ones, so the quit-save prompt's key list
    wraps onto the next row) must not defeat substring assertions: the wrap
    point is a layout artifact, not a missing prompt.
    """
    return re.sub(r"\s+", " ", text(app))


def row0(app):
    return app.row_text(0)


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
    Returns whether it landed before `timeout`.

    Positive-gated settle (loop-02): use this instead of a fixed `app.wait()`
    before any absence-style assertion ("X" not in screen, count == 0,
    unchanged). `pred()` must be a POSITIVE completion signal proving the
    action rendered (a prompt/count line, an echo, the mode line back to
    `ready`, the target row). Without the gate a too-short read-quiet window
    makes the absence check run before the repaint — a vacuous pass that
    ships a regression green.
    """
    deadline = time.time() + timeout
    while time.time() < deadline:
        app._read(0.2, quiet=0.0)
        if pred():
            return True
    return pred()


def flow_a1(app):
    """U-A1 Cold start (06a): HOME frame — project + "redline" header, the
    derived command groups, the standard help line — + status line + ready.
    `*scratch*` no longer exists at boot."""
    t = text(app)
    once = lambda tok: sum(1 for r in range(app.rows) if tok in app.row_text(r)) == 1
    header = "redline" in row0(app) and "redline_pyte_repo" in row0(app)
    groups = ("[buffers]" in t and "[files]" in t and "[git]" in t
              and once("[buffers]") and once("[git]") and once("C-x g")
              and "C-x C-f" in t and "C-x C-c" in t)
    help_line = "C-x C-c quit" in t and "? menu" in t
    ok = (header and groups and help_line and "ready" in t
          and "* redline_pyte_repo *  home" in t and "*scratch*" not in t)
    record("U-A1", "(launch)", ok,
           f"header={header} groups={groups} help={help_line} "
           f"home-status={'* redline_pyte_repo *  home' in t} "
           f"no-scratch={'*scratch*' not in t} (row0={row0(app)!r})")


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


def flow_h2_search(app):
    """U-H2 "search running" state: C-g cancels the in-flight search and the
    view stays open on the (partial) results. RET and C-g are written to the
    PTY in a single buffer so C-g is processed while the walk over the
    6000-file repo is still running."""
    app.key("C-c p s s")
    for ch in "target":
        app.key(ch, settle=0.15)
    app.feed(encode_key("RET") + encode_key("C-g"))
    # Positive gate (loop-02): the cancel message IS the assertion — poll
    # for it instead of a fixed settle. If the walk finished before C-g
    # landed the message never appears and the flow FAILs (no longer an
    # in-flight cancel).
    cancelled = wait_for(app, lambda: "search cancelled"
                         in app.row_text(app.rows - 2), 4.0)
    view_stays = "Search:" in row0(app)
    alive = "(cancelled)" in row0(app)
    record("U-H2 search", "C-c p s s,target,RET+C-g(fast)", cancelled and view_stays and alive,
           f"in-flight-cancel-message={cancelled} view-stays-open={view_stays} "
           f"title={row0(app)[:40]!r}")
    app.key("q")
    app.wait(0.6)


def _open_ann_file(app, name):
    """Open a file via the find-file picker (the annotation suite's files)."""
    app.key("C-x C-f")
    app.wait(0.8)
    for ch in name:
        app.key(ch, settle=0.2)
    app.key("RET")
    app.wait(0.8)





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
            # (loop-02) The notes file is CREATED BY THE APP on `C-x n`; its
            # first watcher event is consumed by design (store created_paths
            # guard) and the debouncer coalesces that creation with a rapid
            # first external append into ONE swallowed batch — the banner
            # then legitimately does not fire for that batch. A second,
            # genuine external edit MUST raise the banner; that is the
            # discriminating assertion, the first append is only a trigger.
            # If the banner never lands after the bounded retry, FAIL.
            banner_landed = wait_for(app, lambda: "changed on disk" in text(app), 2.0)
            if not banner_landed:
                with open(notes_path, "a") as f:
                    f.write("second_external_edit\n")
                banner_landed = wait_for(
                    app, lambda: "changed on disk" in text(app), 4.0)
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



def reap(app):
    """Reap the PTY child if it has exited. Returns the exit status (int,
    negative when killed by a signal), or None while the process is alive."""
    try:
        pid, status = os.waitpid(app.pid, os.WNOHANG)
    except ChildProcessError:
        return 0  # already reaped
    if pid == 0:
        return None  # still running
    if os.WIFEXITED(status):
        return os.WEXITSTATUS(status)
    if os.WIFSIGNALED(status):
        return -os.WTERMSIG(status)
    return status


def wait_exit(app, timeout=12.0):
    """Pump the PTY until the process has exited (or the timeout lapses).
    Returns the exit status, or None if the app is still alive."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        status = reap(app)
        if status is not None:
            return status
        app._read(0.3, quiet=0.2)
    return reap(app)


def _read_notes(path):
    if not os.path.exists(path):
        return None
    with open(path) as f:
        return f.read()


def flow_quit_prompt_y():
    """Modified notes + `C-x C-c` → prompt names the path; `y` writes the
    file (contents asserted on disk) and the process ends with exit 0."""
    notes = os.path.join(REPO, ".redline-notes.md")
    try:
        app = App(REPO, rows=ROWS, cols=COLS)
        app.key("C-x n")
        app.wait(0.8)
        notes_open = ".redline-notes.md" in row0(app)
        app.key("Q", settle=0.4)
        app.key("Y", settle=0.4)
        app.key("C-x C-c", settle=1.0)
        # Positive gate (loop-02): the prompt must render before the
        # path/keys asserts — a quit that hasn't painted yet is not a
        # verdict (and exit-0 below independently gates it: a pending
        # prompt blocks the exit).
        prompted = wait_for(
            app, lambda: (PROMPT_HEAD in flat_text(app)
                          and ".redline-notes.md" in flat_text(app)
                          and PROMPT_KEYS in flat_text(app)), 4.0)
        alive = reap(app) is None
        app.key("y", settle=2.0)
        status = wait_exit(app)
        on_disk = _read_notes(notes) or ""
        ok = (notes_open and prompted and alive and status == 0
              and "QY" in on_disk)
        record("quit-prompt-y", "C-x n,QY;C-x C-c;y", ok,
               f"notes-open={notes_open} prompt-rendered-with-path={prompted} "
               f"alive-during-prompt={alive} exit-0-after-y={status == 0} "
               f"file-contains-edit={('QY' in on_disk)} (status={status})")
    finally:
        try:
            app.kill()
        except NameError:
            pass
        try:
            os.remove(notes)
        except FileNotFoundError:
            pass

def flow_annotation_create(app):
    """A on a line: the minibuffer prompt, RET commits — a ▎ marker on the
    code row, the note as a dim row directly under it, the status count,
    and a real record in .redline-notes.md."""
    for _ in range(2):
        app.key("C-n")
        app.wait(0.3)
    app.key("A")
    app.wait(0.4)
    prompt = "Note: " in app.row_text(app.rows - 2)
    for ch in "check bounds":
        # The PTY encoder splits key sequences on whitespace, so the space
        # is fed as a raw byte (encode_key would drop a bare "").
        if ch == " ":
            app.feed(b" ", settle=0.2)
        else:
            app.key(ch, settle=0.2)
    typed = "Note: check bounds" in app.row_text(app.rows - 2)
    app.key("RET")
    app.wait(0.6)
    saved = "note saved" in app.row_text(app.rows - 2)
    code_rows = rows_containing(app, "line three")
    marker = any("\u258e" in app.row_text(r) for r in code_rows)
    note_rows = rows_containing(app, "\u25b8 check bounds")
    under = bool(code_rows) and bool(note_rows) and note_rows[0] == code_rows[0] + 1
    count = "1 note" in app.row_text(app.rows - 1)
    disk = open(NOTES_PATH).read() if os.path.exists(NOTES_PATH) else ""
    disk_rec = ("[annotation]" in disk and "note: check bounds" in disk
                and "anchor: ann line three" in disk and "line: 2" in disk)
    ok = prompt and typed and saved and marker and under and count and disk_rec
    record("ann-create", "C-n C-n,A,check bounds,RET", ok,
           f"prompt={prompt} typed={typed} saved-msg={saved} marker={marker} "
           f"note-row-under={under} status-count={count} disk-record={disk_rec}")

def flow_annotation_toggle(app):
    """C-c a hides the note rows (the marker stays) and shows them again."""
    app.key("C-c a")
    app.wait(0.4)
    hidden_msg = "note rows: hidden" in app.row_text(app.rows - 2)
    note_gone = not rows_containing(app, "\u25b8 check bounds")
    marker_stays = any("\u258e" in app.row_text(r) for r in rows_containing(app, "line three"))
    app.key("C-c a")
    app.wait(0.4)
    shown_msg = "note rows: shown" in app.row_text(app.rows - 2)
    note_back = bool(rows_containing(app, "\u25b8 check bounds"))
    ok = hidden_msg and note_gone and marker_stays and shown_msg and note_back
    record("ann-toggle", "C-c a ×2", ok,
           f"hidden-msg={hidden_msg} note-gone={note_gone} marker-stays={marker_stays} "
           f"shown-msg={shown_msg} note-back={note_back}")

def flow_annotation_crossing(app):
    """Map correctness: with the cursor on the annotated line, C-n lands on
    the NEXT CODE line (L4, not the note row) and C-p returns (L3); the
    note row still sits between the two code rows on screen. The file is
    30 lines so the position display reads L*, not Bot (Bot is the
    near-bottom position, viewport 21)."""
    app.key("C-n")
    app.wait(0.3)
    l4 = "L4," in app.row_text(app.rows - 1)
    app.key("C-p")
    app.wait(0.3)
    l3 = "L3," in app.row_text(app.rows - 1)
    r3 = rows_containing(app, "line three")
    r4 = rows_containing(app, "line four")
    between = bool(r3) and bool(r4) and r4[0] == r3[0] + 2
    ok = l4 and l3 and between
    record("ann-crossing", "C-n,C-p on annotated line", ok,
           f"c-n-lands-L4={l4} c-p-back-L3={l3} note-row-between={between}")

def flow_annotation_notes_editable(app):
    """In the EDITABLE notes buffer `d`/`A` self-insert as printables (the
    line-anchored semantics are file-view only; no prompt, no silent
    delete, no unbound-key echo)."""
    app.key("C-x n")
    app.wait(0.6)
    edit_mode = "Edit" in app.row_text(app.rows - 1)
    app.key("d")
    app.wait(0.3)
    no_delete = ("deleted annotation" not in app.row_text(app.rows - 2)
                 and "no annotation" not in app.row_text(app.rows - 2))
    self_inserted = any(app.row_text(i).rstrip().endswith("d")
                        for i in range(1, app.rows - 2))
    app.key("A")
    app.wait(0.3)
    no_prompt = "Note: " not in app.row_text(app.rows - 2)
    ok = edit_mode and no_delete and self_inserted and no_prompt
    record("ann-notes-editable", "C-x n;d;A", ok,
           f"edit-mode={edit_mode} d-no-delete={no_delete} d-self-inserted={self_inserted} "
           f"A-no-prompt={no_prompt}")

def flow_annotation_drift(app):
    """Out-of-band edit moves the anchored line: the auto-reload's
    re-anchor pass moves the cue with the content (silent, by text)."""
    app.key("C-x b")
    app.wait(0.6)
    for ch in "ann.rs":
        app.key(ch, settle=0.2)
    app.key("RET")
    app.wait(0.6)
    with open(ANN_PATH, "r") as f:
        old = f.read()
    with open(ANN_PATH, "w") as f:
        f.write("top extra line\n" + old)
    reloaded = wait_for(app, lambda: "top extra line" in text(app), 4.0)
    marker_moved = any("\u258e" in app.row_text(r) for r in rows_containing(app, "line three"))
    note_under = bool(rows_containing(app, "\u25b8 check bounds"))
    count = "1 note" in app.row_text(app.rows - 1)
    ok = reloaded and marker_moved and note_under and count
    record("ann-drift", "disk prepend;top extra line", ok,
           f"auto-reload={reloaded} cue-follows-content={marker_moved} "
           f"note-row={note_under} count-kept={count}")

def flow_annotation_orphan(app):
    """Delete the anchored line out-of-band: the annotation is NOT lost —
    it is flagged orphaned (the cue carries the tag) and the record stays
    in the notes file."""
    lines = [l for l in open(ANN_PATH).read().splitlines() if l != "ann line three"]
    with open(ANN_PATH, "w") as f:
        f.write("\n".join(lines) + "\n")
    orphaned = wait_for(app, lambda: "(orphaned)" in text(app), 4.0)
    record_kept = "1 note" in app.row_text(app.rows - 1)
    disk_kept = "note: check bounds" in open(NOTES_PATH).read()
    ok = orphaned and record_kept and disk_kept
    record("ann-orphan", "disk delete of anchor line", ok,
           f"orphan-flag-shown={orphaned} status-count-kept={record_kept} "
           f"disk-record-kept={disk_kept}")

def flow_annotation_delete(app):
    """d on the (orphaned) annotated line deletes the record with an echo
    (cue gone, count gone, disk record gone); d on an unannotated line is
    a no-op with a clear message."""
    app.key("C-n")
    app.wait(0.3)
    app.key("d")
    # Positive gate (loop-02): the delete echo proves the `d` dispatched and
    # rendered before the cue-gone / count-gone / disk-gone absence checks.
    echo = wait_for(app, lambda: "deleted annotation: check bounds"
                    in app.row_text(app.rows - 2), 4.0)
    # Content rows only: the minibuffer's echo legitimately contains the
    # note text, so the cue check excludes it.
    cue_gone = not any("\u25b8 check bounds" in app.row_text(r)
                       for r in range(1, app.rows - 2))
    count_gone = "1 note" not in app.row_text(app.rows - 1)
    disk_gone = "check bounds" not in open(NOTES_PATH).read()
    app.key("C-p")
    app.wait(0.3)
    app.key("d")
    # Positive gate (loop-02): the no-op message IS the assertion — poll for
    # the transient echo instead of a fixed settle that can race the repaint.
    # If a late repaint (e.g. the in-flight watcher re-anchor from the
    # orphan leg's disk write) overwrote the echo, re-arm with the same
    # idempotent no-op `d`: the echo re-prints, the assertion is unchanged,
    # and a `d` that never echoes still FAILs both polls.
    no_ann_msg = wait_for(app, lambda: "no annotation on this line"
                          in app.row_text(app.rows - 2), 1.5)
    if not no_ann_msg:
        app.key("d")
        no_ann_msg = wait_for(app, lambda: "no annotation on this line"
                              in app.row_text(app.rows - 2), 4.0)
    ok = echo and cue_gone and count_gone and disk_gone and no_ann_msg
    record("ann-delete", "d on annotated line; d on clean line", ok,
           f"echo={echo} cue-gone={cue_gone} count-gone={count_gone} "
           f"disk-record-gone={disk_gone} unannotated-msg={no_ann_msg}")

def flow_annotation_cu_still_scrolls(app):
    """Guard: C-u is still half-page scroll (the annotate bindings did not
    shadow it — there is no C-u A; the delete key is d). At the bottom of
    a 30-line file the window top row is 'cu line 11'; C-u moves the window
    up a half-page to 'cu line 1'. The file was created before startup so
    the find-file picker (the cached walk) lists it."""
    _open_ann_file(app, "notes_ann2.rs")
    app.key("G")
    app.wait(0.4)
    at_bottom = "Bot" in app.row_text(app.rows - 1)
    top_before = app.row_text(1)
    app.key("C-u")
    app.wait(0.4)
    top_after = app.row_text(1)
    moved_up = "cu line 11" in top_before and "cu line 1" in top_after
    ok = at_bottom and moved_up and top_before != top_after
    record("ann-cu-scroll", "G,C-u (30-line file)", ok,
           f"at-bottom-first={at_bottom} top-row {top_before!r} → {top_after!r} "
           f"c-u-moved-window-up={moved_up}")


def flow_accurate_suite():
    """plan-015 issue 03: the accurate-mode point-accurate editing legs.

    One leg that drives EVERY point-accurate binding through the real terminal
    encoder + the notes-edit guard (the thing only a live app proves), and
    proves each fires by byte: after each binding the on-screen buffer row
    reflects exactly that operation (a wrong / unbound / mis-routed binding
    would leave the row unchanged or produce a different byte), and the final
    `C-x C-s` save is read back from disk byte-for-byte. C-u stays half-page
    scroll (universal-argument is 015 item 9, out of scope): it must scroll,
    not self-insert 'u'. Own App + a dedicated file (created before startup so
    the find-file picker lists it, removed after)."""
    # 40 lines: line 0 "ab cd", line 1 "ef gh", then 38 fillers. Tall enough
    # that a half-page scroll is observable; the first two lines are the edit
    # targets.
    with open(ACC_PATH, "w") as f:
        f.write("ab cd\n")
        f.write("ef gh\n")
        f.write("".join(f"filler {i:02d}\n" for i in range(1, 39)))
    try:
        app = App(REPO, rows=ROWS, cols=COLS)
        _open_ann_file(app, "acc_edit.rs")
        opened = "ab cd" in text(app) and "ef gh" in text(app)
        # Enter Accurate mode (C-x C-q toggle-read-only).
        app.key("C-x C-q")
        app.wait(0.4)
        accurate = "accurate mode" in app.row_text(app.rows - 1) or \
                   "accurate mode" in app.row_text(app.rows - 2)
        # (1) insert: type 'Z' at the start (point (0,0)) → "Zab cd".
        app.key("Z", settle=0.2)
        app.wait(0.3)
        did_insert = "Zab cd" in text(app)
        # (2) C-t: transpose 'Z'/'a' → "aZb cd"; the pre-swap bytes gone.
        app.key("C-t")
        app.wait(0.4)
        did_ct = "aZb cd" in text(app) and "Zab cd" not in text(app)
        # (3) M-DEL: kill-word-backward kills 'a' (the word before the point) →
        # "Zb cd". M-DEL = Alt+Backspace (ESC + 0x7F); encode_key has no M-DEL
        # token, so feed the raw bytes.
        app.feed(b"\x1b\x7f", settle=0.3)
        app.wait(0.4)
        did_mdel = "Zb cd" in text(app) and "aZb cd" not in text(app)
        # (4) C-d: delete-char-forward removes 'Z' (the char at the point) →
        # "b cd".
        app.key("C-d")
        app.wait(0.4)
        did_cd = "b cd" in text(app) and "Zb cd" not in text(app)
        # (5) C-o: open-line. C-f to the space (col 1), then C-o splits the
        # line: "b" on one row, " cd" on the next. The single "b cd" row is
        # gone (split); the " cd" remainder is on its own row.
        app.key("C-f", settle=0.2)
        app.wait(0.2)
        app.key("C-o")
        app.wait(0.4)
        did_co = " cd" in text(app) and "b cd" not in text(app)
        # (6) C-k: kill-line. C-n C-n lands on the "ef gh" line (now line 2
        # after C-o pushed it down); C-a to line start (C-n carries goal_col,
        # so without it the point lands mid-line and C-k kills only part);
        # C-k clears the whole line's content.
        app.key("C-n", settle=0.2)
        app.wait(0.2)
        app.key("C-n", settle=0.2)
        app.wait(0.2)
        app.key("C-a", settle=0.2)
        app.wait(0.2)
        app.key("C-k")
        app.wait(0.4)
        did_ck = "ef gh" not in text(app)
        # (7) C-u: half-page scroll UP. Scroll down first (C-v page-down; a
        # window scroll, not an accurate-mode edit) so there is room to scroll
        # up; then C-u moves the window up a half-page (the top content row
        # changes). A no-op or a self-inserted 'u' would not change the TOP row.
        app.key("C-v")
        app.wait(0.4)
        top_before = app.row_text(1)
        app.key("C-u")
        app.wait(0.4)
        top_after = app.row_text(1)
        did_cu = (top_before != top_after
                  and "filler" in top_before and "filler" in top_after)
        # Save and read the disk bytes back (the definitive by-byte artifact).
        app.key("C-x C-s")
        saved = wait_for(app, lambda: "wrote" in flat_text(app), 4.0)
        app.wait(0.3)
        with open(ACC_PATH, "r") as f:
            lines = f.read().split("\n")
        disk_ok = (len(lines) >= 4 and lines[0] == "b" and lines[1] == " cd"
                   and lines[2] == "" and lines[3] == "filler 01")
        ok = (opened and accurate and did_insert and did_ct and did_mdel
              and did_cd and did_co and did_ck and did_cu and disk_ok)
        record("accurate-mode", "Z,C-t,M-DEL,C-d,C-o,C-k,C-u,C-x C-s",
               ok,
               f"opened={opened} accurate={accurate} insert={did_insert} "
               f"C-t={did_ct} M-DEL={did_mdel} C-d={did_cd} C-o={did_co} "
               f"C-k={did_ck} C-u-scroll={did_cu} saved={saved} "
               f"disk-bytes={disk_ok} top {top_before!r}->{top_after!r} "
               f"disk[:4]={lines[:4]}")
        app.kill()
    finally:
        try:
            os.remove(ACC_PATH)
        except FileNotFoundError:
            pass


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


def flow_annotation_suite():
    """plan-005 issue 02: the inline-annotation legs (create → toggle →
    C-n/C-p map crossing → notes-buffer editable semantics → drift
    re-anchor → orphan → delete → C-u guard). Own App; the dedicated
    files are created before startup (the find-file picker lists the
    cached walk) and removed after (the fixture's .redline-notes.md is
    swept by the reset)."""
    with open(ANN_PATH, "w") as f:
        # 30 content lines: the four named lines + fillers (30 lines keeps
        # the annotated line far enough from the bottom that the status
        # position display reads L*, not Bot).
        f.write("ann line one\nann line two\nann line three\n"
                "ann line four\n" +
                "".join(f"ann filler {i:02d}\n" for i in range(5, 31)))
    with open(ANN2_PATH, "w") as f:
        f.write("".join(f"cu line {i}\n" for i in range(1, 31)))
    try:
        app = App(REPO, rows=ROWS, cols=COLS)
        _open_ann_file(app, "notes_ann.rs")
        flow_annotation_create(app)
        flow_annotation_toggle(app)
        flow_annotation_crossing(app)
        flow_annotation_notes_editable(app)
        flow_annotation_drift(app)
        flow_annotation_orphan(app)
        flow_annotation_delete(app)
        flow_annotation_cu_still_scrolls(app)
        app.kill()
    finally:
        for p in (ANN_PATH, ANN2_PATH):
            try:
                os.remove(p)
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
    # loop-03: the demoted flows are NOT "not applicable" — they are
    # unit-driven: see src/app/flow_tests.rs and the ledger in
    # docs/ux-testing-plan.md. Only U-G4 stays a true non-PTY item:
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


def main():
    _reset_fixture()
    app = App(REPO, rows=ROWS, cols=COLS)
    flow_a1(app)
    # Editable-keys: its own App (it leaves the notes buffer current, which
    # would change the view state the remaining flows assume).
    app.kill()
    app = App(REPO, rows=ROWS, cols=COLS)
    flow_editable_keys(app)
    app.kill()
    # U-H2 "search running" state: a large repo so the walk is in flight
    # when C-g lands (the small fixture finishes a search too fast).
    ensure_slow_repo()
    app = App(SLOW_REPO, rows=ROWS, cols=COLS)
    flow_h2_search(app)
    app.kill()
    # U-BHN: the banner debounce leg (its own App; drives + asserts the
    # hint text on the notes buffer).
    flow_banner_hint()
    # Watcher legs (G1 delivery smoke + G6 suspend) against a dedicated
    # src/g_watch.rs so the baseline fixture files are never mutated.
    gw = os.path.join(REPO, "src", "g_watch.rs")
    with open(gw, "w") as f:
        f.write("gwatch line one\ngwatch line two\n")
    try:
        app = App(REPO, rows=ROWS, cols=COLS)
        app.key("C-c p i")
        app.wait(1.0)
        app.key("C-x C-f")
        app.wait(0.8)
        for ch in "g_watch":
            app.key(ch, settle=0.2)
        app.key("RET")
        app.wait(0.8)
        flow_g1(app, gw)
        flow_g6(app, gw)
        app.kill()
    finally:
        try:
            os.remove(gw)
        except FileNotFoundError:
            pass
    # quit-prompt-y: the quit-lifecycle leg (ends the process: its own App).
    flow_quit_prompt_y()
    # plan-005-issue-02 inline annotation legs (own App + dedicated files):
    # the thin tier keeps the full suite (the ann-delete transient-echo
    # repaint race + the raw space-byte encoding); the state halves are
    # the unit_flow_ann_* twins.
    flow_annotation_suite()
    # plan-015 issue 03: the accurate-mode point-accurate editing leg (own
    # App + a dedicated file; drives insert/C-t/M-DEL/C-d/C-o/C-k/C-u and
    # reads the save back from disk by byte).
    flow_accurate_suite()

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

#!/usr/bin/env python3
"""Plan 018 issue 03: the search RESULTS view narrows on its own prompt.

The PTY leg for the results-view narrowing:
  * `C-c p s s` + query + RET lists the hits; the view carries a ONE-row
    NoWrap narrow prompt under the title, DECISION KEYS LEADING
    (`filter:  RET jump · n/p · g re-run · C-g clear · q close`), the
    query trailing (PLAN 5.2 — a clip must never hide the keys);
  * typing narrows the rows LIVE (FilterOnly: non-matching hits drop
    out, file headers survive iff a child does); the title carries both
    numbers ("1 of 8 matches") so the narrowing state is always visible;
  * RET jumps to the narrowed row's hit; `M-,` returns to the results
    with the selection restored, the narrow still active;
  * C-g (job idle) CLEARS the query — the full list re-derives, the
    selection not lost; C-g again (empty query) is the no-op echo;
  * `g` re-run clears the query (a new job is a new result set).

Query strings deliberately avoid q/n/p/g — those are the view's own
decision keys (q closes, n/p move, g re-runs); they fall through to the
keymap, not the query ("src", "lib", "mai" are all clean).

Every navigation step asserts exactly ONE blue row (the selected-row bar)
and no reverse rows. Fixture: /tmp/redline_pyte_repo (8 "target" hits:
2 README.md, 1 src/lib.rs, 5 src/main.rs).

Registered in tools/gate.sh SHARED_SUITES and tools/pool.py BATTERY
(src/gate_registry.rs cross-checks both).
"""
import os
import sys
import time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo

PROMPT = "filter:"
DECISIONS = ["RET jump", "n/p", "g re-run", "C-g clear", "q close"]
QUERY = "target"
N_HITS = 8
N_FILES = 3

BAD = []


def verdict(tag, ok, detail=""):
    print(f"   {'OK' if ok else 'FAIL'} {tag}: {detail}")
    BAD.append(not ok)


def one_blue(app):
    return len(app.blue_rows()) == 1 and len(app.reverse_rows()) == 0


def prompt_row(app, expect_query=None, expect_placeholder=False):
    """The prompt row: decision keys leading; the query (or the
    placeholder) trailing on the SAME row."""
    lines = app.screen_text().splitlines()
    idx = next((i for i, l in enumerate(lines) if PROMPT in l), None)
    if idx is None:
        return False, "<prompt row missing>"
    text = lines[idx]
    ok = all(d in text for d in DECISIONS)
    if expect_query is not None:
        ok = (
            ok
            and expect_query in text
            and text.find(expect_query) > text.find(DECISIONS[-1])
        )
    if expect_placeholder:
        ok = ok and "type to narrow" in text
    return ok, text


def wait_done(app, timeout=15.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        app._read(0.4, quiet=0.2)
        text = app.screen_text()
        if "searching" not in text and "matches in" in text:
            return True
    return False


def main():
    app = App(repo("redline_pyte_repo"), rows=24, cols=80)
    app.key("C-c p s s", 1.0)
    for ch in QUERY:
        app.key(ch, settle=0.3)
    app.key("RET", 1.0)

    if not wait_done(app):
        print("FAIL: search did not finish")
        print(app.screen_text())
        app.kill()
        sys.exit(1)

    print("=== results frame (un-narrowed) ===")
    for r in range(app.rows):
        t = app.row_text(r)
        if t.strip():
            print(f"{r:02d} | {t}")
    print()

    # 1. The at-rest prompt row: keys leading, the placeholder trailing.
    ok, text = prompt_row(app, expect_placeholder=True)
    verdict("prompt row (keys leading, placeholder)", ok, f"prompt={text!r}")
    verdict("title: both numbers un-narrowed",
            f"{N_HITS} matches in {N_FILES} files" in app.screen_text(),
            "full-count title")
    verdict("one-blue at open", one_blue(app), f"blue={app.blue_rows()}")

    # 2. Type "lib": the list narrows LIVE to the single lib.rs hit.
    for ch in "lib":
        app.key(ch, settle=0.4)
    screen = app.screen_text()
    verdict("narrowed: only the lib.rs hit row",
            "pub fn target_lib() {}" in screen
            and "target_one" not in screen
            and "target README" not in screen,
            f"lib in: {'pub fn target_lib() {}' in screen}, "
            f"main gone: {'target_one' not in screen}, "
            f"README gone: {'target README' not in screen}")
    ok, text = prompt_row(app, expect_query="lib")
    verdict("query trails the decision keys", ok, f"prompt={text!r}")
    verdict("title: both numbers while narrowed",
            f"1 of {N_HITS} matches" in app.screen_text(), "narrowed title")
    blues = app.blue_rows()
    verdict("one-blue on the narrowed hit",
            one_blue(app) and "target_lib" in app.row_text(blues[0]),
            f"blue={[app.row_text(r) for r in blues]}")

    print("=== narrowed frame (query 'lib') ===")
    for r in range(app.rows):
        t = app.row_text(r)
        if t.strip():
            print(f"{r:02d} | {t}"
                  + ("  [BLUE]" if r in blues else ""))
    print()

    # 3. RET jumps to the narrowed row's hit.
    app.key("RET", 1.0)
    status = app.row_text(app.rows - 1)
    verdict("RET jumped to the narrowed hit's file",
            "src/lib.rs" in status, f"status={status!r}")

    # 4. M-, returns to the results — selection restored, narrow active.
    app.key("M-,", 1.0)
    screen = app.screen_text()
    blues = app.blue_rows()
    verdict("M-, returned to the results (narrow still active)",
            "filter:" in screen and f"1 of {N_HITS} matches" in screen,
            "narrowed title + prompt row")
    verdict("M-, restored the selection",
            one_blue(app) and "target_lib" in app.row_text(blues[0]),
            f"blue={[app.row_text(r) for r in blues]}")

    # 5. C-g (job idle) CLEARS the query: full list re-derives, the
    #    selection is not lost.
    app.key("C-g", 1.0)
    screen = app.screen_text()
    blues = app.blue_rows()
    verdict("C-g cleared the query (placeholder back)",
            "type to narrow" in screen and "lib  " not in screen,
            "prompt row")
    verdict("full list re-derived (both numbers un-narrowed)",
            f"{N_HITS} matches in {N_FILES} files" in screen
            and "target_one" in screen and "target README" in screen,
            "README + main rows back")
    verdict("the selection survived the clear",
            one_blue(app) and "target_lib" in app.row_text(blues[0]),
            f"blue={[app.row_text(r) for r in blues]}")

    # 6. A fresh query ("mai" -> main.rs's 5 hits), then g re-run: the
    #    narrow query is CLEARED (a new job is a new result set). (The
    #    absence checks use the HIT LINE text — the status line shows the
    #    current buffer's which-function `(target_lib)` after step 3's
    #    jump, so the bare symbol would false-positive.)
    for ch in "mai":
        app.key(ch, settle=0.4)
    screen = app.screen_text()
    verdict("fresh query narrowed to main.rs",
            "5 of 8 matches" in screen
            and "# target README" not in screen
            and "pub fn target_lib() {}" not in screen,
            "narrowed title + rows")
    app.key("g", 1.0)
    ok, text = prompt_row(app, expect_placeholder=True)
    verdict("g re-run cleared the query (placeholder back)", ok,
            f"prompt={text!r}")
    if not wait_done(app):
        print("FAIL: re-run did not finish")
        print(app.screen_text())
        app.kill()
        sys.exit(1)
    verdict("re-run finished on the full list",
            f"{N_HITS} matches in {N_FILES} files" in app.screen_text(),
            "full-count title again")

    # 7. C-g with an EMPTY query: today's no-op echo (the C-g split's
    #    third case — nothing was cancelled, nothing cleared).
    app.key("C-g", 1.0)
    verdict("C-g on the empty query is the no-op echo",
            "nothing to cancel" in app.screen_text(), "minibuffer echo")

    # 8. q closes the view.
    app.key("q", 1.0)
    verdict("q closed the results view",
            "filter:" not in app.screen_text(),
            "prompt row gone")

    app.kill()
    print(f"\n{len(BAD) - sum(BAD)}/{len(BAD)} checks passed")
    sys.exit(1 if any(BAD) else 0)


if __name__ == "__main__":
    main()

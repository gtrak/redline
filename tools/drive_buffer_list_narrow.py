#!/usr/bin/env python3
"""Plan 018 issue 04: the buffer list (`C-x C-b`) narrows on its own prompt.

The PTY leg for the buffer-list narrowing:
  * `C-x C-b` shows the list with the narrow prompt row — ONE row,
    DECISION KEYS LEADING (`open RET · kill d · close q`), the query
    trailing (PLAN §5.2 — a clip must never hide the decision keys);
  * typing narrows the set LIVE (FilterOnly: MRU order, no re-rank);
  * RET opens the buffer the SELECTED (narrowed) row identifies;
  * `d` kills the buffer the SELECTED NARROWED row identifies (not the
    first match); the list stays open and the narrowed set re-derives;
  * C-g CLEARS THE QUERY (the list stays open — v1: clear, not close);
  * `q` closes the list.

Every navigation step asserts exactly ONE blue row (the selected-row
bar) and no reverse rows — the buffer list's one unambiguous cursor
treatment. Fixture: /tmp/redline_pyte_repo. NOTE: the query strings
deliberately avoid q/n/p/d — those are the view's own decision keys
(q closes, n/p move, d kills), so they fall through to the keymap,
not the query.

Registered in tools/gate.sh SHARED_SUITES and tools/pool.py BATTERY
(src/gate_registry.rs cross-checks both).
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo

PROMPT = "open RET"
DECISIONS = ["open RET", "kill d", "close q"]

BAD = []


def verdict(tag, ok, detail=""):
    print(f"   {'OK' if ok else 'FAIL'} {tag}: {detail}")
    BAD.append(not ok)


def one_blue(app):
    return len(app.blue_rows()) == 1 and len(app.reverse_rows()) == 0


def prompt_row_ok(app, expect_query=None, expect_placeholder=False):
    """The prompt row: decision keys leading, on the row under the title."""
    text = app.row_text(1)
    ok = PROMPT in text
    for d in DECISIONS:
        ok = ok and d in text
    if ok and expect_query is not None:
        # The query trails: it must sit to the RIGHT of the leading
        # decision keys on the same row.
        ok = expect_query in text and text.find(expect_query) > text.find(PROMPT)
    if ok and expect_placeholder:
        ok = "type to narrow" in text
    return ok, text


def main():
    app = App(repo("redline_pyte_repo"), rows=24, cols=80)

    # Open two buffers so the list has a non-trivial narrowed set:
    # lib.rs first, then sym_cjk.rs (-> MRU [sym_cjk (current), lib]).
    app.key("C-x C-f", 1.0)
    for ch in "lib":
        app.key(ch, settle=0.3)
    app.key("RET", 1.0)
    app.key("C-x C-f", 1.0)
    for ch in "sym_cjk":
        app.key(ch, settle=0.3)
    app.key("RET", 1.0)

    # The list opens with the keys-leading prompt row.
    app.key("C-x C-b", 1.0)
    ok, text = prompt_row_ok(app, expect_placeholder=True)
    verdict("prompt row (keys leading, placeholder)", ok, f"row1={text!r}")
    verdict("one-blue at open", one_blue(app), f"blue={app.blue_rows()}")

    # Typing narrows live; the non-matching row drops out.
    for ch in "cjk":
        app.key(ch, settle=0.3)
    screen = app.screen_text()
    verdict("narrowed: only the matched row", "lib.rs" not in screen,
            f"lib.rs row must be gone (screen has it: {'lib.rs' in screen})")
    ok, text = prompt_row_ok(app, expect_query="cjk")
    verdict("query trails the decision keys", ok, f"row1={text!r}")
    verdict("one-blue while narrowed", one_blue(app), f"blue={app.blue_rows()}")
    # The surviving row is the current buffer: it carries the `*` marker.
    verdict("marker slot on the narrowed row",
            any("*src/sym_cjk.rs" in app.row_text(r) for r in app.blue_rows()),
            f"blue rows: {[app.row_text(r) for r in app.blue_rows()]}")

    # RET opens the buffer the selected (narrowed) row identifies.
    app.key("RET", 1.0)
    status = app.row_text(app.rows - 1)
    verdict("RET opened the narrowed row's buffer",
            "src/sym_cjk.rs" in status, f"status={status!r}")

    # d kills the selected NARROWED row's buffer (not the first match).
    app.key("C-x C-b", 1.0)
    for ch in "li":
        app.key(ch, settle=0.3)   # narrows to lib.rs only (sym_cjk has no "li")
    # The list ROW for sym_cjk is gone (the status line still shows the
    # current buffer name — list rows carry "(N lines)", the status line
    # doesn't, so the marker is unambiguous).
    verdict("narrowed to lib.rs",
            "sym_cjk.rs (" not in app.screen_text()
            and "lib.rs" in app.screen_text(),
            f"sym_cjk row gone: {'sym_cjk.rs (' not in app.screen_text()}")
    verdict("one-blue on the narrowed row",
            one_blue(app) and "lib.rs" in app.row_text(app.blue_rows()[0]),
            f"blue={[app.row_text(r) for r in app.blue_rows()]}")
    app.key("d", 1.0)
    screen = app.screen_text()
    # List rows carry "(N lines)"; the kill echo ("killed src/lib.rs")
    # doesn't — assert the ROW is gone, not the string.
    verdict("d killed the selected narrowed row's buffer",
            "lib.rs (" not in screen, "lib.rs row must be gone")
    # The query is still 'li' and now matches nothing: the list stays OPEN
    # on the empty narrowed set (no blue row), the prompt row survives.
    ok, text = prompt_row_ok(app)
    verdict("list stays open after d (empty narrowed set)", ok, f"row1={text!r}")

    # C-g clears the query (NOT close): the full list re-derives.
    app.key("C-g", 1.0)
    ok, text = prompt_row_ok(app, expect_placeholder=True)
    verdict("C-g cleared the query, list still open", ok, f"row1={text!r}")
    verdict("full list re-derived (surviving buffer listed)",
            "sym_cjk.rs (" in app.screen_text() and one_blue(app),
            f"blue={[app.row_text(r) for r in app.blue_rows()]}")

    # A fresh query, then q closes.
    app.key("s", settle=0.3)      # 's' narrows (sym_cjk has s)
    verdict("fresh query typed", "s" in app.row_text(1), f"row1={app.row_text(1)!r}")
    app.key("q", 1.0)
    verdict("q closed the list",
            PROMPT not in app.screen_text() and "src/sym_cjk.rs" in app.row_text(app.rows - 1),
            f"status={app.row_text(app.rows - 1)!r}")

    app.kill()
    print(f"\n{len(BAD) - sum(BAD)}/{len(BAD)} checks passed")
    sys.exit(1 if any(BAD) else 0)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""U-E10: the magit status view narrows on its own section-narrow prompt.

`C-x g` opens the magit status view. U-E10 adds a section-narrow prompt on the
row under the title (`type to narrow`): printable chars re-derive the
section-structural projection at view time (the canonical tree is never
mutated by the query), the view's own keys (s/u, n/p, g, RET, C-g, q, …) fall
through to the keymap instead of the query, Backspace pops, and C-g CLEARS the
query (NOT close).

This drive gates the NEW step AND the OLD outcomes:
  * the prompt row exists at rest (placeholder + decision keys, keys leading);
  * typing narrows the section set LIVE (a query that matches only the staged
    file drops the unstaged/untracked rows);
  * a RET leg: RET on the selected NARROWED row visits that row's REAL target
    file (the projection's cursor is a real section, not a synthetic row);
  * a STAGING leg: `u` while narrowed hits the RIGHT file — the narrowed
    selection is the staged file, so unstaging it moves it to the unstaged side
    (footer `+1 ~1` -> `+0 ~2`); if narrowing had NOT positioned the cursor on
    the staged file, `u` would be a no-op and the staged count would stay +1;
  * C-g CLEARS the query (NOT close): the full list re-derives;
  * a pending-chord leg: a chord that arms a prefix (`C-x`) must NOT be
    stranded by the narrow guard — the follow-up `2` (completing bound `C-x 2`)
    reaches the engine and dead-ends to the unbound-key echo, leaving the
    query untouched (the three-surface class-bug fix, magit surface).

The query string `ri` is measured against this exact fixture baseline: it
matches the staged file `src/lib.rs` (r in `src/`, i in `lib`) — the claim
that it matches ONLY that row is false and was corrected after measurement:
the machine-local untracked strays whose headings carry an `r` before an `i`
(`src/fieldtype.rs`, `src/indented_demo.rs`) match too and legitimately
survive. The load-bearing assertion is the pair: lib.rs SURVIVES and
README.md (the other tracked file section) DROPS. The query deliberately
avoids the view's own bound keys (s/u/n/p/g/q/l/b/c/y/z/h/k), which fall
through to the keymap rather than the query.

Registered in tools/gate.sh SHARED_SUITES and tools/pool.py BATTERY
(src/gate_registry.rs cross-checks both).
"""
import os
import re
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo, reset

PROMPT = "s/u stage"
PLACEHOLDER = "type to narrow"

BAD = []


def verdict(tag, ok, detail=""):
    print(f"   {'OK' if ok else 'FAIL'} {tag}" + (f"  {detail}" if detail else ""))
    BAD.append(not ok)


def one_blue(app):
    return len(app.blue_rows()) == 1 and len(app.reverse_rows()) == 0


def file_rows(app):
    """The `M ...`/`? ...` file rows in the status body (rows 2..12)."""
    return [
        app.row_text(r).strip()
        for r in range(2, 13)
        if app.row_text(r).startswith("  M ") or app.row_text(r).startswith("  ? ")
    ]


def footer_counts(app):
    """The magit footer `+<staged> ~<unstaged> ?<untracked>` -> (staged, unstaged)."""
    f = app.row_text(app.rows - 1)
    m = re.search(r"\+(\d+) ~(\d+) \?(\d+)", f)
    return (int(m.group(1)), int(m.group(2))) if m else None


def prompt_ok(app, query=None):
    """The prompt row: decision keys leading, placeholder / query present."""
    text = app.row_text(1)
    ok = PROMPT in text
    if query is None:
        ok = ok and PLACEHOLDER in text
    else:
        # The typed query trails the leading decision keys on the prompt row.
        ok = ok and query in text and text.find(query) > text.find(PROMPT)
    return ok, text


def main():
    reset()
    app = App(repo("redline_pyte_repo"), rows=24, cols=80)

    # ── Open the magit status view ───────────────────────────────────
    app.key("C-x g", 1.0)
    verdict("magit status opened", "*magit-status*" in app.row_text(0),
            f"top={app.row_text(0)!r}")
    ok, text = prompt_ok(app)
    verdict("prompt row at rest (keys leading, placeholder)", ok, f"row1={text!r}")
    verdict("one-blue at open", one_blue(app), f"blue={app.blue_rows()}")
    verdict("baseline footer +1 ~1", footer_counts(app) == (1, 1),
            f"footer={app.row_text(app.rows - 1)!r}")

    # ── NEW step: live section narrowing ─────────────────────────────
    for ch in "ri":
        app.key(ch, settle=0.3)
    rows = file_rows(app)
    verdict("narrowed: lib.rs survives, README drops",
            any("src/lib.rs" in r for r in rows)
            and not any("README.md" in r for r in rows),
            f"rows={rows}")
    ok, text = prompt_ok(app, query="ri")
    verdict("query trails the decision keys", ok, f"row1={text!r}")
    verdict("one-blue while narrowed", one_blue(app), f"blue={app.blue_rows()}")

    # ── RET leg: the selected narrowed row's REAL target ─────────────
    app.key("RET", 1.0)
    top = app.row_text(0)
    verdict("RET visited the narrowed row's target (src/lib.rs)",
            top.strip() == "src/lib.rs" and "src/lib.rs" in app.row_text(app.rows - 1),
            f"top={top!r}")

    # ── STAGING leg: `u` while narrowed hits the RIGHT file ─────────
    # Reopen the status view (full list), narrow to the staged file again,
    # and unstage it. The staged count must drop +1 -> +0 (unstaged ~1 -> ~2):
    # this only happens if the narrowed cursor is on the STAGED lib.rs, not on
    # the already-unstaged README (which would make `u` a no-op).
    app.key("C-x g", 1.0)
    for ch in "ri":
        app.key(ch, settle=0.3)
    before = footer_counts(app)
    app.key("u", 1.0)
    after = footer_counts(app)
    verdict("unstage while narrowed: staged +1 -> +0",
            before == (1, 1) and after == (0, 2),
            f"before={before} after={after}")

    # ── C-g clears (NOT close): the full list re-derives ─────────────
    app.key("C-g", 1.0)
    rows = file_rows(app)
    ok, text = prompt_ok(app)
    verdict("C-g cleared the query, view still open", ok, f"row1={text!r}")
    verdict("full list re-derived (both file rows back)",
            any("src/lib.rs" in r for r in rows) and any("README.md" in r for r in rows),
            f"rows={rows}")
    verdict("one-blue after clear", one_blue(app), f"blue={app.blue_rows()}")

    # ── Pending-chord leg: C-x 2 reaches the engine, not the query ───
    # A chord arms a prefix (C-x has no char value, so the guard cannot
    # swallow it); the follow-up `2` completes the BOUND `C-x 2`. On the
    # pre-fix guard the `2` was silently fed to the narrow query (stranding
    # the sequence); now a non-empty pending means the key MUST reach the
    # engine, so it dead-ends to the unbound-key echo and the query stays put.
    app.key("C-x", 0.6)
    app.key("2", 0.9)
    msg = app.row_text(app.rows - 2)
    verdict("C-x 2 dead-ends to the unbound-key echo",
            "unbound key: 2" in msg, f"msg={msg!r}")
    # The query must be EMPTY (the placeholder back): checking for the
    # PREVIOUS query string (`ri`) is vacuous here — C-g already cleared it
    # two legs up, so that assertion passed even when the mutated guard
    # swallowed the `2` into the query (row1 read `…q close  2`). The
    # load-bearing property is that no query char was appended at all.
    r1 = app.row_text(1)
    verdict("the `2` was NOT swallowed into the query",
            "· type to narrow" in r1 and " 2" not in r1, f"row1={r1!r}")
    verdict("the view is still magit status",
            "*magit-status*" in app.row_text(0), f"top={app.row_text(0)!r}")

    app.kill()
    print(f"\n{len(BAD) - sum(BAD)}/{len(BAD)} checks passed")
    sys.exit(1 if any(BAD) else 0)


if __name__ == "__main__":
    main()

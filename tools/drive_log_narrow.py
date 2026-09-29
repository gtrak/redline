#!/usr/bin/env python3
"""U-E11: the log view narrows at the git-log QUERY level.

`C-x g` opens the magit status view, `l` opens the log. U-E11 adds a
query-level narrow prompt on the row under the title (`type to narrow`):
the log is SERVER-paged (a git log range fetch), so a client-side filter
of one fetched page is FORBIDDEN (it would hide commits and break `n`/`p`
— PLAN 018 §2.3-4). Instead the query is part of the git log WALK, applied
BEFORE offset/limit: `total` is the FILTERED count and every page is a
page of the filtered set. Printable chars re-walk live, the view's own
keys (n/p page, j/k motion, RET, q, C-g) fall through to the keymap,
Backspace pops, and C-g CLEARS the query (the full unfiltered log
re-derives — NOT close).

This drive gates the NEW step AND the OLD outcomes:
  * the prompt row exists at rest (placeholder + decision keys, keys
    leading);
  * typing `commit` narrows the set LIVE: the footer's total becomes the
    FILTERED count (5 of 6 — the init row drops) and adding `3`
    (`commit3`) drops it to 1;
  * a paging leg: `n`/`p` echo the FILTERED set's boundaries ("end of
    log" / "start of log") — the multi-page within-set walk is the unit
    pin `log_narrow_total_is_the_filtered_count_and_paging_walks_within_
    the_set` (the 6-commit fixture keeps the filtered set on one page);
  * Backspace pops the last query char and the total RE-DERIVES up
    (1 -> 5: the pop is a live re-walk, not a stale page);
  * a RET leg: RET on the selected NARROWED row opens that row's REAL
    commit diff (the `commit 1` commit — the only `commit1` match);
  * C-g CLEARS the query (NOT close): the full unfiltered log re-derives
    (total back to 6);
  * a pending-chord leg: a chord that arms a prefix (`C-x`) must NOT be
    stranded by the narrow guard — the follow-up `2` (completing bound
    `C-x 2`) reaches the engine and dead-ends to the unbound-key echo,
    leaving the query untouched (the class-bug leg, log surface);
  * the OLD outcome: in-page motion still walks the commits one row at
    a time (the drive_log trajectory), exactly one blue row at every
    step.

The queries are measured against this exact fixture baseline (6 commits,
subjects `commit 5..1` + `init`, author `Test`): `commit` is a
subsequence of the five `commit N` subjects and of NO other row text
(the `init` subject and the `Test` author field hold no `c…o…m…m…i…t`
chain), so the filtered count is exactly 5; `commit3` keeps exactly the
`commit 3` subject; `commit1` keeps exactly the `commit 1` subject.
The load-bearing assertions are the footer's totals: 6 -> 5 -> 1 ->
(pop) 5 -> (C-g) 6 -> 1 (`commit1`) -> (C-g) 6. Every query char is
unbound in the log view (n/p/j/k/q are the view's own keys and fall
through to the keymap rather than the query).

Registered in tools/gate.sh SHARED_SUITES and tools/pool.py BATTERY
(src/gate_registry.rs cross-checks both).
"""
import os
import re
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo, reset

PROMPT = "n/p page"
PLACEHOLDER = "type to narrow"

BAD = []


def verdict(tag, ok, detail=""):
    print(f"   {'OK' if ok else 'FAIL'} {tag}" + (f"  {detail}" if detail else ""))
    BAD.append(not ok)


def one_blue(app):
    return len(app.blue_rows()) == 1 and len(app.reverse_rows()) == 0


def commit_rows(app):
    """The log's commit rows (the windowed content, rows 2..20)."""
    out = []
    for r in range(2, 21):
        t = app.row_text(r)
        if t.startswith("##") or "n next" in t:
            continue
        m = re.match(r"^\s*([0-9a-f]{7})", t)
        if m:
            out.append(t.strip())
    return out


def footer(app):
    """The paging footer row (the `(start–end/total · …)` line)."""
    for r in range(2, 21):
        t = app.row_text(r)
        if "n next" in t and "p prev" in t:
            return t
    return ""


def footer_total(app):
    m = re.search(r"\((\d+)–(\d+)/(\d+)", footer(app))
    return (int(m.group(1)), int(m.group(2)), int(m.group(3))) if m else None


def prompt_ok(app, query=None):
    """The prompt row (row 1): decision keys leading, placeholder / query trailing."""
    text = app.row_text(1)
    ok = PROMPT in text
    if query is None:
        ok = ok and PLACEHOLDER in text
    else:
        # The typed query trails the leading decision keys on the prompt row.
        ok = ok and query in text and text.find(query) > text.find(PROMPT)
    return ok, text


def type_query(app, q):
    for ch in q:
        app.key(ch, settle=0.3)


def main():
    reset()
    app = App(repo("redline_pyte_repo"), rows=24, cols=80)

    # ── Open the log view ───────────────────────────────────────────
    app.key("C-x g", 1.0)
    app.key("l", 1.0)
    verdict("log opened", "log —" in app.row_text(0), f"top={app.row_text(0)!r}")
    ok, text = prompt_ok(app)
    verdict("prompt row at rest (keys leading, placeholder)", ok, f"row1={text!r}")
    verdict("one-blue at open", one_blue(app), f"blue={app.blue_rows()}")
    verdict("baseline footer total 6", footer_total(app) == (1, 6, 6),
            f"footer={footer(app)!r}")
    b = app.blue_rows()
    verdict("initial blue is commit 5", len(b) == 1 and "commit 5" in app.row_text(b[0]),
            f"row={app.row_text(b[0] if b else 0)!r}")

    # ── NEW step: live query-level narrowing ────────────────────────
    type_query(app, "commit")
    ft = footer_total(app)
    verdict("narrowed `commit`: footer total is the FILTERED count (6 -> 5)",
            ft == (1, 5, 5), f"footer={footer(app)!r}")
    rows = commit_rows(app)
    verdict("narrowed `commit`: the init row drops, all 5 commit rows survive",
            len(rows) == 5 and not any("init" in r for r in rows),
            f"rows={rows}")
    ok, text = prompt_ok(app, query="commit")
    verdict("query trails the decision keys", ok, f"row1={text!r}")
    verdict("one-blue while narrowed", one_blue(app), f"blue={app.blue_rows()}")

    type_query(app, "3")   # `commit3` -> only `commit 3`
    ft = footer_total(app)
    verdict("narrowed `commit3`: footer total 5 -> 1", ft == (1, 1, 1),
            f"footer={footer(app)!r}")
    b = app.blue_rows()
    verdict("the narrowed blue row is commit 3",
            len(b) == 1 and "commit 3" in app.row_text(b[0]),
            f"row={app.row_text(b[0] if b else 0)!r}")

    # ── paging leg: the FILTERED set's boundaries echo ──────────────
    app.key("n", 0.8)
    verdict("n on the 1-commit filtered set: end of log",
            "end of log" in app.row_text(app.rows - 2),
            f"msg={app.row_text(app.rows - 2)!r}")
    app.key("p", 0.8)
    verdict("p at the filtered set's top: start of log",
            "start of log" in app.row_text(app.rows - 2),
            f"msg={app.row_text(app.rows - 2)!r}")

    # ── Backspace pops the last char; the total RE-DERIVES up ───────
    # The DEL byte (0x7f) — the real Backspace keysym crossterm decodes to
    # KeyCode::Backspace (0x08 is the C-h control byte, a different key).
    os.write(app.master, b"\x7f")
    app.wait(0.7)
    ft = footer_total(app)
    verdict("Backspace popped `3`: the total re-derives 1 -> 5",
            ft == (1, 5, 5), f"footer={footer(app)!r}")
    ok, text = prompt_ok(app, query="commit")
    verdict("the prompt row shows the popped query", ok, f"row1={text!r}")

    # ── C-g clears (NOT close): the full log re-derives ─────────────
    app.key("C-g", 1.0)
    ok, text = prompt_ok(app)
    verdict("C-g cleared the query, view still open", ok, f"row1={text!r}")
    verdict("full log re-derived (footer total back to 6)",
            footer_total(app) == (1, 6, 6), f"footer={footer(app)!r}")
    verdict("one-blue after clear", one_blue(app), f"blue={app.blue_rows()}")

    # ── RET leg: the selected NARROWED row's REAL target ────────────
    type_query(app, "commit1")
    verdict("narrowed `commit1`: footer total 1", footer_total(app) == (1, 1, 1),
            f"footer={footer(app)!r}")
    app.key("RET", 1.2)
    top = app.row_text(0)
    verdict("RET opened the narrowed row's commit diff (the commit 1 commit)",
            top.strip().startswith("commit") and top.strip().endswith("commit 1"),
            f"top={top!r}")
    app.key("q", 0.8)   # back to the log (the query persists store-side)
    verdict("q returned to the log view (query still active)",
            "log —" in app.row_text(0) and footer_total(app) == (1, 1, 1),
            f"top={app.row_text(0)!r} footer={footer(app)!r}")
    app.key("C-g", 1.0)
    verdict("C-g after the RET round-trip: full log again",
            footer_total(app) == (1, 6, 6), f"footer={footer(app)!r}")

    # ── Pending-chord leg: C-x 2 reaches the engine, not the query ───
    # A chord arms a prefix (C-x has no char value, so the guard cannot
    # swallow it); the follow-up `2` completes the BOUND `C-x 2`. On the
    # pre-fix guard the `2` was silently fed to the narrow query (stranding
    # the sequence); now a non-empty pending means the key MUST reach the
    # engine, so it dead-ends to the unbound-key echo and the query stays
    # put. The load-bearing property is that no query char was appended
    # at all (asserting a PREVIOUS query's absence is vacuous after C-g).
    app.key("C-x", 0.6)
    app.key("2", 0.9)
    msg = app.row_text(app.rows - 2)
    verdict("C-x 2 dead-ends to the unbound-key echo",
            "unbound key: 2" in msg, f"msg={msg!r}")
    r1 = app.row_text(1)
    verdict("the `2` was NOT swallowed into the query",
            PLACEHOLDER in r1 and " 2" not in r1, f"row1={r1!r}")
    verdict("the view is still the log", "log —" in app.row_text(0), f"top={app.row_text(0)!r}")

    # ── OLD outcome: in-page motion still walks the commits ─────────
    expected_subjects = ["commit 4", "commit 3", "commit 2", "commit 1", "init"]
    for i, subj in enumerate(expected_subjects, start=1):
        app.key("down")
        b = app.blue_rows()
        verdict(f"down x{i}: one blue row", one_blue(app), f"blue={b}")
        if len(b) == 1:
            verdict(f"down x{i}: cursor-on-{subj!r}",
                    subj in app.row_text(b[0]), f"row={app.row_text(b[0])!r}")
    for i in range(3):
        app.key("up")
        verdict(f"up x{i+1}: one blue row", one_blue(app), f"blue={app.blue_rows()}")

    app.kill()
    print(f"\n{len(BAD) - sum(BAD)}/{len(BAD)} checks passed")
    sys.exit(1 if any(BAD) else 0)


if __name__ == "__main__":
    main()

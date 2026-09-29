#!/usr/bin/env python3
"""U-E12: the tree sidebar narrows HIERARCHICALLY (filter-children-keep-
parents — the tracker's 018 deferral verdict for this surface).

`C-c p t` toggles the ignore-aware file-tree sidebar (a 34-col left
column). U-E12 adds a narrow prompt row IN THE TREE PANE (row 1, under
the `*tree*` title — keys lead, query trails, `· type to narrow`
placeholder): typing extends the query live, a file SURVIVES iff the
shared core scores its full relative path (its name or an ancestor
directory component — typing a directory name keeps every file under
it), surviving rows keep their full indentation (a child hit is never
hidden behind a filtered-out sibling), non-matching files collapse out,
Backspace pops, and C-g CLEARS the query (the full tree re-derives; the
selection — the file, not a row index — is clamped, never lost). The
guard composes pending+key (the issue-narrow-guard-pending-prefix
discipline, fifth surface: the `2` of the bound C-x 2 family reaches the
engine, never the query). Exactly one blue row at every navigation step.

The landing ALSO re-homed the tree's windowing from the renderer to the
store (PLAN §1 row 4 — the ONE surface whose windowing lived in
`src/ui/tree.rs .skip/.take`): the store's window tracks the selection
and always contains the cursor row; the renderer draws exactly the
window it is handed (pinned by the store/UI unit pins; this drive's
row-set assertions see the windowed rows on screen).

Geometry (measured, deliberate): the prompt row went IN THE TREE PANE —
the sidebar is a left column, so its extra row shifts only the tree
column: title row 0, prompt row 1, window rows 2..=9, help row 10. The
main view's rows, the hardware cursor (the tree is not a cursor
surface), and the click COLUMN split are all untouched; the tree's own
click-ROW mapping carries the +1 (unit-pinned in
`tree_click_row_selects_visible_row_and_leaves_point_alone`).

The queries are measured against this exact fixture baseline (8 file
rows: `README.md` at the root plus the seven `src/*.rs` files — the
three TRACKED files (`README.md`, `src/lib.rs`, `src/main.rs`) plus the
machine-local untracked strays that are part of the de-facto baseline
(`fieldtype.rs`, `indented_demo.rs`, `nestedtype.rs`, `sym_cjk.rs`,
`sym_demo.rs`); the drive was first written against a WIPED fixture and
calibrated to 3 rows — the recalibration to the true 8 is the
orchestrator's, with the strays restored from a known-good root):
`lib` is a subsequence of `src/lib.rs` ONLY (fieldtype.rs holds an `l`
but no `i` after it; no other file holds an `l`); `ma` of `src/main.rs`
ONLY (`README.md` holds an `m` but never an `a` after it; the strays'
`m` is followed by `_` or `o`); `m` of README.md and of
indented_demo/main/sym_cjk/sym_demo (NOT fieldtype/lib/nestedtype);
`zz` of NOTHING. The load-bearing assertions are the rendered row sets:
8 -> (lib) 1 -> (C-g) 8 -> (ma) 1 -> (pop m) 5 -> (pop) 8 -> (zz) 0 ->
(C-g) 8, with the selection's identity (main.rs after the `ma` clamp)
surviving every re-derive — including the clear to empty and the
zero-match round-trip.

Registered in tools/gate.sh SHARED_SUITES and tools/pool.py BATTERY
(src/gate_registry.rs cross-checks both).
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pyte_driver import App
from fixture import repo, reset

KEYS = "↑/↓ · C-g clear"
PLACEHOLDER = "type to narrow"

BAD = []


def verdict(tag, ok, detail=""):
    print(f"   {'OK' if ok else 'FAIL'} {tag}" + (f"  {detail}" if detail else ""))
    BAD.append(not ok)


def one_blue(app):
    return len(app.blue_rows()) == 1 and len(app.reverse_rows()) == 0


def zero_blue(app):
    return len(app.blue_rows()) == 0 and len(app.reverse_rows()) == 0


def tree_rows(app):
    """The tree column's file rows: rows 2..=9 (title 0, prompt 1, help
    10 — the U-E12 geometry), stripped to the 34-col column."""
    out = []
    for r in range(2, 10):
        t = app.row_text(r)[:34]
        s = t.strip()
        if s and not s.startswith("RET open") and s != "toggle":
            out.append((r, s))
    return out


def prompt_ok(app, query=None):
    """The prompt row (row 1): keys leading, placeholder / query trailing."""
    text = app.row_text(1)
    if KEYS not in text:
        return False, text
    if query is None:
        return (PLACEHOLDER in text), text
    # The typed query trails the leading keys on the prompt row.
    q_at = text.find(query)
    return (q_at > 0 and q_at > text.find(KEYS)), text


def type_query(app, q):
    for ch in q:
        app.key(ch, settle=0.4)


def backspace(app):
    # The DEL byte (0x7f) — the real Backspace keysym crossterm decodes to
    # KeyCode::Backspace (0x08 is the C-h control byte, a different key).
    os.write(app.master, b"\x7f")
    app.wait(0.5)


def main():
    reset()
    app = App(repo("redline_pyte_repo"), rows=24, cols=80)

    # ── Open the tree sidebar (the home view is the key surface) ─────
    app.key("C-c p t", 1.0)
    verdict("tree visible (*tree* title row 0)",
            "*tree*" in app.row_text(0), f"row0={app.row_text(0)[:34]!r}")
    ok, text = prompt_ok(app)
    verdict("prompt row at rest (keys leading, placeholder)",
            ok, f"row1={text[:40]!r}")
    rows = tree_rows(app)
    verdict("baseline: 8 file rows at rows 2..9 (under the prompt row)",
            [r for r, _ in rows] == list(range(2, 10)), f"rows={rows}")
    verdict("baseline: one blue (README.md, the first row)",
            one_blue(app) and "README.md" in app.row_text(app.blue_rows()[0]),
            f"blue={app.blue_rows()}")

    # ── NEW: live hierarchical narrowing ─────────────────────────────
    type_query(app, "lib")
    rows = tree_rows(app)
    raw = app.row_text(2)[:34]
    verdict("narrowed `lib`: exactly one row survives (the non-matching "
            "leaves collapsed)",
            rows == [(2, "lib.rs")], f"rows={rows}")
    verdict("narrowed `lib`: the survivor keeps its depth-1 indentation "
            "(the ancestor `src` chain renders — never hidden behind the "
            "filtered-out siblings)",
            raw.startswith("    lib.rs"), f"raw={raw!r}")
    b = app.blue_rows()
    verdict("narrowed `lib`: one blue on the surviving row (selection clamped)",
            len(b) == 1 and "lib.rs" in app.row_text(b[0]), f"blue={b}")
    ok, text = prompt_ok(app, query="lib")
    verdict("the query trails the decision keys", ok, f"row1={text[:40]!r}")

    # ── C-g clears (NOT close): the full tree re-derives ─────────────
    app.key("C-g", 1.0)
    ok, text = prompt_ok(app)
    verdict("C-g cleared the query (placeholder back, view stays)",
            ok, f"row1={text[:40]!r}")
    verdict("C-g: the 'filter cleared' echo lands",
            "filter cleared" in app.row_text(app.rows - 2),
            f"msg={app.row_text(app.rows - 2)[:40]!r}")
    rows = tree_rows(app)
    verdict("C-g: the full 8-row tree re-derived",
            len(rows) == 8, f"rows={rows}")
    b = app.blue_rows()
    verdict("C-g: the selection survived the clear (lib.rs — it clamped onto "
            "the survivor while narrowed; the clear keeps its identity)",
            len(b) == 1 and "lib.rs" in app.row_text(b[0]), f"blue={b}")

    # ── Selection identity: the filter-out clamp ─────────────────────
    type_query(app, "ma")   # only src/main.rs survives (README holds no m…a)
    rows = tree_rows(app)
    verdict("narrowed `ma`: exactly main.rs survives",
            len(rows) == 1 and "main.rs" in rows[0][1], f"rows={rows}")
    b = app.blue_rows()
    verdict("narrowed `ma`: the selection clamped onto the survivor (README filtered out)",
            len(b) == 1 and "main.rs" in app.row_text(b[0]), f"blue={b}")

    # ── Backspace pops: the set RE-DERIVES up (live, not a stale window)
    backspace(app)          # `ma` -> `m`: README + the m-strays re-survive
    rows = tree_rows(app)
    verdict("Backspace popped to `m`: the set re-derives 1 -> 5 "
            "(README.md and the m-holding strays re-survive an `m`)",
            len(rows) == 5 and any("README" in s for _, s in rows)
            and any("main.rs" in s for _, s in rows), f"rows={rows}")
    b = app.blue_rows()
    verdict("Backspace: the selection stayed on its surviving file (main.rs)",
            len(b) == 1 and "main.rs" in app.row_text(b[0]), f"blue={b}")
    backspace(app)          # `m` -> ``: the full tree
    rows = tree_rows(app)
    verdict("Backspace to empty: the full 8-row tree re-derived",
            len(rows) == 8, f"rows={rows}")
    b = app.blue_rows()
    verdict("the selection's IDENTITY held across clear-to-empty (main.rs, not the top)",
            len(b) == 1 and "main.rs" in app.row_text(b[0]), f"blue={b}")

    # ── Zero matches: the honest empty state ─────────────────────────
    type_query(app, "zz")
    rows = tree_rows(app)
    verdict("zero matches: no tree rows render (honest empty state)",
            rows == [], f"rows={rows}")
    verdict("zero matches: no blue row (no row to highlight, no panic)",
            zero_blue(app), f"blue={app.blue_rows()}")
    ok, text = prompt_ok(app, query="zz")
    verdict("zero matches: the prompt row still carries the query", ok,
            f"row1={text[:40]!r}")

    # The stale-index class: the selection must not index into the
    # narrowed set — the clear restores the full list with it intact.
    app.key("C-g", 1.0)
    rows = tree_rows(app)
    verdict("C-g after zero matches: the full tree re-derived",
            len(rows) == 8, f"rows={rows}")
    b = app.blue_rows()
    verdict("C-g after zero matches: the selection survived (main.rs — clamped, not lost)",
            len(b) == 1 and "main.rs" in app.row_text(b[0]), f"blue={b}")

    # ── Pending-chord leg: C-x 2 reaches the engine, not the query ───
    # A chord arms a prefix (C-x has no char value, so the guard cannot
    # swallow it); the follow-up `2` (C-x 2 is a BUFFER-view command —
    # unbound here on the home view) must reach the engine and dead-end
    # to the unbound-key echo, leaving the query untouched. On the
    # pre-fix guard the `2` was silently fed to the narrow query
    # (stranding the sequence) — the class-bug leg, tree surface. The
    # load-bearing property: the placeholder is BACK (the query was
    # empty before C-x, so a swallowed `2` would read `2`, not the
    # placeholder — asserting a previous query's absence would be
    # vacuous).
    app.key("C-x", 0.6)
    app.key("2", 0.9)
    msg = app.row_text(app.rows - 2)
    verdict("C-x 2 dead-ends to the unbound-key echo",
            "unbound key: 2" in msg, f"msg={msg!r}")
    ok, text = prompt_ok(app)
    verdict("the `2` was NOT swallowed into the query (placeholder back)",
            ok, f"row1={text[:40]!r}")

    # ── OLD outcome: arrows still walk the files one row at a time ──
    # The selection is on main.rs (row 6 — NOT the last row: sym_demo.rs
    # is, on the true 8-file baseline). Walk DOWN to the true last row,
    # prove no wrap there, then walk back up through the real order —
    # the drive_tree discipline, exactly one blue at every step.
    for _ in range(3):
        app.key("down", 0.6)
    b = app.blue_rows()
    verdict("down x3: onto sym_demo.rs (the true last row, one blue)",
            len(b) == 1 and "sym_demo" in app.row_text(b[0]), f"blue={b}")
    app.key("down", 0.6)
    b = app.blue_rows()
    verdict("down at the last row: no wrap (still sym_demo.rs, one blue)",
            len(b) == 1 and "sym_demo" in app.row_text(b[0]), f"blue={b}")
    app.key("up", 0.6)
    b = app.blue_rows()
    verdict("up x1: onto sym_cjk.rs (one blue)",
            len(b) == 1 and "sym_cjk" in app.row_text(b[0]), f"blue={b}")
    for _ in range(2):
        app.key("up", 0.6)
    b = app.blue_rows()
    verdict("up x3 total: onto main.rs (one blue)",
            len(b) == 1 and "main.rs" in app.row_text(b[0]), f"blue={b}")
    app.key("up", 0.6)
    b = app.blue_rows()
    verdict("up x4: onto lib.rs (one blue)",
            len(b) == 1 and "lib.rs" in app.row_text(b[0]), f"blue={b}")

    # Toggle the tree off and on: the (cleared) query re-derives and the
    # full tree is back (the sidebar state round-trips through the
    # toggle, like the magit/log views round-trip through their q).
    app.key("C-c p t", 0.8)
    verdict("tree off", "*tree*" not in app.row_text(0), f"row0={app.row_text(0)[:34]!r}")
    app.key("C-c p t", 0.8)
    rows = tree_rows(app)
    ok, text = prompt_ok(app)
    verdict("tree on again: full tree, prompt at rest",
            len(rows) == 8 and ok, f"rows={rows} row1={text[:40]!r}")

    app.kill()
    print(f"\n{len(BAD) - sum(BAD)}/{len(BAD)} checks passed")
    sys.exit(1 if any(BAD) else 0)


if __name__ == "__main__":
    main()

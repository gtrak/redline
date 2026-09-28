#!/usr/bin/env python3
"""Tracker drift check: no task id may carry BOTH an OPEN and a LANDED row.

Why this exists: the tracker is the single authoritative "what is open?" record, and a
task id with two contradictory rows makes every audit wrong in both directions — it
lists landed work as open AND hides real gaps behind a stale row. Observed 2026-09-27:
seven ids carried both states, because landing rows were appended to the log instead of
flipping the queue row in place. The fix was a one-off; THIS check is what keeps it
fixed, in the same spirit as the registry-dispatch cross-check (0829ddd) — make the
drift self-diagnosing rather than rely on a periodic manual audit.

Reads the leading state token of the status cell (not a substring), so a row whose
prose happens to mention "landed" is not misread.

Exit 0 clean, 1 on any contradiction. Usage: python3 tools/check_tracker.py [path]
"""
import re
import sys
from collections import defaultdict

STATES = ("OPEN", "LANDED", "BLOCKED", "SUPERSEDED", "FIXED", "RESOLVED", "COMPLETE", "PARTIAL", "CLOSED", "SPECD")


def state_of(cell: str) -> str | None:
    """The leading state token of a status cell, ignoring markdown emphasis."""
    text = cell.replace("*", "").strip()
    for s in STATES:
        if text.upper().startswith(s):
            return s
    return None


def task_id(id_cell: str) -> str:
    """The LEADING id token of an id cell.

    Cells often carry prose after the id - e.g. `` `016-04` redo + coalescing ``
    versus `` `016-04` redo-and-coalescing (`issue-016-04-...`) ``. Reading the
    whole cell as the id makes those look like two different tasks, so a real
    contradiction silently passes. (Found the hard way 2026-09-27: 016-04 had an
    OPEN row and a LANDED row that this check did not connect.)

    The first whitespace-delimited token is the id; the rest is description. A
    deliberate ``-old`` variant is still its own token, so it stays distinct.
    """
    text = id_cell.replace("*", "").replace("`", "").strip()
    return text.split()[0] if text else ""


def main() -> int:
    path = sys.argv[1] if len(sys.argv) > 1 else ".agents/plans/STATUS.md"
    seen: dict[str, set[str]] = defaultdict(set)
    lines: dict[str, list[int]] = defaultdict(list)
    for n, line in enumerate(open(path), 1):
        if not line.startswith("| `"):
            continue
        cells = line.split("|")
        if len(cells) < 3:
            continue
        tid = task_id(cells[1])
        st = state_of(cells[2])
        if st and tid:
            seen[tid].add(st)
            lines[tid].append(n)

    bad = {t: s for t, s in seen.items() if "OPEN" in s and ("LANDED" in s or "FIXED" in s or "RESOLVED" in s)}
    if bad:
        print(f"FAIL: {len(bad)} task id(s) carry contradictory OPEN + done rows:")
        for t, s in sorted(bad.items()):
            print(f"  {t}  states={sorted(s)}  lines={lines[t]}")
        print("\nFlip the queue row in place when landing; do not append a second row.")
        return 1
    print(f"OK: {len(seen)} task ids, no id carries both OPEN and a done state.")
    return 0


if __name__ == "__main__":
    sys.exit(main())

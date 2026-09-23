# issue-menu80-ellipsis — the 80-col menu check depends on which commands are on the first screen

**Found by:** the marker-cell lane (which flagged it instead of ignoring it); **verified by me on main**
— `check_cursor_stream.py` fails `menu@80: long descriptions ellipsized` on both colour paths with
evidence `description rows with '…': 0`, while every `menu@30` check passes. It fails identically at the
lane's base, so it is not that lane's work.

## Same class as issue-flaky-menu30: the check asserts incidental content

```python
desc_truncated = [r for r in rows if "]" in r and "…" in r and r.index("…") > r.index("]")]
rec("menu@80: long descriptions ellipsized (a DESCRIPTION, not a prefix row)", len(desc_truncated) >= 1, ...)
```

It requires at least one **visible** row whose description was truncated. The menu is **windowed**, so
which descriptions are visible depends on how many commands the current view has — and the two redo
bindings were added since this last passed (the annot-symbol battery logged `description rows with '…': 2`).
Adding commands pushed the long descriptions off the first screen and the check flipped.

That is the lesson from `menu@30` repeating: a check on a *rendering property* must not depend on
which rows happen to be on screen.

## Fix direction

Assert what actually matters — *a description that did not fit carries the ellipsis marker* — without
depending on the visible set:

- ensure a **known-long description is visible** (drive to the page that contains it, or open the menu
  for a view known to have one), or
- assert **nothing is silently clipped**: every description row either fits the width or ends with `…`.

**Whichever is chosen must still FAIL if the ellipsis is dropped.** A version that cannot fail is the
vacuous-pin problem this project has already hit once — the redo-cap assertion, which reddened only at
its own precondition and never at the thing it named.

## Acceptance

- The check passes on main.
- It still fails under a mutation that removes the ellipsis from a truncated description.
- It does not depend on the command count of any particular view (adding a binding must not flip it).

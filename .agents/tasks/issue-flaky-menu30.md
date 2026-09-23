# issue-flaky-menu30 — `menu@30` is order-dependent, so it fails outside the battery

**Found by:** the `015-04` (yank) lane's battery, then reproduced by the supervisor on `main`.
**Not caused by any lane.** The yank change is not implicated: it fails identically on the base.

## The measured facts (all supervisor-run)

| context | result |
|---|---|
| `transient_menu_checks()` **alone**, fresh process | **7/7 PASS**, incl. `menu@30: long description` with evidence `[C-n] Move to the next sectio…` |
| **full `check_cursor_stream.py`** on `main` | `menu@30: long description on its own row` **FAIL** ×2 (256-color + truecolor), 176 PASS |
| the `annot-symbol` battery log | the same check **PASSED** |
| the yank lane's battery | the same check **FAILED** (lane reproduced it on the base binary too) |

**Conclusion: the check is order/context-dependent.** It passes in isolation and inside some
battery runs, and fails in others. That is a flake, and this project's rule applies — **prefer
preventing a flake over retrying through it** — because a battery that cries wolf trains everyone
to ignore failures, which is worse than no check.

## Why it fails, and why the failure looks so opaque

`_menu_rows` scans from the row containing `"Transient menu"`; if that title is absent it returns
**every** row, so the assertion's evidence string comes back **empty** — which is exactly what both
failing logs show. The assertion itself is:

```python
long_rows = [r for r in nrows if "Move to the" in r]
rec(..., len(long_rows) >= 1 and any("…" in r or "buffer" in r for r in long_rows), ...)
```

`"Move to the"` is hard-coded, and it comes from a **magit** command (`[C-n] Move to the next
section`). So the check silently requires the `C-x g` status view to be up *and* the menu to be
that view's menu. When the view is not up in time (or the state left by an earlier check changes
what opens), the menu is a different view's, no row matches, and the evidence is empty — a failure
that tells you nothing about why.

## Fix (prefer a precondition over a longer sleep)

Make the check assert its own precondition instead of hoping:

1. **Assert the magit view is up** (`*magit-status*` in the title) before opening the menu, and
   fail *that* assertion if not — so a real timing problem reports itself instead of surfacing as
   a mysterious empty-evidence menu failure.
2. **Do not hard-code `"Move to the"`.** Assert on the *longest* description row in the menu, or on
   a row that provably exists for the view under test — a fixture must contain the property that
   triggers the assertion.
3. **Reset the shared fixture inside the check** (or otherwise make it independent of whatever an
   earlier check left behind), since the full-script context is what breaks it.

## Acceptance

- `transient_menu_checks()` alone **and** the full `check_cursor_stream.py` **and** a battery run
  all agree. That is the actual bug: three contexts, three answers.
- The failure mode, if it ever recurs, names the precondition that broke rather than printing an
  empty evidence string.
- No sleep is lengthened to paper over it.

---

## ROOT CAUSE FOUND AND FIXED 2026-09-23

**The check was asserting a property the 30-column layout never promises.** Measured by dumping
the actual rows in the failing (full-script) context:

```
 Transient menu · C-g to clos…
[submenus]
C-c …
C-x …
M-s …
```

At 30 cols the menu **collapses to the top-level submenu entries** — it does not list individual
commands, so **no row carries a `[KEY] description`**, and the hard-coded magit `"Move to the"`
could never be guaranteed there. That is why it passed alone and failed in the full script.

**My first hypothesis was wrong, and the precondition I added is what disproved it.** I guessed the
`C-x g` status view was not up; the precondition assertion passed (`title='*magit-status*'`), and
the next dump showed the overlay was open too (`menu_up=True`). Both preconditions held — the
assertion was simply false at that width.

**Fixes applied:**
1. `_wait_for_menu` — **poll** for the overlay instead of the fixed `0.7s` settle (a real latent
   flake in the 80-col block as well, which passed only because 80 cols is quicker to render). No
   sleep was lengthened.
2. Assert the preconditions explicitly (status view up, overlay open) so a future timing problem
   reports *itself* rather than surfacing as an empty-evidence menu failure.
3. Assert what the narrow layout **does** guarantee — long content is truncated **with the ellipsis
   marker**, not clipped — instead of a magit command's wording. The description-ellipsis semantics
   are still asserted at 80 cols, where descriptions exist.

**Result:** full `check_cursor_stream.py` **exit 0, 0 FAIL / 180 PASS** (was exit 1, 2 FAIL /
176 PASS), with evidence `Transient menu · C-g to clos…; [submenus]; C-c …`.

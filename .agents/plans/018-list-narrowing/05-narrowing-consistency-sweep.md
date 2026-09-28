# 05 — Narrowing consistency sweep (inventory cross-check + drives + record)

**Status:** OPEN (after 02 + 03 + 04)

## Objective

Make the plan's inventory (PLAN §1) **un-driftable** and the on-screen
behaviour **drive-pinned**: (a) a cross-check test that fails when a new
list surface lands without a narrows/does-not verdict (the `0829ddd`
registry-driven/cross-checked rule applied to this plan's own inventory);
(b) the PTY drives that assert the unified behaviour end-to-end; (c) the
durable record: which surfaces stay different and WHY, and the named
follow-ups they became.

## Key decisions

- **The inventory is data, not prose.** One test (store-side, so it can read
  the surface state) enumerates every row-list surface — the 13
  `PickerKind`s (via `candidates_for`, `picker.rs:198`), the isearch list,
  the results view, the buffer list, and the declared-DIFFER set (tree, magit
  status, log, blame, home, transient menu, commit diff/editor, file view) —
  and asserts, per surface: `narrows == <the plan's verdict>` AND (for
  narrows=true surfaces) "its recompute path is the shared core" (the
  observable form: a discriminating query produces the core's ordering for
  that surface's rows — the issue-01 Pin C shape, generalized). A new
  `ViewId` or overlay owning row-list state with no declared verdict fails
  the test **and names the surface** in its failure message (the
  `type_globs_covers_all_registry_languages` failure-message rule).
- **The differ verdicts are recorded as named follow-ups, not silent
  omissions** (PLAN §4):
  - magit status: magit-native *section* narrowing (filter children, keep
    section structure; diff payload forbids row-level narrowing — PLAN
    §2.3-2). Recorded as a candidate plan, explicitly NOT this plan's.
  - log: `git log`-query-level narrowing (server-side; client-side page
    filtering is wrong, PLAN §2.3-4).
  - tree: filter-children-keep-parents in the 34-col sidebar, AFTER its
    windowing is re-homed to the store (the renderer-windowing exception,
    PLAN §1 row 4). Note its job is already served by the narrowing FindFile
    picker.
  - isearch: the nucleo filter-inside-the-list second query dimension
    (PLAN §3, deferred by explicit decision).
  Each is one paragraph in `docs/ux-testing-plan.md` (or a plan stub the
  reviewer approves) so the decision is greppable, and each is cross-
  referenced from the inventory test's comments (comment → doc, not comment →
  memory).
- **Drives own the behaviour** (PLAN §5.6): one drive per unified surface
  (isearch list, results narrowing, buffer-list narrowing) exercising
  prompt-typing → live narrowing → selection → decision key → post-decision
  state (jump/kill/open), plus the prompt-clip pins at a gate-shaped deep
  root (the `7f0090a` regime: prompt > 79 cols must still show the keys).
  Existing drives that assert the OLD isearch shape (invisible
  jump-to-match) are updated, disclosed with before/after, and made
  STRONGER (assert the list AND the old landing outcome), never loosened.
- **Tracker + docs land with this issue**: `STATUS.md` rows flip,
  `docs/ux-testing-plan.md` U-E rows get the flow IDs, and this plan's
  success criteria (PLAN § "Success criteria") are checked off in the
  landing commit message.

## Files

| File | Change |
|---|---|
| `src/app/store/tests/narrowing.rs` (new) | the inventory cross-check test + per-surface verdict pins |
| `tools/` (new/updated drives) | per-surface narrowing drives + clip pins; registered in `gate.sh` SHARED_SUITES and `pool.py` BATTERY (verified running INSIDE the battery — the `issue-annotations-symbol-precise` P2-3 rule: an unregistered driver sits outside the battery) |
| `docs/ux-testing-plan.md` | U-E flow IDs; the four named follow-ups (magit section-narrow, log git-query narrow, tree filter, isearch second dimension) |
| `.agents/plans/STATUS.md` | 018 rows flipped to LANDED with evidence (same commit as the code — the plan-process landing rule) |

## Steps

1. Write the inventory cross-check test against the landed 02/03/04 state;
   make it fail-first on a deliberately undeclared surface (a scratch
   mutation) to prove it discriminates, then green.
2. Write/refresh the drives; run the full battery (`tools/gate.sh full`,
   redirected to a file, `GATE_EXIT=$?` — never piped to `tail`).
3. Verify on MAIN after each of 02/03/04's landings already happened — this
   issue's gate is the plan-level combination check (the "two green lanes can
   still break each other" rule).
4. Record the follow-ups + flip the tracker + commit.

## Verification

- The inventory test discriminates (mutation evidence quoted).
- Battery green on main, drives registered and counted in the battery output.
- Tracker rows carry checkable evidence (a `file:line`, a test name, or a
  commit — the STATUS.md evidence rule).
- `python3 tools/check_tracker.py` passes (no task id with both an OPEN and a
  LANDED row).

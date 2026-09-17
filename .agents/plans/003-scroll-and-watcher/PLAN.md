# 003 — Redline: scrollable panes & watcher accuracy

Status: planned
Phases: 2 · Issues: 01–03

## Why

Plan 002's sweep left a prioritized backlog (docs/ux-testing-plan.md). Two
items are user-facing and requested: (1) the **watcher treats its own Access
(read) events as project changes** — a self-sustaining ~500 ms reload/
reindex loop that false-flags "changed on disk" ~0.5 s after the user types
in notes; (2) **long-content panes don't scroll** — the commit-diff
(`git show`) pane, blame view, and long editable buffers clip at the pane
edge with no way to move through them (the user asked for scrollable
`git show` panes "and anything else subject to the same issue").

## What

1. **Issue 01 — Watcher Access fix (small)**: pure-access events never
   become project changes. Drop them in the watcher's `summarize` (a batch
   of only Access events publishes nothing); keep Modify/Create/Remove
   flowing. The self-sustaining loop dies at the source; the false marker
   on locally-owned buffers disappears. `flow_g3` (which currently leans on
   the Access bug to raise the marker) is updated: the marker must come
   only from the real disk append, and typing alone must stay clean.
2. **Issue 02 — Shared windowing, applied (the scrollability sweep)**:
   generalize the issue-02 `magit_scroll` follow-scroll into one store
   helper and apply it to: commit-diff pane (scrollable with emacs motion
   keys), blame view (cursor-following window), log view (in-page selection
   never strands below the fold), editable buffers (windowed viewport,
   chord scrolling). Magit status behavior (already shipped) is untouched.
3. **Issue 03 — Sweep round & carried items**: banner copy fix ("press g to
   reload" is wrong on editable buffers), the small carried non-blockings,
   new sweep flows for the scrollable panes, full suite re-run.

## Key decisions

- Access-dropping happens at the **watcher layer** (publish nothing for
  access-only batches), not by mtime-guards in the store — the pipeline
  goes quiet instead of every consumer learning to filter. Real Modify
  events are unaffected (external edits still arrive).
- Windowing is **one helper**, not four reimplementations: offset +
  `keep_visible(cursor, len)` + clamp, exactly the shipped magit logic,
  extracted and reused. Panes without a cursor (commit-diff) get scroll
  keys instead of a cursor.
- Scroll keys follow the FileView/emacs convention already in the app
  (C-n/C-p/C-v/M-v/M-</M->); no new binding vocabulary.
- Visible insertion-point cue inside editable buffers is **deferred** (it
  is a cursor-rendering feature, not a windowing one); windowing only
  guarantees the active region stays in view.

## Success criteria

- Typing in notes for 5 s idle shows no "changed on disk" marker and no
  repeating reload batches in redline.log.
- A long commit diff, blame output, and long notes buffer are fully
  readable by scrolling; the log selection is always in view.
- All 39 existing sweep flows + the new scroll flows PASS; 14/14 overprint
  matrix clean; tests ≥ 346, 0 failed.

## Task order

| Phase | Issues | Depends on |
|---|---|---|
| 1 | 01 watcher Access fix | — |
| 1 | 02 shared windowing | — |
| 2 | 03 sweep & carried items | 01, 02 |

## Issue index

- [01 — Watcher Access fix](01-watcher-access-fix.md)
- [02 — Shared windowing helper, applied](02-shared-windowing.md)
- [03 — Sweep round & carried items](03-sweep-and-carried.md)

When complete, archive per plan-process.

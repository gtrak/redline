# 002 — Redline: magit depth & cursor polish

Status: planned
Phases: 2 · Issues: 01–03

## Why

Plan 001 shipped the full v1 surface and the first real-user sessions
confirmed the core loops. Field feedback from those sessions names the gap:
**"magit isn't fleshed out yet"** and **"I don't see a cursor"** — the git
surface works but reads thin, and cursor visibility across views is not
landing on the user's terminal. Separately, sized-PTY reconstruction caught a
renderer-layer artifact (frame overprint at layout shifts) that is very
likely the "quick black flashing" the user reported, and which no unit test
can see.

## What

1. **Magit depth (issue 01)**: bring the status buffer to daily-driver
   magit feel — inline diff hunks under files in the status buffer (not
   behind RET), a persistent section cursor with obvious highlighting,
   hunk/line-oriented verbs at the cursor, and the small quality items
   (empty-commit refusal, C-g semantics in the editor, branch-create in the
   picker).
2. **Cursor & rendering audit (issue 02)**: every navigation view shows a
   cursor the user can actually SEE on their terminal (magit rows currently
   use section_heading_selected + invert — verify against the reported
   invisibility); fix the frame overprint/flash at layout shifts
   (investigate iocraft's diff painter; candidate: full-frame clear on
   layout-change renders, or reduce repaint churn further).
3. **Sweep & re-measure (issue 03)**: UX-plan regression pass (U-* flows)
   with pyte screen reconstruction; re-record perf numbers.

## Success criteria

- The user's magit loop (glance → stage hunks → commit) feels like the
  magit subset they asked for, with a cursor they can see at all times.
- No overprint artifacts in pyte reconstructions during view switches;
  flashing gone on the user's terminal.
- All U-* flows green on the user's pass.

## Task order

| Phase | Issues | Depends on |
|---|---|---|
| 1 | 01 magit depth | — |
| 2 | 02 cursor & rendering audit | 01 |
| 3 | 03 sweep & re-measure | 02 |

## Issue index

- [01 — Magit depth: inline diffs, section cursor, editor polish](01-magit-depth.md)
- [02 — Cursor & rendering audit](02-cursor-and-rendering.md)
- [03 — Sweep & re-measure](03-sweep.md)

When complete, archive per plan-process.

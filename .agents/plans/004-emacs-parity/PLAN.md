# 004 — Redline: emacs-parity polish

Status: planned
Phases: 1 · Issues: 01–02

## Why

User directive: polish, keep testing UX, and compare redline against emacs
itself — log every difference and make decisions (docs/emacs-parity-log.md;
vanilla `emacs -Q -nw` 30.2 as the reference, NOT the user's config).
Battery 1 found one functional bug and a set of cheap parity adopts.

## What

1. **Issue 01 — isearch printable interception (bug)**: query keys that are
   bound commands in the current view (`n`, `p`, …) are dropped from the
   isearch query. Every printable must extend the query while isearch is
   active (emacs semantics). Same interception-order class as plan-002
   issue 05.
2. **Issue 02 — parity adopts, batch 1**: the PROPOSE rows that are cheap
   and additive — C-v/M-v 2-line overlap (next-screen-context-lines), line
   and percent position in the status line — plus the DEFER items
   re-verified (C-x b binding, C-l recenter) and the battery harness fix
   (the C-x b keystroke bug in drive_redline_parity.py).

## Key decisions

- Vanilla emacs defaults are the reference; the user's config is never the
  target ("don't match my setup verbatim").
- The parity log (docs/emacs-parity-log.md) is the decision record: every
  difference gets ADOPT/PROPOSE/KEEP/DEFER with rationale; PROPOSE rows
  wait for the user (13 quit save-prompt, 14 kill/yank, 15 undo are
  explicitly theirs to call).
- Results-view search (C-c p s) stays a KEEP divergence — it is the
  deliberate superpower over emacs.

## Success criteria

- isearch accepts every printable into the query (drive: `line_5` round-
  trip, `n`/`p` inside queries).
- C-v/M-v overlap 2 rows; status line shows line/percent position.
- All suites green; parity log updated with the new battery.

## Task order

| Phase | Issues | Depends on |
|---|---|---|
| 1 | 01 isearch interception | — |
| 1 | 02 parity adopts batch 1 | 01 + **user review of the PROPOSE rows** (docs/emacs-parity-log.md — user directive 2026-09-17: they review before feel-changes implement) |

## Issue index

- [01 — isearch printable interception](01-isearch-interception.md)
- [02 — parity adopts batch 1](02-parity-adopts-1.md)

When complete, archive per plan-process.

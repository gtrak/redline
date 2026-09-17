# 004 — Redline: emacs-parity polish

Status: planned
Phases: 1 · Issues: 01–05

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
2. **Issue 02 — parity adopts, batch 1** (user-approved rows 9/11):
   C-v/M-v 2-line overlap, line+percent position in the status line,
   C-x b / C-l re-verified and bound if missing.
3. **Issue 03 — mark, select, kill/yank** (user-approved row 14 with
   "mark/select"): C-SPC set-mark, region face, kill ring, yank/yank-pop —
   full in editable buffers, copy-only in read-only views.
5. **Issue 05 — cursor visibility on real terminals** (user-reported
   recurrence): show + position the hardware cursor at the selected cell
   (emacs parity); truecolor bar under COLORTERM=truecolor (palette
   remapping made the 256-color bar invisible on the user's theme).
4. **Issue 04 — quit save-prompt with buffer selection** (user-approved
   row 13 with "buffer selection"): C-x C-c enumerates locally-modified
   buffers (y/n/!/C-g) instead of silently discarding.
   Row 15 (undo) stays logged-not-implemented ("no undo yet").

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
| 1 | 02 parity adopts batch 1 | 01 (store.rs serialization) |
| 1 | 05 cursor visibility (user-reported) | 01 |
| 1 | 03 mark & kill/yank | 02 |
| 1 | 04 quit save-prompt | 03 |

## Issue index

- [01 — isearch printable interception](01-isearch-interception.md)
- [02 — parity adopts batch 1](02-parity-adopts-1.md)
- [03 — mark & kill/yank](03-mark-and-kill-yank.md)
- [04 — quit save-prompt](04-quit-save-prompt.md)
- [05 — cursor visibility on real terminals](05-cursor-visibility.md)

When complete, archive per plan-process.

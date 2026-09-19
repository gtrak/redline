# 004 — Redline: emacs-parity polish

Status: planned
Phases: 1 · Issues: 01–06

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
   buffers (y/n/!/C-g) instead of silently discarding. **DONE** (0c5dfc6,
   review PASS; 428 tests; 52/52 flows; carried P2 folded into 05d).
   Row 15 (undo) stays logged-not-implemented ("no undo yet").

### Added mid-plan (user reports + UX sweeps, 2026-09-18)

- **07** (DONE, `ea532ac`, review PASS; 547 tests): recenter after a jump —
  emacs' `xref-after-jump-hook` is `(recenter ...)` so jumps land the target
  on the MIDDLE row; we minimal-scrolled and landed below-window targets on
  the BOTTOM row (user report). `recenter_top_for` shared by `C-l` (cycle
  row) and the new jump-only `recenter_landing` (middle row, cycle untouched);
  applied at every jump landing (M-., resolver, crate landing, Xref/imenu
  selection, M-,) and NOT at goto-line/isearch/search-RET/clicks/motion.
  drive_xref +L6 (mid-window landing) → 12/12; live: ~296-line jump lands
  mid-viewport. Divergence kept deliberately: emacs does not recenter M-,
  (documented).

- **05b** (committed 220208c): file-view (line,col) point + emacs motion keys
  (C-n/C-p/C-f/C-b/arrows/C-a/C-e/M-</M->, goal column, scroll screen-row
  pinning, cursor_cell at the point). Answers the user's "cursor stuck at
  top-left / move it with the navigation keys".
- **05c** (committed 197967e + fix bd5c9a7): mouse click sets the full
  (line,col) point; wheel = window scroll; C-l = emacs `recenter-top-bottom`
  (middle→top→bottom, reset on other commands); M-f/M-b emacs word motion.
  Word motion and C-l order were found WRONG by the differential probe
  (tools/probe_emacs_diff.py) and fixed; review VERDICT PASS.
- **05d** (DONE, fc505aa, review PASS; 438 tests):
  display-width cursor/click columns + the dropped-space render bug (found
  by UX sweep); also folds the carried 004-04 review P2 (prompt redisplay
  after a failed save).
- **05e** (DONE, 95f24b1, review PASS; 442 tests):
  clicks must subtract the fixed 34-column tree sidebar offset (found by
  UX sweep: with the tree visible, a click at terminal col 40 lands the
  point at col 22 — wrong char; clicks inside the tree also move the code
  point). QUEUED after 05d (same files: root.rs/tree.rs/store.rs).
- **05f** (DONE, dbb1885, review PASS; 449 tests): menu readability
  (ellipsis + gutter + single-column narrow fallback). The review's P2
  (a weak ellipsis assertion that passed on old output) was strengthened
  in dba535c and proven non-vacuous.
- **05h** (spec: .agents/tasks/issue-004-05h-buffer-list-keys.md):
  buffer-list `n`/`p`/`d` echo unbound (inconsistent with magit/log `n`/`p`
  and the dired `d`-kill convention); found by tools/ux_sweep.py. DONE
  (679c007, review in flight; 450 tests). NOTE: the worker reported
  drive_windowing 23/23 + panes 13/13, but the harnesses actually produce
  28/28 and 4/4 — orchestrator re-ran and flagged the reporting mismatch.
- **05g** (DONE, badaf9b, review PASS; 444 tests): hardware cursor
  absolute with the tree visible (measured before: CUP col 3 over the
  sidebar; after: 37, verified live), plus the TREE_* consts moved to
  model/tree_layout (zero app→ui refs).
- **LANE ORDER REVISED (2026-09-18, orchestrator)**: the user's stated
  intent is editing files + inline annotations + a dump they can hand to an
  agent (plan 005). 004-06 (discovery home) is the largest and riskiest
  remaining 004 item (`BufferTable::new` auto-creates scratch; empty-table
  audit). So 005-01/02/03 go FIRST, then 004-06, then 006-02. Small UX-bug
  lanes 05f/05h finish first since they are already in flight/queued.
- **Battery 3/4 harnesses**: column-level emacs comparison + differential
  probes (tools/drive_*_battery3.py, tools/probe_emacs_diff{,2}.py). Probe
  #2 logged parity rows 32-34 (scroll-model divergence is NOT emacs — kept
  pending the user's call; C-d/C-u adoption; M-> resolved as a match).

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
| 1 | 06 discovery home (drop *scratch*) | 04 |

## Issue index

- [01 — isearch printable interception](01-isearch-interception.md)
- [02 — parity adopts batch 1](02-parity-adopts-1.md)
- [03 — mark & kill/yank](03-mark-and-kill-yank.md)
- [04 — quit save-prompt](04-quit-save-prompt.md)
- [05 — cursor visibility on real terminals](05-cursor-visibility.md)
- [06 — discovery home](06-discovery-home.md)
- [06a — empty buffer table + HOME view](06a-empty-table-home.md)
- [06b — remove scratch affordances](06b-remove-scratch.md)
- [07 — recenter after a jump](07-jump-recenter.md)

When complete, archive per plan-process.

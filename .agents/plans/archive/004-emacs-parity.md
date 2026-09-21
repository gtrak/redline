# 004 — Redline: emacs-parity polish (ARCHIVED)

Status: ARCHIVED 2026-09-21 — all 16 issues (01–07, 05b–05h, 06/06a/06b)
LANDED, each verified at landing (gate + the evidence citation in its
`STATUS.md` row). One caveat on the word "review": **05h**'s own record says
"review in flight" (`679c007`), so for that issue the evidence is the commit
plus its test, not a concluded review — the gate flagged the blanket claim as
overstating it.

This file is the durable record: the full original folder, verbatim —
`PLAN.md` plus all nine issue files, in plan order. No text was edited or
summarized; status lines inside the plan are as written when true (see
`.agents/plans/STATUS.md` for the current row states).

---

## PLAN.md (full text)

# 004 — Redline: emacs-parity polish

Status: complete — all 16 issues (01–07, 05b–05h, 06/06a/06b) LANDED; archive pending (per plan-process, on request)
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

## 01-isearch-interception.md (full text)

# 01 — isearch printable interception

Phase 1 · Depends on: —

## Objective

While isearch is active, every printable extends the query string — keys
bound as commands in the current view (`n`, `p`, `l`, `g`, `q`, `j`, `k`…)
must NOT be dropped or dispatched. Typing `line_5` must yield `line_5`.

## Key decisions

- Fix in the isearch interception branch of `key_event` (store.rs): it must
  run before keymap dispatch for ALL printables while the isearch prompt is
  armed — the exact ordering the notes buffer got in plan-002 issue 05.
- Non-printables keep emacs isearch semantics: C-s next match, C-r
  reverse, RET end at match, C-g cancel, DEL rubout.

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | isearch branch interception order + regression tests (query with n/p/l/g inside; count continuity). |
| `tools/drive_redline_parity.py` | fix the C-x b keystroke bug (0x62, not 0x32); add isearch-with-bound-keys legs. |

## Verification

- Gates green (build / clippy / cargo test ≥ 357, 0 failed).
- PTY: `C-s` → `line_5` full query, match count continuity `[k/120]`; `n`
  inside a query; C-r reverse; RET lands at match; C-g restores point.

## 02-parity-adopts-1.md (full text)

# 02 — parity adopts, batch 1

Phase 1 · Depends on: 01 (store.rs serialization)

## Objective

Land the user-approved rows from docs/emacs-parity-log.md battery 1:
scroll overlap, status-line position, and the re-verified DEFER rows.

## Key decisions

- User approved 2026-09-17: row 9 (C-v/M-v 2-line overlap,
  next-screen-context-lines) and row 11 (line + percent position in the
  status line).
- Re-verify and bind if missing: C-x b → buffer picker (row 6), C-l →
  recenter cycle (row 8). Verification first, binding second.
- Kill/yank, mark/select, quit save-prompt split into issues 03/04 (user
  approved with "buffer selection" and "mark/select" clarifications).
- Undo stays logged-not-implemented (row 15: "no undo yet").

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | scroll context lines (2) in page scroll; C-x b / C-l bindings if missing. |
| `src/ui/root.rs` | status-line position segment (L{n} {pct}%, emacs mode-line vocabulary). |
| `tools/drive_redline_parity.py` | fix C-x b keystroke bug (0x62); re-drive full battery. |
| `docs/emacs-parity-log.md` | flip rows 6/8/9/11 with evidence. |

## Verification

- Gates green; battery re-run both sides; overlap driven (C-v leaves 2
  rows); position segment driven; full drive suite green.

## 03-mark-and-kill-yank.md (full text)

# 03 — mark, select, kill/yank

Phase 1 · Depends on: 02

## Objective

User-approved row 14 with "mark/select" clarification: C-SPC set-mark,
region selection with movement, kill ring — emacs semantics.

## Key decisions

- **Editable buffers (notes/local files): full** — C-SPC set-mark, region
  face on movement (C-w kill region, M-w copy, C-y yank, M-y yank-pop,
  C-x C-x exchange, DEL delete region).
- **Read-only file views: copy-only** — C-SPC + region + M-w copies to the
  kill ring; yank is impossible (read-only), matching emacs read-only
  behavior.
- One shared kill ring (state in store); region state per buffer; kill
  ring default depth small (emacs 60 is fine).
- Region face: high-contrast face from the theme (region), rendered
  attribute-level in pyte drives.
- No undo (row 15 stays logged-not-implemented).

## Files

| File | Change |
|---|---|
| `src/model/buffer.rs` | mark/region state per buffer. |
| `src/app/store.rs` | kill ring state; set-mark/kill/copy/yank/yank-pop/exchange commands; region-aware motion; C-g clears region. |
| `src/app/command.rs` / keymap | bindings (C-SPC, C-w, M-w, C-y, M-y, C-x C-x). |
| `src/ui/file_view.rs` | region face render. |
| `docs/ux-testing-plan.md` | new flows for mark/kill/yank. |

## Verification

- Gates green; PTY: set-mark → region visible (attribute-level), kill
  region → text in ring, yank restores, M-y cycles, copy-from-read-view →
  yank works in notes (cross-buffer ring); C-g clears region; mark
  persists across movement; transient-menu/matrix no regressions.

## 04-quit-save-prompt.md (full text)

# 04 — quit save-prompt with buffer selection

Phase 1 · Depends on: 03 (kill/yank state in buffers)

## Objective

User-approved row 13 with "buffer selection" clarification: C-x C-c with
locally-modified buffers must not silently discard — enumerate and let the
user choose (emacs save-buffers-kill-terminal semantics).

## Key decisions

- On quit with locally-modified buffers: prompt per buffer, oldest-first:
  "Save this buffer: /path? (y, n, !, C-g)" — y saves, n skips, ! saves
  all remaining, C-g cancels the quit entirely.
- Unmodified buffers never prompt. Notes/scratch created by the app count
  as modified only when typed into (locally_modified).
- Saving a notes buffer writes the file (existing save path); after the
  last prompt, quit proceeds. If save fails, report and re-prompt.

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | quit interception → save-prompt state machine (y/n/!/C-g). |
| `src/ui/root.rs` | prompt render (minibuffer row). |
| `docs/ux-testing-plan.md` | quit-prompt flow. |

## Verification

- Gates green; PTY: modified notes + C-x C-c → prompt renders, y saves
  (file written on disk), quit exits 0; n skips (edit lost knowingly);
  ! saves all; C-g cancels quit with buffer intact; unmodified quit stays
  immediate (no regression to the 43-flow q-quit suite).

## 05-cursor-visibility.md (full text)

# 05 — cursor visibility on real terminals

Phase 1 · Depends on: 01 (store.rs serialization) · Priority: user-reported

## Objective

The user reports "the cursor was still hidden last time I checked" on their
real terminal (COLORTERM=truecolor) despite the pyte-verified 002-02 fix.
Raw-stream investigation (2026-09-17): the app emits `ESC[?25l` (hardware
cursor HIDDEN) once and never re-shows it; all CUP sequences park at
30;1. The user sees no pointer anywhere. Additionally the bar uses
256-color index 12, which user themes can remap to near-background.

## Key decisions

- **Show and position the hardware cursor** at the selected row/cell after
  each render (emacs -nw parity: the terminal cursor sits on point). The
  blanket cursor-hide must go or be countered per-render.
- **Truecolor bar when COLORTERM=truecolor** (SGR 48;2;r;g;b with the
  theme's exact RGB), 256-color index fallback otherwise — palette
  remapping can no longer hide the bar.
- Bar keeps bold; face unchanged in the theme model (only the escape
  strategy changes).

## Files

| File | Change |
|---|---|
| `src/ui/root.rs` (or render loop per iocraft skill) | post-render cursor positioning + visibility; truecolor bar escapes. |
| `src/ui/*` cursor-bar emitters | truecolor variant per theme RGB. |
| `tools/` | raw-stream check: ?25l countered, CUP lands on the selected cell, 48;2 present under COLORTERM. |

## Verification

- Gates green; raw-PTY: hardware cursor visible and positioned at the
  cursor cell after n/p movement (CUP row == selected row); no lingering
  ?25l after first render; bar truecolor under COLORTERM=truecolor and
  still correct in pyte; full drive suite green.

## 06-discovery-home.md (full text)

# 06 — discovery home (drop *scratch*)

Phase 1 · Depends on: 04 (serial on store.rs)

## Objective

User directive 2026-09-17: "I think we also need a top-level hydra menu
when no buffer is open to discover all the functionality. We don't need
*scratch*." When no buffer is open, show a discovery home — the derived
transient-menu machinery as a full top-level map — instead of an empty
*scratch* buffer.

## Key decisions

- **No default *scratch* buffer**: a fresh session (and any state where
  the buffer list is empty) renders the HOME view; it is not a buffer
  (nothing enters the buffer list/recents).
- **Derived, anti-drift**: home content is generated from the live keymap×
  registry (the transient-menu engine from 002-01) — all top-level groups
  at once: single keys, prefix groups with their members, chords. Never a
  hand-maintained string.
- Header row: project name + dirty counts + "redline"; footer: the
  standard help line (C-x C-c quits, ? opens the descendable menu).
- Opening anything (file, magit, search, notes via C-x n) replaces home.
  q unbound on home; C-x C-c quits (immediate — nothing to save when no
  buffers, per 004-04 semantics).
- Parity log: divergence from emacs splash is deliberate (KEEP) — log it.

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | drop scratch auto-create; empty-stack render state (home); menu derivation exposure for home. |
| `src/ui/` (new home view or root arm) | home render from derived groups. |
| `src/ui/transient_menu.rs` | reuse/extend the derived-menu renderer for the all-groups layout. |
| tests + tools/ | home renders derived groups (anti-drift test: registry change changes home); opening C-x C-f from home; C-x n from home; q-quit suite updated (q unbound on home). |

## Orchestrator pre-check (2026-09-18, against committed tree)

- **Derive machinery exists**: `menu_entries_for_path(&KeySeq)`,
  `menu_entries()`, `menu_rows()`, `menu_height()`, `menu_bindings()` in
  `store.rs` (~4034-4195) already turn the live keymap × command registry
  into grouped rows. Home reuses exactly this — call it with an "all
  top-level groups" query rather than a path; do not hand-write content.
- **`open_scratch` call sites** (3 real ones): command registration
  (`command.rs:154`), `kill_buffer` when the last buffer dies
  (`store.rs:2602`), a stale-reference fallback in the buffer-list open
  path (`store.rs:5411`), plus the boot path. Dropping `*scratch*` means
  those fallbacks must render HOME instead (empty stack = home), not
  create a buffer. Beware: tests call `store.open_scratch()` directly
  (`store.rs:7503`) — the helper may stay (an explicit command) while the
  *auto-create* paths change.
- **`*scratch*` is also a buffer key** (`Buffers` docs ~363) and appears
  in buffer-list expectations (e.g. `C-x b` picker "3 of 3" including
  `*scratch*` in the parity log). Those assertions must move to the home
  state (no scratch entry).
- `q` is currently bound to `quit` only in the transient/list contexts;
  on home it must be unbound (verify where the bare `q` → quit binding
  lives so home does not inherit it).

## Verification

- Gates green (test count changes: scratch-related tests updated); boot →
  home with derived groups; every listed binding works from home; q-quit
  and C-x C-c semantics per above; sweep_flows updated for the new boot
  state, 43+ flows green.

## 06a-empty-table-home.md (full text)

# 06a — empty buffer table + HOME view

Phase 1 · Depends on: 04 (serial on store.rs)

## Objective

First half of the original 004-06 (split per that spec's own instruction —
the orchestrator pre-check confirmed it is too large for one issue): the
buffer table starts EMPTY, `current` is `None`, and a new `ViewId::Home`
renders the derived-groups discovery home. `*scratch*` is no longer
auto-created; home is not a buffer.

## Key decisions

- Split boundary: **06a owns boot state + Home render**; 06b owns removing
  the remaining scratch affordances, flow key expectations, the parity-log
  splash-divergence row, and `q`-on-home rules.
- Home content is DERIVED from the live keymap × command registry (the 002-01
  transient-menu machinery) — never a hand-maintained list; an anti-drift test
  proves it.
- `SCRATCH_NAME` / explicit scratch creation remain available (06b decides the
  command's fate); only the AUTO-create at boot is removed.
- `q` is unbound on home; `C-x C-c` quits immediately (no buffers ⇒ nothing to
  save, per 004-04).

## Files

| File | Change |
|---|---|
| `src/model/buffer.rs` | `BufferTable::new()` starts empty, `current: None` |
| `src/app/store.rs` | `ViewId::Home` (+ name/keymap), all-groups menu query, boot-`None` audits |
| `src/ui/root.rs` (and/or a home module) | the Home render arm |
| `src/ui/transient_menu.rs` | reuse/extend the derived renderer |
| tests + `tools/` | anti-drift test; boot-home flow; scratch-assertion updates |

## Verification

- Anti-drift test (registry mutation changes home).
- PTY: boot renders home with real derived groups; entry points work from home
  and replace it; `q` no-op; `C-x C-c` immediate; `?` opens the menu;
  `*scratch*` absent at boot.
- `tools/gate.sh full` green with every changed count explained.

## 06b-remove-scratch.md (full text)

# 06b — remove the remaining scratch affordances

Phase 1 · Depends on: 06a

## Objective

Second half of 004-06, after 06a lands the empty table + Home view: retire the
remaining `*scratch*` affordances and update everything that assumed them.

## Key decisions

- The explicit `open-scratch` command's fate is decided here (keep as a
  non-default entry, or drop it) — state the choice and why.
- Flow key expectations, buffer-list counts, and the parity log's
  splash-divergence row (KEEP, deliberate) are updated here.
- `q`-on-home rules are confirmed/refined against emacs parity.

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | retirement of scratch entry points; `kv`/list counts |
| `tools/` flows | buffer-list/picker count expectations |
| `docs/emacs-parity-log.md` | the splash divergence row (KEEP) |

## Verification

- No `*scratch*` reachable by default; counts updated and explained.
- `tools/gate.sh full` green.

## 07-jump-recenter.md (full text)

# 07 — recenter after a jump

Phase 1 · Depends on: 05b (point/window) and 05c (C-l recenter)

## Objective

User report 2026-09-18: "when jumping to a new file, the followed entity
should not be at the bottom of the viewport."

## Key decisions

- Emacs parity reference (`emacs -Q -nw` 30.2, verified): xref sets
  `xref-after-jump-hook=(recenter xref-pulse-momentarily)` and
  `recenter-positions=(middle top bottom)` — so a jump lands the target at
  the MIDDLE of the window. Redline's landing used `keep_cursor_visible`
  (minimal scroll), which lands a below-window target on the last row.
- Applies to JUMP landings only (M-., resolver landing, Xref/imenu
  selection, M-,) — NOT goto-line, isearch, search-RET, clicks, or plain
  motion (emacs does not recenter those).
- Reuses the existing recenter arithmetic but must NOT advance
  `recenter_cycle` (a jump is not a `C-l`).

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | a jump-landing recenter helper + calls at the jump sites + tests |
| `tools/` | a PTY leg asserting the landing row is mid-window |

## Verification

- Unit tests fail against `keep_cursor_visible` (down/up/beyond-window,
  short-file clamp, C-l cycle untouched).
- `tools/gate.sh full` green with existing suites unchanged.
- Live: M-. far down a large file lands mid-viewport.

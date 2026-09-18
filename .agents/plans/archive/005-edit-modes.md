# 005 — Redline: per-buffer edit modes (file vs annotate) (ARCHIVED)

Status: implemented (3/3 issues, all review-gated to PASS; 02 took two fix
rounds — the 02b marker/canvas P1s and the 02c largest-span fill; 01/03
passed review with only P2/P3 follow-ups, resolved in-round or by the
orchestrator).

## Scope delivered

- **01 File edit mode** (b31a494, + 8e8d600 P2 resolution): `C-x C-q`
  toggles the current buffer Read-only ⇄ Edit (status line shows the
  mode), y/n confirm before a read-only flip with unsaved edits, `C-x C-s`
  save-in-place with expected-mtime self-write suppression
  (`saved_paths`), and `is_locally_owned` guards edit-mode file buffers so
  an external change cannot reload under the cursor. Notes buffers toggle
  too (`toggle-read-only` is buffer-agnostic). Verified end-to-end: typed
  bytes land on disk; `git diff` sees them.
- **02 Inline annotations** (f70fae7, fixed in 02b 0bd99ad, filled in 02c
  eb5be7c): `A` anchors an annotation to the line at point; `d` deletes
  (orchestrator ruling — `C-u` is a complete binding, numeric-prefix
  decision deferred); `C-c a` toggles visibility. Storage is a real
  editable `.redline-notes.md` with a structured `<!-- redline-annotations
  -->` section; anchors are automatic (captured line text, ±25 unique-match
  re-anchoring, explicit `ORPHANED`, never silently moved). The file view
  renders a 1-cell `▎` marker gutter plus dim note rows via a row-map
  (`file_view_rows`/`row_for_line`/`line_for_row`) that keeps the point's
  row drawn and the cursor/click coordinates consistent across four
  coordinate systems (tree offset, gutter, note rows, canvas cap).
- **02c Largest-span canvas fill** (eb5be7c): the note-budget span is the
  largest `s` with `s + notes_in_window(s) <= viewport` (monotone,
  downward scan); floor at 1 kept as the blank-view guarantee.
  All-annotated repro: 3 → 20 of 21 content rows. Backlog #14 CLOSED;
  backlog #15 opened (watcher create-events don't invalidate the file
  walk).
- **03 Agent-consumable quit dump** (a495351): on real exit intent (never
  on a C-g'd prompt), annotations print to stdout as a self-contained
  brief — `path:line` (1-based), the anchored code line (+4 indent,
  byte-exact), `NOTE:` text; orphaned records show the stored anchor with
  an explicit marker. `--notes=plain` = bare `path:line: text` (grep
  shape); unknown args hard-error (exit 2) before the TUI starts. The
  stdout reroute (`dup2` /dev/tty O_RDWR onto fd 1 before the render
  loop, dump to the saved fd) makes `redline > notes.txt` carry ZERO
  escape bytes — proven by tools/probe_notes_dump.py (17/17) whose
  zero-ESC assertions are always paired with frames-on-pty evidence.

## Test/gate footprint at archive

486 passed / 0 failed / 2 ignored; sweep 14/14; sweep_flows 65/65
(incl. ann-* and quit-prompt legs); drive_all 6/6; drive_windowing 28/28;
drive_windowing_panes 4/4; check_cursor_stream 80/80 (incl. the
all-annotated fill leg); ux_sweep 3 pre-existing findings (unbound
C-x 2/1/0); probe_notes_dump 17/17.

## Carry-forwards

- Backlog #15: watcher create-events don't invalidate the file walk
  (`ensure_files` caches per root; `C-c p i` is the manual refresh).
- P3 from the 03 review, fixed post-review by the orchestrator:
  non-unix `DumpSink::Rerouted` cfg hole, dup2-failure fd leak,
  BufWriter drop-flush silence; README plain-mode staleness caveat and
  `redline.1` OPTIONS entry added.
- Plain mode + orphaned records: a grep consumer sees no orphan marker
  (documented in README).
- The syntax-anchored annotation upgrade (anchors as syntax nodes, not
  line text) is plan 007 issue 02.

## Historical detail

The full per-round record (worker reports, reviewer verdicts, live
orchestrator verifications) is in git history and the pre-archive PLAN.md
(`git log -- .agents/plans/005-edit-modes/`).

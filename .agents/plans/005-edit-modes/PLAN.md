# 005 — Redline: per-buffer edit modes (file vs annotate)

Status: planned
Phases: 1 · Issues: 01–03
Depends on: plan 004 complete (save machinery, kill/yank, quit save-prompt)

## Why

User directive 2026-09-17: "I want 2 edit modes selectable per buffer.
file-based vs notes/annotation. Notes get dumped on quit with line numbers,
so the user can redirect to a file or copy-paste. And we can actually edit
files."

Redline is read-focused today: file buffers are read-only (auto-reload on
external change) and editing exists only in notes/scratch. This plan makes
the per-buffer edit mode explicit and adds real file editing plus a
stdout annotation dump that fits shell workflows.

## What

1. **File-edit mode (per buffer)**: file-backed buffers stay read-only by
   default; `C-x C-q` (emacs toggle-read-only muscle memory) toggles the
   current buffer into file-edit mode. While editing: the rope is
   editable, locally-modified discipline applies (external change →
   changed-on-disk marker → plan-004-04 save-prompt on quit), and
   `C-x C-s` saves to disk. Our own save must not false-flag the watcher
   (extend the event-suppression machinery to saved paths). Mode shown in
   the status line.
2. **Inline annotations (per buffer)**: `A` anchors an annotation to the
   line at point and shows it **inline in the file view** — a margin
   marker on the anchored line plus a dim virtual note line under it.
   Storage is a real, editable notes file (`.redline-notes.md`) with a
   structured annotation section; anchors are **automatic**: the captured
   line text is a drift anchor, re-anchored on open/save (±25-line
   search), flagged orphaned (never silently moved) when the content is
   gone.
3. **Quit dump (agent-consumable)**: after the TUI tears down, annotations
   print to stdout as a per-annotation block with the path, line, the
   anchored **code line**, and the `NOTE:` text — self-contained so it can
   be handed to an agent. `--notes=plain` restores `path:line: text`.
   Empty when none (clean for pipes). Dump reflects final state (after
   save prompts).

## Key decisions

- `C-x C-q` for the edit toggle (emacs read-only-toggle vocabulary, no new
  key class); `C-x C-s` for save (universal emacs muscle memory).
- Annotations are anchored records, not free text: (path, line, col,
  anchored-line text). The line text makes anchors self-maintaining;
  the dump carries the code line so an agent has context without the
  repo open.
- **Inline visibility is the point** (user directive 2026-09-18): reading
  a file shows its annotations in place; the notes buffer is the
  persistence/edit surface, not the only place you ever see them.
- Dump goes to stdout AFTER the alternate screen exits — TUI output stays
  on the terminal, dump is pipable.
- Read-only by default remains the app's identity: edit mode is explicit,
  per-buffer, and visibly indicated.
- Design details (exact key choices, dump format, notes-file section
  format) are preference-level: implemented as stated here, user can veto
  in review of the live app.

## Status (2026-09-18)

- **01 file edit mode — DONE (b31a494, review in flight; 465 tests)**: C-x C-q
  toggle (Read-only ⇄ Edit in the status line), y/n confirm before a
  read-only flip with unsaved edits, C-x C-s save-in-place with saved-path
  self-write suppression (expected-mtime, same branch as created_paths),
  and is_locally_owned now guards edit-mode file buffers so an external
  change cannot reload under the cursor. Verified end-to-end by the
  orchestrator: typed bytes landed on disk. Review PASS; P2 resolved by the
  orchestrator (notes buffers DO toggle — toggle-read-only is
  buffer-agnostic — pinned by a new test; 466 tests). Lane order note: 005 runs
  BEFORE 004-06 (the user's stated priority is editing/annotating).
- 02 inline annotations — DONE (f70fae7, review in flight; 476 tests).
  Model + tolerant storage + content re-anchoring + A/d/C-c a + the
  rendered-row map (file_view_rows with row_for_line/line_for_row).
  DELETE is `d` in the buffer view (orchestrator ruling: `C-u A` would
  preempt the user's pending C-u/numeric-prefix decision, parity row 22).
  02 review was BLOCKING: note rows overflowed the canvas (point's line
  could go undrawn; cursor one row off). Both that P1 AND the marker
  clobber were fixed in **02b (0bd99ad, review PASS; 479 tests)** and
  verified live by the orchestrator: `▎fn tall_0() {}` renders the full
  text, and at the window bottom under an annotation `tall_20` is drawn
  with the cursor on it. Also: tools now flock the shared fixture
  (8f0375e) after concurrent suites produced phantom failures.
- 02c (largest-span canvas fill, backlog #14) — **DONE (eb5be7c, review
  PASS; 479 tests)**. 005-02c and 005-02d converged into ONE delivery
  (sequencing overlap on my part; both reported the same design + commit).
  Largest-span downward scan (monotone `s + notes_in_window(s)`), floor at
  1 kept as the blank-view guarantee, point-advance + final recount/cap +
  `↑` semantics preserved byte-for-byte. All-annotated repro: 3 → 20 of 21
  content rows. Reviewer hand-traced all four legs (incl. proving s=6
  can't fit in the 22-records case) and judged all four worker deviations
  in-scope: new cursor-stream leg 5 (all-annotated fill, 80/80 total),
  `C-c p i` re-walk necessity (→ backlog #15: watcher create-events don't
  invalidate the file walk), post-leg annotation-file hygiene (second-run
  menu@30 flake fixed), reshaped unit legs with preserved properties.
  Gates re-run by orchestrator (reviewer had no shell): 479/0/2, sweep
  14/14, sweep_flows 65/65, drive_all 6/6, windowing 28/28, panes 4/4,
  cursor-stream 80/80, ux_sweep 3 pre-existing findings. Backlog #14
  CLOSED.
- 03 agent-consumable dump — QUEUED (spec corrected: the TUI owns stdout,
  so a redirect needs /dev/tty rendering).

## Success criteria

- A file buffer toggled to edit mode accepts edits and `C-x C-s` writes
  them to disk (git sees the change); toggling back to read-only restores
  auto-reload behavior.
- `A` on a line shows an inline cue (marker + note line) in the FILE
  view; quit prints the agent block; `redline --notes=plain 2>/dev/null
  | grep store.rs` works; edit the file elsewhere → annotation re-anchors
  by content.
- Full drive suite green; no regressions to the watcher conflict
  discipline or the quit save-prompt.

## Task order

| Phase | Issues | Depends on |
|---|---|---|
| 1 | 01 file edit mode | plan 004 complete |
| 1 | 02 line-anchored annotations | 01 (store/buffer state) |
| 1 | 03 quit dump | 02 |

## Issue index

- [01 — file edit mode](01-file-edit-mode.md)
- [02 — line-anchored annotations](02-line-annotations.md)
- [03 — quit dump](03-quit-dump.md)

When complete, archive per plan-process.

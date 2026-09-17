# 003 — Redline: scrollable panes & watcher accuracy (ARCHIVED)

Status: implemented (3/3 issues, all review-gated to PASS; issue 02 took
one fix round, 01/03 passed round 1).

## Scope delivered

- **01 Watcher Access fix** (b6730ab): `summarize` drops access-kind notify
  events — access-only batches publish nothing, mixed batches keep their
  real create/modify/remove events. The app's own reads no longer
  self-sustain the ~500 ms change-batch loop; locally-owned buffers no
  longer false-flag "changed on disk" ~0.5 s after typing. `flow_g3`
  reworked truthfully (marker only from a real disk append + type-then-idle
  quiet leg). 4 new tests (incl. a positive-controlled live test).
- **02 Shared windowing** (05d0797): one helper trio
  (`pane_window`/`window_slice`/`keep_cursor_visible`) extracted from the
  magit math; magit repointed byte-identically (drive_windowing 28/28 as
  the net). Applied: commit-diff pane scrolls with emacs motion keys
  (C-n/C-p/C-v/M-v/M-</M->, windowed + indicator + pinned help — the
  user's git-show request), blame cursor-following, log in-page
  keep-visible (stranding closed on reopen/paging/refresh), editable
  buffers keep the insertion row in view. 12 registry commands on the
  existing motion vocabulary (palette 71→83). New drive
  tools/drive_windowing_panes.py (4/4).
- **03 Sweep round & carried items** (fd03e39, 49abfac): banner hint per
  buffer kind (editable → `M-x reload-buffer`; plain → `g`, with the plain
  branch documented render-unreachable since plain files auto-reload);
  six carried small items closed each with a test or truthful note; sweep
  grown to 43 flows (U-CDS/U-BLW/U-NSL/U-BHN); backlog rows #5/#6 added,
  addressed rows flipped FIXED.

## Verification state at archive

357 tests / 0 failed / 2 ignored; clippy --all-targets -D warnings clean;
sweep_flows 43/43; sweep.py 14/14; drive_windowing 28/28; drive_all 6/6;
drive_windowing_panes 4/4.

## Carried forward

Backlog (docs/ux-testing-plan.md): #4 deferred trio (menu overflow "+N
more", armed-discard TOCTOU, misleading "nothing staged" error),
insertion-point cue in editable buffers, #5 search_keep_visible repoint to
the shared helper, #6 commit-editor unwindowed, #7 conflict minibuffer
message copy (store.rs:4396 — same family as #3, found by the issue-03
review).

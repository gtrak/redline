# 02 — Shared windowing helper, applied

Phase 1 · Depends on: —

## Objective

Every long-content pane is readable by scrolling, reusing ONE windowing
mechanism: commit-diff (`git show`) pane, blame view, log view (in-page),
and editable buffers. The user's ask: "magit 'git show' panes scrollable
and anything else subject to the same issue."

## Key decisions

- Extract the shipped magit logic (`magit_scroll` / `magit_keep_visible` /
  `magit_window`, store.rs) into a generic helper (offset + keep_visible +
  clamp); magit status must keep byte-identical behavior (its tests and
  drive suite are the regression net).
- Panes with a cursor get cursor-following windows (blame, log in-page);
  panes without one (commit-diff) get emacs-motion scroll keys instead
  (C-n/C-p/C-v/M-v/M-</M-> — the FileView vocabulary, no new bindings).
- Editable buffers: windowed viewport with chord scrolling; the insertion
  row is kept in view on insert. A visible insertion-point cue is DEFERRED
  (cursor-rendering feature, out of scope here).
- Window state lives in the store next to the pane state it belongs to.

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | Generic window helper; scroll state for commit-diff, blame, log window, editable buffers; keep_visible wired to their cursor/insert paths. |
| `src/ui/rows_view.rs` | Render the computed window (rows slice + optional scroll indicator), same pattern as magit_status. |
| `src/ui/file_view.rs` / commit-diff render arm | Commit-diff renders its window; motion keys adjust offset. |
| `src/app/command.rs` / keymap | Bind the motion keys for commit-diff view (emacs motion, FileView-consistent). |
| tests (store + structural) | Per-pane: window bounded, cursor/insert row in view, top advances/clamps; magit windowing test unchanged and green. |

## Steps

1. Extract helper; repoint magit to it (no behavior change; magit tests green).
2. Commit-diff: window + motion keys + structural test.
3. Blame: cursor-following window + test.
4. Log: in-page keep_visible (selection never strands) + test.
5. Editable buffers: windowed viewport, chord scroll, insert keeps row visible + test.

## Verification

- Gates green (build / clippy / cargo test, ≥ 346, 0 failed).
- New pyte drives in tools/: commit-diff scroll through a >viewport diff
  (M-> lands on last row, M-< round-trips); blame cursor stays in window
  across a long file; log selection in-window after many in-page moves;
  notes typing near the bottom keeps the active region in view.
- `tools/sweep.py` 14/14 unchanged; magit drives unchanged.

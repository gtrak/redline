# 02 — parity adopts, batch 1

Phase 1 · Depends on: 01 (store.rs serialization)

## Objective

Land the cheap ADOPT/PROPOSE-default rows from docs/emacs-parity-log.md and
re-verify the DEFER rows.

## Key decisions

- In scope: C-v/M-v 2-line overlap (next-screen-context-lines), line+percent
  position in the status line, C-x b bound to the buffer picker (if
  unbound), C-l recenter (bind/fix if missing), parity-log updates.
- Out of scope (user's call, logged): quit save-prompt (#13), kill/yank
  (#14), undo (#15).

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | scroll context lines; status-line position segment; C-x b / C-l if missing. |
| `src/ui/root.rs` | status-line render segment. |
| `tools/drive_redline_parity.py` | re-drive the full battery; parity log row updates. |
| `docs/emacs-parity-log.md` | flip rows 8/9/11/6 to decided with evidence. |

## Verification

- Gates green; battery re-run both sides; drive suite all green.

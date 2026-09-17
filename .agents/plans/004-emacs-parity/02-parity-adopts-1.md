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

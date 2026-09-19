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

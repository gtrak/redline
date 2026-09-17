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

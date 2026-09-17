# Task: plan 004 issue 05b — file-view point (line, col) + emacs motion keys

You are the implementation worker. Repo root is your cwd. This spec is
self-contained. `.agents/skills/*.md` are authoritative ground truth.

Context: 004-05 (committed 51badab) made the hardware cursor visible.
USER REPORT (2026-09-17): "I do see the cursor now at the top left, but I
need to be able to move the cursor with the regular navigation keys" —
then, explicitly: "**arrow keys, C-f and C-b as well**."

Two root causes:
1. `cursor_cell` (src/ui/root.rs) anchors the FILE VIEW cursor to the top
   visible line, not the point — the cursor sits top-left and doesn't
   track navigation.
2. The file view's point is LINE-BASED only (no column), and the arrow
   keys are bound to window scrolling (plan-001 "PART A item 4" stopgap);
   C-f/C-b/Left/Right are unbound in the file view. There is no char/col
   motion at all.

## Working agreement

- Skills are truth (ropey, iocraft, emacs-ux; no registry reads, no
  docs.rs, no fetch). Write-first. Skill corrections minimal, listed.
  graft available.

## Read first

1. `src/app/store.rs` — file-view point state (which-function line),
   Buffer keymap (scroll bindings at ~98-110: j/k/C-v/M-v/PgUp/PgDn/C-d/
   C-u + arrows bound to SCROLL), isearch landing, goto-line, region mark
   (byte offsets), `buffer_keep_insert_visible`.
2. `src/ui/root.rs` — cursor_cell + the deferred cursor task (004-05).
3. `.agents/skills/emacs-ux/SKILL.md` — point/motion semantics (C-f/C-b
   wrap at line boundaries, goal-column across lines, C-a/C-e).
4. `tools/check_cursor_stream.py` — extend with the file-view leg.

## What to build

1. **Point becomes (line, col)** for file-backed buffers (read-only
   included). Col clamps to the line's length at EOL (emacs clamps; no
   horizontal scrolling in this issue — keep col within the visible
   width; document what happens at the pane edge).
2. **Motion commands + bindings in the file view**:
   - `C-n`/Down → point down (goal-column: keep col across lines, clamped
     to each line's length — emacs goal-column semantics).
   - `C-p`/Up → point up (same).
   - `C-f`/Right → point forward char, wrapping to the next line at EOL.
   - `C-b`/Left → point backward char, wrapping to the previous line end
     at BOL.
   - `C-a`/`C-e` → line start / line end.
   - `M-<`/`M->` → point to buffer start/end (window follows; currently
     scroll-only — rebind to move the point).
   - The window follows the point (keep the point's row in the viewport —
     the shipped follow-scroll pattern).
   - **REBIND the arrows**: Up/Down move the point now (not window
     scroll). This deliberately supersedes the plan-001 item-4 stopgap —
     the user directive is explicit. Left/Right join as point motion.
3. **Scroll keeps emacs semantics**: `C-v`/`M-v`/`C-d`/`C-u`/PgUp/PgDn
   move the WINDOW; the point's SCREEN ROW stays fixed (recompute the
   point's buffer line from its screen position after the window moves —
   emacs behavior). `j`/`k` stay window-scroll (non-emacs keys).
4. **cursor_cell** (root.rs): file-view arm anchors on the point:
   row = point_line − scroll_top + title/banner offset; col = point col.
   (List-view arms are already correct; unchanged.)
5. **Integrations must hold**: isearch landing lands the point (line —
   col 0); goto-line lands the point; which-function tracks point line;
   the region mark stays at line starts (region semantics unchanged —
   document that point col does not extend the region); editable buffers
   (notes) are OUT OF SCOPE: append-at-end stays (their cursor cell is
   the insertion row, unchanged); tree/magit/log/blame arms unchanged.

## Constraints

- Render + store point state + bindings; no dependency changes; no notes
  editing changes; no undo (row 15 stands). Scope fence: src/app/store.rs
  (point state, motion commands, bindings, snapshot fields),
  src/app/command.rs (new motion commands registered, palette-count tests
  updated), src/ui/root.rs (cursor_cell + snapshot plumbing),
  tools/check_cursor_stream.py (+ file-view leg). Nothing else.

## Verification (iterate until ALL pass)

- Gates: build / clippy -D warnings / cargo test (396+ / 2 ignored) green.
- Suites: sweep.py 14/14; sweep_flows.py 46/46 (U-M flows included);
  drive_all 6/6; drive_windowing 28/28; drive_windowing_panes 4/4;
  check_cursor_stream extended all-pass.
- Raw-PTY file-view leg (new in check_cursor_stream.py): cursor (CUP
  row AND column) tracks C-n/C-p/C-f/C-b/arrows/C-a/C-e step by step;
  EOL/BOL wrapping verified; goal-column across short/long lines; C-v
  keeps the point's screen row; M->/M-< land point at end/start with the
  window following; which-function matches the point line.
- Coverage split in the report.

## Report format

- Point state design (where (line,col) lives, goal-column handling,
  scroll/point interaction). Per-command table (binding → command →
  semantics). Gate outputs (exact counts). Skill corrections (or none).
  Deviations; known gaps.

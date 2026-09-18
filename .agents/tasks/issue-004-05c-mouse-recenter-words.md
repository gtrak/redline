# Task: plan 004 issue 05c — mouse click column, wheel parity, C-l recenter, word motion

You are the implementation worker. Repo root is your cwd. This spec is
self-contained. `.agents/skills/*.md` are authoritative ground truth.

User reports (2026-09-18), after 05b gave the file view a (line,col) point:
1. "clicks set the line, but they don't set the point" — `mouse_click_position`
   maps the click row to a line but always lands col 0.
2. "I also want to support mouse navigation" — wheel/click navigation
   should behave like the keyboard model.
3. "C-l doesn't quite work right" — diagnosed live: `recenter()` cycles the
   WINDOW through buffer thirds while the point stays put, so the window
   scrolls away from the point (observed: point line 20, window top 47),
   the cursor is clamped to row 1, and the next C-n snaps.
4. "M-b M-f doesn't work" — word motion is unimplemented (deliberately out
   of 05b's fence; now directed).

## Working agreement

- Skills are truth (emacs-ux SKILL.md for motion/recenter/mouse semantics;
  iocraft SKILL.md for render; no registry reads, no docs.rs, no fetch).
  Write-first; compile early; iterate on named errors. Minimal skill
  corrections, listed. `graft` available.

## Read first

1. `src/app/store.rs` — 05b's point state (`point_line`/`point_col`/
   `set_point`/`set_point_line`, `scroll_window_point`), `recenter()`
   (~2889), `mouse_scroll_up`/`mouse_scroll_down` (~2808/2831),
   `mouse_click_position` (~2857), the C-v scroll-holds-screen-row helper
   added by 05b, goal-column handling.
2. `src/ui/root.rs` — the mouse event arm (~270-290: `ScrollUp`,
   `ScrollDown`, `Down(Left)`; note only `mouse.row` is passed today).
3. `src/ui/file_view.rs` — text renders from column 0 (no gutter), so a
   terminal column maps 1:1 to a char index in the line.
4. `.agents/skills/emacs-ux/SKILL.md` — `recenter-top-bottom` and word
   motion semantics.

## What to build

### 1. Click sets the point (line AND col)
- `mouse_click_position(row, col)`: set the point to the clicked
  (line, col) — `col` is the terminal column, which equals the char index
  (no gutter; rendering is `chars()`-based like the rest of the app).
  Clamp `col` to the line's char length (clicking past EOL lands at EOL).
- `src/ui/root.rs`: pass `mouse.col` through (keep the existing row
  offset). Keep the window following (05b behavior).
- Click-to-select in list-ish views is a welcome extra if cheap and
  consistent (magit/log/blame/tree/buffer-list/search → move the
  selection to the clicked row); do NOT open on click. If it needs more
  than a small amount of plumbing, skip it and note the gap.

### 2. Wheel parity with the keyboard scroll model
- File view: the wheel is a WINDOW scroll with the point's SCREEN ROW
  pinned (emacs `mwheel-scroll`) — i.e. the same primitive C-v/M-v use
  after 05b, with a step of 3 lines. It must NOT drag the point line along
  (the pre-05b `scroll_window_point` behavior). Verify the point's buffer
  line advances only because the window moved under it.
- List-ish views: keep the existing wheel behavior (moves the selection) —
  that is correct for a cursor list. No change.

### 3. `C-l` = emacs `recenter-top-bottom`
- The point does NOT move. The WINDOW repositions so the point's screen
  row cycles top → middle → bottom → top within the viewport:
  desired_row ∈ {0, viewport/2, viewport−1}; `scroll_top = point_line −
  desired_row`, clamped to [0, max_scroll]. Tiny viewports must still
  cycle without dead-ends (keep the existing degenerate-range protection;
  the current `(max_scroll/3).max(1)` regression test family must be
  reworked to the new contract, not deleted).
- Update the doc comment (the old one describes the retired
  cursor-is-window-top model).

### 4. Word motion `M-f` / `M-b`
- Point moves by word in the file view: forward-word to the end of the
  next word, backward-word to the start of the previous word, emacs-style
  (wraps across line boundaries; word = run of alphanumeric/underscore,
  non-word = punctuation/whitespace runs — document the exact rule and
  note emacs syntax-table differences).
- Update `goal_col` consistently (a word motion sets the goal column to
  the landing column, as emacs does).
- Bindings: `M-f` / `M-b` in the file-view keymap. Add the commands to
  `src/app/command.rs` (palette count tests updated).

## Constraints

- Scope fence: `src/app/store.rs`, `src/app/command.rs`, `src/ui/root.rs`
  (mouse col plumbing + any list click-select), tests, `tools/` flows,
  `docs/`. No dependency changes; no undo; no editing-model changes
  (notes/scratch stay append-at-end); do not touch kill/yank, isearch,
  region, or the quit machinery.
- All suites green: `cargo test`, `sweep.py` 14/14, `sweep_flows.py`
  46/46, `drive_all` 6/6, `drive_windowing` 28/28,
  `drive_windowing_panes` 4/4, `check_cursor_stream` all.

## Verification (iterate until ALL pass)

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test green.
- Unit tests: click (line,col) mapping incl. past-EOL clamp and empty
  lines; word motion forward/backward incl. wrap, punctuation runs, and
  goal-col; recenter-top-bottom cycle (point line unchanged, window top
  moves, tiny-viewport cycle terminates).
- Raw-PTY legs: click at a non-zero column → CUP column equals it; wheel
  → window scrolls while the cursor's screen row is pinned (assert both
  the top line changed AND the point line advanced under the window);
  `C-l` ×3 → point line fixed, point screen row 1 → mid → last → 1; `M-f`
  /`M-b` → cursor column lands on word boundaries (assert exact columns).
- Keep the existing 46 flows green.

## Report format

Design notes per item (1-4). Per-command/binding table. Gate outputs
(exact counts). Skill corrections (or none). Deviations; known gaps.

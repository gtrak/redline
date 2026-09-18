# Task: plan 004 issue 05d — display-width-correct cursor and click columns

You are the implementation worker. Repo root is your cwd. This spec is
self-contained.

## Reported defect (orchestrator UX sweep, 2026-09-18)

With wide (CJK) characters in a file, the terminal cursor and mouse-click
mapping use the **char index** where the terminal needs the **display
column**. Reproduced live (pty/pyte):

- File line `abcd中efgh` (chars: a,b,c,d,中,e,f,g,h; display width 10 since
  中 is 2 cells). After `C-a` + 13× `C-f` the point is on char index 13 in
  `CJK: abcd中 efgh`; the hardware cursor lands at column **13**, but the
  glyph occupies display column 14 (one extra cell for 中). The cursor is
  therefore drawn one cell left of the intended character (and worsens
  with more wide chars).
- The same arithmetic underlies `mouse_click_position(row, col)`: the
  clicked terminal column is treated as a char index, so clicks after a
  wide char land on the wrong character.
- Rendering itself is CORRECT (the canvas lays wide glyphs out by display
  width — verified: pyte shows `fn 中() { let x = 1; }` aligned properly).
  This is purely the point↔screen column conversion.

**Second, CONFIRMED bug (same root cause).** `fn über() { let 中 = 1; }`
renders as `fn über() { let 中 =1; }` — the space between `=` and `1` is
genuinely dropped in pyte's `display` output (not a buffer artifact).
Root cause established from the raw escape stream: the app DOES emit the
space (`...let 中 = 1; }` appears in the byte stream). `draw_line` places
each syntax segment at an absolute x and advances `x += segment.chars()
.count()`; a segment ending in a wide char advances x by 1 while the
terminal advanced 2, so the NEXT segment is drawn one cell early and
clobbers a cell. Fix = the same display-width x-advance as item 1 (the
render must be byte-faithful for all text).

## Read first

1. `src/ui/file_view.rs` — `draw_line`: it walks segments and advances
   `x += segment.chars().count()`; `truncate()` counts chars. Both need
   display-width awareness for any x-advance that positions the cursor or
   clips at the pane edge. (Do NOT change the text content emitted — only
   the column arithmetic.)
2. `src/ui/root.rs` — `cursor_cell`: the Buffer arm sets
   `col = point_col` (a char index). Needs a char-index → display-column
   conversion for the point's line prefix.
3. `src/app/store.rs` — `mouse_click_position(row, col)` (display column
   → char index, the inverse), and `line_char_len` (char count — correct
   for point clamping; do not conflate with display width).
4. `.agents/skills/iocraft/SKILL.md` — canvas `set_text` semantics.

## What to build

1. A single helper for char-index ↔ display-column conversion on a line
   (prefix width). `unicode-width` is already in the tree as a transitive
   dep of iocraft (`unicode-width v0.1.14`); adding it as a direct
   dependency in `Cargo.toml` is ALLOWED for this issue (justify it; it is
   the standard crate for this). Handle combining marks (width 0) and wide
   (2) — `UnicodeWidthChar::width()` returns Option; treat None as 1.
2. `cursor_cell`: display column = width of the line prefix [0, point_col).
   Combining-mark positions (width 0) are acceptable to land on the same
   column as their base.
3. `mouse_click_position`: convert the clicked display column back to the
   nearest char index (a click inside a wide char maps to that char).
4. `file_view.rs` x-advance: use display width wherever x accumulates, so
   truncation at the pane edge and any overlay x-positions stay aligned.
5. Tests: unit for the conversion helper (ASCII, wide, combining, mixed);
   cursor-cell tests asserting display columns; a PTY leg with a wide-char
   fixture asserting the CUP column is the DISPLAY column (the current
   check_cursor_stream file-view legs use ASCII — add a wide variant).

## Constraints

- Scope fence: `src/ui/file_view.rs`, `src/ui/root.rs`,
  `src/app/store.rs` (click mapping only), `Cargo.toml` + `Cargo.lock`
  (the one allowed dependency add), tests, `tools/check_cursor_stream.py`.
  Do not change point motion semantics (char-index based) — only the
  index↔column conversions at the render/input boundary.
- All suites green: `cargo test`, `tools/sweep.py`, `tools/sweep_flows.py`,
  `tools/drive_all.py`, `tools/drive_windowing.py`,
  `tools/drive_windowing_panes.py`, `tools/check_cursor_stream.py`.
- Wrap EVERY python PTY invocation in `timeout`.

## Verification

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test green.
- The live repro above must be fixed: cursor on char index 13 of
  `CJK: abcd中 efgh` sits at display column 14; clicking display column 14
  lands on that char.
- Report: helper design, the dependency justification, before/after column
  numbers from the repro, gate counts.

## Report format

Design (helper API + width rules). Before/after repro numbers. Gate
outputs (exact counts). Skill corrections (or none). Deviations.

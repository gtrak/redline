# Emacs parity log — every difference, with decisions

Method: vanilla **GNU Emacs 30.2** (`emacs -Q -nw`, no init — defaults are
the reference, NOT the user's config) driven through the same pty+pyte
harness as redline, 100×30, on identical fixture repos. Harness committed:
`tools/drive_emacs.py` (reference battery) and
`tools/drive_redline_parity.py` (mirrored battery). Captures:
`/tmp/emacs_parity_capture.txt`, `/tmp/redline_parity_capture.txt`.

Decision vocabulary — every difference gets one verdict:
- **ADOPT** — match emacs (bug fix or clear improvement; I implement).
- **PROPOSE** — behavior change that alters feel; user decides.
- **KEEP** — divergent by design; rationale recorded.
- **DEFER** — real but not now; backlog row.

This log is append-only: new comparisons append rows; decisions never get
rewritten (a changed mind adds a new row).

## Battery 1 — 2026-09-17 (vanilla emacs 30.2 vs redline @ 713851f)

| # | Behavior | emacs 30.2 (-Q) | redline | Verdict |
|---|---|---|---|---|
| 1 | **isearch query interception** | every printable extends the search string; `n`/`p` etc. are just letters | **BUG**: query keys bound in the current view are dropped — typing `line_5` yields `I-search: lie_5` (the `n` vanishes; reproduced: `l` → `l [1/120]` ok, then `ine_5` → `lie_5`). Same class as plan-002 issue 05's editable-buffer bug, in the isearch branch. | **ADOPT** (bug fix; issue 01 of plan 004) |
| 2 | isearch prompt | `I-search: query` in the echo area; failing search says `[no matches]`-style feedback | same shape: `I-search: lie_5 [no matches]` in the minibuffer row; match count `l [1/120]` | KEEP (already at parity in shape; #1 fixes the content) |
| 3 | isearch cancel | C-g aborts, returns to original point, echo `Quit` | C-g aborts to original point, echo `cancel` | KEEP (wording differs; same semantics) |
| 4 | isearch model | inline: point jumps match-to-match in the buffer, highlight in place | file-view C-s is inline isearch with match count; project search (C-c p s) streams to a results view | KEEP (results view is the deliberate superpower; inline path stays emacs-like) |
| 5 | find-file | `C-x C-f` → minibuffer prompt with TAB completion | `C-x C-f` → picker overlay with fuzzy query | KEEP (picker is deliberate) |
| 6 | buffer switch | `C-x b` → minibuffer prompt; `C-x C-b` → Buffer List window | buffer list picker (C-x C-b); C-x b unbound (battery sent C-x 2 by harness bug — re-verify C-x b next battery) | DEFER (re-verify; consider binding C-x b to the buffer picker for muscle memory) |
| 7 | goto line | `M-g g 30` → buffer line 30, `Mark set` | `M-g g 30` → buffer line 30 (renders line_16 region — correct line-number semantics) | KEEP (at parity) |
| 8 | recenter | `C-l` recenters point (top/middle/bottom cycle) | C-l: no visible change in battery frame (binding unverified) | DEFER (verify binding; ADOPT if missing — standard muscle memory) |
| 9 | scroll page | `C-v`/`M-v` leave `next-screen-context-lines` (2) rows of overlap | C-v appears to land a full page with no overlap | PROPOSE (2-line overlap is cheap and reduces disorientation) |
| 10 | windows | `C-x 2` split, `C-x o` other-window, `C-x 1` | no window model; tree sidebar + single content pane; C-x 2 / C-x o unbound ("unbound key: C-o") | KEEP (deliberate architecture; the tree is the second pane) |
| 11 | mode line | menu bar + mode line: flags, buffer name, position (Top/L1/All), mode, git branch | status line: project, dirty counts, buffer, which-function; no position-% | PROPOSE (add line/percent position to status line — emacs muscle memory reads it constantly) |
| 12 | M-x | echo-area prompt with completion | picker overlay with query | KEEP |
| 13 | quit with unsaved buffers | `C-x C-c` prompts to save each modified buffer | `C-x C-c` quits immediately — unsaved notes edits are lost silently | **PROPOSE — needs user decision** (emacs's save-prompt is a data-loss guard; redline currently silent) |
| 14 | kill/yank | C-w/M-w/C-y/M-y everywhere | none (read-focused app; notes accept typing but no kill ring) | PROPOSE (kill ring inside notes editing at least; full buffer kill/yank is a bigger read-view question) |
| 15 | undo | C-/ / C-x u universal | none in notes editing | PROPOSE (undo in notes; cheap with ropey? verify) |
| 16 | echo of unbound keys | emacs: `C-x 2` is bound; unknown keys self-insert or beep | minibuffer echo `unbound key: C-o` | KEEP (honest and better than emacs's bell) |
| 17 | dirty indicators | `**` modified flag in mode line | `*` dirty counts in status line | KEEP |
| 18 | which-function | separate echo area / mode-line imenu entry | which-function in status line (verified in battery: `(line_48)`) | KEEP (already better: always visible) |

| 19 | no-buffer state | emacs -Q shows *scratch* (+ splash) | **discovery home**: derived keymap×registry menu replaces *scratch* entirely (user directive 2026-09-17) | KEEP (deliberate divergence: discovery over scratch; emacs muscle memory bindings unchanged) |
## Findings vs battery notes

- The isearch bug (#1) is the only functional defect found in battery 1.
- Harness bug on our side: the battery's `C-x b` step actually sent `C-x 2`
  (0x32 vs 0x62) — C-x b untested; row 6 stays DEFER until re-driven.

## Open questions for the user (PROPOSE rows)

9 (C-v overlap), 11 (position %), 13 (quit save-prompt), 14 (kill/yank
scope), 15 (undo in notes). **User decided 2026-09-17: 9 YES, 11 YES,
13 YES with buffer selection ("I guess we need buffer selection?"),
14 YES with mark/select, 15 NO ("no undo yet" — stays logged).**
Implementation: rows 9/11 → plan-004 issue 02; row 14+select → issue 03;
row 13+selection → issue 04. Only clear BUGS go forward without review
(row 1, isearch — plan-004 issue 01). DEFER rows 6/8 get re-verified in
the next battery regardless (verification, not decision).

## Battery 1 follow-up — 2026-09-17 (plan-004 issue 02: parity adopts, batch 1)

| # | Row | Disposition | Evidence |
|---|---|---|---|
| 6 | C-x b buffer switch | **VERIFIED — already bound** | `C-x b` → `switch-buffer` picker (binding present at store.rs global keymap; test `issue_02_bindings_resolve_including_c_c_p_prefix` asserts `expect("C-x b", "switch-buffer")`; battery step drives it: picker opens showing README.md, src/main.rs, *scratch* (3 of 3)). No new code needed. |
| 8 | C-l recenter cycle | **ADOPT (implemented)** | New `recenter` command registered (motion category); `C-l` bound in the Buffer view keymap. Cycle: top zone → middle (`max_scroll/2`) → bottom (`max_scroll`) → top (`0`). Zones determined by thirds of the scroll range. Unit tests: `recenter_cycles_top_to_middle`, `recenter_cycles_middle_to_bottom`, `recenter_cycles_bottom_to_top`, `recenter_full_cycle_returns_to_start`, `recenter_noop_when_buffer_fits_viewport`. Battery leg (121-line fixture, viewport=27): from Bot, C-l→Top, C-l→Middle (`L48,39%`), C-l→Bottom (`Bot`). |
| 9 | C-v/M-v 2-line overlap | **VERIFIED — already implemented** | `scroll_page_down`/`scroll_page_up` use `step = viewport - 2` (since plan-002 issue 03, "PART A fix item 5"). Unit tests: `scroll_page_down_keeps_two_line_overlap` (viewport=10, 100 lines: step=8, not 10). Battery legs (121-line fixture, viewport=27): C-v from `L6,4%` to `L31,25%` (step=25=viewport−2); M-v returns to `L6,4%` (step=25=viewport−2). 2-row overlap confirmed. |
| 11 | Status-line position | **ADOPT (implemented)** | New `file_view_position_display()` on the store; format: `Top` at line 1, `Bot` when the window shows the buffer end (`scroll_top + viewport >= total`), otherwise `L{n},{pct}%` (1-based line, integer percent through the buffer, round-half-up). Appended to the status line after the searching indicator. Updates on every scroll/cursor movement (the status line re-renders every tick). Unit tests: `position_display_top`, `position_display_bot`, `position_display_middle`, `position_display_single_line_buffer`, `position_display_empty_buffer`. Battery legs (121-line fixture): after M-< shows `Top`, after C-n×5 shows `L6,4%`, after M-> shows `Bot`. |

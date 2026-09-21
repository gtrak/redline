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
| 20 | word motion | `M-f`/`M-b` move by word (column moves) | unverified at column level in this battery (frames don't show column) | re-verify battery 3 |
| 21 | kill-line / kill-word | `C-k` / `M-d` in editable text | unbound in read-only views (expected); notes have no kill-line/kill-word (single-char backspace only) | PROPOSE (kill-line/kill-word in editable buffers, on the 004-03 ring machinery) |
| 22 | numeric prefix | `C-u 3 C-n` / `M-5 C-n` repeat 3/5 | none; **`C-u` is bound to half-page scroll** (deliberate but conflicting — adopting numeric args needs a different home for half-page) | PROPOSE (user decision: real `C-u` args vs keep half-page) |
| 23 | registers | `C-x r s a` / `C-x r i a` prompt + restore | none | DEFER (kill ring covers the common case) |
| 24 | query-replace | `M-% line RET rope RET !` rewrote the buffer | none | PROPOSE (in-file replace once file-edit mode lands, plan 005) |
| 25 | word search | `M-s w` | isearch (substring) + project search cover it | KEEP |
| 26 | dired | `C-x d` directory editor (flag-based file ops) | tree sidebar + finder model | KEEP (deliberate architecture) |
| 27 | vc git | `C-x v v` next-action, `C-x v d` vc-dir (vanilla VC) | full magit surface (status/stage/hunks/discard/log/blame/commit) — redline ahead of vanilla emacs | KEEP (ahead) |
| 28 | describe-key | `C-h k` explains a keybinding | transient menu + discovery home (plan 004-06) covers discoverability | KEEP (note `C-h k` as future candidate) |
| 29 | minibuffer completion | TAB completion list | fuzzy filter-as-you-type in pickers | KEEP (no TAB needed) |
| 30 | minibuffer history | `M-p`/`M-n` in prompts | finder has recents; query history none | DEFER |
| 31 | kill buffer | `C-x k` prompts by name | **exists**: opens the buffer-list picker with kill affordance (2-of-2 driven); shape differs | KEEP (backlog: `d` inside the list echoes unbound — bind it as the kill verb per dired convention) **[backlog DONE 2026-09-18, plan 004 issue 05h — see follow-up]** |

## Battery 2 — 2026-09-17 (word motion, prefixes, registers, replace, dired, VC, minibuffer)

Driver fixes first: the `C-x 2`/`C-x o` keystroke bugs were fixed in BOTH
drivers (`0x18 0x32` / `0x18 0x6f`) — the review-flagged family. Harness
note: the dired leg typed `src/` relative to an already-src directory
(emacs errored src/src/ — harness path mistake, dired itself works).

| 20 | word motion | `M-f`/`M-b` move by word (column moves) | unverified at column level in this battery (frames don't show column) | re-verify battery 3 |
| 21 | kill-line / kill-word | `C-k` / `M-d` in editable text | unbound in read-only views (expected); notes have no kill-line/kill-word (single-char backspace only) | PROPOSE (kill-line/kill-word in editable buffers, on the 004-03 ring machinery) |
| 22 | numeric prefix | `C-u 3 C-n` / `M-5 C-n` repeat 3/5 | none; **`C-u` is bound to half-page scroll** (deliberate but conflicting — adopting numeric args needs a different home for half-page) | PROPOSE (user decision: real `C-u` args vs keep half-page) |
| 23 | registers | `C-x r s a` / `C-x r i a` prompt + restore | none | DEFER (kill ring covers the common case) |
| 24 | query-replace | `M-% line RET rope RET !` rewrote the buffer | none | PROPOSE (in-file replace once file-edit mode lands, plan 005) |
| 25 | word search | `M-s w` | isearch (substring) + project search cover it | KEEP |
| 26 | dired | `C-x d` directory editor (flag-based file ops) | tree sidebar + finder model | KEEP (deliberate architecture) |
| 27 | vc git | `C-x v v` next-action, `C-x v d` vc-dir (vanilla VC) | full magit surface (status/stage/hunks/discard/log/blame/commit) — redline ahead of vanilla emacs | KEEP (ahead) |
| 28 | describe-key | `C-h k` explains a keybinding | transient menu + discovery home (plan 004-06) covers discoverability | KEEP (note `C-h k` as future candidate) |
| 29 | minibuffer completion | TAB completion list | fuzzy filter-as-you-type in pickers | KEEP (no TAB needed) |
| 30 | minibuffer history | `M-p`/`M-n` in prompts | finder has recents; query history none | DEFER |
| 31 | kill buffer | `C-x k` prompts by name | **exists**: opens the buffer-list picker with kill affordance (2-of-2 driven); shape differs | KEEP (backlog: `d` inside the list echoes unbound — bind it as the kill verb per dired convention) **[backlog DONE 2026-09-18, plan 004 issue 05h — see follow-up]** |

M-b` move by word (column moves) | unverified at column level in this battery (frames don't show column) | re-verify battery 3 |
| 21 | kill-line / kill-word | `C-k` / `M-d` in editable text | unbound in read-only views (expected); notes have no kill-line/kill-word (single-char backspace only) | PROPOSE (kill-line/kill-word in editable buffers, on the 004-03 ring machinery) |
| 22 | numeric prefix | `C-u 3 C-n` / `M-5 C-n` repeat 3/5 | none; **`C-u` is bound to half-page scroll** (deliberate but conflicting — adopting numeric args needs a different home for half-page) | PROPOSE (user decision: real `C-u` args vs keep half-page) |
| 23 | registers | `C-x r s a` / `C-x r i a` prompt + restore | none | DEFER (kill ring covers the common case) |
| 24 | query-replace | `M-% line RET rope RET !` rewrote the buffer | none | PROPOSE (in-file replace once file-edit mode lands, plan 005) |
| 25 | word search | `M-s w` | isearch (substring) + project search cover it | KEEP |
| 26 | dired | `C-x d` directory editor (flag-based file ops) | tree sidebar + finder model | KEEP (deliberate architecture) |
| 27 | vc git | `C-x v v` next-action, `C-x v d` vc-dir (vanilla VC) | full magit surface (status/stage/hunks/discard/log/blame/commit) — redline ahead of vanilla emacs | KEEP (ahead) |
| 28 | describe-key | `C-h k` explains a keybinding | transient menu + discovery home (plan 004-06) covers discoverability | KEEP (note `C-h k` as future candidate) |
| 29 | minibuffer completion | TAB completion list | fuzzy filter-as-you-type in pickers | KEEP (no TAB needed) |
| 30 | minibuffer history | `M-p`/`M-n` in prompts | finder has recents; query history none | DEFER |
| 31 | kill buffer | `C-x k` prompts by name | **exists**: opens the buffer-list picker with kill affordance (2-of-2 driven); shape differs | KEEP (backlog: `d` inside the list echoes unbound — bind it as the kill verb per dired convention) **[backlog DONE 2026-09-18, plan 004 issue 05h — see follow-up]** |

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


## Battery 4 — differential probe #2 (2026-09-18, equal viewports 24x80)

Method: `tools/probe_emacs_diff2.py` runs the same controlled keystrokes on
vanilla `emacs -Q -nw` (pty forced to 24x80 so viewports match) and on
redline, and diffs cursor (row,col). 15/22 rows matched exactly; the
divergences are below. (The first battery of this method, battery 3,
found and fixed the 05c word-motion bug — see that section.)

| # | Row | Disposition | Evidence |
|---|---|---|---|
| 32 | Page-scroll point model | **DELIBERATE DIVERGENCE — user veto welcome** | emacs `C-v`/`M-v` scroll the window and leave the point at its **buffer** line, clamping it to the window edge only when it would go off-screen (observed: point near the top, C-v → cursor row 1 = the new top line; point near the bottom, C-v → cursor row 1 as well). redline instead **pins the point's screen row** and recomputes its buffer line from the new window top (observed: C-v ×2 → cursor row 11 fixed, point line 10 → 35 → 60). Consequence: scrolling in redline advances the "current line" (status `L{n}`, which-function, region/anchor origin) through the buffer; in emacs the point stays put unless forced. The 05b/05c specs (written by the orchestrator) called the redline model "emacs behavior" — that claim was WRONG and is corrected here and in the spec text. The redline model is coherent for a cursor-centric TUI (the cursor never leaves the screen) and is kept pending a user call. |
| 33 | `C-d` / `C-u` | **DOCUMENTED ADOPTION (unchanged)** | emacs: `C-d` = `delete-char` (no-op on a read-only buffer), `C-u` = universal prefix. redline: half-page scroll down/up (README keymap). Already a recorded adoption from plan 002/004-02; the probe re-confirms the keys do different things, by design. |
| 34 | `M->` last-line landing | **MATCH (resolved)** | Follow-up probe on a 5-line file WITH a trailing newline: both land at cursor row 6, i.e. the empty line after the final newline (end-of-buffer), status `Bot`. The earlier row-19-vs-21 discrepancy in probe #2 was the unequal-viewport artifact (emacs 30 rows vs redline 24), not a point-line difference. (Minor caveat: the emacs leg echoed "Mark set" in the echo area, so the leg may have driven an extra command; the end-of-buffer row matched regardless.) |

## Plan 004 issue 05c follow-up — 2026-09-18 (C-l contract + word motion rule)

| # | Row | Disposition | Evidence |
|---|---|---|---|
| 8 | C-l recenter | **CONTRACT REPLACED (05c)** | The battery-adopted model (cycle the WINDOW through buffer thirds, cursor-is-window-top) misbehaved: the window scrolled off the point (observed: point line 20, window top 47). 05c implements emacs `recenter-top-bottom`: the point does not move; its SCREEN ROW moves among the positions in emacs `recenter-positions`'s default order `(middle top bottom)`. `recenter-top-bottom` advances the position only when the immediately-preceding command was also recenter (a `recenter_cycle` index reset on any other command, like emacs `recenter-last-op`); otherwise it starts at the first position. So a fresh C-l goes to MIDDLE (`viewport/2`), and consecutive C-l's cycle middle → top → bottom → middle. Each position is `scroll_top = point_line − desired_row` clamped to `[0, max_scroll]`. Tiny viewports still cycle (rows that collide after clamping simply repeat); barely-scrolling buffers collapse to the reachable positions (same as emacs). Unit tests reworked (not deleted): `recenter_fresh_goes_to_middle`, `recenter_consecutive_cycles_middle_top_bottom`, `recenter_resets_to_middle_after_other_command`, `recenter_resets_cycle_through_dispatch`, `recenter_full_cycle_returns_to_start`, `recenter_tiny_scroll_ranges_keep_the_point_in_view`, `recenter_tiny_viewports_still_cycle`; `recenter_noop_when_buffer_fits_viewport` retained. Verified live against vanilla emacs (`tools/probe_emacs_diff.py`): point line 30, viewport 22 → cursor rows 14 → 1 → 27 → 14. The `cursor_cell` hardware-cursor row is clamped to the viewport so a window that sits off the point can never place the cursor outside the file area. |
| word | M-f / M-b | **IMPLEMENTED (05c)** | `word-forward` / `word-backward` registered (motion); `M-f` / `M-b` bound in the Buffer view keymap. `M-f` = emacs `forward-word`: skip the non-word run, then walk word chars to the word's END (lands at the word's end, not its first char). `M-b` = emacs `backward-word`: skip the non-word run backward, then retreat word chars to the word's START (lands at the word's start, not its end). Fixed rule (documented on `is_word_char` in store.rs): word = run of alphanumeric/`_`; non-word = every other character (punctuation AND whitespace, one class); newlines are non-word, so the skip crosses line boundaries — but the landing is at the adjacent word's END (forward) / START (backward), exactly like emacs `forward-word`/`backward-word` (NOT at the non-word run boundary, which is what `skip-chars` would do). Difference from the emacs syntax table: emacs's word-constituent set is per-buffer/configurable (e.g. `?`/`!` may be word-constituents; symbol vs whitespace categories differ) — redline's is a fixed class. Word motion sets `goal_col` to the landing column (emacs). Verified live against vanilla emacs (`tools/probe_emacs_diff.py`) on `fn alpha() { let x = 1; }` (len 25): M-f 0→2, 2→8, 3→8, 8→16, 9→16, 12→16, 25→(wrap)→2; M-b 0→0, 2→0, 3→0, 8→3, 9→3, 12→3, 25→21. Unit tests: `word_forward_walks_words_and_punctuation`, `word_backward_walks_words_and_punctuation`, `word_motion_over_empty_lines`, `word_motion_sets_goal_column_to_landing_col`. |
| mouse | click col / wheel | **IMPLEMENTED (05c)** | Click now sets the full (line, col) point — `mouse_click_position(row, col)`, col = terminal column (1:1 char index, no gutter), clamped to EOL; the existing goal column is preserved. The file-view wheel is the C-v primitive at a 3-line step (`scroll_window_point(±3)`, 05b): the point's screen row is pinned and its buffer line advances only because the window moves under it (no line-drag). List views keep their wheel behavior (selection moves). Known limitation (pre-existing, unchanged): click-select in list views is not implemented (per-view selection models differ); a click's column is shifted by the tree sidebar width when the tree is visible (clamped to EOL at worst). |

## Plan 004 issue 05h follow-up — 2026-09-18 (buffer-list key consistency)

| # | Row | Disposition | Evidence |
|---|---|---|---|
| 31 | `d` in the buffer list | **BACKLOG DONE (05h)** | `d` now kills the SELECTED buffer in the `C-x C-b` list, reusing the existing `kill_buffer` path the `C-x k` picker runs (new `buffer-list-kill-selected` command; no second kill path). `d` does not close the list; the selection clamps to a valid row after a kill. Also bound in the same fix: `n`/`p` move the selection (same commands as the existing arrows/`C-n`/`C-p` binds, matching the magit-status `n`/`p` convention); the key-help footer lists n/p/d. Unit test `buffer_list_n_p_d_keys`; raw-PTY legs in `tools/sweep_flows.py`. Verified live on a fresh app: `n`/`p` no longer echo `unbound key`, `d` drops the buffer count and keeps the list open. |
| 6 | `C-x b` re-verify | **RE-VERIFIED WORKING (05h)** | Confirmed live 2026-09-18 on a fresh app instance: `C-x b` opens the buffer picker (`switch-buffer`). Closes the re-verify half of row 6's DEFER; the picker-vs-minibuffer shape stays a deliberate KEEP (row 5's convention). |

## Plan 005 issue 01 — file edit mode (2026-09-19)

| # | Row | Disposition | Evidence |
|---|---|---|---|
| 1 | `C-x C-q` toggle-read-only | **IMPLEMENTED (005-01)** | New `toggle-read-only` command (buffers category) bound to `C-x C-q` (global keymap). File-backed buffers start read-only; the command flips `Buffer.editable` via `BufferTable::get_mut` (the first mid-session editability flip). Non-buffer views and `*scratch*` (no path) are no-ops with a minibuffer message. Toggling back to read-only with unsaved edits arms a y/n discard confirm (`toggle_ro_confirm`, routed in `key_event` before the self-insert branch): `y` re-reads the on-disk content (edits discarded, `locally_modified`/`changed_on_disk` cleared) and makes the buffer read-only; `n`/C-g/ESC cancel and keep edit mode + text; stray keys are swallowed. No state was cached from `editable` at construction: the changed-on-disk banner hint, the cursor's editable arm, and the edit path all read it live per keypress/frame, so the flip re-derives everything. Unit tests: `toggle_read_only_flips_file_buffer_on_and_off`, `toggle_read_only_noop_on_scratch_and_non_buffer_views`, `edit_mode_file_buffer_accepts_typing_and_set_locally_modified`, `save_buffer_clears_locally_modified_and_writes_disk`, `save_read_only_buffer_refuses`, `toggle_back_to_read_only_with_unsaved_edits_arms_confirm`, `toggle_ro_confirm_cancel_keeps_edit_mode_and_text`, `toggle_ro_confirm_accept_discards_and_makes_read_only`, `c_x_c_q_and_c_x_c_s_are_bound`. Raw-PTY legs in `tools/sweep_flows.py` (`edit-toggle`, `edit-save`, `edit-confirm`, `edit-conflict`). |
| 2 | `C-x C-s` save in place | **IMPLEMENTED (005-01)** | Reuses `save_buffer()` → `save_buffer_key()` (004-04) unchanged: any editable buffer with a path saves, updates mtime, clears `locally_modified`/`changed_on_disk`, re-highlights. The new piece is watcher self-write suppression for saved paths (`created_paths` only covered files we CREATED): `save_buffer_key` records the post-save mtime in `saved_paths: HashMap<String, SystemTime>`; `apply_project_change` (same branch as `created_paths`) suppresses the event only while the on-disk mtime still matches the recorded one — our own write. A genuinely later external write changes the mtime, the marker is consumed, and the event falls through to the normal conflict/auto-reload handling. Unit tests: `saved_path_suppresses_own_watcher_event`, `genuine_external_write_after_save_still_conflicts`, `read_only_file_buffer_still_auto_reloads_on_external_change`. |
| 3 | Edit-mode reload guard | **DECISION: edit mode is locally owned (005-01)** | `Buffer::is_locally_owned()` is now `path.is_none() \|\| locally_modified \|\| (editable && path.is_some())` — the recommended option from the spec: a file in edit mode is guarded even before the first keystroke, so an external change can never silently rewrite the buffer under the cursor mid-edit-session (it raises the changed-on-disk marker instead; force-reload via `M-x reload-buffer` / `g`-when-read-only supersedes). Read-only file buffers auto-reload exactly as before (unit `read_only_file_buffer_still_auto_reloads_on_external_change`; PTY `edit-conflict` contrast leg). Side effect, accepted: an untouched notes buffer is now conflict-flagged on external change rather than auto-reloaded (it is an editable file buffer — coherent). Unit tests: `edit_mode_without_edits_is_locally_owned`, `is_locally_owned_edit_mode_file_buffer` (model). |
| 4 | Status-line mode word | **IMPLEMENTED (005-01)** | The status line shows `Edit` / `Read-only` after the view name in the buffer view (a word is clearer here than emacs's `%%`/`--`); empty in the other views. `buffer_mode_display()` on the store; `StatusLine` prop. Unit test `status_line_shows_buffer_mode`; palette-count assertions updated 101 → 102 (registry seed +1: `toggle-read-only`). |

## Picker-density follow-up — remaining pickers converted to name-first rows

| # | Row | Disposition | Evidence |
|---|---|---|---|
| 1 | imenu | **CONVERTED** | `imenu_candidate` now sets label = the (indented) name, detail = `[kind]`; `display` (the nucleo match target) stays byte-identical. The impl-parent grouping/indent travels with the label. Pins: `imenu_groups_impl_methods_under_the_struct` (store, label/detail + refilter re-derivation), `unit_flow_imenu_impl_parent_grouping` (re-pinned — see #2). |
| 2 | flow_tests re-pin | **RE-PINNED (intent preserved)** | `flow_tests.rs` pinned the implementation-level shapes: exact `display == "  new  [fn]"` and the contiguous rendered substring `frame.contains("  new  [fn]")`. Both are now pinned on the new name-first shape: the label carries the 2-space indent (`"  new"`), the detail carries the kind tag (`"[fn]"`), and the render80 leg asserts the indented method row and the flat free-fn row each carry their tag on the rendered row. Requirement-level intent kept: imenu rows still show the kind tag; impl methods stay grouped/indented under their struct. |
| 3 | impls | **CONVERTED** | label = `impl <Trait> for <Type>`, detail = `file:line`. Pin: `find_implementations_opens_picker_of_trait_impls`. |
| 4 | find-file / recents | **CONVERTED** | label = the file name, detail = the project-relative path (root-level files carry the path once, as the name). `display` stays the full path (matching unchanged). Pins: `find_file_candidates_are_name_first`, recents leg in `find_file_picker_opens_filters_and_opens_on_ret`, `unit_flow_picker_name_first_rows_find_file_and_buffers` (render80 row shape). |
| 5 | buffers (switch/kill) | **CONVERTED** | label = the buffer name (`display`), detail = the absolute path (the buffer key). Pins: `switch_buffer_and_kill_buffer` (store) + the render80 buffer leg above. |
| 6 | project switcher | **CONVERTED** | label = the project name, detail = the root path. Pin: `switch_project_lands_in_new_projects_file_picker`. |
| 7 | branch | **CONVERTED** | label = the branch name, detail = the HEAD marker `*` (non-current rows have no detail and render `display` only). `display` keeps the `*`-prefixed form for matching. Pin: `branch_and_stash_candidates_are_name_first`. |
| 8 | stash | **CONVERTED** | label = `stash@{n}`, detail = the subject. Pin: `branch_and_stash_candidates_are_name_first`. |
| 9 | palette | **KEPT DISPLAY-ONLY (deliberate)** | A palette row's whole content is one command identifier — no kind or path to right-align — so the name-first split would just duplicate the name. Documented at `palette_candidates` and in the `PickerCandidate`/ui docs. Pin: `palette_rows_stay_display_only`. |

Notes: matching semantics unchanged (nucleo still scores `display`); the selected-row bar contiguity is unchanged (ui leg in `src/ui/picker.rs`). PTY drives re-verified: none assert picker-row shape (sweep_flows / drive_xref / drive_external_crate assert status lines, buffer content, and jump messages only). `tools/gate.sh full` PTY battery DEFERRED: swap was exhausted at landing (cargo test --workspace: 849 + 120 + 7 + 2 + 3 + 1 + 1 passed, 0 failed; clippy clean).

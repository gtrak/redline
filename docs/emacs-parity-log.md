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

## Findings vs battery notes

- The isearch bug (#1) is the only functional defect found in battery 1.
- Harness bug on our side: the battery's `C-x b` step actually sent `C-x 2`
  (0x32 vs 0x62) — C-x b untested; row 6 stays DEFER until re-driven.

## Open questions for the user (PROPOSE rows)

9 (C-v overlap), 11 (position %), 13 (quit save-prompt), 14 (kill/yank
scope), 15 (undo in notes). **User answered 2026-09-17: they review these
first — no default implementations.** Only clear BUGS go forward without
review (row 1, isearch — in flight as plan-004 issue 01). Rows 9/11/13/14/
15 wait for explicit user decisions; DEFER rows 6/8 get re-verified in the
next battery regardless (verification, not decision).

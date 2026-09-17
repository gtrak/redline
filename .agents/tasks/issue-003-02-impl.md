# Task: Implement plan 003 issue 02 — Shared windowing helper, applied (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained: read it, then execute it in order. `.agents/skills/*.md` are
authoritative ground truth.

Context: plan 003 issue 01 (watcher Access fix) is committed. Plan 002
issue 02 shipped magit-status windowing: `magit_scroll` state +
`magit_keep_visible()` (clamped follow-scroll) + `magit_window()` slicing
+ scroll indicators + pinned help line (store.rs ~2955-2999; the pyte
drive `tools/drive_windowing.py` and the store test
`magit_status_window_keeps_cursor_visible` are its regression net). This
issue generalizes that mechanism to the panes that still clip: the
commit-diff (`git show`) pane, the blame view, the log view's in-page
selection, and editable buffers. The user's ask: make the git-show pane
scrollable "and anything else subject to the same issue".

## Working agreement (overrides any caution)

- **Skills are truth.** Work from `.agents/skills/*.md`. Do NOT read
  dependency sources under `~/.cargo/registry`, do NOT browse docs.rs, do
  NOT fetch anything.
- **Write-first.** Extract the helper and scaffold the first pane in your
  first handful of tool calls, compile early, iterate. No front-loaded
  research.
- **Skill corrections.** If running code contradicts a skill file: code
  wins, AND make a minimal factual correction to the skill file; list it
  under "skill corrections".
- **graft** CLI available (`graft map/ask/callers/skeleton/grep`).

## Read first

1. `.agents/plans/003-scroll-and-watcher/02-shared-windowing.md` — THIS
   issue (the contract).
2. `src/app/store.rs` — `magit_scroll` / `magit_keep_visible` /
   `magit_window` / `magit_view_info` (~2955-2999); log view state
   (`log.selected`, paging in `n`/`p` handlers ~3449-3492); blame state
   (`b.selected`, ~3820+); commit-diff state; buffer/rope state for
   editable buffers (`locally_modified`, insertion path).
3. `src/ui/rows_view.rs`, `src/ui/magit_status.rs` — the windowed render
   pattern (rows slice + scroll indicator + pinned help).
4. `src/ui/file_view.rs` + the commit-diff render arm in
   `src/ui/root.rs` — where the commit-diff pane renders.
5. `src/app/command.rs` + keymap — FileView motion bindings (C-n/C-p/C-v/
   M-v/M-</M->) as the vocabulary for commit-diff scrolling.
6. `tools/drive_windowing.py`, `tools/drive_all.py`, `tools/sweep_flows.py`
   — the drive patterns to extend.

## What to build

1. **Generic helper in store.rs**: extract magit's window math into a
   reusable helper (offset + keep_visible(cursor, total) + clamp + window
   slice). REPOINT magit to it with zero behavior change (the magit tests
   + drives are the proof). Then per-pane scroll state:
   - **commit-diff**: scroll offset + emacs-motion keys (C-n/C-p/C-v/M-v/
     M-</M->) bound for the commit-diff view; windowed render with a scroll
     indicator; help line pinned. No cursor — keys move the window.
   - **blame**: cursor-following window (b.selected must stay in view on
     every move; top clamps).
   - **log**: in-page keep_visible — in-page motion (arrows) must keep the
     selection in view; paging (n/p) behavior unchanged.
   - **editable buffers**: windowed viewport with chord scrolling; typing
     keeps the insertion row in view. A visible insertion-point cue is
     DEFERRED (cursor rendering, out of scope).
2. **Structural/unit tests per pane**: window bounded, cursor/insert row in
   view, top advances and clamps; magit windowing test unchanged and green.
3. **pyte drives** (extend tools/): commit-diff scroll through a diff
   taller than the viewport (M-> lands on the last row, M-< round-trips);
   blame cursor stays in-window across a long file; log selection in-window
   after many in-page moves; notes typing near the bottom keeps the active
   region in view.

## Constraints

- No dependency changes. Magit status behavior byte-identical (its unit
  test + drive_windowing.py + sweep flows are the regression net).
- Motion-key vocabulary only (C-n/C-p/C-v/M-v/M-</M->); no new binding
  names. Editing semantics in notes unchanged (printables self-insert;
  chords scroll).
- Scope fence: store.rs, rows_view.rs, magit_status.rs (repoint only),
  file_view.rs / root.rs (commit-diff arm), command.rs/keymap (motion for
  commit-diff), tests, tools/ drives. Nothing else.
- Layering: window math is plain-Rust store code; UI imports stay in ui/.

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green (baseline 350 passed / 2 ignored, + new tests).
- `python3 tools/sweep_flows.py` 39/39; `python3 tools/sweep.py` 14/14;
  `python3 tools/drive_windowing.py` + `tools/drive_all.py` unchanged PASS.
- New pyte drives PASS (the four panes above).
- Coverage split in the report: test-covered / PTY-covered / manual-only.

## Report format

- Helper design + per-pane wiring table (pane | state | keys | window
  behavior).
- Gate outputs (exact counts). Skill corrections (or none). Deviations;
  known gaps.

# Task: Implement plan 004 issue 02 — parity adopts, batch 1 (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained. `.agents/skills/*.md` are authoritative ground truth.

Context: plan 004 issues 01 (isearch) and 05 (cursor visibility: hardware
cursor + truecolor bar) are committed. This issue lands the USER-APPROVED
parity rows from `docs/emacs-parity-log.md` (user decisions 2026-09-17:
9 YES, 11 YES; rows 6/8 re-verified as part of this work).

## Working agreement (overrides any caution)

- **Skills are truth.** Work from `.agents/skills/*.md`. Do NOT read
  dependency sources under `~/.cargo/registry`, no docs.rs, no fetching.
- **Write-first.** Fix early, compile, iterate.
- **Skill corrections.** Code reality wins; minimal factual correction,
  listed under "skill corrections".
- **graft** CLI available.

## Read first

1. `.agents/plans/004-emacs-parity/02-parity-adopts-1.md` — the issue.
2. `docs/emacs-parity-log.md` — battery 1 rows 6/8/9/11 (the decisions).
3. `src/app/store.rs` — file-view scroll commands (C-v/M-v/M-</M-> paths),
   buffer-list picker opening, keymap bindings, status-line info fns.
4. `src/ui/root.rs` — status-line render.
5. `tools/drive_redline_parity.py` — the battery (its C-x b key is 0x62
   after the issue-01 fix; verify, then extend).

## What to build

1. **Row 9 — scroll overlap**: `C-v`/`M-v` leave exactly 2 context rows
   overlapping (emacs `next-screen-context-lines`). Applies to the file
   view page-scroll path (and any other pane using the same scroll
   commands — check log/blame/commit-diff which gained scroll keys in
   003-02; keep the behavior uniform and driven).
2. **Row 11 — status-line position**: append emacs-vocabulary position to
   the status line: `Ln N, Col M` and the percent through the buffer
   (emacs shows e.g. `(43%, L30)`-style; pick the compact form `L30,43%`
   — state your exact format). Top = `Top`, bottom = `Bot` (emacs
   convention). Updates on every cursor movement.
3. **Row 6 — C-x b**: verify the binding; if missing, bind `C-x b` to the
   buffer-list picker (emacs switch-buffer muscle memory).
4. **Row 8 — C-l**: verify the binding; if missing, bind `C-l` to recenter
   cycling (top → middle → bottom → top), emacs semantics, in the file
   view (and panes with a cursor where sensible — same uniformity rule).
5. **Battery**: extend `tools/drive_redline_parity.py` — overlap leg
   (C-v leaves 2 rows of overlap: the top visible row before C-v appears
   2 rows above the window top after), position segment leg, C-x b leg,
   C-l cycle leg. Flip parity-log rows 6/8/9/11 to decided-with-evidence
   (append the evidence, do not rewrite the rows).

## Constraints

- No dependency changes. No new commands beyond the two bindings if
  missing (C-x b reuses the existing buffer-list open command; C-l is a
  new `recenter` command — register it with docs/category per registry
  conventions and update the palette-count tests).
- Scope fence: src/app/store.rs, src/app/command.rs (+keymap), src/ui/root.rs,
  tools/drive_redline_parity.py, docs/emacs-parity-log.md. Nothing else.
- The cursor-stream harness (check_cursor_stream.py) must stay green:
  C-l/C-v affect the window, not the cursor bar logic.

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green (361 / 2 ignored baseline + new tests, 0 failed).
- `tools/sweep.py` 14/14; `tools/sweep_flows.py` 43/43; `tools/drive_all.py`
  6/6; `tools/drive_windowing.py` 28/28; `tools/drive_windowing_panes.py`
  4/4; `tools/check_cursor_stream.py` 11/11.
- PTY (battery): overlap = exactly 2 rows after C-v (both directions);
  position segment renders and tracks movement; C-x b opens the picker;
  C-l cycles top/middle/bottom.
- Coverage split in the report.

## Report format

- Per-row disposition (binding existed? exact overlap math; exact position
  format). Gate outputs (exact counts). Skill corrections (or none).
  Deviations; known gaps.

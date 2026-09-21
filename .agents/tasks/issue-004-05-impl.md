# Task: Implement plan 004 issue 05 — cursor visibility on real terminals (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained. `.agents/skills/*.md` are authoritative ground truth.

Context: plan 004 issue 01 (isearch) is committed. This is a USER-REPORTED
visibility bug with the raw-stream diagnosis already performed:

**Diagnosis (orchestrator, raw PTY capture, 2026-09-17)**: on `COLORTERM=
truecolor` / `TERM=xterm-256color`, the redline startup emits `ESC[?25l`
(hardware cursor HIDDEN) exactly once and never re-shows it; every CUP
(cursor-position) sequence parks at row 30 col 1 (status-line sync) — the
pointing cursor is invisible everywhere. The selected-row bar renders via
SGR `48;5;12` (256-color index 12), which user terminal THEMES can remap
to near-background — pyte resolves index 12 to nominal bright blue, but
the user's theme evidently does not. Hence "the cursor was still hidden."

## Working agreement (overrides any caution)

- **Skills are truth.** `.agents/skills/iocraft/SKILL.md` is the ground
  truth for render internals; do NOT read dependency sources under
  `~/.cargo/registry`, no docs.rs, no fetching.
- **Write-first.** Fix early, compile, iterate.
- **Skill corrections.** Code reality wins; minimal factual correction,
  listed under "skill corrections".
- **graft** CLI available.

## Read first

1. `.agents/plans/archive/004-emacs-parity.md` — the issue.
2. `.agents/skills/iocraft/SKILL.md` — render loop internals, alternate
   screen, synchronized output (`?2026`), any cursor-control API.
3. `src/ui/root.rs` — render loop, the status-line, how views compose.
4. `src/ui/magit_status.rs`, `src/ui/rows_view.rs`, `src/ui/tree.rs`,
   `src/ui/views/buffer.rs`, `src/ui/results_view.rs` — the five
   cursor-bar emitters (per-row View background, from 002-02).
5. `src/model/theme.rs` — face colors (White-on-Blue selected faces).

## What to build

1. **Hardware cursor (the core fix)**: after each render, the terminal
   cursor must be VISIBLE and positioned on the current view's cursor cell
   (the selected row/column — the blue bar row, at the cursor's logical
   column where meaningful; a sane column default is fine, document it).
   Emacs -nw parity: the blinking terminal cursor sits on point. The
   blanket `?25l` must be countered (either not emitted, or re-show +
   reposition per render). Coordinate with iocraft's synchronized output:
   the cursor update lands AFTER the sync pause, never inside it.
2. **Truecolor bar**: when `COLORTERM=truecolor`, emit the selected-row
   background as SGR `48;2;r;g;b` with the theme's exact RGB (not palette
   index 12); otherwise keep the 256-color path. The face model stays —
   only the escape strategy gains a truecolor variant. All five bar
   emitters covered (magit, MagitRowsView, tree, buffer list, results) via
   the shared face→escape path if one exists (create one if not).
3. **Verification tooling** (extend tools/): a raw-stream check that
   asserts — `?25l` never survives past the first render (or is never
   emitted); CUP row tracks the selected row across n/p movement; `48;2;`
   present under COLORTERM=truecolor and `48;5;12` retained without it;
   pyte still verifies the bar (drive_all unchanged-passes).

## Constraints

- No dependency changes. The cursor-bar face/colors are unchanged in the
  theme model (only escapes change). No key/store behavior changes.
- Scope fence: the render path (src/ui/, src/main.rs render loop if the
  cursor logic lives there), theme escape plumbing, tools/. store.rs only
  if a cursor-cell accessor is genuinely needed (prefer reading from
  existing view_info structures).

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green (361 / 2 ignored baseline, + new).
- `tools/sweep.py` 14/14; `tools/sweep_flows.py` 43/43; `tools/drive_all.py`
  6/6; `tools/drive_windowing.py` 28/28; `tools/drive_windowing_panes.py`
  4/4.
- Raw-PTY (the core evidence): with COLORTERM=truecolor, drive file view +
  magit: `?25l` countered; CUP row == selected row across n/p; `48;2;`
  bar present; without COLORTERM: `48;5;12` retained. pyte drives green.
- Coverage split in the report.

## Report format

- Where the cursor logic landed (render loop vs component) and how the
  sync-pause interplay is handled. Gate outputs (exact counts). Raw-stream
  evidence excerpts. Skill corrections (or none). Deviations; known gaps.

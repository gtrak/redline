# Task: Implement plan 003 issue 01 — Watcher Access fix (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained: read it, then execute it in order. `.agents/skills/*.md` are
authoritative ground truth.

Context: plan 002 is archived. Its sweep (tools/sweep_flows.py, 39 flows)
recorded a NEW unfixed finding: the watcher treats its own ACCESS (read)
events as project changes. Mechanism (confirmed in source): notify access
kinds map to `ChangeKind::Other` in `summarize` (src/app/watcher.rs:138)
past the path noise filter, so every debounced read produces a change
batch. The app's own reads (buffer reload, symbol index) then self-sustain
a ~500 ms batch loop; and `apply_project_change` (src/app/store.rs:4106+)
flags locally-owned buffers `changed_on_disk` from those batches — a false
marker ~0.5 s after the user's first keystroke in notes. The open-form
(no local edits) is verified clean; the editing-form is the bug.

## Working agreement (overrides any caution)

- **Skills are truth.** Work from `.agents/skills/*.md`. Do NOT read
  dependency sources under `~/.cargo/registry`, do NOT browse docs.rs, do
  NOT fetch anything.
- **Write-first.** Make the watcher change in your first handful of tool
  calls, compile early, iterate. No front-loaded research.
- **Skill corrections.** If running code contradicts a skill file: code
  wins, AND make a minimal factual correction to the skill file; list it
  under "skill corrections".
- **graft** CLI available (`graft map/ask/callers/skeleton/grep`).

## Read first

1. `.agents/plans/003-scroll-and-watcher/01-watcher-access-fix.md` — THIS
   issue (the contract).
2. `docs/ux-testing-plan.md` — the NEW FINDING row (U-G3/notes-edit) with
   the full evidence chain.
3. `src/app/watcher.rs` — `summarize`, the noise filter, the existing tests
   (`summarize_drops_git_and_log_noise`,
   `git_and_log_noise_produce_no_event` — note its positive-control
   pattern).
4. `src/app/store.rs` — `apply_project_change`, `created_paths`,
   `reload_buffer` (the locally-owned branch at ~4127-4144).
5. `tools/sweep_flows.py` — `flow_g3` (currently leans on the Access bug
   to raise the marker) and the harness patterns (pump/wait_for,
   deadline reads).

## What to build

1. **Watcher**: in `summarize`, an event batch whose events are ALL
   access-kind publishes NOTHING. In mixed batches, access-kind events are
   dropped and real kinds (create/modify/remove) flow unchanged. The path
   noise filter is untouched. Keep the existing public API; no new deps.
2. **Watcher unit tests**: (a) access-only batch → no event published;
   (b) mixed batch → only the modify/create/remove arrives; (c) positive
   control with a live watcher (the established pattern) so the test can't
   false-pass on a dead pipeline.
3. **Store test**: locally-owned buffer + access-only batch → NO
   changed_on_disk marker (mirrors the bug's exact path).
4. **`flow_g3` rework** (tools/sweep_flows.py): the marker must now be
   raised ONLY by the real external disk append; add a type-then-idle leg
   (type into notes, pump ~2-3 s) asserting NO marker appears and the app
   stays quiet. Keep the reload-supersede assertions (M-x reload-buffer
   clears the marker, disk line shown, typed char superseded).
5. **Findings log**: flip the U-G3/notes-edit NEW FINDING row to FIXED with
   the new evidence (per docs/ux-testing-plan.md conventions — append, do
   not rewrite history rows).

## Constraints

- No dependency changes. No keymap/behavior changes beyond the watcher
  classification. `created_paths` stays.
- Real external modifications MUST still land (the external-append leg of
  G3 and F4's auto-reload flow are the regression net — both must stay
  green).
- Scope fence: watcher.rs, store.rs (test only unless a compile forces a
  touch), tools/sweep_flows.py, docs/ux-testing-plan.md. Nothing else.

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green (baseline 346 passed / 2 ignored).
- `python3 tools/sweep_flows.py` — all 39 flows PASS (G3 reworked).
- `python3 tools/sweep.py` — 14/14 transitions clean.
- PTY evidence per the issue file: type-then-idle 5 s → no marker, no
  repeating Access batches in redline.log; external append → marker,
  reload clears it.
- Coverage split in the report: test-covered / PTY-covered / manual-only.

## Report format

- Design summary (classification approach, where dropped, what still flows).
- Gate outputs (exact counts). Skill corrections (or none). Deviations;
  known gaps.

# Task: plan 004 issue 04 — quit save-prompt with buffer selection

You are the implementation worker. Repo root is your cwd. This spec is
self-contained. `.agents/skills/*.md` are authoritative ground truth.

Issue contract (plan row 13, user-approved "with buffer selection"):
`C-x C-c` (and `q` in the transient quit path) with locally-modified
buffers must NOT silently discard edits — enumerate modified buffers and
let the user choose, like emacs `save-buffers-kill-terminal`.

## Working agreement

- Skills are truth (no registry reads, no docs.rs, no fetch). Write-first;
  compile early, iterate on named errors. Minimal skill corrections,
  listed. `graft` available.

## Read first

1. `src/app/store.rs` — `quit` flag (~828), the `quit` command wiring,
   `save_buffer()` (~1458), `Buffer.locally_modified`, the buffers map,
   the minibuffer/prompt state machine patterns (isearch, kill/yank
   C-g cancel matrix from 004-03), and the commit-editor editable buffer.
2. `src/app/command.rs` — the `quit` command registration (~112) and how
   commands set `store.quit`.
3. `src/ui/root.rs` — the minibuffer row render (prompt echo patterns).
4. `.agents/skills/emacs-ux/SKILL.md` — save-buffers-kill-terminal prompt
   semantics (`y`, `n`, `!`, `C-g`).

## What to build

1. **Quit interception**: `quit` no longer flips `store.quit` directly.
   If NO buffer is `locally_modified`, quit proceeds immediately
   (existing behavior preserved — the 43/46-flow q-quit suite must stay
   green). If ≥1 buffer is modified, enter a save-prompt state machine.
2. **State machine** (`store.quit_prompt: Option<...>`), oldest-first over
   the modified buffers:
   - Prompt text in the minibuffer row:
     `Save this buffer: <path>? (y, n, !, C-g)`.
   - `y` → `save_buffer()` for that buffer, next modified buffer.
   - `n` → skip it, next modified buffer.
   - `!` → save this and ALL remaining modified buffers, then quit.
   - `C-g` → cancel the quit entirely (no buffer saved beyond those
     already answered y), prompt cleared, app stays alive.
   - After the last answer (or !), `store.quit = true`.
   - If a save fails: report the error (minibuffer message) and re-prompt
     the SAME buffer (do not advance).
3. Modified-set snapshot is computed at interception time (do not
   recompute mid-prompt; a buffer saved during the prompt must not
   re-appear).
4. Notes/scratch count only when `locally_modified` (typed into).
5. C-g here must obey the 004-03 cancel discipline: quit_prompt clears,
   mark/pending unaffected beyond normal cancel.

## Constraints

- Scope fence: `src/app/store.rs` (state machine + quit interception),
  `src/app/command.rs` (quit command delegates to the interceptor), and
  `src/ui/root.rs` (prompt render if the minibuffer path needs a new arm).
  No dependency changes. No undo. Do not alter kill/yank, isearch,
  region, cursor-point (004-05b) behavior.
- All suites green: `sweep.py` 14/14, `sweep_flows.py` (all driven),
  `drive_all` 6/6, `drive_windowing` 28/28, `drive_windowing_panes` 4/4,
  `check_cursor_stream` (all), plus `cargo test` (405+ / 2 ignored).

## Verification (iterate until ALL pass)

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test green.
- New PTY flow(s) in `tools/sweep_flows.py`:
  - modified notes buffer (type into it) + `C-x C-c` → prompt renders with
    the path; `y` → file written on disk (assert contents) and app exits
    (process ends, exit 0); 
  - `n` on a modified buffer → exits without writing (assert file
    unchanged), edit knowingly lost;
  - two modified buffers → `!` saves both (both files written) then exits;
  - `C-g` → prompt gone, buffer content intact, app STILL RUNNING
    (assert process alive), then unmodified/re-quit proceeds;
  - unmodified quit stays immediate (no prompt rendered).
- Unit tests for the state machine (y/n/!/C-g ordering, save-fail
  re-prompt, snapshot immutability, no-modified fast path).

## Report format

State machine design (fields, transitions). Per-key table. Gate outputs
(exact counts). Skill corrections (or none). Deviations; known gaps.

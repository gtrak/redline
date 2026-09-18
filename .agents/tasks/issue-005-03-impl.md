# Task: plan 005 issue 03 — quit dump (agent-consumable)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained. `.agents/skills/*.md` are authoritative ground truth.

Issue contract (`.agents/plans/005-edit-modes/03-quit-dump.md` — read it;
it has an orchestrator pre-check with the exact teardown point): on quit,
annotations print to stdout as a **self-contained brief any agent can act
on** — file, line, the anchored code, and the user's note.

User intent, verbatim (2026-09-18): "I would probably just pass the notes
into an agent, so enough information there that it'll know what to do."

## Read first

1. `.agents/plans/005-edit-modes/03-quit-dump.md` — contract + pre-check
   (teardown point `src/main.rs:133` `app.fullscreen()...await?`; the
   store is behind `store_handle: Arc<Mutex<AppStore>>`).
2. `src/main.rs` (143 lines) — the whole file; the dump goes after the
   fullscreen await and the watcher stop, before `Ok(())`.
3. `src/app/store.rs` — the annotation model + lookup from 005-02.

## What to build

1. **Formatter** (pure function, unit-tested first):
   - Per-annotation block:
     ```
     src/app/store.rs:1420
         fn save_buffer(&mut self) {
       NOTE: this silently overwrites the mtime; check the conflict marker first
     ```
     path:line, then the anchored line indented +4 as code, then `NOTE:`
     with the text (subsequent note lines aligned under the first).
   - Orphaned annotations: print the block with an explicit
     `  ORPHANED (anchor text not found)` line — the agent must not trust
     a stale line number silently.
   - Header: `# redline annotations — <project root>` then a blank line;
     **suppressed entirely when there are no annotations** (zero bytes).
   - Ordering: path asc, then line asc.
2. **`--notes=plain`** flag: same content as `path:line: text` (grep/pipe
   shape); no header change needed beyond what plain implies. Minimal
   `std::env::args` parsing in `main.rs`; default (no flag) = the block.
   Unknown args: ignore (do not break startup) — or error clearly;
   choose and state which.
3. **Emit after teardown**: after `app.fullscreen().ignore_ctrl_c().await?`
   returns (the alternate screen has exited) and after the watcher stops,
   lock the store handle and print the dump. Never print before that point
   (must not interleave with the TUI).

   **CRITICAL — stdout is NOT clean; the TUI owns it (verified live
   2026-09-18).** The earlier assumption that iocraft "renders to the tty"
   is WRONG: `root.rs:568` writes the cursor via
   `crossterm::execute!(std::io::stdout(), …)` and the fullscreen loop emits
   frames to stdout. I forked the app with stdout redirected to a FILE and
   the pty as the controlling terminal: the file received **10207 bytes of
   escape codes** (`\x1b[?1049h` alt-screen, `\x1b[?25l`, CUP, SGR). So
   `redline > notes.txt` today captures the whole frame stream — useless for
   the stated workflow (handing notes to an agent).
   **Required**: when stdout is NOT a tty (`std::io::IsTerminal`), render the
   TUI to `/dev/tty` (opened separately) so stdout carries ONLY the dump;
   the cursor `execute!` site must write to that same handle, not a
   hardcoded `std::io::stdout()`. When stdout IS a tty, current behavior is
   fine (dump printed after teardown on the same terminal). If `/dev/tty`
   cannot be opened, degrade gracefully and SAY so (report + skip the dump
   rather than corrupt it). Add a test that redirects stdout to a file and
   asserts ZERO escape bytes there while frames still appear on the pty.
4. Final state only: whatever the annotation set is after any 004-04 save
   prompts (the await returns after the user finishes them).

## Constraints

- Scope fence: `src/main.rs`, `src/app/store.rs` (formatter + a
  `annotations_for_dump()` accessor if needed), tests, `tools/` (a pipe
  probe), `docs/`. No dependency changes; no TUI-key changes.
- All suites green: `cargo test`, `tools/sweep.py`,
  `tools/sweep_flows.py`, `tools/drive_all.py`, `tools/drive_windowing.py`,
  `tools/drive_windowing_panes.py`, `tools/check_cursor_stream.py`.
- Wrap EVERY python PTY invocation in `timeout`.

## Verification (iterate until ALL pass)

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test green.
- Unit: formatter for normal / orphaned / multi-line note / empty;
  ordering; plain-flag parity; no-annotations ⇒ empty string.
- PTY pipe leg: create annotations in the app, quit, and capture the
  process's stdout SEPARATELY from the tty (fork the app with stdout to a
  pipe, the tty on the slave fd); assert the block appears on stdout after
  teardown and that the TUI frames did NOT contain the dump text;
  `redline --notes=plain` prints the `path:line: text` form; with no
  annotations stdout is empty (0 bytes). Put this probe in `tools/`
  (e.g. `tools/probe_notes_dump.py`) so it is reproducible.

## Report format

Formatter spec (exact bytes for each case). The arg-parsing choice.
Gate outputs (exact counts). Skill corrections (or none). Deviations.

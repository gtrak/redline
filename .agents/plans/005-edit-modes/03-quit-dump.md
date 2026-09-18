# 03 — quit dump (agent-consumable)

Phase 1 · Depends on: 02

## Objective

On quit, the annotations print to stdout as a **self-contained brief any
agent can act on** — file, line, the anchored code, and the user's note.

User intent (2026-09-18): "I would probably just pass the notes into an
agent, so enough information there that it'll know what to do."

## Key decisions

- Dump AFTER alternate-screen exit and AFTER quit save-prompts (final
  state); the render loop writes to the tty, never stdout.
- **Per-annotation block**, not just `path:line: text` — the agent needs
  the code in context:
  ```
  src/app/store.rs:1420
      fn save_buffer(&mut self) {
    NOTE: this silently overwrites the mtime; check the conflict marker first
  ```
  (path:line, then the anchored line indented +4 as code, then the note
  as `NOTE:` — readable and pipeable, diffable by line order.)
- `path:line: text` stays available via a flag (`--notes=plain`) for
  grep/redirect; default is the agent block (the stated use).
- Ordering: file path (asc), then line (asc) — stable for diffing.
- Header line names the project root so paths are unambiguous:
  `# redline annotations — /home/gary/dev/red` (suppressed when empty, so
  pipes stay clean).
- Orphaned annotations are included with an explicit `ORPHANED` marker
  (the agent must know the anchor is stale, not silently trust the line
  number).
- Empty annotation set → zero bytes on stdout.

## Orchestrator pre-check (2026-09-18)

- **Teardown point is exact**: `src/main.rs:133` `app.fullscreen()
  .ignore_ctrl_c().await?` returns AFTER the alternate screen exits; the
  watcher is then stopped (`take_watcher()` + `stop_and_wait()`). The dump
  goes after that await, before `Ok(())`. The store is behind
  `store_handle: Arc<Mutex<AppStore>>` created at `main.rs:120` — available
  post-loop; read the annotations from it there.
- **CORRECTED (2026-09-18, verified live): stdout is NOT clean — the TUI
  owns it.** iocraft's fullscreen loop renders to stdout and the cursor
  write is `crossterm::execute!(std::io::stdout(), …)` (`root.rs:568`).
  Forking the app with stdout redirected to a file captured **10207 bytes
  of escape codes** (alt-screen + CUP + SGR), so `redline > notes.txt`
  would capture the frame stream. When stdout is not a tty, render to
  `/dev/tty` and keep stdout for the dump only.
- **Quit paths to respect**: `q`-quit and `C-x C-c` both funnel through the
  store's `quit` flag, but 004-04 will introduce a save-prompt state
  machine — in that case the outer `fullscreen()` await returns only after
  the user finishes the prompts. The dump must therefore reflect the FINAL
  buffer states (annotations saved/edited during prompts are included).
- `--notes=plain` is a new CLI arg: `main.rs` currently parses no args
  (config comes from the file) — add minimal `std::env::args` handling and
  keep default behavior unchanged when the flag is absent.

## Files

| File | Change |
|---|---|
| `src/main.rs` + `src/app/store.rs` | collect annotations at quit; print post-teardown; `--notes=plain` flag. |
| tests + `tools/` flows | block format, ordering, orphan marker, empty-is-silent, plain flag, pipe test. |

## Verification

- Gates green; unit: formatter output for normal/orphaned/empty, ordering,
  plain-flag parity.
- PTY: create annotations (one orphaned), quit → block dump on stdout
  post-teardown; `redline --notes=plain 2>/dev/null | grep store.rs`
  works; no annotations → nothing; save-prompt interaction intact
  (004-04 suite green).

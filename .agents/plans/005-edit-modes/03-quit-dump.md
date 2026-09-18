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

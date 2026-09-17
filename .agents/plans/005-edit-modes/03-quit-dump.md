# 03 — quit dump

Phase 1 · Depends on: 02

## Objective

On quit, annotations print to stdout as `path:line: text` after the TUI
tears down — redirect/copy-paste friendly.

## Key decisions

- Dump AFTER alternate-screen exit and AFTER quit save-prompts (final
  state). Empty when no annotations (clean pipe).
- Ordering: file path (asc), then line (asc) — stable for diffing.
- TUI output never mixes with the dump (stdout used only post-teardown;
  verify the render loop writes to the tty, not stdout).

## Files

| File | Change |
|---|---|
| `src/main.rs` + `src/app/store.rs` | collect annotations at quit; print post-teardown. |
| tests + tools/ flows | dump content/order/emptiness; pipe test (pty captures app output; dump lands after teardown). |

## Verification

- Gates green; PTY: create annotations, quit → dump lines on stdout
  post-teardown; no annotations → nothing; save-prompt interaction intact
  (004-04 suite green).

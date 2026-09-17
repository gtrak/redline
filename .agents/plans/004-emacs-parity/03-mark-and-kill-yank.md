# 03 — mark, select, kill/yank

Phase 1 · Depends on: 02

## Objective

User-approved row 14 with "mark/select" clarification: C-SPC set-mark,
region selection with movement, kill ring — emacs semantics.

## Key decisions

- **Editable buffers (notes/local files): full** — C-SPC set-mark, region
  face on movement (C-w kill region, M-w copy, C-y yank, M-y yank-pop,
  C-x C-x exchange, DEL delete region).
- **Read-only file views: copy-only** — C-SPC + region + M-w copies to the
  kill ring; yank is impossible (read-only), matching emacs read-only
  behavior.
- One shared kill ring (state in store); region state per buffer; kill
  ring default depth small (emacs 60 is fine).
- Region face: high-contrast face from the theme (region), rendered
  attribute-level in pyte drives.
- No undo (row 15 stays logged-not-implemented).

## Files

| File | Change |
|---|---|
| `src/model/buffer.rs` | mark/region state per buffer. |
| `src/app/store.rs` | kill ring state; set-mark/kill/copy/yank/yank-pop/exchange commands; region-aware motion; C-g clears region. |
| `src/app/command.rs` / keymap | bindings (C-SPC, C-w, M-w, C-y, M-y, C-x C-x). |
| `src/ui/file_view.rs` | region face render. |
| `docs/ux-testing-plan.md` | new flows for mark/kill/yank. |

## Verification

- Gates green; PTY: set-mark → region visible (attribute-level), kill
  region → text in ring, yank restores, M-y cycles, copy-from-read-view →
  yank works in notes (cross-buffer ring); C-g clears region; mark
  persists across movement; transient-menu/matrix no regressions.

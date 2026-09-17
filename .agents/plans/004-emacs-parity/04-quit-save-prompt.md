# 04 — quit save-prompt with buffer selection

Phase 1 · Depends on: 03 (kill/yank state in buffers)

## Objective

User-approved row 13 with "buffer selection" clarification: C-x C-c with
locally-modified buffers must not silently discard — enumerate and let the
user choose (emacs save-buffers-kill-terminal semantics).

## Key decisions

- On quit with locally-modified buffers: prompt per buffer, oldest-first:
  "Save this buffer: /path? (y, n, !, C-g)" — y saves, n skips, ! saves
  all remaining, C-g cancels the quit entirely.
- Unmodified buffers never prompt. Notes/scratch created by the app count
  as modified only when typed into (locally_modified).
- Saving a notes buffer writes the file (existing save path); after the
  last prompt, quit proceeds. If save fails, report and re-prompt.

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | quit interception → save-prompt state machine (y/n/!/C-g). |
| `src/ui/root.rs` | prompt render (minibuffer row). |
| `docs/ux-testing-plan.md` | quit-prompt flow. |

## Verification

- Gates green; PTY: modified notes + C-x C-c → prompt renders, y saves
  (file written on disk), quit exits 0; n skips (edit lost knowingly);
  ! saves all; C-g cancels quit with buffer intact; unmodified quit stays
  immediate (no regression to the 43-flow q-quit suite).

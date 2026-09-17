# 01 — file edit mode

Phase 1 · Depends on: plan 004 complete (save machinery, quit save-prompt)

## Objective

File-backed buffers can be toggled into a real file-edit mode; edits are
saved to disk with C-x C-s under the existing conflict discipline.

## Key decisions

- `C-x C-q` toggles edit mode on the current file buffer (emacs
  toggle-read-only); `C-x C-s` saves. Unbound today — both are new
  bindings in the Buffer keymap.
- While editing: locally_modified semantics (external change → marker →
  quit save-prompt); save clears locally_modified and records the saved
  path so the watcher does not false-flag our own write (extend
  created_paths-style suppression to saved paths + expected content).
- Toggling back to read-only with unsaved edits: confirm (same
  discard-guards as notes) — no silent loss.
- Status line indicator shows the mode.

## Files

| File | Change |
|---|---|
| `src/model/buffer.rs` | per-buffer edit-mode flag. |
| `src/app/store.rs` | toggle command, save command + watcher suppression, conflict integration. |
| `src/app/command.rs` / keymap | C-x C-q, C-x C-s bindings. |
| `src/ui/root.rs` | status-line mode indicator. |
| tests + tools/ flows | toggle, save, suppression, conflict, discard-guard. |

## Verification

- Gates green; PTY: C-x C-q → edit, type, C-x C-s → git sees the change;
  external edit while editing → marker; quit prompts (004-04 machinery);
  read-only buffers auto-reload unchanged.

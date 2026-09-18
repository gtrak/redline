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
| `src/model/buffer.rs` | `Buffer.editable` already exists (field + `Buffer::new(.., editable)`); the toggle flips it via the existing `BufferTable::get_mut(key)`. No new setter needed. (CORRECTION: this file DOES exist — my first pre-check claim that it did not was wrong.) |
| `src/app/store.rs` | toggle command, save command + watcher suppression, conflict integration. |
| `src/app/command.rs` / keymap | C-x C-q, C-x C-s bindings. |
| `src/ui/root.rs` | status-line mode indicator. |
| tests + tools/ flows | toggle, save, suppression, conflict, discard-guard. |

## Orchestrator pre-check (2026-09-18, against committed tree)

CORRECTION (2026-09-18): an earlier version of this pre-check claimed
`src/model/buffer.rs` does not exist. That was WRONG — the file is real
(`Buffer` struct with `editable`, `locally_modified`, `changed_on_disk`,
`mark`; `Buffer::new(path, rope, mtime, editable)`). The `Buffers`
collection that owns the map lives in the app layer; check which one holds
the mutable accessor the toggle needs.

Confirmed building blocks already exist; this issue is mostly wiring + one
new suppression path:

- `Buffers::insert_rope(Some(path), rope, mtime, editable)` — the 4th arg
  is the per-buffer editable flag; `open_notes` passes `true`. Toggling a
  file buffer = flipping that flag (add a setter if none).
- `save_buffer()` (`store.rs` ~1458) already saves ANY editable buffer
  with a path (writes `rope.to_string()`, updates `mtime`, clears
  `locally_modified`/`changed_on_disk`, re-highlights). Reusable as-is for
  `C-x C-s`; the only reason notes worked is `editable=true` — a file
  buffer with the flag flipped will save with no new code path.
- **The real gap**: watcher self-write suppression exists ONLY for files
  we *created* (`self.created_paths` + the `created_by_us` logic in
  `open_notes`, store.rs ~1418/5050). There is NO suppression for a path
  we *save in place*, so `C-x C-s` on a real file would false-flag
  "changed on disk". The fix is the named work: record saved paths (and
  ideally expected mtime/content) in a suppression set consulted by the
  reload/conflict path, mirroring `created_paths`.
- External-change → conflict marker already exists (`changed_on_disk`,
  reload/conflict sweep ~5039); edit mode must keep it (do not
  auto-reload a buffer with `locally_modified`).
- `C-x C-q` / `C-x C-s` are unbound today (no collisions) — new Buffer
  keymap bindings + command registrations (palette count updated).

## Verification

- Gates green; PTY: C-x C-q → edit, type, C-x C-s → git sees the change;
  external edit while editing → marker; quit prompts (004-04 machinery);
  read-only buffers auto-reload unchanged.

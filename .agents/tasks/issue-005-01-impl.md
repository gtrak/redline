# Task: plan 005 issue 01 — file edit mode

You are the implementation worker. Repo root is your cwd. This spec is
self-contained. `.agents/skills/*.md` are authoritative ground truth.

Issue contract (`.agents/plans/005-edit-modes/01-file-edit-mode.md` — read
it; it has an orchestrator pre-check confirming the building blocks):
file-backed buffers stay read-only by default; `C-x C-q` toggles the
current buffer into a real file-edit mode; `C-x C-s` saves to disk under
the existing conflict discipline.

## Working agreement

- Skills are truth (ropey for the text model, iocraft for render; no
  registry reads, no docs.rs, no fetch). Write-first; compile early;
  iterate on named errors. Minimal skill corrections, listed. `graft`
  available.

## Read first

1. `.agents/plans/005-edit-modes/01-file-edit-mode.md` — contract + the
   pre-check section (blocking blocks already exist).
2. `src/model/buffer.rs` — the `Buffer` struct (`editable`,
   `locally_modified`, `changed_on_disk` fields) and `BufferTable`
   (`get`/`get_mut`/`list`/`insert_rope`). The toggle flips
   `BufferTable::get_mut(key)?.editable` — no new setter needed.
3. `src/app/store.rs`:
   - `BufferTable::insert_rope(Some(path), rope, mtime, editable)` — the
     4th arg is the per-buffer editable flag (`open_notes` passes `true`).
   - `save_buffer()` now delegates to `save_buffer_key(key)` (004-04) —
     which already saves a NON-current buffer correctly.
   - `save_buffer()` (~1458) — already saves ANY editable buffer with a
     path; updates mtime, clears `locally_modified`/`changed_on_disk`,
     re-highlights. Reuse as-is.
   - `insert_text` / `notes_backspace` (the editable-buffer edit path).
   - `self.created_paths` + the `created_by_us` logic in `open_notes`
     (~1418) and the reload/conflict sweep (~5040) — the ONLY self-write
     suppression today (for files WE created).
   - `changed_on_disk` conflict marker + the reload sweep.
3. `src/app/command.rs` (~112 onward) — command registration pattern.
4. `src/ui/root.rs` — status-line render (where the mode indicator goes).

## What to build

### 1. The toggle
- `C-x C-q` on a file-backed buffer flips `editable` true/false
  (emacs `toggle-read-only`). Add a setter on `Buffers` if none exists.
- **Toggling back to read-only with unsaved edits must not silently lose
  them**: confirm first (reuse the existing discard-guard/confirm
  machinery if one exists; otherwise a minibuffer y/n confirm in the same
  style as the 004-04 save-prompt). Cancel leaves edit mode on.
- Non-file buffers (notes are already editable) and non-buffer views:
  no-op with a minibuffer message. `*scratch*`/home if present: no-op.

### 2. Editing while in edit mode
- Typing/Backspace already work through `insert_text`/`notes_backspace`
  for `editable` buffers — verify a file buffer genuinely accepts them
  (do NOT assume; drive it). Set `locally_modified` on edits (existing).
- **External change while editing**: keep the existing conflict discipline
  — do NOT auto-reload a buffer with `locally_modified`; show the
  changed-on-disk marker. Auto-reload behavior for read-only buffers must
  be unchanged.

### 3. Save + the real gap: watcher self-write suppression
- `C-x C-s` calls `save_buffer()` (already correct for editable buffers).
- **The gap**: suppression exists only for paths we CREATED
  (`created_paths`). A save-in-place must also be suppressed, or our own
  write false-flags "changed on disk". Add a suppression set for saved
  paths (mirror `created_paths`; prefer recording the expected mtime so a
  genuinely-later external change still conflicts). Wire it into the same
  reload/conflict path that consults `created_paths`.
- After a save, the buffer must NOT show locally_modified and must NOT
  show changed_on_disk.

### 4. Presentation + registration
- Status line shows the mode (e.g. `Edit` vs read-only) for the current
  buffer — emacs shows `%%` vs `--`; a word is clearer here.
- Register `toggle-read-only` and `save-buffer` commands if not already
  registered under those names; bind `C-x C-q` (C-x C-s is bound to
  `save-buffer` already). Update palette-count assertions.

## Constraints

- Scope fence: `src/app/store.rs`, `src/app/command.rs`, `src/ui/root.rs`,
  tests, `tools/` flows, `docs/`. No dependency changes; no undo; do not
  alter kill/yank, isearch, region, point motion (05b/05c), or the 004-04
  quit machinery beyond the documented interface.
- All suites green: `cargo test`, `tools/sweep.py` 14/14,
  `tools/sweep_flows.py` 46/46, `tools/drive_all.py` 6/6,
  `tools/drive_windowing.py` 28/28, `tools/drive_windowing_panes.py` 4/4,
  `tools/check_cursor_stream.py` all.
- Wrap EVERY python PTY invocation in `timeout` (e.g. `timeout 600 python3
  ...`) so a hung read cannot stall you.

## Verification (iterate until ALL pass)

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test green.
- Unit tests: toggle on/off; unsaved-toggle confirm (accept and cancel);
  edit sets locally_modified; save clears it; saved-path suppression
  (the watcher's event for our own write does not flag the buffer); a
  genuinely external write after our save STILL flags.
- Raw-PTY legs (new, in `tools/sweep_flows.py` or `check_cursor_stream.py`):
  `C-x C-q` → type → `C-x C-s` → the file on disk changed (assert bytes);
  status line shows the edit mode; toggle back with unsaved edits →
  confirm prompt, cancel keeps edit mode and the text; read-only buffer
  auto-reload still works (open a file, modify it out-of-band, assert the
  view reloads) — and does NOT auto-reload while edited.
- Report the exact git-visible change (`git diff` of the fixture file).

## Report format

Design notes (toggle, suppression mechanism with the mtime choice, status
indicator). Per-command/binding table. Gate outputs (exact counts).
Skill corrections (or none). Deviations; known gaps.

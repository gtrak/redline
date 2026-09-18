# Task: plan 004 issue 05h — buffer-list key consistency (n/p move, d kill)

You are the implementation worker. Repo root is your cwd. Self-contained.

## Reported defect (orchestrator UX sweep, 2026-09-18, verified live)

In the **buffer list** view (`C-x C-b`), navigation and the kill verb do
not match the app's own conventions:

- `n` / `p` echo `unbound key` — but `n`/`p` move the selection in
  magit-status and in the log view. Only arrows / `C-n` / `C-p` / PgUp /
  PgDn work here (`store.rs` ViewId::BufferList arm).
- `d` echoes `unbound key` — emacs/dired convention for "mark for
  deletion", and the parity log row 31 already backlogged: "`d` inside the
  list echoes unbound — bind it as the kill verb per dired convention".

Measured live (each on a fresh app, so no stale-frame confusion):
`C-x C-b` alone → no echo; `C-x C-b` then `n` → `unbound key`; then `d`
→ `unbound key`; `C-x k` inside the list → works (no echo).

Also verified for the parity log: **`C-x b` works** (opens the buffer
picker) — row 6's "re-verify" can be closed.

## Read first

1. `src/app/store.rs` — the `ViewId::BufferList` keymap arm (~170-182:
   `q`/RET/arrows/`C-n`/`C-p`/PgDn/PgUp) and `buffer_list_next` /
   `buffer_list_prev` / `open_buffer_list_selected` / `kill_buffer`.
2. `src/app/store.rs` — the `ViewId::MagitStatus` arm for the `n`/`p`
   convention (`magit-next`/`magit-prev`).
3. `docs/emacs-parity-log.md` — row 31 (the `d` backlog) and row 6
   (`C-x b` re-verify).

## What to build

1. Bind `n` → `buffer-list-next`, `p` → `buffer-list-prev` in the
   buffer-list arm (same commands as the existing arrow/C-n/C-p binds) —
   consistency with magit/log.
2. Bind `d` → a kill action for the SELECTED buffer, following the
   existing `C-x k` path in this view (whatever `C-x k` dispatches to when
   the list is focused — reuse that command rather than inventing a
   second kill path). Guard the last-buffer case exactly as `kill_buffer`
   already does (it falls back to a fresh `*scratch*` today — see note).
   `d` must NOT quit or close the list; after killing, the selection
   should land on a valid row (clamp).
   NOTE: 004-06 (discovery home) will later remove `*scratch*`; do not
   depend on scratch existing — just call the existing kill command.
3. Update the buffer-list help line if it lists keys (the view renders a
   key-help footer similar to magit's) so `n/p`/`d` are discoverable.
4. Update the parity log: row 31's `d` backlog → DONE; row 6 note that
   `C-x b` was re-verified working.

## Constraints

- Scope fence: `src/app/store.rs` (BufferList arm + a command
  registration if the kill verb needs one), `docs/emacs-parity-log.md`,
  tests. No dependency changes; no changes to other views' keymaps.
- All suites green: `cargo test`, `tools/sweep.py`, `tools/sweep_flows.py`,
  `tools/drive_all.py`, `tools/drive_windowing.py`,
  `tools/drive_windowing_panes.py`, `tools/check_cursor_stream.py`,
  `tools/ux_sweep.py` (should report zero findings for buffer-list keys).
- Wrap EVERY python PTY invocation in `timeout`.

## Verification (iterate until ALL pass)

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test green.
- Unit: `n`/`p` resolve to the same commands as `C-n`/`C-p` in the
  BufferList map; the kill command removes the selected buffer and clamps
  the selection.
- Raw-PTY legs (add to `tools/sweep_flows.py`): open `C-x C-b`, press `n`
  → no `unbound key` echo and the selection moved (assert the highlighted
  row changed); press `d` → the buffer count drops by one and the list
  stays open; `q` still closes.
- `python3 tools/ux_sweep.py` → zero buffer-list findings.

## Report format

Binding table for the buffer-list view (before → after). Gate counts.
Parity-log rows updated. Deviations.

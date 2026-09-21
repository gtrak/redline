# Task: 015-03 — accurate mode: insert at point, backspace-before-point, RET, C-k

The user's cutline for accurate mode. **Depends on 015-02** (the honest point + the
mode flag) — do not start this until 02 has landed.

The commands, in the user's priority order:

| # | Command | Binding | Today |
|---|---|---|---|
| 1 | insert at point | self-insert | appends at the **end of the buffer** |
| 2 | delete before point | `BACKSPACE` / `C-h` | deletes the **last char of the buffer** |
| 3 | **newline** | `RET` | **unhandled in the Buffer view** |
| 4 | **kill line** | `C-k` | missing |

`RET` and `C-k` are free in the Buffer view's keymap table (`src/app/store/mod.rs` — the
`"RET"` bindings there belong to *other* views: BufferList, MagitStatus, Log, Search).

## The shape: mode-aware operations, not new machinery

Today's coarse behaviours are **already explicit** and live in exactly two places:
- `insert_text` (`buffers.rs:12`) — `pos = rope.len_chars()`, appends at the end;
- `notes_backspace` (`notes.rs`) — `remove((len-1)..len)`, deletes at the end.

So this issue adds **point-accurate counterparts** and selects between them by the
per-buffer mode (015-02). Suggested shape (judge it yourself):

- `insert_text_at_point(&mut self, text: &str)` — insert at the honest point byte
  (char-converted), advancing the point by the inserted length.
- `delete_char_before_point(&mut self)` — remove the char before the point, moving the
  point back; **no-op at the buffer start** (emacs behaviour).
- `newline_at_point(&mut self)` — insert `"\n"` at point (which is just
  `insert_text_at_point("\n")` — say so rather than duplicating).
- `kill_line(&mut self)` — kill from the point to end-of-line **and push it to the kill
  ring** (emacs `C-k` kills, so `C-y` can yank it back; the kill ring already exists with
  `kill_region`/`copy_region`/`yank`/`yank-pop`). Decide and state the end-of-line
  behaviour: emacs kills to EOL, and at EOL kills the newline itself (joining lines) —
  implement that or state the deviation.

Then the dispatch in the notes-edit modal guard (`notes_edit_key_event`, `keys.rs`) and
the keymap: in `Accurate` mode route self-insert/Backspace/`RET`/`C-k` to the
point-accurate functions; in `Annotation` mode keep today's behaviour **exactly**.

## Key decisions

- **Annotation mode must not change.** Every current coarse behaviour stays byte-for-byte
  for non-accurate buffers. If a change leaks into annotation mode, that is a P1.
- **All four are edits: they require `editable`** (the existing gate) and must set
  `locally_modified`, `retain_rope_edit` (the incremental-reparse record) and
  `invalidate_highlight_for_key` exactly as the existing edit paths do. **Missing any of
  those three is a P1** — they are how the highlight cache and the watcher stay correct.
  Copy the pattern from `insert_text`/`kill_region`, do not invent it.
- **The mark after an edit.** `insert_text` currently clears the mark ("the text
  insertion shifts byte offsets"). Decide per-command: emacs keeps the mark and *moves*
  it when text is inserted before it. Keeping the current "clear the mark" is acceptable
  if stated; silently leaving a stale byte offset is not.
- **Kill-line and the kill ring**: reset the yank-pop state (`yank_pos`/`yank_len`/
  `yank_ring_index`) exactly as `kill_region` does, or `M-y` will cycle against a stale
  entry.
- **Do not touch** `kill_region`/`copy_region`/`yank` (04 owns yank) or `C-d`/`C-u`
  (they collide with scroll bindings and are **not** in this cutline).
- **Bindings:** `RET` → newline, `C-k` → kill-line, added to the Buffer view's
  declarative table. Self-insert and Backspace are handled by the modal guard, not the
  table — keep it that way.

## Files

`src/app/store/buffers.rs` (the new point-accurate operations), `src/app/store/notes.rs`
(the guard's routing), `src/app/store/keys.rs` (`notes_edit_key_event`),
`src/app/store/mod.rs` (the Buffer table: `RET`, `C-k`), `src/app/command.rs`
(registration for `kill-line`), tests (`tests/buffers.rs`, `tests/notes.rs`,
`tests/keys.rs`).

## Verification

- `cargo build`; `cargo test --workspace` — reconcile against the current baseline
  (measure it; after 02 it will have moved) and account for every change.
- `cargo clippy --workspace --all-targets -- -D warnings` (read `${PIPESTATUS[0]}`).
- **Tests to add** (each must discriminate — a col-0 / end-of-buffer fixture cannot):
  (a) insert mid-line lands at the point and advances it; (b) backspace mid-line removes
      the *preceding* char and moves the point back; (c) backspace at the buffer start is
      a no-op; (d) `RET` splits the line at the point; (e) `C-k` kills to EOL and pushes
      to the kill ring (`C-y` then re-inserts it); (f) **in annotation mode the old
      behaviour is unchanged** (the append/backspace-at-end path) — the regression guard
      for the whole issue; (g) a multibyte case for the point arithmetic.
- **`timeout 900 tools/gate.sh full`** — typing is PTY-visible; the battery matters.
- Report: each command's implementation, the end-of-line choice for `C-k`, the mark
  decision, the three edit-hygiene calls per command, the annotation-mode regression
  test, and the gate output.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.

## Folded in from the 015-02 gate: the baseline predicate is root-relative

`buffer_baseline_editable` (`src/app/store/buffers.rs`) decides "was this buffer
editable before Accurate?" as `buf.path.is_none() || notes_key() == key`. That is
**root-relative, not buffer-relative**, and the 015-02 gate demonstrated the drift
by execution: open the notes buffer → `C-x C-q` (Accurate) → `switch_project_root`
to another project → the old notes buffer **survives in the table** but the
predicate now returns `false`, so leaving Accurate on it yields `Annotation` +
**read-only** ("read-only (C-x C-q to edit)"). The `Accurate ⟹ editable` invariant
still holds, so this is not a correctness bug — but it contradicts the doc's claim
that "the baseline is what the buffer IS, so it cannot drift with session state",
and it is the kind of surprise that makes a user think the editor lost their mode.

**Fix it here** (this issue already touches the same files): use the per-buffer
`kind`/`is_notes` flag that `issue-015-02-point-and-modes.md` itself offered as the
cleaner mechanism, or compare against the notes path recorded when the buffer was
opened. Then **correct the doc** so the narrow claim (save/reload) and the broader
claim match reality, and pin the cross-project-root case with a test — the gate's
probe is the reproduction.

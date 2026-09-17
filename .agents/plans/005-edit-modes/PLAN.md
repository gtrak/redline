# 005 — Redline: per-buffer edit modes (file vs annotate)

Status: planned
Phases: 1 · Issues: 01–03
Depends on: plan 004 complete (save machinery, kill/yank, quit save-prompt)

## Why

User directive 2026-09-17: "I want 2 edit modes selectable per buffer.
file-based vs notes/annotation. Notes get dumped on quit with line numbers,
so the user can redirect to a file or copy-paste. And we can actually edit
files."

Redline is read-focused today: file buffers are read-only (auto-reload on
external change) and editing exists only in notes/scratch. This plan makes
the per-buffer edit mode explicit and adds real file editing plus a
stdout annotation dump that fits shell workflows.

## What

1. **File-edit mode (per buffer)**: file-backed buffers stay read-only by
   default; `C-x C-q` (emacs toggle-read-only muscle memory) toggles the
   current buffer into file-edit mode. While editing: the rope is
   editable, locally-modified discipline applies (external change →
   changed-on-disk marker → plan-004-04 save-prompt on quit), and
   `C-x C-s` saves to disk. Our own save must not false-flag the watcher
   (extend the event-suppression machinery to saved paths). Mode shown in
   the status line.
2. **Annotate mode (per buffer)**: `A` on a file line opens/edits a
   line-anchored annotation — stored as `path:line: text` in
   `.redline-notes.md` (the notes buffer stays the annotation surface;
   opening an annotation focuses that entry). Annotations anchor to the
   line under the cursor when created.
3. **Quit dump**: after the TUI tears down, all annotations print to
   stdout as `path:line: text` (grep -n style — redirect or copy-paste).
   Empty when none (clean for pipes). Dump reflects the final state
   (after plan-004-04 save prompts).

## Key decisions

- `C-x C-q` for the edit toggle (emacs read-only-toggle vocabulary, no new
  key class); `C-x C-s` for save (universal emacs muscle memory).
- Annotations are line-anchored records, not free text: the dump needs
  line numbers, so the anchor is captured at creation (path + line).
- Dump goes to stdout AFTER the alternate screen exits — TUI output stays
  on the terminal, dump is pipable.
- Read-only by default remains the app's identity: edit mode is explicit,
  per-buffer, and visibly indicated.
- Design details (exact key choices, dump format, notes-file section
  format) are preference-level: implemented as stated here, user can veto
  in review of the live app.

## Success criteria

- A file buffer toggled to edit mode accepts edits and `C-x C-s` writes
  them to disk (git sees the change); toggling back to read-only restores
  auto-reload behavior.
- `A` on a line creates an annotation; quit prints `src/main.rs:42: text`
  on stdout; `redline 2>/dev/null | grep main.rs` works.
- Full drive suite green; no regressions to the watcher conflict
  discipline or the quit save-prompt.

## Task order

| Phase | Issues | Depends on |
|---|---|---|
| 1 | 01 file edit mode | plan 004 complete |
| 1 | 02 line-anchored annotations | 01 (store/buffer state) |
| 1 | 03 quit dump | 02 |

## Issue index

- [01 — file edit mode](01-file-edit-mode.md)
- [02 — line-anchored annotations](02-line-annotations.md)
- [03 — quit dump](03-quit-dump.md)

When complete, archive per plan-process.

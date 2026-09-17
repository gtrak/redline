# 02 — line-anchored annotations

Phase 1 · Depends on: 01

## Objective

A on a file line creates/edits a line-anchored annotation stored as
`path:line: text` in .redline-notes.md; the notes buffer remains the
annotation surface.

## Key decisions

- `A` in the file view anchors an annotation to the line under the cursor
  and opens the notes buffer focused on that entry (free notes text
  outside `path:line:` lines is preserved untouched).
- The notes file gains a structured annotation section; parsing is
  tolerant (unknown lines kept as-is).
- Mode indicator: annotate buffers show as annotations in the status line.

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | annotation model (parse/store/render), A handler, notes-buffer focus. |
| `src/ui/*` | annotation rendering in notes; marker in the file-view gutter if cheap. |
| tests + tools/ flows | create/edit/delete annotation, file persistence, dump ordering. |

## Verification

- Gates green; PTY: A on line → entry appears with path:line; edit +
  re-open shows persisted text; external note file edits keep conflict
  semantics.

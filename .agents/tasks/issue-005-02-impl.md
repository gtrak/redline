# Task: plan 005 issue 02 — inline annotations (visual cues + automatic linking)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained. `.agents/skills/*.md` are authoritative ground truth.

Issue contract (`.agents/plans/005-edit-modes/02-line-annotations.md` —
read it, INCLUDING the "row map is the hard part" risk note): annotate the
file you are browsing **inline** — `A` anchors an annotation to the line at
point, which is shown as a visible cue in the FILE view; storage is a real,
editable notes file; anchors maintain themselves by content.

User intent, verbatim (2026-09-18): "I want to annotate the file I'm
browsing inline, but having it as a real file buffer is ok. So I need some
visual cues within an editor buffer, and automatic linking machinery. I
would probably just pass the notes into an agent, so enough information
there that it'll know what to do."

## Working agreement

- Skills are truth (ropey, iocraft; no registry reads, no docs.rs, no
  fetch). Write-first; compile early. Minimal skill corrections, listed.

## Read first

1. `.agents/plans/005-edit-modes/02-line-annotations.md` — contract + risk
   note (the rendered-row map is the hard part; `mouse_click_position`
   does `scroll_top + row` and must become map-aware).
2. `src/ui/file_view.rs` — the canvas renderer, `FileViewLine`, the
   viewport pre-computation, `region_lines` background painting.
3. `src/ui/root.rs` — `cursor_cell` (row = `point_line − scroll_top +
   title_offset`), the Snapshot field plumbing (region_lines precedent).
4. `src/app/store.rs` — `FileViewLine` viewport build, `open_notes`
   (~1400, creates/loads `.redline-notes.md`), `insert_text`, the
   point/scroll helpers (`set_point`, `scroll_top`, `keep_cursor_visible`),
   `mouse_click_position` (~2874), `menu`/registry patterns.
5. `src/syntax/` — how byte↔char offsets are handled (reuse the existing
   `byte_to_char` helpers; the file view is char-indexed).

## What to build

### 1. Annotation model (store)
- `Annotation { path: String, line: usize, col: usize, anchor: String,
  text: String, orphaned: bool }`. `anchor` = the exact text of the
  anchored line when the note was created.
- Persist in `.redline-notes.md` in a **tolerant structured section**;
  free text outside the section is preserved verbatim. Malformed records
  are kept verbatim, never dropped.
- Parse on load `open_notes`; look up by file path for the view.

### 2. Automatic anchoring
- On load/reload (and on 005-01 edit-mode save), for each annotation whose
  stored line no longer holds `anchor` exactly, search ±25 lines: unique
  match → re-anchor (update `line`); no match → set `orphaned = true`
  (NEVER silently move to a guessed line).
- Re-anchoring must be stable and idempotent.

### 3. Inline visual cues (the point of the feature)
- Every annotated line renders a **margin marker** (e.g. `▎` at the left
  edge) — always visible.
- The note text renders as a **dim/italic virtual row directly under the
  anchored code row** (e.g. `    ▸ note text`), toggled by `C-c a`.
- **Row-map correctness (the hard part — see the risk note)**: the
  viewport becomes an ordered list of rendered rows (code row → buffer
  line, or note row). `cursor_cell` AND `mouse_click_position` must
  translate buffer_line ↔ rendered_row both ways. Do not fudge offsets.

### 4. `A` flow
- `A` on a line prompts for note text in the **minibuffer**; RET commits
  (writes the record + shows the cue immediately). `A` on an annotated
  line pre-fills for edit. `C-u A` (or `d` on an annotated line) deletes.
- Opening the notes buffer shows the same records as ordinary editable
  text (the real-file surface).

### 5. Presentation
- Status line shows the current file's annotation count.
- Register `annotate` (+ `annotate-delete`, `annotate-toggle`) commands;
  update palette-count assertions.

## Constraints

- Scope fence: `src/app/store.rs`, `src/app/command.rs`,
  `src/ui/file_view.rs`, `src/ui/root.rs`, tests, `tools/` flows, `docs/`.
  No dependency changes; no undo; do not alter kill/yank, isearch, region,
  quit machinery, or 005-01's save path beyond calling the re-anchor pass.
- All suites green (counts change where palette/keys change): `cargo test`,
  `tools/sweep.py`, `tools/sweep_flows.py`, `tools/drive_all.py`,
  `tools/drive_windowing.py`, `tools/drive_windowing_panes.py`,
  `tools/check_cursor_stream.py`.
- Wrap EVERY python PTY invocation in `timeout`.

## Verification (iterate until ALL pass)

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test green.
- Unit tests: record parse/serialize round-trip; tolerant parse keeps
  malformed lines; re-anchor on drift; orphan flag when the anchor is
  gone; ±25 bounds; delete; `A` prefill; the rendered-row map
  (buffer_line ↔ rendered_row) round-trips with interleaved note rows.
- Raw-PTY legs: `A` on a line → a `▎` marker on that code row AND the note
  text on the row below; `C-c a` hides/shows note rows without moving the
  marker; with note rows visible, `C-n`/`C-p` land on CODE rows with the
  correct buffer line in the status (map correctness); clicking a code row
  under an annotation maps to the right line; edit the file out-of-band so
  the anchored line moves → annotation re-anchors by content; delete the
  anchored line → orphan flag, annotation NOT lost.
- The notes file remains a valid editable buffer; external edits to it
  keep conflict semantics.

## Report format

Model + storage format. The rendered-row map design (both directions).
Per-command/binding table. Gate outputs (exact counts). Skill corrections
(or none). Deviations; known gaps.

# 02 — inline annotations (visual cues + automatic linking)

Phase 1 · Depends on: 01

## Objective

Annotate the file you are browsing, **inline and in context**. `A` anchors
an annotation to the line at point; the annotation is stored in an
editable, real notes file AND rendered as a visible cue **in the file
view**, so reading a file shows its annotations. Anchors maintain
themselves automatically (content-based re-anchoring on drift).

User intent (2026-09-18): "I want to annotate the file I'm browsing
inline, but having it as a real file buffer is ok. So I need some visual
cues within an editor buffer, and automatic linking machinery. I would
probably just pass the notes into an agent, so enough information there
that it'll know what to do."

## Key decisions

- **Anchor = (path, line, col, anchored-line text).** The captured line
  text is the drift anchor. On open/reload, if the stored line no longer
  holds the anchor text, search ±25 lines: match → re-anchor silently;
  no match → keep the annotation but flag it **orphaned** (never move an
  anchor blindly to a wrong line).
- **Inline cues in the file view** (the "visual cues within an editor
  buffer"):
  - Every annotated line shows a **margin marker** (e.g. `▎`) — always
    on, the navigational cue.
  - The note text renders as a **dim/italic virtual line directly under
    the anchored line** (diff/overlay style, e.g. `  ▸ note text`).
  - Toggle: `C-c a` shows/hides the overlay note lines (markers stay).
  - The file view is **virtualized**: inserting virtual rows must extend
    the rendered-line map so `point_line ↔ rendered row` stays exact
    (04-05b/c math). Do NOT hack offsets into the renderer — fix the
    map, and add a cursor/cue regression leg for annotated files.
- **Storage**: the notes file stays a real, human/agent-editable buffer
  (`.redline-notes.md`), gaining a tolerant **structured annotation
  section** (one record per annotation: path, line, anchor text, note).
  Free text outside the section is preserved untouched. Malformed records
  are kept verbatim, never dropped.
- **`A` flow** (fast path + real-file path):
  - `A` on a line prompts for the note text in the **minibuffer** (the
    common case is a one-liner instruction). RET commits → the record is
    written to the notes file and the cue appears immediately.
  - `A` on an already-annotated line edits that annotation (pre-filled).
  - `C-c a`… / opening the notes buffer shows the same records as
    ordinary editable text (the "real file buffer" surface) for longer
    editing.
  - `C-u A` (or a delete key on an annotated line) removes the annotation.
- **Automatic linking** also runs when an edit-mode buffer is **saved**
  (issue 01), re-anchoring that file's annotations against the saved
  content in the same pass.
- Status line shows the current file's annotation count.

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | annotation model (parse/serialize/anchor/re-anchor), `A` handler, minibuffer note entry, delete, per-file annotation lookup for the view. |
| `src/ui/file_view.rs` | margin marker + virtual note lines in the rendered-line map (point↔row math preserved). |
| `src/ui/root.rs` | snapshot fields for current-file annotations; status-line count. |
| `src/app/command.rs` | `annotate` (+ delete) commands; palette counts. |
| tests + `tools/` flows | create/edit/delete; persistence round-trip; drift re-anchor + orphan flag; marker + virtual-line rendering; point math on annotated files. |

## Verification

- Gates green; `cargo test` adds: anchor parse/serialize round-trip,
  tolerant parse (malformed kept), re-anchor on drift, orphan flag,
  ±25-line search bounds, delete, `A` prefill.
- Raw-PTY legs: `A` on a line → marker `▎` on that row AND the note line
  under it; `C-c a` hides/shows overlays without moving the marker;
  `C-n`/`M-f` over an annotated file land the cursor where the *code*
  line is (point↔row math with virtual rows); edit the file elsewhere →
  annotation survives by content anchor; delete an anchored line →
  orphaned flag, annotation not lost.
- The notes file remains a valid editable buffer (01 + save machinery);
  external edits to it keep conflict semantics.

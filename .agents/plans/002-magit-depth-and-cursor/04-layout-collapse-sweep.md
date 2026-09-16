# 04 — Layout collapse sweep (canvas views overprint titles)

Phase 1 (parallel to 01) · Depends on: —

## Objective

Row-content views (magit status, tree, log, blame, results) render their rows
over the view title/section headers — the same missing
`flex_direction: Column` + definite-height class FileView had (issue 03/09).
Until fixed, magit reads as garbage and cursors are buried.

## Steps

1. Audit every canvas/rows view container: title row + content rows must be
   siblings in a Column with the content area flex_grow and definite height.
2. Fix magit status, tree, log, blame, results (whichever collapse — pyte
   verify each: title on its own row, content below, no overlap).
3. Store-level + pyte regression checks per view.

## Verification

- pyte reconstruction at 100x30: title row distinct; section headers on
  their own rows; no overlapping text columns.

# 05 — Editable-buffer keys & content coherence

Phase 2 · Depends on: —

## Objective

Editing surfaces must not break the command system, and the content set must
cohere.

## Steps

1. Editable-buffer interception: printable keys self-insert ONLY when they
   do not continue an armed/possible prefix sequence; C-x/C-c prefixes and
   C-g must work while editing (mirror the commit editor's interception,
   which got this right for C-c).
2. Notes false conflict: app-created notes file must not flag
   "changed on disk" (suppress the self-inflicted watcher event).
3. graft cards out of the file finder (keep them greppable via .ignore for
   search only); or rank real source above cache cards + label them.
4. Demo commands: remove demo-message-1/2, insert-demo-text, message-echo
   from the seed registry (or gate behind a dev feature).
5. q semantics: bare q in the main Buffer view must not quit the app
   (close-view/no-op instead); C-x C-c remains the quit.

## Verification

- Store tests per interception rule; pty: C-c p f works while a notes buffer
  is open and focused; notes typing still inserts; q in Buffer view does not
  exit; finder lists src/main.rs above graft cards for query "main".

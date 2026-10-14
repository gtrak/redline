# Task spec — W13: drop the file view's duplicate title row

## User directive (2026-10-02)

> "I don't want the file name at the top of the window, it's at the bottom
> anyway."

## Problem

`src/ui/file_view.rs:726` renders `props.title` (the buffer's display name)
as the **first row** of the file pane. The status line at the bottom already
shows it (`src/ui/root/widgets.rs:79`: `* {project} *  {view}  {mode} …`).
Live frame proof:
```
Cargo.toml              <- file-view title row (DUPLICATE)
[workspace]
members = ["crates/*"]
... (bottom) ...
* red *  Cargo.toml  Read-only  Top   <- status line (keep this)
```

## Goal

Remove the file view's title row. **Scope: the FILE VIEW ONLY** (the user's
directive). Do NOT touch Blame/Log/Search/Magit rows — their titles carry
information the status line does not (search counts, blame/log subject,
magit section labels).

## The blast radius (every `+1` that encodes the title row)

Removing the row **shifts every file-view content row up by one**. Fix all of
these in the same commit (grep for the comments to find them):

1. **`src/ui/file_view.rs`** — delete the `Text(content: &props.title, …)`
   element at ~line 725 (keep the `changed_on_disk` banner element — it
   stays, and it is now the only optional row above the canvas).
   The `title` prop may become unused: if so, remove it from
   `FileViewProps` and its two call sites in `src/ui/root/render.rs`
   (`file_view_element`, and the half-screen search overlay's top half),
   and from `src/ui/root/mod.rs` tests that pass it. Keep the
   `view_title` FACE only if still used elsewhere; otherwise leave the
   theme untouched (do not delete theme faces).
2. **`src/ui/root/geometry.rs`** `cursor_cell`, `ViewId::Buffer` arm:
   `Some((col, 1 + banner + content_row))` → `Some((col, banner + content_row))`
   (the `1` WAS the title row). Update the doc comment block at the top of
   the file (it says "the main view's title is row 0 and its first content
   row is row 1").
3. **`src/ui/root/hooks.rs`** mouse handlers (TWO sites, ~line 77 and ~113):
   `(mouse.row as usize).saturating_sub(1)` → the title offset is gone, so
   the row is now `mouse.row` directly. Update both comments. **BUT** the
   file-view content also starts below the `changed_on_disk` banner when it
   is present — check whether the existing code already accounted for the
   banner here or only the title; do not regress the banner case.
4. **`src/app/store/file_view.rs`** — `mouse_click_position` and
   `click_target_line` doc comments say "the caller subtracts the file
   view's title-line offset" (lines ~150, ~327). Update the comments (the
   caller no longer subtracts it). The row arithmetic itself lives in the
   caller.
5. **Static-render tests that pinned the title row** — update them, keeping
   the assertion STRONGER or equal, never weaker:
   - `src/ui/file_view.rs::tests` — `assert!(s.contains("src/main.rs"),
     "title missing")` (~1245): the title no longer renders; re-express as
     "the status line carries the name" IF that test renders the whole root;
     if it renders only the FileView, assert the buffer CONTENT is present
     instead and delete the title claim (say so in the report).
   - the `code_idx` tests (~1628, ~1906, ~2036, ~2106): the comments say
     "title + line 0 code + 2 notes"; with the title gone the expected index
     shifts by −1. Update the index arithmetic AND the comment; the
     assertions (note rows precede the code row, in order) must stay
     load-bearing.
   - `src/ui/root/mod.rs` tests that assert a frame title.
6. **PTY drives** — any drive that maps a terminal row to a buffer line via
   a title offset. `tools/drive_*.py` comments mention "row 0 = the file
   title" (`drive_all.py:112`, `drive_isearch_list.py:78`, `drive_redo_live.py:68`,
   etc.). Grep `tools/` for `title` and for hardcoded row numbers in
   FILE-VIEW suits (search/xref/symbol/annotations), and shift them. NOTE:
   the search/magit/log/tree/buffer-list views keep THEIR title rows, so
   only file-view row maps move.

## Evidence to produce

- The live frame BEFORE/AFTER (the `quote` verb makes this easy:
  `target/debug/redline --project . -c 'open Cargo.toml' quote | head -3`)
  showing the file view now starts with the first code line, and the bottom
  status line still naming the file.
- The CUP (hardware cursor) row for a known point is now one row higher —
  prove it with `tools/check_cursor_stream.py` (or the relevant cursor leg).

## Verification

```
cargo build --workspace > /tmp/b13.log 2>&1; echo BUILD=$?
cargo clippy --workspace --all-targets -- -D warnings > /tmp/c13.log 2>&1; echo CLIPPY=$?
cargo test --bin redline > /tmp/t13.log 2>&1; echo TEST=$?
timeout 1800 tools/gate.sh full > /tmp/g13.log 2>&1; echo GATE=$?
```
Every shared suite OK (the fixture is now reproducible — see
`issue-fixture-reproducible-impl.md`; if it is not yet landed, fixture-
dependent suites may still fail at setup: note that and run the rest).

## Constraints
- Do not remove theme faces (`view_title` is used by other views).
- No assertion may be weakened; a shifted index is re-expressed exactly.
- Do not touch non-file-view titles.
- Commit at first green.

## Report
Files changed with the row-arithmetic deltas; the before/after frames; the
cursor-row proof; the gate table; any drive row-map you moved (before/after);
deviations.

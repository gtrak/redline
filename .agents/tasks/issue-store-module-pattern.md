# Task: establish the store module pattern + the two safe extractions (012-02)

You are the lane that turns `src/app/store.rs` into a directory module. This is the
pattern-setting stage of plan 012 phase 2; every later concern move follows it.

## Step 0 (mandatory, before any move)
The concern map `docs/architecture.md` was measured at commit `acd974e` and its
**line numbers are stale** — `store.rs` gained 45 lines / lost 1 when the
picker-density work landed (`6d01393`), so it is now ~23,037 lines. Re-run the
census against the CURRENT file and update the map's line numbers before moving
anything. The invariants to re-establish (they must hold after your update):
- `impl AppStore` has **421 methods = 253 `pub` + 168 private** (note: two of them,
  `collect_syntax_anchor_nodes` and `is_syntax_anchor_kind`, are formatted at
  **column 0**, so an indent-based count finds only 419 — do not "fix" the count to 419);
- the per-concern ranges are **disjoint** and cover every method line exactly once;
- the per-module name lists reconcile (no duplicates; union = the methods + the
  assigned free fns).
Write the re-derived numbers back into `docs/architecture.md` (line numbers only;
the counts are stable).

## The move
`src/app/store.rs` → `src/app/store/mod.rs`, then add the submodule files. Two traps
the map documents — do not hit them:
1. `#[path = "flow_tests.rs"]` must become **`#[path = "../flow_tests.rs"]`** (the
   attribute resolves relative to the containing file's directory; the verbatim move
   fails with E0583).
2. Submodules must be **children** of the module defining `AppStore` (i.e. under
   `app::store::*`), which is what makes the private fields reachable with **no
   visibility pass**. Private *methods* called across concern boundaries will need
   `pub(super)` — that is expected and additive, but only add it where the compiler
   actually demands it, in the stage that needs it.

## Stage A — free helpers (do this first; lowest risk)
Move the module-level free fns out of the impl's file into `src/app/store/helpers.rs`:
per the map these are `reload_anchor`, `pane_window`, `window_slice`,
`keep_cursor_visible`, `recenter_top_for`, `point_byte_offset`, `log_entry_display`,
`blame_line_display`, `prefill_commit_message`, `extract_commit_message`,
`editor_cursor_line`, `file_candidate` (confirm the exact set by reading; the map's
line numbers are being re-derived in Step 0). Re-export anything the rest of the
crate imports (`pub use self::helpers::…` in `mod.rs`) so no external path changes.

## Stage B — notes doc (after Stage A is green)
Move the notes-doc parse/serialize machinery into `src/app/store/notes_doc.rs`:
`NotesDoc` and `parse_notes` / `parse_notes_section` / `parse_record_block` /
`serialize_notes` (confirm by reading). These are already store-free; if any of them
turns out to need `&AppStore`, stop and report instead of forcing it.

## Rules
- **Behavior-preserving only.** No logic edits, no renames beyond the module move,
  no "while I'm here" cleanups. Each stage is a move and must land gate-green on its own.
- **Honest-stop at half budget**: if you are not through Stage B, commit what is
  green (Stage A alone is a valid landing) and report exactly where you stopped.
- Do NOT start the test-module split (that is a later lane), do NOT move methods out
  of the impl yet, and do NOT touch any file outside `src/app/store*` +
  `docs/architecture.md`.

## Gate
`cargo build` first (the module move must compile before anything else), then
`cargo test --workspace`, `cargo clippy --workspace --all-targets` (read
`${PIPESTATUS[0]}`), and `tools/gate.sh full` after each stage.
Budget ~50 tool calls. Report: the re-derived census, the Stage A/B file contents,
the trap handling (especially the `#[path]` change), gate counts per stage, where you stopped.

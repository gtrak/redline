# Architecture — module map and the `store.rs` split plan

Plan 012, issue 01. All line numbers below were **re-derived against the
CURRENT** `src/app/store.rs` (23,126 lines); the original measurement was at
**`acd974e`** (22,992 lines). This file is the shared map for every later 012
stage and for reviewers.

> **The map is baseline-pinned — re-derive before executing.** The numbers
> below were re-derived (012-02, Step 0) after `store.rs` moved: the
> picker-density landing (`6d01393`) added 45 / removed 1 lines and a later
> model change (`e2e1221`) touched the test module. The invariants were
> re-verified: the `impl` method count (421 = 253 `pub` + 168 private), the §3
> ranges are disjoint and cover every method line exactly once, and the
> per-module name lists reconcile (no duplicates; union = the methods + the
> 12 assigned free fns). The **impl-method counts** are stable; note the
> test-fn count drifted 389 → 401 (350 `#[test]` + 22 `#[tokio::test]` +
> 29 helpers) when model work landed tests. Re-run the census against the
> current file and update this map if `store.rs` moves again.

## 1. Crate module map

```
crates/
  redline-resolve/        1,426  background M-. workspace fall-through (cargo metadata, lockfile)

src/
  main.rs                  363  entrypoint: config, watcher startup, bus drain tasks, event loop
  perf.rs                  134  timing helpers
  theme.rs                 306  Theme / palette

  app/
    command.rs            1,039  CommandRegistry (M-x name → closure)
    config.rs              267  TOML config load/validate
    events.rs              201  ChangeBus / ProjectChange (project-change bus)
    flow_tests.rs         3,624  PTY-style flow tests, hung off store/mod.rs via #[path = "../flow_tests.rs"]
    keymap.rs              628  KeymapEngine, KeySeq, C-x/C-c prefixes
    store/
      mod.rs            13,701  struct + 18 core methods + the test module (see §3)
      helpers.rs           172  free fns extracted in 012-02 Stage A
      notes_doc.rs          ~  NotesDoc parse/serialize, 012-02 Stage B
    watcher.rs             656  ActiveWatcher (notify-based, debounced)

  git/
    blame.rs               246 · commit.rs 231 · diff.rs 272 · error.rs  (small)
    log.rs                 341 · refs.rs   268 · status.rs 101
    repo.rs               1,713  GitRepo facade (status/staging/log/blame/commit)

  model/
    buffer.rs              596  BufferTable + ropey-backed buffers (SCRATCH)
    files.rs               292  FileList (ignore-aware project walk)
    project.rs             364  Project root detection, ProjectStore (recents)
    sections.rs            761  MagitRow / StatusTree (status section model)
    text_width.rs          111  width-safe string truncation

  nav/
    index.rs              1,075  SymbolIndex, background indexing, IndexBus

  search/
    occur.rs               178  in-buffer occur
    references.rs          257  cross-file reference search
    rg.rs                 1,039  ripgrep process wrapper + SearchBus

  syntax/
    cache.rs               407  HighlightCache (path/mtime/theme keyed)
    highlight.rs          1,158  tree-sitter → theme highlight rows
    language.rs             791  the LanguageSpec table — single source of truth
    node.rs               2,038  per-language tree walking (012/09 target)
    queries.rs            1,653  extraction engine (consts moved to language.rs)
    registry.rs             284  GrammarRegistry, built from the table
    tokens.rs               300  token classification

  ui/
    blame_view.rs · commit_editor.rs · diff_view.rs · home_view.rs · log_view.rs
    magit_status.rs        172 · picker.rs 193 · results_view.rs 292
    rows_view.rs           168 · transient_menu.rs 259 · tree.rs 160
    file_view.rs           581
    root.rs                1,752  Root widget: render loop, bus drains, input routing
    views/buffer.rs        102
```

## 2. `store.rs` measured structure

| Region | Lines | Contents |
|---|---|---|
| 1–1444 | 1,444 | helper types + free fns (see below) |
| 1445–1715 | 271 | `pub struct AppStore` — **83 fields: 19 `pub`, 64 private** |
| 1717–11638 | 9,922 | single `impl AppStore { … }` — **421 methods (253 `pub`, 168 private; 2 private methods are formatted at column 0 — lines 2618/2643)** |
| 11639–11811 | 173 | 11 module-level **free fns** (assigned to core/commit/picker in §3) + the `EditorMove` enum (11669 → commit) |
| 11813–11817 | 5 | `impl Default for AppStore` (delegates to `new`) |
| 11819–11843 | 25 | `impl Picker` (`recompute`) |
| 11844–11854 | 11 | free fn `point_byte_offset` (→ navigation, §3) |
| 11856–23119 | 11,264 | `#[cfg(test)] mod tests` — **401 top-level items = 379 plain `fn` + 22 `async fn`**, plus **11 nested** (9 `fn git_cli` variants + 2 nested `main`s) = 412 total fns. Earlier figures here (389, 401, 379) were different denominators for the same set — this is the reconciled census, re-derived by the store-tests lane and its gate. **LANDED**: the block is now `src/app/store/tests/*.rs` (14 top-level files + `navigation/`, 7 files after A7) with `tests/mod.rs` holding the shared fixtures; `mod.rs` dropped from 13,701 to 2,439 lines. |
| 23121–23126 | 6 | loop-03 comment (23121–23123) + `#[cfg(test)] #[path = "flow_tests.rs"] mod flow_tests;` (23124–23126) |

Helper items in lines 1–1444 (move with their concern in Phase 2):

- Buses: `ResolveEvent`/`ResolveBus` (56/73), `CrateIndexEvent`/`CrateIndexBus` (109/121),
  consts `EXT_INDEX_CAP` (149), `EXT_INDEX_FILE_CAP` (153) → **navigation / index_wiring**
- `ViewId` (176) → **views**; `FilePoint` (550), `JumpEntry`/`JumpStack` (559/576) →
  **file_view / navigation**
- `PickerKind` (505) → **picker**; `PickerCandidate`/`Picker` (659/674), `BufferRow` (738),
  `TreeRow` (748), `TreeState` (765), `FileViewRow` (780) →
  **picker / buffers / project / file_view**
- `TransientMenuState`/`TransientMenuEntry`/`TransientMenuRow` (690–715),
  `DiscardTarget` (728) → **views / magit**
- `SyntaxAnchor` (832), `Annotation` (843), `NotesEntry`/`NotesDoc` (859–887),
  consts `NOTES_BEGIN`/`NOTES_END`/`NOTES_RECORD_START` (894–898),
  `ANNOTATION_REANCHOR_WINDOW` (902), free fns `parse_notes` (911),
  `parse_notes_section` (946), `parse_record_block` (974), `serialize_notes` (1048),
  `DumpAnnotation`/`format_notes_dump` (1099–1126) → **notes**
- `IsearchState`/`IsearchDirection` (1192/1184), `DirtyCounts` (1221),
  `SearchKind`/`ResultRow`/`SearchState` (1230–1270), `SearchPrompt` (1298),
  `SearchPromptKind` (1305) → **search / magit**
- `QuitPrompt` (1329) → **keys**; `LogState` (1343), `CommitDiffState` (1366),
  `BlameState` (1372), `CommitEditorState` (1384), `KillRing` (1401) → **commit / buffers**
- Outside the 1–1444 framing: `EditorMove` (11669, `enum`, used by
  `commit_editor_move`) → **commit**; free fn `point_byte_offset` (11850) →
  **navigation** — both sit after the `impl AppStore` block (see the region
  rows above); they are listed here so no item of the file is unassigned.

> **Deviation from PLAN.md:** the plan's 869-method figure was a bad grep.
> The measured count is **421 methods** in the single `impl AppStore`
> block (lines 1717–11638: 253 `pub fn`, 168 private `fn`, of which 2 are
> formatted at column 0 — §5 item 8; several signatures wrap to a second
> line). The previously reported **432 = 421 methods + 11 associated free
> fns** (lines 11649–11811, folded into the concern counts in §3).
> Full-file total: **869 `fn` declarations** = 421 (`impl AppStore`)
> + 45 helper fns (28 struct fns at lines 78–1444 + 17 free fns at lines
> 911–11850, the last of which is `point_byte_offset` at 11850) + 1
> (`impl Default`) + 1 (`impl Picker::recompute`) + 401 test fns
> (350 `#[test]` + 22 `#[tokio::test]` + 29 non-test helpers).
> 421 + 45 + 1 + 1 + 401 = 869. The 869 figure appears to have conflated
> impl methods with test fns; the 421/11 split is the authoritative
> number. Per-module counts in §3 include the 12 associated free fns
> (11 + `point_byte_offset`); the methods-only counts sum exactly to 421.

## 3. Concern inventory (421 `impl` methods + 12 associated free fns)

Concern counts sum: 23+18+29+30+52+39+24+61+52+34+42+17+1+11 = **433**
= 421 `impl` methods + 12 associated free fns (the earlier headline 432
= 421 + 11, before `point_byte_offset` was assigned to navigation).
Methods-only counts: core 18, commit 56, picker 41, navigation 51 (the rest as listed).
Line ranges are disjoint runs (verified by program) — each method line in
1717–11638 falls in exactly one concern. Concerns are
interleaved in the file — methods are ordered roughly by feature-landing,
not by concern; Phase 2 stages will de-interleave as they move.

| # | Concern (target module) | Methods | Line runs (store.rs) |
|---|---|---:|---|
| 1 | core / shared helpers (`mod.rs`) | 23 | 1720–2060, 4773, 11449–11471, 11649, 11748–11787 |
| 2 | views / windowing + transient menu (`views.rs`) | 18 | 2077–2123, 6305–6552 |
| 3 | buffers / edit / save / region-kill-ring (`buffers.rs`) | 29 | 2073, 2142–2249, 2314–2328, 2769, 3081–3198, 3253–3501, 4566–4614 |
| 4 | notes / inline annotations (`notes.rs`) | 30 | 2263, 2405–2761, 2781–3062, 3206–3219 |
| 5 | file-view point/cursor/scroll (`file_view.rs`) | 52 | 4778–5643, 5883–5946 |
| 6 | isearch + project search / occur (`search.rs`) | 39 | 5708–5876, 5957, 10500, 10518–10997 |
| 7 | magit status / staging / discard (`magit.rs`) | 24 | 5984–6281, 6579–6648 |
| 8 | log / blame / commit-diff / editor / branch / stash (`commit.rs`) | 61 | 6676–7461, 11679–11718, 11800–11805 |
| 9 | M-. / jump stack / xref / imenu / impls (`navigation.rs`) | 52 | 3632–3655, 7766–7814, 7980–9541, 9817–10384, 10442 (`which_function` singleton), 11850 (`point_byte_offset`) |
| 10 | index + crate-index + watcher/auto-reload (`index_wiring.rs`) | 34 | 7471–7745, 7857–7907, 9571–9791, 10417 (`apply_index_event`), 10469–10491, 10505–10511 (`set_index_rx`/`take_index_rx`) |
| 11 | pickers (all flavors + previews) (`picker.rs`) | 42 | 3760–4407, 4541–4550, 11656 |
| 12 | project / files / recents / file-tree sidebar (`project.rs`) | 17 | 3561–3576, 3698–3731, 4493, 4632–4762 |
| 13 | minibuffer / echo-area message (`minibuffer.rs`) | 1 | 11475 |
| 14 | key dispatch + quit prompts (`keys.rs`) | 11 | 11059–11430, 11486–11633 |

**Free-fn note.** 12 of the 433 items are module-level free fns, not
`impl` methods: core +5 (`reload_anchor` 11649, `pane_window` 11748,
`window_slice` 11755, `keep_cursor_visible` 11767, `recenter_top_for`
11787), commit +5 (`prefill_commit_message` 11679, `extract_commit_message`
11700, `editor_cursor_line` 11718, `log_entry_display` 11800,
`blame_line_display` 11805), picker +1 (`file_candidate` 11656),
navigation +1 (`point_byte_offset` 11850 — it appeared in no earlier
module list; all five call sites are navigation methods:
`xref_find_definitions` 8050/8072, `resolver_scope_for` 8656,
`crate_xref_outcome` 9966/9985).

### Per-module method lists

**core (23)** — `new at apply_config theme top_view render_view
normalize_top_view view_name view_name_display project_display buffer_display
pending_display activity_display buffer_text buffer_rows cancel clear_pending
set_viewport_lines reload_anchor pane_window window_slice keep_cursor_visible
recenter_top_for`
*(the struct definition lives here; `reload_anchor/pane_window/window_slice/
keep_cursor_visible/recenter_top_for` are module-level free fns — 5 of the
23 — that serve as the shared windowing helpers every pane concern uses)*

**views (18)** — `push_view close_view close_other_views
split_window_vertical cycle_view home_rows home_title home_body_rows
menu_open menu_path open_menu close_menu menu_bindings menu_entries_for_path
menu_entries menu_rows menu_height menu_key_event`

**buffers (29)** — `buffer_list_selected insert_text
buffer_keep_insert_visible invalidate_highlight_for_key retain_rope_edit
open_scratch save_buffer save_buffer_key kill_buffer
open_buffer_list_selected buffer_list_next buffer_list_prev
buffer_list_kill_selected replace_buffer_text toggle_read_only
toggle_ro_active toggle_ro_key toggle_ro_accept toggle_ro_cancel
current_point_byte region_byte_range region_size_bytes region_line_range
set_mark exchange_point_and_mark kill_region copy_region yank yank_pop`

**notes (30)** — `open_notes notes_key ensure_notes_doc
current_annotation_path record_index_for_line reanchor_for_key
build_syntax_index_for_key collect_syntax_anchor_nodes
is_syntax_anchor_kind reanchor_all_buffers annotations_for_dump
buffer_line_text_for_rel sync_notes_from_doc drop_retained_tree annotate
note_prompt_active note_prompt_input note_prompt_char note_prompt_backspace
note_prompt_cancel note_prompt_confirm capture_syntax_anchor annotate_delete
delete_annotation_at delete_annotation_at_index annotate_toggle
current_buffer_annotation_count annotation_count_display notes_insert_char
notes_backspace`

**file_view (52)** — `scroll_top set_scroll_top scroll_line_down
scroll_line_up scroll_page_down scroll_page_up scroll_half_page_down
scroll_half_page_up mouse_scroll_up mouse_scroll_down mouse_click_position
scroll_to_top scroll_to_bottom recenter recenter_landing
file_view_position_display file_view_rows file_view_total_rows
buffer_is_project_owned buffer_annotation_path file_view_scroll_info
file_view_point current_line_count line_char_len line_chars is_word_char
file_point point_line point_col set_point set_point_line
scroll_window_point point_down point_up point_forward point_backward
point_line_start point_line_end point_word_forward point_word_backward
point_buffer_start point_buffer_end buffer_highlight_result ensure_highlight
ensure_highlight_for_key goto_line_start goto_line_active goto_line_input
goto_line_digit goto_line_backspace goto_line_confirm goto_line_cancel`

**search (39)** — `isearch_start isearch_query_char isearch_backspace
isearch_recompute isearch_next isearch_prev isearch_jump_to_current
isearch_confirm isearch_cancel isearch_active isearch_match_count
isearch_match_index find_all_matches search_rx cancel_search_job search_cancel
search_close search_rerun search_next search_prev search_keep_visible
search_jump search_view_info search_title search_display search_running
search_error search_prompt_start search_prompt_char search_prompt_backspace
search_prompt_cancel search_prompt_confirm begin_search start_project_search
references_at_point start_references_search start_occur apply_search_event
search_sort_hits`

**magit (24)** — `open_magit_status magit_refresh magit_toggle_fold
magit_cursor_down magit_cursor_up magit_visit_file magit_stage magit_unstage
magit_rows magit_window magit_keep_visible magit_view_info dirty_counts
with_git with_git_mut ensure_git git_status collect_diffs refresh_magit
magit_discard discard_key_event discard_armed confirm_discard execute_discard`

**commit (61)** —
*log (14):* `open_log log_next_page log_prev_page log_offset_to log_move_down
log_move_up log_open_commit refresh_log_page log_rows log_row_count
log_keep_visible log_view_info log_title log_entry_display`
*blame (13):* `open_blame blame_rows blame_row_count blame_keep_visible
blame_view_info blame_cursor_down blame_cursor_up blame_page_down
blame_page_up blame_cursor_bottom blame_cursor_top blame_title
blame_line_display`
*commit editor + diff (23):* `open_commit_editor commit_editor_commit
commit_editor_abort commit_editor_insert commit_editor_backspace
commit_editor_newline commit_editor_move commit_editor_rows commit_diff_rows
commit_diff_row_count commit_diff_view_info set_commit_diff_scroll
commit_diff_scroll_down commit_diff_scroll_up commit_diff_page_down
commit_diff_page_up commit_diff_scroll_bottom commit_diff_scroll_top
commit_diff_title commit_editor_title prefill_commit_message
extract_commit_message editor_cursor_line`
*branch/stash (11):* `open_branch_picker checkout_branch
create_branch_from_head open_stash_picker stash_pop stash_drop
branch_create_start branch_create_char branch_create_backspace
branch_create_cancel branch_create_confirm`

**navigation (52)** — *jump (5):* `current_jump_entry record_jump jump_back
jump_forward navigate_to_entry` · *xref/resolver (16):* `xref_find_definitions
find_implementations xref_definition_candidates self_receiver_candidates
local_binding_candidates type_member_candidates rust_dotted_receiver
start_symbol_resolution apply_resolve_event resolving_display
resolution_language resolver_scope resolver_scope_for xref_in_external_buffer
crate_xref_outcome resolver_from_file` · *free fn:* `point_byte_offset`
(11850; byte-offset helper for the tree-sitter surfaces — all five
call sites are navigation methods, see the free-fn note) ·
*per-language import-path
extraction (20):* `use_path_for_symbol use_decl_path use_group_entries
import_segments node_text is_ascii_identifier js_ts_scope_for
js_ts_bare_import_path js_ts_import_stmt_path js_ts_require_path
js_ts_import_hint js_ts_specifier js_ts_namespace_member_path python_scope_for
python_import_path_for_symbol python_import_stmt_path
python_dotted_segments go_scope_for go_import_path_for_symbol
go_string_content` · *imenu/symbol pickers (6):* `open_imenu
current_buffer_outline current_buffer_rust_tables open_imenu_picker
open_symbol_picker which_function` · *landing (4):* `symbol_at_point
dotted_path_container open_external_path open_resolved_source`

**index_wiring (34)** — *index (8):* `start_indexing refresh_index
apply_index_event indexing_display set_index index set_index_rx take_index_rx`
*crate index (10):* `start_crate_indexing source_extensions_for
crate_source_files under_node_modules apply_crate_index_event
crate_indexing_display crate_index_arc crate_index_arc_for_path crate_rel
bump_current_crate_recency` · *watcher/auto-reload (16):* `watch_bus
watcher_count watcher_active_root watcher_suspended start_watcher
start_watcher_at stop_watcher toggle_watcher take_watcher apply_project_change
reload_buffer reload_current_buffer mark_locally_modified
current_buffer_changed_on_disk current_buffer_editable buffer_mode_display`

**picker (42)** — `palette_candidates find_file_candidates
recent_file_candidates buffer_candidates project_candidates candidates_for
branch_candidates stash_candidates xref_candidates impls_candidates
imenu_candidates imenu_candidate imenu_depth symbol_candidates open_palette
open_find_file open_recent_files open_switch_buffer open_kill_buffer
open_switch_project open_picker picker_query_char picker_query_backspace
set_picker_query picker_open picker_kind picker_prompt picker_query
picker_filtered picker_count picker_selected picker_preview refresh_preview
file_preview picker_file_abs file_preview_at_line buffer_preview
file_preview_current run_selected picker_select_next picker_select_prev
file_candidate`

**project (17)** — `open_path open_project_path record_recent ensure_files
re_walk switch_project_root toggle_tree build_tree_rows tree_visible
tree_rows tree_selected tree_move_down tree_move_up tree_click_row
tree_open_selected toggle_tree_follow tree_follow_opened`

**minibuffer (1)** — `minibuffer_message`
*(the concern is thin today: the "minibuffer" is the `message` echo field
plus one prompt field per concern — note/search/branch/quit prompts each
live with their concern. Keep the module as the shared message + future
shared prompt plumbing.)*

**keys (11)** — `key_event dispatch_key dispatch begin_quit
quit_prompt_active quit_prompt_buffer quit_prompt_show
quit_prompt_show_with_error quit_prompt_key quit_prompt_advance
quit_prompt_cancel`

## 4. Target layout — **LANDED in 012-03** (all 13 concerns)

```
src/app/store/
  mod.rs         struct AppStore + its 83 fields (byte-identical), Default impl,
                 `pub use` re-exports, core (18) + the test module (see §3)
  helpers.rs     12 free fns — LANDED in 012-02 Stage A (11 moved; `point_byte_offset`
                 stayed in mod.rs because its 4 call sites are in-impl)
  notes_doc.rs   NotesDoc + parse/serialize — LANDED in 012-02 Stage B
  views.rs        18  LANDED
  buffers.rs      29  LANDED
  notes.rs        30  LANDED
  file_view.rs    52  LANDED
  search.rs       39  LANDED
  magit.rs        24  LANDED
  commit.rs       56  LANDED
  navigation/     51  LANDED (A7: split by contiguous range — `mod.rs` jump history (7)
                 + `definitions.rs` (13, definitions + resolution lifecycle) +
                 `imports.rs` (21, use-path extraction + js_ts/python/go import
                 parsing) + `xref.rs` (10, external/crate xref + symbol-at-point +
                 imenu/outline))
  index_wiring.rs 34  LANDED
  picker.rs       41  LANDED
  project.rs      17  LANDED
  minibuffer.rs    1  LANDED
  keys.rs         11  LANDED
```

`pub(super)` added by the moves: **73** (each demanded by a cross-concern call; 0
unreferenced, no `pub`/`pub(crate)` added, no field visibility changed). The original
estimate was 168 private methods — most are called from within their own concern.
Per-file sizes after 012-03: navigation 2,392 · file_view 999 · commit 797 · notes 761 ·
picker 746 · search 737 · buffers 733 · index_wiring 718 · keys 561 · magit 419 ·
views 335 · project 321 · minibuffer 7. After A7 (navigation split): navigation
2,392 → `navigation/` mod 166 · definitions 738 · imports 902 · xref 601 (one
`impl AppStore` wrapper per file; 6 new `pub(super)` for cross-file calls + 9
`pub(super)`→`pub(in crate::app::store)` to preserve the exact original effective
visibility of store-visible methods — 0 `pub`/`pub(crate)` added, no
field-visibility change); the test file (3,074) splits along the same seams
into `tests/navigation/` mod 8 · jump 180 (8) · definitions 1,548 (46) ·
resolver 392 (18) · xref 406 (8) · imenu 94 (3) · imports 458 (17) = 100 tests
(each concern file does `use super::*`, fixtures stay in `tests/mod.rs`).

Fields needed per module (all are private fields of `AppStore`, defined in
`mod.rs` — see §5 for why that makes them reachable as-is):

| Module | Fields it reads/writes |
|---|---|
| core | theme, registry, engine, view_stack, buffers, activity, message, pending, quit, project (display only) |
| views | view_stack, menu, buffers, registry (menu bindings) |
| buffers | buffers, buffer_list_selected, kill_ring, yank_pos/len/ring_index, toggle_ro_confirm, created_paths, saved_paths, highlight_cache (pub) |
| notes | notes_doc, notes_doc_loaded, notes_doc_mtime, notes_buffer_dirty, note_prompt_active/input/line, show_note_rows, buffers (pub), grammar_registry/highlight_cache (pub) |
| file_view | scroll, point, viewport_lines, recenter_cycle, goto_line_active/input, buffers (pub), highlight_cache (pub) |
| search | isearch, search, search_generation, search_prompt, search_rx, search_bus (pub), buffers (pub), grammar_registry (pub) |
| magit | git, status_tree, magit_scroll, dirty, discard_confirm, git_status/collect_diffs → git (pub(crate) facade via `with_git`), buffers (pub), files |
| commit | log, log_scroll, blame, blame_scroll, commit_diff, commit_diff_scroll, commit_editor, branch_create, git, buffers (pub) |
| navigation | jump_stack, xref_lookup_name, impls_keys, xref_crate_root, resolve_generation, resolving, external_buffers, resolve_bus (pub), index, external_indexes, crate_indexing, buffers (pub) |
| index_wiring | index, indexing, indexing_incremental, index_generation, pending_index_changes, index_rx, external_indexes, crate_indexing, crate_index_bus (pub), index_bus (pub), watcher, watch_suspended, watch_bus (pub), auto_reload (pub), project, created_paths, saved_paths, files |
| picker | picker, matcher, file_matcher, files, project, project_store (pub), buffers (pub), index, jump_stack, git |
| project | project (pub), project_store (pub), files, tree, buffers (pub), matcher/file_matcher |
| minibuffer | message (pub) |
| keys | registry (pub), engine (pub), pending (pub), quit (pub), quit_prompt, message (pub) |

**No field must become `pub(crate)`** for the split itself (see §5, item 2).
If a later phase hoists a field out of `mod.rs` into a submodule, that
field would need `pub(crate)` — not required by this plan.

## 5. Mechanical constraints

1. **Multiple `impl AppStore` blocks are legal** — Rust allows `impl
   AppStore` in any module of the same crate. Phase 2 moves are literal
   `git mv`-shaped: copy the `impl AppStore { … }` fragment into
   `src/app/store/<concern>.rs`, rename nothing, touch no signatures
   except visibility (item 3). `impl Default` and `impl Picker` move with
   `mod.rs` / `picker.rs` respectively.

2. **Field visibility needs no change — Rust private = "this module and
   descendants".** A struct field with no `pub` is accessible from the
   module that defines the struct *and every nested module*. After the
   split the struct stays defined in `store/mod.rs` and every concern
   module (`store::magit`, `store::keys`, …) is a *descendant* of
   `store` — so all 64 private fields remain reachable from every
   submodule with zero visibility edits. (Corrections the plan's
   "one-time visibility pass" assumption: the pass is for *methods*,
   below.) Fields that `src/ui/` reads stay exactly as today: the 19
   `pub` fields.

3. **Method visibility is the real friction.** 168 of the 421 `impl`
   methods are currently private (`fn`, no `pub` — including the two
   column-0 formatters, §5 item 8). A private method is visible
   only in its defining module and its descendants — after a move,
   `store::keys` cannot call a private `store::magit` method. Fix: give
   moved methods `pub(super)` (visible in `store` + all submodules —
   exactly the needed scope; cheaper than `pub(crate)`). The 253 `pub`
   methods keep their visibility. The 11 associated free fns at
   11649–11811 (core 5, commit 5, picker 1) and `point_byte_offset`
   (11850, navigation) need **no visibility change**: the free fns that stay in
   `store/mod.rs` are already visible to every submodule, and the rest live in
   `store/helpers.rs` (`pub(super)` — 012-02 Stage A landed this) or are called
   only from within their own target module.
   This is the one mechanical pass, and it is
   additive (widening), so no call site breaks.
   Per-module private method counts (methods only; sum 168): core 1,
   views 3, buffers 7, notes 19, file_view 17, search 13, magit 11,
   commit 13, navigation 41, index_wiring 9, picker 24, project 5,
   minibuffer 0, keys 5.

4. **`pub use` re-exports in `mod.rs`.** UI code imports helper types by
   `crate::app::store::{…}`: `BufferRow`, `DirtyCounts`, `FileViewRow`,
   `PickerCandidate`, `ResultRow`, `TransientMenuRow`, `TreeRow`,
   `ViewId`, `AppStore`, plus `ui/magit_status` and others use
   `AppStore` only. Outside `src/ui/*` as well: `IsearchDirection` and
   `SearchPromptKind` (imported by `src/app/command.rs:335,343,661,675`;
   moving to `search.rs`), the free fn `format_notes_dump` (called from
   `src/main.rs:355`; moving to `notes.rs`), and `PickerKind` (used by
   `src/app/flow_tests.rs:3524`; moving to `picker.rs`). When those
   types move to submodules, `store/mod.rs`
   must re-export them (`pub use self::picker::PickerCandidate; …`) to
   keep the `crate::app::store::X` paths stable.

5. **`#[path]`-style wiring for `flow_tests.rs`.** The last lines of
   store.rs hang the flow tests off the module:
   `#[cfg(test)] #[path = "flow_tests.rs"] mod flow_tests;`
   (flow_tests.rs itself stays at `src/app/flow_tests.rs`, 3,624 lines,
   split in Phase 3). `#[path]` resolves **relative to the directory of
   the containing file**, so the attribute does not move verbatim: in
   `store/mod.rs` it becomes **`#[path = "../flow_tests.rs"]`**. A
   verbatim `#[path = "flow_tests.rs"]` inside `src/app/store/mod.rs`
   would resolve to `src/app/store/flow_tests.rs` (which does not exist)
   → E0583. Because `flow_tests` is a *child* of the
   `store` module, it keeps access to every private field/method of
   `mod.rs` — which is exactly why the comment at lines 23121–23123
   ("hung off this module so the AppStore's private fields are visible")
   keeps working. If Phase 3 splits flow_tests into per-concern test files
   inside `store/`, the same descendant rule applies.

6. **The `#[cfg(test)] mod tests` move (lines 11856–23119).** 401 fns
   (350 `#[test]` + 22 `#[tokio::test]` + 29 non-test helpers), 11,264
   lines. Tests call private
   methods and private fields freely (descendant access). Phase 3 moves
   test fns *with* their concern's module; until then any interim
   location must remain a descendant of the `store` module or the tests
   stop compiling. Do not "fix" visibility to decouple tests — move the
   tests instead.

7. **`flow_tests.rs` today is compiled as part of the workspace test
   build** — it is not a separate integration test; moving the `mod`
   declaration (item 5, with the path corrected to
   `../flow_tests.rs`) changes no build-graph behavior.

8. **Two impl methods are formatted at column 0.**
   `collect_syntax_anchor_nodes` (2618) and `is_syntax_anchor_kind`
   (2643) are `impl AppStore` methods whose signatures sit at column 0 —
   indent-based tooling must not treat them as free fns (moving them
   out of the impl breaks their `Self::` calls).

## 6. How to find things (one-line guide)

| You need… | Look in… |
|---|---|
| app state / any "where is field X" | `src/app/store/mod.rs` (struct + field docs) |
| key handling / what a binding does | `src/app/store/keys.rs` → `src/app/keymap.rs` → `src/app/command.rs` |
| magit status/staging behavior | `src/app/store/magit.rs` (facade `with_git`); git plumbing in `src/git/repo.rs` |
| log/blame/commit-editor | `src/app/store/commit.rs`; raw git in `src/git/{log,blame,commit,diff}.rs` |
| cursor/scroll in the file view | `src/app/store/file_view.rs`; rendering in `src/ui/file_view.rs` |
| search (C-s, project, occur) | `src/app/store/search.rs`; engines in `src/search/` |
| M-. / imenu / jump | `src/app/store/navigation/`; index in `src/nav/index.rs`; fall-through in `crates/redline-resolve` |
| pickers of any flavor | `src/app/store/picker.rs`; rendering in `src/ui/picker.rs` |
| annotations / notes | `src/app/store/notes.rs` (format: `NOTES_*` consts + `parse_notes`) |
| project / files / recents / tree sidebar | `src/app/store/project.rs`; walk in `src/model/files.rs` |
| background jobs & their buses | `src/app/store/index_wiring.rs`; bus types defined at the top of the same module family |
| rendering / drain loops | `src/ui/root.rs` (owns the event loop, bus drains) |
| syntax / highlighting | `src/syntax/` (registry = the only tree-sitter API surface) |
| PTY behavior tests | `src/app/flow_tests.rs` (hung off `store/mod.rs`) + `tools/sweep_flows.py` |

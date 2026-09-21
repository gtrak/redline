# Architecture — module map and the `store.rs` split plan

Plan 012, issue 01. All line numbers and counts below were measured from
`src/app/store.rs` at **`acd974e`** (22,992 lines). This file is the shared map
for every later 012 stage and for reviewers.

> **The map is baseline-pinned — re-derive before executing.** `store.rs` has
> already moved once since measurement: the picker-density landing (`6d01393`)
> added 45 lines and removed 1, so **every line number below is stale** (the file
> is now 23,037 lines). A stage that trusts these numbers edits the wrong lines.
> **Step 1 of the split is therefore to re-run the census against the current
> file and update this map** — the invariants to check are: the `impl` method
> count (421 = 253 `pub` + 168 private), the §3 ranges are disjoint and cover
> every method line exactly once, and the per-module name lists reconcile (no
> duplicates; union = the methods + the assigned free fns). The **counts** are
> stable across the move; only the **line numbers** need re-deriving.

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
    flow_tests.rs         3,624  PTY-style flow tests, hung off store.rs via #[path]
    keymap.rs              628  KeymapEngine, KeySeq, C-x/C-c prefixes
    store.rs            22,992  AppStore — THE file this plan splits (see §2–5)
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
    highlight.rs          1,166  tree-sitter → theme highlight rows
    node.rs              2,251  per-language tree walking (012/09 target)
    queries.rs            1,697  per-language query consts (012/09 target)
    registry.rs            502  GrammarRegistry (one tree-sitter API surface)
    tokens.rs              308  token classification

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
| 1–1428 | 1,428 | helper types + free fns (see below) |
| 1429–1699 | 271 | `pub struct AppStore` — **83 fields: 19 `pub`, 64 private** |
| 1701–11588 | 9,888 | single `impl AppStore { … }` — **421 methods (253 `pub`, 168 private; 2 private methods are formatted at column 0 — lines 2602/2627)** |
| 11589–11759 | 171 | 11 module-level **free fns** (assigned to core/commit/picker in §3) + the `EditorMove` enum (11617 → commit) |
| 11761–11765 | 5 | `impl Default for AppStore` (delegates to `new`) |
| 11767–11791 | 25 | `impl Picker` (`recompute`) |
| 11793–11802 | 10 | free fn `point_byte_offset` (→ navigation, §3) |
| 11804–22985 | 11,182 | `#[cfg(test)] mod tests` — **389 fns (349 `#[test]` + 22 `#[tokio::test]` + 18 non-test helpers)** |
| 22987–22992 | 6 | loop-03 comment (22987–22989) + `#[cfg(test)] #[path = "flow_tests.rs"] mod flow_tests;` |

Helper items in lines 1–1428 (move with their concern in Phase 2):

- Buses: `ResolveEvent`/`ResolveBus` (77), `CrateIndexEvent`/`CrateIndexBus` (121),
  consts `EXT_INDEX_CAP` (149), `EXT_INDEX_FILE_CAP` (153) → **navigation / index_wiring**
- `ViewId` (201) → **views**; `FilePoint` (550), `JumpEntry`/`JumpStack` (576) →
  **file_view / navigation**
- `PickerKind` (505) → **picker**; `PickerCandidate`/`Picker` (648), `BufferRow` (722),
  `TreeRow` (732), `TreeState` (749), `FileViewRow` (764) →
  **picker / buffers / project / file_view**
- `TransientMenuState`/`TransientMenuEntry`/`TransientMenuRow` (674–712),
  `DiscardTarget` (712) → **views / magit**
- `SyntaxAnchor` (816), `Annotation` (827), `NotesEntry`/`NotesDoc` (848–878),
  consts `NOTES_BEGIN`/`NOTES_END`/`NOTES_RECORD_START` (878–886),
  `ANNOTATION_REANCHOR_WINDOW` (886), free fns `parse_notes` (895),
  `parse_notes_section` (930), `parse_record_block` (958), `serialize_notes` (1032),
  `DumpAnnotation`/`format_notes_dump` (1083–1110) → **notes**
- `IsearchState`/`IsearchDirection` (1168/1176), `DirtyCounts` (1205),
  `SearchKind`/`ResultRow`/`SearchState` (1225–1281), `SearchPrompt` (1282),
  `SearchPromptKind` (1289) → **search / magit**
- `QuitPrompt` (1313) → **keys**; `LogState` (1327), `CommitDiffState` (1350),
  `BlameState` (1356), `CommitEditorState` (1368), `KillRing` (1389) → **commit / buffers**
- Outside the 1–1428 framing: `EditorMove` (11617, `enum`, used by
  `commit_editor_move`) → **commit**; free fn `point_byte_offset` (11798) →
  **navigation** — both sit after the `impl AppStore` block (see the region
  rows above); they are listed here so no item of the file is unassigned.

> **Deviation from PLAN.md:** the plan's 869-method figure was a bad grep.
> The measured count is **421 methods** in the single `impl AppStore`
> block (lines 1701–11588: 253 `pub fn`, 168 private `fn`, of which 2 are
> formatted at column 0 — §5 item 8; several signatures wrap to a second
> line). The previously reported **432 = 421 methods + 11 associated free
> fns** (lines 11599–11753, folded into the concern counts in §3).
> Full-file total: **857 `fn` declarations** = 421 (`impl AppStore`)
> + 45 helper fns (28 struct fns at lines 78–1424 + 17 free fns at lines
> 895–11798, the last of which is `point_byte_offset` at 11798) + 1
> (`impl Default`) + 1 (`impl Picker::recompute`) + 389 test fns
> (349 `#[test]` + 22 `#[tokio::test]` + 18 non-test helpers).
> 421 + 45 + 1 + 1 + 389 = 857. The 869 figure appears to have conflated
> impl methods with test fns; the 421/11 split is the authoritative
> number. Per-module counts in §3 include the 12 associated free fns
> (11 + `point_byte_offset`); the methods-only counts sum exactly to 421.

## 3. Concern inventory (421 `impl` methods + 12 associated free fns)

Concern counts sum: 23+18+29+30+52+39+24+61+52+34+42+17+1+11 = **433**
= 421 `impl` methods + 12 associated free fns (the earlier headline 432
= 421 + 11, before `point_byte_offset` was assigned to navigation).
Methods-only counts: core 18, commit 56, picker 41, navigation 51 (the rest as listed).
Line ranges are disjoint runs (verified by program) — each method line in
1701–11588 falls in exactly one concern. Concerns are
interleaved in the file — methods are ordered roughly by feature-landing,
not by concern; Phase 2 stages will de-interleave as they move.

| # | Concern (target module) | Methods | Line runs (store.rs) |
|---|---|---:|---|
| 1 | core / shared helpers (`mod.rs`) | 23 | 1704–1712, 1916–2044, 4737, 11399–11421, 11599, 11696–11735 |
| 2 | views / windowing + transient menu (`views.rs`) | 18 | 2061–2107, 6261–6508 |
| 3 | buffers / edit / save / region-kill-ring (`buffers.rs`) | 29 | 2057, 2126–2233, 2298–2312, 2753, 3065–3182, 3237–3485, 4530–4578 |
| 4 | notes / inline annotations (`notes.rs`) | 30 | 2247, 2389–2473, 2563–2752, 2754–2824, 2932–3046, 3190–3203 |
| 5 | file-view point/cursor/scroll (`file_view.rs`) | 52 | 4742–5013, 5146–5607, 5847–5910 |
| 6 | isearch + project search / occur (`search.rs`) | 39 | 5672–5840, 5921, 10450, 10468–10547, 10615–10872, 10947 |
| 7 | magit status / staging / discard (`magit.rs`) | 24 | 5948–6237, 6535–6604 |
| 8 | log / blame / commit-diff / editor / branch / stash (`commit.rs`) | 61 | 6632–7417, 11627–11666, 11748–11753 |
| 9 | M-. / jump stack / xref / imenu / impls (`navigation.rs`) | 52 | 3616–3639, 7722–7770, 7936, 8133, 8231–8328, 8398, 8461–8654, 8726–8777, 8853–8892, 8954–8975, 9036, 9124–9243, 9319, 9401–9495, 9771–9805, 9893, 10024, 10190–10366, 10392 (`which_function` singleton) |
| 10 | index + crate-index + watcher/auto-reload (`index_wiring.rs`) | 34 | 7427–7522, 7599–7701, 7813–7863, 9525, 9593–9745, 10367–10391 (`apply_index_event`), 10419–10441, 10455–10461 (`set_index_rx`/`take_index_rx`) |
| 11 | pickers (all flavors + previews) (`picker.rs`) | 42 | 3744–4371, 4505–4514, 11606 |
| 12 | project / files / recents / file-tree sidebar (`project.rs`) | 17 | 3545–3560, 3682–3715, 4457, 4596–4726 |
| 13 | minibuffer / echo-area message (`minibuffer.rs`) | 1 | 11425 |
| 14 | key dispatch + quit prompts (`keys.rs`) | 11 | 11009, 11356–11380, 11436–11583 |

**Free-fn note.** 12 of the 433 items are module-level free fns, not
`impl` methods: core +5 (`reload_anchor` 11599, `pane_window` 11696,
`window_slice` 11703, `keep_cursor_visible` 11715, `recenter_top_for`
11735), commit +5 (`prefill_commit_message` 11627, `extract_commit_message`
11648, `editor_cursor_line` 11666, `log_entry_display` 11748,
`blame_line_display` 11753), picker +1 (`file_candidate` 11606),
navigation +1 (`point_byte_offset` 11798 — it appeared in no earlier
module list; all five call sites are navigation methods:
`xref_find_definitions` 8006/8028, `resolver_scope_for` 8610,
`crate_xref_outcome` 9918/9937).

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
(11798; byte-offset helper for the tree-sitter surfaces — all five
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

## 4. Target layout

```
src/app/store/
  mod.rs         struct AppStore + its 83 fields, Default impl,
                 `pub use` re-exports for UI-imported items, core (23)
                 → ~1,430 + 670 lines (struct + helpers)
  views.rs        18
  buffers.rs      29
  notes.rs        30
  file_view.rs    52
  search.rs       39
  magit.rs        24
  commit.rs       61
  navigation.rs   52
  index_wiring.rs 34
  picker.rs       42
  project.rs      17
  minibuffer.rs   1
  keys.rs         11
```

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
   11599–11753 (core 5, commit 5, picker 1) and `point_byte_offset`
   (navigation) need **no visibility change**: the five core free fns
   land in `store/mod.rs`, where module privacy is already visible to
   every submodule, and the remaining seven (commit 5, picker 1,
   navigation 1) are called only from within their own target module.
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
   `mod.rs` — which is exactly why the comment at lines 22987–22989
   ("hung off this module so the AppStore's private fields are visible")
   keeps working. If Phase 3 splits flow_tests into per-concern test files
   inside `store/`, the same descendant rule applies.

6. **The `#[cfg(test)] mod tests` move (lines 11804–22985).** 389 fns
   (349 `#[test]` + 22 `#[tokio::test]` + 18 non-test helpers), 11,182
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
   `collect_syntax_anchor_nodes` (2602) and `is_syntax_anchor_kind`
   (2627) are `impl AppStore` methods whose signatures sit at column 0 —
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
| M-. / imenu / jump | `src/app/store/navigation.rs`; index in `src/nav/index.rs`; fall-through in `crates/redline-resolve` |
| pickers of any flavor | `src/app/store/picker.rs`; rendering in `src/ui/picker.rs` |
| annotations / notes | `src/app/store/notes.rs` (format: `NOTES_*` consts + `parse_notes`) |
| project / files / recents / tree sidebar | `src/app/store/project.rs`; walk in `src/model/files.rs` |
| background jobs & their buses | `src/app/store/index_wiring.rs`; bus types defined at the top of the same module family |
| rendering / drain loops | `src/ui/root.rs` (owns the event loop, bus drains) |
| syntax / highlighting | `src/syntax/` (registry = the only tree-sitter API surface) |
| PTY behavior tests | `src/app/flow_tests.rs` (hung off `store/mod.rs`) + `tools/sweep_flows.py` |

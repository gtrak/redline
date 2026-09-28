# Plan 018 — unified list narrowing (fzf/helm-style, every list surface)

**COMPLETION RECORD (2026-09-28).** All five issues landed. The user's ask, verbatim:
*"i-search results could be more interactive and helm-like UX, same with the symbol-search. Every
list result should be narrowable the same way, just like fzf or helm."*

| issue | landing | what |
|---|---|---|
| 01 shared narrowing mechanism | `7e60917` | the ONE core moved out of `Picker` into `src/app/store/narrowing.rs` (`narrow(query, displays, matcher) -> Vec<(usize,u32)>` + `NarrowSession`), byte-for-byte picker migration; 4 discrimination pins |
| 02 isearch as a list | `43ad828` | helm-occur shape: a live browsable match list in a new `src/ui/isearch_list.rs` overlay (NOT a new `ViewId`), CHAR-column landing, `keys.rs` 0-line diff |
| 03 results-view narrowing | `14ec169` | prompt row (keys LEAD, NoWrap) at the top of the view + live FilterOnly narrowing; projection `"{file}:{line_no} {line}"` so narrowing by FILE NAME works |
| 04 buffer-list narrowing | `ec134af` | `C-x C-b` narrows (MRU preserved, windowing moved to the store) + ONE shared display source with the Buffers picker, cross-checked load-bearing in both directions |
| 05 consistency sweep | `db8bde7` | the inventory as DATA (`src/app/store/tests/narrowing.rs`, 25 rows) with a cross-check that names any undeclared surface; the four deliberate differences as named follow-ups U-E10..U-E13 |

**Verified on main**: full gate `OK (full)` after every landing, including the plan-level combination
with all four narrowing drives running in the battery (`drive_isearch_list`, `drive_buffer_list_narrow`,
`drive_results_narrow`, plus the unmodified `drive_search`); bin tests 1153, clippy `-D warnings` clean.

**Success criteria (each with evidence):** (1) ONE store-side core, **with isearch as a measured
exception** — the plan's original wording claimed "the picker's 13 kinds, isearch, the results view, and
the buffer list all recompute through it", which 018-02 measurably disproved and this plan therefore
CORRECTED (see the criteria section below): `narrow` is a nucleo score-and-REORDER over display strings,
while isearch rows are a literal byte-search result where several rows share one line's text (scoring
cannot recover row identity) and match order IS the search. The inventory test declares it
`narrows=true, mechanism=own (literal search)`; (2) isearch is a browsable live list with
confirm/cancel/highlight/restore semantics intact (018-02 pins + `drive_isearch_list.py` + U-E8);
(3) results and buffer lists narrow at keys-leading one-row prompts with RET/n/p unchanged on
un-narrowed behaviour (018-03/04 pins + both drives + U-E9/U-B4 + unmodified `drive_search.py`);
(4) the inventory cross-check fails on a new surface with no verdict (mutation-verified: dropping the
`Palette` row reddens it, naming the kind — re-run independently by the orchestrator); (5) every changed
drive disclosed, updated, stronger-or-equal, full battery green on main.

**What deliberately does NOT unify, as named follow-ups (U-E10..U-E13 in `docs/ux-testing-plan.md`,
cross-referenced from the inventory test):** magit status (magit-native *section* narrowing; the diff
payload forbids row-level), log (`git log`-query-level, server-side), tree (filter-children-keep-parents,
AFTER its renderer-side windowing re-homes to the store), and isearch's second query dimension (a nucleo
filter inside the literal-search list).

**Process note (orchestrator, recorded because it is this plan's own lesson):** the two lanes for 02 and
04 were dispatched in parallel on a plan claim of "file-disjoint", which was FALSE — both edit
`src/app/store/mod.rs` and `src/app/store/keys.rs` (corrected in the 04 issue file, left visible with the
correction beside it). The rebase conflict that followed was in `src/ui/root/snapshot.rs` (both lanes
extended `Snapshot`), resolved keeping both sides and re-verified with both drives in one gate. Both
lanes' gate commands were told only to "redirect to a file" and both chose `/tmp/gate_full.log`, so that
log held two interleaved runs and neither verdict was trustworthy from it — re-evidenced with unique
paths. **A brief that names a file and a log path must name a unique one.**

---

# PLAN.md

# Plan 018 — unified list narrowing (helm/fzf-style narrowing on every list surface)

**Status:** COMPLETE (all five issues landed: 01 `7e60917`, 02 `43ad828`, 03 `14ec169`, 04 `ec134af`, 05 `d7c35f6` — see `.agents/plans/STATUS.md`, Plan 018). The §4 verdicts are cross-checked by `src/app/store/tests/narrowing.rs` (018-05), and the four named follow-ups (magit section-narrow, log git-query narrow, tree filter, isearch second dimension) live at `docs/ux-testing-plan.md` U-E10..U-E13.

**Origin (user, verbatim):** *"I also think the i-search results could be more
interactive and helm-like UX, same with the symbol-search. Every list result
should be narrowable the same way, just like fzf or helm."*

Note the criterion is *the same way*: every narrowable surface uses ONE
mechanism, not per-surface filters. That criterion is what this plan exists to
satisfy — it is the same shape as the 017 "ALL symbols jump to the right
definition" governing criterion.

---

## 1. The inventory (re-derived at `20e0162`, every row cited)

Every component in `src/ui/` that renders more than a handful of rows, what its
rows are, where the row data lives, how it is windowed, its keys, and whether
it already narrows.

| # | Surface | Rows (what a row is) | Row data (store owner) | Windowing today | Navigation keys | Narrows today? |
|---|---|---|---|---|---|---|
| 1 | **Picker overlay** (`src/ui/picker.rs` canvas) | `PickerCandidate` (13 `PickerKind`s: `mod.rs:676-711` — Palette, FindFile, RecentFiles, Buffers, KillBuffer, Projects, Xref, Impls, Imenu, Symbols, Branch, Stash, Annotations) | `Picker { kind, prompt, query, selected, filtered: Vec<(PickerCandidate, u32)>, preview }` (`mod.rs:864-873`); candidates built per kind in `src/app/store/picker.rs` (`candidates_for`, `:198`) | store: full filtered list; renderer windows to `min(candidates+2, viewport-1)` canvas, list window `start = selected.saturating_sub(win-1)` (`src/ui/picker.rs` `canvas_height`/`draw`) | printable chars extend query, Backspace/C-h edit, RET runs, C-g cancels, arrows/C-n/C-p move; per-kind verbs (Stash `x`, Annotations `d`) — `src/app/store/keys.rs:169` `picker_key_event` | **YES** — `Picker::recompute` (`mod.rs:2944`): `nucleo_matcher::Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart)` + `pattern.score(Utf32Str::new(&c.display, &mut buf), matcher)` over `display`, sort `Reverse(score)` (stable, ties keep source order), selection clamped `selected.min(filtered.len()-1)` (`picker.rs:583` `set_picker_query`) |
| 2 | **Isearch** (`IsearchState`, buffer-view modal) | not a list — byte offsets into the current buffer | `IsearchState { active, query, direction, matches: Vec<usize>, current, pre_search_line, pre_search_col }` (`mod.rs:1463-1478`); logic in `src/app/store/search.rs` (`isearch_start:7`, `isearch_recompute:49`, `find_all_matches:259` — **case-sensitive literal substring, not nucleo**) | none — the point jumps match-to-match; minibuffer shows `I-search: {query} [{idx}/{count}]` (`search.rs:49-86`) | printable self-insert into query, Backspace, C-s/C-r next/prev (repeat), RET confirm, C-g cancel — `keys.rs:304` `isearch_key_event` | **NO** — emacs-style jump-to-match; never renders matches as browsable rows. Side effects: `match_context` highlight (`isearch_sync_match_context`, `search.rs:140`), cleared on confirm AND cancel (lifetime rule, issue-jump-highlight) |
| 3 | **Search results** (`ViewId::Search`, `src/ui/results_view.rs`) | `ResultRow` — file-group `Header { file, count, final_count }` or `Hit { hit, hit_index }` (`mod.rs:1529-1540`); hits from the rg pipeline, streamed | `SearchState` (`mod.rs:1546`): `rows`, `hits`, `hit_rows`, `selected`, `scroll`, `generation`; events installed by `apply_search_event` (`search.rs:810`), deterministic `(path,line,col)` sort on `Finished` (`search_sort_hits`) | store: `search_view_info()` (`search.rs:535`) returns `(rows window, top, total, selected_row: Option<usize>)`, keeps `selected` in window | `n`/`p`/`C-n`/`C-p` next/prev match, RET jump (jump-stack sentinel so M-, returns), `g` re-run, `q`/ESC close, `C-g` cancel in-flight — `SEARCH_BINDINGS` (`mod.rs:616`, 14 entries) | **NO** — the query is entered ONCE in a pre-flight prompt (`search_prompt`, `search.rs:610`) before the job starts; the results list itself has no prompt and cannot be narrowed |
| 4 | **Tree sidebar** (`src/ui/tree.rs`, 34-col left column) | `TreeRow { depth, name, is_dir, rel_path }` (`mod.rs:938-948`) | `TreeState { visible, rows, selected, follow }` (`mod.rs:955-960`); accessors `tree_rows`/`tree_selected` (`project.rs:267,275`) | **RENDERER-side**: `start = selected.saturating_sub(5)`, `take(TREE_VISIBLE_ROWS = 8)` (`src/ui/tree.rs`, constants in `src/model/tree_layout.rs:14,19`) — the one windowing exception to the store-owns pattern | Down/Up/PGDN/PGUP move, RET open, `C-c p t` toggle — `tree_key_event` (`keys.rs:556`), active only on top view Buffer/Home | **NO** — but it is hierarchical (depth/`is_dir`), and its flat-file narrowing is already served by the FindFile picker |
| 5 | **Magit status** (`src/ui/magit_status.rs` + shared `src/ui/rows_view.rs`) | `MagitRow { text, role, selected }` (`src/model/sections.rs:136`) — section headings, hunk headers, **diff body lines** (`RowRole::{DiffContext, DiffAdd, DiffDelete, ...}`, `sections.rs:33`) | `StatusTree` (fold/cursor model, `model/sections.rs`), `status_tree` + `magit_scroll` (store, `magit.rs`); rows via `magit_rows()` / window via `magit_view_info()` (`magit.rs:200`, `window_slice` from `helpers.rs`) | store cursor-following window (`magit_keep_visible`, `magit.rs:170`) | `s`/`u` stage/unstage, TAB fold, RET visit, `n`/`C-n`/`p`/`C-p` move sections, `g` refresh, `y`/`z` branch/stash (both open NARROWED pickers), `h` menu, `k` discard — `MAGIT_STATUS_BINDINGS` (`mod.rs:505`, 21 entries) | **NO** — and its rows carry diff payload: any score-reorder of diff body lines would destroy the diff; magit's own answer is *section* narrow, not row reorder |
| 6 | **Log** (`src/ui/log_view.rs` via `MagitRowsView`) | `MagitRow` from `LogEntry` (`commit.rs:495` `log_rows`, display in `log_entry_display`, `helpers.rs`) | `LogPage { offset, entries, total }` — **server-paged** (git log range fetch, `commit.rs:6-60`: `open_log`/`log_next_page`/`log_prev_page`) | store window (`log_view_info`, `commit.rs:772`) | `n`/`p` PAGE, arrows/`j`/`k` in-page move, RET open commit diff — `LOG_BINDINGS` (`mod.rs:540`, 12 entries) | **NO** — and client-side filtering of one page would hide commits and break paging; true log narrow needs `git log --grep/-G` (a different mechanism) |
| 7 | **Blame** (`src/ui/blame_view.rs` via `MagitRowsView`) | `MagitRow` from `BlameLine` (`commit.rs:520` `blame_rows`, aligned display in `helpers.rs`) | `blame` page + `blame_scroll` (`commit.rs:690` `blame_view_info`) | store window (cursor-following) | `C-n`/`C-p`, `C-v`/`M-v` pages, `M-<` top — `BLAME_BINDINGS` (`mod.rs:560`, 7 entries) | **NO** — read-only aligned lines; no query to filter by |
| 8 | **Buffer list** (`C-x C-b`, `src/ui/buffer_view.rs`) | `BufferRow { name, current, lines }` (`mod.rs:928-932`) | `buffers` (MRU) + `buffer_list_selected` (`mod.rs:1731`); `buffer_rows()` (`mod.rs:2425`) | no store windowing today (list is short; renderer lists all rows) | `q`/RET, Down/C-n/Up/C-p, `n`/`p`, `d` kill, PGDN/PGUP — `BUFFER_LIST_BINDINGS` (`mod.rs:483`, 11 entries) | **NO** — and its candidate source duplicates the Buffers-picker source (`buffer_candidates`, `picker.rs:62`): two hand-maintained views of the same rows = the drift the `0829ddd` precedent forbids |
| 9 | **Home view** (`src/ui/home_view.rs`) | `TransientMenuRow` (category headers + `key docs` rows) | `home_rows()` (`views.rs:241`) — derived live from keymap × registry, viewport-bounded in the store | store-bounded | **none** — `HOME_BINDINGS` is deliberately empty (`mod.rs:606`); no selection cursor | n/a — a help screen, not a navigable list |
| 10 | **Transient menu** (`src/ui/transient_menu.rs`, `?` / magit `h`) | `TransientMenuRow` (bindings of the active view, hydra tree) | `TransientMenuState` (`mod.rs:880`); `menu_rows()` (`views.rs:175`) | full list (short) | **prefix keys** (the menu is navigated by the binding itself, not a cursor) | n/a — prefix-key navigation is its whole design; a query prompt would fight it |
| 11 | **Commit editor** (`src/ui/commit_editor.rs` via `MagitRowsView` rows) | editable `Rope` text with a cursor — NOT a row list | `CommitEditorState` (`commit.rs`) | text editing (emacs motion) | full editing + `C-c C-c`/`C-c C-k` — `COMMIT_EDITOR_BINDINGS` | n/a — it is a text buffer |
| 12 | **Commit diff** (`ViewId::CommitDiff`) | `MagitRow` diff body | diff of the selected commit (`commit.rs`) | store window | diff navigation; see `COMMIT_DIFF_BINDINGS` | n/a — diff payload, same obstacle as magit status |

Out of scope by construction: the **file view** (code, not a list) and the
**picker preview pane** (not a list).

**Verdict from the table:** exactly ONE narrowing engine exists (surface 1, and
it covers all 13 picker kinds, so "symbol-search" — `C-c p s`, `PickerKind::Symbols`,
`picker.rs:465` — and "file-search" already narrow helm-style). The surfaces the
user is missing narrowing on are 2 (isearch), 3 (results), and 8 (buffer list);
surfaces 4/5/6/7 have real structural reasons not to (below); 9/10/11/12 are not
navigable lists.

### Disagreements with the audit brief's recon (re-measured, not assumed)

1. **PickerKind count/names.** The brief said the picker serves
   `PickerKind::{Files, Symbols, Xref, Annotations, Buffers}` (five). Measured:
   **13** variants (`mod.rs:676-727`), and the file one is named `FindFile`, not
   `Files`. All 13 go through the same `recompute` — the claim "files and
   symbols already narrow" is true but understated; so do the palette, buffers,
   projects, branches, stashes, imenu, impls, and annotations.
2. **`IsearchState` location.** The brief said "`src/app/store/search.rs`,
   ~line 1463". The struct is at **`src/app/store/mod.rs:1463`**; `search.rs`
   holds its methods (7/49/259). The line number is right, the file is wrong.
   Also: isearch matching is a **case-sensitive literal substring**
   (`find_all_matches`, `search.rs:259`), not nucleo — a deliberate
   emacs-fidelity gap from the picker, and it matters for the design below.
3. **`Picker::recompute` at `mod.rs:2944`** — confirmed, and `display` is the
   scored field — confirmed.
4. **Results-view claim** — confirmed: `ResultsViewProps` carries a pre-computed
   window (`results_view.rs:18-33`), the store owns `search_view_info`, keys are
   `n`/`p`/RET (+ `g`/`q`/ESC/`C-g`), and there is no query prompt on the list.
   One addition the brief missed: results are **streamed** (events, generation
   guard, sort on `Finished`), which constrains WHERE the narrowing can live.
5. **Tree windowing exception** (brief did not name it): the tree is the ONLY
   list surface windowed in the **renderer** (`src/ui/tree.rs`), against the
   store-owns-windowing rule every other surface follows.

---

## 2. The unification design

### 2.1 The hypothesis, argued

*Hypothesis: there is one narrowing mechanism, so narrowing should become a
property of a generic row list rather than of the picker overlay.*

**For it (evidence):**
- The engine is already a pure function of `(query, rows-with-display,
  matcher) -> ranked rows`: `Picker::recompute` (`mod.rs:2944`) touches nothing
  but its arguments and the `Matcher`. Its only picker-specific parts are
  (a) the `PickerCandidate` clone and (b) the selection clamp
  (`picker.rs:583`). Both are portable.
- Every candidate surface already has — or trivially gets — a store-side
  **display string** to score: the picker's `display` field (deliberately
  "byte-stable", separate from the drawn `label`/`detail`); results hits have
  `Hit { file, line_no, line }`; isearch matches have the buffer line text;
  buffers have `name`. Scoring a string is the whole engine.
- This project has been treated for this exact disease three other ways:
  `issue-ignore-single-engine` (two hand-rolled ignore decisions),
  `issue-provider-ident-char` (three `is_ident_char` copies), `0829ddd`
  (`type_globs` hand-list drifted from the registry). A second narrowing
  engine for isearch and a third for the results view would be the same bug
  with a better UI.
- The user's criterion is normative: "narrowable **the same way**". One
  mechanism is the only reading that satisfies it.

**Against it (the counter-arguments, and why they lose):**
- *Different surfaces want different ORDER semantics.* The picker reorders by
  score (fzf style). Issearch must keep match order (forward/reverse from the
  point — that order IS the search). Results are streamed then sorted by
  `(path, line, col)`. → Reordering is a **policy knob of the surface**, not of
  the mechanism. The shared core is *score + filter + selection-clamp*; each
  surface chooses `Reorder` (picker, unchanged) or `FilterOnly` (isearch,
  results, buffer list — v1 decision, §2.3). The seam is the scoring/filtering,
  not a universal sort.
- *Some rows have no meaningful display string (diff bodies, menu rows).* →
  Those surfaces are excluded by design (§4), not forced through. The seam
  takes a display **projection closure**, so "no good display string" is a
  surface decision, not a mechanism failure.
- *The picker's matcher instances are specialized* (two `Matcher`s:
  `file_matcher` vs `matcher`, `picker.rs:548,581`) — a naive "one matcher for
  everything" would change file-pick matching behaviour. → The seam takes the
  `Matcher` as a parameter; surfaces pass the instance they want.

**Conclusion: accept the hypothesis, with the mechanism defined as a
store-side narrowing core + a per-surface session, and reordering demoted to a
per-surface policy.**

### 2.2 The single seam (concretely)

All of it store-side (the durable rule, §5.1): the UI renders; the store
recomputes, so navigation works headlessly in tests exactly like the picker's.

1. **The core** — a free function in a new `src/app/store/narrowing.rs` (or
   `mod.rs` if extraction proves the module is too thin; decide by content,
   not by tidiness), semantically IDENTICAL to `Picker::recompute` but
   projection-based:
   - `narrow(query: &str, displays: &[&str], matcher: &mut Matcher) -> Vec<(usize, u32)>`
     — `(source-row index, score)`, best-first, `sort_by_key(Reverse(score))`
     (stable — ties keep source order, exactly today's picker). Empty query →
     every row at `u32::MAX` (today's picker behavior, `mod.rs:2948-2952`).
   - One `Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart)`
     per recompute; one `Utf32Str` scratch buffer (today's `buf`).
2. **The session** — `NarrowSession { query: String, selected: usize,
   filtered: Vec<(usize, u32)> }` with:
   - `recompute(&mut self, displays, matcher)` — calls the core;
   - the picker's selection clamp as the one rule:
     `selected = selected.min(filtered.len().saturating_sub(1))`;
   - the picker's `kind == Annotations`-style special verbs do NOT live in the
     session — they stay in the surface's key handler (verb keys are
     surface-specific, §4).
3. **The surface contract** — each narrowable surface supplies:
   - a **display projection**: `row -> String` (what gets scored; for results
     that is e.g. `"{file}:{line_no} {line}"`; for isearch the matched line's
     text; for buffers the buffer name — the SAME string the Buffers picker's
     `display` uses, built from ONE shared helper so the two cannot drift,
     §5.5);
   - a **policy**: `Reorder` (score order; the picker, unchanged) or
     `FilterOnly` (drop non-matches, keep source order; isearch, results,
     buffer list in v1);
   - a **prompt row** (what to show, §2.4) and a **key guard** (printable chars
     route to the query while the session is active — the
     `picker_key_event` guard-6 pattern, `keys.rs:169`);
   - a **decision function** over the selected source-row index (what RET does).
4. **Picker migration** — `Picker` keeps its `kind`/`prompt`/`preview`/verb
   machinery; its `query`/`selected`/`filtered` triple becomes the
   `NarrowSession` (display projection = `|c| &c.display`). Byte-for-byte
   behaviour — the picker's whole existing test suite is the acceptance, plus
   the discrimination proof in issue 01.

### 2.2a Is the seam "make every list produce picker-shaped rows"? No — named, with the cost either way

The two facts that make the unification tempting are real: `search_view_info`
(`search.rs:535`) already returns a store-computed `(rows window, top, total,
selected_row)` — the picker and the results view already speak the same
*windowing* currency — and `annotations_candidates` / `symbol_candidates`
(`picker.rs:116,465`) already produce `PickerCandidate` rows that nucleo can
filter. The question is whether the seam is "every list emits
`PickerCandidate`s and reuses the picker's machinery whole".

**Decision: NO. The seam is the display-string projection + the
`NarrowSession`/core of §2.2; `PickerCandidate` stays the picker's row type.**

- `PickerCandidate` carries picker-specific baggage that other surfaces do not
  have: `label`/`detail` (the name-first split, `mod.rs:837-859`), `docs`/`category`
  (palette/menu rows), and `ann_col` (present ONLY on the Annotations picker's
  rows, whose `detail` `path:line` is shared by every record on that line).
  Forcing a results hit or an isearch match into that shape means stuffing
  `label`/`detail`/`docs`/`category`/`ann_col` with empties in every new
  consumer — a struct that is 80% `String::new()` is the
  `issue-picker-density` disease in reverse: the field exists so one surface's
  rule does not surprise another's.
- `ResultRow` is a **discriminated** row (Header vs Hit, `mod.rs:1529`) — a
  flat candidate list cannot express the header rule (§2.3-1) without a fake
  candidate kind. `MagitRow` is *already* the magit-family's own narrow row
  type (`sections.rs:136`): the codebase has decided, twice, that list rows
  are surface-typed and the store pre-computes the window. What the surfaces
  share is the WINDOWING shape (`(rows, top, total, selected)`) and the
  NARROWING shape (query → scored/filtered indices → clamped selection) — not
  the row type.
- **Cost of this choice (named):** each surface writes a small display
  projection and a decision function; the picker and the buffer list are the
  two surfaces whose rows happen to already be `PickerCandidate`s, so issue 04
  gets nearly free (it reuses the picker's projection directly). The seam is
  ~one function + one struct, not a row-type migration.
- **Cost of the alternative (also named):** a row-type migration would
  (a) rewrite every `PickerCandidate` construction site's call shape or
  accept near-empty candidates in 4+ new places, (b) lose the header/hit
  distinction in results (the grouping rule would move into a candidate-kind
  hack), and (c) change the picker's rendered rows' data source — risking the
  name-first layout rules that `issue-picker-density` and
  `issue-picker-preview-gutter` pinned — for zero user-visible gain.

The shared currency is therefore: **display string (in), scored/filtered row
indices (out), selection (clamped), window (surface-computed)**. `PickerCandidate`
is the first and, in v1, the last *picker-only* consumer of it — exactly as
today, just through the seam.

Why a projection over the canonical rows (not a mutation of them): results
stream in through `apply_search_event` and are re-sorted by
`search_sort_hits`; a narrowing that rewrote `rows`/`hit_rows`/`selected`
mid-stream would fight the generation/generation-guard and the `(hit_index)`
invariant. The projection keeps `hits` canonical and narrows at
`search_view_info` time — a keystroke re-projects, it does not re-search.
(Isearch is the exception by nature: its query IS the search, so typing
re-runs `find_all_matches` — but the *list* then goes through the same
session/core as everything else.)

### 2.3 The real obstacles (named honestly, each with its verdict)

1. **Hierarchy.** Results (file headers over hits), magit status (folded
   sections over diff bodies), tree (depth over files) are not flat lists.
   "Narrow a hierarchy" = **filter children, keep parents**: a header stays iff
   ≥1 child survives (or the header itself matches). The core stays flat; the
   group-aware pass is a small per-surface function (results: one — §3; magit:
   declined, §4). The picker's Imenu rows are *displayed* indented but are a
   flat list today, so they need no group logic.
2. **Diff-bearing payload.** Magit status rows carry diff bodies: reorder
   destroys the diff; even filter-only must not orphan context lines from their
   hunk header. The unit of narrowing would be the *section* (magit's own
   `magit-section-narrow` concept), a different design. **Verdict: stays
   different (issue 05 records it, no implementation).**
3. **Streaming (results).** See §2.2: the narrowing must be a projection at
   view-info time; `selected` (a flat hit index) and `hit_rows` (hit→row map)
   must survive narrowing, and a narrowed-out selection must be clamped like
   the picker's. `search_view_info` grows a query parameter; nothing in
   `SearchState.hits` moves.
4. **Server-paging (log).** Filtering one page hides commits and breaks
   `n`/`p` paging semantics; real log narrowing is a `git log` query change.
   **Verdict: stays different.**
5. **Prompt-row geometry.** Every narrowed view needs a query row. Hard
   constraint (paid for twice, §5.2): ONE row, `NoWrap`, **decision keys lead**
   so a clip can never hide them. Placement decision: the query row is drawn
   **inside the view, at its top** (the picker's canvas row-0 precedent), NOT
   in the minibuffer — the minibuffer already hosts isearch's prompt, search
   echoes, and status echoes, and a second query surface there would make one
   row serve two active sessions. The isearch list keeps its EXISTING
   minibuffer prompt (§3).
6. **Key-routing conflicts.** While a narrow session is active, printable
   chars must feed the query, not self-insert / run commands. Per-surface guard
   in the modal chain (`keys.rs:11` priority chain; the picker's guard is the
   template). Each issue that adds a session owns its guard AND its
   `*_BINDINGS` count pin (`keymap.rs:712` `load_bindings_equivalence` counts
   every per-view table — a new binding moves the 127 total; the pin must be
   updated with the reason, not bumped silently).
7. **Units.** This codebase's recurring defect class is byte/char/column
   confusion (the skill's standing warning; isearch itself pins it:
   `pre_search_col` is a CHAR index, `find_all_matches` returns BYTES,
   `isearch_jump_to_current` converts). The seam deals only in display strings
   and row indices — it must never touch byte/char conversion, which stays in
   each surface's landing path.
8. **Two matcher instances.** `file_matcher` vs `matcher` exist so file
   matching is tuned separately. The seam takes `&mut Matcher` as a parameter;
   no unification of the instances (behaviour change, out of scope).

### 2.4 What "the same way" means at the keyboard (the UX contract)

Every narrowed surface, once its session is active: printable char → append to
query; Backspace/C-h → pop; the list re-ranks/re-filters live; arrows/C-n/C-p
(or that surface's existing n/p) move the selection within the filtered set;
RET runs the surface's decision on the selected row; C-g cancels (surface
semantics: close list / cancel isearch / clear results filter); the prompt row
shows `decision-keys … query` (keys lead, §5.2). The per-surface differences
are exactly: which keys, the verb keys (Stash `x`, Annotations `d`, buffer list
`d`), and what RET does. That is the entire surface of the design — anything
else is implementation.

---

## 3. The isearch decision (the user's most specific complaint)

**Decision: isearch becomes a LIST — the helm-occur / helm-swoop shape.**
"helm-like isearch" means: a live, browsable, narrowable list of the matches
in the current buffer, with a selection, instead of a cursor that teleports
match-to-match invisibly. This is what the user asked for ("more interactive"),
and it is the only reading under which "every list result should be narrowable"
covers isearch at all — a jump-to-match modal has no list to narrow.

Concrete design (issue 02):

- **Shape.** Isearch renders its matches as rows: `line number + line text +
  the match's column` (display projection = the line text; the column is
  shown, not scored). The list appears in place of the buffer content (the
  picker-overlay precedent: an overlay list with a prompt row 0 — NOT a new
  `ViewId`, because isearch must keep living inside the Buffer-view modal,
  keep `pre_search_line/col`, and must not touch the view stack; a new
  `ViewId` would change `M-,`/`close_view` semantics that many pins cover).
- **What happens to today's behaviours — each named:**
  - *Live jump while typing:* kept. On every recompute the selection is the
    first match in the search direction from the pre-search point (today's
    `isearch_recompute` rule, `search.rs:49-86`), and the buffer view behind
    the list scrolls to it — the user keeps the "the cursor moves as I type"
    feedback, now with the list beside/below it.
  - *`n`/`p` vs `C-s`/`C-r`:* C-s/C-r stay the isearch repeat keys (they move
    the selection through the list, wrapping); bare `n`/`p` are free for the
    list-navigation convention if the overlay supports it — v1: C-s/C-r only,
    matching the modal's existing key set (no new keys while the list is a
    modal; a follow-up may add n/p).
  - *RET:* confirms the **selected** match (today: the current one — same
    thing for a freshly-typed query), deactivates, point lands on the match
    (byte→line/char conversion via the existing
    `isearch_jump_to_current`, `search.rs:168`).
  - *`C-g` restore:* UNCHANGED — `pre_search_line`/`pre_search_col`
    (`mod.rs:1473,1477`) restore, the match highlight clears
    (`match_context = Default`, the issue-jump-highlight lifetime rule), the
    minibuffer says `cancel`. The list disappears with the session.
  - *Match highlighting:* UNCHANGED — `isearch_sync_match_context` keeps
    painting all matches (current one prominent) on the buffer view behind the
    list; confirm/cancel still clear it.
  - *Minibuffer prompt:* KEEPS the existing `I-search: {query} [{idx}/{count}]`
    echo (it is the prompt row for isearch, and the keys-leading rule applies:
    if the echo must be one NoWrap row, the decision info leads — the existing
    format already does). This is the ONE surface that keeps the prompt in the
    minibuffer, because isearch's prompt predates this plan and its cancel
    semantics (`cancel` echo, restore) are pinned by existing drives.
  - *Matching engine:* STAYS `find_all_matches` (case-sensitive literal, the
    emacs-fidelity choice). The list is narrowed BY the search itself — every
    keystroke re-runs the search, and the list is its result set. A second,
    nucleo-based filter-inside-the-list (typing `foo` then filtering the 200
    `foo` matches further) is a second query dimension: **explicitly deferred**,
    not silently dropped — if wanted, it is a follow-up issue that plugs into
    the same session (the session's display projection already exists).
- **Obstacle specific to isearch:** it is the one narrowable surface whose
  source rows are **recomputed on every keystroke** (the search re-runs), so
  the session must re-derive its display list each recompute (cheap: lines
  only), and the selection rule (first-in-direction from the pre-search point,
  not "keep old index") differs from the picker's clamp. The session supports
  both selection policies (clamp / re-derive) — the picker keeps clamp.

---

## 4. What must stay different, per surface (unify / differ-with-reason)

| Surface | Verdict | Reason (the honest one) |
|---|---|---|
| Picker overlay (13 kinds) | **UNIFIED** (source of the seam) | already narrows; migrated to the session byte-for-byte in issue 01 |
| Isearch | **UNIFY** (issue 02) | the user's complaint; a list with the session; §3 |
| Search results | **UNIFY** (issue 03) | flat hits under file headers; FilterOnly + keep-header-if-child-survives; projection at `search_view_info` time (§2.3-3) |
| Buffer list | **UNIFY** (issue 04) | flat, small, display string already exists; MUST share the Buffers-picker display source or the two drift (§5.5, `0829ddd`) |
| Tree sidebar | **DIFFER** | (a) hierarchical — narrowing means filter-children-keep-parents in a 34-col fixed sidebar; (b) it is the one renderer-windowed surface (§1 row 4) — unifying it starts with re-homing its windowing to the store, a refactor with no user value this week; (c) its actual job (find a file fast) is ALREADY the FindFile picker, which narrows. If the user wants tree narrowing, it is a named follow-up, not a silent extension |
| Magit status | **DIFFER** | diff-bearing payload (§2.3-2): reorder destroys diffs, filter must keep context glued to hunk headers; the right primitive is magit's own section-narrow — a different design, recorded in issue 05's inventory pin, not implemented here |
| Log | **DIFFER** | server-paged (§2.3-4); client-side filter of a page breaks paging; real narrowing = `git log` query, a different mechanism |
| Blame | **DIFFER** | read-only aligned lines; there is no query dimension the user would type (filter by author? by commit? — each is a git query, not a string filter) |
| Home view | **DIFFER** | no cursor, no selection — a help screen (`HOME_BINDINGS` deliberately empty) |
| Transient menu | **DIFFER** | prefix-key navigation IS the interaction; a query prompt would shadow the very keys it would list |
| Commit editor | **DIFFER** | a text buffer, not a list |
| Commit diff | **DIFFER** | diff payload, same reason as magit status |

---

## 5. Constraints (the durable rules this feature must respect)

1. **The store owns recomputation, not the renderer.**
   `Picker` doc, `mod.rs:860-862`: *"The filtered list is recomputed in the
   store (not the ui) so navigation keys can move through it headlessly."*
   The seam, sessions, prompts-as-state, and all windowing (the tree's
   renderer-side windowing is the named exception, §1 row 4) stay store-side;
   `src/ui/*` renders only.
2. **A prompt/minibuffer row is NoWrap, one row, and the decision keys lead,
   so a clip can never hide them.** Paid for twice: plan-017 F2 made the
   minibuffer `NoWrap` + hidden-overflow (`src/ui/root/widgets.rs` `Minibuffer`
   + `minibuffer_long_text_stays_one_row`; *"the status line is a single
   clipped row, so a right-edge clip must never hide the decision the user
   just made"*), and `7f0090a` (issue-quit-prompt-keys-invisible) then made the
   quit prompt keys-FIRST: *"(y, n, !, C-g) Save {path}? … At extreme lengths
   the truncation eats the path tail (recognisable), never the keys
   (unguessable) — a stated priority, not an accident."* Every prompt row this
   plan adds is one NoWrap row whose LEFTMOST text is the decision/verb
   information; each issue pins it the way `7f0090a` did (a mutation that
   reorders the row reddens the clip pin).
3. **One word rule.** `is_word_char(lang, c)` (`src/model/buffer.rs:71`,
   delegating to the `redline_syntax::language` table) is the single
   word-constituent rule; the ripgrep sink, M-? extraction, and motion all
   route through it (`issue-language-aware-symbols`, `b3f8234`). Any
   boundary/split logic a narrowing query needs (e.g. if a follow-up adds
   `^`/`$` anchoring or word filters) MUST call it — a second copy is the
   `issue-provider-ident-char` disease (three `is_ident_char`s still open in
   `redline-resolve`; do not add a fourth).
4. **No LSP.** The 017 doctrine: resolution/narrowing stays pure and
   in-process; no new daemon, no new protocol. (The seam scores strings; it
   needs nothing external.)
5. **Registry-driven or cross-checked — never a hand-maintained list that can
   drift (`0829ddd`).** Concretely: (a) the buffer list and the Buffers
   picker must derive their display strings from ONE helper (issue 04
   pins it both directions); (b) issue 05 lands the inventory as a
   **cross-check test**: every `ViewId`/overlay that owns row-list state
   declares narrows/does-not-narrow, and the test fails when a new list
   surface appears without a declared verdict (the `type_globs` lesson: the
   list that "can never drift" is the one derived from the same source as the
   thing it describes).
6. **Tests never veto; the lane that changes user-visible behaviour owns the
   drives.** Each issue that changes on-screen behaviour updates/registers the
   PTY drives that assert it (`tools/`, gate `SHARED_SUITES` + `pool.py`
   BATTERY), discloses each drive edit with before/after, and no assertion is
   weakened to get green (the quit-prompt precedent is the template: the drive
   was made STRONGER, asserting the new order unconditionally).
7. **A discrimination proof is required for the mechanism issue (01)** — the
   standing rule: *a check aimed at the wrong subject passes silently, and a
   silent pass looks exactly like success.* The proof must redden on a
   mutation of the SHARED scorer, with the mutation aimed at the site the
   tests actually exercise (read the test's module path first).
8. **Byte/char/column discipline.** The seam deals in display strings and row
   indices; every landing path keeps its existing conversions (isearch:
   byte→line/char, `search.rs:168-185`; results: byte col→char col,
   `search.rs:463`). No new conversion in the seam; any that is added is
   pinned per the `issue-isearch-column` precedent (multibyte fixtures).
9. **Keymap count pins move with stated reasons.** `load_bindings_equivalence`
   (`keymap.rs:712`) counts every per-view table (127 total at `20e0162`).
   Every issue that adds bindings updates the count AND the "restate the
   history, never renumber it" comment.

---

## 6. Issues and their order

Dependency order (files in parentheses — disjointness is the scheduling):

```
01 mechanism (store/mod.rs + new narrowing core; picker.rs)
 ├─ 02 isearch → list   (store/search.rs, store/keys.rs, mod.rs)
 ├─ 03 results narrowing (store/search.rs, store/keys.rs, ui/results_view.rs)  ← after 02 (same files)
 ├─ 04 buffer list      (store/mod.rs, store/buffers.rs, ui/buffer_view.rs)   ← after 01 only
 └─ 05 consistency sweep (cross-check inventory, drives, docs)                ← after 02+03+04
```

02 and 03 share `search.rs`, so they are **sequenced** (02 first), not
parallel — per the "two green lanes can still break each other" rule, 03
rebases onto 02.

### The vertical-slice question — decided

*Is the first slice (a) isearch→list, or (b) the shared mechanism?*

**Decision: (b) the mechanism first — issue 01 is the mechanism, and it lands
byte-for-byte invisible.**

Reasons (not "either is fine"):
1. **The user's own criterion forbids (a) first.** "Narrowable the same way"
   is the requirement. (a)-first means isearch builds its narrowing against
   the picker's private `recompute`, and issue 03 then either copies it or
   refactors it — either way the first consumer is welded to a
   picker-private API that the second consumer will force into the open. The
   mechanism is the deliverable; isearch is its first proof.
2. **The mechanism issue is cheap to prove and cheap to land.** It changes no
   user-visible behaviour (the picker's existing suite is the acceptance), so
   it needs no drive updates and no UX-flow additions — it is the
   lowest-risk lane in the plan, and its discrimination proof (mutate the
   shared scorer → picker tests redden) is the plan's foundation: if issue 01
   cannot prove its seam is load-bearing, no consumer issue should start.
3. **(a) first would still end with issue 01**, but with the isearch list's
   shape guessing at the seam instead of the seam constraining the isearch
   list. The order (b)→(a) is the only order in which every consumer
   implements against a proven core.
4. **The cost of (b)-first is bounded**: one refactor lane, byte-for-byte
   acceptance, before any UX work. The cost of (a)-first is a second engine
   — the exact disease this project keeps re-treating.

### Success criteria (plan level)

- ONE store-side narrowing core (§5.5), **with isearch as a measured, argued
  exception rather than a silent one.** The picker's 13 kinds, the buffer list
  and the results view recompute through it; **isearch does NOT** — it keeps
  `find_all_matches` (a literal byte search), because `narrow` is a nucleo
  score-and-REORDER over display strings and (i) isearch rows are a literal
  byte-search result where several rows can share one line's text, so scoring
  cannot recover row identity, and (ii) reordering would violate "match order
  IS the search". Isearch takes the seam's *shape* (source-ordered rows + one
  selection index + a display projection per row). **CORRECTED 2026-09-28**:
  this criterion originally read "13 kinds, isearch, the results view, and the
  buffer list all recompute through it", which was FALSE once 018-02 landed
  the mismatch finding — a criterion that cannot be met must be corrected, not
  quietly ticked. When 018-05 writes the inventory cross-check, isearch's row
  must be `narrows=true, mechanism=own (literal search)`, and the test must
  describe it that way rather than asserting a shared-core path it does not
  have.
- Isearch is a browsable, live list with today's confirm/cancel/highlight/
  restore semantics intact (each pinned; §3).
- Results and buffer lists narrow at a keys-leading one-row prompt; RET/n/p
  semantics unchanged on un-narrowed behaviour.
- The inventory cross-check test fails when a new list surface lands without a
  narrows/does-not verdict.
- Every changed drive is disclosed, updated, and stronger-or-equal; the full
  battery passes on main after each landing (the "two green lanes" rule).


---

# 01-shared-narrowing-mechanism.md

# 01 — Extract the shared narrowing mechanism (byte-for-byte picker migration)

**Status:** OPEN

## Objective

Move the ONE narrowing engine out of `Picker`'s private method and into a
shared, store-side seam, so that every later narrowable surface implements
against a proven core instead of copying `Picker::recompute`. This issue
changes **no user-visible behaviour**: the picker's observable behaviour
(query parse, scores, order, selection clamp, preview refresh) is
byte-for-byte, and its entire existing test suite is the acceptance.

## Key decisions

- The core is a pure function over **display strings and row indices** —
  semantically identical to `Picker::recompute` (`src/app/store/mod.rs:2944`):
  - `narrow(query: &str, displays: &[&str], matcher: &mut Matcher) -> Vec<(usize, u32)>`
    best-first, `sort_by_key(Reverse(score))` (stable — ties keep source
    order, exactly today); empty query → every row at `u32::MAX`
    (`mod.rs:2948-2952`); one `Pattern::parse(query, CaseMatching::Ignore,
    Normalization::Smart)` per call, one `Utf32Str` scratch buffer.
  - Lives in `src/app/store/narrowing.rs` (module) or `mod.rs` if the content
    proves too thin — decide by content, not tidiness. No new crate.
- The session is `NarrowSession { query: String, selected: usize, filtered:
  Vec<(usize, u32)> }` with `recompute(displays, matcher)` and the picker's
  selection clamp as the one rule:
  `selected = selected.min(filtered.len().saturating_sub(1))`
  (`src/app/store/picker.rs:583`, `set_picker_query`).
- The seam takes the **`Matcher` as a parameter** — the two existing
  instances (`file_matcher` vs `matcher`, `picker.rs:548,581`) stay as they
  are; the seam does not unify them.
- The seam deals in display strings and indices ONLY. No byte/char/column
  conversion enters it (PLAN §5.8).
- Picker migration: `Picker` keeps `kind`/`prompt`/`preview`/verb machinery;
  its `query`/`selected`/`filtered` triple becomes a `NarrowSession` over
  `display` projections (`|c| &c.display`). The per-kind verb keys (Stash
  `x`, Annotations `d` — `keys.rs:173,187`) stay in the surface's key handler,
  NOT in the session.
- **Not** "make every list produce `PickerCandidate`s" — the row type stays
  per-surface (PLAN §2.2a, with the costs named both ways).

## Files

| File | Change |
|---|---|
| `src/app/store/narrowing.rs` (new) or `mod.rs` | the `narrow` core + `NarrowSession` |
| `src/app/store/picker.rs` | `Picker`'s query/selected/filtered triple → `NarrowSession`; `open_picker`/`picker_query_char`/`picker_query_backspace`/`set_picker_query` route through it |
| `src/app/store/mod.rs` | `Picker::recompute` deleted (moved); `Picker` struct updated; `mod narrowing;` |
| `src/app/store/tests/picker.rs` | unchanged behaviour; ADD the discrimination pins below |

## Steps

1. Extract `recompute`'s body into the core, verbatim semantics (empty-query
   branch, `CaseMatching::Ignore`, `Normalization::Smart`, stable
   `Reverse(score)` sort, `u32::MAX` sentinel).
2. Build `NarrowSession` with the clamp rule; port `Picker` onto it. Keep
   `picker_filtered()`'s public shape (it returns
   `&[(PickerCandidate, u32)]` — `picker.rs:605`; callers in `keys.rs` and
   tests depend on the `(candidate, score)` pairing).
3. Discrimination proof (PLAN §5.7) — the acceptance, not a nicety:
   - Pin A: a mutation of the SHARED core's parse flags (e.g.
     `CaseMatching::Sensitive`) reddens an existing picker test — run it,
     quote the red test's name in the report.
   - Pin B: a mutation of the clamp (e.g. drop the `.min(...)`) reddens a
     test where the selection survives a shrinking filtered set (add one if
     none covers it).
   - Pin C (registry cross-check, PLAN §5.5, `0829ddd`): a test that
     enumerates all **13** `PickerKind`s and asserts each kind's
     `set_picker_query` path re-derives its rows through the shared core
     (e.g. by asserting the filtered order through the session for a
     discriminating query per kind, or by a code-shape check the reviewer can
     falsify). A new `PickerKind` landing without a shared-path row fails
     this test.
4. Verify byte-for-byte: the full picker test suite (`tests/picker.rs`,
   `tests/project.rs`, the Xref/Imenu/Annotations picker pins in
   `tests/navigation/*`) passes UNMODIFIED. Any test that needed editing is a
   finding to report, not a silent update.

## Verification

- `cargo build` + `cargo clippy -D warnings` + `cargo test` (bin + workspace).
- The three discrimination pins (A/B/C) with the mutation evidence quoted.
- No drive changes expected (no user-visible change); if any drive asserted
  picker internals that moved, disclose with before/after.


---

# 02-isearch-as-a-list.md

# 02 — Isearch as a list (helm-occur shape)

**Status:** OPEN (depends on 01)

## Objective

Isearch becomes a **browsable, live list** of the current buffer's matches
instead of an invisible cursor that teleports match-to-match — the user's
verbatim complaint ("the i-search results could be more interactive and
helm-like UX"). The list narrows through the SAME seam issue 01 landed
(PLAN §2): every keystroke re-runs the in-buffer search (the query IS the
search), and the resulting match set is the list.

## Key decisions (PLAN §3 is the design; this issue is its contract)

- **Shape:** the list renders as an **overlay** in place of the buffer content
  (the picker-overlay precedent, `src/ui/picker.rs`), NOT a new `ViewId`:
  isearch must keep living inside the Buffer-view modal — keep
  `pre_search_line/col` (`mod.rs:1473,1477`), must not touch the view stack
  (`M-,`/`close_view`/`normalize_top_view` pins would move).
- **Rows:** one row per match: `line number + line text + the match's
  column`. Display projection (what the session scores/filters, should a
  second filter dimension ever be added) = the matched line's text. v1: the
  list is the search's result set, in search order (forward/reverse from the
  pre-search point, `find_all_matches`, `search.rs:259`) — **FilterOnly
  policy**, no reordering (match order IS the search).
- **Matching engine stays `find_all_matches`** (case-sensitive literal — the
  emacs-fidelity choice). A nucleo filter-inside-the-list is a **second query
  dimension: explicitly deferred**, not silently dropped (it plugs into the
  same session if ever wanted).
- **Selection rule:** on every recompute the selection is the first match in
  the search direction from the pre-search point (today's
  `isearch_recompute` rule, `search.rs:49-86`) — the session's "re-derive"
  policy, not the picker's clamp. The buffer view behind the overlay scrolls
  to it (today's `isearch_jump_to_current` behaviour, `search.rs:168`).
- **RET:** confirms the selected match; point lands on the match via the
  EXISTING byte→line/char conversion (`isearch_jump_to_current`) — no new
  conversion (PLAN §5.8).
- **C-s / C-r:** unchanged keys; they move the selection through the list
  (wrapping) — exactly today's `isearch_next`/`isearch_prev`
  (`search.rs:104,119`).
- **C-g restore: UNCHANGED.** `pre_search_line/col` restore,
  `match_context` cleared (the issue-jump-highlight lifetime rule),
  minibuffer `cancel` echo, list disappears with the session.
- **Match highlighting: UNCHANGED** — `isearch_sync_match_context`
  (`search.rs:140`) keeps painting all matches (current prominent) on the
  buffer view behind the overlay; confirm/cancel still clear it.
- **Minibuffer prompt: KEEPS** the existing `I-search: {query} [{idx}/{count}]`
  echo (this surface keeps its prompt in the minibuffer — §2.4's
  keys-leading/NoWrap one-row constraint still applies to it; the existing
  format already leads with the decision info).
- **Key routing:** while the session is active the existing
  `isearch_key_event` guard (`keys.rs:304`, dispatched at `keys.rs:97-99` —
  before any keymap dispatch) is the prompt guard — printable
  chars extend the query, Backspace pops, C-s/C-r/RET/C-g route. No new keys
  in v1 (n/p list-navigation is a named follow-up, not this issue).

## Files

| File | Change |
|---|---|
| `src/app/store/search.rs` | isearch methods: recompute now (re)derives the match ROWS (line_no, text, col) in addition to `matches`; selection semantics per Key decisions |
| `src/app/store/mod.rs` | `IsearchState` gains the list fields (rows/selection or a `NarrowSession`-shaped view over `matches`); display projection |
| `src/app/store/keys.rs` | `isearch_key_event` routes selection moves; no other modal-chain changes |
| `src/ui/file_view.rs` (or a new `src/ui/isearch_list.rs`) | the overlay list renderer (prompt-less rows: number + text + col; selected-row bar per the shared cursor treatment) |
| `src/app/store/tests/search.rs` | selection/restore/highlight pins; discrimination pins |

## Steps

1. Re-derive `IsearchState`'s list: from `matches` (byte offsets) derive
   `(line_no, line_text, match_col)` rows per recompute (byte→char/line
   conversions ALREADY exist: `try_byte_to_line_col`, `buffer.rs:437`).
2. Render the overlay list while `isearch.active`; the buffer view behind
   keeps scrolling to the selected match and keeps painting
   `match_context`.
3. Key routing per Key decisions; the `isearch_key_event` guard stays the
   only entry (dispatched before any keymap dispatch, `keys.rs:97-99`).
4. Pins: RET lands on the SELECTED match's column (multibyte fixture — the
   `issue-isearch-column` class); C-g restores pre-search line AND column and
   clears highlight; a fresh query selects the first-in-direction match from
   the pre-search point (both directions); backspace to empty clears the
   list and the `[no matches]`/empty echo stays byte-for-byte.
5. Discrimination: mutate the selection rule to "keep old index across
   recomputes" → the first-in-direction pin reddens.

## Verification

- `cargo build` / `clippy -D warnings` / `cargo test` (the existing isearch
  pins in `src/app/store/tests/search.rs` — including the C-s/C-r latch and
  the highlight-lifetime pins — must pass UNMODIFIED or be reported as
  findings).
- PTY drive: isearch opens a list, typing narrows it, RET jumps to the
  selected match's column, C-g restores the pre-search point and the
  highlight vanishes. Disclose the drive with before/after (PLAN §5.6);
  register it in `gate.sh` SHARED_SUITES + `pool.py` BATTERY.
- UX flow IDs in `docs/ux-testing-plan.md` (U-E · Search & references).


---

# 03-results-view-narrowing.md

# 03 — Narrowing on the search results view

**Status:** OPEN (depends on 01; sequenced AFTER 02 — same files: `search.rs`, `keys.rs`)

## Objective

The results view (`ViewId::Search`) gets a query prompt on the list itself:
typing narrows the result rows live, fzf-style but **FilterOnly** (v1
decision, PLAN §2.2/§2.3): non-matching hits drop out, source order is kept,
file-group headers survive iff ≥1 child hit survives. `n`/`p`/RET keep their
exact semantics on the canonical hit list; the narrowing is a **projection**
computed at view time, not a mutation of `SearchState` (PLAN §2.3-3 — the
results stream in through `apply_search_event` with a generation guard and a
`Finished`-time sort; mutating `rows`/`hit_rows`/`selected` mid-stream would
fight all of it).

## Key decisions

- **Prompt placement:** a one-row, `NoWrap` prompt row **at the top of the
  view** (the picker canvas row-0 precedent), NOT the minibuffer — the
  minibuffer already hosts search echoes, isearch, and status (PLAN §2.3-5).
  **Decision keys lead** the row (PLAN §5.2, `7f0090a`): e.g.
  `filter:  RET jump · n/p · g re-run · C-g clear · q close` with the query
  trailing — at any width the clip eats the query tail, never the keys.
  Pinned by the `7f0090a` method: a mutation that reorders the row reddens
  the clip pin.
- **Narrowing target:** the flat hit list (the `hits` the user navigates),
  with display projection `"{file}:{line_no} {line}"` — file and line number
  are scored, so narrowing by file name works (the fzf property the results
  view lacks today: you cannot say "only hits in src/app"). Headers are
  re-derived from the surviving hits, in the surviving hits' file order.
- **Policy: FilterOnly.** No score-reorder in v1: `hits` stay in their
  `(path, line, col)` order (`search_sort_hits`); a narrowing reorders would
  move RET's landing target under the cursor in a way the jump-stack
  sentinel (`search_jump`, `search.rs:374` — `line: sel` recorded as the hit
  index) does not anticipate. Score-order is a named follow-up.
- **Selection:** `selected` stays a flat **hit index** (RET and the M-,
  sentinel depend on it). The prompt's selection cursor indexes the
  **narrowed** list; `n`/`p` move within the narrowed set when a query is
  active, over all hits when it is empty. When narrowing removes the
  selected hit, the session clamps (issue 01's rule).
- **Streaming:** a query typed while `running` applies to hits arrived so
  far (the projection is at `search_view_info` time — nothing buffers,
  nothing re-spawns the job). `g` re-run clears the query (a new job is a new
  result set; the prompt row shows empty) — stated, not accidental.
- **C-g semantics split:** C-g in the results view TODAY cancels the
  in-flight search (`search_cancel`, `search.rs:299`) — that stays. C-g with
  a query active and the job NOT running **clears the query** (a no-op
  message if the query is already empty). ESC/q/RET/n/p/g unchanged.
- **Title:** carries the narrowing state (e.g. ` — 12 of 480 matches` when
  narrowed), so the two numbers are always visible together.

## Files

| File | Change |
|---|---|
| `src/app/store/mod.rs` | `SearchState` gains the narrow session (or a `search_narrow` field); `SEARCH_BINDINGS` unchanged in v1 (the prompt uses printable-char routing, not new view bindings — verify; if a binding IS added, the `keymap.rs:712` count pin moves with a stated reason, PLAN §5.9) |
| `src/app/store/search.rs` | `search_view_info` takes/uses the session: narrow `hits` → re-derive `rows` window + `selected_row`; `search_next`/`search_prev` respect the active query; C-g clear path |
| `src/app/store/keys.rs` | results-view prompt guard: printable chars → query, Backspace → pop, while top view is Search and no picker is open |
| `src/ui/results_view.rs` | the prompt row (keys leading, NoWrap, one row) + render the narrowed window |
| `src/app/store/tests/search.rs` | narrowing/selection/streaming pins; discrimination pins |

## Steps

1. Add the session + display projection (`file:line line-text`).
2. `search_view_info` projects: filter hits via issue-01's core (FilterOnly),
   re-derive headers (`count` = surviving count for that file,
   `final_count` unchanged), keep `selected_row` = the selected hit's row in
   the narrowed window (`None` when narrowed out — same as today's
   out-of-window case).
3. `search_next`/`search_prev` step within the active (narrowed or full) hit
   sequence; `search_keep_visible` runs on the narrowed window.
4. Pins: narrow to a file subset (RET lands on a hit in that file; `n`/`p`
   wrap within the subset); narrow-then-clear restores the full list with
   the selection clamped, not lost; narrow mid-stream (job still running)
   shows only arriving matching hits; `g` clears the query; the prompt row
   is one NoWrap row with keys leading at 80 cols with a 200-char query
   (the `7f0090a` pin shape); RET's jump-stack sentinel still restores the
   selection on M-, (existing `search_jump` behaviour unbroken).
5. Discrimination: mutate the header rule to "keep all headers" → the
   dropped-header pin reddens; mutate the clamp to "reset selection to 0" →
   the clamp pin reddens.

## Verification

- `cargo build` / `clippy -D warnings` / `cargo test` — the existing results
  pins (including `results_view_static_render` and the U-K watchlist items:
  `search_jump` keeps the view, M-, sentinel pop) pass UNMODIFIED or are
  reported as findings.
- PTY drive: type into the results prompt, watch the list narrow, RET jump,
  M-, return with selection, C-g clear. Disclose with before/after; register
  in `gate.sh` + `pool.py`. UX flow ID in `docs/ux-testing-plan.md` U-E.


---

# 04-buffer-list-narrowing.md

# 04 — Narrowing on the buffer list (`C-x C-b`) + one shared display source

**Status:** OPEN (depends on 01). **CORRECTION (2026-09-28): the "file-disjoint from
02/03" claim in the original status line was FALSE** — measured, 04 and 02 both edit
`src/app/store/mod.rs` and `src/app/store/keys.rs`. They are independent in *substance*
(no shared logic) but NOT in *files*, so they must not land concurrently: land 02 first,
then rebase 04 onto it, and verify both sides survived. The original claim is left visible
here rather than deleted because it is the kind of design assertion a lane would otherwise
trust without measuring — which is how a silent revert happens.)

## Objective

The `C-x C-b` buffer list narrows like the other lists: a query prompt on
the list, live narrowing through issue 01's core (FilterOnly — MRU order is
the source order, and the user's muscle memory for buffer lists is MRU, not
rank). AND: the buffer list and the Buffers picker stop carrying two
hand-maintained views of the same rows (PLAN §5.5, the `0829ddd`
registry-driven/cross-checked rule).

## Key decisions

- **One display source.** The buffer list's rows and the Buffers
  `PickerCandidate`s (`buffer_candidates`, `picker.rs:62`) both derive their
  display strings from ONE shared helper (e.g. `buffer_display(key, name,
  current) -> String`), so the two surfaces score the SAME strings. Cross
  check (the `0829ddd` shape): a test that builds both row sets from the same
  fixture buffer set and asserts the display strings are pairwise identical —
  it names the diverging buffer in its failure message. A change to one
  surface's display alone reddens it.
- **Rows:** `BufferRow` stays the row type (it already exists,
  `mod.rs:928-932`; `buffer_rows`, `mod.rs:2425`). Display projection = the
  shared helper's string (the current-buffer `*` marker stays in the marker
  slot, matching `buffer_candidates`' `display` convention,
  `picker.rs:227` — the leading-space-for-`*` trick is the picker's, and the
  shared helper owns it for BOTH surfaces).
- **Prompt:** one `NoWrap` row at the top of the view, **decision keys lead**
  (`open RET · kill d · close q` + query trailing; PLAN §5.2). Keys:
  printable → query, Backspace → pop, RET opens the selected buffer, `d`
  kills the selected buffer (the list stays open, the narrowed set
  re-derives — a killed buffer drops out of the source rows, the selection
  clamps), q closes, n/p/arrows move within the narrowed set, C-g clears the
  query (v1: C-g = clear query, NOT close — stated; closing stays on
  q/ESC).
- **Selection policy:** issue-01 clamp (MRU order is stable; the selection
  survives narrowing the picker way).
- **Windowing:** today the renderer lists all rows (`src/ui/buffer_view.rs`)
  and the store keeps only `buffer_list_selected` (`mod.rs:1731`). The issue
  moves windowing to the store (`window_slice`/`pane_window`/
  `keep_cursor_visible`, `helpers.rs`) — the list must be windowed before it
  is narrowed-and-scrolled, or a 50-buffer session scrolls past the
  viewport. This is the second store-owns-windowing normalisation after the
  tree exception (PLAN §1 row 4 is DIFFER — the tree is NOT touched here).

## Files

| File | Change |
|---|---|
| `src/app/store/mod.rs` | `buffer_display` shared helper; buffer-list narrow session; `buffer_rows` + a new `buffer_list_view_info` (windowed + narrowed); `SEARCH_`-style prompt guard wiring |
| `src/app/store/picker.rs` | `buffer_candidates` (`picker.rs:62`, the Buffers + KillBuffer kinds) switches to the shared helper |
| `src/app/store/buffers.rs` | `buffer_list_next/prev/kill_selected` respect the narrowed set |
| `src/app/store/keys.rs` | buffer-list prompt guard (printable → query while on BufferList view) |
| `src/ui/buffer_view.rs` | prompt row (keys leading, NoWrap) + render the store-windowed rows |
| `src/app/store/tests/buffers.rs` | narrowing pins + the display-source cross-check |

## Steps

1. Extract the shared display helper; migrate `buffer_candidates`; prove the
   picker's Buffers/KillBuffer behaviour is byte-for-byte (existing pins in
   `tests/buffers.rs:715,746` pass unmodified).
2. Store windowing + the narrow session; prompt row; key guard.
3. Cross-check test (shared display source, names the diverging buffer).
4. Pins: narrow by buffer name; `*` current-buffer survives a query that
  matches it only via the marker slot (or not — state the rule: the marker
  is in the display string, so it IS scoreable, same as the picker today);
  `d` on the selected narrowed row kills the right buffer (the
  `issue-annotation-per-symbol-creation` lesson: address the row by what it
  identifies, not by first-match); C-g clears the query, q closes; windowing
  keeps the selection visible at 50 buffers.
5. Discrimination: mutate the cross-check's source (give the list its own
  display string again) → the cross-check reddens while behaviour tests stay
   green (the `0829ddd` failure mode: behaviour tests cannot see a display
   drift the scoring does).

## Verification

- `cargo build` / `clippy -D warnings` / `cargo test` — the existing
  buffer-list pins (`buffer_list_n_p_d_keys`, `tests/buffers.rs` picker
  pins) pass UNMODIFIED or are reported.
- PTY drive: `C-x C-b`, type to narrow, RET opens the right buffer; `d`
  kills the selected narrowed row's buffer. Disclose with before/after;
  register in `gate.sh` + `pool.py`.


---

# 05-narrowing-consistency-sweep.md

# 05 — Narrowing consistency sweep (inventory cross-check + drives + record)

**Status:** OPEN (after 02 + 03 + 04)

## Objective

Make the plan's inventory (PLAN §1) **un-driftable** and the on-screen
behaviour **drive-pinned**: (a) a cross-check test that fails when a new
list surface lands without a narrows/does-not verdict (the `0829ddd`
registry-driven/cross-checked rule applied to this plan's own inventory);
(b) the PTY drives that assert the unified behaviour end-to-end; (c) the
durable record: which surfaces stay different and WHY, and the named
follow-ups they became.

## Key decisions

- **The inventory is data, not prose.** One test (store-side, so it can read
  the surface state) enumerates every row-list surface — the 13
  `PickerKind`s (via `candidates_for`, `picker.rs:198`), the isearch list,
  the results view, the buffer list, and the declared-DIFFER set (tree, magit
  status, log, blame, home, transient menu, commit diff/editor, file view) —
  and asserts, per surface: `narrows == <the plan's verdict>` AND (for
  narrows=true surfaces) "its recompute path is the shared core" (the
  observable form: a discriminating query produces the core's ordering for
  that surface's rows — the issue-01 Pin C shape, generalized). A new
  `ViewId` or overlay owning row-list state with no declared verdict fails
  the test **and names the surface** in its failure message (the
  `type_globs_covers_all_registry_languages` failure-message rule).
- **The differ verdicts are recorded as named follow-ups, not silent
  omissions** (PLAN §4):
  - magit status: magit-native *section* narrowing (filter children, keep
    section structure; diff payload forbids row-level narrowing — PLAN
    §2.3-2). Recorded as a candidate plan, explicitly NOT this plan's.
  - log: `git log`-query-level narrowing (server-side; client-side page
    filtering is wrong, PLAN §2.3-4).
  - tree: filter-children-keep-parents in the 34-col sidebar, AFTER its
    windowing is re-homed to the store (the renderer-windowing exception,
    PLAN §1 row 4). Note its job is already served by the narrowing FindFile
    picker.
  - isearch: the nucleo filter-inside-the-list second query dimension
    (PLAN §3, deferred by explicit decision).
  Each is one paragraph in `docs/ux-testing-plan.md` (or a plan stub the
  reviewer approves) so the decision is greppable, and each is cross-
  referenced from the inventory test's comments (comment → doc, not comment →
  memory).
- **Drives own the behaviour** (PLAN §5.6): one drive per unified surface
  (isearch list, results narrowing, buffer-list narrowing) exercising
  prompt-typing → live narrowing → selection → decision key → post-decision
  state (jump/kill/open), plus the prompt-clip pins at a gate-shaped deep
  root (the `7f0090a` regime: prompt > 79 cols must still show the keys).
  Existing drives that assert the OLD isearch shape (invisible
  jump-to-match) are updated, disclosed with before/after, and made
  STRONGER (assert the list AND the old landing outcome), never loosened.
- **Tracker + docs land with this issue**: `STATUS.md` rows flip,
  `docs/ux-testing-plan.md` U-E rows get the flow IDs, and this plan's
  success criteria (PLAN § "Success criteria") are checked off in the
  landing commit message.

## Files

| File | Change |
|---|---|
| `src/app/store/tests/narrowing.rs` (new) | the inventory cross-check test + per-surface verdict pins |
| `tools/` (new/updated drives) | per-surface narrowing drives + clip pins; registered in `gate.sh` SHARED_SUITES and `pool.py` BATTERY (verified running INSIDE the battery — the `issue-annotations-symbol-precise` P2-3 rule: an unregistered driver sits outside the battery) |
| `docs/ux-testing-plan.md` | U-E flow IDs; the four named follow-ups (magit section-narrow, log git-query narrow, tree filter, isearch second dimension) |
| `.agents/plans/STATUS.md` | 018 rows flipped to LANDED with evidence (same commit as the code — the plan-process landing rule) |

## Steps

1. Write the inventory cross-check test against the landed 02/03/04 state;
   make it fail-first on a deliberately undeclared surface (a scratch
   mutation) to prove it discriminates, then green.
2. Write/refresh the drives; run the full battery (`tools/gate.sh full`,
   redirected to a file, `GATE_EXIT=$?` — never piped to `tail`).
3. Verify on MAIN after each of 02/03/04's landings already happened — this
   issue's gate is the plan-level combination check (the "two green lanes can
   still break each other" rule).
4. Record the follow-ups + flip the tracker + commit.

## Verification

- The inventory test discriminates (mutation evidence quoted).
- Battery green on main, drives registered and counted in the battery output.
- Tracker rows carry checkable evidence (a `file:line`, a test name, or a
  commit — the STATUS.md evidence rule).
- `python3 tools/check_tracker.py` passes (no task id with both an OPEN and a
  LANDED row).

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

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

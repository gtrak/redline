# 04 — Narrowing on the buffer list (`C-x C-b`) + one shared display source

**Status:** OPEN (depends on 01; file-disjoint from 02/03 — may run after 01
independently)

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

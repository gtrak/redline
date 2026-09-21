# Task: highlight search matches in the buffer view (all matches one colour, the selected one another)

**User-reported (hands-on):** *"the cursor is a bit hard to see when I jump to a search
result, I want highlighting, all visible matches one color, selected another."*

There is **no match highlighting anywhere today** (verified: no `search_face` /
`match_highlight` / `lazy` / `current_match` in `src/`). But the machinery is all there.

## What already exists (verified)

- **The row renderer is segment-based.** `render_row(x_start, row, width, text, spans, t)`
  (`src/ui/file_view.rs:170`) builds `segments: Vec<(char_start, char_end, Option<face_index>)>`
  from the syntax spans and renders each with `t.syntax_face(index)` (or the default view
  face for `None`). **A match overlay is a second pass over that segment list** — the
  natural place for this feature.
- **The span type**: `LineSpan { start, end, face: Option<usize> }`
  (`crates/redline-syntax/src/highlight.rs:57`) — byte offsets, face index into
  `HIGHLIGHT_FACES`.
- **The theme has named faces** (`src/theme.rs:49`): `status_line`, `list_item`,
  `list_item_selected`, `view`, `region`, `diff_add`, … plus `syntax_faces: Vec<Face>`
  with `Theme::syntax_face(index)` and constructors `Theme::dark(name)` / `Theme::light(name)`.
  `ThemeChoice` comes from config.
- **The store pre-computes the visible rows**: `FileViewRow { line, is_note, annotated,
  text, spans, … }` (`src/app/store/mod.rs:841`) — so per-row match ranges belong here,
  computed by the store (which knows the query), not by the UI.
- **Both match sources are available in the store**:
  - isearch: `isearch.query`, `isearch.matches` (**byte** offsets), `isearch.current`;
  - the last project search: `search.query`, `search.hits: Vec<Hit>` (project-wide — each
    hit carries a path; filter to the current buffer), `search.selected`. These **persist
    after a jump** (the jump reads `search.hits[search.selected]`), which is exactly the
    case the user reported.

## What to build

1. **Two faces in `Theme`** — one for all matches, one for the selected match (e.g.
   `search_match` and `search_match_current`). Add them to both constructors
   (`dark`/`light`) with readable colours, and keep the existing faces untouched. Emacs's
   analogues are `lazy-highlight` (others) and `isearch` (current) — the *current* one must
   be the more prominent.
2. **A match context in the store**, holding: the query, the match byte-ranges **for the
   current buffer**, and which one is selected. Set it from:
   - active isearch (query/matches/current), and
   - a search-result jump (query + the hits for this buffer + the selected hit).
   Keep it in the state that already exists where possible (`isearch`/`search`) rather than
   inventing a third copy of the query — but if a small "active match context" field is the
   honest way to express "what the buffer should highlight right now", say so and justify it.
3. **Per-row match ranges on `FileViewRow`** — computed for the visible lines only (do not
   scan the whole buffer per frame). Byte offsets, clipped to the line, and marked
   selected-or-not.
4. **The overlay in `render_row`**: after building the syntax segments, split them at the
   match boundaries and substitute the match face for the overlapping ranges. The selected
   match's face wins where they overlap. **A match range that is partly off-screen must
   clip, not panic** (the row renderer already clips by width).
5. **The selected match must be the one the cursor is on** after a jump — that is the
   user's actual complaint ("the cursor is hard to see"), so the selected highlight should
   coincide with the point.

## Key decisions

- **Lifetime — decide and state it.** When does the highlighting clear? Options: (a) on
  `C-g`/cancel and on the next *different* search; (b) whenever the point moves; (c) until
  an explicit clear. Emacs's isearch faces vanish when the search ends, but the user wants
  the context *after* jumping from results, so (a) is the coherent choice. Whatever you
  pick, **pin it with a test** — an unstated lifetime becomes an unexplainable flicker.
- **Case sensitivity and regex**: reuse whatever the search/isearch already uses; do not
  introduce a new matching rule. The isearch matches are already computed — prefer reusing
  them over re-scanning.
- **Multibyte correctness**: the renderer's segments are **char**-indexed while spans and
  hits are **byte**-offset (`byte_to_char_offset` already converts in `render_row`). Match
  ranges must go through the same conversion — a byte/char mix-up here would highlight the
  wrong cells on any non-ASCII line (the same class of bug as the isearch column fix).
- **The status line's search count already exists** — do not duplicate it.
- **Do not touch the syntax spans' own faces** outside the match ranges.

## Files

`src/theme.rs` (two faces + both constructors), `src/app/store/mod.rs` (`FileViewRow` +
the match context), `src/app/store/file_view.rs` (row construction: compute the ranges),
`src/app/store/search.rs` / `notes.rs`-adjacent (set the context from isearch/search),
`src/ui/file_view.rs` (`render_row`'s overlay), tests (`tests/file_view.rs`,
`tests/search.rs`, and a theme/face test).

## Verification

- `cargo build`; `cargo test --workspace` — reconcile against the **current** baseline
  (measure it; it will have moved) and account for every change.
- `cargo clippy --workspace --all-targets -- -D warnings` (read `${PIPESTATUS[0]}`).
- **Tests to add** (each must discriminate):
  (a) a visible line with two matches produces two match ranges, the selected one flagged;
  (b) after a search jump, the buffer's match context covers the hits **in this buffer
      only** (a hit in another file must not highlight);
  (c) the selected range is the hit that was jumped to;
  (d) a **multibyte** line: the highlighted char range is right (byte ≠ char);
  (e) a match range beyond the visible width clips without panicking;
  (f) the lifetime rule you chose (e.g. cancel clears it).
- **`timeout 900 tools/gate.sh full`** — this is a rendering change, so the PTY battery is
  the real check. If swap blocks it, run the workspace suite + both windowing drives and
  report it DEFERRED.
- Report: the two faces and their colours, the match-context design and the lifetime you
  chose, the byte→char handling, the tests with why each discriminates, and the gate output.
- **Resource guard**: `export CARGO_BUILD_JOBS=4`; check `free -g` and swap first.

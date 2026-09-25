# issue-current-line-highlight: a subtle background on the line the point is on

**User request (verbatim):** *"I want a color highlight for the current line, but not too blatant"*

## What exists today

- `FileView` paints the **region** background for rows whose buffer line is in `region_lines`, from
  `t.region.background` (`src/ui/file_view.rs:96-107`), explicitly **excluding** synthetic note rows
  (P3-3 of the clipboard work: the highlight must mean "this text will be copied").
- Two overlay passes follow, and their precedence is already established in code:
  `overlay_match_range` (search matches) then `overlay_jump_range` (the landing highlight), which
  **wins** (`src/ui/file_view.rs:362-371`).
- `Theme` is `src/theme.rs:49`, faces are `Face { background: Color, … }` (`:31`), and there is a
  `pub region: Face` (`:74`). The theme is **config-derived, set once at startup** (`:268`) — so a new
  face is a config-tunable value, which is exactly what "not too blatant" needs.

**There is no current-line face and no notion of one**, and `FileViewProps` receives `region_lines` but
**not** the point's line.

## What to build

A **subtle background tint** on the display row carrying the point — in the **file view**, and in the
**buffer view** (the notes/commit editors) if it shares the rendering path, since the request is about
"the current line" wherever the user is typing or moving. Plumb it like `region_lines` (the existing
pattern) rather than inventing a second mechanism.

**Subtlety is the requirement, and it must be made checkable rather than asserted:**
1. Add a **new theme face** (config-derived like the rest) for the current line — not a hard-coded color,
   so the shade is tunable without a rebuild.
2. Pick a **low-contrast** tint and **report the exact value plus its per-channel delta from the view
   background**. "Not too blatant" means a reader notices the line they are on without the file looking
   like it has a selected row; a delta of a few percent, not a saturated block.
3. Assert it at the **cell** level: the point's row has the face's background, and the rows immediately
   above and below have the view's normal background. A screenshot is not evidence.

## Precedence — decide it, state it, pin it

The current-line tint is a **backdrop** and must lose to every existing highlight. Establish and pin:

| on the point's line | what should be visible |
|---|---|
| a **region** span | the region wins (the user made that selection deliberately; it is more salient) |
| a **search match** | the match face wins over the tint |
| a **jump landing** | the jump face wins (already the highest precedence) |
| a **note row** (synthetic, above its code line) | **no tint** — consistent with the region's P3-3 rule; the tint belongs to rows carrying buffer text |
| the point **off-screen** | nothing is tinted |

Also state what happens when the point is on a row that is both in the region *and* a match, and when
the buffer is **folded** (a folded annotation emits no note rows).

## Scope and non-goals

- **In scope:** the file view and the buffer/notes view (wherever a buffer point exists). The editor
  cursor itself is a separate, pre-existing thing — note it, do not change it.
- **Not in scope:** the tree, picker, menu, magit status, log/blame lists — a "current row" there is a
  selection, not a point, and they already have their own selected-item faces. Say so rather than
  silently tinting one of them.
- **Rendering-only.** No change to the anchor model, notes format, region semantics, or the
  selection/clipboard behaviour landed earlier.

## Acceptance

- Point's row tinted; neighbours not; asserted per cell.
- Every precedence row in the table above asserted (each is a distinct observable).
- The face is config-derived: changing it changes the rendering, with a test proving the value is read
  from the theme rather than hard-coded.
- **Mutations**: drop the paint → the tint test reddens; invert the precedence (tint over the region or
  over a match) → the precedence test reddens.
- The existing region, match and jump behaviour is unchanged.
- Report a **literal rendered frame** for the user to judge the subtlety — that judgement is theirs, not
  the implementer's, so give them the artifact and the exact RGB values.

## Verification notes for the implementer

- `cargo build` before any PTY check (**`cargo test`/`clippy` do NOT produce the app binary**; a probe
  against a stale binary measures old code — that mistake was made twice in this project).
- **Never pipe a battery or probe to `tail`** (`cmd | tail` returns *tail's* status; it has produced both
  a false pass and a spurious FAIL here). Redirect to a file, then read `$?`.
- `pgrep -x -c timeout` before a battery (exits 1 at count 0 — never chain with `&&`). One battery at a
  time. The box runs 48 G used / 0 MB swap with sglang resident: if something fails once, capture the
  name and assertion and re-run before concluding.
- Known flake: `issue-sweep-file-search-flake` (filed, intermittent) — capture, re-run, do not dismiss
  without reproducing.
- Fence: `src/ui/file_view.rs`, `src/ui/buffer_view.rs` (only if it shares the path), `src/theme.rs`, the
  props plumbing for the point line, and tests. Disclose anything else.

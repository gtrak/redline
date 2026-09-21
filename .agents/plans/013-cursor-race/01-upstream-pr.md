# 013-01 — upstream: propose `use_cursor_position` to iocraft

**Objective.** Get a post-canvas cursor seam into iocraft, so an app can place the
hardware cursor correctly by construction instead of by timing.

**Upstream:** `github.com/ccbrown/iocraft` (the vendored 0.9.1 source links its own
issues, e.g. `crossterm.rs:530` → `issues/118`).

## Key decisions

- **Ask for the declarative hook, not a raw escape hatch.** `use_cursor_position(col, row)`
  (or a `Cursor` element) is backend-agnostic — a grid backend moves its own cursor
  rather than emitting ANSI — and matches ratatui's `Frame::set_cursor_position`
  precedent. A `use_post_frame_output(bytes)` variant is easier to implement but
  re-exposes the raw-escape layering violation redline has today; offer it only as a
  fallback if the maintainer prefers minimal surface.
- **Frame the ask as an extension of what exists** (§5 of `PLAN.md`): iocraft already has
  `use_output` → `Passthrough` → `TerminalBackend::print_above`, already threads
  `Option<&mut Terminal>` into `UpdateContext`, and already has one frame boundary
  (`synchronized_update`, `render.rs:481-497`). The missing piece is only that the
  existing channel is defined as "lines above the canvas, reflowed" instead of "a cursor
  directive after it".
- **State the ordering requirement precisely** — the emit point must be *after*
  `write_canvas`'s park (`crossterm.rs:583-586`) and *inside* the region closed by
  `end_frame` (`crossterm.rs:677`). That is the entire bug.
- **Include the minimal repro**, not just the request: iocraft hides the cursor once and
  never re-shows it; an app that wants a visible cursor on point must currently write
  `?25h` + CUP from outside the frame and race the park.

## Files

| File | Change |
|---|---|
| (upstream repo) | the proposal — issue first, PR only if invited |
| `docs/` or the issue text | the minimal repro |

## Steps

1. Reproduce minimally against iocraft 0.9.1: a component whose `use_effect` writes
   `Show` + `MoveTo` — observe it clobbered by the park; add a sleep and observe it
   become load-dependent. (Redline itself is the full repro; a 30-line standalone is
   better for upstream.)
2. File the issue with: the observed behaviour, the exact emit-point requirement, the
   proposed API, and the ratatui precedent.
3. Only open a PR if the maintainer is receptive — otherwise this plan's option B is the
   path.

## Verification

- The issue exists and is linkable; the repro is self-contained and does not depend on
  redline.
- **Do not** block redline work on an upstream response (see `02-vendored-patch.md`).

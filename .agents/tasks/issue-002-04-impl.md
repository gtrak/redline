# Task: Implement plan 002 issue 04 — Layout collapse sweep (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained: read it, then execute it in order. The library references in
`.agents/skills/` are authoritative ground truth.

Context: plan 001 archived; plan 002 issue 01 (transient menus, discard,
inline hunks) committed. A live sized-PTY drive of the shipped build found
that **row-content views overprint their own titles/section headers** —
magit status renders `*magit-status*`, `## Staged changes`, and file rows on
the SAME rows; the tree sidebar stamps its 120 file rows onto ~4 lines
(`CarCaR01- 02-…`). This is the missing `flex_direction: Column` +
definite-height class FileView had (fixed in issue 03/09) — but it was never
swept across the other views. The user experiences this as "magit isn't
fleshed out" and "no cursor": the cursor IS there, buried in collapsed rows.

## Working agreement (overrides any caution)

- **Skills are truth.** Work straight from `.agents/skills/*.md`. Do NOT read
  dependency sources under `~/.cargo/registry`, do NOT browse docs.rs, do NOT
  fetch anything. If a skill lacks an API detail you need, write the most
  reasonable call consistent with the skill and keep moving.
- **Write-first.** Audit and fix in your first handful of tool calls per
  view, compile early, iterate. No front-loaded research.
- **Skill corrections.** If running code contradicts a skill file: code
  reality wins, AND you make a minimal, factual correction to the skill file;
  list edits under "skill corrections".

## Read first (in this order)

1. `.agents/plans/002-magit-depth-and-cursor/04-layout-collapse-sweep.md` —
   THIS issue.
2. `.agents/skills/iocraft/SKILL.md` — flexbox layout (flex_direction
   defaults to ROW; a View of Text rows laid out horizontally = all rows
   stamped on one line — the exact bug).
3. `src/ui/file_view.rs` — the FIXED reference pattern (title + canvas in a
   Column, canvas flex_grow with definite parent height).
4. The view renderers: `src/ui/magit_status.rs`, `src/ui/tree.rs`,
   `src/ui/rows_view.rs` (log/blame/commit-diff/editor rows),
   `src/ui/results_view.rs`, `src/ui/views/buffer.rs`,
   `src/ui/transient_menu.rs`, and their render arms in `src/ui/root.rs`.

## Constraints

- Do not add, remove, or bump any dependency in `Cargo.toml`.
- Fix LAYOUT only — no behavior changes, no new features, no face changes
  (cursor visibility is issue 02). Keep every existing binding, keymap, and
  store interaction untouched.
- The FileView (already fixed) and the Picker (already renders correctly)
  must not regress.
- Where a container needs a definite height, use the established
  `use_terminal_size` pattern from root.rs; where rows are Text children,
  ensure the parent is `FlexDirection::Column`.

## What to build

- Sweep EVERY view that renders rows of text and verify/fix its container
  structure so that: the title (if any) is on its own row; section headers,
  rows, and help lines each occupy their own row; the content area fills the
  available pane; nothing overprints. Views to audit (fix as needed):
  - MagitStatus (title + section headers + rows + help line — the live-
    verified broken one),
  - TreeSidebar (rows stamped onto ~4 lines — live-verified broken),
  - LogView / BlameView / CommitDiffView (via rows_view.rs),
  - ResultsView (search results),
  - BufferListView + BufferView (views/buffer.rs),
  - TransientMenu (bottom-of-frame overlay — verify it does not overprint
    the status line and is cleared when closed).
- The render arms in `root.rs` may need the Column/flex_grow structure per
  view; keep the tree+main Row layout (tree left 34 chars) intact.

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green (baseline 334).
- **Sized-PTY verification is the core of this issue** (the bug is invisible
  to unit tests — that is why it shipped). Use the established harness:
  python pty + pyte screen reconstruction at 100x30, against the built
  binary. For EACH view, reconstruct the screen and assert:
  - the title row and content rows are DISTINCT (no row contains text from
    two different view elements),
  - section headers / rows appear once each (no duplicates from overprint),
  - the status line is on the last row and spans the width.
  Views to drive: magit status (with a dirty file staged/unstaged), tree
  toggle, search results (run a search), buffer list, transient menu open +
  closed, log, blame (on a file with history).
- Structural regression guard: add a store/renderer-level test that pins the
  element structure per view where feasible (e.g. static render via
  `to_string()` asserting the title and first content row render on separate
  lines) — the 09 keymap-table test is the anti-drift precedent.
- Declare in your report exactly which views were broken, what the structural
  fix was per view, and the pyte evidence per view.

## Report format

- **Per-view table**: view | was broken? | structural fix | pyte evidence.
- **Verification**: exact commands + pass/fail + counts.
- **Skill corrections** (or none). **Deviations**; known gaps.

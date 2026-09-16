# Task: Implement plan 002 issue 02 — Cursor & rendering audit (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained: read it, then execute it in order. The library references in
`.agents/skills/` are authoritative ground truth.

Context: plan 002 issues 01 (transient menus, discard, inline hunks), 04
(layout collapse sweep), and 05 (editable keys + coherence) are committed.
A live sized-PTY drive of the shipped build found, with terminal-attribute
evidence: **the magit cursor bar does not render** — after 8 cursor-down
(`n`) presses, NO row in the magit view changed attributes (checked via pyte
`Char.fg/.bg/.reverse`). The user independently reports "I don't see a
cursor". Magit rows stack `invert: selected` ON TOP OF the
`section_heading_selected` face (white-on-blue bold) — the double treatment
can visually cancel on some terminals, or the face may not reach the canvas.

## Working agreement (overrides any caution)

- **Skills are truth.** Work straight from `.agents/skills/*.md`. Do NOT read
  dependency sources under `~/.cargo/registry`, do NOT browse docs.rs, do NOT
  fetch anything. If a skill lacks an API detail you need, write the most
  reasonable call consistent with the skill and keep moving.
- **Write-first.** Fix in your first handful of tool calls per finding,
  compile early, iterate. No front-loaded research.
- **Skill corrections.** If running code contradicts a skill file: code
  reality wins, AND you make a minimal, factual correction to the relevant
  skill file; list edits under "skill corrections".
- **graft is available** (CLI on PATH): `graft map/ask/callers/skeleton/grep`.

## Read first (in this order)

1. `.agents/plans/002-magit-depth-and-cursor/02-cursor-and-rendering.md` —
   THIS issue.
2. `.agents/plans/002-magit-depth-and-cursor/PLAN.md` — the live-UX audit
   findings.
3. `src/ui/diff_view.rs` — `row_face` (the magit/log/blame row face
   selection: `section_heading_selected` + invert stacking).
4. `src/ui/magit_status.rs`, `src/ui/rows_view.rs`, `src/ui/tree.rs`,
   `src/ui/results_view.rs`, `src/ui/picker.rs` — how the selected row is
   rendered per view (face vs invert vs canvas attributes).
5. `.agents/skills/iocraft/SKILL.md` — Text styling (invert, colors),
   canvas drawing attributes.

## The finding to fix (with diagnosis)

**Cursor invisibility in magit (and audit everywhere else).** The magit
selected row applies `row_face(role, selected=true)` =
`section_heading_selected` (white fg, blue bg, bold) AND sets
`invert: selected` on the Text. Stacking invert on an explicit fg/bg
swaps fg<bg — white-on-blue becomes blue-on-white — which on some terminals
renders nearly identical to unselected rows (and loses the blue bar). The
list views (picker/buffer-list/tree) use `list_item_selected` (white/blue +
bold) WITHOUT invert and are visible.

Fix approach (verify against the live render, not just the code):
1. In the magit/log/blame row renderers: use ONE unambiguous treatment —
   either the explicit white-on-blue face WITHOUT invert, or invert WITHOUT
   a competing face — pick the one that pyte-verifies as a distinct
   attribute change (fg/bg swap present in the output stream).
2. Sweep every cursor-bearing view (magit, log, blame, commit-diff, results,
   tree, buffer-list, picker) and confirm the selected row produces a
   DISTINCT attribute signature in the render output.
3. Add a pyte-based cursor-visibility check to the verification: navigate
   with `n`/`p` and assert the highlighted row's attributes CHANGE
   (fg/bg/reverse present in the emitted stream for the selected row and
   absent for others). A cursor test that passes on unselected rows is
   worthless — the assertions must discriminate.

## Constraints

- Do not add, remove, or bump any dependency in `Cargo.toml`.
- No behavior changes: keys, store state, commands untouched. This is a
  RENDER-level fix + verification issue (plus the magit status windowing
  below).
- Do not regress the issue-04 layout fixes (distinct rows) or issue-05 keys.

## Also in scope: magit status windowing

Long magit buffers clip at the pane edge (rows past the viewport vanish; the
help line pushes off). Add cursor-following windowing to the magit status
rows (the store already tracks the cursor; compute a scroll window like
FileView's — visible rows around the cursor, help line pinned). Keep it
simple: a scroll offset in the store's magit state, adjusted on cursor
movement, clamped to the row count.

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green (baseline 345).
- **pyte cursor-visibility checks (the core)**: in a tempdir git repo with
  staged + unstaged changes (so magit has files AND hunks), drive the magit
  view: press `n` repeatedly and assert, at each step, that exactly ONE row
  carries the selected-row attribute signature (fg/bg swap or reverse) and
  it is the row the store's cursor points at; press `p` and assert it moves
  back. Same for the tree (`C-c p t` + arrows), buffer list, log, and search
  results.
- Windowing: with a repo producing a tall status buffer, the cursor stays
  in view while pressing `n` past the pane bottom (window scrolls), and the
  help line remains visible.
- No-regression: FileView/Picker render unchanged; issue-04 layout
  assertions still pass; issue-05 key tests still pass.
- Declare in your report which behaviors are test-covered, PTY-covered,
  manual-only.

## Report format

- **Per-view cursor table**: view | render treatment before | after | pyte
  evidence.
- **Windowing**: design + pyte evidence.
- **Verification**: exact commands + pass/fail + counts.
- **Skill corrections** (or none). **Deviations**; known gaps.

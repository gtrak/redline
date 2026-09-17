# Task: Implement plan 002 issue 03 — Sweep & re-measure (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained. This is the FINAL issue of plan 002: a verification-first
sweep of the shipped surface, with two bounded fixes if (and only if) the
sweep reproduces them.

## Working agreement (overrides any caution)

- **Skills are truth.** Work from `.agents/skills/*.md`. Do NOT read
  dependency sources under `~/.cargo/registry`, do NOT browse docs.rs, do
  NOT fetch anything.
- **Write-first / act.** Drive the app early, iterate on what you see. No
  front-loaded research.
- **Skill corrections.** If running code contradicts a skill file: code
  reality wins, AND make a minimal factual correction to the skill file;
  list it under "skill corrections".
- **graft** CLI available if you want code map queries.

## Read first

1. `docs/ux-testing-plan.md` — the U-* flow definitions (~84 references;
   the findings log you will update) and the live-drive harness notes.
2. `tools/` — the committed pyte drive suite (`drive_all.py`,
   `pyte_driver.py`, …) and its assertion style (attribute-level,
   exactly-one, non-vacuous).
3. `README.md` perf table (lines ~128-136) and how those numbers were
   produced.
4. `.agents/plans/002-magit-depth-and-cursor/PLAN.md` — issue 03 contract.

## Part A — the sweep (primary deliverable)

Drive the REAL binary (`target/debug/redline`, freshly built) in sized PTYs
with pyte reconstruction, executing every U-* flow in
`docs/ux-testing-plan.md` that applies to the current build. For each flow:
the keys sent, the pyte-verified outcome, PASS/FAIL. Extend `tools/` with a
sweep script if that makes the run reproducible (preferred — the suite is
committed evidence).

Layout-shift overprint check (the one plan-002 objective not yet
attribute-verified): at every view switch (FileView↔magit↔log↔results,
tree toggle `C-c p t`, palette open/close, picker open/close), reconstruct
the frame and assert no row carries content belonging to two views
(overprint = title text sharing a row with body content, or stale
view content persisting after the switch). Record PASS/FAIL per transition.

## Part B — bounded fixes (only if the sweep reproduces them)

1. **Overprint/flash at layout shifts.** If any transition shows overprint
   or stale-frame artifacts: investigate iocraft's diff painter (0.9.1
   render internals) via the `.agents/skills/iocraft/SKILL.md` ground
   truth; candidate fixes: full-frame clear on layout-change renders, or
   forcing a full repaint when the view stack changes. Render-layer only;
   no store/keys changes. Must keep 346 tests green.
2. **Tree test pin.** The tree structural test (added in issue 04) can pass
   vacuously (asserted row can clip onto the title line pre-fix). Strengthen
   it: assert the FIRST tree row's text lands on a line distinct from the
   title line, and ≥2 tree rows render on distinct lines. Test-only.

Do NOT fix anything else you find — report it as a finding with pyte
evidence; the orchestrator scopes follow-ups.

## Part C — re-measure

- Re-record the README perf numbers with the same measurement method the
  current table used (cold start, index, search first-hit; verify the
  method from git history/docs if unclear — state the method in your
  report). Update the table if numbers moved materially (>20%); otherwise
  leave and note.
- Update `docs/ux-testing-plan.md` findings log: mark which U-* findings
  from plans 001/002 are now verified fixed (cursor, layout collapse,
  editable keys, notes conflict, graft pollution, demo commands, q-quit,
  windowing, overprint status), and append any NEW findings from Part A
  with pyte evidence.

## Constraints

- Do not add/remove/bump dependencies. No new features. Keys and store
  behavior unchanged except where Part B.1 explicitly authorizes a
  render-layer fix.
- Gates: `cargo build` clean; `cargo clippy --all-targets -- -D warnings`
  clean; `cargo test` all green (baseline 346; +1 if the tree-pin test
  changes assertions, still 0 failures).

## Report format

- **Sweep table**: flow | keys | pyte evidence | PASS/FAIL.
- **Overprint matrix**: transition | PASS/FAIL | (fix applied + re-verify if
  fixed).
- **Perf**: method, before/after numbers, table updated or not.
- **Docs**: findings-log diff summary.
- **Verification**: exact commands + counts. **Skill corrections** (or
  none). **Deviations**; **new findings** (unfixed, with evidence).

# Task: Implement plan 003 issue 03 — Sweep round & carried items (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained. `.agents/skills/*.md` are authoritative ground truth.

Context: plan 003 issues 01 (watcher Access fix) and 02 (shared windowing
across commit-diff/blame/log/editable buffers) are committed. This is the
FINAL issue: close the small carried non-blockings, fix the banner copy,
extend the sweep, re-run everything.

## Working agreement (overrides any caution)

- **Skills are truth.** Work from `.agents/skills/*.md`. Do NOT read
  dependency sources under `~/.cargo/registry`, do NOT browse docs.rs, do
  NOT fetch anything.
- **Write-first / act.** No front-loaded research.
- **Skill corrections.** Code reality wins over a skill file; make a
  minimal factual correction and list it under "skill corrections".
- **graft** CLI available.

## Read first

1. `.agents/plans/003-scroll-and-watcher/03-sweep-and-carried.md` — THIS
   issue (the contract).
2. `docs/ux-testing-plan.md` — Backlog table (plan-003 candidates) and the
   findings-log conventions.
3. `src/ui/file_view.rs` (~237) + `src/app/store.rs` (~4157) — the
   changed-on-disk banner text and where it renders.
4. `tools/sweep_flows.py` — flow_f4, flow_c6, flow_g3, flow_b3, the
   D-group N/A entry (~662); `tools/drive_windowing_panes.py` — the new
   pane drives.

## What to build

1. **Banner hint per kind** (small src change, the only one): the
   "⚠ changed on disk — press g to reload" hint is misleading on editable
   buffers, where plain `g` self-inserts by design and the reachable path
   is `M-x reload-buffer`. Render the hint with the accurate key per
   buffer kind (editable → `M-x reload-buffer`; plain → `g`). Unit tests
   assert both texts.
2. **Carried small items** (each with a test or a truthful note):
   - F4 tracked-path magit refresh: store unit test — tracked-path change
     → dirty counts updated (the classifier is tested; the refresh leg is
     not).
   - flow_c6: assert the bottom anchor (last content row reads line 50
     after M->/G), not just movement.
   - flow_g3: assert the typed char is present before reload (typed-char
     precondition).
   - flow_b3: mark the partial leg in the record (no-match leg only) per
     the log's own conventions.
   - D-group N/A: drop the overbroad "not a single-PTY pyte assertion"
     clause for imenu/symbol pickers; keep the true unit-test claim.
   - tree.rs test doc-comment: "before the store is moved" → the store is
     Arc-wrapped (comment precision).
3. **New sweep flows** in tools/sweep_flows.py (thin legs are fine — the
   deep drives exist in tools/drive_windowing_panes.py; cite it): one
   commit-diff scroll leg, one blame windowing leg, one notes-scroll leg,
   and a banner-hint check driven per kind (type in notes → hint says
   M-x reload-buffer; plain file → hint says g).
4. **Backlog rows** (docs only): add search_keep_visible repoint-to-helper
   and commit-editor unwindowed as backlog candidates (from the issue-02
   review); flip backlog items addressed by this issue to FIXED.
5. **Full suite re-run** (below) and findings-log update.

Deferred (do NOT do): menu overflow "+N more" indicator; armed-discard
TOCTOU re-validation; misleading "nothing staged" error when git_status
errors; insertion-point cue in editable buffers.

## Constraints

- The banner hint is the ONLY src behavior change. Everything else: tests,
  flows, docs. No dependency changes. PTY invocations wrapped in
  timeout/deadline-read.

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green (baseline 354 / 2 ignored, + new tests, 0 failed).
- `tools/sweep_flows.py` all driven flows PASS (39 + new legs);
  `tools/sweep.py` 14/14; `tools/drive_windowing.py` 28/28;
  `tools/drive_all.py` 6/6; `tools/drive_windowing_panes.py` 4/4.
- PTY: banner text correct per kind on the real render.
- Coverage split in the report: test-covered / PTY-covered / manual-only.

## Report format

- Per-item disposition (done + evidence / note). Gate outputs (exact
  counts). Skill corrections (or none). Deviations; known gaps.

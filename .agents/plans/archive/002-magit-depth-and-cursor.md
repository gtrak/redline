# 002 — Redline: magit depth & cursor polish (ARCHIVED)

Status: implemented. Issues 01/02/04/05/03 in that execution order; field
feedback kept reshaping scope mid-plan (issues 04/05 were added from the
orchestrator's sized-PTY drives of the shipped build).

## Scope delivered

- **01 Magit depth** (2c0faf0): transient menu (hydra tree, `?`/`h`,
  anti-drift — derived from keymap×registry), inline diff hunks under status
  rows (fold-aware, cursor-addressed: n/p/s/u/TAB/RET), k-discard verbs
  (confirmation-gated), EOFNL mapping from libgit2 ground truth
  (blob-truth-derived splice flags after 5 review rounds), editor polish
  (nothing-staged refusal), branch-create in picker.
- **02 Cursor & rendering audit** (fe1a951): every cursor-bearing view
  renders the selected row as a per-row View background bar (white-on-blue,
  ANSI 256-color, no invert — light-theme white-on-white failure
  structurally impossible); magit status windowing (clamped follow-scroll,
  pinned help line); committed pyte drive suite under tools/ (attribute-
  level, exactly-one-blue-row, zero-reverse, cursor-trajectory matched).
- **04 Layout collapse sweep** (ebddde9): five views fixed with the
  FileView pattern (magit status, tree sidebar, MagitRowsView, results,
  buffer list) — row content no longer overprints titles; 5 structural
  regression tests through the real flexbox pass.
- **05 Editable keys & content coherence** (9dbc8c9): prefixes/prefix-tails
  route through the keymap before self-insert while editing; printable
  depth-1 leaf commands self-insert (g can no longer destroy unsaved notes
  text — orchestrator decision on reviewer escalation); notes self-creation
  doesn't false-conflict (created_paths); graft/ pruned from file walk
  (grep keeps .ignore); demo commands removed (75→71); bare q closes views,
  C-x C-c quits.
- **03 Sweep & re-measure** (2b1fac0, 5a1be9f): 39 pyte-driven U-* flows +
  14/14 overprint matrix (negative-controlled), committed under tools/;
  perf re-measured with a committed method (src/perf.rs microbench; README
  table updated: cold start ~310/~75 ms, index ~110/~33 ms, first-hit
  ~6/~1.1 ms); findings log updated; new unfixed finding recorded (watcher
  Access events self-sustain a ~500 ms reload loop → false changed-on-disk
  ~0.5 s after typing).

## Verification state at archive

346 tests / 0 failed / 2 ignored; clippy --all-targets -D warnings clean;
tools/sweep.py 14/14; tools/sweep_flows.py 39/39 (two consecutive runs);
every issue review-gated to VERDICT: PASS.

## Carried forward

Backlog (docs/ux-testing-plan.md): scrollability sweep (commit-diff/
git-show pane, blame, long editable buffers, log within-page stranding —
user-requested), watcher Access-event fix, banner nuance (g on editable
buffers), nine carried review non-blockings.

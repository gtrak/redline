# 01 — Magit depth: inline diffs, section cursor, editor polish

Phase 1 · Depends on: —

## Objective

Bring the magit status buffer from "works" to "daily-driver": the user's
field report is "magit isn't fleshed out yet".

## Key decisions

- **Transient menus (user directive: "magit has a hydra tree, I want the
  full functionality")**: the magit interaction model is its transient tree
  -- `?`/`h` shows a menu at the bottom of the frame listing the view's
  bindings (key + description, grouped by registry category); prefix keys
  shown with an ellipsis expand into their submenu (the hydra tree);
  pressing a listed leaf key executes it directly; C-g closes. Source of
  truth: the view keymap x registry metadata.
- **Missing verbs**: `k` discard at file/hunk level (destructive,
  confirmation-gated per magit), `h` top-level dispatch. The full verb table
  lives in .agents/skills/magit/SKILL.md.
- Inline hunks: show staged/unstaged hunks under their files directly in the
  status buffer (fold state per file, TAB toggles), reusing the diff
  renderer. RET on a hunk still visits the file.
- Section cursor: the cursor row must be unmistakable on any terminal
  (coordinate with issue 02's audit; the magit face may need the same
  white-on-blue treatment as list views rather than invert).
- Editor polish (from review non-blockings, now promoted): refuse commit with
  nothing staged; C-g in the editor clears only an armed prefix (emacs
  convention), not the whole buffer; drop dead ESC/C-g bindings in the
  CommitEditor keymap; fix the cursor doc (char index, not byte offset).

## Steps

1. Inline hunks in the status section tree (fold-aware, cursor-aware).
2. Section cursor visibility pass on magit rows.
3. Editor polish items above (each with a regression test).

## Verification

- Sized-PTY: status buffer shows hunks inline; fold/unfold; cursor obvious.
- Git-CLI cross-checks unchanged (staging exactness regression tests).
- Editor: nothing-staged refusal test; C-g prefix-only test.

# 02 — Cursor & rendering audit

Phase 2 · Depends on: 01

## Objective

Every navigation view shows a visible cursor on the user's terminal; the
frame overprint/flash at layout shifts is gone.

## Steps

1. Cursor audit: magit, search, tree, buffer-list, log, blame — same
   high-contrast face everywhere; pyte-verify the selected row renders
   distinct (background/invert escapes present in the frame).
2. Overprint/flash: reproduce in pyte at layout shifts (tree toggle, view
   switches); investigate iocraft's diff painter (0.9.1 render internals);
   candidates: full-frame clear on layout-change renders, double-buffer
   strategy, or reduced repaint churn. Fix + verify no overprint artifacts.

## Verification

- pyte: no stale cells after view switches/tree toggle (reconstruct screen,
  assert no ghost text from previous frames).
- User confirms flashing gone on their terminal.

# 03 — Sweep & re-measure

Phase 3 · Depends on: 02

## Objective

Close the plan with a full regression sweep and fresh numbers.

## Steps

1. Run all U-* flows from docs/ux-testing-plan.md (sized-pty + manual).
2. Re-record perf numbers (cold start, index throughput, search latency).
3. Update the UX findings log; archive this plan.

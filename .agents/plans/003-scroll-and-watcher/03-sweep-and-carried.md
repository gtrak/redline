# 03 — Sweep round & carried items

Phase 2 · Depends on: 01, 02

## Objective

Close the small carried non-blockings, fix the misleading conflict-banner
copy, extend the sweep to the new scrollable panes, and re-run the full
suite so plan 003 archives with green books.

## Key decisions

- Banner copy: the "⚠ changed on disk — press g to reload" hint must be
  accurate per buffer kind — on editable buffers the reachable path is
  `M-x reload-buffer` (plain `g` self-inserts by design). Smallest honest
  fix: per-kind hint text (unit-tested for both kinds).
- Carried non-blockings are all small; each lands with its own test or a
  true NOT_APPLICABLE note. No scope creep beyond the backlog table.

## Files

| File | Change |
|---|---|
| `src/ui/file_view.rs` + store | Per-kind banner hint (editable vs plain), unit-tested. |
| `tools/sweep_flows.py` | New flows: commit-diff scroll legs, blame windowing, notes scrolling, banner-hint check per kind. |
| Small carried items (each with test/note) | F4 tracked-path magit-refresh unit test; flow_c6 bottom-anchor assert; flow_g3 typed-char precondition; flow_b3 partial-leg note; D-group N/A imenu clause fix; tree.rs test doc-comment wording. |

Deferred (not in this plan): menu overflow "+N more" indicator; armed-
discard TOCTOU re-validation; misleading "nothing staged" error when
git_status errors — they stay in the backlog table.

## Steps

1. Banner hint per kind + tests.
2. Carried small items (list above).
3. New sweep flows for scrollable panes + banner.
4. Full suite re-run.

## Verification

- Gates green (build / clippy / cargo test, ≥ 346 + new tests, 0 failed).
- `tools/sweep_flows.py` all driven flows PASS (39 + new); `tools/sweep.py`
  14/14.
- Findings log updated (backlog rows flipped to FIXED where addressed).

# 01 — isearch printable interception

Phase 1 · Depends on: —

## Objective

While isearch is active, every printable extends the query string — keys
bound as commands in the current view (`n`, `p`, `l`, `g`, `q`, `j`, `k`…)
must NOT be dropped or dispatched. Typing `line_5` must yield `line_5`.

## Key decisions

- Fix in the isearch interception branch of `key_event` (store.rs): it must
  run before keymap dispatch for ALL printables while the isearch prompt is
  armed — the exact ordering the notes buffer got in plan-002 issue 05.
- Non-printables keep emacs isearch semantics: C-s next match, C-r
  reverse, RET end at match, C-g cancel, DEL rubout.

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | isearch branch interception order + regression tests (query with n/p/l/g inside; count continuity). |
| `tools/drive_redline_parity.py` | fix the C-x b keystroke bug (0x62, not 0x32); add isearch-with-bound-keys legs. |

## Verification

- Gates green (build / clippy / cargo test ≥ 357, 0 failed).
- PTY: `C-s` → `line_5` full query, match count continuity `[k/120]`; `n`
  inside a query; C-r reverse; RET lands at match; C-g restores point.

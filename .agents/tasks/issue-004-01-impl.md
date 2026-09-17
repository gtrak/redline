# Task: Implement plan 004 issue 01 — isearch printable interception (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained. `.agents/skills/*.md` are authoritative ground truth.

Context: plan 003 is archived; plan 004 (docs/emacs-parity-log.md is the
decision record) is active. The orchestrator drove vanilla emacs 30.2 and
redline through the same pty+pyte battery and found one functional defect:

**BUG (reproduced)**: while the file-view isearch prompt is armed, printable
keys that are bound commands in the current view are DROPPED from the
query. Exact repro (tools/drive_redline_parity.py battery): open
src/main.rs, `C-s`, type `l` → `I-search: l [1/120]` (correct); continue
`ine_5` → `I-search: lie_5 [no matches]` — the `n` vanished. In emacs,
every printable extends the search string; `n` is just a letter.

## Working agreement (overrides any caution)

- **Skills are truth.** Work from `.agents/skills/*.md`. Do NOT read
  dependency sources under `~/.cargo/registry`, do NOT browse docs.rs, do
  NOT fetch anything.
- **Write-first.** Fix early, compile, iterate. No front-loaded research.
- **Skill corrections.** Code reality wins over a skill file; minimal
  factual correction, listed under "skill corrections".
- **graft** CLI available.

## Read first

1. `.agents/plans/004-emacs-parity/01-isearch-interception.md` — the issue.
2. `docs/emacs-parity-log.md` — battery 1, row 1 (the defect) and row 2-4
   (what must NOT regress).
3. `src/app/store.rs` — the isearch interception branch in `key_event`
   (search for the `I-search` prompt state and its arm/disarm paths), the
   plan-002 issue-05 notes-branch interception (the correct-order reference:
   pending-prefix and depth-1 printable handling).
4. `src/app/command.rs` — which printables are depth-1 leaf commands in the
   file view (`n`, `p`, `l`, `g`, `q`, `j`, `k`, `G`…).
5. `tools/drive_redline_parity.py` — the battery (also fix its C-x b
   keystroke bug: it sends 0x32 '2' instead of 0x62 'b').

## What to build

1. **Fix**: while the isearch prompt is armed in the file view, EVERY
   printable self-inserts into the query — run the isearch branch before
   any keymap dispatch for printables (the isearch analogue of the
   plan-002 issue-05 notes fix). Chords (C-s next, C-r reverse, RET end,
   C-g cancel, DEL rubout, arrows if bound) keep their isearch semantics.
2. **Unit tests**: query containing `n`, `p`, `l`, `g`, `q` letters
   end-to-end; match-count continuity while typing; C-s/C-r/RET/C-g
   unchanged; the regression guard must fail against the old code
   (discriminating).
3. **Battery fix + legs** (tools/drive_redline_parity.py): correct the
   C-x b key; add an isearch leg typing a query containing bound-command
   letters and asserting the full query renders with match-count
   continuity.

## Constraints

- The isearch interception must not regress: results-view search (C-c p s),
  the C-g matrix, pending-prefix semantics, notes editable branch, the
  magit/tree/log panes. No dependency changes.
- Scope fence: src/app/store.rs (+ its tests), tools/drive_redline_parity.py.
  Nothing else.

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green (baseline 357 / 2 ignored, + new, 0 failed).
- `python3 tools/sweep_flows.py` 43/43; `python3 tools/sweep.py` 14/14;
  `python3 tools/drive_all.py` 6/6; `python3 tools/drive_windowing.py`
  28/28; `python3 tools/drive_windowing_panes.py` 4/4.
- PTY: the exact repro now types `line_5` fully (`I-search: line_5 [k/120]`),
  C-s advances, C-r reverses, RET lands at match, C-g restores point.
- Coverage split in the report.

## Report format

- Root cause (which branch ate the key and why) + fix shape. Gate outputs
  (exact counts). Skill corrections (or none). Deviations; known gaps.

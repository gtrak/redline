# 07 — Git status & staging

Phase 4 · Git · Depends on: 01 (soft: 03 for visit-file; 04 for live refresh)

## Objective

Magit status — the glance surface for agent churn: see what changed, read
diffs, stage/unstage files and hunks.

## Key decisions

- **git2** wrappers isolated in `git/`; git2 types never leak into the UI.
- Magit buffers are **section trees** (plain data models) rendered by
  generic components → snapshot-testable.
- **Hunk staging** via patch application against the index; every staging
  operation is verified against the git CLI in tests.
- Status subscribes to the watcher bus — no manual refresh needed.

## Files

| Area | Change |
|---|---|
| `src/git/` | repo, status, diff wrappers |
| `src/model/sections.rs` | magit section tree |
| `src/ui/magit_status.rs` | status buffer |
| `src/ui/diff_view.rs` | shared diff renderer |
| `src/app/commands.rs` | `C-x g`, stage/unstage, fold, visit, refresh |

## Steps

1. Repo wrapper: status (staged/unstaged/untracked), branch, dirty counts.
2. Status section tree + magit buffer rendering (collapsible sections).
3. Diff renderer: per-file and per-hunk, theme-colored.
4. Stage/unstage file (`s`/`u`); hunk-level `s`/`u`.
5. `TAB` fold/unfold; `RET` visits the file in FileView; `g` manual refresh.
6. Watcher-driven auto-refresh; status line dirty counts go live.

## Verification

- With mixed staged/unstaged/untracked changes: stage and unstage files
  and individual hunks; resulting index matches `git status` and
  `git diff --cached` exactly (asserted via git CLI in tests).
- Diffs render correctly for adds/modifications/deletes/renames; sections
  fold and unfold.
- Agent edits on disk appear in status without a keypress.
- `RET` on a file opens it in FileView at the hunk.

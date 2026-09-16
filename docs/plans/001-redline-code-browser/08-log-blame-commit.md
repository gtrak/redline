# 08 — Log, blame, commit, branches

Phase 4 · Git · Depends on: 07

## Objective

Finish the magit v1 loop: log browsing, blame, inline commit with an
editable message buffer, branch switching, stash.

## Key decisions

- **Commit editor** is the first real editable buffer (ropey + explicit
  save path); magit bindings `C-c C-c` commit, `C-c C-k` abort.
- **Log v1**: one-line entries, `RET` opens a read-only commit diff; no
  graph rendering yet.
- **Push/pull optional** here: attempted via git2 + ssh-agent credentials;
  deferred to 09 if auth proves brittle.

## Files

| Area | Change |
|---|---|
| `src/git/` | log, blame, commit, refs wrappers |
| `src/ui/log_view.rs` | log buffer (commit list + diff on `RET`) |
| `src/ui/commit_editor.rs` | inline message buffer |
| `src/ui/blame_view.rs` | blame buffer |
| `src/ui/picker sources` | branch picker; stash list |
| `src/app/commands.rs` | `l`, `b`, `c`, `y`, `z` (+ `P`/`F` if push/pull lands) |

## Steps

1. Log view (branch-aware, paging); `RET` opens the commit diff read-only.
2. Blame buffer for the current file (`b`): per-line hash/author/age.
3. Commit flow: stage → `c` → inline message editor → `C-c C-c` commits
   staged changes → log/status update.
4. Branch picker (`y`): checkout, create from HEAD.
5. Stash (`z`): list, pop, drop.
6. (Optional) push/pull with ssh-agent auth; defer to 09 if brittle.

## Verification

- Complete loop in-pane: dirty tree → stage hunks → commit with inline
  message → commit visible in log; status goes clean.
- Aborted commit (`C-c C-k`) leaves repo state untouched.
- Blame output matches `git blame` on sampled files.
- Branch switch updates the status line and triggers an index refresh;
  stash pop restores changes.

# Task: Implement issue 08 — Log, blame, commit, branches (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained: read it, then execute it in order. The library references in
`.agents/skills/` are authoritative ground truth.

Context: issues 01 (app skeleton), 02 (project/buffers), 03 (syntax/ropey/
virtualized views), 04 (watcher/bus), 05 (Xref/jump stack/which-function),
06 is being built in parallel — you may READ but NOT rely on it (do not
reference `src/search/`), and 07 (magit status/staging, `src/git/` wrappers,
section tree, diff renderer) is implemented, reviewed, and committed. Build
on 07's `src/git/` wrapper surface and UI patterns.

## Working agreement (overrides any caution)

- **Skills are truth.** Work straight from `.agents/skills/*.md`. Do NOT read
  dependency sources under `~/.cargo/registry`, do NOT browse docs.rs, do NOT
  fetch anything. If a skill lacks an API detail you need, write the most
  reasonable call consistent with the skill and keep moving.
- **Write-first.** Create all new modules in your first handful of tool calls,
  then run `cargo build` early and iterate on specific errors. No front-loaded
  research.
- **Skill corrections.** If running code (compiler errors, runtime behavior)
  contradicts a skill file: code reality wins for the implementation, AND you
  make a minimal, factual correction to the relevant skill file so future
  agents are not misled. Never rewrite a skill file wholesale. List every
  skill-file edit in your report under "skill corrections".
- **graft is available** (CLI on PATH, graph in `graft/`, kept fresh by the
  orchestrator). Use it for cross-file orientation instead of grepping
  blind: `graft map` (repo orientation), `graft ask "<query>"` (ranked
  file:line), `graft callers <symbol>` / `--direction out` (who calls / what
  it calls), `graft skeleton <file>` (API surface), `graft grep <pattern>`
  (symbol-grouped search). Optional — use it when it answers faster than
  grep; never let it replace the read-first list.

## Read first (in this order)

1. `docs/plans/001-redline-code-browser/PLAN.md` — esp. decision #5 (magit
   buffers are section trees, plain data, snapshot-testable) and decision #6
   (light editing bounded: commit messages, notes, explicitly-opened files;
   ropey editable flag; no undo in v1).
2. `docs/plans/001-redline-code-browser/08-log-blame-commit.md` — THIS issue:
   objective, key decisions, files table, 6 steps, verification.
3. `.agents/skills/git2/SKILL.md` — log/revwalk, blame, commit, refs API
   (authoritative; mind the rename/status traps documented there).
4. `.agents/skills/magit/SKILL.md` — magit key conventions (`l` log, `b`
   blame/branch, `c` commit, `y` checkout, `z` stash) and buffer semantics.
5. `.agents/skills/iocraft/SKILL.md` — established UI patterns (canvas views,
   TextInput for the editable buffer — read how the skill's TextInput works
   before designing the commit editor).
6. `.agents/skills/support-crates/SKILL.md` — tempfile + git CLI for test
   repos, insta snapshots for section trees.
7. Existing code: `src/git/` (07's wrapper patterns, error enum), `src/ui/`
   (magit_status.rs, diff_view.rs — reuse the diff renderer for commit
   diffs), `src/app/store.rs` (view stack, keymaps, status line).

## Constraints

- Do not add, remove, or bump any dependency in `Cargo.toml`.
- git2 types never leave `src/git/` (07's rule). Plain structs + the existing
  `GitError` pattern. Commit-time author identity: read from git config
  (user.name/user.email via the repo/attempt global config through git2's
  documented config API per the skill); if unset, fail with a clear
  `GitError` (tests set config explicitly).
- The commit editor is the first editable buffer: ropey-backed, explicit
  save/commit path (`C-c C-c` commit, `C-c C-k` abort). NO general editing
  of files — the editor is bounded to the commit-message buffer only (plan
  decision #6). No undo.
- Log v1: one-line entries (short-hash, subject, relative date, author),
  paged; `RET` opens a read-only commit diff (reuse the diff renderer; show
  the full tree diff of that commit, plus stats if cheap). No graph
  rendering in v1.
- Push/pull (step 6) is OPTIONAL: attempt only if straightforward with
  git2's documented credential-helper/ssh-agent support; if auth proves
  brittle in tests, DEFER it explicitly to 09 and say so in the report (the
  plan sanctions this). Do not burn budget on network auth.
- Branch checkout must trigger the established refresh path (04's bus or a
  direct refresh — your choice, but the status line and magit buffer must
  reflect the new branch + clean/dirty state; the symbol index refresh path
  from 05 must fire for checkout wholesale changes — use the existing
  `start_indexing`/full-rebuild path on checkout).

## What to build

- `src/git/` — wrappers: `log` (branch-aware revwalk, paging by offset/limit,
  plain `LogEntry` {short_id, subject, author, time}), `commit_diff` (full
  tree diff of a commit as the existing plain diff structs),
  `blame` (per-line {commit short_id, author, time, line} via git2 blame),
  `commit` (stage is already index-based from 07; commit takes message +
  author, updates HEAD), `branches` (list with current marker, checkout,
  create-from-HEAD), `stash` (list {index, subject}, pop, drop).
- `src/ui/log_view.rs` — log buffer view: commit list, `n`/`p` paging,
  `RET` → read-only commit diff view (reuse diff renderer + section-tree
  patterns where sensible).
- `src/ui/commit_editor.rs` — inline commit message buffer: TextInput-style
  editing over a ropey buffer, magit bindings `C-c C-c` (commit staged) /
  `C-c C-k` (abort, discard buffer, touch nothing). Editor opens pre-filled
  with a magit-style summary comment block (staged file list as comments).
- `src/ui/blame_view.rs` — blame buffer for the current file: per-line
  hash/author/age prefix, aligned.
- Branch picker (`y`) + stash list (`z`) as Picker sources over the git
  wrappers; stash pop/drop from the picker.
- `src/app/` — commands + bindings (magit-status context keys per the magit
  skill): `l` log, `b` blame current file, `c` commit flow, `y` branch
  picker, `z` stash. Wire into the existing keymap/registry patterns; status
  line shows the current branch (07 shows it already — keep it fresh after
  checkout).

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green (baseline 227).
- Git integration tests (tempdir repos + git CLI cross-checks per the
  established 07 pattern; hermetic env):
  - log paging matches `git log --oneline` (order, count, subjects) for a
    repo with 25+ commits; paging windows are exact and stable;
  - commit_diff of a known commit matches `git show --stat`/`git diff
    <hash>^ <hash>` content assertions;
  - blame matches `git blame --porcelain` on a sampled file (line →
    short-hash mapping equal on every line);
  - commit flow: stage (via the existing 07 wrappers) → commit → `git log`
    shows the commit with the exact message; HEAD moved; author from test
    config; status goes clean (07's status wrapper);
  - abort path: open editor, `C-c C-k` → repo untouched (HEAD unchanged,
    index unchanged — assert via git CLI);
  - branch checkout: dirty-tree guard behavior you choose must be documented
    (git checkout refuses or stashes — magit default refuses; pick one and
    test it); clean checkout updates HEAD, status, and triggers the symbol
    index full-rebuild path (assert the rebuild scheduled/executed);
  - branch create-from-HEAD matches `git branch` list;
  - stash: pop restores the working changes (assert via git CLI), drop
    removes the entry; empty stash is a clean no-op.
- Snapshot tests: log-entry and blame-row rendering via insta (sorted/
  deterministic — remember parallel output must be sorted before
  snapshotting).
- Commit editor tests: pre-filled comment block, text edits land in the
  buffer, `C-c C-c` path extracts the message (comment lines stripped),
  `C-c C-k` discards; both keybindings resolve through the keymap engine
  (prefix `C-c` pending state included).
- Declare in your report exactly which behaviors are covered by tests and
  which are untested.

## Report format

- **Module map**: file + one-line purpose.
- **Design**: log paging model, blame wrapper, commit flow (editor → extract
  → commit), branch/stash surfaces (short).
- **Verification**: exact commands run + pass/fail + test counts; the git-CLI
  cross-checks asserted.
- **Skill corrections**: every skill-file edit (or "none").
- **Deviations** from the issue and why (incl. the push/pull decision);
  known gaps.

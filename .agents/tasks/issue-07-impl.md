# Task: Implement issue 07 — Git status & staging (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained: read it, then execute it in order. The library references in
`.agents/skills/` are authoritative ground truth.

Context: issue 01 (app skeleton) and issue 02 (project layer, buffers,
picker) are implemented, reviewed, and committed. Issue 03 (FileView) and 04
(watcher) are NOT yet implemented — see "Deferred" below.

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

## Read first (in this order)

1. `docs/plans/001-redline-code-browser/PLAN.md` — esp. decision #5 (git2 for
   the magit subset; magit buffers are section trees, plain data models
   rendered by generic components → snapshot-testable).
2. `docs/plans/001-redline-code-browser/07-git-status-and-staging.md` — THIS
   issue: objective, key decisions, files table, 6 steps, verification.
3. `.agents/skills/git2/SKILL.md` — libgit2 API reference (authoritative).
4. `.agents/skills/magit/SKILL.md` — the magit UX contract (status buffer,
   sections, stage/unstage keys).
5. `.agents/skills/support-crates/SKILL.md` — insta snapshots for section
   trees, tempfile for test repos, thiserror for the git error enum.
6. `.agents/skills/iocraft/SKILL.md` — UI conventions established in 01/02
   (read the existing `src/ui/` code and follow its patterns).

## Constraints

- Do not add, remove, or bump any dependency in `Cargo.toml` (git2, insta,
  tempfile are pinned).
- **git2 types never leak into the UI or models**: `src/git/` wraps libgit2
  and returns plain structs; `src/model/sections.rs` is a plain data section
  tree; `src/ui/` renders it. Errors: a `thiserror` enum in `src/git/`,
  surfaced through the existing app error path.
- No LSP, no tree-sitter, no watcher in this issue. No commit/branch logic
  (that is issue 08) — staging only.

## Deferred (do NOT implement, leave seams)

- Step 6 of the issue (watcher-driven auto-refresh): instead expose a public
  `refresh()` on the status model + command (`g` = manual refresh), and leave
  a one-line TODO where the watcher bus (issue 04) will subscribe.
- `RET` visiting a hunk inside FileView: FileView arrives with issue 03. Make
  `RET` on a file open it through the existing buffer model (issue 02) at the
  file level; leave a TODO for hunk-offset visiting.

## What to build

- `src/git/` — repo wrapper: open/discover from the project root; status
  (staged / unstaged / untracked, renames included), current branch, dirty
  counts; per-file diff (unified) for staged and unstaged variants.
- `src/model/sections.rs` — magit section tree as plain data: nested sections
  (Staged / Unstaged / Untracked → file → hunks), fold state, cursor position;
  serializable enough for insta snapshots.
- `src/ui/magit_status.rs` — the status buffer view: renders the section tree
  with collapsible sections (`TAB` fold/unfold), theme-colored diff rendering
  shared with `src/ui/diff_view.rs` (adds/deletes/context/diff-header faces).
- `src/app/` — register commands: open magit status (`C-x g`), stage file
  (`s`), unstage file (`u`), stage/unstage hunk at point, fold/unfold, visit
  file (`RET`), manual refresh (`g`). Status line gains live dirty counts.
- **Hunk staging** = apply the hunk's patch to the index via git2 (build the
  patch from the diff, apply to index). Every staging path must be exact.

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green.
- **Git integration tests**: build scratch repos with `tempfile` +
  `std::process::Command` git CLI (per support-crates skill — keep TempDir
  alive; set author/env so commits work), create mixed staged/unstaged/
  untracked/renamed states, then:
  - status matches `git status --porcelain` output exactly;
  - stage file → `git diff --cached` matches the wrapper's staged diff;
  - unstage → index matches HEAD again;
  - **stage individual hunks** → resulting index verified against
    `git diff --cached` (the issue requires this exactness — construct
    multi-hunk files, stage middle hunks, assert the index contains only
    those hunks);
  - renames appear as renames with the right old/new paths.
- **Snapshot tests**: section trees for representative repo states via
  `insta::assert_snapshot!` (or `assert_debug_snapshot!`) per the
  support-crates skill — sortable deterministic output (note: git2 iteration
  order is not sorted; sort before snapshotting).
- Fold/cursor logic unit tests on the section tree (no git needed).
- Declare in your report exactly which behaviors are covered by tests and
  which are untested (e.g. live in-TUI folding interaction).

## Report format

- **Module map**: file + one-line purpose.
- **Design**: git wrapper surface, section-tree shape, hunk-patch staging
  flow (short).
- **Verification**: exact commands run + pass/fail + test counts; list the
  git-CLI cross-checks you asserted.
- **Skill corrections**: every skill-file edit (or "none").
- **Deviations** from the issue and why; known gaps.

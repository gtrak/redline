# Task: Implement plan 002 issue 01 — Magit depth: transients, full verbs, inline diffs (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained: read it, then execute it in order. The library references in
`.agents/skills/` are authoritative ground truth.

Context: plan 001 (issues 01–09) is implemented, reviewed, and archived. The
app is a working TUI (310 tests). This issue opens plan 002 from field
feedback: **"magit isn't fleshed out yet — magit has a hydra tree, I want the
full functionality."** The magit interaction model is its transient menu
tree; implement it plus the missing verbs.

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
- **graft is available** (CLI on PATH, graph in `graft/`): `graft map`,
  `graft ask "<query>"`, `graft callers <symbol>`, `graft skeleton <file>` —
  for cross-file orientation; never a replacement for the read-first list.

## Read first (in this order)

1. `.agents/plans/002-magit-depth-and-cursor/PLAN.md` — plan context.
2. `.agents/plans/002-magit-depth-and-cursor/01-magit-depth.md` — THIS issue
   (the contract; note the transient directive).
3. `.agents/skills/magit/SKILL.md` — transients (§ Transients: menu at the
   bottom of the frame, infix/suffix listing), the FULL verb table (`h`
   dispatch, `k` discard, `c`/`b`/`z`/`l`/`d` transients), discard semantics
   (destructive; confirmation), section cursor semantics.
4. Existing code: `src/app/store.rs` (keymap seeds, magit commands,
   dispatch_key + pending), `src/app/keymap.rs` (engine trie, resolve,
   keys_display), `src/app/command.rs` (registry with name/docs/category),
   `src/ui/magit_status.rs` + `rows_view.rs` (renderer), `src/ui/root.rs`
   (render arms, snapshot, tick).
5. `.agents/skills/iocraft/SKILL.md` — rendering patterns (canvas views).

## Constraints

- Do not add, remove, or bump any dependency in `Cargo.toml`.
- The transient menu is a **renderer over existing state** — the store owns
  menu state (open/closed, current submenu path) exactly like picker state.
  No iocraft in the store.
- Menu source of truth: the ACTIVE VIEW's keymap bindings (view map +
  global) joined with registry metadata (docs, category). Do not hand-write
  menu tables — derive them, so menus never drift from bindings (the
  keymap-table test pattern from 09 is the precedent).
- Destructive verbs are confirmation-gated: `k` first press arms a
  confirmation in the minibuffer ("discard FILE? y/n" style), `y` executes,
  `n`/`C-g` cancels. Never discard without confirmation.
- Layering: menu rows/discard logic in the store (plain Rust); rendering in
  `src/ui/`.

## What to build

- **Transient menu system** (the hydra tree):
  - `?` (or `h` where free) in any view opens the menu: a bottom-of-frame
    overlay listing the view's bindings as `[key] description`, grouped by
    registry category, sorted within groups. Prefix keys (sequences that are
    a strict prefix of longer bindings) render as `KEY …` and open their
    SUBMENU when pressed (the tree: `C-c p …` → projectile menu; `c` →
    commit menu per the magit skill; `l` → log menu; etc.).
  - While the menu is open: pressing a listed LEAF key executes it (closes
    the menu first, then dispatches — magit transient semantics); pressing a
    listed PREFIX key descends into its submenu; `C-g` closes; keys not in
    the menu are ignored (menu swallows them) except `C-g`.
  - The menu renders at the bottom of the frame (above the status line),
    columns like magit transients (key + description pairs), scrollable if
    taller than the frame.
- **Missing verbs** (bind + register with docs/category, per the magit skill
  table):
  - `k` magit-discard at file level AND hunk level (cursor-addressed):
    unstaged file → workdir restore; staged file → unstage + workdir
    restore; hunk → workdir hunk revert; untracked file → delete the file
    (check the magit skill's discard semantics; magit deletes untracked on
    discard). ALL confirm-gated. Implement via git2 in `src/git/`
    (workdir discard = checkout pathspec from HEAD / index for the
    unstage+restore case — follow the skill's recipe; hunk workdir revert
    mirrors `revert_hunk_in_content` patterns already in repo.rs).
  - `h` in magit views → the top-level dispatch menu (same component).
- **Inline hunks in the status buffer** (the issue's existing scope): staged/
  unstaged hunks render directly under their file rows (fold state per file,
  TAB toggles), cursor-addressed like magit — reuse the diff renderer rows.

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green (baseline 310).
- Menu tests: menu opens with the view's bindings derived from the keymap ×
  registry (assert a derived row set matches the seeded bindings — the
  anti-drift property); prefix descent (open C-c p submenu, assert its
  entries); leaf execution dispatches the right command; C-g closes;
  non-menu keys swallowed while open.
- Discard tests (tempdir repos + git CLI cross-checks per the established
  pattern): discard unstaged file (workdir restored to HEAD/HEAD-equivalent),
  discard staged file (index + workdir), discard hunk (only that hunk
  reverts), discard untracked (file deleted), confirmation gate (n/C-g
  changes nothing), cursor-addressing (discard acts on the row under the
  cursor).
- Inline hunk tests: section tree contains hunk rows per file; fold toggles
  remove/restore them; cursor moves across them; RET visits.
- Sized-PTY checks (mandatory — see docs/ux-testing-plan.md + the
  subagent-orchestration skill's PTY recipe): open the menu (frame shows the
  grouped columns), descend a submenu, execute a leaf from the menu, discard
  flow with confirmation, inline hunks visible in the status buffer, cursor
  bar obvious.
- Declare in your report exactly which behaviors are covered by tests, which
  by PTY checks, which remain manual.

## Report format

- **Module map**: file + one-line purpose (new + modified).
- **Design**: menu state machine, derivation from keymap×registry, discard
  paths (short).
- **Verification**: exact commands + pass/fail + counts; PTY checks list.
- **Skill corrections**: every skill-file edit (or "none").
- **Deviations**; known gaps.

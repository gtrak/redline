# Task: Implement issue 02 — Project layer & file picker (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained: read it, then execute it in order. The library references in
`.agents/skills/` are authoritative ground truth.

Issue 01 (app skeleton: store, command registry, keymap engine, picker
component, M-x palette, config) is implemented, reviewed, and committed.
This issue builds the browse layer on top of it.

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

1. `docs/plans/001-redline-code-browser/PLAN.md` — the product; note the 8
   architectural decisions (esp. #3: one helm-style Picker; #8: everything
   embedded, cache in `~/.cache/redline/`).
2. `docs/plans/001-redline-code-browser/02-project-and-file-picker.md` — THIS
   issue: objective, key decisions, files table, 6 steps, verification.
3. `.agents/skills/iocraft/SKILL.md` — UI library (authoritative).
4. `.agents/skills/support-crates/SKILL.md` — serde/serde_json for persistence,
   toml config, dirs paths, tempfile for test repos.
5. `.agents/skills/nucleo/SKILL.md` — fuzzy matcher (the picker already uses
   it; reuse the established pattern from issue 01).
6. `.agents/skills/ropey/SKILL.md` — buffer text model (buffer model here is
   plain text; ropey arrives with 03, but note where it will slot in).
7. `.agents/skills/helm-ux/SKILL.md` and `.agents/skills/projectile-ux/SKILL.md`
   — the UX contracts this issue implements.

## Carry-over from the issue 01 review (do these first, they are small)

1. `src/ui/root.rs` `to_app_key` currently maps ~30 unmapped iocraft key codes
   to `AppKeyCode::Space` (fabricated keypresses). Return `None` for unmapped
   codes and drop the event instead.
2. The Picker needs **query editing** for real file finding: Backspace edits
   the query (and `C-h` if you bind it); unbound-key echo stays suppressed
   while the picker is open. Implement in the store's picker state + tests.

## Constraints

- Do not add, remove, or bump any dependency in `Cargo.toml`. `ignore`,
  `serde`, `serde_json`, `dirs`, `tempfile` are already pinned.
- Layering: `src/model/` is plain Rust (zero iocraft/tokio/crossterm). Picker
  sources and views live in `src/ui/`. New commands register in the existing
  `src/app/command.rs` registry; bindings go through the existing keymap
  engine + config overrides.
- No tree-sitter (preview is plain text — highlighting is issue 03). No
  watcher (refresh is manual via a re-walk command; auto-refresh is 04). No
  git operations beyond root detection (git lane is issues 07/08).

## What to build

- `src/model/project.rs` — root detection (innermost git root by scanning
  upward for `.git`; else marker files: `Cargo.toml`, `package.json`,
  `pyproject.toml`, `.projectile`, …), project identity (canonical root path),
  and a persisted project registry.
- `src/model/files.rs` — cached file list via an `ignore`-crate walk
  (respects `.gitignore`, skips hidden/ignored by default); manual re-walk
  command invalidates the cache.
- `src/model/buffer.rs` — open-buffer set + current buffer; plain-text content
  for now; kill removes from the set.
- `src/ui/` — picker sources for files (find-file), buffers (switch-buffer),
  projects (switch-project); buffer-list view; preview shows the selected
  file's first page as plain text.
- `src/app/` — register commands: find-file, switch-buffer, list-buffers,
  kill-buffer, switch-project, recent-files, re-walk files. Wire default
  bindings per the issue's step 6: `C-x C-f`, `C-x b`, `C-x C-b`, `C-x k`,
  `C-c p p`, `C-c p f`. Status line shows the project name (replaces the
  placeholder).
- Persistence under `dirs::cache_dir()/redline/`: per-project recents + the
  project registry (serde_json), surviving restarts.

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green.
- Unit tests (use `tempfile::tempdir()` for fake projects per the
  support-crates skill; keep TempDir alive through each test):
  root detection (git root wins, marker fallback, no-project case), walk
  respects `.gitignore` (create ignored paths, assert absence), walk cache +
  invalidation, buffer open/switch/kill, recents + registry persistence
  roundtrip (serde_json to a tempdir — inject the base dir, do NOT hardcode
  the real cache path in tests), keymap bindings resolve for every new
  sequence (including the `C-c p …` prefix path).
- Static-render tests via `element!(...).to_string()` for the buffer-list
  view and picker-with-preview, following the issue 01 test patterns.
- You cannot test TUI interactivity: after building, declare in your report
  exactly which behaviors are covered by tests and which are untested.
- Perf sanity: the walk + filter design must plausibly handle a 10k+-file
  repo with no perceptible lag on `C-x C-f` (walk is cached; filter is nucleo
  over the cached list). State your reasoning; no benchmark theater.

## Report format

- **Module map**: file + one-line purpose.
- **Design**: project identity, walk caching, picker source flow, persistence
  layout (short).
- **Verification**: exact commands run + pass/fail + test counts.
- **Skill corrections**: every skill-file edit (or "none").
- **Deviations** from the issue and why; known gaps.

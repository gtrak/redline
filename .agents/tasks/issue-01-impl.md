# Task: Implement issue 01 — App skeleton & command system (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained: read it, then execute it in order. The library references in
`.agents/skills/` are authoritative ground truth.

## Working agreement (overrides any caution)

- **Skills are truth.** Work straight from `.agents/skills/*.md`. Do NOT read
  dependency sources under `~/.cargo/registry`, do NOT browse docs.rs, do NOT
  fetch anything. If a skill lacks an API detail you need, write the most
  reasonable call consistent with the skill and keep moving.
- **Write-first.** Scaffold ALL modules (`src/main.rs`, `src/app/*`, `src/ui/*`,
  `src/theme.rs`) within your first handful of tool calls, then run
  `cargo build` early and iterate on specific errors. No front-loaded research.
- **Skill corrections.** If running code (compiler errors, runtime behavior)
  contradicts a skill file: code reality wins for the implementation, AND you
  make a minimal, factual correction to the relevant skill file so future
  agents are not misled. Never rewrite a skill file wholesale. List every
  skill-file edit in your report under "skill corrections".

## Read first (in this order)

1. `docs/plans/001-redline-code-browser/PLAN.md` — the product; note the 8
   architectural decisions.
2. `docs/plans/001-redline-code-browser/01-app-skeleton.md` — THIS issue:
   objective, key decisions, files table, 7 steps, verification checklist.
3. `.agents/skills/iocraft/SKILL.md` — UI library (iocraft 0.9): `element!`
   syntax, hooks, `use_terminal_events`, `SystemContext`,
   `mock_terminal_render_loop` testing.
4. `.agents/skills/tokio/SKILL.md` — async runtime patterns.
5. `.agents/skills/support-crates/SKILL.md` — serde/toml/anyhow/thiserror/dirs.
6. `.agents/skills/nucleo/SKILL.md` — fuzzy matcher for the picker.

## Constraints

- Do not add, remove, or bump any dependency in `Cargo.toml`. Everything needed
  is already pinned and pre-compiled.
- All iocraft imports live in `src/ui/` only (`src/main.rs` may import iocraft
  only to start/stop the render loop). `src/app/` and `src/theme.rs` are plain
  Rust with zero iocraft/tokio dependencies so they are unit-testable.
- Scope: issue 01 only. No file tree, no git, no tree-sitter, no watcher.

## What to build

- `src/main.rs` — entry: load config, init tracing to a log file (e.g.
  `dirs::cache_dir()/redline/redline.log`, create parent dirs), run the iocraft
  fullscreen render loop on the tokio runtime, restore the terminal cleanly on
  every exit path including panics (per the iocraft skill; install a panic hook
  that restores + logs).
- `src/app/` — plain Rust, zero iocraft imports:
  - `store.rs`: central `AppStore` — view stack, minibuffer message state,
    status line state (project placeholder, current view name, pending key
    sequence, async-activity indicator slots), quit flag.
  - `command.rs`: command registry — every interactive action is a named
    command with name, one-line docs, category, and a handler.
    Dispatch-by-name API. Seed ~10 placeholder commands (quit, cancel, open
    palette, cycle views, echo demo messages, etc.).
  - `keymap.rs`: emacs-style keymap engine. Define your own `Key` type
    (modifiers + key enum) inside `app/` — the ui layer converts crossterm
    events to it. A key sequence is a `Vec<Key>`. Include a parser for emacs
    notation strings: `"C-x C-f"`, `"M-x"`, `"RET"`, `"SPC"`, `"TAB"`, `"C-g"`
    (required because config overrides are TOML strings). Global keymap +
    per-view keymaps where per-view beats global on conflict. Prefix sequences
    supported: when input is a strict prefix of some binding, record pending
    keys in the store so the status line shows them; more keys extend the
    sequence; `C-g` or an unmatchable key cancels pending. A full match
    dispatches the bound command.
  - `config.rs`: serde `Deserialize` config for
    `~/.config/redline/config.toml`: theme selection + key binding overrides
    (command name -> sequence string). Missing file = all defaults; tolerate
    unknown fields; unit-test parsing with inline TOML strings.
- `src/ui/` — ALL iocraft code lives here and only here:
  - root component: renders the top-of-stack view, the minibuffer line, the
    status line, and the picker overlay when open. Converts crossterm key
    events (`use_terminal_events`) into app `Key` presses fed to the keymap
    engine against the store.
  - picker component: prompt line, nucleo-matcher fuzzy filter over candidates,
    scrollable candidate list, preview stub pane. First consumer: the `M-x`
    palette whose candidates are the command registry (name + docs). Type to
    filter, arrows / `C-p` / `C-n` to move, `RET` runs, `C-g` cancels.
  - a placeholder scratch view so the app shows something.
- `src/theme.rs` — stub color/face definitions consumed by ui components.

## Verification (iterate until ALL pass)

- `cargo build` — clean.
- `cargo clippy --all-targets -- -D warnings` — clean.
- `cargo test` — passes, including unit tests for: keymap resolution (per-view
  override beats global; prefix sequences go pending then match; pending
  cancel), the sequence-string parser, registry dispatch-by-name, config
  parsing + defaults + override application.
- Behavioral targets: `M-x` opens palette and typing filters (nucleo), `RET`
  runs a placeholder, `C-g` cancels, `q` / `C-x C-c` quit and restore the
  terminal; unknown keys echo in the minibuffer; pending prefixes show in the
  status line. You cannot test the TUI interactively — use iocraft
  `mock_terminal_render_loop` (skill Testing section) for at least one
  root-component keypress test if practical; otherwise state clearly in your
  report which behaviors are untested.
- Keep modules small and focused; comment only non-obvious design.

## Report format

- **Module map**: file + one-line purpose.
- **Design**: how keymap/registry/dispatch work (short).
- **Verification**: exact commands run + pass/fail.
- **Skill corrections**: every skill-file edit (or "none").
- **Deviations** from the issue and why; known gaps.

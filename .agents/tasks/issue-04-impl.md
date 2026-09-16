# Task: Implement issue 04 — File watching (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained: read it, then execute it in order. The library references in
`.agents/skills/` are authoritative ground truth.

Context: issues 01 (app skeleton), 02 (project layer, buffers), 03 (syntax,
ropey buffers, virtualized FileView, highlight cache) and 07 (git status with
its `refresh()` seam) are implemented, reviewed, and committed. This issue
makes the repo live. Issues 05 (symbol index) and 08 are NOT implemented —
subscription stubs only, per the plan.

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

1. `docs/plans/001-redline-code-browser/PLAN.md` — esp. decision #7 (watching
   is first-class; keep scroll anchors; publish project-change events that
   git status and the symbol index subscribe to) and the tokio skill's
   project-change guidance (watch channel = latest value wins).
2. `docs/plans/001-redline-code-browser/04-file-watching.md` — THIS issue:
   objective, key decisions, files table, 5 steps, verification.
3. `.agents/skills/notify/SKILL.md` — notify + notify-debouncer-full API
   (authoritative; mind the version family: notify 8.2 / debouncer 0.7).
4. `.agents/skills/tokio/SKILL.md` — channels: watch for the change bus
   (many subscribers, latest-value), mpsc if ordering matters; select!
   cancellation-safety rules for the UI poll loop.
5. `.agents/skills/iocraft/SKILL.md` — follow the established src/ui patterns
   (canvas views, store-driven rendering, key handling in root).
6. `.agents/skills/support-crates/SKILL.md` — tempfile for watch tests.

## Constraints

- Do not add, remove, or bump any dependency in `Cargo.toml` (notify 8.2 +
  notify-debouncer-full 0.7 are pinned; tokio has sync/time/fs features).
- Layering: the watcher + event bus live in `src/app/` as plain Rust (they may
  use tokio + notify + std; NO iocraft). `src/ui/` only reacts to store state
  set from the bus. The watcher thread/task posts into the store via the
  established pattern (the store is behind a lock; follow how existing code
  crosses the async/UI boundary).
- Scroll-anchor preservation is a hard requirement: an auto-reload that bumps
  the reader off their line is a failed implementation (when the anchor line
  still exists; snap sensibly when it vanished).
- Locally-edited buffers: no editing exists in the UI yet (issues 01-03 are
  read-only), but the buffer model must already carry the `editable`/
  `locally-modified` flag path (plan decision #6) so the conflict logic is
  real, not a stub. Scratch buffers count as locally-owned.
- No symbol index or git-status wiring beyond subscription stubs (05/07/08
  own those).

## What to build

- `src/app/watcher.rs` — one debounced watcher per open project
  (notify-debouncer-full): watch the project root recursively, coalesce rapid
  event bursts (agents churn; the debounce window from the notify skill's
  guidance, ~300-500ms, is fine), map debounced events onto the change bus.
  Filter noise (ignore the tracing log file, cache files, `.git` internal
  churn unless the working tree status actually changes).
- `src/app/events.rs` — the project-change bus: a tokio watch channel (latest
  value wins) carrying a change summary (changed paths, kinds). Subscribers:
  FileView reload logic, git status `refresh()` (07's seam — wire it now),
  symbol index stub (05 will fill it). Document the subscription pattern in
  the module.
- `src/ui/file_view.rs` + store — auto-reload on change for non-edited
  buffers: reload content, re-highlight (existing cache invalidation by
  mtime), keep the scroll anchor (same line number if it still exists; else
  clamp and preserve relative position within reason). Conflict path:
  buffers flagged locally-edited get a "changed on disk" marker in the view
  and NO auto-reload; `g` forces reload (existing refresh command gains this
  role).
- Config: `auto_reload` toggle (default on) + a runtime suspend/resume
  command (M-x `toggle-watcher`); config-plumbing follows the existing
  config.rs patterns. Suspending stops consuming events (not just UI updates).
- Watcher lifecycle: starts on project open/switch (02's switch path),
  stops/replaces on project switch, stops cleanly at shutdown. Exactly one
  watcher per project at a time.

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green (baseline 169).
- Integration tests with `tempfile` + real notify events (write files in a
  temp project from the test, with the debouncer's real timing — keep
  debounce windows short in tests but nonzero; assert eventual delivery, not
  instant):
  - write to a viewed file → change event observed; reload path re-reads
    content (store-level test of the reload function on a changed file);
  - scroll anchor preserved: simulate a view at line N, file gains lines
    above/below N, assert anchor behavior per the rule;
  - locally-edited (flagged) buffer: change does NOT auto-reload; force
    reload via the `g` path does; marker state flips correctly;
  - coalescing: 10 rapid writes to the same file within one debounce window
    → 1 change event for that path (bounded, not 10);
  - event filter: writes to a `.git/` internal file or the log file do not
    produce project-change events;
  - suspend/resume: suspended → events buffered-or-dropped per design (state
    which) but no reloads happen; resume → watching works again;
  - project switch: old watcher stopped (no events from old root), new
    watcher active; exactly-one-watcher invariant (no leak per switch —
    assert via the watcher registry/handle count if exposed, else by
    observing no cross-project events).
- Store-level unit tests for: anchor math (pure function — extract it so it
  is testable without notify), conflict flag transitions, config toggle
  plumbing, bus subscribe/notify semantics (watch channel latest-wins).
- Declare in your report exactly which behaviors are covered by tests and
  which are untested (e.g. real-terminal visual refresh timing).

## Report format

- **Module map**: file + one-line purpose.
- **Design**: watcher lifecycle, bus shape, anchor algorithm, conflict flow
  (short).
- **Verification**: exact commands run + pass/fail + test counts.
- **Skill corrections**: every skill-file edit (or "none").
- **Deviations** from the issue and why; known gaps.

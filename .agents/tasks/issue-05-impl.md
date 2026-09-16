# Task: Implement issue 05 — Symbols & jump navigation (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained: read it, then execute it in order. The library references in
`.agents/skills/` are authoritative ground truth.

Context: issues 01 (app skeleton), 02 (project layer, cached file walk,
buffers), 03 (syntax: grammar registry, highlight pipeline, ropey buffers,
highlight cache), 04 (project-change bus + watcher with incremental-refresh
seams), and 07 (git) are implemented, reviewed, and committed. READ the
existing `src/syntax/` (registry + highlight pipeline), `src/app/events.rs`
(change bus), and `src/model/files.rs` (cached walk) — this issue builds
directly on all three.

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
- **Dependency guardrail (lesson from issue 03)**: grammar pins in Cargo.toml
  were corrected for ABI compatibility during issue 03 — do NOT bump any
  grammar or runtime version. If `set_language` rejects a grammar, check the
  tree-sitter skill's ABI notes and report; do not solve it by upgrading.

## Read first (in this order)

1. `docs/plans/001-redline-code-browser/PLAN.md` — esp. decision #4 (NO LSP in
   v1: definitions from a background, ignore-aware, rayon-parallel tree-sitter
   symbol index; navigation behind an `Xref` trait so LSP slots in later).
2. `docs/plans/001-redline-code-browser/05-symbols-and-jump.md` — THIS issue:
   objective, key decisions, files table, 8 steps, verification.
3. `.agents/skills/tree-sitter/SKILL.md` — queries, captures, ABI notes
   (rust 0.23.3 / md 0.3.2 pins are ABI-14; the runtime accepts max 14).
4. `.agents/skills/tokio/SKILL.md` — spawn_blocking for rayon fan-in,
   channels for progress, cancellation via watch/flags (no tokio-util).
5. `.agents/skills/support-crates/SKILL.md` — rayon: `par_bridge` is a trait
   method on the iterator (`use rayon::iter::ParallelBridge`), not a free
   function; sort before snapshotting unordered parallel output.
6. `.agents/skills/iocraft/SKILL.md` — established UI patterns.
7. Existing code: `src/syntax/registry.rs` (how configs are built),
   `src/app/events.rs` (bus), `src/model/files.rs` (walk cache),
   `src/app/store.rs` (picker state, status line, scroll state — follow the
   established shapes).

## Constraints

- Do not add, remove, or bump any dependency in `Cargo.toml` (rayon is
  already there; tree-sitter runtime + grammars are pinned ABI-14).
- Layering: `src/nav/` is plain Rust (tree-sitter + rayon + tokio allowed;
  zero iocraft). The indexer communicates with the app via the change bus /
  channels, never by holding locks the UI needs. UI reads only store state.
- The indexer must NEVER block the UI: parsing runs on background threads
  (rayon via spawn_blocking or a dedicated worker); the UI reads whatever
  index snapshot exists and shows a progress indicator while indexing.
- Symbol extraction is per-language tree-sitter QUERIES (definitions only:
  functions, methods, types/structs/enums/traits/interfaces, constants,
  macros) — not highlight captures. Queries live in `src/syntax/queries/`
  as embedded strings in the registry module (follow the existing registry
  pattern; all tree-sitter churn stays in `src/syntax/`).
- Ambiguity routes through the existing Picker; every navigation pushes the
  origin position onto the jump stack.

## What to build

- `src/syntax/queries.rs` (or extend the registry) — definition queries for
  the supported languages: Rust, TS/TSX, JS, Python, Go, C, C++, TOML (keys),
  JSON (keys), YAML (keys), Bash (functions), Markdown (headings). Languages
  where definitions are not meaningfully queryable may return empty outlines
  — but every language in the registry must have either a working query or a
  documented empty fallback.
- `src/nav/index.rs` — background indexer: builds a per-file symbol table
  (name, kind, line, byte range) over the whole project using the cached
  file walk + rayon parallel parse; in-memory per session; a status-line
  progress indicator while running; incremental refresh on watcher events
  (only changed files reparse — full rebuild only on project switch or
  manual command).
- `src/nav/xref.rs` — `Xref` trait (find_definition(symbol/position) ->
  location(s), outline(file), all_symbols()) + the tree-sitter backend
  implementing it. No LSP.
- `src/app/store.rs` — jump stack (position + buffer identity + label);
  navigation commands push origins; `M-,` pops and returns to the EXACT
  prior position (line + column), `C-i` walks forward again.
- `src/ui/` — symbol picker source (fuzzy over all project symbols, preview
  showing the definition line plus context); imenu as a nested/indented
  picker over the current file's outline; which-function in the status line
  (enclosing symbol for the cursor line, from the current file's outline).
- `src/app/` — commands + bindings: `M-.` (xref-find-definitions), `M-,`
  (pop), `C-i` (jump forward), `M-i` (imenu), symbol-picker command (bind
  per helm-ux/projectile-ux skill conventions, e.g. `C-c p s` style — check
  the skills' quick references and pick one documented binding).

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green (baseline 169).
- Query tests (embedded snippets per language, like issue 03): extract
  expected symbols with correct kind + line from synthetic Rust / TS /
  Python / Go / C snippets (fn + method + struct + const; interface + method;
  class + def; func + type; function + struct). Markdown headings outline.
  A language with an empty fallback documents it via its test.
- Indexer tests: parallel index over a tempdir project (a few files across
  languages) finds cross-file definitions; incremental refresh reparse-only-
  changed (mutate one file, assert the update touches only that file's
  entries — expose a counter or diff the snapshot); progress callback
  observed; no UI-blocking (indexer runs on threads; store reads a snapshot
  — assert the store is queryable mid-index).
- Xref tests: unique definition jumps directly; ambiguous name (same symbol
  defined in 2+ files) returns both candidates (Picker routing is store
  logic — assert the candidate list, not the UI); unknown symbol -> empty.
- Jump stack tests: push/pop exactness (line AND column), forward-walk
  semantics, stack bounds (bounded depth per emacs convention or document).
- which-function tests: enclosing-symbol lookup for cursor lines inside
  nested functions (method inside impl inside mod for Rust; method inside
  class for Python).
- Declare in your report exactly which behaviors are covered by tests and
  which are untested.
- Perf sanity, stated briefly: cross-file `M-.` on a large repo must be a
  lookup in the in-memory index (no reparse on jump); full index of a 10k
  file repo is background + parallel; incremental refresh is O(changed files).

## Report format

- **Module map**: file + one-line purpose.
- **Design**: query strategy per language family, indexer threading model,
  Xref trait surface, jump-stack semantics (short).
- **Verification**: exact commands run + pass/fail + test counts.
- **Skill corrections**: every skill-file edit (or "none").
- **Deviations** from the issue and why; known gaps.

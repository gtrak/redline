# Task: Implement issue 06 — Search & references (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained: read it, then execute it in order. The library references in
`.agents/skills/` are authoritative ground truth.

Context: issues 01 (app skeleton), 02 (project layer, cached walk, buffers,
picker), 03 (syntax: grammar registry with per-language configs, ropey
buffers, virtualized views), 04 (project-change bus, watcher), 05 (symbol
index behind the `Xref` trait, jump stack, which-function), and 07 (git) are
implemented, reviewed, and committed. READ the existing `src/nav/` (Xref,
jump stack), `src/model/files.rs` (walk), `src/syntax/` (grammar registry),
and `src/app/store.rs` patterns before designing.

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

1. `docs/plans/001-redline-code-browser/PLAN.md` — esp. decision #4's
   references approach (embedded ripgrep + tree-sitter comment/string
   filtering) and the streaming/cancelable requirements.
2. `docs/plans/001-redline-code-browser/06-search-and-references.md` — THIS
   issue: objective, key decisions, files table, 5 steps, verification.
3. `.agents/skills/ripgrep-crates/SKILL.md` — grep-searcher / grep-regex /
   ignore API (authoritative: searcher sink trait, regex builder, walk
   builder parallelism).
4. `.agents/skills/tree-sitter/SKILL.md` — token-class queries for
   comment/string filtering (ABI-14 pins; no bumps).
5. `.agents/skills/tokio/SKILL.md` — cancellation via watch/flags (no
   tokio-util), select! cancellation-safety, channels for streaming results.
6. `.agents/skills/iocraft/SKILL.md` — established UI patterns.
7. `.agents/skills/helm-ux/SKILL.md` — results/occur UX contracts.

## Constraints

- Do not add, remove, or bump any dependency in `Cargo.toml` (grep-searcher,
  grep-regex, ignore, tree-sitter family all pinned). No external `rg` or
  `git grep` binary — everything in-process (plan decision #8: no PATH
  dependence). The git CLI may be used ONLY in tests (established pattern).
- Layering: `src/search/` is plain Rust (grep crates + tree-sitter + tokio;
  zero iocraft). Results stream into the store via the established
  async->store pattern (04's bus precedent); the UI renders store state.
- Cancellation is a hard requirement: ESC (or starting a new search) must
  stop the running search promptly — a canceled search must not pin a thread
  walking a huge repo (cooperative checks between files via the pattern in
  the ripgrep-crates/tokio skills).
- Reference filtering drops comment/string token-class hits only where a
  grammar exists (reuse issue 05's query infrastructure for token classes);
  plain word-boundary search fallback elsewhere. Be honest in tests about
  which languages get filtering.
- Results view is a view (store-owned rows), grouped by file, with running
  counts and live updates while the search streams.

## What to build

- `src/search/rg.rs` — streaming, cancelable pipeline: regex pattern, path
  glob/type filters, sorted walk (reuse the ignore-crate walk), searcher
  per file; results (file, line, column, line text) streamed through a
  channel into the store; cancel flag checked between files.
- `src/search/references.rs` — `M-?` on symbol under point: word-boundary
  search (regex-escaped, `\b`-anchored) + token-class filter (drop
  comment/string hits) for languages with grammars; returns/reuses the
  results view. Cross-check with the symbol index where a definition exists
  is NOT required (plan keeps references search-based).
- `src/search/occur.rs` — per-buffer occurrences of a pattern: reuse the
  same pipeline scoped to the current buffer (in-memory scan or single-file
  searcher, whichever fits the established patterns).
- `src/ui/results_view.rs` — grouped results view (file headers with per-
  file counts, match lines with line numbers), running total, live updates;
  `n`/`p` navigate matches; `RET` jumps to the match (pushing onto the
  jump stack so `M-,` returns to the results — reuse issue 05's stack);
  `g` re-runs the search; `ESC`/`q` cancels/closes.
- `src/app/` — commands + bindings: project search (`C-c p s s` per the
  projectile-ux skill's quick reference), references `M-?`, occur
  `M-s o` (verify the prefix path works through the existing keymap engine;
  `M-s o` is a two-key sequence). Status line: running-search indicator
  (the async-activity slot from issue 01 finally gets a real occupant).

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green (baseline 193).
- Pipeline tests against a tempdir project with known content (multiple
  files, nested dirs, a `.gitignore`d file that must NOT appear):
  - counts match a reference computation for the same query (the issue says
    "counts match the rg CLI" — the worker may not invoke rg; compute the
    expected counts by hand in the test and assert equality);
  - glob/type filters exclude correctly; ignored paths excluded;
  - streaming: first results observable before completion (channel +
    partial-store assertions with the searcher running);
  - cancellation: start a search over many files, cancel mid-flight, assert
    prompt stop (bounded wait) and a consistent store state;
  - multibyte-safe columns/line text (lesson from issue 03).
- Reference filtering tests: for a language with a grammar (e.g. Rust),
  place the same identifier in code, comment, and string; assert comment/
  string hits are dropped; for a fallback case, assert they are kept.
- Results view tests: grouping, counts, n/p cursor movement, RET jump
  target correctness (jump-stack entry recorded: exact line+column), `g`
  re-run, cancel/close paths.
- Occur tests: all in-file matches with per-file count.
- Declare in your report exactly which behaviors are covered by tests and
  which are untested.
- Perf sanity, stated briefly: first hits must appear before the walk
  finishes (streaming); cancellation latency bounded; per-file work is
  O(file).

## Report format

- **Module map**: file + one-line purpose.
- **Design**: pipeline threading/cancellation model, filtering approach,
  results-view store flow (short).
- **Verification**: exact commands run + pass/fail + test counts.
- **Skill corrections**: every skill-file edit (or "none").
- **Deviations** from the issue and why; known gaps.

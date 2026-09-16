# Task: Implement issue 03 — Syntax & file view (Redline)

You are the implementation worker. Repo root is your cwd. This spec is
self-contained: read it, then execute it in order. The library references in
`.agents/skills/` are authoritative ground truth.

Context: issues 01 (app skeleton: store, registry, keymap, picker, config)
and 02 (project layer, cached file walk, buffer table, file preview, status
line) are implemented, reviewed, and committed. Read the existing
`src/model/buffer.rs` — this issue upgrades it.

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

1. `docs/plans/001-redline-code-browser/PLAN.md` — esp. decision #8 (11
   grammars embedded) and the tree-sitter version pins in `Cargo.toml`
   (DO NOT bump grammars — several latest releases need tree-sitter ^0.25+ and
   would fork the runtime; the pins are deliberate).
2. `docs/plans/001-redline-code-browser/03-syntax-and-file-view.md` — THIS
   issue: objective, key decisions, files table, 6 steps, verification.
3. `.agents/skills/tree-sitter/SKILL.md` — highlighting pipeline API
   (authoritative for `tree_sitter_highlight` usage, query files, config).
4. `.agents/skills/ropey/SKILL.md` — rope buffer API.
5. `.agents/skills/iocraft/SKILL.md` — UI conventions; follow the existing
   `src/ui/` patterns (canvas drawing established in 01/02).
6. `.agents/skills/support-crates/SKILL.md` — tempfile/insta if snapshotting.

## Constraints

- Do not add, remove, or bump any dependency in `Cargo.toml`. All grammars,
  `tree-sitter-highlight`, and `ropey` are already pinned and pre-compiled.
- Layering: `src/syntax/` is plain Rust (zero iocraft/tokio). The grammar
  registry is ONE module that pins all grammars + highlight queries — all
  tree-sitter API churn is isolated there. `src/ui/file_view.rs` consumes
  pre-highlighted lines (theme faces), it never touches tree-sitter types.
- Theme: extend the existing `src/theme.rs` face map (token category →
  color/style); config drives theme selection (existing config plumbing).
- Editing stays out: buffers are ropey-backed but the UI is read-only in this
  issue (no insert/delete commands).
- No watcher: cache invalidation on reopen only (04 adds live invalidation).
- No LSP; isearch is plain text.

## What to build

- `src/syntax/registry.rs` — grammar registry for the polyglot set: Rust,
  TypeScript/TSX, JavaScript, Python, Go, C, C++, TOML, JSON, YAML, Bash,
  Markdown + plain-text fallback. Map file extension → language (not
  exhaustive; unknown → fallback). Include highlight queries per the
  tree-sitter skill (each grammar crate ships its `highlights` query; TSX uses
  the typescript crate's tsx queries).
- `src/syntax/highlight.rs` — pipeline: ropey buffer + file extension →
  highlighted line spans (token category + byte range per line).
- `src/model/buffer.rs` — ropey-backed text buffer: load from disk, cheap
  line access, byte↔line conversion. Keep the open-buffer set/current-buffer
  API from 02 intact.
- Highlight cache — keyed by (path, mtime, theme); bounded memory.
- `src/ui/file_view.rs` — virtualized view: render only visible lines ±
  margin; smooth scroll (line/half-page/page), `g`/`G` goto top/bottom,
  `M-g g` goto-line, `M-<`/`M->`. Scroll state lives in the store (per
  buffer), so view switches preserve position (plan decision: keeping scroll
  anchor matters).
- `src/app/` — register motion commands + isearch: `C-s`/`C-r` incremental
  search with live match count in the minibuffer, wrap-around, `n`/`N`
  navigate, clean exit (`C-g`, `RET`); goto-line command. Wire default
  bindings through the existing keymap engine.
- Big-file fallback: files >~10MB render as plain text (no highlighting),
  never a hang.

## Verification (iterate until ALL pass)

- `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` all green.
- Highlighting tests: small sample snippets per language (embed as string
  constants — no fixture files needed) assert expected token categories on
  known positions (e.g. a Rust `fn` name is a function face; a TS interface
  name is a type face; a Bash comment is a comment face). Unknown extension →
  fallback path asserted.
- Ropey buffer tests: load, line count, line access, byte↔line roundtrips,
  large-file handling.
- Cache tests: (path, mtime, theme) key behavior — same key hits cache,
  changed mtime invalidates, theme switch invalidates; bounded size.
- Isearch tests: incremental query state, match counting with wrap, n/N
  navigation over a known buffer, clean exit restoring pre-search position.
- Scroll/virtualization tests: window slice math (visible ± margin) for small
  files, exact-boundary cases, and a 50k-line synthetic buffer (build in
  memory via ropey — do NOT write 50k-line fixture files); fallback path for
  a >10MB synthetic buffer.
- Static-render tests via `element!(...).to_string()` for the FileView
  (visible window + scroll indicators), following existing patterns.
- Declare in your report exactly which behaviors are covered by tests and
  which are untested (live scrolling feel, isearch typing UX).
- Perf sanity, stated in the report (no benchmark theater): highlight only
  the visible window? (justified per the tree-sitter skill — highlight
  cost model), cache bounds memory on large repos, 50k-line scroll is O(view)
  per frame not O(file).

## Report format

- **Module map**: file + one-line purpose.
- **Design**: registry shape, highlight pipeline flow, virtualization slice
  math, cache keying (short).
- **Verification**: exact commands run + pass/fail + test counts.
- **Skill corrections**: every skill-file edit (or "none").
- **Deviations** from the issue and why; known gaps.

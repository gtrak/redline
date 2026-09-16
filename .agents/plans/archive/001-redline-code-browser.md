# 001 — Redline: a read-focused TUI code browser (ARCHIVED)

Status: implemented (9/9 issues, ~44 review rounds total). Superseded by
field feedback → plan 002.

Redline is a single-binary Rust TUI code browser for the agent-coding loop:
instant navigation in any repo (files, symbols, definitions, references,
grep), a magit-subset git surface (status/stage/log/blame/commit/branches/
stash), live views (watcher-driven reloads keeping scroll anchors), and light
editing bounded to commit messages and notes. Built on iocraft (UI), tokio
(async), tree-sitter (13 embedded grammars, highlighting + symbol index),
embedded ripgrep crates (search), git2 (git), ropey (buffers).

## Scope delivered

- App chassis: central store, named command registry (73 commands), emacs-
  style keymap engine (per-view override, prefix sequences, pending display),
  minibuffer/status line, picker component (nucleo), M-x palette, config.
- Project layer: root detection, cached ignore-aware walk, buffers, recents
  + project registry persistence, file/buffer/project pickers with preview.
- Reading: ropey buffers, tree-sitter highlighting with bounded cache,
  virtualized file view, emacs motion, incremental isearch, >10MB fallback.
- Navigation: per-language definition queries, rayon-parallel background
  symbol index (single-flight, pending-coalescing, generation-tagged), Xref
  trait (LSP-ready), M-./M-,/C-i jump stack, imenu, which-function.
- Search: streaming cancelable in-process ripgrep pipeline, grouped results
  view wired to the jump stack, M-? references with token-class filtering,
  occur.
- Git: status/stage/unstage (file + hunk, byte-exact, git-CLI-verified),
  log/blame (workdir-coordinate)/commit (inline editor)/branches/stash.
- Live repo: debounced watcher, change bus, scroll-anchor reloads, conflict
  markers, silent incremental reindexing.
- Polish: tree sidebar, dark/light themes, notes buffer, mouse, perf numbers
  (cold start ~15ms release), README + man page + keymap-table test.

## Phases

1. Foundation (01) — 2. Browse (02–04) — 3. Navigate (05–06) — 4. Git
(07–08) — 5. Polish (09).

## Lessons that shaped the process (see skills)

- iocraft fullscreen exits on Ctrl+C by default (ignore_ctrl_c required for
  C-c bindings); store mutations need a State bump to repaint; tokio watch
  sends with zero receivers are discarded (pre-subscribe before background
  jobs publish). Library skills + subagent-orchestration + plan-process carry
  the full set; UX verification requires sized-PTY runs, not unit tests.

## Known follow-ups → plan 002

Magit depth (user: "not fleshed out"), cursor visibility audit on real
terminals, frame overprint/flash at layout shifts (renderer-layer), tree
column polish.

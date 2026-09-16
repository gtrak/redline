# 001 — Redline: a read-focused TUI code browser

Status: planned
Phases: 5 · Issues: 01–09

## Why

Code review happens in a herdr pane while agents write code. Hunk covers deep
diff review; nothing covers the *quick* loop — "what changed, where is this
defined, how does this connect, let me stage and commit that" — without
switching to an editor. Redline is an emacs/helm/magit-flavored,
**read-focused** code browser that lives in a pane:

- instant navigation in any repo (files, symbols, definitions, references, grep)
- a git surface for glance + stage + commit (magit subset)
- live views: what agents write on disk shows up immediately
- light editing only (commit messages, notes); deep review stays in Hunk

## What

A single-binary Rust TUI named `redline` (source dir stays `red/`).

### Key architectural decisions

1. **iocraft** for the UI (declarative components + hooks). All rendering
   lives in `ui/` so a ratatui fallback remains possible if iocraft strains
   at very large buffers.
2. **Command registry**: every interactive action is a named, documented
   command. Keymaps (global + per-view, prefix-sequence aware) bind keys to
   commands; `M-x` is a Picker over the same registry. Config-remappable
   keys fall out of this for free.
3. **One helm-style Picker component** (prompt + nucleo fuzzy filter +
   candidate list + live preview pane) powers files, buffers, symbols,
   commands, branches, and commits.
4. **No LSP in v1.** Definitions come from a background, ignore-aware,
   rayon-parallel tree-sitter symbol index; references from embedded ripgrep
   with tree-sitter comment/string filtering. Navigation sits behind an
   `Xref` trait so an LSP backend can slot in later.
5. **git2 (libgit2)** for the magit subset. Magit buffers are section trees
   (plain data models rendered by generic components → snapshot-testable).
6. **Light editing, bounded**: ropey buffers carry an editable flag; only
   commit messages, notes/scratch, and explicitly-opened files are editable.
   No undo system in v1.
7. **File watching is first-class**: a debounced watcher reloads views
   (keeping scroll anchor), flags conflicts for locally-edited buffers, and
   publishes project-change events that git status and the symbol index
   subscribe to.
8. **Everything embedded**: 11 tree-sitter grammars statically linked;
   ripgrep crates linked in (no PATH dependence). Cache in
   `~/.cache/redline/`, config in `~/.config/redline/config.toml`.

### Out of scope (v1)

LSP, undo, window splits (single main view + overlays), interactive rebase,
runtime-loaded grammars, generic source editing.

## Success criteria

User-visible, on a real large repo:

- `redline` cold-starts well under a second.
- Emacs flow works from muscle memory: `C-x C-f`, `C-x b`, `M-x`, `M-.`,
  `M-,`, `M-?`, `C-s`, `C-c p f`, `C-c p s s`, `C-x g`.
- A full review loop never leaves the pane: open repo → find file → jump to
  definition → list references → read blame/log → stage hunks → commit.
- Files agents write appear or refresh within ~1s without losing the
  reader's scroll position.
- Indexing and searching never block or freeze the UI.

## Task order

| Phase | Issues | Depends on |
|---|---|---|
| 1 · Foundation | 01 app skeleton & command system | — |
| 2 · Browse | 02 project & file picker → 03 syntax & file view → 04 file watching | 01 |
| 3 · Navigate | 05 symbols & jump → 06 search & references | 03, 04 |
| 4 · Git | 07 git status & staging → 08 log, blame, commit, branches | 01 (parallel to phases 2–3) |
| 5 · Polish | 09 polish & packaging | 01–08 |

```
01 ──> 02 ──> 03 ──> 04
 │            │      │
 │            v      v
 │           05 ──> 06
 └─> 07 ──> 08    (07 can start any time after 01)
  \_________|____
            v
            09
```

07 only soft-depends on 03 (for "open file under point"). Implement and
review issue by issue.

When the plan is fully implemented, archive per the plan-process skill:
consolidate into `docs/plans/archive/001-redline-code-browser.md`, then
remove this folder.

## Issue index

- [01 — App skeleton & command system](01-app-skeleton.md)
- [02 — Project layer & file picker](02-project-and-file-picker.md)
- [03 — Syntax & file view](03-syntax-and-file-view.md)
- [04 — File watching](04-file-watching.md)
- [05 — Symbols & jump navigation](05-symbols-and-jump.md)
- [06 — Search & references](06-search-and-references.md)
- [07 — Git status & staging](07-git-status-and-staging.md)
- [08 — Log, blame, commit, branches](08-log-blame-commit.md)
- [09 — Polish & packaging](09-polish.md)

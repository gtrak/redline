# Redline

A read-focused TUI code browser for the terminal. Emacs/helm/magit-flavored,
built in Rust with iocraft.

## What it does

- Instant navigation in any repo: files, symbols, definitions, references, grep
- Git surface: magit-status subset (stage, unstage, commit, log, blame)
- Live views: files agents write on disk appear immediately
- Light editing: commit messages, per-project notes (no undo in v1)
- Tree sidebar, 2 built-in themes (dark/light), config-remappable keybindings

## Install

```sh
cargo install --path .
# or for development:
cargo build --release
# binary at target/release/redline
```

## Quick start

```sh
cd your/repo
redline
```

Redline auto-detects the project root (git repo or marker file). The status
line shows the project name and current view.

## Keymap cheat sheet

Generated from the command registry. The `M-x` palette lists all commands.

| Key | Command | Description |
|-----|---------|-------------|
| `C-x C-f` / `C-c p f` | find-file | Find a file in the project |
| `C-x b` | switch-buffer | Switch to an open buffer |
| `C-x C-b` | list-buffers | List open buffers |
| `C-x k` | kill-buffer | Kill the selected buffer |
| `C-x n` | open-notes | Open the per-project notes buffer |
| `C-x C-s` | save-buffer | Save the current buffer to disk |
| `C-x C-c` | quit | Quit redline |
| `C-x g` | magit-status | Show/refresh git status |
| `C-c p p` | switch-project | Switch project (projectile) |
| `C-c p e` | recent-files | Open a recently visited file |
| `C-c p i` | re-walk | Re-walk the project file list |
| `C-c p s s` | project-search | Project-wide literal search |
| `C-c p t` | toggle-tree | Toggle the file-tree sidebar |
| `M-x` | toggle-tree-follow | Toggle tree buffer-follow (off by default) |
| `M-x` | open-palette | Command palette |
| `M-.` | xref-find-definitions | Jump to definition under point |
| `M-,` | jump-back | Pop back in the jump stack |
| `C-i` / `Tab` | jump-forward | Walk forward in the jump stack |
| `M-i` | imenu | Open the imenu outline |
| `M-?` | references-at-point | References to the symbol under point |
| `M-s o` | occur | Regex occurrences in the current buffer |
| `C-s` | isearch-forward | Incremental search forward |
| `C-r` | isearch-backward | Incremental search backward |
| `M-g g` | goto-line | Jump to a line number |
| `C-v` / `PageDown` | scroll-page-down | Scroll down one page (point's screen row pinned) |
| `M-v` / `PageUp` | scroll-page-up | Scroll up one page |
| `j` | scroll-line-down | Scroll down one line |
| `k` | scroll-line-up | Scroll up one line |
| `C-n` / `↓` | point-down | Point down one line (goal column preserved) |
| `C-p` / `↑` | point-up | Point up one line |
| `M-f` | word-forward | Point forward to the END of the next word (wraps across lines) |
| `M-b` | word-backward | Point backward to the START of the previous word |
| `C-l` | recenter | Recenter (emacs `recenter-top-bottom`): the point stays put; a fresh C-l puts it on the MIDDLE row, and consecutive C-l's cycle middle → top → bottom (the cycle resets on any other command) |
| `g` | reload-buffer | Force-reload the current file |
| `G` / `M->` | point-buffer-end | Point to the buffer end; the window follows |
| `M-<` | point-buffer-start | Point to the buffer start; the window follows |
| `C-g` | cancel | Cancel a pending key sequence |

### Magit status view (C-x g)

| Key | Command |
|-----|---------|
| `s` | Stage the file/hunk at point |
| `u` | Unstage the file/hunk at point |
| `k` | Discard the file/hunk at point (confirmation-gated: y/n) |
| `TAB` | Fold/unfold the section |
| `RET` | Visit the file at point |
| `g` | Refresh |
| `n` / `C-n` / `↓` | Next section |
| `p` / `C-p` / `↑` | Previous section |
| `h` | Open the transient command menu (magit dispatch) |
| `?` | Open the transient command menu |
| `l` | Open git log |
| `b` | Blame the current file |
| `c` | Open the commit editor |
| `y` | Branch picker |
| `z` | Stash list |
| `q` | Close |

### Commit editor (c in magit-status)

| Key | Command |
|-----|---------|
| `C-c C-c` | Commit with the editor's message |
| `C-c C-k` / `ESC` / `C-g` | Abort |
| Printable | Type the message |
| `Backspace` | Delete |

### Mouse (best-effort)

- Wheel scroll: scrolls the current view (file, magit, search, buffer list)
- Left click in the file view: positions the cursor at the clicked line
- Limitations: no drag-select, no click in pickers/menus (v1)

## Configuration

`~/.config/redline/config.toml`:

```toml
# Theme: "dark" (default) or "light"
theme = "dark"

# Live file watching (default true)
auto_reload = true

# Key bindings: command = "sequence"
[key-bindings]
# find-file = "C-x C-f"
# quit = "C-x C-c"
```

## Performance

Measured on a 500-file Rust repo (each ~50 lines) — store-level microbench
(headless `AppStore` on the production code paths, cold cache, median of 5;
harness: `src/perf.rs`, run with `cargo test -- perf_remeasure -- --include-ignored --nocapture`):

| Metric | Debug | Release |
|--------|-------|---------|
| Cold start (store init + file walk) | ~310 ms | ~75 ms |
| Index (500 files) | ~110 ms | ~33 ms |
| Index throughput | ~4500 files/s | ~15000 files/s |
| Search first-hit | ~6 ms | ~1 ms |

(Numbers vary by hardware. Re-recorded 2026-09-17, superseding the 2025
mid-range-laptop numbers; cold start and index moved >20%, search first-hit
is within 20% of the prior table. Method: store init = `AppStore::at`
(project detection + store construction) + the initial project file walk;
index = `start_indexing` to the final index event (includes the store's own
walk, ~1–2 ms at 500 files); search first-hit = `start_project_search` to
the first hit event.)

## Architecture

- `src/app/` — store, command registry, keymap engine, watcher
- `src/model/` — project, buffer (ropey), file walk
- `src/nav/` — symbol index (tree-sitter, rayon-parallel)
- `src/search/` — ripgrep-embedded, references, occur
- `src/git/` — git2 wrapper: status, staging, log, blame, commit
- `src/syntax/` — tree-sitter grammars, highlight cache
- `src/ui/` — iocraft components: file view, magit, picker, tree, log
- `src/theme.rs` — dark/light themes

## License

MIT

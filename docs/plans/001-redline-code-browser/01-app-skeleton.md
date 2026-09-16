# 01 — App skeleton & command system

Phase 1 · Foundation · Depends on: —

## Objective

A runnable `redline` TUI with the application chassis: event loop, central
app store, command registry + keymap engine, minibuffer, status line, and an
`M-x` palette over placeholder commands. This issue ships no features — it
ships the *shape* every later feature plugs into.

## Key decisions

- **Command registry**: actions are named commands (name, docs, category);
  keymaps bind key sequences → commands; per-view keymaps override global;
  prefix sequences supported (`C-x …`, `C-c p …`).
- **Single main view + overlays** (no splits in v1); views push/pop a stack.
- **Picker component** introduced here — the palette is its first consumer;
  file/symbol/git sources arrive in later issues.
- `tracing` writes to a log file (TUI debugging); terminal restored cleanly
  on every exit path, including panics.

## Files

| Area | Change |
|---|---|
| `Cargo.toml` | crate scaffold; deps: iocraft, tokio, tracing, tracing-subscriber, serde, toml, anyhow, thiserror, dirs |
| `src/main.rs` | entry: config load, tracing init, event loop |
| `src/app/` | store (view stack, minibuffer, status), command registry, keymap engine, config |
| `src/ui/` | root component, view stack, minibuffer, status line, picker (palette) |
| `src/theme.rs` | color/face definitions (stub values) |

## Steps

1. Scaffold crate + deps; file logging; launch/restore terminal safely.
2. App store + view stack; quit works from any view.
3. Command registry with metadata; dispatch-by-name API.
4. Keymap engine: global + per-view maps, sequences/prefixes, pending-key
   indicator in the status line.
5. Minibuffer (echo area + messages) and status line (project placeholder,
   view name, pending keys, async-activity indicators).
6. Picker component (prompt, fuzzy filter, list, preview stub); `M-x`
   palette over the registry.
7. Config skeleton (`~/.config/redline/config.toml`): theme selection and
   key-binding overrides loaded and applied.

## Verification

- `redline` launches; `M-x` opens the palette, typing filters, `RET` runs a
  placeholder; `C-g` cancels; `q` / `C-x C-c` quit and restore the terminal.
- Unknown keys echo in the minibuffer; pending prefixes show in the status
  line.
- Unit tests: keymap resolution (override, prefixes), registry dispatch.
- `cargo clippy` clean; tracing file captures a full session.

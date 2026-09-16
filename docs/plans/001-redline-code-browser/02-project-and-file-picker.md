# 02 — Project layer & file picker

Phase 2 · Browse · Depends on: 01

## Objective

Projectile-style project layer plus helm file finding: open, switch, and
kill files/buffers instantly in any project; switch projects; recents
persist across sessions.

## Key decisions

- Project root = innermost git root, else marker-file scan (`.git`,
  `Cargo.toml`, `package.json`, `pyproject.toml`, `.projectile`, …).
- File list built by an `ignore`-crate walk (respects `.gitignore`), cached
  per project; refreshed on demand now, automatically in 04.
- Recents + project registry persisted under `~/.cache/redline/`.
- Picker preview shows the selected file's first page as plain text
  (highlighting arrives with 03).

## Files

| Area | Change |
|---|---|
| `src/model/project.rs` | root detection, project identity, registry |
| `src/model/files.rs` | cached ignore-aware walk |
| `src/model/buffer.rs` | open-buffer set, current buffer (plain-text view placeholder) |
| `src/ui/` | file/buffer/project sources for picker, buffer-list view |
| `src/app/commands.rs` | find-file, switch-buffer, list-buffers, kill-buffer, switch-project, recent-files |

## Steps

1. Root detection + project identity; status line shows project name.
2. Cached ignore-aware file walk; manual re-walk command.
3. File picker source + live preview; `RET` opens the file.
4. Buffer model: open set, current buffer, `C-x k`; buffer list + switch.
5. Recents (per project) + `C-c p p` project switcher; persist registry.
6. Wire default bindings: `C-x C-f`, `C-x b`, `C-x C-b`, `C-x k`,
   `C-c p p`, `C-c p f`.

## Verification

- In a 10k+-file repo, `C-x C-f` filters interactively with no perceptible
  lag; preview follows selection; `RET` opens the file.
- `target/`, `node_modules/`, and other ignored paths never appear.
- Buffer list/switch/kill behave; killed buffers disappear.
- `C-c p p` switches between two repos; recents and the project list
  survive a restart.

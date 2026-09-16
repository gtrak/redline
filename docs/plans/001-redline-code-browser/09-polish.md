# 09 — Polish & packaging

Phase 5 · Polish · Depends on: 01–08

## Objective

Daily-driver polish and packaging: tree sidebar, themes and full keymap
config, notes/scratch, mouse, performance pass, docs, and the Hunk handoff
experiment.

## Key decisions

- Tree sidebar = treemacs-lite (ignore-aware, optional buffer-follow).
- **Notes/scratch** is a per-project notes file opened as an editable
  buffer — exercises the light-editing path outside git.
- **Hunk handoff is a stretch goal**: investigate the Hunk CLI; only wire
  `H` (open current file/hunk in Hunk) if a stable surface exists.

## Files

| Area | Change |
|---|---|
| `src/ui/tree.rs` | project tree sidebar |
| `src/theme.rs` + themes | 2–3 built-in themes |
| `src/app/config.rs` | full keybinding overrides |
| `src/model/notes.rs` | per-project notes file |
| `README.md`, `redline.1` | docs, man page |

## Steps

1. Tree sidebar (toggle; ignore-aware; optional cursor-follow).
2. Built-in themes; full keymap overrides via config.
3. Notes/scratch buffer per project (create, edit, save).
4. Mouse: click to position, wheel scroll.
5. Performance pass: cold start, index speed, search latency, memory —
   record numbers.
6. (Stretch) Hunk handoff via its CLI, if feasible.
7. README + man page; release build; install instructions.

## Verification

- Fresh-machine walkthrough using only the README: clone a repo → browse,
  jump, search, stage, commit without leaving `redline`.
- All tests green; `cargo clippy` + rust-safety audit pass.
- Recorded startup < 500ms cold on a large repo; index/search numbers
  noted in the archive summary.

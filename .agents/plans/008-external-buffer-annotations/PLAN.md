# 008 — Redline: annotate external/library buffers

Status: planned (queued ahead of 004-06; user request 2026-09-18)
Phases: 1 · Issues: 01
Depends on: plan 005 (annotations — archived, machinery in place), 006-02
(external read-only landing — in review)
Origin: user: "I can't annotate library buffers and want to" — i.e. the
registry sources M-. lands in (`~/.cargo/registry/src/...`) reject the
whole annotation flow.

## Why

006-02 made `M-.` land in dependency sources (READ-ONLY via
`open_external_path`). The user immediately wants the primary read-side
feature there: inline notes on library code. The blocker is path keying:
`buffer_rel_path` does `abs.strip_prefix(project.root)` and returns
`None` for anything outside the workspace, so `A`'s prefill matches
against `""`, a committed record would key to an empty path, the `▎`
marker never renders, `d` finds no record, and the dump shows `:line`
with no path. All TEN consumers of `buffer_rel_path`/
`current_buffer_rel` are annotation machinery (verified:
store.rs:2101/2108/2124/2233/2308/2370/2492/4288/4406) — no
tree/recents/watcher consumer touches the path, so the seam is clean.

## What

1. **Issue 01 — absolute-path annotation keys.** New single
   key-derivation point (e.g. `buffer_annotation_path`): project-relative
   when under the root (backward-compatible with every existing record),
   else the ABSOLUTE path string. All annotation consumers repoint:
   `A` prefill, prompt-commit, `d` delete, `record_index_for_line`,
   re-anchor, marker rendering in `file_view_rows`, the dump accessor.
   Storage stays the project's `.redline-notes.md` (the note is about
   this project's dependency usage). Dump prints the absolute path
   verbatim — unambiguous for agents.

## Key decisions (inferred intent, veto in review)

- **Notes live in the project's notes file, keyed by absolute path.** A
  note about ropey's internals belongs with the project whose code calls
  ropey; agents get a fully-qualified `path:line` they can open from
  anywhere.
- **Registry-hash path instability is inherent and acceptable**: a crate
  update changes the source dir, which orphans the record — the existing
  ORPHANED discipline handles that honestly (explicit marker, never a
  silently wrong line).
- **No watcher/auto-reload for external buffers** (pre-existing: they are
  not in the project walk). Re-anchor runs on load; a long-lived session
  watching a library file that changes on disk is out of scope.
- **Read-only buffers annotate fine** — annotations are a read-side
  feature (that was true before 006-02 for project files); editability is
  irrelevant to `A`.

## Success criteria

- `M-.` into a registry source, `A`, type, Enter → marker + inline note
  row render in the external buffer; `d` deletes; quit dump carries
  `~/.cargo/.../rope.rs:123: NOTE: ...` (or the absolute path) with the
  code line.
- Existing project-relative annotations are byte-identical in the notes
  file (no key migration).
- Full gates green; new PTY legs: annotate/delete/dump on an external
  buffer.

## Task order

| Phase | Issue | Depends on |
|---|---|---|
| 1 | 01 absolute-path annotation keys | 006-02 landed |

When complete, archive per plan-process.

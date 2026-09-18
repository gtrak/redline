# 008 — Redline: annotate external/library buffers (ARCHIVED)

Status: implemented (1/1 issue, review PASS round 1; 2026-09-18)

## Scope delivered

- **01 Absolute-path annotation keys** (870c9d8): one new key-derivation
  point, `buffer_annotation_path(key)` — the lossy project-relative path
  when under the root (the *identical expression* the old
  `buffer_rel_path` used; existing records key byte-identically, no
  migration), else the absolute path string. `current_annotation_path()`
  for current-buffer call sites. All eight annotation consumers repointed
  (record_index_for_line, reanchor_for_key, buffer_line_text_for_rel,
  annotate prefill, prompt-commit, status count, file_view_rows
  marker/note matching, annotations_for_dump) — the old names were
  REMOVED (zero occurrences in src), making an un-repointed consumer a
  compile error rather than a silent bug. Notes stay in the project's
  `.redline-notes.md`; external records key absolute; dump prints the
  path verbatim (block and plain). `open_external_path` read-only
  semantics, watcher scoping, recents, and tree-follow all untouched.
- **Why**: user request — "I can't annotate library buffers and want to."
  The pre-008 blocker: `buffer_rel_path` strip_prefix(project.root) →
  None for `~/.cargo/registry/...`, killing A/d/markers/re-anchor/dump on
  dependency sources landed in by 006-02's M-. fall-through.

## Verification at archive

503 passed / 0 failed / 2 ignored; all suites green incl. the new
`tools/drive_external_notes.py` 14/14 — which drives a GENUINE M-.
resolver landing into a cached ropey-1.6.1 registry source (dependency
renamed `extrope` in the leg fixture so the crate name never hits the
in-project TOML-key index and fakes the leg). Reviewer confirmed the
impostor test (a rel-path record can never match an absolute key and vice
versa) and the byte-identity test are discriminating. Live check: note
anchored inside `ropey-1.6.1/src/rope.rs:82`, rendered inline, dumped
with the absolute path + code line.

## Carry-forwards (folded into 006-02b)

- `Annotation` struct doc (~store.rs:666) still says "path is
  project-relative" — one-line fix queued.
- Mixed dump ordering (absolute vs relative record) not order-pinned —
  queued.
- `drive_external_notes.py` not wired into `drive_all.py` — queued.
- Known inherent limit: a crate update changes the registry hash dir and
  orphans external records (explicit ORPHANED marker, by design).

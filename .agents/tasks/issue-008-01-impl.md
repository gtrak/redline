# Task: plan 008 issue 01 — annotate external (library) buffers

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## Origin (user, 2026-09-18)

"I can't annotate library buffers and want to." Library buffers = the
external sources `M-.` lands in since 006-02 (`~/.cargo/registry/src/...`,
opened READ-ONLY via `open_external_path`). The whole annotation flow is
dead there.

## Diagnosis (orchestrator-verified — start here)

`buffer_rel_path` (store.rs:2106) does `abs.strip_prefix(project.root)` →
`None` for external paths. Every annotation consumer keys off
`current_buffer_rel()` / `buffer_rel_path`, so on an external buffer:

- `A` prefill (store.rs:2308) compares `a.path == ""` — never matches;
- the prompt-commit (store.rs:2370) bails at `let Some(rel) = ... else`;
- `d` delete (store.rs:2492) bails the same way;
- `record_index_for_line` (2108), re-anchor (2124/2233), marker rendering
  in `file_view_rows` (4288), and the dump path (4406) all see `None`.

All TEN call sites are annotation machinery — verified: no
tree/recents/watcher consumer uses these functions. The seam is clean.

## What to build

1. **One new key-derivation point**, e.g. `fn buffer_annotation_path(&self,
   key) -> Option<String>`: project-relative path (lossy) when the
   buffer's path strips under the project root — IDENTICAL to today for
   existing buffers — else the absolute path string
   (`abs.to_string_lossy()`). `current_buffer_rel()` variant
   (`current_annotation_path()`) for the current-buffer call sites.
2. **Repoint all annotation consumers** to the new functions: annotate
   prefill, prompt-commit, `d`, `record_index_for_line`, re-anchor
   (both sites), `file_view_rows` marker/note matching, dump accessor
   chain. Existing project-relative records must key byte-identically —
   NO migration, NO notes-file change.
3. **Dump**: the record's path prints verbatim (absolute for external —
   unambiguous for agents; rel for project files, unchanged). Both block
   and plain modes. No format change otherwise.
4. **Do NOT** extend the watcher/file-walk to external paths; do not
   touch `open_external_path`'s read-only/no-recents semantics; do not
   make external buffers editable; no annotation-model changes (the
   `path` field already holds an arbitrary string).

## Constraints

- Skills are truth (`.agents/skills/*.md`; no registry/docs.rs/fetch).
  Write-first; compile early. PTY suite flock: "shared PTY fixture is
  busy" + exit 3 ⇒ wait and retry; NEVER two suites concurrently; wrap
  EVERY python PTY invocation in `timeout`. Honest gate counts. Repo
  identity configured — plain `git commit`, no -c overrides.
- Scope fence: src/app/store.rs (the key functions + consumer repoint +
  tests), tools/ (new PTY legs), docs/README if the annotate docs mention
  project-only scope. No main.rs, no resolver, no queries, no model
  changes.

## Verification (iterate until ALL pass)

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test /
  sweep.py / sweep_flows.py / drive_all.py / drive_windowing.py /
  drive_windowing_panes.py / check_cursor_stream.py / ux_sweep.py /
  probe_notes_dump.py / drive_xref.py — all green, HONEST counts.
- Unit tests: external-path key derivation (under-root unchanged,
  out-of-root = absolute); annotate+delete round-trip on an external
  buffer; marker rendering keyed by absolute path; dump shows the
  absolute path; existing rel-path records untouched (byte-identical
  notes file).
- New PTY legs (extend drive_xref.py or a sibling drive): `M-.` into an
  external source (fixture has no Cargo.toml — drive `open_external_path`
  via the resolver miss path or a dedicated leg harness that seeds an
  out-of-project file and opens it the way the resolver would), then
  `A` → type → Enter → marker + note row visible; `d` removes it; quit
  dump carries the absolute path + code line.
- Live check: in the real repo, `M-.` on `Rope` → registry source → `A`
  → note renders inline → quit dump contains the absolute path.

## Report format

Key-derivation design + the full consumer list repointed. Gate counts
(honest). Deviations. The live-check dump excerpt.

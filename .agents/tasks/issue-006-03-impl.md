# Task: plan 006 issue 03 — navigate within external sources (follow types inside a library buffer)

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## Origin (user, 2026-09-18, after 008-01 landed)

"library annotation working splendidly. I want to follow other types once
inside a library buffer." Since 006-02, `M-.` lands in registry sources
(`~/.cargo/registry/src/...`) read-only; annotations work there (008-01).
But INSIDE such a buffer, `M-.` refuses with "buffer not in project"
(store.rs ~7105, `xref_find_definitions`), and imenu refuses the same way
(~7413, `open_imenu`). The user wants to follow types/functions within the
library they just jumped into.

## The key fact that makes this tractable

`ResolvedSource` (crates/redline-resolve/src/lib.rs:92) already carries
`source_root`: "Root of the source tree the file belongs to (external
crate root or the workspace itself)" — for a registry landing that is the
crate's source dir (e.g.
`~/.cargo/registry/src/index.crates.io-.../ropey-1.6.1/`).
`nav::index::build_index(root, files, progress)` (src/nav/index.rs:278) is
root-agnostic: it parses whatever root+file list you give it with the same
rayon-parallel tree-sitter machinery the project index uses. So an index
of the crate's own source tree is a solved problem — it just has never
been built or consulted.

## What to build

1. **Crate index (background, off the input path).** A small cache of
   external source indexes, e.g.
   `external_indexes: Vec<(PathBuf source_root, Arc<Mutex<SymbolIndex>>)>`
   (LRU, cap ~2-3 entries; evict oldest when a new crate is landed). When
   `open_resolved_source` lands on an external file whose source_root has
   no index yet, walk `**/*.rs` under source_root (relative paths) and
   build the index on `tokio::task::spawn_blocking`, publishing through a
   bus mirroring the existing `IndexBus`/`ResolveBus` patterns
   (watch channel, latest-wins, internal receiver). While building, the
   status line shows an `indexing crate …` indicator exactly like the
   project indexing indicator. Set a sane cap (e.g. refuse-to-index +
   clear message above ~2000 files — no known registry crate approaches
   this). Do NOT block the landing on the index: M-. works with whatever
   is indexed when pressed (misses behave as below).
2. **M-. inside an external buffer.** In `xref_find_definitions`, replace
   the "buffer not in project" refusal for pathed external buffers: find
   the cached index whose source_root is an ancestor of the buffer's path
   (there is exactly one — the one that landed you here); derive the
   crate-relative path; run the SAME selection rule as the project path
   (`symbol_at_point`, `::`-path token, same-file-first candidate
   ordering, dedup) against the crate index. Exactly one candidate → jump
   (open via `open_external_path` — read-only, and the landing stays
   inside the same source_root so the cache stays valid); several → the
   existing Xref picker with crate-relative display paths; miss →
   enclosing-symbol fallback from the crate index outline; miss → the
   resolver fall-through (unchanged semantics: `SymbolContext` still
   carries the ORIGIN project's workspace_root, so following a type into
   ANOTHER dependency resolves through the origin project's metadata —
   landing in a second crate registers its index per (1), and the LRU cap
   governs). Project-file M-. behavior is untouched.
3. **imenu inside an external buffer.** `open_imenu`'s outline comes from
   the owning crate index for external buffers instead of refusing.
4. **blame stays refused** for external buffers (registry sources are not
   git checkouts) — but improve the message to say why ("no git history
   for external sources").
5. **Non-goals**: the crate index must never be treated as "the project"
   for recents, tree sidebar, file watcher, or edit mode (external buffers
   stay read-only and outside the project registry — 008-01 semantics
   preserved). No resolver-crate changes. No annotation changes (keys are
   absolute since 008-01; re-anchor on load already covers external
   buffers).

## Constraints

- Skills are truth (`.agents/skills/*.md`; no registry/docs.rs/fetch).
  Write-first; compile early. PTY flock: "shared PTY fixture is busy" +
  exit 3 ⇒ wait and retry; NEVER two suites concurrently; wrap EVERY
  python PTY invocation in `timeout`. Honest gate counts from actual
  output. Repo identity configured (Gary Trakhman) — plain `git commit`,
  no `-c` overrides, no history rewrite.
- Scope fence: src/app/store.rs (cache + xref/imenu/landing wiring +
  tests), src/ui/root.rs (status indicator drain only, mirroring the
  existing index drain), src/nav/index.rs ONLY if a small accessor is
  genuinely missing (prefer none), tools/ (new PTY drive), docs/README
  keymap row. No main.rs, no resolver crate, no queries, no annotation
  changes.
- Note: 006-02b may land concurrently-to-sequentially before you; it
  touches store.rs (ownership guard, generation bump, symbol_at_point
  colon boundary). Rebase your understanding on the actual HEAD when you
  start; if HEAD already contains 02b, its `symbol_at_point` fixes are
  the baseline.

## Verification (iterate until ALL pass)

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test /
  sweep.py / sweep_flows.py / drive_all.py / drive_windowing.py /
  drive_windowing_panes.py / check_cursor_stream.py / ux_sweep.py /
  probe_notes_dump.py / drive_xref.py / drive_external_notes.py — all
  green, HONEST counts.
- Unit tests: crate index builds for a synthetic out-of-root tree; M-.
  same-file and cross-file WITHIN the crate (same-file-first ordering);
  imenu outline for an external buffer; miss falls through to the
  resolver (origin-project metadata); LRU eviction keeps the cap; a
  project file's M-. is byte-identical in behavior (regression).
- New PTY drive (own fixture repo + own per-repo flock, the
  drive_external_notes pattern): real Cargo.toml depending on ropey (via
  the `extrope` rename trick so TOML-key index entries can't fake it),
  M-. lands in the registry source, then M-. on another type WITHIN
  ropey jumps within the crate (cross-file), imenu lists the current
  file's symbols, `indexing crate` indicator appears and clears, M-,
  walks the jump stack back.
- Live check: in the real repo, M-. on `Rope` → registry source → M-. on
  another ropey type → lands within the crate; report the jump messages.

## Report format

Cache/bus design; the exact refusal sites lifted and how each now behaves;
gate counts (honest); deviations; live-check jump messages.

# 02 — app wiring: M-. fall-through

Phase 1 · Depends on: 01 + plan 005 (src/ work — serialized with UX lanes)

## Objective

The app consumes redline-resolve: M-. misses fall through to the provider
chain; external sources open read-only.

## Key decisions

- store's M-. handler: workspace Xref hit → jump as today; miss → build
  SymbolContext → redline-resolve chain → open ResolvedSource (read-only
  when external). Status-line activity while resolving/fetching.
- Spawn the resolution off the input path (non-blocking UI), result lands
  like a jump-stack entry.

## Files

| File | Change |
|---|---|
| `src/app/store.rs` | fall-through wiring + status activity (serialized with UX lanes). |
| `Cargo.toml` (root) | dependency on redline-resolve (path). |
| tests + tools/ flows | external jump E2E through the app. |

## Orchestrator pre-check (2026-09-18, against committed tree)

- **The M-. handler is `xref_find_definitions`** (`store.rs`), bound at
  `store.rs:158` (`M-.`). Its current flow: current buffer → project-rel
  path → collect identifiers on the point line → index lookups → dedup →
  "enclosing symbol" fallback → if no defs: `minibuffer_message("no
  definition for `<name>`")`. **That message is the fall-through seam**
  (and the `defs.is_empty()` on the enclosing-symbol path). The
  SymbolContext the resolver needs: `workspace_root = project.root`,
  `symbol` = the identifier we tried (`lookup_name`, or the
  `tokio::spawn`-shaped text under point if parseable — note the current
  code tokenizes on `!is_alphanumeric() && != '_'`, which BREAKS `a::b`
  into `a`/`b`; for the resolver the path-shaped symbol matters, so the
  fall-through should re-derive the dotted/scoped token at point rather
  than reuse `lookup_name` blindly), `from_file` = `rel`.
- **`open_path(rel)` is project-relative** (`project.root.join(rel)`)
  and inserts with `editable = false` (correct: external sources open
  read-only by default — no new flag needed). External absolute paths do
  NOT fit `open_path`'s signature (`strip_prefix`/`join` on the project
  root), so add a small `open_abs_path(abs)` (or extend `open_path` to
  accept an absolute path) that inserts read-only and skips the project
  recents/project-registry writes — the plan already says external jumps
  are per-session.
- **Async pattern to mirror**: the symbol indexer uses
  `index_bus: IndexBus` + `tokio::task::spawn_blocking` (`store.rs:5476`,
  `:5528`) and the drain installs results. The resolver shells out
  (`cargo metadata`/`cargo fetch` — network, seconds), so it MUST run off
  the input path the same way: spawn_blocking → result lands via a bus
  (or a oneshot the drain polls) → then open the source and push the jump
  entry. Status-line activity text while pending.
- **`redline-resolve` public API** (crates/redline-resolve/src/lib.rs):
  `Resolver::new()/with_providers/add<P: ToolingProvider>/
  resolve(&SymbolContext) -> anyhow::Result<ResolvedSource>` (and
  `resolve_traced` → `ResolveOutcome` with `Attempt`s for activity text).
  `ResolvedSource` and `Attempt` are the types to consume. Root
  `Cargo.toml` needs the path dependency; check the crate's own
  `Cargo.toml` for its name.
- **Jump stack**: `current_jump_entry()` + the existing record-navigate
  path (see the unique-def branch) is the pattern for the external jump
  entry; out-of-workspace paths must not enter the project registry.

## Verification

- Gates green; PTY: M-. on tokio symbol lands in registry source
  read-only; workspace jumps unchanged.

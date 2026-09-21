# Task: plan 006 issue 02 — M-. fall-through to the tooling resolver

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## User directive (plan 006 origin, 2026-09-17)

"jump-to-selection should be language and tooling aware, eg resolve the
real cargo source folder, pull it if necessary by shelling out to those
tools."

## User report (2026-09-18) — the CURRENT M-. is unpredictable

"I'm noticing jump-to-definition doesn't use my cursor position. I don't
know what it's doing, but it's quite unpredictable. If I'm on a field '.'
access, I want to see where the field is defined, for example. If I'm on
a Trait, I want to jump into the trait definition."

This issue therefore has TWO halves: (A) make M-. cursor-aware and
same-file-capable (a real bug, fixable now), and (B) wire the resolver
chain (the plan's main goal).

## Working agreement

- Skills are truth (emacs-ux; no registry reads, no docs.rs, no fetch).
  Write-first; compile early; iterate on named errors. Minimal skill
  corrections, listed. `graft` available. The PTY suite takes an exclusive
  flock on the shared fixture — if a suite says "shared PTY fixture is
  busy" and exits 3, wait and retry; NEVER run two PTY suites
  concurrently. Wrap EVERY python PTY invocation in `timeout`.

## Read first

1. `src/app/store.rs` — `xref_find_definitions` (grep `pub fn
   xref_find_definitions`): the current seam. It uses `self.point_line()`
   ONLY (never the column), splits `line_text` on
   `!is_alphanumeric() && != '_'` (which BREAKS `tokio::spawn` into two
   candidates), filters `.filter(|d| d.file != rel)` (deliberately
   excludes SAME-FILE definitions), picks `all_defs[0]` after sorting by
   (file, line, name) — i.e. an arbitrary identifier on the line, not the
   one under the cursor — and falls back to
   `index.enclosing_symbol(outline, line)`.
2. `src/app/store.rs` — the jump machinery the fall-through must reuse:
   `current_jump_entry()`, `open_path(&rel)`, `set_point_line(...)` (the
   unique-def branch of `xref_find_definitions` is the pattern).
3. `crates/redline-resolve/src/lib.rs` — the public API you are wiring:
   `Resolver::new()/with_providers/add<P: ToolingProvider>`,
   `resolve(&SymbolContext) -> anyhow::Result<ResolvedSource>` /
   `resolve_traced(...) -> ResolveOutcome` (with `Attempt`s for the status
   line), `SymbolContext { workspace_root, symbol, from_file }`,
   `crate_from_symbol(&str) -> Option<&str>`, and `ResolvedSource`
   (location to open).
4. `crates/redline-resolve/src/cargo.rs` + `providers/` — what the cargo
   provider needs (crate name+version → registry source dir; `cargo fetch`
   if absent). Note the provider deadlines (30s local / 120s network).
5. `.agents/plans/006-tooling-aware-jump/02-app-wiring.md` — the issue
   contract + the orchestrator pre-check (teardown, async pattern,
   `open_path` is project-relative).
6. `.agents/plans/archive/007-syntax-aware-linking.md` — the roadmap context
   for why symbol extraction is currently text-splitting (007-01 will
   replace it with node-at-point; do NOT build that here, but do not make
   it harder either).

## What to build

### A. Make M-. cursor-aware and same-file-capable (fix the unpredictability)

1. **Use the point's column.** Select the identifier AT the point (the
   run of `[A-Za-z0-9_]` around `point_col()` on the point line), not
   "every identifier on the line". A `::`-separated path under the point
   (`tokio::spawn`) should be extracted as ONE path-shaped token when the
   cursor is inside it — keep the raw token available for the resolver,
   and for the workspace index try BOTH the last segment and the full
   path (the index is name-keyed).
2. **Stop excluding same-file definitions.** The current filter makes
   every same-file symbol unjumpable (very common: struct + impl in one
   file), which is a large part of the unpredictability. Instead:
   prefer the definition whose name matches AND is nearest the point
   (e.g. same-file first if it is a real match), and keep the
   ambiguous-multi-candidate case going to the Xref picker (which already
   exists) so the user chooses. Same-file jumps must work.
3. **No behavior change for the enclosing-symbol fallback** beyond what
   (1)/(2) require.
4. Document the new selection rule in the code comment and the README
   keymap row.

### B. Wire the resolver chain (the plan's goal)

1. Add the path dependency on `redline-resolve` (root `Cargo.toml`).
2. On M-.: workspace index hit → jump as today. **Miss → fall through**:
   build `SymbolContext { workspace_root, symbol: <path-shaped token>,
   from_file: rel }`, run the resolver chain OFF the input path
   (`tokio::task::spawn_blocking` + a `tokio::sync::watch` bus, mirroring
   the symbol-indexer pattern at `store.rs` ~5476/5528 — `cargo fetch` is
   a network shell-out, it must never block the key path).
3. Status line shows activity while resolving/fetching (`resolving …` /
   provider attempts from `ResolveOutcome`), and the result lands like a
   jump-stack entry: `open_path`/open-absolute the resolved source READ-ONLY
   (external sources must never enter edit mode or the project recents;
   `open_path` is project-relative so an external absolute path needs a
   small `open_abs_path`-style helper that inserts with `editable=false`
   and skips the project registry).
4. Failures degrade gracefully: a miss reports
   "no provider resolution for `<symbol>`" — it must never block the
   workspace path or panic.
5. A miss on a workspace symbol behaves as today (no resolver spam).

## Constraints

- Scope fence: `src/app/store.rs` (M-. + the async bus + the open helper),
  `src/app/command.rs` (counts), root `Cargo.toml`/`Cargo.lock` (the one
  dependency add), `src/ui/root.rs` (status activity only), tests,
  `tools/` flows, `docs/`. No changes to annotations (005), no resolver
  crate changes beyond what its API already supports (if a change there is
  genuinely required, STOP and report — it is a separate review surface).
- All suites green: `cargo test`, `tools/sweep.py`, `tools/sweep_flows.py`,
  `tools/drive_all.py`, `tools/drive_windowing.py`,
  `tools/drive_windowing_panes.py`, `tools/check_cursor_stream.py`,
  `tools/ux_sweep.py`.
- Report HONEST gate counts from actual harness output.

## Verification (iterate until ALL pass)

- Gates: build / `clippy --all-targets -- -D warnings` / cargo test green.
- Unit tests: cursor-aware selection (identifier at point vs elsewhere on
  the line); `::`-path extraction; same-file jump works; the resolver
  fall-through fires only on a workspace miss; the async result lands as
  a jump; failure degrades to a message.
- Raw-PTY legs: M-. with the cursor on a symbol resolves and lands the
  point at the definition (same-file AND cross-file cases); M-. with the
  cursor on a `.`-accessed field name behaves predictably (resolves by
  name when the index knows it, else a clear message — say which);
  a workspace miss falls through to the resolver and reports its outcome.
- Live check: `cargo metadata` runs against the real repo (it is a dev
  dependency of the build already; no network fetch needed for a
  workspace-local dependency).

## Report format

Selection-rule design (A). Resolver wiring (B) incl. the async pattern and
the read-only external open. Per-command table. Gate counts (honest).
Deviations; known gaps.

# Task: plan 007 issue 03 — scope-aware resolution for the resolver chain

You are the implementation worker. Repo root is your cwd. Self-contained.
`.agents/skills/*.md` are authoritative ground truth.

## Origin

All four resolver providers (cargo/rust, js, python, go) refuse a BARE
symbol with "needs scope info (tree-sitter) not yet provided by the app".
That refusal is literal: they are waiting for the app to supply the `use`
path / enclosing scope. Plan 007-01 (`src/syntax/node.rs`) now provides it.

## The refusal sites (verified in the current tree)

- `crates/redline-resolve/src/cargo.rs:151-160` — `crate_from_symbol` on a
  bare symbol yields the symbol itself as the "crate" and a
  single-segment path then bails.
- `crates/redline-resolve/src/providers/js_provider.rs:~105-115` — bare
  (dot-free) symbol bails.
- `crates/redline-resolve/src/providers/python_provider.rs:~140-148` — same
  (its test at `:592` pins the message).
- `crates/redline-resolve/src/providers/go_provider.rs` — check it too.

## What to build

1. **Extend `SymbolContext`** (`crates/redline-resolve/src/lib.rs:121`) with
   an optional scope hint, e.g.
   `pub scope: Vec<String>` (or `import_path: Option<String>` — pick and
   justify). Semantics: the information needed to turn a bare symbol into a
   path-shaped one. Two distinct shapes matter, and be honest about which
   you can support per language:
   - **Rust**: the `use` declaration that brings the name into scope
     (`use serde::Deserialize;` → the bare `Deserialize` should resolve as
     `serde::Deserialize`), else the enclosing module path.
   - Other languages: the app can supply only what its tree-sitter layer
     knows; degrade to today's refusal when the hint is absent.
2. **App side (`src/app/store.rs`)**: at the resolver fall-through
   (`SymbolContext` construction, `store.rs:7659`), populate the new field
   from 007-01's `node_at`/`scope_path_at` on the current buffer:
   - if the symbol at the point is bare, find its `use` declaration in the
     file (this needs a small syntax query/walk — 007-01's `node_at` gives
     the node, not the imports; a modest, bounded walk over
     `use_declaration` nodes is in scope),
   - else pass `scope_path_at(...)` as the enclosing scope,
   - if neither is available, pass empty (⇒ providers behave exactly as
     today — the degradation contract must be preserved byte-for-byte).
3. **Providers consume the hint.** For cargo: a bare symbol + a scope/use
   path that names a crate (e.g. `serde`) should resolve through the SAME
   `locate_source_dir`/`locate_in_pkg` machinery as a path-shaped symbol —
   do not write a parallel code path; normalize the bare symbol into
   `crate::item` early (`resolve_cargo`) and let the rest flow.
   Do the same shape of change in js/python/go where a hint exists.
4. **Remove/degrade the refusals honestly.** Where a hint makes resolution
   possible, remove the "needs scope info" bail. Where it does not (no hint,
   std/core/prelude, macros, `self`/`super` paths), KEEP the bail with an
   accurate message. Do not silently guess a crate name.

## Explicit non-goals

- No LSP, no external language server (user ruled LSP out).
- No macro expansion, no generics-driven inference. This is
  import/scope resolution only — the honest, bounded improvement.
- Do not change `crates/redline-resolve`'s public trait shape beyond adding
  the optional context field (it is an app-free crate; keep it app-free —
  no iocraft/store types).
- Do not touch `src/syntax/node.rs` behavior (007-01 is frozen; call it).

## Constraints

- Skills are truth (`.agents/skills/*.md`; no registry/docs.rs/fetch).
  Write-first; compile early. If code contradicts a skill file, code wins +
  record a minimal correction.
- Gate runner: `tools/gate.sh fast` (inner loop), `tools/gate.sh full`
  (final; progress streams to stderr — do NOT pipe stdout through `tail`).
- PTY flock: "shared PTY fixture is busy" + exit 3 ⇒ wait and retry; NEVER
  two suites concurrently; wrap EVERY python PTY invocation in `timeout`.
  Honest gate counts.
- Scope fence: `crates/redline-resolve/src/**`, `src/app/store.rs` (context
  population + tests), and a `tools/` PTY leg if useful. No `src/ui/`, no
  `src/syntax/node.rs`, no annotations.
- Plain `git commit` (identity configured); do not `git add -A` other
  sessions' files. Commit on the current branch.

## Verification (iterate until ALL pass)

- `tools/gate.sh full` green, honest counts; all existing suites unchanged
  where they should be (`drive_xref` 10/10 includes the bare-miss graceful
  report leg — its expectations may legitimately change if a previously-missed
  bare symbol now resolves; if so, that is the FEATURE — say exactly which
  leg changed and why).
- Unit tests:
  - resolver-crate: a bare symbol + `scope=["serde"]` resolves through
    `locate_in_pkg` (a synthetic/known crate) — and with an EMPTY scope it
    still bails with the existing message (regression pin).
  - app: the fall-through context carries the `use` path for a bare symbol in
    a fixture file, and carries an empty scope when there is no `use`.
  - a `use x as y` alias resolves `y` → the aliased path.
  - `std::`/prelude names are NOT guessed (still bail).
- A PTY leg (if practical): in a real Cargo project, M-. on a bare
  `Deserialize` after `use serde::Deserialize;` lands in the serde registry
  source. Reuse the `drive_external_crate.py` pattern (own repo + own flock);
  if not practical, say so and rely on the unit tests plus an honest note.
- Live check in the real repo: M-. on a bare `ResolvedSource` (a name
  imported via `use redline_resolve::{...}`) and report what happens.

## Report format

The `SymbolContext` field + semantics chosen and why; the import-walk design
(bounded, where it lives); the provider changes (same-machinery proof); the
per-language support matrix (what resolves now vs still bails); test list
with discriminating tests called out; gate counts (honest); any `drive_xref`
leg that legitimately changed; skill corrections; deviations.

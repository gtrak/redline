# Task: plan 011 issue 08 — per-language golden-corpus suites for the providers

You are the implementation worker for ONE language lane (see LANE below).
Repo root is your cwd. Self-contained. `.agents/skills/*.md` are authoritative
ground truth.

## Origin (user directive)

The provider work deserves its own golden-corpus test suites: real sample
files written in each language (not fabricated minimal fixtures), exercised
through the provider pipeline, with expected outcomes snapshotted to
reviewable golden artifacts. A behavior change in a provider or in the
resolution seam shows up as a deliberate golden diff — never a silent
regression. This ALSO replaces most PTY-level provider testing: what a
corpus test can pin reliably at the unit/integration level should never
ride on PTY.

## Lane assignment (parallel — one worker per language, file-disjoint)

Each lane owns exactly its own files; the four lanes never touch a shared
file except this spec:

| LANE | arg | Corpus dir | Integration test |
|---|---|---|---|
| Rust | `rust` | `crates/redline-resolve/tests/corpus/rust/` | `crates/redline-resolve/tests/golden_rust.rs` |
| JS/TS | `js` | `crates/redline-resolve/tests/corpus/js/` | `crates/redline-resolve/tests/golden_js.rs` |
| Python | `python` | `crates/redline-resolve/tests/corpus/python/` | `crates/redline-resolve/tests/golden_python.rs` |
| Go | `go` | `crates/redline-resolve/tests/corpus/go/` | `crates/redline-resolve/tests/golden_go.rs` |

Commit only YOUR two paths (+ `crates/redline-resolve/Cargo.toml` ONLY if a
dev-dependency must be added — coordinate via the report; insta is already a
workspace dev-dep if usable from integration tests; prefer plain checked-in
`.golden` files to avoid adding deps).

## What to build (per language)

1. **Real sample files** in your corpus dir — a small but GENUINE project
   shape for the language, files that look like real code (multi-hundred-line
   samples where honest, drawn from your knowledge of idiomatic code; no
   LSP, no fetching from the network — WRITE the samples yourself, real
   language, real imports, real call sites):
   - Rust: a small cargo project (Cargo.toml + src/) exercising deep `::`
     chains, turbofish, generic args, use-aliases, prelude names
     (`Deserialize`-style bare use), module trees, `impl` blocks.
   - JS/TS: package.json + mixed `.js`/`.ts`/`.mjs` sources with named,
     default, namespace, `import type`, CJS `require`, and dotted call sites
     (`pkg.fn(...)`).
   - Python: a package with `__init__.py`, sibling modules, `from x import y
     as z` aliases, dotted use sites, stdlib-shaped imports (the provider
     shells to the REAL python3 for stdlib resolution — keep those legs).
   - Go: go.mod + sources with plain/aliased/grouped/dot imports and
     qualified use sites. The go toolchain is ABSENT in this sandbox — build
     the fixture as a LAYOUT (a module-cache-shaped directory tree the
     provider's injected-cache pattern accepts) and mark every go golden
     result honestly as layout-based, not live-toolchain.
2. **Probe spec per corpus**: a checked-in manifest (TOML or the golden
   format itself) listing probe points — file, byte/line-col (or symbol at
   a use site), the resolution input shape (path-shaped/bare) — and the
   expected outcome: `ResolvedSource` (file path shape, line, external flag)
   or the exact bail message (byte-for-byte). Deterministic probe walk;
   ONE pass over each file's parse where extraction is involved.
3. **Golden artifacts, reviewable**: expected outcomes checked in as
   `.golden` files (or insta snapshots). A provider/seam behavior change
   must surface as a conscious diff to the goldens, never silently. Include
   the honest-degradation cases IN the corpus (the bails are golden too —
   e.g. Rust bare-without-hint, JS relative-import, Python wildcard,
   Go ambiguous dot-import).
4. **Run discipline**: integration tests under the existing workspace gate
   (`cargo test --workspace` picks up `crates/redline-resolve/tests/*`).
   Real-toolchain legs (npm install/python3 shell-outs) must skip LOUDLY
   when the toolchain is absent (the established pattern); go never
   requires the toolchain (layout-based). No PTY in this issue at all.

## Explicit non-goals

- No PTY. No app/store.rs changes. No provider logic changes — the corpus
  OBSERVES the pipeline; if a corpus run exposes a REAL bug, report it as a
  finding (do not fix providers in this issue; the corpus pins current
  behavior, bugs get their own fix).
- No new runtime dependencies (insta only if integration-test compatible;
  otherwise plain .golden).

## Constraints

- Skills are truth. Write-first; compile early.
- Gate: `cargo test --workspace` green (integration tests run in it); if
  your lane is the last to land, `tools/gate.sh full`.
- Budget: land within ~40 tool calls; no new investigations after it
  compiles.
- Plain `git commit` (NOT `git add -A` — other lanes share this repo).

## Report

Corpus files (count, LOC) per language; probe points and outcome shapes
covered (landing/bail); which results are live-toolchain vs layout-based;
golden artifact format; honest counts from `cargo test --workspace`; any
REAL bug the corpus exposed (do not fix — report); deviations with reasons.

# 011 — Redline: resolver parity across languages

Status: 01+02+03 PASS+MERGED; 04 landed (1effdef, in review); 05 last
Phases: 2 · Issues: 01–05
Depends on: 007-03 (`SymbolContext` scope field) for 01; 007-01 (node-at-point)
for 03–04
Origin: user question 2026-09-19 — "do we have resolver parity across
languages?" Answer: no. Three providers are written and unit-tested but
**never registered**; the chain has no language dispatch; and every upstream
layer (node-at-point, scope hints, crate index) is Rust-only.

## Why

`M-.` on a path-shaped symbol in a JS/Python/Go project falls through to a
cargo-only chain and dies on "no Cargo.toml". Meanwhile
`crates/redline-resolve/src/providers/{js,python,go}_provider.rs` sit fully
implemented (880 / 714 / 1468 lines, ~70 tests that shell out to real
`npm`/`node`/`python3`/`go`). The value is written and unused.

Verified state (2026-09-19):

| Layer | State |
|---|---|
| Provider impls (js/python/go) | written + unit-tested (live shell-outs) |
| Chain registration | **Rust only** — `store.rs:7684` adds only `CargoProvider` |
| Language dispatch | **absent** — `resolve_traced` tries every provider, ignoring the trait's `languages()` |
| `SymbolContext` | **has no language field** — a provider cannot know what it is resolving |
| node-at-point / scope (007-01) | Rust only (deliberate; other languages return None) |
| Crate index (006-03) | `**/*.rs` only |
| PTY drives | Rust only |

## What

1. **Issue 01 — language dispatch + register the three providers.** Add a
   language to `SymbolContext` (or an explicit chain filter) and have
   `resolve_traced` consult the trait's existing `languages()` so only the
   matching provider(s) run. Register js/python/go in the app chain. This
   alone makes PATH-SHAPED symbols work (`lodash.map`, `os.path.join`,
   `github.com/x/y`), which is most of the practical value — no syntax work
   needed. CRITICAL: registering without dispatch would probe four toolchains
   per miss; the dispatch must land in the same issue.
2. **Issue 02 — per-language scope hints (pairs with 007-03).** Extend the
   007-03 scope/import hint beyond Rust: a bounded per-language import
   declaration walk (JS `import … from`, Python `import …` / `from … import`,
   Go `import` blocks) so a BARE symbol resolves. Degrade to the existing bail
   where unavailable.
3. **Issue 03 — node-at-point per language.** 007-01 is Rust-only by design;
   add the identifier predicates + scope walk for js/ts/python/go behind the
   same per-language extension point (`resolve_language`/`is_*_identifier_kind`).
   Each language is independently degradable.
4. **Issue 04 — per-language source index.** 006-03's crate index walks
   `**/*.rs`; generalize to the language's file extensions so M-./imenu inside
   a landed JS/Python/Go dependency work (the in-library follow-up the user
   asked for in 006-03, extended).
5. **Issue 05 — parity harness.** A PTY drive per language (own repo + own
   flock, the `drive_external_crate.py` pattern), plus an a11y-style
   "provider matrix" doc row: which language resolves path-shaped / bare /
   in-library today.

## Outcome record (live)

- **03 PASS + MERGED** (`6c02690`, reviewer PASS, merged `ab75247`): per-language node-at-point
  in `src/syntax/node.rs` — JS/TS(+TSX), Python, Go identifier predicates +
  enclosing-scope walks behind 007-01's extension point, whole-path rule per
  grammar (`member_expression` / `nested_type_identifier`→`nested_identifier` /
  `attribute` / `selector_expression`→`qualified_type`), Rust byte-identical
  (orchestrator-verified by fn extraction comparison). Grammar facts verified
  against the pinned `NODE_TYPES` via a throwaway probe, NOT from memory; two
  grammar-shape corrections vs the spec's own list (JS has no
  `type_identifier`/`nested_type_identifier`). 24 new tests; 672 workspace
  tests green; PTY suites deliberately skipped (no user-visible behavior, and
  the fixtures are shared with two live lanes).
- **01** in flight on main: dispatch verified live — `M-.` in a python buffer
  attempts exactly ONE provider (python), the cargo provider is never probed;
  new `tools/drive_issue_011_01.py` 3/3 on real python3.

- **Mapping honesty (011-01 review P2-4, deliberate UX change)**: registry
  languages with no provider (c/cpp/json/yaml/toml/sh/md) now bail
  "no tooling provider handles language `X`" instead of the pre-011-01 cargo
  probe. Correct per the contract — no wrong provider is ever probed — and
  the honest bail is more informative than a guaranteed cargo failure.
## Key decisions

- **Path-shaped first, bare second.** Issue 01 delivers value with zero syntax
  changes; bare symbols need 02+03. Sequence accordingly.
- **Dispatch by language, never by trial.** A miss must probe at most the
  providers whose `languages()` match the buffer's `LanguageId`.
- **Per-language, independently degradable.** One language landing must not
  regress another; each returns None/`[]` where unimplemented (007-01's
  pattern).
- **No LSP** (user ruled it out) and no new dependencies — the providers
  already shell out to the language's own tooling, which is the
  operator-sanctioned mechanism.
- Non-goals: macro expansion, generics inference, arbitrary-expression typing.

## Success criteria

- In a Node project, `M-.` on `lodash.map` lands in the installed package's
  source; Python `os.path.join` lands in the stdlib/venv source; Go
  `github.com/...` lands in the module cache.
- A bare imported symbol resolves via its import declaration (per language
  where implemented).
- An unknown/unsupported language still bails with an accurate message, and no
  miss probes more than the matching providers.

## Task order

| Phase | Issue | Depends on |
|---|---|---|
| 1 | 01 language dispatch + register | 007-03 |
| 1 | 02 per-language scope hints | 01, 007-03 |
| 2 | 03 node-at-point per language | 01 |
| 2 | 04 per-language source index | 01 |
| 2 | 05 parity harness + matrix doc | 01–04 |

When complete, archive per plan-process.

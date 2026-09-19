# 011 — Redline: resolver parity across languages (ARCHIVED)

Status: implemented (5/5 issues, reviewer PASS each; 2026-09-19/20)

## Scope delivered

Origin: user question — "do we have resolver parity across languages?"
Answer then: **no**. Three providers were written and unit-tested but never
registered; the chain had no language dispatch; every upstream layer
(node-at-point, scope hints, crate index) was Rust-only.

- **01 Language dispatch + registration** (`65111c7`): `SymbolContext.language:
  Option<String>` (lowercase, app-free); `resolve_traced` filters eligible
  providers by `languages()` — `None` ⇒ all eligible (byte-for-byte pre-011-01
  walk). Skipped providers leave NO trace entry (trace = real attempts only).
  App registers cargo → js → python → go and passes `resolution_language`
  (buffer extension → language; Plain → None). Registry languages with no
  provider (c/cpp/json/yaml/toml/sh/md) now bail honestly
  ("no tooling provider handles language `X`") instead of probing cargo.
- **02 Per-language scope hints** (`ab90231`): bounded per-language import
  walks in `resolver_scope_for` (JS/TS imports incl. `import type`, default,
  namespace, CJS `require`; Python module+enclosing-blocks, alias→original;
  Go dot-imports, exactly one). Relative Python imports bail honestly
  (sys.path ambiguity). New `scope_qualified_alias` for JS/TS namespace-member
  rewrites. Review P2-1 fix (`50aa315`): a 1-segment hint names the package
  ENTRY — bare `ns` from `import * as ns from "pkg"` lands on the entry file.
- **03 Per-language node-at-point** (`6c02690`, merged `ab75247`): JS/TS(+TSX),
  Python, Go identifier predicates + enclosing-scope walks behind 007-01's
  extension point; whole-path rule per grammar (outermost path container
  returned — structurally guaranteed, not pattern-matched); Rust
  byte-identical. Grammar facts verified against pinned `NODE_TYPES`, not
  memory (JS has no `type_identifier`/`nested_type_identifier` — TS-only).
- **04 Per-language source index** (`1effdef`): the 006-03 background index
  walks the owning language's extension set derived from the LANDED source
  file (Rust `rs`; JS/TS union `js/jsx/ts/tsx/mjs/cjs`; Python `py/pyi`; Go
  `go`); unknown language → empty set → no build (pre-011-04 end state,
  pinned). Extension sets live in `store.rs` (fence kept registry.rs
  untouched), pinned to the registry map by a round-trip test. Nested
  `node_modules` skipped post-root (the root may BE `node_modules/<pkg>`),
  consistent with the js provider walk; 2000-file refusal cap stays the
  backstop. `.pyi` included — PEP 561 stubs are first-class declaration
  surfaces (reviewer-judged correct; contrast the excluded machine-generated
  `.rsi`). Symbol extraction was already language-aware — only the
  file-selection predicate changed.
- **05 Parity lock-in** (`70fad9d`): `tools/drive_issue_011_05.py`
  (own repo/flock, gate-wired) — Python live 3 legs, JavaScript live 4 legs
  (incl. the 011-04 in-library cell, previously unit-only), Go SKIPPED with a
  loud banner (no toolchain; unit-covered only, 39 tests). The honest matrix:
  `docs/provider-matrix.md` — per language × (path-shaped / bare via import /
  in-library follow-up / blame), each cell works / degrades-to-bail /
  unit-covered-only, with the verification status per cell.

## The honest end state (the matrix's core finding)

M-. symbol extraction (`symbol_at_point`) extends only across `::`, so
**non-Rust path-shaped use sites bail at M-. today** (Python `json.dumps`,
JS `ns.member` reach the resolver as BARE identifiers; the providers' dotted
handling is unit-tested but unreachable from M-.). Bare-via-import and
in-library follow-up work live for Python and JS (verified on real
toolchains). Rust is fully live. Go is unit-covered only (no toolchain in
the sandbox; loud skip in every drive). Follow-up candidate: per-language
path-token extraction (the `.`-extension of `symbol_at_point`) — that would
activate the already-unit-tested provider dotted paths.

## Verification at archive

708 unit tests (606 root + 95 resolver lib + 7 integration); 17/17 PTY
suites per `tools/gate.sh full`; every "works (live)" matrix cell cites a
gate-battery leg; no cell mislabeled in either direction (reviewer-audited).

## Post-archive completion (011-06/07/08 — the carry-forwards, in flight)

- **07** PASS (`f13ece6`, merged `766ac9d`): walk sets for C (`c/h` — headers
  are definition sources), C++ (all six map keys), Markdown (`md/markdown/
  mdx` — atx only, the setext branch is dormant, pinned honestly);
  JSON/TOML/YAML/Bash stay empty (011-04 judgment carried, canary pinned).
  No C/C++/MD provider landing is app-reachable, so no PTY leg (reasoned
  deviation). Review PASS, 2 doc P2s fixed.
- **08 rust lane** PASS (`372f6a0`+fixes, merged `2049350`): 17-probe golden
  corpus, all live via real cargo metadata, 5 byte-exact bail goldens;
  observation pinned: path-shaped deep chains keep the second-segment rule
  (follow-up decision).
- **08 js lane** (in review): 18 probes; FOUND 3 REAL PROVIDER BUGS, pinned
  as goldens (relative specifiers collapse to base package `.` with
  empty-backtick messages; node_modules dir probed as a package; side-effect
  imports mis-split) — fixes get their own issues; corpus observes.
- **06** (in flight): language-aware symbol_at_point — legacy bail pins in
  011-01/011-05 drives superseded (sanctioned fence amendment, worker
  updating legs in place with the dispatch guarantee preserved).
- Go corpus lane queued.

## Carry-forwards

- Per-language path-token extraction at M-. (activates provider dotted
  handling) — the biggest remaining parity gap, now documented in
  `docs/provider-matrix.md`.
- Walk sets for C/C++/Markdown (definition queries exist; no index set yet).
- Local-shadowing wrong-hint class (all languages, accepted 007-03
  discipline — no local-binding tracking) — revisit if hints mature.
- Python module-vs-item corner (`from a import b` where b is a submodule)
  degrades to a bail in most real cases.
- Go multi-spec `type_declaration` groups report the first spec's name only
  (documented in node.rs; narrow — never affects function bodies).

## Process notes

- The 011-02 review survived 8 provider-stream deaths ("Stream ended without
  finish_reason") on `opencode-go/glm-5.3-flash`; a fresh reviewer on
  `local/local:high` completed on the first try. Lesson recorded: switch
  provider after repeated stream deaths on one provider rather than
  reviving again.
- Gate discipline held throughout: `--workspace` on build/clippy/test
  (556 → 708 tests over the plan), every new drive wired into
  `gate.sh SHARED_SUITES` (the 011-01 review P2 pattern).

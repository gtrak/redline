# Plan 017 — Import / module / alias resolution across languages

**Origin (user):** *"we should probably do similar work for all languages, yea? resolve imports, aliases,
modules?"* — after reporting that Clojure's `jwks/fetch-issuer-info` was split on hyphens
(`issue-language-aware-symbols`). That report is one instance of a general gap.

**Governing criterion (user):** *"ALL symbols should jump to the right definition"* — right file, right
symbol, right column. That criterion is what this plan exists to satisfy for languages that cannot reach
it today.

## The three layers (the organising idea)

Resolution today is **toolchain-dependent and available to 4 of 19 languages**
(`crates/redline-resolve/src/providers/`: `python` via `importlib.find_spec` in a subprocess, `js` via
`node_modules`, `go` via `go mod`, plus `cargo`). Everything else has nothing.

| Layer | What it is | Availability | Status |
|---|---|---|---|
| **1. Syntax** | What the grammar gives: the symbol/import/require forms and their parts | **all 19** (tree-sitter) | largely present |
| **2. Convention** | An import/alias/module name → **a file**, by that language's own published mapping rules | language-dependent, **no toolchain needed** | **mostly missing — this is the work** |
| **3. Tooling** | A real resolver: `find_spec`, `node_modules`, `go mod`, `cargo` | 4 languages | present |

Layer 2 is where the value is: it brings *right file, right symbol* to languages that will never have a
tooling provider (Clojure, Scheme, Ruby, Java, C#, C/C++, …), and — unlike layer 3 — it is **testable
without any toolchain**, which is the strongest kind of test this project can have.

**The layers compose, and they are consulted in order.** Tooling (3) is authoritative when available;
convention (2) covers the rest; syntax (1) is always the fallback for "which symbol is at the point".

## Doctrine that already applies

- **Flag, never guess.** When a name cannot be resolved, mark it unresolved rather than fabricating a
  target. A confident wrong jump is worse than an honest miss (landed in the annotation work as
  `orphaned`).
- **A test never vetoes a requirement**, and a resolution claim is only evidence if it was **executed**.
- **A rule comes from the language's grammar/reader/tooling — never from memory.** Every mapping in
  layer 2 must cite its source and be pinned by a test.
- **Both directions always**: a resolvable name lands on the right file/symbol, *and* an unresolvable one
  is flagged rather than mis-resolved. Over-eager resolution is its own defect (it was the shape of the
  `pip install df` P1).
- **No new side effects.** Layer 2 must be **pure and offline** — no installing, no network, no
  subprocess unless it is layer 3 by explicit consent (see the fetch-confirmation gate).

## Shape of the work

1. **The audit first** (`00-audit.md`) — every language × its import/alias/module forms × what resolves
   today × what would resolve by convention × the evidence. The audit's output is the **matrix**, and the
   matrix decides the order of everything after it. Do not guess the ordering.
2. **The mechanism, not per-language special cases** — extraction of the import/alias forms (tree-sitter
   queries, alongside the existing `flat_define`/`rust_tables_query` style), plus a **conventions table**
   (name → relative path) and the resolution order. A new language should be a table entry plus a query,
   not a new branch in `xref`.
3. **Per-language wiring, ranked by the audit** — starting where the user actually works (Clojure) and
   where the mapping is unambiguous and cheap to verify.
4. **The honest gap list** — languages whose resolution is genuinely undecidable statically (C++ ADL and
   templates, Ruby metaprogramming, dynamic `require`/`importlib`, Java without a source root). For each:
   state that it cannot be resolved, and make it **flag**, not guess.

## Non-goals

- **No LSP** (standing project decision). This is syntax + convention + the existing narrow tooling
  providers.
- **No new network or install path.** Any tooling consult keeps the landed confirmation gate.
- Not a general "find symbol by name across the project" search — this is *name → defining file*, driven
  by the import/alias structure.

## Status

- **00-audit** — OPEN (the entry point; nothing else should be ordered before it).
- Clojure alias resolution is being landed as part of `issue-language-aware-symbols` (the user's original
  report), which is this plan's first concrete instance and will feed the audit.

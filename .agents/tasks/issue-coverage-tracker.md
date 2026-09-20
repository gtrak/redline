# Task: language-coverage tracker (docs — the "what's covered, per language" record)

You are the implementation worker. Repo root is your cwd. Self-contained.

## Origin (user directive)

The user wants BASIC SUPPORT FOR MANY LANGUAGES with explicit tracking of
what is covered per language vs not — expecting more parallel provider
work. The information exists but is scattered (docs/provider-matrix.md is
provider-resolution-centric; capability facts live in different plan
records). This issue builds the single tracker.

## What to build

`docs/language-coverage.md` — language × capability grid over ALL 14
registry languages (Rust, TypeScript, Tsx, JavaScript, Python, Go, C, Cpp,
Toml, Json, Yaml, Bash, Markdown, Plain), each cell:
`works (live)` / `works (unit)` / `degrades to the bail` / `not implemented` /
`N/A (not a code language)` — with the evidence citation (test name,
drive leg, or doc line) per cell, following provider-matrix.md's
capability-terms discipline (a cell cannot be misread).

Capability columns (derive the final set by reading the code — this list
is the starting frame):
- **outline** (symbol extraction / definition query exists in queries.rs)
- **M-. path-shaped** (the 011-06 seam — requires node_at path containers)
- **M-. bare via import** (011-02 scope hints — provider-chain languages
  only)
- **scope walk** (node.rs `scope_path_at` — 011-03 machinery; which
  languages return None)
- **index walk set** (011-07's source_extensions_for — which languages
  build dependency indexes)
- **provider** (which chain provider, or "none — honest bail")
- **in-library follow-up** (crate index)
- **blame/edit/notes** (language-agnostic — one row/column noting that)
- **highlighting** (all 13 non-Plain have queries — verify)

Method: verify EVERY cell against code or a landed test/drive (grep + the
existing tests), the same standard the matrix reviews used. Mark cells
you could not verify as "unverified by worker — orchestrator spot-check".
Then add a **Gaps section**: the prioritized list of what's missing per
language (e.g. "C/C++: node predicates + scope walks → path-shaped M-.
unavailable" citing the unimplemented_languages_return_none test).

## Constraints

- DOCS ONLY — `docs/language-coverage.md` (+ one pointer line in
  `docs/provider-matrix.md`'s header and one in `README.md` if a natural
  spot exists). NO code changes. Parallel lanes are editing
  src/syntax/node.rs and (soon) other code — your fence is docs-only so
  you conflict with nobody.
- Budget ~25 tool calls. Commit to main (established style).

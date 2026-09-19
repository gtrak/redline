# Provider Matrix — the honest answer to "do we have resolver parity across languages?"

Plan 011, issue 05 (2026-09). Issues 01–04 made the non-Rust providers
reachable and per-language; this issue **locks it in** with drives
(`tools/drive_issue_011_05.py`, plus the 011-01/02/04 legs already in the
gate battery) and records the end state here. Every cell below was
checked against the actual code path or a live PTY leg at the commit that
added this file; the "verified" column says which.

## Capability terms (so the cells cannot be misread)

- **path-shaped** — M-. on a qualified use (`pkg.member`). 011-06 made
  the app's M-. extraction language-aware: in a non-Rust buffer the
  path token is the WHOLE dotted path when the point sits inside the
  language's path container (`json.dumps`, `ns.member`, `pkg.Fn` —
  011-03's `node_at` whole-path machinery, one parse), so the providers'
  dotted handling is reachable from M-.. Rust keeps `::` byte-for-byte
  (a Rust `.` field access still stays bare — fields are not in the
  index), and a parse failure / unsupported shape degrades to the
  exact pre-011-06 bare extraction (never a guess). The cells record
  what M-. does at a qualified use site, per language.
- **bare via import** — M-. on a bare identifier that an import statement
  in the current buffer binds (the 011-02 scope hints).
- **in-library follow-up** — after a landing, M-. on a symbol defined in
  ANOTHER file of the landed dependency (the 011-04 per-language crate
  index).
- **blame** — `b` (issue 08 git blame) on the current buffer.
- Cell values: **works** / **degrades to the bail** (the exact
  no-hint / no-provider message, byte-for-byte — never a wrong
  provider) / **unit-covered only** (no live verification possible in
  this sandbox).

## The matrix

| Capability | Rust | JS/TS | Python | Go |
|---|---|---|---|---|
| path-shaped | works (live) | **works (live)** (011-06) | **works (live)** (011-06) | unit-covered only |
| bare via import | works (live) | works (live) | works (live) | unit-covered only |
| in-library follow-up | works (live) | works (live) | works (live) | unit-covered only |
| blame, project file | works (live, U-F7) | works (git-based, language-agnostic) | works (git-based, language-agnostic) | works (git-based, language-agnostic) |
| blame, external landing | **degrades to the bail** (`no git history for external sources`) — same for every language, one code path (`store.rs::open_blame` external branch), live-pinned in the python leg | | | |

### Rust

- path-shaped: `drive_external_crate` L1 — `ropey::Rope` lands in the
  cargo registry source. Live in the gate battery.
- bare via import: `drive_external_use` L1 — `use serde::Deserialize;`
  → the registry `pub trait Deserialize`.
- in-library follow-up: `drive_external_crate` L4 — an in-crate M-. on
  `RopeBuilder` jumps cross-file with the crate-relative message.
- Pre-006-03/011 this column existed alone; 011-01 added the language
  dispatch so a Rust miss still probes cargo exactly once and never the
  other toolchains.

### JS/TS (verified live: node 24 + npm present; the drive builds a
hand-rolled `node_modules/fakelib` — no install runs)

- path-shaped: **works live from 011-06 (drive_issue_011_06 L-J1)** —
  `import * as fakelib` + `fakelib.apply(5)` + M-. on the use now lands
  on the `function apply` in the package's entry file: the language-aware
  extraction feeds the provider the dotted token `fakelib.apply`, and the
  provider's dotted handling locates the member. Pre-011-06 this degraded
  to the exact bail (the resolver got the bare `apply`; a namespace import
  hints only the entry name; the 011-02 walk never guesses a member's
  path — pinned live by 011-05 L-J1a). The namespace ENTRY landing is
  unchanged (011-05 L-J1b, still live).
- bare via import: works live (011-05 L-J2: `import { clamp }` →
  `util.js`). Named / default / alias / namespace / CJS-require shapes
  are unit-pinned in `store.rs` (`resolver_scope_js_*`).
- in-library follow-up: works live (011-05 L-J3: inside the landed
  `index.js`, M-. on `clamp` jumps in-crate to crate-relative
  `util.js:1` through the freshly built js-tree index — pre-011-04 this
  bailed to the resolver). Unit twin:
  `crate_index_builds_for_js_dependency_tree`.
- The miss probes ONLY the js provider (011-05 L-J1a: "tried 1
  provider(s): javascript").

### Python (verified live: python3 3.14.4 present)

- path-shaped: **works live from 011-06 (drive_issue_011_06 L-P1)** —
  plain `import json` + `json.dumps('x')` + M-. on the use now lands in
  the stdlib json source (`def dumps`): the language-aware extraction
  feeds the provider the dotted token `json.dumps`, which resolves on its
  OWN path (dotted symbols get no import-walk hint — 011-02/011-06
  interplay, unit-pinned in `store.rs`). Pre-011-06 this degraded to the
  exact no-hint bail (the resolver got the bare `dumps` with no hint —
  pinned live by 011-05 L-P2). The `os.path` frozen-fallback chain shape
  is unit-covered (`python_provider.rs`); the app-side deep-chain token
  (`os.path.join` → the whole path) is unit-pinned in `store.rs`.
- bare via import: works live (011-05 L-P1: `from json import dumps` →
  stdlib `def dumps`; first pinned by `drive_issue_011_02`).
  Module aliases (`import a.b as c`) and from-imports with aliases are
  unit-pinned in `store.rs` (`resolver_scope_python_*`); relative
  imports, wildcards, and prelude names (`print`) bail byte-for-byte
  (011-02 L2).
- in-library follow-up: works live (`drive_issue_011_04` L3: inside
  `json/__init__.py`, M-. on `JSONDecoder` jumps to crate-relative
  `json/decoder.py` through the python-tree index; the `indexing crate`
  indicator appears and clears — impossible pre-011-04, whose `.rs`-only
  walk found 0 files under `/usr/lib/python3.14`).
- The miss probes ONLY the python provider (011-01 live leg + 011-05
  L-P2: "tried 1 provider(s): python").

### Go (unit-covered ONLY — the `go` toolchain is ABSENT in this
sandbox)

- Every Go cell above is covered by `go_provider.rs`'s 39 unit tests
  (which shell out through overridable binaries / injected module
  caches, so they pass without a toolchain), the 011-02 Go import-walk
  unit pins (`resolver_scope_go_*`), and the 011-04 pure-tree-sitter Go
  walk/extraction tests. Since 011-06 the app-side token extraction is
  also unit-pinned (`symbol_at_point_dotted_path_extends_token_per_
  language`: `fmt.Println` / `fmt.Stringer` come back whole). **None is
  live-verified here.**
- `tools/drive_issue_011_05.py` prints a LOUD banner and records a SKIP
  for the go leg when `go` is missing — by design: no fixture is built
  (a fixture with no toolchain to run against would be a fake), and the
  gate never silently passes a language it could not check.

## Regression guard: a miss probes only the matching providers

- Unit (011-01, `crates/redline-resolve/src/lib.rs`):
  `language_dispatch_only_matching_provider_attempted` (probe spy +
  trace — a non-matching provider's `resolve` is never called),
  `python_context_never_reaches_cargo_provider` (real `CargoProvider` in
  the chain), `language_dispatch_no_matching_provider_errors` (zero
  probes), `all_miss_error_names_eligible_providers` (the all-miss
  message names only eligible providers), plus the unset-language
  backward-compat pin.
- Live: the pre-011-06 "tried 1 provider(s)" miss pins (011-05 L-P2 /
  L-J1a) were SUPERSEDED by 011-06 — those misses now land (the dotted
  token reaches the matching provider, whose landing is the stronger
  evidence: a wrong-provider probe would bail with that toolchain's own
  error, e.g. "no Cargo.toml" in a python project). The live dispatch
  pin on the MISS path now lives in `drive_issue_011_02` L2 (prelude
  `print` → "tried 1 provider(s): python", byte-for-byte).

## Deliberate non-goals (this is a feature list, not a bug list)

- **No LSP** (ruled out by the user; plan 011 decision).
- **No macro expansion, generics inference, or arbitrary-expression
  typing** (plan 011 non-goals).
- **App-level dotted tokens outside Rust**: CLOSED by 011-06 — M-.
  extraction is language-aware (the path token is the whole dotted path
  in a non-Rust buffer's path container; byte-for-byte bare otherwise;
  Rust `::` unchanged). The live cells are the python + js legs of
  `drive_issue_011_06.py`; Go is unit-covered only (toolchain absent).
- **Languages with no provider** (C/C++, JSON, YAML, TOML, shell,
  Markdown, …): the dispatch bails honestly — "no tooling provider
  handles language `X`" (011-01 mapping honesty) — instead of probing
  cargo.

## Drives (all in `tools/gate.sh` SHARED_SUITES, `timeout`-wrapped,
per-repo flock)

| Drive | Legs | Toolchain |
|---|---|---|
| `drive_issue_011_01.py` | python buffer attempts exactly ONE provider; cargo never probed | python3 |
| `drive_issue_011_02.py` | bare `dumps` lands; prelude `print` bails byte-for-byte | python3 |
| `drive_issue_011_04.py` | python stdlib tree IS indexed; in-crate M-. answers | python3 |
| `drive_issue_011_05.py` | python: L-P1 bare-import landing, L-P3 external blame bail, L-P2 path-shaped degrade + dispatch pin · js: L-J1a ns.member bail + dispatch pin, L-J1b namespace-entry landing, L-J3 in-crate follow-up, L-J2 bare named-import landing · go: LOUD skip when absent | python3, node+npm (go: absent → skip) |
| `drive_issue_011_06.py` | the two CHANGED path-shaped cells, live: L-P1 dotted `json.dumps` lands in the stdlib json source · L-J1 dotted `fakelib.apply` lands in the package entry file · loud per-runtime skip when absent (no go leg — unit-covered) | python3, node+npm |
| `drive_external_crate.py` / `drive_external_use.py` | the Rust column (registry landing, in-crate jump, bare `use` landing) | cargo |

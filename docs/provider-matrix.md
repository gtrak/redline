# Provider Matrix — the honest answer to "do we have resolver parity across languages?"

Plan 011, issue 05 (2026-09); 011-07 extends it with the C / C++ /
Markdown columns (walk sets; no new providers). Issues 01–04 made the
non-Rust providers reachable and per-language; issue 05 **locked it
in** with drives (`tools/drive_issue_011_05.py`, plus the 011-01/02/04
legs already in the gate battery) and recorded the end state here.
Every cell below was checked against the actual code path or a live
PTY leg at the commit that added this file; the "verified" column says
which. The per-language × capability grid over all 14 registry
languages lives in `docs/language-coverage.md` (sibling tracker).

## Capability terms (so the cells cannot be misread)

- **path-shaped** — M-. on a qualified use (`pkg.member`). 011-06 made
  the app's M-. extraction language-aware: in a non-Rust buffer the
  path token is the WHOLE dotted path when the point sits inside the
  language's path container (`json.dumps`, `ns.member`, `pkg.Fn` —
  011-03's `node_at` whole-path machinery, one parse), so the providers'
  dotted handling is reachable from M-.. Rust keeps `::` byte-for-byte,
  with ONE `.` exception (010-01, plan 010 Shape A rung 1): the `self.`
  receiver — `self.<member>` inside an `impl` resolves against the
  lexically enclosing impl's self type BEFORE the name-keyed index lookup
  (field → the struct's field line; method → the impl method's line;
  same-file first, then the index's cross-file locations). Every other
  Rust `.` access stays bare (fields of arbitrary receivers are not in
  the index), and a generic impl (`impl<T> Foo<T>`) / no enclosing impl /
  unknown member degrades to the exact pre-010-01 bare behavior (never a guess; a whole-path upgrade
  additionally requires EVERY dot-delimited segment to be a bare
  identifier — wrong-container shapes like `a?.b`, `foo().bar`, `(*p).field`
  degrade to the bare token rather than feeding a non-path token to a
  provider). The cells record what M-. does at a qualified use site, per
  language.
  (The `self.` exception is a resolution pre-step, not a provider
  matter: it consumes the 010-01 per-file Rust tables — struct fields +
  impl methods, built in the same index pass as the outlines — and is
  recorded in the Rust section below, not in the provider cells. 010-03,
  rung 3, adds a SECOND Rust `.` pre-step — `x.<member>` when the local
  binding `x` has a WRITTEN-DOWN type in the enclosing scope — which is
  even more strictly a resolution matter: it scans the line itself and
  the extraction's bare path token is NEVER changed, so a miss is
  byte-for-byte the pre-010-03 behavior; also recorded in the Rust
  section below, not in the provider cells.)
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

| Capability | Rust | JS/TS | Python | Go | C | C++ | Markdown |
|---|---|---|---|---|---|---|---|
| path-shaped | works (live) | **works (live, JS leg only)** (011-06; TS unit-covered — no live TS leg) | **works (live)** (011-06; 011-08 alias rewrite unit-pinned) | unit-covered only (incl. 011-08 alias rewrite) | **degrades to the bail** (unit) | **degrades to the bail** (unit) | **degrades to the bail** (unit) |
| bare via import | works (live) | works (live, JS leg only; TS unit-covered) | works (live) | unit-covered only | **degrades to the bail** (unit) | **degrades to the bail** (unit) | **degrades to the bail** (unit) |
| in-library follow-up | unit-covered only (demoted L4) | works (live, JS leg only; TS unit-covered) | works (live) | unit-covered only | unit-covered only (011-07) | unit-covered only (011-07) | unit-covered only (011-07) |
| blame, project file | works (live, U-F7) | works (git-based, language-agnostic) | works (git-based, language-agnostic) | works (git-based, language-agnostic) | works (git-based, language-agnostic) | works (git-based, language-agnostic) | works (git-based, language-agnostic) |
| blame, external landing | **degrades to the bail** (`no git history for external sources`) — same for every language, one code path (`store.rs::open_blame` external branch), live-pinned in the python leg | | | | | | |

### Rust

- self-receiver resolution (010-01, plan 010 Shape A rung 1): M-. on
  `self.<member>` inside `impl Foo` resolves `<member>` against `Foo` —
  a struct field lands on its `field_declaration` line (same file first,
  then the index's cross-file locations — in-project AND the per-crate
  indexes of external landings, one code path); an impl method lands on
  its `fn` line. The per-file tables (struct fields, impl methods with
  their impl kind — inherent vs the trait's full path) build in the SAME
  rayon index pass as the symbol outlines (the tables query runs on the
  same tree — zero extra parse cost; `queries::extract_all`), so a
  reparse refreshes both. Honest degradation everywhere: a generic self
  type (`impl<T> Foo<T>`), a path/qualified self type, a missing
  enclosing impl, a member the tables don't record, a macro-built impl,
  a deref chain, or a non-Rust buffer → the exact pre-010-01 behavior
  (bare member → index / enclosing-symbol / resolver fall-through) —
  never a guess. The self type is resolved LEXICALLY (innermost
  enclosing `impl_item`), so no expression typing is involved. find-
  implementations (who impls `Trait`?) is Rung 4's read-only view of the
  same table — the `ImplKind::Trait` + impl line are already stored; the
  picker command itself is deferred (out of this issue's scope fence).
  Unit-pinned in `store.rs` (`xref_self_*` — same-file field, cross-file
  field, method, ambiguous member picker, generic-impl + no-impl
  degradation) and `queries.rs` / `nav/index.rs` (the tables themselves).
- local-binding resolution (010-03, plan 010 Shape A rung 3): M-. on
  `x.<member>` / `x.<member>()` resolves `<member>` against the local
  binding `x`'s WRITTEN-DOWN type when one exists in the enclosing
  scope — a `let x: Type` annotation (bare identifier only) or a
  `let x = Type { … }` struct-literal RHS — through the SAME field /
  method tables as the self pre-step (fields cross-file via the index's
  field map, methods same-file via the impl table; same-file first,
  ambiguous members → the picker). The per-file binding map (scope
  range → (binding, type, line, let byte)) builds in the SAME rayon
  index pass (the table query runs on the same tree — zero extra parse
  cost; `queries::extract_all`). Scoping is LEXICAL over the current
  buffer's enclosing `block` chain (innermost first — a fn body IS a
  `block`, as are closure / loop / arm / nested-block bodies): the
  innermost scope with a yet-active binding wins, and within a scope
  the last `let` before the use — shadow rule. `let mut x: T` is the
  same as `let x: T`. Degradation, pinned per case: unannotated
  bindings (never INFERRED — that's the compiler's job), generic /
  path-shaped annotations (`Vec<T>`, `std::path::PathBuf`), non-struct
  types and type aliases (a `type P = …` alias's name simply isn't in
  the field map — a natural miss, no alias following), `&T { … }` /
  `T::<u8> { … }` literals, expression receivers (`(e).f`, `a[0].f`),
  `::`-path receivers, dot-chained receivers (`a.b.f` — the middle
  segment is a field access, never a local binding), pattern bindings
  (`if let` / `while let` / match arms — not `let_declaration`s),
  shadowed-out names, and non-Rust buffers →
  byte-for-byte today's bare behavior (the extraction's path token is
  never changed for these — the pre-step owns its own receiver scan).
  The pre-step runs in the project AND the external-crate M-. paths
  (uniform by construction, as the self pre-step). Unit-pinned in
  `store.rs` (`xref_local_binding_*` — same-file + cross-file field,
  struct-literal RHS, mut, ambiguous-impls narrowing, shadow,
  unannotated + non-struct degradation pins; `rust_dotted_receiver_*`
  the scan pin, dot-chained receivers included), `queries.rs`
  (`rust_binding_type_at_*` — the innermost-wins / shadow /
  string-literal misses; the `&T { … }` / `T::<u8> { … }` literal misses
  pinned at table level in `rust_tables_extract_local_bindings`), and
  `nav/index.rs` (the 010-01-review-P1 mirror: a bindings-only file's
  same-content refresh keeps its table).
- path-shaped: `drive_external_crate` L1 — `ropey::Rope` lands in the
  cargo registry source. Live in the gate battery.
- bare via import: `drive_external_use` L1 — `use serde::Deserialize;`
  → the registry `pub trait Deserialize`.
- in-library follow-up: `drive_external_crate` L4 — **demoted to a unit**:
  `unit_flow_ext_crate_in_crate_mdot` (src/app/flow_tests.rs:2830) pins
  the in-crate M-. on `RopeBuilder` jumping cross-file with the
  crate-relative message; the drive now runs L1 only (the registry
  landing).
- Pre-006-03/011 this column existed alone; 011-01 added the language
  dispatch so a Rust miss still probes cargo exactly once and never the
  other toolchains.

### JS/TS (verified live: node 24 + npm present; the drive builds a
hand-rolled `node_modules/fakelib` — no install runs). **TS/Tsx tier:**
the live legs exercise JavaScript ONLY — every TypeScript/Tsx cell
rests on the shared JS machinery's unit pins (the
`LanguageId::TypeScript | Tsx` branches in `node.rs`/`store.rs`); no
live TS leg exists.

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
- relative import: **the app now EMITS the relative hint (011-08
  fix-jsrel P2-7 follow-up)** — `./…` / `../…` specifiers carry into the
  hint (item included for named / aliased / default / destructured
  bindings; the specifier alone for whole-module and namespace
  bindings), so M-. on a relative use site lands in the SIBLING file
  through the provider's relative branch: workspace-local
  (`external = false`), opened as an EDITABLE project buffer
  (`open_resolved_source`'s project branch — unit-pinned live in
  `store.rs`, `xref_relative_import_lands_in_sibling_file_editable`;
  the hint shapes in `resolver_scope_js_relative_*`). Absolute and
  side-effect shapes still hint nothing (the provider's dedicated
  bails — `resolver_scope_js_no_import_absolute_and_side_effect_are_
  not_guessed`). Provider-side landing goldens: the js corpus
  (`relative-import-whole-bail`, `relative-member-bail`,
  `live-relative-lands`).
- in-library follow-up: works live (011-05 L-J3: inside the landed
  `index.js`, M-. on `clamp` jumps in-crate to crate-relative
  `util.js:1` through the freshly built js-tree index — pre-011-04 this
  bailed to the resolver). Unit twin:
  `crate_index_builds_for_js_dependency_tree`.
- The miss path's live dispatch pin: 011-02 L2's python twin is the
  cross-language proof (same dispatch walk); the JS miss path itself is
  unit-pinned in the resolver crate (`language_dispatch_*`). 011-05
  L-J1a now pins the 011-06 LANDING (index.js:3).

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
  011-08: the ALIASED path-shaped use (`engine.torque` from
  `from gears import engine`, the app's hint naming the original import
  path) rewrites the alias to `gears.engine` and lands through the same
  `find_spec` machinery — unit-pinned
  (`aliased_dotted_use_rewrites_to_original_path` + its no-op pins);
  the live cell above is the non-aliased shape.
- bare via import: works live (011-05 L-P1: `from json import dumps` →
  stdlib `def dumps`; first pinned by `drive_issue_011_02`).
  Module aliases (`import a.b as c`) and from-imports with aliases are
  unit-pinned in `store.rs` (`resolver_scope_python_*`); relative
  imports, wildcards, and prelude names (`print`) bail byte-for-byte
  (011-02 L2).
- src/-layout workspace packages (011-08 py-roots): a PEP 621
  `src/`-layout project (`pyproject.toml` + sources under `src/`) is not
  importable from the CWD, so a CWD-based `find_spec` miss triggers
  honest discovery: walk up from the buffer's directory (bounded by the
  workspace root) for a `pyproject.toml`, and if the project carries a
  `src/` dir, re-probe `find_spec` with that dir on `sys.path` — the
  sys.path entry is derived from a FOUND file on disk, never a guess
  (no pyproject / no `src/` dir leaves the bail byte-for-byte,
  unit-pinned by `src_layout_without_pyproject_is_not_guessed`; the
  returned `src/` is canonicalized and must stay under the workspace —
  a symlinked-outside `src/` is a miss, not a discovery). Live-pinned
  by corpus probe 016 (`016-src-layout-workspace-package`, root
  `project_src` → `src/gears/engine.py:6`).
- in-library follow-up: works live (`drive_issue_011_04` L3: inside
  `json/__init__.py`, M-. on `JSONDecoder` jumps to crate-relative
  `json/decoder.py` through the python-tree index; the `indexing crate`
  indicator appears and clears — impossible pre-011-04, whose `.rs`-only
  walk found 0 files under `/usr/lib/python3.14`).
- The miss probes ONLY the python provider (011-01 live leg + 011-05
  live miss pin: 011-02 L2 `print` → "tried 1 provider(s): python" — the
  011-05 L-P2 leg now pins the 011-06 LANDING (the bare-miss bail it used
  to pin is superseded).

### Go (unit-covered ONLY — the `go` toolchain is ABSENT in this
sandbox)

- Every Go cell above is covered by `go_provider.rs`'s 41 unit tests
  (38 of them run without a toolchain — they shell out through
  overridable binaries / injected module caches, so they pass here —
  incl. the 011-08 path-
  shaped alias rewrite, `pe.Wrap` under `import pe "github.com/pkg/`
  `errors"`, pinned by
  `aliased_dot_qualified_use_rewrites_to_real_package` + its no-op
  twins; the other 3 are `#[ignore]`d live legs requiring the go
  toolchain, two of them + network: skipped, never silently passed),
  the 011-02 Go import-walk
  unit pins (`resolver_scope_go_*`), and the 011-04 pure-tree-sitter Go
  walk/extraction tests. Since 011-06 the app-side token extraction is
  also unit-pinned (`symbol_at_point_dotted_path_extends_token_per_
  language`: `fmt.Println` / `fmt.Stringer` come back whole). **None is
  live-verified here.**
- `tools/drive_issue_011_05.py` prints a LOUD banner and records a SKIP
  for the go leg when `go` is missing — by design: no fixture is built
  (a fixture with no toolchain to run against would be a fake), and the
  gate never silently passes a language it could not check.

### C / C++ (unit-covered ONLY — NO PROVIDER: no package-manager
machinery for these languages exists in this repo, and 011-07
deliberately invented none)

- **M-. from a project buffer degrades to the bail, byte-for-byte**: no
  tooling provider handles language `c` / `cpp`, so the dispatch emits
  011-01's honest message — `no tooling provider handles language `c``
  (`(N provider(s) registered, none attempted)`) — instead of probing
  cargo, **for symbols outside the project index: in-project definitions
  still jump through the project index** (the project walk has no
  extension filter). Same code path and message shape as the live-pinned
  python/js miss (011-02 L2 — the live dispatch pin on the miss path;
  011-05 L-P2 / L-J1a now pin the 011-06 LANDINGS); the language
  parameter is the
  only difference, so no live leg was added (the units are sufficient —
  see the "Drives" note below).
- **011-07 gave these languages their 011-04 walk sets**, derived from
  `registry.rs`'s extension map (the authority): C `c/h`, C++
  `cc/cpp/cxx/hh/hpp/hxx`. HEADERS ARE DEFINITION SOURCES: a landed C/C++
  tree indexes its `.h`/`.hpp`/`.hh` files alongside the implementation
  files (functions, structs, classes — pinned by
  `crate_index_builds_for_c_dependency_tree` /
  `crate_index_builds_for_cpp_dependency_tree`, incl. header symbols and
  a landing on the `.hpp` itself). The pre-011-07 end state (empty set →
  walk finds 0 files → no index, silent) is gone for these languages.
- **in-library follow-up is unit-covered only — and app-unreachable
  today**: the follow-up lookup (`xref_candidates` against the owning
  crate's index) is language-agnostic and live-pinned for python/js, and
  the C/C++ index side is unit-pinned (above). But no provider can ever
  LAND in a C/C++ dependency, so the live app cannot produce such a
  landing; the cell is the honest “would answer if a landing existed”
  state, not a live-verified jump.
- **Superseded: the 011-03 degradation.** The lang-pred lane landed the
  C/C++ node predicates + scope walks (`c_member_path_comes_back_whole`,
  `cpp_member_path_comes_back_whole`,
  `cpp_qualified_path_comes_back_whole`,
  `c_scope_chain_struct_in_struct_and_function`,
  `cpp_scope_chain_namespace_class_method` — `src/syntax/node.rs`), so
  the per-language `node_at`/scope walks no longer stay `None` for
  C/C++. What still stands from that bullet: import-based bare-symbol
  hints do not apply — there is no import machinery for C to rebuild
  qualified paths from — and the app-side whole-path upgrade
  (`store.rs::dotted_path_container`) does not enumerate the C/C++
  containers, so M-. extraction stays at the bare segment even though
  the syntax layer answers the whole path (unit-pinned; see
  `docs/language-coverage.md`, "Known corners").

### Markdown (unit-covered ONLY — NO PROVIDER; M-. targets are ATX +
SETEXT headings, verified, not assumed)

- **M-. from a project buffer degrades to the bail** (same 011-01
  dispatch: `no tooling provider handles language `markdown``).
- **011-07 walk set**: `md/markdown/mdx` (the full registry map). The
  definition query captures `#`/`##`/… atx headings AND `Title\n====`
  setext headings as `Heading` symbols, so a landed markdown tree's
  headings become M-. targets through the crate index
  (`crate_index_builds_for_markdown_dependency_tree`).
- **HONEST SCOPE — verified, not assumed**: both heading kinds extract
  (`# Alpha` → `Alpha`, `Delta\n====` → `Delta`); the setext branch's
  node shape is `setext_heading → paragraph → inline` (verified against
  the pinned tree-sitter-md 0.3.2 grammar — its first draft was
  structurally unmatchable and fixed in the 011-07 follow-up). No
  paragraphs, no links. Pinned by `setext_markdown_file_contributes_heading_symbols`
  and the present `docs/setext.md` entry in the e2e test. **The e2e
  alone would not catch a LOOSE-PARAGRAPH regression** (a query that
  started matching plain paragraphs would still land the heading set in
  the e2e's dependency tree): the unit pin is the extraction-shape
  guard.
- **in-library follow-up**: unit-covered only (same app-unreachable
  reason as C/C++: no provider can land in a markdown dependency).

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
  cargo. 011-07 split this class: C, C++, and Markdown now have 011-04
  walk sets (a landed tree would index; M-. from a project buffer still
  bails — unit-pinned, see their sections), while JSON/TOML/YAML/Bash
  keep 011-04's judgment that their outline queries are not M-. source
  walks (data/config/shell files stay out of the index walk).

## Drives (all in `tools/gate.sh` SHARED_SUITES, `timeout`-wrapped,
per-repo flock)

| Drive | Legs | Toolchain |
|---|---|---|
| `drive_issue_011_01.py` | python buffer attempts exactly ONE provider; cargo never probed | python3 |
| `drive_issue_011_02.py` | bare `dumps` lands; prelude `print` bails byte-for-byte (L2 = the live dispatch pin on the miss path) | python3 |
| `drive_issue_011_04.py` | python stdlib tree IS indexed; in-crate M-. answers | python3 |
| `drive_issue_011_05.py` | python: L-P1 bare-import landing, L-P3 external blame bail, L-P2 path-shaped LANDING pin (011-06: json/__init__.py:185, was the bail) · js: L-J1a ns.member LANDING pin (011-06: index.js:3, was the bail), L-J1b namespace-entry landing, L-J3 in-crate follow-up, L-J2 bare named-import landing · go: LOUD skip when absent | python3, node+npm (go: absent → skip) |
| `drive_issue_011_06.py` | the two CHANGED path-shaped cells, live: L-P1 dotted `json.dumps` lands in the stdlib json source · L-J1 dotted `fakelib.apply` lands in the package entry file · loud per-runtime skip when absent (no go leg — unit-covered) | python3, node+npm |
| `drive_external_crate.py` / `drive_external_use.py` | the Rust column live: registry landing (L1) + bare `use` landing (L1); the in-crate jump (ex-L4) is now unit-pinned (`unit_flow_ext_crate_in_crate_mdot`, flow_tests.rs:2830), not a drive leg | cargo |

No 011-07 drive: a live C/C++/Markdown landing is IMPOSSIBLE in this
app (no provider can resolve a symbol into those files — the landing
that would trigger `start_crate_indexing` never happens), and the one
observable live behavior — the honest no-provider bail — shares 011-01's
language-parameterized dispatch with the live-pinned python/js misses,
so a live leg would re-prove the same code path. The 011-07 evidence is
the unit set: the extended round-trip test + the three e2e
`start_crate_indexing` tests (C, C++, Markdown) + the setext extraction
pin. (For the same reason, a C *toolchain* is not needed by anything in
this issue — the fixture never runs, it only parses.)

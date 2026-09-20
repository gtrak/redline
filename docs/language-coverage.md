# Language Coverage — what redline covers, per language

The single "what's covered, per language" record. Sibling of
`docs/provider-matrix.md` (which is provider-resolution-centric): this
file is the **language × capability** grid over ALL 14 registry
languages (`LanguageId::ALL` + `Plain`, `src/syntax/registry.rs`):
Rust, TypeScript, Tsx, JavaScript, Python, Go, C, Cpp, Toml, Json,
Yaml, Bash, Markdown, Plain.

## Verification method (same standard the matrix reviews used)

Every cell below was checked by the worker against the **code at this
commit** (grep + read) or a **landed unit test / live drive leg**, and
the evidence is cited in-cell: a test name, a drive leg, or a
source line. Capability terms follow `provider-matrix.md` so no cell
can be misread:

- **works (live)** — pinned by a drive leg in `tools/gate.sh`'s
  battery (a PTY run in this sandbox).
- **works (unit)** — pinned by a landed unit test; no live leg exists
  for this cell (either none was added, or the toolchain is absent in
  the sandbox).
- **degrades to the bail** — M-. emits the exact 011-01
  no-hint / no-provider message, byte-for-byte; never a wrong
  provider.
- **not implemented** — the machinery exists in the code but returns
  its graceful-degradation value for this language (`None` / `[]` /
  empty set).
- **N/A (not a code language)** — no grammar, query, or provider;
  plain-text fallback.
- **not applicable** — the capability cannot apply to this language
  (e.g. an empty index-walk set and no provider that can land, so
  in-library follow-up has no reachable trigger).

Cells the worker could NOT verify are marked
`unverified by worker — orchestrator spot-check`. Three cells carry
that marker (TS/Tsx ride the shared JS machinery, whose unit pins are
cited alongside): the TypeScript in-library TS-specific landing, the
Tsx outline TSX-specific extraction test name, and the Tsx bare
Tsx-specific test name.

The registry facts (extension map, grammar + highlight configs) are
pinned by `registry.rs` tests: `ext_map_covers_all_11_languages`,
`unknown_extension_falls_back_to_plain`, `all_grammars_have_configs`,
`language_for_uses_extension`.

## The grid

Capability columns, in the code's terms:

- **outline** — symbol extraction / definition query exists
  (`src/syntax/queries.rs::query_for`); the project index is built
  from these queries (the project walk has no extension filter).
- **M-. path-shaped** — M-. on a qualified use (`pkg.member`):
  011-06 language-aware extraction requires 011-03's `node_at`
  path-container machinery (`src/syntax/node.rs`).
- **M-. bare via import** — M-. on a bare identifier an import in the
  buffer binds: 011-02 scope hints
  (`store.rs::resolver_scope_for` — provider-chain languages only).
- **scope walk** — `node.rs::scope_path_at` (011-03); non-`[]` only
  for Rust / JS / TS / Tsx / Python / Go.
- **index walk set** — 011-07's `store.rs::source_extensions_for`:
  which languages build dependency (crate) indexes for a landed tree.
- **provider** — which chain provider handles misses
  (`store.rs` resolver chain: `CargoProvider`, `JsProvider`,
  `PythonProvider`, `GoProvider`); "none — honest bail" = 011-01's
  `no tooling provider handles language \`X\``.
- **in-library follow-up** — after a landing, M-. on a symbol defined
  in ANOTHER file of the landed dependency (011-04 per-language crate
  index; the `xref_candidates` lookup itself is language-agnostic).
- **highlighting** — a `HighlightConfiguration` + highlight query is
  built for the language at startup.

| Language | outline | M-. path-shaped | M-. bare via import | scope walk | index walk set | provider | in-library follow-up | highlighting |
|---|---|---|---|---|---|---|---|---|
| **Rust** | works (live) — `drive_external_crate` L1 (registry landing); unit `rust_extracts_fn_method_struct_const` (queries.rs) | works (live) — `drive_external_crate` L1 `ropey::Rope` | works (live) — `drive_external_use` L1 (`use serde::Deserialize`) | works (unit) — `scope_path_inside_impl_fn`, `scope_path_includes_nested_mods`, `generic_impl_scope_is_the_bare_type_name`, `trait_signature_item_contributes_scope` (node.rs) | `rs` — `source_extensions_for` (store.rs); `source_extensions_round_trip_through_registry_map` | `CargoProvider` (`cargo.rs::languages() = ["rust"]`) | works (unit) — `unit_flow_ext_crate_in_crate_mdot` (flow_tests.rs:2830; the demoted L4 in-crate `RopeBuilder` jump — `drive_external_crate` now runs L1 only) | works — `all_grammars_have_configs` (registry.rs) |
| **TypeScript** | works (unit) — `ts_extracts_interface_and_method`, `ts_extracts_exported_constants` (queries.rs) | works (unit) — `ts_member_path_comes_back_whole`, `ts_nested_type_identifier_comes_back_whole` (node.rs); `symbol_at_point_dotted_path_extends_token_per_language` (store.rs: "TS gets the same answer"); no live TS leg (the JS drive is JavaScript-only) | works (unit) — `resolver_scope_ts_import_type_carries_package_path` (store.rs); live leg is JS-only (011-05 L-J2) | works (unit) — `ts_scope_chain_function_class_method_arrow` (node.rs, loops TS+Tsx) | `js/jsx/ts/tsx/mjs/cjs` — `source_extensions_for` | `JsProvider` (`js_provider.rs::languages() = ["javascript","typescript","tsx","jsx"]`) | works (unit) — shares the js-tree index (`crate_index_builds_for_js_dependency_tree`); TS-specific landing: unverified by worker — orchestrator spot-check | works — `all_grammars_have_configs` |
| **Tsx** | works (unit) — shares `TYPESCRIPT_QUERY` (`query_for`, queries.rs:251); TSX-specific extraction test name: unverified by worker — orchestrator spot-check | works (unit) — same shared machinery; node.rs ts tests loop TS+Tsx (`ts_member_path_comes_back_whole`); no live leg | works (unit) — `js_ts_scope_for` covers Tsx (`resolver_scope_for`, store.rs:8173-8176; `js_ts_scope_for` at store.rs:8441); no Tsx-specific test name verified | works (unit) — `ts_scope_chain_function_class_method_arrow` (loops TS+Tsx) | `js/jsx/ts/tsx/mjs/cjs` (shared with JS/TS) | `JsProvider` (`"tsx"` in `languages()`) | works (unit) — same shared js-tree index; landing leg: unverified by worker — orchestrator spot-check | works — `all_grammars_have_configs` |
| **JavaScript** | works (unit) — `js_extracts_exported_constants` (queries.rs); live landings go through the provider's own walk (011-05 L-J1b) | works (live) — `drive_issue_011_06` L-J1 (`fakelib.apply` → entry file) | works (live) — `drive_issue_011_05` L-J2 (`import { clamp }`); relative-import shape unit-pinned (`resolver_scope_js_relative_*`) | works (unit) — `js_member_path_comes_back_whole`, `js_scope_chain_function_class_method_arrow` (node.rs) | `js/jsx/ts/tsx/mjs/cjs` — `source_extensions_for` | `JsProvider` | works (live) — `drive_issue_011_05` L-J3; unit `crate_index_builds_for_js_dependency_tree` | works — `all_grammars_have_configs` |
| **Python** | works (unit) — `python_extracts_class_and_def` (queries.rs); `crate_index_builds_for_python_dependency_tree` (store.rs) | works (live) — `drive_issue_011_06` L-P1 (`json.dumps` → stdlib); alias rewrite unit-pinned (`aliased_dotted_use_rewrites_to_original_path`) | works (live) — `drive_issue_011_05` L-P1 / `drive_issue_011_02` L1; alias/wildcard bails unit-pinned (`resolver_scope_python_*`) | works (unit) — `python_scope_path` (node.rs:389) + scope pins `python_scope_chain_function_in_class`, `python_boundary_offsets_do_not_panic` (node.rs) | `py/pyi` — `source_extensions_for` | `PythonProvider` (`python_provider.rs::languages() = ["python"]`) | works (live) — `drive_issue_011_04` L3 (`JSONDecoder` → `json/decoder.py`); unit `crate_index_builds_for_python_dependency_tree` | works — `all_grammars_have_configs` |
| **Go** | works (unit) — `go_extracts_func_and_type` (queries.rs); go_provider.rs unit set (38 run without a toolchain) | works (unit) — `symbol_at_point_dotted_path_extends_token_per_language` (`fmt.Println` / `fmt.Stringer`); `aliased_dot_qualified_use_rewrites_to_real_package` (go_provider.rs) | works (unit) — `resolver_scope_go_dot_import_carries_package_name` + not-guessed pins (store.rs) | works (unit) — `go_scope_path` + `go_scope_chain_method_and_function` (node.rs:412, :911) | `go` — `source_extensions_for` | `GoProvider` — present in the chain but **not live-verified here** (`go` toolchain absent; 3 `#[ignore]`d live legs) | works (unit) — 011-04 pure-tree-sitter Go walk/extraction (go_provider.rs / store.rs); no live leg | works — `all_grammars_have_configs` |
| **C** | works (unit) — `c_extracts_function_and_struct` (queries.rs); `crate_index_builds_for_c_dependency_tree` (incl. `.h`) | **degrades to the bail** — `node_at` → `None` (no path container; `unimplemented_languages_return_none`, node.rs:593) → bare token → no provider → `no tooling provider handles language \`c\`` (011-01) | **degrades to the bail** — `resolver_scope_for` → `Vec::new()` (no hint, store.rs:8184); same message; pinned live for the identical dispatch by 011-02 L2 (`drive_issue_011_02.py:112-122`: prelude `print` → the exact no-hint bail — 011-05 L-P2/L-J1a now pin the 011-06 LANDINGS) | **not implemented** — `scope_path_for` `_ => Vec::new()` (node.rs:350); `unimplemented_languages_return_none` | `c/h` (headers ARE definition sources — 011-07) — `source_extensions_for` | **none — honest bail** (no C provider; 011-07 deliberately invented none) | works (unit) — index side unit-pinned; **app-unreachable**: no provider can ever land in a C dependency | works — `all_grammars_have_configs` |
| **Cpp** | works (unit) — `cpp_extracts_function_class_struct` (queries.rs); `crate_index_builds_for_cpp_dependency_tree` (incl. `.hpp`/`.hh`) | **degrades to the bail** — same as C (`unimplemented_languages_return_none` + no provider → 011-01 message) | **degrades to the bail** — same as C (no hint) | **not implemented** — `unimplemented_languages_return_none` | `cc/cpp/cxx/hh/hpp/hxx` (full registry map — 011-07) — `source_extensions_for` | **none — honest bail** | works (unit) — same app-unreachable note as C | works — `all_grammars_have_configs` |
| **Toml** | works (unit) — `toml_extracts_keys` (queries.rs) | **degrades to the bail** — `no tooling provider handles language \`toml\`` (011-01); `node_at` → `None` | **degrades to the bail** — no import machinery; `resolver_scope_for` → `Vec::new()` | **not implemented** — `unimplemented_languages_return_none` | **none** (empty set — 011-04 judgment carried by 011-07: config files are not M-. definition sources; asserted empty by `source_extensions_round_trip_through_registry_map`) | **none — honest bail** | **not applicable** — no index builds (empty walk set) and no provider can land | works — `all_grammars_have_configs` |
| **Json** | works (unit) — `json_extracts_keys` (queries.rs) | **degrades to the bail** — `no tooling provider handles language \`json\`` (011-01) | **degrades to the bail** — no import machinery | **not implemented** — `unimplemented_languages_return_none` | **none** (empty set — same 011-04/011-07 judgment) | **none — honest bail** | **not applicable** — no index builds; no provider can land | works — `all_grammars_have_configs` |
| **Yaml** | works (unit) — `yaml_extracts_keys` (queries.rs) | **degrades to the bail** — `no tooling provider handles language \`yaml\`` (011-01) | **degrades to the bail** — no import machinery | **not implemented** — `unimplemented_languages_return_none` | **none** (empty set — same judgment) | **none — honest bail** | **not applicable** — no index builds; no provider can land | works — `all_grammars_have_configs` |
| **Bash** | works (unit) — `bash_extracts_functions` (queries.rs) | **degrades to the bail** — `no tooling provider handles language \`bash\`` (011-01) | **degrades to the bail** — no import machinery | **not implemented** — `unimplemented_languages_return_none` | **none** (empty set — same judgment) | **none — honest bail** | **not applicable** — no index builds; no provider can land | works — `all_grammars_have_configs` |
| **Markdown** | works (unit) — `markdown_extracts_headings`, `setext_markdown_file_contributes_heading_symbols` (ATX + SETEXT headings only — verified scope, no paragraphs/links) | **degrades to the bail** — `no tooling provider handles language \`markdown\`` (011-01); `node_at` → `None` | **degrades to the bail** — no import machinery; `resolver_scope_for` → `Vec::new()` | **not implemented** — `unimplemented_languages_return_none` | `md/markdown/mdx` (headings only — 011-07) — `source_extensions_for` | **none — honest bail** | works (unit) — `crate_index_builds_for_markdown_dependency_tree`; **app-unreachable**: no provider can land | works — `all_grammars_have_configs` |
| **Plain** | N/A (not a code language) — no grammar, `query_for` → `None` (queries.rs:262) | N/A (not a code language) — no path container; the miss path is the 011-01 unset-language backward-compat walk, pinned by `language_dispatch_unset_tries_all_in_order` (lib.rs:575) | N/A (not a code language) — `resolver_scope_for` → `Vec::new()` | N/A (not a code language) — `scope_path_at` → `[]` | **none** (empty set — asserted by `source_extensions_round_trip_through_registry_map`) | none — the unset-language dispatch walks the whole chain (backward-compat pin above) | **not applicable** | N/A (not a code language) — `LanguageId::Plain => None` (registry.rs); `all_grammars_have_configs` asserts it |

**blame / light editing / notes (language-agnostic — one row for all 14
languages):** works. `b` is git-based, not language-based — project
buffers: works (live, provider-matrix U-F7); external landings:
`no git history for external sources`, one code path
(`store.rs::open_blame` external branch, live-pinned in the python leg
L-P3) — same for every language. Editing (commit messages) and notes
(`.redline-notes.md`, README "Annotations") are text-based and
language-agnostic; notes add a Rust-only syntax anchor on top of the
line text ("Rust files get a syntax anchor" — README), degrading to the
plain line anchor for every other language.

## In-project M-. vs cross-project M-. (reading the grid)

The outline column is the floor: for every language with a definition
query (all 13 non-Plain), definitions **inside the opened project** are
M-. targets through the project index (the project walk has no
extension filter — provider-matrix, C/C++ section). Everything in the
right half of the grid (path-shaped, bare-via-import, provider,
in-library follow-up) is about M-. **outside** the project index, and
is where languages genuinely diverge.

## Gaps (prioritized — what is missing per language)

1. **C / C++: no node predicates + no scope walk + no provider.**
   `node_at`/`scope_path_at` return `None`/`[]`
   (`unimplemented_languages_return_none`, node.rs:593) and no
   tooling provider exists, so path-shaped and bare M-. both degrade
   to the 011-01 bail and only in-project definitions resolve. The
   011-07 walk sets (`c/h`, `cc/cpp/cxx/hh/hpp/hxx`) already make a
   *landed* C/C++ tree indexable — but no provider can ever produce
   the landing. Highest-value gap if C/C++ source navigation is a
   goal; the index side is done, the resolution side is absent.
2. **TypeScript / Tsx: no live drive leg.** Every TS/Tsx cell rests on
   the shared JS machinery's unit pins; `drive_issue_011_05`/`_06` only
   exercise JavaScript. A TS leg (or an explicit "TS rides the JS
   mechanism, live-verified once for JS" gate annotation) would close
   the live-verification hole.
3. **Go: unit-covered ONLY in this sandbox.** The `go` toolchain is
   absent; 3 of go_provider.rs's 41 tests are `#[ignore]`d live legs
   (two of them + network), and `drive_issue_011_05` loud-skips the Go
   leg by design. Every Go cell in this grid is `works (unit)` for
   that reason — not because coverage is thinner.
4. **In-library follow-up for C / C++ / Markdown is app-unreachable.**
   The `xref_candidates` lookup is language-agnostic and the index side
   is unit-pinned (`crate_index_builds_for_{c,cpp,markdown}_dependency_
   tree`), but no provider can land in those languages, so the live
   app can never exercise the jump. It becomes reachable only with gap
   1's provider (C/C++) or a markdown link-resolution provider.
5. **JSON / TOML / YAML / Bash: deliberately out of the index walk.**
   Their outline queries are not M-. definition sources (011-04
   judgment, carried by 011-07 — `source_extensions_for` returns
   `&[]`, asserted by `source_extensions_round_trip_through_registry_
   map`), so no index builds even for a hypothetical landing, and M-.
   bails with the honest 011-01 message. This is a judgment call, not
   a bug — but it is the one class where "basic support" is
   highlighting + in-project outline only.
6. **Markdown outline is headings-only.** No paragraphs, no links
   (verified scope — provider-matrix, Markdown section;
   `markdown_extracts_headings` + `setext_markdown_file_contributes_
   heading_symbols`). In-project M-. targets only `#`/SETEXT headings.
7. **Rust `rsi` is excluded from the index walk** (registry maps
   `.rsi` → Rust, but the walk indexes `rs` only — documented in
   `source_extensions_for` + the round-trip test). Intentional
   (rust-analyzer interface files are not definition sources);
   recorded here so it is not re-filed as a gap.

## What this file deliberately does NOT claim

No per-language performance numbers, no LSP (ruled out — plan 011),
no macro expansion / generics inference / arbitrary-expression typing.
For the provider-resolution side (miss dispatch, relative imports,
src/-layout discovery, the live drive inventory), see
`docs/provider-matrix.md`.

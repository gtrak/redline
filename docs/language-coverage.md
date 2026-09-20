# Language Coverage — what redline covers, per language

The single "what's covered, per language" record. Sibling of
`docs/provider-matrix.md` (which is provider-resolution-centric): this
file is the **language × capability** grid over ALL 17 registry
languages (`LanguageId::ALL` + `Plain`, `src/syntax/registry.rs`):
Rust, TypeScript, Tsx, JavaScript, Python, Go, C, Cpp, Toml, Json,
Yaml, Bash, Markdown, Java, CSharp, Ruby, Plain.

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
- **scope walk** — `node.rs::scope_path_at` (011-03, extended to
  C / Cpp / Bash / Toml / Json / Markdown by the lang-pred lane):
  non-`[]` for Rust / JS / TS / Tsx / Python / Go / C / Cpp / Bash /
  Toml / Json / Markdown; `Yaml` and `Plain` stay `[]`
  (`unimplemented_languages_return_none` now pins only those two).
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
| **C** | works (unit) — `c_extracts_function_and_struct` (queries.rs); `crate_index_builds_for_c_dependency_tree` (incl. `.h`) | **degrades to the bail** — the whole path now comes back at the syntax layer (`c_member_path_comes_back_whole`: `o.x.y` / `p->x` as one `field_expression`, node.rs), but the app-side whole-path upgrade does not enumerate the C containers (Known corners, item 4), so the extraction stays bare → no C provider → `no tooling provider handles language \`c\`` (011-01) | **degrades to the bail** — `resolver_scope_for` → `Vec::new()` (no hint, store.rs:8184); same message; pinned live for the identical dispatch by 011-02 L2 (`drive_issue_011_02.py:112-122`: prelude `print` → the exact no-hint bail — 011-05 L-P2/L-J1a now pin the 011-06 LANDINGS) | **works (unit)** — `c_scope_chain_struct_in_struct_and_function` (node.rs); pointer-returning functions scope as the function declarator text — Known corners, item 2 | `c/h` (headers ARE definition sources — 011-07) — `source_extensions_for` | **none — honest bail** (no C provider; 011-07 deliberately invented none) | works (unit) — index side unit-pinned; **app-unreachable**: no provider can ever land in a C dependency | works — `all_grammars_have_configs` |
| **Cpp** | works (unit) — `cpp_extracts_function_class_struct` (queries.rs); `crate_index_builds_for_cpp_dependency_tree` (incl. `.hpp`/`.hh`) | **degrades to the bail** — same as C: the syntax layer answers the whole path (`cpp_member_path_comes_back_whole`, `cpp_qualified_path_comes_back_whole` — `ns::Base::C` as one `qualified_identifier`) but the app-side whole-path upgrade does not enumerate the C++ containers (Known corners, item 4) and no provider exists → 011-01 message | **degrades to the bail** — same as C (no hint) | **works (unit)** — `cpp_scope_chain_namespace_class_method` (node.rs: namespace → class → method chain); pointer-returning corner as C — Known corners, item 2 | `cc/cpp/cxx/hh/hpp/hxx` (full registry map — 011-07) — `source_extensions_for` | **none — honest bail** | works (unit) — same app-unreachable note as C | works — `all_grammars_have_configs` |
| **Toml** | works (unit) — `toml_extracts_keys` (queries.rs) | **degrades to the bail** — the dotted key now comes back whole at the syntax layer (`toml_dotted_key_comes_back_whole`, `toml_table_header_dotted_key_comes_back_whole` — one `dotted_key`), but the app-side whole-path upgrade does not enumerate the TOML containers (Known corners, item 4) and no TOML provider exists → `no tooling provider handles language \`toml\`` (011-01) | **degrades to the bail** — no import machinery; `resolver_scope_for` → `Vec::new()` | **works (unit)** — `toml_table_header_dotted_key_comes_back_whole` (an enclosing `[table.sub]` header IS the scope — its text as written, one element); `toml_top_level_dotted_key_has_no_scope`; `[[array.table]]` headers contribute no element — Known corners, item 3 | **none** (empty set — 011-04 judgment carried by 011-07: config files are not M-. definition sources; asserted empty by `source_extensions_round_trip_through_registry_map`) | **none — honest bail** | **not applicable** — no index builds (empty walk set) and no provider can land | works — `all_grammars_have_configs` |
| **Json** | works (unit) — `json_extracts_keys` (queries.rs) | **degrades to the bail** — `no tooling provider handles language \`json\`` (011-01); JSON has no dotted path syntax — its path concept is STRUCTURAL (the scope column's key chain), and `node_at` answers keys in pair position (the `string` key kind; `json_boundary_offsets_do_not_panic`) | **degrades to the bail** — no import machinery | **works (unit)** — `json_scope_chain_is_the_enclosing_key_chain` (node.rs: the enclosing keys, unquoted — the structural path); an escaped key's scope element is its first `string_content` child — Known corners, item 1 | **none** (empty set — same 011-04/011-07 judgment) | **none — honest bail** | **not applicable** — no index builds; no provider can land | works — `all_grammars_have_configs` |
| **Yaml** | works (unit) — `yaml_extracts_keys` (queries.rs) | **degrades to the bail** — `no tooling provider handles language \`yaml\`` (011-01) | **degrades to the bail** — no import machinery | **not implemented** — `unimplemented_languages_return_none` | **none** (empty set — same judgment) | **none — honest bail** | **not applicable** — no index builds; no provider can land | works — `all_grammars_have_configs` |
| **Bash** | works (unit) — `bash_extracts_functions` (queries.rs) | **degrades to the bail** — `no tooling provider handles language \`bash\`` (011-01) (N/A path-shaped: commands are not dotted uses — `node_at` resolves `command_name` / `variable_name`, `bash_command_name_resolves_as_command_name`) | **degrades to the bail** — no import machinery | **works (unit)** — `bash_scope_chain_function_body` (node.rs: a function body is the only non-empty scope; top-level commands `[]`) | **none** (empty set — same judgment) | **none — honest bail** | **not applicable** — no index builds; no provider can land | works — `all_grammars_have_configs` |
| **Markdown** | works (unit) — `markdown_extracts_headings`, `setext_markdown_file_contributes_heading_symbols` (ATX + SETEXT headings only — verified scope, no paragraphs/links) | **degrades to the bail** — `no tooling provider handles language \`markdown\`` (011-01); `node_at` → `None` (heading titles are `inline` nodes, not identifier-ish — `markdown_heading_text_is_not_identifier_ish`) | **degrades to the bail** — no import machinery; `resolver_scope_for` → `Vec::new()` | **works (unit)** — `markdown_scope_chain_is_the_enclosing_headings` + `markdown_setext_heading_contributes_scope` (node.rs: the ATX + SETEXT heading chain is the outline) | `md/markdown/mdx` (headings only — 011-07) — `source_extensions_for` | **none — honest bail** | works (unit) — `crate_index_builds_for_markdown_dependency_tree`; **app-unreachable**: no provider can land | works — `all_grammars_have_configs` |
| **Java** | works (unit) — `java_extracts_class_and_method` (queries.rs: classes / interfaces / enums / methods / constructors; fields and enum constants deliberately out — the outline is the honest minimal classes/methods set, and the pinned grammar version does not parse standalone record declarations, probe-verified); ABI pin `all_grammars_set_language_succeeds` (registry.rs) | **degrades to the bail** — the syntax layer answers the whole path (`java_member_path_comes_back_whole`: `A.c` as one `field_access`; `java_scoped_type_path_comes_back_whole`: `com.example.Foo` as one `scoped_type_identifier`; `java_method_invocation_stays_bare`: `o.m(…)` stays bare — node.rs), but the app-side whole-path upgrade does not enumerate the Java containers (`store.rs::dotted_path_container` — owned by the parallel M-. lane; Known corners, item 4's shape) and no Java provider exists → 011-01 message | **degrades to the bail** — no import machinery; `resolver_scope_for` → `Vec::new()` | **works (unit)** — `java_scope_chain_class_and_method` (node.rs: class → method chain; top-level code `[]`) | `java` — `source_extensions_for` (new-languages lane) | **none — honest bail** | **app-unreachable** — no provider can land (same note as C/C++); the index walk is the language-agnostic 011-04 machinery over the `java` set | works — `all_grammars_have_configs` |
| **CSharp** | works (unit) — `csharp_extracts_class_and_method` (queries.rs: namespaces / classes / interfaces / enums / structs / records / properties / methods / constructors / delegates; enum members and local variables deliberately out); highlight query vendored verbatim from the pinned crate (`third_party/tree-sitter-c-sharp-0.23.1/highlights.scm` — the crate exports no `HIGHLIGHTS_QUERY` constant) | **degrades to the bail** — the syntax layer answers the whole path (`csharp_member_path_comes_back_whole`: `o.P` as one `member_access_expression`; `csharp_qualified_name_comes_back_whole`: `N.Inner` as one `qualified_name` — node.rs), but the app-side whole-path upgrade does not enumerate the C# containers (`store.rs::dotted_path_container` — owned by the parallel M-. lane; Known corners, item 4's shape) and no C# provider exists → 011-01 message | **degrades to the bail** — no import machinery; `resolver_scope_for` → `Vec::new()` | **works (unit)** — `csharp_scope_chain_namespace_class_method` (node.rs: namespace → class → method chain) | `cs` — `source_extensions_for` (new-languages lane) | **none — honest bail** | **app-unreachable** — no provider can land (same note as C/C++); the index walk is the language-agnostic 011-04 machinery over the `cs` set | works — `all_grammars_have_configs` (the vendored highlight query must compile) |
| **Ruby** | works (unit) — `ruby_extracts_module_class_method` (queries.rs: modules / classes / defs / singleton defs / top-level constants; local variables, instance-variable assignments, and superclass references deliberately out) | **degrades to the bail** — the syntax layer answers the whole path (`ruby_method_chain_comes_back_whole`: `obj.name` as one argumentless `call`; `ruby_scope_resolution_comes_back_whole`: `Foo::Bar` as one `scope_resolution`; `ruby_bare_call_stays_bare`: `puts 1` stays bare — the argumentless-receiver rule, node.rs), but the app-side whole-path upgrade does not enumerate the Ruby containers (`store.rs::dotted_path_container` — owned by the parallel M-. lane; Known corners, item 4's shape) and no Ruby provider exists → 011-01 message | **degrades to the bail** — no import machinery; `resolver_scope_for` → `Vec::new()` | **works (unit)** — `ruby_scope_chain_module_class_method` (node.rs: module → class → method chain) | `rb` — `source_extensions_for` (new-languages lane) | **none — honest bail** | **app-unreachable** — no provider can land (same note as C/C++); the index walk is the language-agnostic 011-04 machinery over the `rb` set | works — `all_grammars_have_configs` |
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
query (all 16 non-Plain), definitions **inside the opened project** are
M-. targets through the project index (the project walk has no
extension filter — provider-matrix, C/C++ section). Everything in the
right half of the grid (path-shaped, bare-via-import, provider,
in-library follow-up) is about M-. **outside** the project index, and
is where languages genuinely diverge.

## Gaps (prioritized — what is missing per language)

1. **C / C++: no provider** (the node predicates + scope walk landed
   with the lang-pred lane). `node_at`/`scope_path_at` now answer for
   C/C++ (`c_member_path_comes_back_whole`,
   `cpp_qualified_path_comes_back_whole`,
   `c_scope_chain_struct_in_struct_and_function`,
   `cpp_scope_chain_namespace_class_method` — node.rs), and
   `unimplemented_languages_return_none` now covers only Yaml + Plain.
   What still bails: no tooling provider exists, and the app-side
   whole-path upgrade (`store.rs::dotted_path_container`) does not
   enumerate the C/C++ containers (Known corners, item 4), so
   path-shaped and bare M-. both degrade to the 011-01 bail and only
   in-project definitions resolve. The 011-07 walk sets (`c/h`,
   `cc/cpp/cxx/hh/hpp/hxx`) already make a *landed* C/C++ tree
   indexable — but no provider can ever produce the landing.
   Highest-value gap if C/C++ source navigation is a goal; the index
   side and the syntax side are done, the resolution side is absent.
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
8. **Clojure / Lisp / C# / Ruby: lane deferred — grammar crates
   not usable against the pinned runtime.** The new-languages lane
   (the user directive "add clojure and lisp support, java, c#, ruby")
   first round (offline-first) deferred everything: none of the needed
   crates were vendored in `~/.cargo/registry` and mid-task network
   fetches were disallowed. Re-run under the amended contract
   (crates.io sanctioned, ABI-pinning rules as the real constraint):
   **Java landed** (`tree-sitter-java =0.23.5`, grammar ABI 14 —
   `set_language` probe-verified against the pinned 0.24.7 runtime;
   registry/queries/node/walk-set work per the capability ladder,
   NODE_TYPES probe first). **C# landed** (`tree-sitter-c-sharp
   =0.23.1`, grammar ABI 14 — same staging; the crate's own
   `queries/highlights.scm` is vendored verbatim under
   `third_party/tree-sitter-c-sharp-0.23.1/` because the crate does
   not export a `HIGHLIGHTS_QUERY` constant). The remaining languages
   are still deferred:
   `tree-sitter-clojure` 0.1.0 requires tree-sitter `^0.25.6` (cargo
   `links` conflict with the pinned runtime — does not even resolve),
   `tree-sitter-clojure-orchard` 0.2.x requires `^0.25.9`/`^0.26.11`
   (same), and `arborium-clojure` 2.18.2 resolves but its grammar is
   ABI 15 → `set_language` fails (all three probe-verified); the
   elisp-family crate (`tree-sitter-elisp` 1.7.2) is likewise ABI 15 →
   `set_language` fails. The lisp family lands as Scheme
   (`tree-sitter-scheme` 0.24.7, ABI 14 — the only Lisp-family crate
   matching the pinned runtime); **Ruby landed** (`tree-sitter-ruby`
   0.23.1, ABI 14 — same staging; its `LOCALS_QUERY` feeds the
   highlight config like TypeScript's).

## Known corners (documented simplifications — not gaps)

The lang-pred lane's per-language node predicates + scope walks carry
these pinned corners, recorded here so they are not re-filed as bugs:

1. **JSON keys with escape sequences: the scope element is the FIRST
   `string_content` child.** `json_scope_path` (node.rs) reads the
   key's `string_content` child; for an escaped key the first child is
   only the leading unescaped run — `{"a\\nb": 1}` → scope element
   `a`, not the full unescaped key. A silent partial, no app-side
   consumer reads JSON scope paths yet, so it is not reachable today.
   (Verified by direct probe of `scope_path_at`.)
2. **C/C++ pointer-returning functions scope as the function
   declarator text.** `c_scope_path`/`cpp_scope_path` take the
   `function_definition`'s `declarator` field, then its `declarator`
   field: for a plain function that lands on the name (bare `f`), but
   for a pointer return the intermediate node is a `pointer_declarator`,
   so the element is the inner `function_declarator` text — `int *f()`
   → scope element `f()` (it can carry the parameter list and is not
   the bare name). Deliberate simplification; the resolver matches on
   this text. (Verified by direct probe of `scope_path_at`.)
3. **TOML `[[array.table]]` headers contribute no scope element.**
   `toml_scope_path` (node.rs) walks `table` nodes only; an
   array-table header is an `array_table` node, so keys inside a
   `[[a.b]]` block see `[]` (a plain `[a.b]` header DOES contribute —
   its text as written, one element). (Verified by direct probe of
   `scope_path_at`.)
4. **The app-side whole-path upgrade does not enumerate the C / Cpp /
   Toml containers.** `store.rs::dotted_path_container` lists only
   JS/TS/Tsx (`member_expression`, `nested_type_identifier`,
   `nested_identifier`), Python (`attribute`), Go
   (`selector_expression`, `qualified_type`) — every other language
   returns `false`, so M-. extraction stays at the bare segment even
   though the syntax layer's `node_at` returns the whole path
   (`field_expression` / `qualified_identifier` / `dotted_key`).
   Boundary: the syntax layer answers the whole path; the app's M-. 
   extraction does not consume it. (JSON/Bash/Markdown have no dotted
   path container to enumerate in the first place.)

## What this file deliberately does NOT claim

No per-language performance numbers, no LSP (ruled out — plan 011),
no macro expansion / generics inference / arbitrary-expression typing.
For the provider-resolution side (miss dispatch, relative imports,
src/-layout discovery, the live drive inventory), see
`docs/provider-matrix.md`.

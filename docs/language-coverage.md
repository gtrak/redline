# Language Coverage — what redline covers, per language

The single "what's covered, per language" record. Sibling of
`docs/provider-matrix.md` (which is provider-resolution-centric): this
file is the **language × capability** grid over ALL 19 languages
(`crates/redline-syntax/src/language.rs`'s descriptor table — one row per `LanguageId`):
Rust, TypeScript, Tsx, JavaScript, Python, Go, C, Cpp, Toml, Json,
Yaml, Bash, Markdown, Java, CSharp, Ruby, Scheme, Clojure, Plain.

**Runtime note (ts-bump lane):** the pinned tree-sitter runtime moved
0.24.7 → 0.25.10 (all 16 pre-existing grammar pins UNCHANGED — their
`tree-sitter` requirements are dev-dependencies and their grammar ABIs
13/14 sit inside the 0.25.10 window 13..=15; the full compatibility
matrix lives in `docs/tree-sitter-runtime-matrix.md`). Clojure is the
first 0.25-generation landing (its crate hard-requires `tree-sitter
^0.25.6` — the bump that unblocked it). Addendum (grammar-bumps lane):
the "UNCHANGED" clause above is stale — nine of those pins moved to
their 0.25-generation releases (rust 0.24.2, javascript/python/go
0.25.0, c 0.24.2, bash 0.25.1, yaml 0.7.2, c-sharp 0.23.5, md 0.5.1 —
ABI 15); verdict table in `docs/tree-sitter-runtime-matrix.md`.

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
pinned by `registry.rs` tests: `ext_map_covers_all_19_languages`,
`unknown_extension_falls_back_to_plain`, `all_grammars_have_configs`,
`language_for_uses_extension`.

## The grid

Capability columns, in the code's terms:

- **outline** — symbol extraction / definition query exists
  (`crates/redline-syntax/src/queries.rs::query_for`); the project index is built
  from these queries (the project walk has no extension filter).
- **M-. path-shaped** — M-. on a qualified use (`pkg.member`):
  011-06 language-aware extraction requires 011-03's `node_at`
  path-container machinery (`crates/redline-syntax/src/node/`).
- **M-. bare via import** — M-. on a bare identifier an import in the
  buffer binds: 011-02 scope hints
  (`store.rs::resolver_scope_for` — provider-chain languages only).
- **scope walk** — `node.rs::scope_path_at` (011-03, extended to
  C / Cpp / Bash / Toml / Json / Markdown by the lang-pred lane and to
  Java / CSharp / Ruby by the new-languages lane):
  non-`[]` for Rust / JS / TS / Tsx / Python / Go / C / Cpp / Bash /
  Toml / Json / Markdown / Java / CSharp / Ruby; `Yaml`, `Plain`,
  `Scheme`, and `Clojure` stay `[]` (Scheme's and Clojure's flat
  S-expression grammars have no named definition containers — the
  honest N/A, `scheme_has_no_path_or_scope`, `clojure_has_no_scope`;
  `unimplemented_languages_return_none` pins Yaml + Plain).
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
| **C** | works (unit) — `c_extracts_function_and_struct` (queries.rs); `crate_index_builds_for_c_dependency_tree` (incl. `.h`) | **degrades to the bail** — the whole path now comes back at the syntax layer (`c_member_path_comes_back_whole`: `o.x.y` / `p->x` as one `field_expression`, node.rs) and the app-side whole-path upgrade now enumerates C's `field_expression` (010-rung4-and-paths), so M-. carries the whole path (`o.x` — unit-pinned by `symbol_at_point_c_cpp_toml_containers_extend_the_token`; in-project, a field access lands via the enclosing-symbol fall-through, `xref_c_field_access_lands_via_index_fall_through`; `p->x` degrades byte-for-byte, `symbol_at_point_c_arrow_and_json_keys_stay_bare`) — but no C provider exists → `no tooling provider handles language \`c\`` (011-01) | **degrades to the bail** — `resolver_scope_for` → `Vec::new()` (no hint, store.rs:8184); same message; pinned live for the identical dispatch by 011-02 L2 (`drive_issue_011_02.py:112-122`: prelude `print` → the exact no-hint bail — 011-05 L-P2/L-J1a now pin the 011-06 LANDINGS) | **works (unit)** — `c_scope_chain_struct_in_struct_and_function` (node.rs); pointer-returning functions scope as the function declarator text — Known corners, item 2 | `c/h` (headers ARE definition sources — 011-07) — `source_extensions_for` | **none — honest bail** (no C provider; 011-07 deliberately invented none) | works (unit) — index side unit-pinned; **app-unreachable**: no provider can ever land in a C dependency | works — `all_grammars_have_configs` |
| **Cpp** | works (unit) — `cpp_extracts_function_class_struct` (queries.rs); `crate_index_builds_for_cpp_dependency_tree` (incl. `.hpp`/`.hh`) | **degrades to the bail** — same as C: the syntax layer answers the whole path (`cpp_member_path_comes_back_whole`, `cpp_qualified_path_comes_back_whole` — `ns::Base::C` as one `qualified_identifier`) and the app-side whole-path upgrade now enumerates Cpp's `field_expression` (010-rung4-and-paths; its `::` shape stays the byte-scan token — deliberately not double-handled, Known corners, item 4) — but no provider exists → 011-01 message | **degrades to the bail** — same as C (no hint) | **works (unit)** — `cpp_scope_chain_namespace_class_method` (node.rs: namespace → class → method chain); pointer-returning corner as C — Known corners, item 2 | `cc/cpp/cxx/hh/hpp/hxx` (full registry map — 011-07) — `source_extensions_for` | **none — honest bail** | works (unit) — same app-unreachable note as C | works — `all_grammars_have_configs` |
| **Toml** | works (unit) — `toml_extracts_keys` (queries.rs) | **degrades to the bail** — the dotted key now comes back whole at the syntax layer (`toml_dotted_key_comes_back_whole`, `toml_table_header_dotted_key_comes_back_whole` — one `dotted_key`) and the app-side whole-path upgrade now enumerates TOML's `dotted_key` (010-rung4-and-paths — the index stores the dotted key as ONE symbol name, so a segment's M-. reaches it only with the whole path; unit-pinned by `symbol_at_point_c_cpp_toml_containers_extend_the_token`) but no TOML provider exists → `no tooling provider handles language \`toml\`` (011-01) | **degrades to the bail** — no import machinery; `resolver_scope_for` → `Vec::new()` | **works (unit)** — `toml_table_header_dotted_key_comes_back_whole` (an enclosing `[table.sub]` header IS the scope — its text as written, one element); `toml_top_level_dotted_key_has_no_scope`; `[[array.table]]` headers contribute no element — Known corners, item 3 | **none** (empty set — 011-04 judgment carried by 011-07: config files are not M-. definition sources; asserted empty by `source_extensions_round_trip_through_registry_map`) | **none — honest bail** | **not applicable** — no index builds (empty walk set) and no provider can land | works — `all_grammars_have_configs` |
| **Json** | **zero symbols (deliberate)** — `json_yields_zero_symbols` (queries.rs); JSON keys are not navigation targets (issue-json-yaml-no-symbols: they were 90% of a real project's symbol index) | **degrades to the bail** — `no tooling provider handles language \`json\`` (011-01); JSON has no dotted path syntax — its path concept is STRUCTURAL (the scope column's key chain), and `node_at` answers keys in pair position (the `string` key kind; `json_boundary_offsets_do_not_panic`) | **degrades to the bail** — no import machinery | **works (unit)** — `json_scope_chain_is_the_enclosing_key_chain` (node.rs: the enclosing keys, unquoted — the structural path); an escaped key's scope element is its first `string_content` child — Known corners, item 1 | **none** (empty set — same 011-04/011-07 judgment) | **none — honest bail** | **not applicable** — no index builds; no provider can land | works — `all_grammars_have_configs` |
| **Yaml** | **zero symbols (deliberate)** — `yaml_yields_zero_symbols` (queries.rs); YAML keys are not navigation targets (issue-json-yaml-no-symbols) | **degrades to the bail** — `no tooling provider handles language \`yaml\`` (011-01) | **degrades to the bail** — no import machinery | **not implemented** — `unimplemented_languages_return_none` | **none** (empty set — same judgment) | **none — honest bail** | **not applicable** — no index builds; no provider can land | works — `all_grammars_have_configs` |
| **Bash** | works (unit) — `bash_extracts_functions` (queries.rs) | **degrades to the bail** — `no tooling provider handles language \`bash\`` (011-01) (N/A path-shaped: commands are not dotted uses — `node_at` resolves `command_name` / `variable_name`, `bash_command_name_resolves_as_command_name`) | **degrades to the bail** — no import machinery | **works (unit)** — `bash_scope_chain_function_body` (node.rs: a function body is the only non-empty scope; top-level commands `[]`) | **none** (empty set — same judgment) | **none — honest bail** | **not applicable** — no index builds; no provider can land | works — `all_grammars_have_configs` |
| **Markdown** | works (unit) — `markdown_extracts_headings`, `setext_markdown_file_contributes_heading_symbols` (ATX + SETEXT headings only — verified scope, no paragraphs/links) | **degrades to the bail** — `no tooling provider handles language \`markdown\`` (011-01); `node_at` → `None` (heading titles are `inline` nodes, not identifier-ish — `markdown_heading_text_is_not_identifier_ish`) | **degrades to the bail** — no import machinery; `resolver_scope_for` → `Vec::new()` | **works (unit)** — `markdown_scope_chain_is_the_enclosing_headings` + `markdown_setext_heading_contributes_scope` (node.rs: the ATX + SETEXT heading chain is the outline) | `md/markdown/mdx` (headings only — 011-07) — `source_extensions_for` | **none — honest bail** | works (unit) — `crate_index_builds_for_markdown_dependency_tree`; **app-unreachable**: no provider can land | works — `all_grammars_have_configs` |
| **Java** | works (unit) — `java_extracts_class_and_method` (queries.rs: classes / interfaces / enums / methods / constructors; fields and enum constants deliberately out — the outline is the honest minimal classes/methods set, and the pinned grammar version does not parse standalone record declarations, probe-verified); ABI pin `all_grammars_set_language_succeeds` (registry.rs) | **degrades to the bail** (cross-project) — the app-side whole-path upgrade now enumerates Java's `field_access` / `scoped_identifier` / `scoped_type_identifier` (newlang-paths), so M-. carries the whole path (`A.c`, `com.example.Foo`; `o.m(…)` stays bare — node.rs `java_member_path_comes_back_whole` / `java_scoped_type_path_comes_back_whole` / `java_method_invocation_stays_bare`, store.rs `symbol_at_point_java_csharp_ruby_containers_extend_the_token` / `symbol_at_point_java_csharp_ruby_degradation_stays_bare`); the outline indexes classes / methods only, so a field access lands via the enclosing-symbol fall-through (`xref_java_field_access_lands_via_index_fall_through`), and no Java provider exists → 011-01 message | **degrades to the bail** — no import machinery; `resolver_scope_for` → `Vec::new()` | **works (unit)** — `java_scope_chain_class_and_method` (node.rs: class → method chain; top-level code `[]`) | `java` — `source_extensions_for` (new-languages lane) | **none — honest bail** | **app-unreachable** — no provider can land (same note as C/C++); the index walk is the language-agnostic 011-04 machinery over the `java` set | works — `all_grammars_have_configs` |
| **CSharp** | works (unit) — `csharp_extracts_class_and_method` (queries.rs: namespaces / classes / interfaces / enums / structs / records / properties / methods / constructors / delegates; enum members and local variables deliberately out); highlight query vendored verbatim from the pinned crate (`third_party/tree-sitter-c-sharp-0.23.5/highlights.scm` — the crate exports no `HIGHLIGHTS_QUERY` constant) | **degrades to the bail** (cross-project) — the app-side whole-path upgrade now enumerates C#'s `member_access_expression` / `qualified_name` (newlang-paths), so M-. carries the whole path (`o.P`, `N.Inner` — node.rs `csharp_member_path_comes_back_whole` / `csharp_qualified_name_comes_back_whole`; store.rs `symbol_at_point_java_csharp_ruby_containers_extend_the_token`); a property access lands in the project index — the fall-through to the last segment `P` hits the indexed property (`xref_csharp_property_access_lands_in_project_index`) — and no C# provider exists → 011-01 message | **degrades to the bail** — no import machinery; `resolver_scope_for` → `Vec::new()` | **works (unit)** — `csharp_scope_chain_namespace_class_method` (node.rs: namespace → class → method chain) | `cs` — `source_extensions_for` (new-languages lane) | **none — honest bail** | **app-unreachable** — no provider can land (same note as C/C++); the index walk is the language-agnostic 011-04 machinery over the `cs` set | works — `all_grammars_have_configs` (the vendored highlight query must compile) |
| **Ruby** | works (unit) — `ruby_extracts_module_class_method` (queries.rs: modules / classes / defs / singleton defs / top-level constants; local variables, instance-variable assignments, and superclass references deliberately out) | **degrades to the bail** (cross-project) — the app-side whole-path upgrade now enumerates Ruby's argumentless `call` (newlang-paths — a call WITH arguments must NEVER match: node.rs's receiver/no-`arguments` position gate + the 011-06 all-identifier-segment guard pin `a.b(1).c` → bare `c`, `symbol_at_point_java_csharp_ruby_degradation_stays_bare`), so M-. carries the whole path (`obj.name` — node.rs `ruby_method_chain_comes_back_whole` / `ruby_bare_call_stays_bare`; store.rs `symbol_at_point_java_csharp_ruby_containers_extend_the_token`); a method access lands in the project index — the fall-through to the last segment `name` hits the indexed `def` (`xref_ruby_method_access_lands_in_project_index`) — and no Ruby provider exists → 011-01 message | **degrades to the bail** — no import machinery; `resolver_scope_for` → `Vec::new()` | **works (unit)** — `ruby_scope_chain_module_class_method` (node.rs: module → class → method chain) | `rb` — `source_extensions_for` (new-languages lane) | **none — honest bail** | **app-unreachable** — no provider can land (same note as C/C++); the index walk is the language-agnostic 011-04 machinery over the `rb` set | works — `all_grammars_have_configs` |
| **Scheme** | works (unit) — `scheme_extracts_define_and_library` (queries.rs: `define` fn / `define` var / `define-library` / `define-record-type` / `define-macro` — the flat S-expression grammar has no definition node kinds and its `symbol` kind refuses literal content matches (probe-verified), so the outline captures every first-child-anchored `(list HEAD …)` candidate and `flat_define_kind` gates on (pattern index, head symbol); `export` / `set!` / record accessors deliberately out) | **N/A (no path syntax)** — the grammar has no dotted-path construct (module paths are `list`s); `node_at` resolves any `symbol` to itself (`scheme_symbol_resolves_bare`); a bare M-. on a symbol resolves in-project only, then bails (011-01, no provider) | **degrades to the bail** — no import machinery; `resolver_scope_for` → `Vec::new()` | **N/A (honest)** — the flat grammar has no named definition containers to walk; `scope_path_at` stays `[]` even inside a `define-library` body (`scheme_has_no_path_or_scope`) | `scm/ss/sls/sld` — `source_extensions_for` (new-languages lane) | **none — honest bail** | **app-unreachable** — no provider can land (same note as C/C++); the index walk is the language-agnostic 011-04 machinery over the `scm/ss/sls/sld` set | works — `all_grammars_have_configs` |
| **Clojure** | works (unit) — `clojure_extracts_defn_and_defs` (queries.rs: `defn` / `defn-` / `def` / `defmacro` / `defmulti` / `defmethod` / `defrecord` / `deftype` / `defprotocol` — the flat S-expression grammar has no definition node kinds (probe-verified: a `defn` is a bare `sym_lit` head inside a `list_lit`), so the outline captures every two-symbol `list_lit` candidate and `flat_define_kind` gates on the head symbol's text; application forms, `ns`, and `let` deliberately out; `defmethod` lands by the multimethod var — the class+constructor precedent) | **N/A (no path syntax)** — a namespaced symbol (`my.lib`, `ns/var`) is ONE token inside a single `sym_name` leaf (probe-verified), not a path container; `node_at` resolves a symbol to its whole `sym_lit`, a namespaced symbol whole, a meta prefix (`^:doc`) to the whole meta-carrying `sym_lit` text (`^:doc x` — the meta sits INSIDE the symbol node; the outline still captures the bare `x`), and a bare keyword to nothing (`clojure_symbol_resolves_bare_and_namespaced`); a bare M-. resolves in-project only, then bails (011-01, no provider) | **degrades to the bail** — no import machinery; `resolver_scope_for` → `Vec::new()` | **N/A (honest)** — the flat grammar has no named definition containers to walk (`defn` is a `list_lit`); `scope_path_at` stays `[]` even inside a defn body (`clojure_has_no_scope`) | `clj/cljs/cljc` — `source_extensions_for` (runtime-bump lane) | **none — honest bail** | **app-unreachable** — no provider can land (same note as C/C++); the index walk is the language-agnostic 011-04 machinery over the `clj/cljs/cljc` set | works — `all_grammars_have_configs` (the vendored highlight query must compile) |
| **Plain** | N/A (not a code language) — no grammar, `query_for` → `None` (queries.rs:262) | N/A (not a code language) — no path container; the miss path is the 011-01 unset-language backward-compat walk, pinned by `language_dispatch_unset_tries_all_in_order` (lib.rs:575) | N/A (not a code language) — `resolver_scope_for` → `Vec::new()` | N/A (not a code language) — `scope_path_at` → `[]` | **none** (empty set — asserted by `source_extensions_round_trip_through_registry_map`) | none — the unset-language dispatch walks the whole chain (backward-compat pin above) | **not applicable** | N/A (not a code language) — `LanguageId::Plain => None` (registry.rs); `all_grammars_have_configs` asserts it |

**blame / light editing / notes (language-agnostic — one row for all 18
languages):** works. `b` is git-based, not language-based — project
buffers: works (live, provider-matrix U-F7); external landings:
`no git history for external sources`, one code path
(`store.rs::open_blame` external branch, live-pinned in the python leg
L-P3) — same for every language. Editing (commit messages) and notes
(`.redline-notes.md`, README "Annotations") are text-based and
language-agnostic; notes add a per-language syntax anchor on top of the
line text — every grammar-bearing language with an identifier-ish kind gets
a symbol anchor (issue-annotations-symbol-identity; the old Rust-only gate
is gone), and the ones that ALSO have a scope walker key it by scope
(Scheme and Clojure have an identifier kind but no scope walker, so their
scope is always empty and they resolve across distances but not across
same-scope name repeats), degrading to the plain line anchor for languages
with no identifier kind (Yaml, Markdown, Plain), for legacy records, and
for points off a symbol. Two measured limits of the Rust symbol anchor
are filed, not fixed (`issue-annotation-stage-2b` addendum, 2026-09-23):
a struct field's type identity carries **no scope** (the Rust scope
walker covers impls / fns / mods, not struct bodies — `struct A { name:
String }` anchors as `(type_identifier, "String", [])`, so a common
type like `String` collides across structs and the note degrades to the
text rules), and a reference type such as `&'a str` captures **no anchor
at all** (a silent line-tied degradation).

## Mechanisms with no grid column (measured the same way)

The grid was built capability-by-capability as features landed, so
whole mechanisms never got a row. Each entry below carries the same
evidence standard as the grid (a test name, a drive leg, or a source
line) and the same vocabulary.

**Rust field / trait tables (field and impl-method M-.,
find-implementations) — RUST-ONLY.** `rust_tables` / `rust_fields` /
`rust_traits` (`src/nav/index/symbol_index.rs`) are built in the same
rayon pass as the outline (the same trees — zero extra parse cost):
`self.<member>` resolves through the LEXICALLY ENCLOSING impl's type
to the struct's field line or the impl method's line, a
`let x: Type` / `let x = Type { … }` binding rides the same tables,
and a trait name opens the find-implementations picker over the
`impl Trait for Type` blocks. Pinned: `xref_self_receiver_field_lands_on_the_field_name_column`,
`xref_local_binding_method_call_narrows_ambiguous_impls`,
`find_implementations_opens_picker_of_trait_impls` (tests/navigation/
definitions.rs). Every other language: no tables exist (`rust_tables_query: Some`
on the Rust registry row only), and M-. falls through to the
name-keyed index byte-for-byte. **Stated correctness limit:**
`field_locations` is keyed by the **bare struct name** (`rust_fields`:
struct name → field name → locations), so two same-named structs in
different modules with a same-named field COLLAPSE — the never-guess
contract is weakened by the keying, not by the lookup logic.

**imenu impl-parent grouping — RUST-ONLY.** An impl method renders one
level deeper, under the impl's type (`imenu_candidate` /
`imenu_depth`, `src/app/store/picker.rs`; the data is
`current_buffer_rust_tables`, `src/app/store/navigation/xref.rs` —
`None` for every non-Rust buffer, so the flat enclosing-extent indent
stands byte-for-byte in all other languages). Pinned:
`unit_flow_imenu_impl_parent_grouping` (flow_tests.rs),
`imenu_groups_impl_methods_under_the_struct`,
`imenu_non_rust_stays_flat` (the non-Rust degradation).

**M-? references — UNIFORM for all 18 grammar-bearing languages.** A
word-boundary, fixed-string ripgrep search over the project, filtered
per file by the tree-sitter token class: comment / string hits are
dropped wherever a grammar exists (`references_filter` →
`comment_string_ranges`, `src/search/references.rs` +
`crates/redline-syntax/src/tokens.rs`), and the extracted run obeys the
per-language word rule below (the Lisp family references the last `/`
segment, keyword markers stripped). Two documented exceptions: Markdown's
filter is INACTIVE (the grammar has no comment or string node kinds —
the hits are kept), and Plain is the no-filter fallback (comment / string
hits are kept). Pinned: `references_drops_comment_and_string_hits`,
`references_fallback_keeps_comment_and_string_hits`,
`references_at_point_searches_the_symbol`.

**`injections_query` — loaded but INERT (uniformly, for every
language).** Three registry rows carry a non-empty injections query
(Rust, JavaScript, Markdown), and `registry.rs` passes it to
`HighlightConfiguration::new` — but the injection callback is
`|_| None` (`crates/redline-syntax/src/highlight.rs`). Net effect:
fenced code blocks in Markdown, Rust `html!` macro bodies, and JS
tagged templates are **never** highlighted as their embedded language.
The inertness is uniform, so the Markdown "highlighting — works" cell in
the grid above is read as NOT covering embedded-language highlighting.

**Per-language word / symbol rule** (issue-language-aware-symbols,
landed `b3f8234`). The word-constituent set per language (`WordRule` /
`is_word_char`, `crates/redline-syntax/src/language.rs` — consulted by
the word motions, the M-? / M-. extraction, the ripgrep word-boundary
sink, and the kill-word walks; alnum + `_` is the base for every
language):

- `-` additionally: **Scheme, Clojure** (the Lisp reader's symbol
  alphabet) and **Toml** (bare keys — probe: `a-b` / `key-x` each parse
  as ONE `bare_key`; `-` is not an operator there)
- `$` additionally: **JavaScript, TypeScript, Tsx** (probe: `$foo`,
  `bar$`, `$bar` each parse as ONE `identifier`; `-` STAYS a boundary
  — arithmetic)
- `?` `!` additionally: **Ruby** (method suffixes — probe: `empty?` /
  `save!` are ONE `identifier`; the ternary `?` stays a separate
  whitespace-separated node)
- `* + ! - ? < > = . / :` additionally: **Scheme, Clojure** (the Lisp
  reader's symbol alphabet `[a-zA-Z0-9*+!?:_.-/]` — probe:
  `jwks/fetch-issuer-info` and `::jwks/local` are ONE node each; the
  quote form keeps its own `'` and stays a boundary)
- base only (alnum + `_`): **Rust, Python, Go, C, Cpp, Java, CSharp,
  Bash, Json, Yaml, Markdown, Plain** — 12 rows

Measured from the registry's `word_rule` column: 12 Default / 3 Dollar /
1 Suffix / 2 Lisp / 1 BareKey = 19 rows. Pinned BOTH directions by
`word_rule_is_per_language` (the new constituents are one word AND the
operators that should split still split). What the rule does NOT cover:
a Rust lifetime `'a` still extracts `a` on M-. (the tick is not a word
char — audit B4, recorded, not claimed), and the struct-field /
reference-type anchor limits noted in the notes row above.

**Import / alias / module resolution — the plan-017 conventions
mechanism.** A SHARED pre-step in M-. (`crates/redline-syntax/src/
conventions.rs`: the conventions table, name → path tails, plus the
per-language import/alias extractor; ONE call site in
`src/app/store/navigation/definitions.rs` — no per-language branch, a
new language is a table row plus a query). It composes with the index:
the convention narrows the index's name-keyed candidates to the files
it places the definition in — never a second resolver engine, never an
absolute guess (the tails are matched against the indexed files):

- **Clojure — works (unit).** The `ns`-form alias map (`clojure::ns_aliases`
  — alias entries + identity entries for un-aliased requires; first-
  `ns`-form-only, no parseable ns form → `None`, caller must not guess),
  `alias/var` and `::alias/var` references → the namespace → file tails
  (dots → `/`, hyphens → `_`, `.clj` / `.cljc` / `.cljs`). SOFT layout
  semantics: an empty intersection degrades to the bare name-keyed
  superset (never an empty answer where the name is indexed); a dotless
  alias the file's `ns` form does not declare is FLAGGED (`cannot
  resolve namespace alias …`), never guessed. Pinned:
  `xref_clojure_namespace_alias_jumps_to_var`,
  `xref_clojure_unresolvable_alias_is_flagged`,
  `ns_form_alias_and_identity_entries` (clojure.rs).
- **Java — works (unit).** A single-type `import a.b.C` binds `C` →
  `a/b/C.java` (JLS §7.6 — the pinned grammar holds the whole path as
  ONE `scoped_identifier`); a qualified type reference WITHOUT an import
  rides the same convention. HARD semantics: the import placed the name
  in a file the index does not hold → the named `unresolved` flag
  (below), not a same-named-file jump. Pinned:
  `xref_java_import_narrows_to_convention_file_and_lands`,
  `xref_java_qualified_type_reference_uses_convention_without_import`,
  `xref_java_field_access_is_unresolved_not_misrouted`,
  `single_type_import_binds_simple_name_to_fqn` (java.rs). Wildcard and
  static-member imports: no convention row — they stay on the name-keyed
  lookup.
- **C / C++ — works (unit).** A QUOTED `#include "a/b.h"` → the include's
  path (with the directory join — `sub/thing.h`, never `thing.h`, which
  is what keeps a same-base-name decoy in another directory out) as a
  tail, matched against the index's name-keyed candidates (the
  quoted-include search order: the including file's directory / source
  roots). ANGLE includes are excluded **by construction**, not by
  convention: `#include <a.h>` is a DIFFERENT node kind
  (`system_lib_string` — a leaf with no content child), so the `-I` /
  system path simply is not in the tree to extract; an angle-only name
  stays on the `unresolved` flag (no C/C++ tooling provider). Pinned:
  `xref_c_quoted_include_narrows_to_convention_header_and_lands`,
  `xref_cpp_quoted_include_narrows_to_convention_header_and_lands`,
  `xref_c_angle_include_is_unresolved_not_a_guess`,
  `c_quoted_includes_are_the_carrier_angle_is_excluded` (c_cpp.rs).
- **Go — works (unit).** An in-module `import "a/b/c"` (plain / aliased /
  dot / blank forms) → the module-relative package DIRECTORY; the module
  path is read from the repository's own `go.mod` (the nearest go.mod
  walking up from the file, BOUNDED to the workspace root so a stray
  parent go.mod cannot capture a file) — offline, no toolchain; the
  import alias shadows the last path segment. An import OUTSIDE the
  module path is FLAGGED (a third-party dependency — no toolchain, no
  guess); cross-module / download resolution stays on the `go` tooling
  provider (toolchain-gated — no `go` on this box, audit F4). Pinned:
  `xref_go_local_import_narrows_to_module_directory_and_lands`,
  `xref_go_aliased_import_lands_in_the_module_directory`,
  `import_shapes_bind_per_the_grammar` (go.rs).
- **Ruby — IN FLIGHT in a sibling lane** (plan 017 issue 06,
  `require_relative` → the sibling `.rb`; the `?`/`!` constituent rule it
  rides on has landed — see the word rule above). Not claimed here.
- **Not landed — the flag rows** (plan 017 issue 08, by design
  "flag, don't guess"): C# (no directory convention — a convention row
  would be a guess), Scheme (library layout implementation-defined),
  Ruby bare `require` ($LOAD_PATH), C++ semantic forms (ADL / templates /
  using-directives), Python relative `from .` (the sys.path root is
  unknown); plus Bash relative `source` / `.` (plan 017 issue 07). None
  of these is claimed by this file.

**The unresolved flag (plan-017 B5) — every language without a tooling
provider.** When the point sits on a token that is NOT the enclosing
symbol's own name, the index misses, and no tooling provider handles
the buffer's language, M-. reports `unresolved: \`<name>\`` — a named
flag (the M-. analog of the annotations' `orphaned`): no picker, no
jump, no enclosing-symbol fallback. The audit's misroute (M-. on a C
function called inside `main` opened a picker on `main`) is gone. The
enclosing fallback stands byte-for-byte in its two legitimate shapes —
the point on the enclosing symbol's OWN NAME (column-precise via
`point_on_symbol_name`, compared through the per-language word rule) —
or the point carrying NO token at all (a blank / comment point keeps
the by-line "jump to the enclosing function" press). When a provider
DOES handle the language (or the extension is unknown — the chain keeps
its in-order walk), the point's own token goes to the tooling seam
instead: tooling stays authoritative over the flag (an in-function
`tokio::spawn` resolves through cargo, it does not report
unresolved). Pinned: `xref_b5_c_call_in_main_is_unresolved_not_a_picker`,
`xref_b5_point_on_enclosing_name_keeps_the_picker`,
`xref_c_field_access_is_unresolved_not_misrouted`,
`xref_java_field_access_is_unresolved_not_misrouted`.

**Fetch confirmation — and a refusal now says WHY (F2).** No provider
ever installs without the operator: an install step (`cargo fetch`,
`pip install`, `npm install`) runs only after the input-path banner
`fetch on demand: <command> (from <file>) (y/n)?` — `y` approves the
exact command on screen, `n` / C-g / ESC decline, and a stale or
superseded ask is declined WITHOUT a banner (no unconfirmed install,
ever; a missing confirmation hook REFUSES — fail-safe, not fail-open).
A refusal now carries the reason, and the reason LEADS the report (F2,
`ba70fa8`): the provider's own miss detail — e.g. `install refused:
\`pip install x\` was declined at the fetch confirmation (nothing was
installed)`, `not an npm project`, `no go.mod under workspace root …` —
leads the `no provider resolution for …` line (a single-attempt chain
surfaces its provider's own reason; a multi-provider walk names the
provider on an `install refused` detail; unrelated multi-provider bails
keep the generic shape). Pinned: `confirm_or_refuse` (crates/
redline-resolve), `apply_fetch_prompt` / `fetch_confirm_key` (src/app/
store/navigation/definitions.rs), `fetch_declined_refuses_pip_install`,
`fetch_accepted_runs_stub_pip_exactly_once`,
`fetch_stale_ask_is_declined_without_a_banner` (tests/navigation/
receiver.rs).

## In-project M-. vs cross-project M-. (reading the grid)

The outline column is the floor: for every language with a definition
query (16 of the 18 grammar-bearing languages — JSON and YAML carry
`definition_query: None` deliberately, issue-json-yaml-no-symbols, and
Plain has no grammar at all), definitions **inside the opened project**
are
M-. targets through the project index (the project walk has no
extension filter — provider-matrix, C/C++ section). Everything in the
right half of the grid (path-shaped, bare-via-import, provider,
in-library follow-up) is about M-. **outside** the project index, and
is where languages genuinely diverge. The mechanisms that never got a
grid column (the Rust field/trait tables, imenu impl-parent grouping,
`M-?` references, the inert `injections_query`, the per-language word
rule, the plan-017 import conventions, the B5 unresolved flag, and the
fetch confirmation) are measured in the section after the
language-agnostic row below.

## Gaps (prioritized — what is missing per language)

1. **C / C++: no provider** (the node predicates + scope walk landed
   with the lang-pred lane). `node_at`/`scope_path_at` now answer for
   C/C++ (`c_member_path_comes_back_whole`,
   `cpp_qualified_path_comes_back_whole`,
   `c_scope_chain_struct_in_struct_and_function`,
   `cpp_scope_chain_namespace_class_method` — node.rs), and
   `unimplemented_languages_return_none` now covers only Yaml + Plain.
   What still bails: no tooling provider exists, so M-. on a symbol
   outside the project index degrades to the 011-01 bail (only
   in-project definitions resolve). The extraction side is done since
   010-rung4-and-paths: `dotted_path_container` enumerates the C/Cpp
   `field_expression` container, so path-shaped M-. carries the whole
   path (`o.x`, unit-pinned) — only the resolution side is absent. The 011-07 walk sets (`c/h`,
   `cc/cpp/cxx/hh/hpp/hxx`) already make a *landed* C/C++ tree
   indexable — but no provider can ever produce the landing.
   Highest-value gap if C/C++ source navigation is a goal; the index
   side and the syntax side are done, the resolution side is absent.
   *Addendum (plan 017): the in-workspace half has since LANDED — the
   QUOTED `#include` resolves through the conventions mechanism
   (017-04; the angle-include limit is STRUCTURAL: `system_lib_string`
   is a leaf with no content child, so the `-I` / system path is not in
   the tree to extract), and the enclosing-fallback misroute is gone
   (B5 — an unresolvable name in a function reports the named
   `unresolved` flag instead of a picker on the enclosing symbol; see
   the mechanism rows above). What still stands: no C/C++ tooling
   provider, and the C++ semantic forms stay flagged.*
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
8. **Clojure: lane deferred — no usable grammar crate.**
   *Addendum: since landed by the ts-bump lane — see the runtime note
   above; the probe evidence below is historical.* The new-languages lane (the user directive "add clojure and lisp
   support, java, c#, ruby") first round (offline-first) deferred
   everything: none of the needed crates were vendored in
   `~/.cargo/registry` and mid-task network fetches were disallowed.
   Re-run under the amended contract (crates.io sanctioned,
   ABI-pinning rules as the real constraint): **Java**
   (`tree-sitter-java =0.23.5`), **C#** (`tree-sitter-c-sharp
   =0.23.1`), **Ruby** (`tree-sitter-ruby` 0.23.1), and **Scheme**
   (the lisp family; `tree-sitter-scheme` 0.24.7) all landed — every
   one grammar ABI 14, `set_language` probe-verified against the pinned
   0.24.7 runtime, staged registry/queries/node/walk-set work per the
   capability ladder, NODE_TYPES probe first. The pinned Scheme grammar
   is a flat S-expression parser — no `defun`/`defvar` node kinds
   exist, so its outline captures every first-child-anchored
   `(list HEAD …)` candidate (the grammar's `symbol` kind refuses
   literal content matches at query compile time — probe-verified)
   and keeps only the true define forms (the query's `.` anchors are
   load-bearing, probe-verified); its module paths are `list` nodes,
   so node-at / scope are the honest N/A. **Clojure stays deferred on ABI evidence**:
   `tree-sitter-clojure` 0.1.0 requires tree-sitter `^0.25.6` (cargo
   `links` conflict with the pinned runtime — does not even resolve),
   `tree-sitter-clojure-orchard` 0.2.x requires `^0.25.9`/`^0.26.11`
   (same), and `arborium-clojure` 2.18.2 resolves but its grammar is
   ABI 15 → `set_language` fails (all three probe-verified); the
   elisp-family crate (`tree-sitter-elisp` 1.7.2) is likewise ABI 15 →
   `set_language` fails. Re-landing Clojure needs a grammar crate whose
   parser targets the pinned runtime — or a deliberate runtime bump,
   which is a separate ABI-pinning decision, not this lane's.
   *Addendum (plan 017): the RESOLUTION half has landed too — the
   Clojure alias/namespace conventions ride the shared conventions
   mechanism (the ns-form alias map; `alias/var` and `::alias/var` →
   namespace → file tails; an undeclared dotless alias is flagged, never
   guessed — see the "Import / alias / module resolution" row above).*

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
4. **The app-side whole-path upgrade deliberately stops short of some
   containers.** `store.rs::dotted_path_container` enumerates JS/TS/Tsx
   (`member_expression`, `nested_type_identifier`, `nested_identifier`),
   Python (`attribute`), Go (`selector_expression`, `qualified_type`),
   C / Cpp (`field_expression`), Toml (`dotted_key`) — 011-06 +
   010-rung4-and-paths — and Java (`field_access`,
   `scoped_identifier`, `scoped_type_identifier`), CSharp
   (`member_access_expression`, `qualified_name`), and Ruby's
   argumentless `call` — newlang-paths. Deliberately NOT enumerated:
   Cpp's `qualified_identifier` and Ruby's `scope_resolution`
   (`Foo::Bar`) — `symbol_at_point`'s `::` byte-scan already carries
   those whole, so enumerating them would be byte-for-byte double
   handling; and an argument-carrying Ruby `call` never reaches the arm
   (node.rs's receiver/no-`arguments` gate + the caller's
   all-identifier-segment guard — `a.b(1).c` degrades byte-for-byte,
   `symbol_at_point_java_csharp_ruby_degradation_stays_bare`).
   (JSON/Bash/Markdown/Scheme have no dotted path container to
   enumerate in the first place.)
5. **The js provider's `jsx` dispatch string is dead (audit F1).**
   `JsProvider::languages()` is
   `["javascript","typescript","tsx","jsx"]`, but no registry row is
   named `"jsx"` — jsx FILES map to the JavaScript row (its extension
   list includes `jsx`), so the `"jsx"` string never matches a
   dispatch. The provider's effective coverage is the JavaScript /
   TypeScript / Tsx rows; the dead string is harmless, but the
   dispatch table names a language that does not exist.
6. **`.mdx` files map to the Markdown grammar (audit F3).** The Markdown
   row's extensions include `mdx`, so an MDX `import X from "…"` line
   parses as a plain `paragraph` — MDX imports are invisible to redline
   (the Markdown grammar has no import construct). Known limitation, not
   a plan-017 work item (MDX as its own language is out of scope).
7. **A Scheme library's name is indexed under its FIRST name-list
   symbol only (audit F6).** `(define-library (foo core) …)` keys the
   outline under `foo`, dropping the library path `foo.core` (the flat
   grammar has no path-shaped node for it). Any future Scheme import
   work must re-key to the full library path.

## What this file deliberately does NOT claim

No per-language performance numbers, no LSP (ruled out — plan 011),
no macro expansion / generics inference / arbitrary-expression typing.
For the provider-resolution side (miss dispatch, relative imports,
src/-layout discovery, the live drive inventory), see
`docs/provider-matrix.md`.

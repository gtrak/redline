# 017-00 — the capability audit, measured

**Base:** `9f8f2e7` (worktree slot 2, branch `plan-017-audit`). **Offline, side-effect-free.** No file
under `src/` or `crates/` was modified; all probes ran through one throwaway example
(`examples/audit017_dump.rs`, deleted after this audit) and one throwaway PTY driver under
`/tmp/audit017/`. Fixtures live in `/tmp/audit017/`.

**Method.** Three measurement layers, matching the plan's three layers:

1. **Grammar layer** — for every registered language, a real import/alias statement was written to a
   fixture file and parsed with the pinned grammar; the dump quotes (a) the structural sexp and
   (b) a text-annotated subtree (`kind [start..end] "text"`). All 18 grammar-bearing languages
   parsed with `has_error = false`.
2. **Tooling layer** — the EXACT app provider chain (`CargoProvider` → `JsProvider` →
   `PythonProvider` → `GoProvider`, the registration order of `start_symbol_resolution`,
   `definitions.rs:1783-1787`) was run with the `SymbolContext` the app builds for each buffer
   (language dispatch string, scope hint, `confirm_fetch = None` = the headless refusal). This is
   what M-. does on a workspace miss, minus the async wrapper.
3. **End-to-end (PTY)** — one 7-leg battery (`/tmp/audit017/pty_drive.py`, fresh App per leg)
   against a fixture git repo: Rust control, Clojure plain-var, Clojure alias+hyphen, Python
   stdlib import, C header-declared call, C header macro, Ruby `?`-method. **7/7 legs passed**;
   the results are the "resolves today?" column for the languages they cover, and every other
   language's miss path is the layer-2 measurement (the async wrapper adds nothing to the message).

Toolchain state on this box (measured): `cargo` ✓, `python3` 3.14.4 ✓, `node` 24.13.1 ✓, `go` ✗
(not installed). `node_modules` hits were measured against a fabricated
`/tmp/audit017/jsproj/node_modules/acme-lib` (offline fixture — the js provider only shells out to
`npm` for *installs*, which the `confirm_fetch = None` refusal blocks anyway).

---

## Measured counts (not the claims)

| quantity | claim (PLAN.md) | measured |
|---|---|---|
| registered languages | 19 | **19** — `LanguageId` has 19 variants and `LANGUAGES` is `[LanguageSpec; 19]` (`crates/redline-syntax/src/language.rs`); 18 carry a grammar, `Plain` does not (`language::parseable`) |
| tooling providers | 4 | **4** — `cargo.rs` (`["rust"]`), `providers/python_provider.rs` (`["python"]`), `providers/js_provider.rs` (`["javascript", "typescript", "tsx", "jsx"]`), `providers/go_provider.rs` (`["go"]`) |
| languages with a provider | "4 of 19" | **6 of 19 rows**: Rust, JavaScript, TypeScript, Tsx, Python, Go — the js provider covers 4 rows. Its `"jsx"` string is **dead**: `resolution_language` (`definitions.rs:1778`) passes the row's `name()` (the JavaScript row is named `"javascript"`); no row is named `"jsx"` (jsx *files* map to the JavaScript row) |
| languages with a scope-hint extractor | — | **6 of 19 rows**: Rust (`use_path_for_symbol`), JS/TS/TSX (`js_ts_scope_for`), Python (`python_scope_for`), Go (`go_scope_for`) — all in `imports.rs`; every other language's `SymbolContext.scope` is always empty |

So the PLAN.md figures hold, with one refinement: provider coverage is 6 language rows, not 4
languages, because the js provider spans TS/TSX. And every provider still **shells out**
(`cargo`, `python`/`pip`, `npm` (install-only), `go` — `Command::new` at `cargo.rs:86,124`,
`python_provider.rs:121,352`, `js_provider.rs:176`, `go_provider.rs:94,113`): toolchain-dependent,
as the plan states.

---

## The matrix (one row per registered language)

"Resolves today?" is measured, per the legend: **WS-HIT** = workspace-index hit (picker or silent
jump); **TOOL-HIT** = provider hit; **BAIL** = `no tooling provider handles language …` (measured,
layer 2); **MISROUTE** = lands somewhere it should not.

| # | language (name) | import / alias / module **forms** | **resolves today?** (measured) | **convention rule** (name → path) + citation | **decidable?** | **plan-013 work** (concrete first step) |
|---|---|---|---|---|---|---|
| 1 | **rust** | `use a::b::{C as D};` (grouped, aliased, nested `sub::{…}`), `use crate/self/super` (same-crate), edition-2018 absolute paths | **WS-HIT** (L1: cross-file unique `Alpha` → picker on `src/alpha.rs`). External: **TOOL-HIT** — `serde::Deserialize` + scope → `~/.cargo/registry/src/…/serde-1.0.229/src/core/de/mod.rs:554` (offline, lock+cache) | crate name → registry source dir via `cargo metadata` (layer 3, measured). In-workspace: module path → file is name-keyed indexing, not a path convention | yes (layer 3 + workspace index) | **Done** — control lane. Nothing to do; keep as the regression control. |
| 2 | **typescript** | `import { X as Y } from 'pkg'`, `import * as ns`, `import def from`, `import "pkg"` (side-effect), `export { x } from 'pkg'`, `require('pkg')` | **WS-HIT** intra-workspace (index); external **TOOL-HIT** (js provider: bare+scope → `node_modules/acme-lib/index.js:1`; namespace-rewrite `ns.helper`+scope → `:2`; relative `./rel-mod` → sibling file, `external=false`) | package spec → `node_modules/<pkg>` + `package.json` `main`/`exports` (Node/ECMAScript module resolution); relative `./x` → sibling file with extension walk (measured: `rel-mod.ts` hit) | yes (provider) + partly (TS `paths`/`baseUrl` remaps need the tsconfig — flag) | **Done** via provider; add nothing except the `$` constituent fix (sibling lane). |
| 3 | **tsx** | identical to typescript (same `import_statement` nodes — dump quoted below) | same as typescript (provider `languages()` includes `"tsx"`) | same as typescript | yes | **Done**; row is documentation. |
| 4 | **javascript** | ESM forms above + CJS `const m = require('pkg')`, `const { x } = require('pkg')` | **WS-HIT** intra-workspace; external **TOOL-HIT** (measured as typescript). **Defect**: `$` — the grammar gives ONE `identifier "$dollar"` (dump below) while `is_word_char` (alnum + `_`) splits it, so M-. on `$dollar` searches `dollar` (recorded, filed in the bug list; sibling-lane fence) | same as typescript | yes (provider) | constituent fix (sibling lane), nothing else. |
| 5 | **python** | `import a.b` (binds only `a`), `import a.b as c`, `from a import b [as c]`, `from . import x` (relative — never hinted), `from a import *` (never hinted) | **WS-HIT** intra-workspace; **TOOL-HIT** — `json.dumps` → `/usr/lib/python3.14/json/__init__.py:185` (L4 end-to-end: Definition picker with the stdlib tooling row); bare `ODict`+scope → `collections/__init__.py:89`. Missing pkg: **flag, no install** — `install refused: pip install missing_pkg_xyz requires a fetch confirmation hook` (measured) | `a.b` → `a/b/__init__.py` or `a/b.py` (standard/flat layout, PEP 3147; PEP 420 namespace packages — a dir WITHOUT `__init__.py` also resolves; measured on this box: `/usr/lib/python3.14/json/__init__.py`, `…/collections/__init__.py`). Layer 3 does the real work (`importlib.find_spec` in a timed subprocess) | yes (installed/stdlib); partly — dynamic `importlib`/`__import__`, relative `from .` (sys.path root unknown), namespace-package subtleties → flag | **Done** via provider. The layer-2 addition would be the *relative-import* root (a `src/` layout hint) — cheap, but low value while layer 3 works. |
| 6 | **go** | `import "a/b/c"`, `import x "a/b/c"` (aliased), `import . "a/b"` (dot — binds bare names), `import _ "a/b"` (blank), C `import "C"` | Intra-workspace **WS-HIT** (index). Tooling: **BAIL-ish** — no go.mod → `no go.mod under workspace root …; not a Go module project`; with go.mod → `package b is not a required module in go.mod`. Beyond go.mod parsing the provider shells out to `go` — **absent on this box**, so the download path is untestable here (finding F5) | import path = module path + **directory path** inside the module (Go spec "Import Declarations"; the post-module-prefix path must match the source directory name) — within a module this is a pure convention (module root → path); cross-module = layer 3 (`go mod`) | yes in-module (pure convention); partly cross-module (needs `go`) | add the in-module path→directory convention (cheap, offline-testable); record the toolchain gap. |
| 7 | **c** | `#include "x/y.h"` (quoted), `#include <x.h>` (angle), `#define NAME` (macro = the only "import-like" name a header contributes) | **Measured, and the shape is surprising** (L5): a header-*declared* function called in the workspace **MISROUTEs** — M-. on `thing_x` (declared in `sub/thing.h`, called inside `main`) opens the **enclosing-fallback picker on `main`**, not a jump to the header and not a no-provider bail (the enclosing fallback, `xref.rs` step 3, always swallows the resolver path for in-function calls). A header *macro* DOES resolve: M-. on `MAX` → WS-HIT on `sub/thing.h` (L5b). Outside any function, the layer-2 measurement stands: `BAIL` — `no tooling provider handles language c (4 provider(s) registered, none attempted)` | quoted `#include "x/y.h"` → **the including file's directory first**, then the `-I` search path; `<x.h>` → the system include dirs (ISO/IEC 9899 7.12 / "6.10.2" preprocessing search order; GCC `#include` docs). Measured anchor: `/usr/include/stdio.h` exists; quoted relative-to-includer was exercised by the L5b fixture (workspace hit on `sub/thing.h` next to `imp.c`) | partly — quoted-includes relative to the includer are decidable; `-I`/system angle-bracket includes and macro expansion are build-dependent → **flag** | (1) quoted-include relative resolution (query on `preproc_include`/`string_literal` — shape measured below) + flag for angle/system; (2) **decide what to do about the enclosing-fallback misroute** (bug list B5) — it makes the C "flag" path unreachable for the common in-function case |
| 8 | **cpp** | `#include` (both forms), `namespace my = myns;` (alias), `using namespace foo;` (using-directive), templates, ADL | **BAIL** (layer-2 measured: `no tooling provider handles language cpp …`); enclosing-fallback misroute applies as in C | header forms same as C (quoted relative-to-includer decidable). Everything else — ADL, templates, using-directives, namespace aliases — needs semantic resolution | **no** for ADL/templates/using; partly for headers | headers: as C. Everything else: **flag, don't guess** (documented non-resolution, `orphaned`-style). |
| 9 | **toml** | none — no import/alias/module concept; `[a.b]` table headers + dotted keys are in-document structure | N/A: nothing to resolve; in-file M-. on a dotted key works today (the index stores `a.b` whole — `extract_symbols` measured — and the 011-06 `dotted_key` container upgrade carries the token) | n/a (TOML v1.0: no cross-file mechanism) | n/a | **N/A row** — no work. (If the user wants cross-table jump within one file, that is imenu/index territory, already present.) |
| 10 | **json** | none (JSON has no imports; the grammar has no import construct) | N/A: JSON keys are not navigation targets (documented: `definition_query: None`, `issue-json-yaml-no-symbols`) | n/a | n/a | **N/A row.** |
| 11 | **yaml** | anchors/aliases (`&a` / `*a`) are **in-document only** — no cross-file import in the YAML spec | N/A: same as JSON (no symbols indexed) | in-document anchor→alias is decidable but out of scope (not an import); no cross-file convention | n/a | **N/A row.** (An anchor/alias in-file jump could be a small separate issue; not a plan-017 gap.) |
| 12 | **bash** | `source ./sub/helper.sh`, `. ./other.sh` (the `.` special builtin), `source helper` (bare → `$PATH` search) | **BAIL** (layer-2 measured). In-file function M-. works (index stores `helper_fn`) | POSIX `.`: a *relative or absolute path* resolves relative to the current directory (in practice, the script's dir for non-interactive scripts under some shells — build-dependent); a **bare name is a PATH lookup → not statically decidable** (bash manual, `source`/`.` entries: "FILE is searched for in $PATH") | partly — `./…`/absolute relative-to-script decidable; bare → **flag** | tiny: relative `source`/`.` resolution (dump measured below); bare names flag. Low value — rank accordingly. |
| 13 | **markdown** | none in the language. **Finding F3**: `.mdx` files map to the *Markdown* row (the `mdx` extension), so MDX `import X from "…"` lines parse as plain `paragraph`s (dump below) — MDX imports are invisible to redline | N/A for md; **finding** for mdx | n/a | n/a | **N/A row** + record the mdx mapping as a known limitation (or a future MDX row — out of scope). |
| 14 | **java** | `import a.b.C;`, `import a.b.*;` (wildcard), `import static a.b.D.m;`, `package com.example;` | **BAIL** (layer-2 measured: `no tooling provider handles language java …`); intra-workspace name-keyed M-. works for classes/methods the index captured (`T`, `use`) | `a.b.C` → `a/b/C.java` under a **source root** (JLS §7.6 "Package and Class Names"; javac resolves binary names to path names this way). Measured grammar shape: `import_declaration` → `scoped_identifier "a.b.C"` (quoted below). **Source root is the open variable**: no tooling provider exists, so the root must come from project layout (repo root, `src/`, `src/main/java` Maven layout) — a heuristic, flagged as such | **partly** — single-type imports decidable once a root is chosen; wildcards and static-member imports → flag; no classpath/jar resolution (no toolchain by design) | a conventions entry (`a.b.C` → `a/b/C.java`, extension `.java`) + source-root heuristic + import-declaration query on the measured node shape. Rank after Clojure (lower user value, still pure and cheap to verify). |
| 15 | **csharp** | `using System;`, `using Alias = A.B.C;` (alias directive), `using Ns1;` | **BAIL** (layer-2 measured) | **There is no directory convention**: C# files may live anywhere in the project (Roslyn/csc take explicit file lists; namespace ≠ directory, unlike Java) — so name→path is **undecidable by convention**. The only correct resolution is a project-wide "find the declaration of this fully-qualified name" search, which the plan explicitly excludes (non-goal: "Not a general find-symbol-by-name search") | **no** | **Flag row** — document "cannot be resolved without project semantics; M-. bails". Do not build a convention table entry that guesses. |
| 16 | **ruby** | `require 'a/b'`, `require_relative 'c'`, `autoload :D, 'd/e'`, `load` (dynamic), metaprogramming (`const_missing`, `eval`) | **Measured defect (L6)**: the index stores the method **whole** (`empty?`, `save!` — `extract_symbols` output) but M-. on `empty?` extracts `empty` (`?` is not in `is_word_char`), so the workspace lookup misses and the jump bails `no tooling provider handles language ruby` — the user's own in-workspace code fails to jump (bug B3, filed). **BAIL** for external (layer-2 measured) | `require_relative 'c'` → `c.rb` **relative to the requiring file** (ruby-lang.org `Module#require_relative` docs; the Ruby spec's `require_relative` semantics) — decidable. `require 'a/b'` → `$LOAD_PATH` search (gem `lib/` roots under Bundler; env-dependent) → **partly**: decidable per gem only with tooling. `autoload`, `const_missing`, stringly metaprogramming → **no** | partly (`require_relative` yes; bare `require`/autoload/metaprogramming no) | two steps: (1) the `?`/`!` constituent fix (sibling-lane fence — prerequisite, already filed there); (2) `require_relative` convention (pure, cheap, offline-testable) + flag the rest. |
| 17 | **scheme** | R6RS `define-library`/`(import (library (foo core) …))`/`(export …)` (the pinned flat grammar sees them as bare `list`s of `symbol`s — dump below) | **BAIL** (layer-2 measured). In-file define M-. works (index: `f`, `g`; library name `foo`) | **Implementation-defined**: R6RS/R7RS mandate the *library form*, not its filesystem layout; Chicken/Gambit/Racket each map libraries to files differently (and Racket has no `.scm` convention at all). No toolchain provider exists | **no** (statically, without a toolchain) | **Flag row** — "(library (foo core)) cannot be mapped to a file portably; M-. bails". Note finding F6: the index keys a `(define-library (foo core))` under `foo` only (the first name-list symbol), dropping the library path `foo.core` — a small correctness note for any future Scheme work. |
| 18 | **clojure** | `(ns my.app (:require [some.ns :as jwks] [other.ns :refer [thunk]]) (:import java.util.Date))`; references `jwks/fetch-issuer-info` (namespaced var) and `::jwks/local-opt` (keyword auto-resolve) | **Measured end-to-end (L2/L3)**: a plain cross-file var `(thunk)` → WS-HIT picker on `some/ns.clj`; the reported case `(jwks/fetch-issuer-info 1)` with the point inside `fetch-issuer-info` → extraction yields `issuer` (hyphen split, CUP (3,19) = end of `issuer`) → index miss (the index stores `fetch-issuer-info` whole) → **BAIL** `no tooling provider handles language clojure` (L3). Grammar shape (measured): `jwks/fetch-issuer-info` is ONE `sym_lit` with fields `sym_ns "jwks"` + `sym_name "fetch-issuer-info"`; `::jwks/local-opt` is ONE `kwd_lit` with `::` + `kwd_ns` + `kwd_name`; the `ns` form's `:require` vecs carry `sym_lit` (the namespace, whole, dots inside `sym_name`) + `:as` + alias `sym_lit` | `foo.bar-baz` → `foo/bar_baz.clj`, also `.cljc`/`.cljs` — dots → directory separators, **hyphens → underscores**. **Verified against real project layouts on this box, not assumed**: `kaocha.core-ext` ↔ `src/kaocha/core_ext.clj` and `kaocha.matcher-combinators` ↔ `src/kaocha/matcher_combinators.clj` (kaocha source tree); jar layout `lambdaisland/tools/namespace/reload.clj` inside `tools.namespace-0.3.256.jar` (dots→slashes); kaocha ships `.clj` **and** `.cljc` (parse/track modules). The `src/` (or `test/`) prefix is project-local, so the convention yields **path tails** to match against known project files — never an absolute guess | **yes** — a pure, toolchain-free convention, and the only language where the user's own report is the gap | **THE first wiring issue** (rank 1): (1) an ns-form alias-map extractor (the shape is the quoted dump; the sibling lane has an uncommitted `ns_aliases()` doing exactly this — see "Sibling lane"); (2) the conventions-table entry (namespace → tail, `.clj`/`.cljc`/`.cljs`); (3) `alias/var` + `::alias/var` reference extraction (node shapes measured above). **Prerequisite**: the sibling lane's constituent fix (bug B1) — until it lands, M-. never extracts the whole `jwks/fetch-issuer-info` unit and the wiring is untestable end-to-end. |
| 19 | **plain** | none (no grammar) | unknown-extension files: `resolution_language` returns `None` → the chain walks ALL providers in order (layer-2 measured: `no tooling provider could resolve symbol ghost_fn (tried 4 provider(s): rust, javascript, python, go)`) | n/a | n/a | **N/A row** — behavior is honest (walk everything, bail). |

### Both-directions pin (per the audit rule)

- **Should resolve, lands right**: Rust `Alpha` (L1 picker), Clojure `thunk` (L2 picker), Python
  `json.dumps` (L4 stdlib tooling row), JS `hello`/`ns.helper`/`./rel-mod` (layer 2, exact lines),
  C `MAX` (L5b header hit), Java/Ruby/C#/Go/TOML/Bash/Markdown in-file definitions (index,
  `extract_symbols` measured per language).
- **Should NOT resolve, and is flagged (not mis-resolved)**: every no-provider language bails with
  the language-named message (layer-2 measured, verbatim above); Python never installs without a
  confirmation hook (`install refused` — measured); JS bails with a named reason when the project
  has no `package.json`; Go bails on missing go.mod / unrequired package; the C **misroute** (B5)
  is the one place a "should-not" case lands *somewhere* — filed, not fixed.

---

## Quoted grammar dumps (the evidence)

Fixtures: `/tmp/audit017/<lang>/…` (contents above in the matrix rows). All parsed
`has_error = false`. Structural sexp first (one line, exactly as the probe printed it), then the
import-relevant annotated fragments.

### rust — `use serde::{Deserialize as D, Serialize}; use std::collections::{BTreeMap, VecDeque as VD};`

```
(source_file (use_declaration argument: (scoped_use_list path: (identifier) list: (use_list (use_as_clause path: (identifier) alias: (identifier)) (identifier)))) (use_declaration argument: (scoped_use_list path: (scoped_identifier path: (identifier) name: (identifier)) list: (use_list (identifier) (use_as_clause path: (identifier) alias: (identifier))))) (struct_item (visibility_modifier) name: (type_identifier) body: (field_declaration_list (field_declaration (visibility_modifier) name: (field_identifier) type: (primitive_type)))) (function_item name: (identifier) parameters: (parameters) body: (block …)))
```

```
  use_declaration [0..48] "use serde::{Deserialize as D, Serialize};"
    scoped_use_list
      path: (identifier) "serde"
      list: (use_list
        (use_as_clause path: (identifier) "Deserialize" alias: (identifier) "D")
        (identifier) "Serialize")
```

### javascript — `import { greet as hello, default as d } from 'acme-lib'; import * as ns from 'acme-lib'; import rel from './rel-mod';`

```
(program (import_statement (import_clause (named_imports (import_specifier name: (identifier) alias: (identifier)) (import_specifier alias: (identifier)))) source: (string (string_fragment))) (import_statement (import_clause (namespace_import (identifier))) source: (string (string_fragment))) (import_statement (import_clause (identifier)) source: (string (string_fragment))) (variable_declaration (variable_declarator name: (identifier) value: (call_expression function: (identifier) arguments: (arguments (string (string_fragment)))))) (function_declaration name: (identifier) parameters: (formal_parameters) body: (statement_block …)))
```

```
  import_statement [0..56] "import { greet as hello, default as d } from 'acme-lib';"
    import_clause
      named_imports
        import_specifier [9..23] "greet as hello"   (name: identifier "greet", alias: identifier "hello")
        import_specifier [25..37] "default as d"     (name: default "default", alias: identifier "d")
    source: (string "'acme-lib'" (string_fragment "acme-lib"))
  import_statement [57..88] "import * as ns from 'acme-lib';"
    namespace_import [64..71] "* as ns"   (identifier "ns")
  import_statement [89..117] "import rel from './rel-mod';"
    import_clause: identifier "rel"      source: string_fragment "./rel-mod"
  lexical_declaration [224..242] "const $dollar = 1;"
    variable_declarator [230..241] "$dollar = 1"
      identifier [230..237] "$dollar"            ← ONE node; redline's word rule splits it (bug B2)
```

### typescript / tsx — `import { greet as hello } from 'acme-lib';`

```
(program (import_statement (import_clause (named_imports (import_specifier name: (identifier) alias: (identifier)))) source: (string (string_fragment))) …)
```

TSX (same fixture, TSX grammar) produces the **same** `import_statement`/`import_clause`/
`named_imports`/`import_specifier`/`namespace_import`/`source` shape (dumped and diffed: identical
for the import prefix). The `$dollar` single-`identifier` shape holds in TS/TSX too.

### python — `import os.path / from collections import OrderedDict as ODict / import json`

```
(module (import_statement (dotted_name (identifier) (identifier))) (import_from_statement (dotted_name (identifier)) (aliased_import (dotted_name (identifier)) (identifier))) (import_statement (dotted_name (identifier))) (function_definition …))
```

```
  import_statement [0..14] "import os.path"
    dotted_name [7..14] "os.path"   (identifier "os", ".", identifier "path")
  import_from_statement [15..59] "from collections import OrderedDict as ODict"
    dotted_name [20..31] "collections"
    aliased_import [39..59] "OrderedDict as ODict"
      dotted_name "OrderedDict"  as  identifier "ODict"
```

### go — dot/aliased/plain imports

```
(source_file (package_clause (package_identifier)) (import_declaration (import_spec_list (import_spec (dot) (interpreted_string_literal (interpreted_string_literal_content))) (import_spec (package_identifier) (interpreted_string_literal (interpreted_string_literal_content))) (import_spec (interpreted_string_literal (interpreted_string_literal_content))))) (function_declaration …))
```

```
  import_declaration [14..49] "import (? . \"a/b\"? x \"a/c\"? \"a/d\"?)"
    import_spec [24..31] ". \"a/b\""        (dot + interpreted_string_literal_content "a/b")
    import_spec [33..40] "x \"a/c\""        (package_identifier "x" + content "a/c")
    import_spec [42..47] "\"a/d\""          (content "a/d")
```

### c — `#include "sub/thing.h" / #include <stdio.h> / #include "gen/missing_xyz.h"`

```
(translation_unit (preproc_include (#include) (string_literal (string_content))) (preproc_include (#include) (system_lib_string)) (preproc_include (#include) (string_literal (string_content))) (function_definition …) (function_definition …))
```

```
  preproc_include [0..23] "#include \"sub/thing.h\""
    string_literal [9..22] "\"sub/thing.h\""   (string_content "sub/thing.h")
  preproc_include [23..42] "#include <stdio.h>"
    system_lib_string [32..41] "<stdio.h>"     ← quoted vs angle is a DIFFERENT node kind
```

### cpp — `#include <vector> / #include "local/a/b.h" / namespace my = myns; / using namespace foo;`

```
(translation_unit (preproc_include (#include) (system_lib_string)) (preproc_include (#include) (string_literal (string_content))) (preproc_include (#include) (string_literal (string_content))) (namespace_alias_definition …) (using_declaration …) (template_declaration …) (function_definition …))
```

```
  namespace_alias_definition [71..91] "namespace my = myns;"
  using_declaration [92..112] "using namespace foo;"
```

### toml — `[a.b] / [a.b.c] / [dotted.key]`

```
(document (table (dotted_key (bare_key) (bare_key)) (pair (bare_key) (integer))) (table (dotted_key (dotted_key (bare_key) (bare_key)) (bare_key)) (pair (bare_key) (integer))) (table (dotted_key (bare_key) (bare_key)) (pair (bare_key) (integer))))
```

(no import construct exists; dotted keys are the only path-shaped nodes)

### json — `{"a": {"b": 1}}`

`(document (object (pair (string (string_fragment)) (object (pair (string (string_fragment)) (number))))))` — no
import construct; keys are not navigation targets (documented).

### yaml — anchors/aliases

`(stream (document (block_mapping (block_mapping_value (anchor) (scalar)) (block_mapping_value (alias)))) )`
— in-document only; no cross-file construct.

### bash — `source ./sub/helper.sh / . ./other.sh / source helper_missing_xyz`

```
(program (comment) (command name: (command_name (word)) argument: (word)) (command name: (command_name (word)) argument: (word)) (command name: (command_name (word)) argument: (word)) (function_definition name: (word) body: (compound_statement …)))
```

```
  command [20..42] "source ./sub/helper.sh"
    command_name [20..26] "source"    argument: word "./sub/helper.sh"
  command [43..55] ". ./other.sh"
    command_name [43..44] "."        argument: word "./other.sh"
```

### markdown — `# Heading One … import X from "some-pkg"` (mdx-style line)

```
(document (section (atx_heading (atx_h1_marker) heading_content: (inline)) (section (atx_heading (atx_h2_marker) heading_content: (inline)) (paragraph (inline)))))
```

The `import X from "some-pkg"` line is a **bare `paragraph`** — no import construct (finding F3:
`.mdx` → Markdown row has no MDX support).

### java — `package com.example; import a.b.C; import a.b.*; import static a.b.D.m;`

```
(program (package_declaration (scoped_identifier (identifier) (identifier))) (import_declaration (scoped_identifier (scoped_identifier (identifier) (identifier)) (identifier))) (import_declaration (scoped_identifier (identifier) (identifier)) (asterisk)) (import_declaration (static) (scoped_identifier (scoped_identifier (scoped_identifier (identifier) (identifier) (identifier)) (identifier)))) (class_declaration …))
```

```
  import_declaration [21..34] "import a.b.C;"
    scoped_identifier [28..33] "a.b.C"   (whole path, one node)
  import_declaration [35..48] "import a.b.*;"
    scoped_identifier "a.b"  asterisk "*"
  import_declaration [49..71] "import static a.b.D.m;"
    static  scoped_identifier "a.b.D.m"
```

### csharp — `using System; / using MyAlias = A.B.C; / using Ns1;`

```
(compilation_unit (using_directive (identifier)) (using_directive (identifier) (qualified_name (qualified_name (identifier) (identifier)) (identifier))) (using_directive (identifier)) (namespace_declaration …))
```

```
  using_directive [14..36] "using MyAlias = A.B.C;"
    identifier "MyAlias"   =   qualified_name "A.B.C"
```

### ruby — `require 'a/b' / require_relative 'c' / autoload :D, 'd/e'`

```
(program (call (identifier) (argument_list (string (string_content)))) (call (identifier) (argument_list (string (string_content)))) (call (identifier) (argument_list (symbol (string (string_content))) (string (string_content)))) (class …))
```

```
  call [0..13] "require 'a/b'"         (identifier "require", string_content "a/b")
  call [14..34] "require_relative 'c'" (identifier "require_relative", string_content "c")
  call [35..54] "autoload :D, 'd/e'"   (identifier "autoload", symbol ":D", string_content "d/e")
  class [55..107] "class F …"
    method [65..86] "def empty?; true; end"
      identifier [69..75] "empty?"        ← ONE node incl. `?` (the index stores it whole;
                                          extraction splits it — bug B3)
```

### scheme — `(define-library (foo core) (export f g) (import (library (bar util))) …)`

```
(program (list (symbol) (list (symbol) (symbol)) (list (symbol) (symbol) (symbol)) (list (symbol) (list (symbol) (list (symbol) (symbol)))) (list (symbol) (list (symbol) (symbol)) (list (symbol) (symbol) (number))) (list (symbol) (symbol) (number))))
```

```
  list [0..115] "(define-library …)"
    symbol "define-library"
    list [16..26] "(foo core)"   (symbol "foo", symbol "core")
    list "(export f g)"
    list "(import (library (bar util)))"
```

Flat S-expression: the library path `(foo core)` is an anonymous `list` of `symbol`s — no
path-shaped node (finding F6: the index keys it under `foo` only).

### clojure — the ns form + the reported reference + keyword auto-resolve

```
(source (list_lit value: (sym_lit name: (sym_name)) value: (sym_lit name: (sym_name)) value: (list_lit value: (kwd_lit name: (kwd_name)) value: (vec_lit value: (sym_lit name: (sym_name)) value: (kwd_lit name: (kwd_name)) value: (sym_lit name: (sym_name))) value: (vec_lit value: (sym_lit name: (sym_name)) value: (kwd_lit name: (kwd_name)) value: (vec_lit value: (sym_lit name: (sym_name))))) value: (list_lit value: (kwd_lit name: (kwd_name)) value: (sym_lit name: (sym_name)))) (list_lit value: (sym_lit name: (sym_name)) value: (sym_lit name: (sym_name)) value: (vec_lit value: (sym_lit name: (sym_name))) value: (sym_lit name: (sym_name))) (list_lit value: (sym_lit namespace: (sym_ns) name: (sym_name)) value: (num_lit)) (list_lit value: (sym_lit name: (sym_name))))
```

```
  list_lit [0..108] "(ns my.app (:require …) (:import java.util.Date))"
    sym_lit "ns"   sym_lit "my.app"
    list_lit "(:require [some.ns :as jwks] [other.ns :refer [thunk]])"
      kwd_lit ":require"
      vec_lit "[some.ns :as jwks]"
        sym_lit [24..31] "some.ns"      (sym_name — the WHOLE dotted namespace, one node)
        kwd_lit ":as"
        sym_lit [36..40] "jwks"          (the alias)
      vec_lit "[other.ns :refer [thunk]]"
        sym_lit "other.ns"  kwd_lit ":refer"  vec_lit (sym_lit "thunk")
  list_lit [142..168] "(jwks/fetch-issuer-info 1)"
    sym_lit [143..165] "jwks/fetch-issuer-info"      ← ONE node:
      sym_ns [143..147] "jwks"   "/"   sym_name [148..165] "fetch-issuer-info"
  kwd_lit [51..67] "::jwks/local-opt"                (auto-resolve keyword)
    "::"  kwd_ns [53..57] "jwks"  "/"  kwd_name [58..67] "local-opt"
```

The sibling lane's conclusion is confirmed by this independent dump: **the grammar is right**
(`sym_lit`/`kwd_lit` cover the namespaced form whole, with the alias separable); the defect is in
redline's text-level word rule (bug B1).

### plain — `hello world` (f.xyz)

`NO GRAMMAR for plain` (the probe's honest output — `language_for(Plain)` is `None`).

---

## What actually happens today on M-. (the measured end-to-end, 7/7 legs)

| leg | buffer | point | measured result |
|---|---|---|---|
| L1 | `src/main.rs` (Rust) | end of `Alpha` | **Definition picker**, cross-file unique, row `src/alpha.rs` — silent-jump rule: same-file-only; cross-file unique goes to the picker (jump-ambiguity) |
| L2 | `app.clj` | `(thunk)` | **Definition picker** on `some/ns.clj` — plain Clojure vars DO resolve in-workspace today |
| L3 | `app.clj` | inside `jwks/fetch-issuer-info` (CUP (3,19) = end of `issuer`) | **`no provider resolution for `issuer`: no tooling provider handles language `clojure` (4 provider(s) registered, none attempted)`** — the index holds `fetch-issuer-info` whole; the extraction split on the hyphen |
| L4 | `imp.py` | `json.dumps({})` | **Definition picker with the stdlib tooling row** (`json/__init__.py`) — the fetch gate never even asks (stdlib is local) |
| L5a | `imp.c` | `thing_x(1)` inside `main` (declared in `sub/thing.h`) | **Picker on `main`** — the enclosing-symbol fallback swallows the miss; the header is never offered (bug B5) |
| L5b | `imp.c` | `MAX(1, 2)` (`#define MAX` in `sub/thing.h`) | **Definition picker on `sub/thing.h`** — header `#define`s DO resolve in-workspace |
| L6 | `rb/f.rb` | `F.new.empty?` (inside `empty`) | **`no provider resolution for `empty`: no tooling provider handles language `ruby` (4 provider(s) registered, none attempted)`** — the index holds `empty?` whole; extraction split on `?` |

Layer-2 (provider chain, verbatim messages, all `confirm_fetch = None`):

```
c / cpp / java / csharp / ruby / scheme / clojure / toml / json / yaml / bash / markdown
  → MISS no tooling provider handles language `<lang>` (4 provider(s) registered, none attempted)
plain (language=None)
  → MISS no tooling provider could resolve symbol `ghost_fn` (tried 4 provider(s): rust, javascript, python, go)
python `json.dumps`            → HIT /usr/lib/python3.14/json/__init__.py line=Some(185)
python bare `ODict`+scope      → HIT /usr/lib/python3.14/collections/__init__.py line=Some(89)
python `missing_pkg_xyz.ghost` → MISS — probe: "install refused: `pip install missing_pkg_xyz`
                       requires a fetch confirmation hook (confirm_fetch); a provider never
                       installs without one"   (FLAG, don't guess — holds)
javascript `hello`+scope       → HIT …/node_modules/acme-lib/index.js line=Some(1)
javascript `ns.helper`+scope   → HIT …/node_modules/acme-lib/index.js line=Some(2)
javascript `rel` + scope ./rel-mod → HIT …/jsrel/rel-mod.ts external=false line=Some(1)
javascript missing pkg (no package.json) → ERR "no package.json found walking up from …; not an npm
                       project, so `missing_lib_x` cannot be resolved"
go (no go.mod)                 → ERR "no go.mod under workspace root …; not a Go module project"
go (go.mod, pkg not required)  → ERR "package `b` is not a required module in go.mod (workspace …)"
rust (not a cargo project)     → ERR "no Cargo.toml under workspace root …; not a cargo project"
rust `serde::Deserialize`+scope (redline worktree) → HIT ~/.cargo/registry/src/…/serde-1.0.229/
                       src/core/de/mod.rs line=Some(554)   (offline: lock + populated cache)
```

---

## Bugs and findings (recorded — NONE fixed in this audit)

- **B1 — symbol constituents are not language-aware** (the sibling lane's subject, `issue-language-aware-symbols`): `is_word_char` = alnum + `_` (`buffer.rs:76`), so Clojure hyphens, `/`, and any language's `?`/`!`/`$` split a grammar-whole name at extraction. End-to-end measured for Clojure (L3) and Ruby (L6). **Fence: sibling lane (slot 1).**
- **B2 — JS/TS `$`**: the grammar emits ONE `identifier "$dollar"` (dump quoted); extraction yields `dollar`; the index would store `$dollar`. Same defect class as B1 — **sibling-lane fence**; recorded here because the matrix's JS/TS rows depend on it.
- **B3 — Ruby `?`/`!`**: the index stores `empty?`/`save!` whole (`extract_symbols` measured); extraction splits at `?`/`!` (L6). Same class — **sibling-lane fence**.
- **B4 — Rust lifetime `'a`**: `'` is not a word char, so M-. inside a lifetime extracts `a`. Minor; same class; no evidence of user impact — recorded for the lane's table.
- **B5 — the enclosing-fallback misroute swallows the no-provider miss for in-function calls** (measured L5a): when the symbol-at-point step finds nothing AND the point sits inside an indexed definition, M-. opens the **enclosing symbol's** picker instead of reaching the resolver (or an honest "unresolved" flag). For C this makes the `#include` gap invisible-and-wrong: the user asked about `thing_x` and got `main` candidates. For every no-provider language, in-function M-. behaves this way today. This is a *selection-rule* question (does the enclosing fallback belong before the tooling fall-through?) — a product decision, filed, not decided here.
- **F1 — `jsx` in the js provider's `languages()` is a dead string**: no row is named `"jsx"`; jsx files dispatch as `"javascript"`. Harmless, but the dispatch table claims a language that never arrives.
- **F2 — provider-miss detail is swallowed by the chain's bail**: the trace carries each provider's reason (e.g. `install refused: …`, `not an npm project`), but `apply_resolve_event` surfaces only the chain-level `no tooling provider could resolve symbol … (tried N provider(s): …)` — so a Python "flagged, not installed" miss and a "no such symbol anywhere" miss read identically. Minor UX; the flag *is* happening (verified the install never ran), just with a generic message.
- **F3 — `.mdx` maps to the Markdown grammar**, which parses MDX `import` lines as plain paragraphs: MDX imports are invisible to redline. Known-limitation, not a gap to fill here (MDX is its own language).
- **F4 — the `go` toolchain is absent on this box**, so the go provider's `go`-invoking paths (past go.mod parsing) are untestable offline; the in-module convention (plan-013 rank 5) must be verified in CI or flagged as toolchain-dependent.
- **F5 — the go provider's miss messages predate toolchain errors**: with a go.mod present but `go` missing, the user would see a spawn error rather than "no go toolchain" — untestable here (no `go`), recorded as a predicted shape from `go_provider.rs:94,113`.
- **F6 — Scheme library names are indexed under the FIRST name-list symbol only**: `(define-library (foo core))` → index key `foo`, dropping `foo.core` (measured: `SYM kind=Type name="foo"`). Any future Scheme work must re-key to the full library path.

## Sibling lane (slot 1, `lang-symbols`) — what I could and could not confirm

Read-only observation (their worktree, uncommitted at audit time; the branch `lang-symbols` is
still at `c5f93c1` — **nothing landed**):

- **Confirmed** (matches my independent measurements): the grammar gives `sym_lit` =
  `sym_ns` + `/` + `sym_name` whole; the namespace→file convention (dots → dirs, hyphens →
  underscores, `.clj`/`.cljc`/`.cljs`) — they cite the SAME kaocha evidence I measured
  (`kaocha.core-ext` → `src/kaocha/core_ext.clj`), plus `rewrite-clj-cli` (`rewrite-clj.cli` →
  `src/rewrite_clj/cli.clj`, which I did not independently check); their probe additionally
  established the `kwd_lit` children are `::` + `kwd_ns` + `kwd_name` (my dump agrees).
- **Not confirmable**: their `ns_aliases()` extraction (uncommitted `crates/redline-syntax/src/clojure.rs`,
  189 lines: alias-map builder over the top-level `ns` form, identity entries for un-aliased
  requires, first-`ns`-form-only rule, "no parseable ns form → None, caller must not guess")
  has no tests I could run without touching their files; plan-017's Clojure issue should
  **re-verify it against the pinned grammar** when it lands, rather than trusting an
  in-flight implementation.

---

## Ranked gap list

Ranked by **user-visible value × evidence of breakage × cheapness to verify**, per the audit rule.
Value is anchored to the origin: the user asked for this because *their* Clojure jump is broken,
and redline's daily material is the agent-coding loop (Rust/TS heavy, plus whatever the user opens).

| rank | gap | value | evidence of breakage | cheapness to verify | why this rank |
|---|---|---|---|---|---|
| 1 | **Clojure alias/namespace resolution** (ns-form alias map + namespace→file + `alias/var` / `::alias/var`) | **highest** — the user's own report; the origin of the plan | **measured end-to-end** (L3): a name whose definition sits in the OPEN PROJECT bails `no tooling provider handles language clojure` | **cheap**: pure convention (no toolchain), verified on real layouts; grammar shapes all dumped; one query + one table row + one extractor | Value dominates: it is the plan's stated first instance and the only row where "should resolve" currently fails *inside the workspace itself* |
| 2 | **Java `a.b.C` → `a/b/C.java`** (source-root heuristic) | medium-high (Java is a registered, indexed language with zero import resolution) | BAIL measured (layer 2); grammar shape dumped | cheap: pure convention (JLS §7.6), offline-testable; source-root heuristic is the only judgment call | Second best value×cheapness ratio after Clojure; no constituent-bug prerequisite (Java identifiers carry no split characters in the current rule) |
| 3 | **C/C++ quoted `#include "x/y.h"`** (relative to the includer) | medium (C/C++ are indexed; the common cross-file case is header → source) | **measured misroute** (L5a) + BAIL outside functions; `#define` header hits DO work (L5b) | cheap: convention is the quoted-include search order (C standard §6.10.2 / GCC); one query on `preproc_include`; angle-bracket/system → flag | Cheapest real resolution left, but lower user value than Java/Clojure; the enclosing-fallback decision (B5) interacts with it |
| 4 | **Go in-module import path → directory** | medium-low (provider already exists; in-module relative imports are the un-covered half) | provider BAILs past go.mod parsing; `go` absent here (F4) | cheap *if* the toolchain exists in CI; otherwise the convention test is file-system-only | Rank below C/Java: the toolchain gap (F4) makes verification less certain, and Go usage here is thinner |
| 5 | **Ruby `require_relative`** (after B3 lands) | low-medium | B3 measured (L6) — but the constituent fix is the sibling lane's, and until it lands even in-file jumps fail | cheap once B3 lands: `require_relative` → sibling `.rb` is a two-line convention | Prerequisite-dependency pushes it down; the pure part is small |
| 6 | **Bash `source ./x` / `. ./x`** | low | BAIL measured (layer 2) | trivial to implement; trivial value | Honesty: it is the cheapest gap with the least user-visible value |
| 7 | **Flag rows**: C# (no directory convention — a convention entry would be a *guess*), Scheme (library layout implementation-defined), Ruby bare `require`/autoload (LOAD_PATH), C++ ADL/templates/using (semantic), Python relative `from .` (sys.path root) | — | each measured as BAIL; each verified undecidable by convention (citations above) | — | They must end as **flag, don't guess** — one documentation+regression issue, last, after the resolutions, so the flag set is the *residual*, not the plan |
| n/a | TOML / JSON / YAML / Markdown / Plain | — | no cross-file import semantics exist (JSON/YAML have no symbols at all — documented) | — | N/A rows; the mdx mapping (F3) is recorded as a known limitation, not a plan-017 work item |

**Why this ordering and not "all languages in one sweep":** the plan's mechanism issue (rank 0)
makes each language a *table entry + query*; the rank-1–6 entries differ only in that table row,
so sequencing them by value×evidence×cheapness is free. The two constituent bugs (B1–B4) are a
prerequisite, not a step: ranks 1/5 are *untestable end-to-end* until the sibling lane lands, but
their offline tests (extractor + convention table) do not depend on it, so plan-017 can proceed in
parallel and integrate.

## Proposed issue order for plan 017

1. **00-audit** (this issue — done: the matrix, the measured counts, the dumps, this file).
2. **01 — the mechanism**: import/alias form extraction (tree-sitter queries in the
   `flat_define`/`rust_tables_query` style, per language) + the **conventions table**
   (name → relative path, language-keyed, offline-testable) + the resolution order
   (tooling provider → convention → flag). No per-language special cases in `xref`; a new
   language is a table row plus a query. Ships with the table rows that are already measured
   above and the flags that need no code.
3. **02 — Clojure wiring** (rank 1): ns-form alias-map query, namespace→file convention row
   (`.clj`/`.cljc`/`.cljs`), `alias/var` + `::alias/var` reference handling. **Prerequisite: the
   sibling lane's constituent fix** (B1) — until then, E2E tests are gated, offline tests are not.
4. **03 — Java** (rank 2): import-declaration query on the dumped node shape + `a.b.C` →
   `a/b/C.java` convention row + source-root heuristic; wildcard/static imports → flag.
5. **04 — C/C++ quoted includes** (rank 3): relative-to-includer resolution; angle/system and all
   C++ semantic forms → flag. **Includes the B5 decision** (enclosing-fallback vs resolver order) —
   the C case is where the misroute is user-visible and measurable.
6. **05 — Go in-module** (rank 4): import path → module-relative directory; alias/dot import
   extraction (shapes dumped); cross-module stays on the provider (toolchain-gated, F4).
7. **06 — Ruby `require_relative`** (rank 5, after B3).
8. **07 — Bash relative `source`/`.`** (rank 6).
9. **08 — the honest gap list lands as flags**: C#, Scheme, Ruby-bare-require, C++-semantic,
   Python-relative — each documented with the citation above, each pinned by a "bails, does not
   guess" regression test; plus the findings F1–F6 disposition.

*Why this order:* the mechanism (02-in-the-plan, issue 01) is a dependency of every wiring issue,
so it goes first; then strictly by the ranked gap list; the flag issue goes last so that it
documents the *residual* after every decidable case has a row — flagging a name that a later
issue could have resolved would be the `pip install df`-class over-eager mistake in reverse.

---

## Disclosure

- Files touched in this worktree: **only** `examples/audit017_dump.rs` (throwaway probe, to be
  deleted before the commit — nothing under `src/` or `crates/` was modified; no battery needed,
  no product behaviour changed).
- Out-of-repo scratch: `/tmp/audit017/` (fixtures, probe output, PTY driver + fixture repo
  `/tmp/audit017/ptyrepo`). Read-only inspection of the sibling worktree (slot 1) for its
  in-flight `clojure.rs` — no file there was modified.
- No network, no installs: the one registry touch was `cargo metadata`-driven via the in-tree
  cargo provider against the project's own lockfile/cache (the sanctioned, pre-gated Rust fetch
  path — it read, it did not fetch: all sources were already cached).
- PTY batteries: one 7-leg battery (7/7) + one 6-leg probe session, each preceded by
  `pgrep -x -c timeout` (= 0); output redirected to files, exit codes read, nothing piped to
  `tail`.

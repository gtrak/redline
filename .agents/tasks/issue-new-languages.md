# Task: basic support for Clojure, Lisp-family, Java, C#, Ruby

You are the implementation worker. Repo root is your cwd. Self-contained.
Read `.agents/skills/tree-sitter/SKILL.md` and the ABI-pinning rules
(001/007 lessons: grammar crates are workspace deps; the registry owns
language_for/extension map; NODE_TYPES probe before any arm).

## Origin (user directive)

"add clojure and lisp support, java, c#, ruby" — basic support per the
capability ladder, tracked in docs/language-coverage.md. The registry
currently has 14 languages; this adds 4-5 new ones.

## What "basic support" means per language (the established ladder)

For each language, in staged commits (one per language, honest-stop at
any point):
1. **registry.rs**: `LanguageId` variant + extension map + grammar
   config (highlight queries — START from the grammar crate's own
   highlights.scm adapted to our query format; verify it compiles).
   Verify what the pinned/available grammar crates actually are FIRST
   (tree-sitter-java, tree-sitter-c-sharp, tree-sitter-ruby are mature;
   for Clojure/Lisp: check what's in the local cargo registry cache —
   tree-sitter-clojure exists; for "lisp" the family choice (common
   lisp vs scheme vs elisp) is YOUR call with a stated rationale —
   pick the grammar crate that actually exists and covers the most,
   or defer lisp honestly if no crate is usable offline).
2. **queries.rs**: an outline/definition query (symbols: functions,
   classes, methods, etc. per idiom — Java classes/methods, C# classes/
   methods/properties, Ruby classes/methods/modules, Clojure defn/def/
   ns, lisp defun/defvar...). Extraction tests per language.
3. **node.rs**: identifier predicates + scope walks per the lang-pred
   pattern (NODE_TYPES probe first; honest N/A where the grammar has no
   member/path concept; Java/C#/Ruby dotted paths ARE genuinely
   path-shaped — member_expression/attribute chains, same rule as JS).
4. **Index walk sets** (store.rs `source_extensions_for`): java/cs/rb/
   clj etc. — verify the registry's extension map entries exist FIRST
   (registry.rs is the authority; add the map entries as part of step 1).
5. **Coverage doc rows** + the matrix's new-language note.
6. **NO providers** — the provider chain stays 4 (package-manager
   machinery is a separate decision); these languages get the honest
   "none — honest bail" provider cell, and M-. works via the index
   (path-shaped via node_at, bare via the project index).

## Constraints

- Gate: `cargo test --workspace` + clippy (read the exit) +
  `tools/gate.sh full` (flock, `cargo build` first). Budget ~70 tool
  calls; staged commits per language; honest-stop at half (commit what
  landed + coverage rows updated).
- Scope fence: `Cargo.toml` + `Cargo.lock` (grammar deps ONLY),
  `src/syntax/registry.rs`, `src/syntax/queries.rs`, `src/syntax/node.rs`,
  `src/app/store.rs` (walk sets ONLY — the source_extensions_for
  function), `docs/language-coverage.md`, `docs/provider-matrix.md`.
  NO app logic, no providers, no resolver crate.
- A parallel lane owns src/app/store.rs's M-./command regions (Rung 4 +
  dotted_path_container) — your store.rs touch is the walk-set function
  ONLY; if a merge conflict appears there, resolve in favor of both
  (different functions).

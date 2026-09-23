# issue-language-grid-truth — the language × capability grid is missing whole capabilities, and three docs state stale facts

**Found by:** the per-language audit (findings F4–F9). One pass, because they share a cause: the grid
in `docs/language-coverage.md` was built capability-by-capability as features landed, so **whole
mechanisms never got a row**, and some counts and claims have drifted since.

## Missing from the grid entirely

- **F4 — the Rust field/trait tables.** `src/nav/index/symbol_index.rs:42-59` (`rust_tables`,
  `rust_fields`, `rust_traits`) power field and impl-method M-. (`definitions.rs:739`,
  `type_member_candidates`). No row, no column, no Known corner in the grid; described only in
  `docs/provider-matrix.md` as "the 010-01 per-file Rust tables". **Also a correctness limit worth
  stating where the grid can see it:** `field_locations` is keyed by the **bare struct name**, so two
  same-named structs in different modules with a same-named field collapse and the "never a guess"
  contract is weakened by keying rather than logic. Today that is recorded only in the internal
  tracker.
- **F5 — imenu impl-parent grouping** is Rust-only (`picker.rs:379,400,420-448` consume
  `RustTables`; `xref.rs:509` is `None` elsewhere, so other languages get the flat indent). The code
  says so; no doc does.
- **F6 — `M-?` references.** `src/search/references.rs:20-30` + `crates/redline-syntax/src/tokens.rs`.
  Behaviour is uniform for every grammar-bearing language (comment/string hits dropped), with two
  documented exceptions (Markdown's inactive filter, Plain's fallback) — so this is a *documentation*
  gap, not a behavioural one, but the grid never mentions the capability.
- **F9 — `injections_query` is loaded but inert.** `language.rs:127,215,408` populate it and
  `registry.rs:145` passes it to `HighlightConfiguration::new`, but the callback is `|_| None`
  (`highlight.rs:117`, documented at `:264-266`). Net: fenced Markdown blocks, Rust `html!` bodies and
  JS tagged templates are never highlighted as their embedded language — **uniformly, for every
  language**, so it is not a per-language gate, but the grid's Markdown "highlighting — works" cell
  can be read as covering it.

## Stale claims (each verified against the code)

- **F8a** `src/app/store/notes.rs:303` — *"Kept in lockstep with `src/syntax/node.rs`'s
  `is_rust_identifier_kind`"*. **`src/syntax/` no longer exists** and the symbol was deleted
  (`language.rs:9-12` says so). The 6-kind list is now a hand-copy of the Rust row's
  `identifier_kinds` (`language.rs:141-148`) with a comment pointing at a deleted symbol — a desync
  hazard. Read the table instead.
- **F8b** `docs/language-coverage.md:129` — "for every language with a definition query (**all 17
  non-Plain**)". There are **18** non-Plain languages and **16** with a `definition_query` (JSON and
  YAML are `None` — the doc's own rows say so). Off by one *and* self-contradictory with `:115`.
- **F8c** `README.md:201` — "the language × capability grid (all **14** registry languages)". The
  registry has **19**.
- **F8d** `README.md:84` — the M-. entry names only the Rust toolchain ("Rust: `cargo metadata` →
  …"); the JS/TS/Python/Go providers are not mentioned. It understates a now-five-language feature.
- **F7** `src/search/rg.rs:462-479` — the doc comment says it mirrors the grammar registry's
  extension map; it lists 12 names and **misses `tsx`, `java`, `csharp`, `ruby`, `scheme`,
  `clojure`**, maps `typescript`→`*.ts` only (no `.tsx`) and `javascript`→`*.js` (no `jsx/mjs/cjs`).
  Currently **latent** — `SearchConfig.file_type` is never `Some` in production (all four sites pass
  `None`) — so fix the claim or delete the table; do not leave a mirror that does not mirror.

## Requirement

The grid is this project's "what's covered, per language" record; a capability that exists but has no
row is invisible in exactly the way the annotation anchor was (documented, and still surprising — see
`issue-annotations-symbol-identity`). So:

1. **Add rows** for the capabilities above, with the same evidence standard the grid already uses
   (a test name, a drive leg, or a source line) and the same honest vocabulary
   (`works (live)` / `works (unit)` / `degrades to the bail` / `not implemented`).
2. **State the per-language truth for each** — including the Rust-only ones, marked as such.
3. **Fix every stale claim** listed above, and **re-verify the counts by measuring** (`rg` for the
   table's rows, the `definition_query` fields, the registry's extensions) rather than adjusting the
   number by one.
4. **Do not change behaviour.** This is a documentation pass; if any claim turns out to be *right* and
   the code wrong, stop and file that separately rather than editing the doc to match.

## Acceptance

- Every capability in the "missing from the grid" list has a row with evidence.
- Every stale claim above is corrected, and each corrected number is accompanied by the command that
  measured it.
- A reader can answer "does this work in Python?" for field M-., imenu grouping, `M-?`, and embedded
  highlighting, without reading code.
- `cargo test --workspace` and clippy stay green (docs-only change should not move the count).
